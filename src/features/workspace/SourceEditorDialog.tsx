import { useEffect, useMemo, useRef, useState, type FormEvent, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { emitFocusSourceRequest, upsertSchedulerGroup } from '../../bridge/desktop'
import { createSourceSyncOptions, resolveInstagramSourceSyncOptions, resolveTikTokSourceSyncOptions, resolveTwitterSourceSyncOptions } from '../../domain/sourceSyncOptions'
import type {
  ProviderKey,
  SchedulerGroup,
  SourceProfile,
  SourceProfileUpsert,
  WorkspaceSnapshot,
} from '../../domain/models'
import { useAppStore } from '../../state/appStore'
import { buildProviderAccountSettingsDraft, extractSourceDefaultsFromAccountSettings } from '../accounts/providerAccountSettings'
import { createSourceDraft, getSourceDisplayName, mapSourceToDraft } from '../sources/sourceDrafts'
import { SourceEditorSyncPanel } from './SourceEditorSyncPanel'
import { findDuplicateSource, type ClipboardProfileSeed } from './workspaceProfiles'

interface SourceEditorDialogProps {
  seed?: ClipboardProfileSeed
  preferredProvider?: ProviderKey
  preferredAccountId?: string
  snapshot: WorkspaceSnapshot
  source?: SourceProfile
  onClose: () => void
  onSaved: (source: SourceProfile) => void
  onDirtyChange?: (dirty: boolean) => void
  onEditAccount?: (accountId: string) => void
  onAdvancedAccountSettings?: (accountId: string) => void
}

type InstagramSyncUpsertOptions = NonNullable<NonNullable<SourceProfileUpsert['syncOptions']>['instagram']>
type TwitterSyncUpsertOptions = NonNullable<NonNullable<SourceProfileUpsert['syncOptions']>['twitter']>
type TikTokSyncUpsertOptions = NonNullable<NonNullable<SourceProfileUpsert['syncOptions']>['tiktok']>
type SourceEditorTabKey = 'profile' | 'sync' | 'history'

export function SourceEditorDialog({
  seed,
  preferredProvider,
  preferredAccountId,
  snapshot,
  source,
  onClose,
  onSaved,
  onDirtyChange,
  onEditAccount,
  onAdvancedAccountSettings,
}: SourceEditorDialogProps) {
  const pendingCommand = useAppStore((state) => state.pendingCommand)
  const loadProviderAccountEditor = useAppStore((state) => state.loadProviderAccountEditor)
  const runSourceSync = useAppStore((state) => state.runSourceSync)
  const upsertSourceProfile = useAppStore((state) => state.upsertSourceProfile)
  const [draft, setDraft] = useState<SourceProfileUpsert>(() =>
    createInitialDraft(source, seed, preferredProvider, preferredAccountId, snapshot),
  )
  const [selectedLabels, setSelectedLabels] = useState<string[]>(() => (source ? [...source.labels] : []))
  const [labelDraft, setLabelDraft] = useState('')
  const [selectedGroupId, setSelectedGroupId] = useState<string>(source?.groupId ?? '')
  const [creatingGroup, setCreatingGroup] = useState(false)
  const [newGroupName, setNewGroupName] = useState('')
  const [localGroups, setLocalGroups] = useState<SchedulerGroup[]>(snapshot.schedulerGroups)
  const [activeTab, setActiveTab] = useState<SourceEditorTabKey>('profile')
  const [accountDefaultsHint, setAccountDefaultsHint] = useState<string>()
  const [submitError, setSubmitError] = useState<string>()
  // O handle fica bloqueado por padrão para evitar trocas acidentais de
  // identidade. Todos os providers permitem um override manual explícito para
  // recuperar perfis renomeados quando a resolução automática não for possível.
  const [handleUnlocked, setHandleUnlocked] = useState(false)
  const appliedDefaultsAccountId = useRef<string | undefined>(undefined)
  const isEditMode = Boolean(source)

  const availableAccounts = useMemo(
    () => snapshot.accounts.filter((account) => account.provider === draft.provider),
    [draft.provider, snapshot.accounts],
  )
  const selectedAccount = useMemo(
    () => snapshot.accounts.find((account) => account.id === draft.accountId),
    [draft.accountId, snapshot.accounts],
  )
  const selectedSourceRuns = useMemo(
    () => (source ? snapshot.sourceSyncRuns.filter((run) => run.sourceId === source.id).slice(0, 10) : []),
    [snapshot.sourceSyncRuns, source],
  )
  const providerDescriptor = useMemo(
    () => snapshot.providerCatalog.find((provider) => provider.key === draft.provider),
    [draft.provider, snapshot.providerCatalog],
  )
  const providerDisplayName = providerDescriptor?.displayName ?? draft.provider
  const instagramSyncOptions = useMemo(
    () => resolveInstagramSourceSyncOptions(draft.provider, draft.syncOptions),
    [draft.provider, draft.syncOptions],
  )
  const labelsSuggestionListId = useMemo(
    () => `source-editor-label-suggestions-${source?.id ?? 'new'}`,
    [source?.id],
  )
  const availableLabelSuggestions = useMemo(() => {
    const labels = new Set<string>()
    snapshot.sources.forEach((entry) => {
      entry.labels.forEach((label) => {
        const normalized = normalizeLabel(label)
        if (normalized) {
          labels.add(normalized)
        }
      })
    })

    return Array.from(labels)
      .filter((label) => !selectedLabels.some((selectedLabel) => selectedLabel.toLowerCase() === label.toLowerCase()))
      .sort((left, right) => left.localeCompare(right))
  }, [selectedLabels, snapshot.sources])
  const filteredLabelSuggestions = useMemo(() => {
    const query = normalizeLabel(labelDraft).toLowerCase()
    if (!query) {
      return availableLabelSuggestions.slice(0, 12)
    }

    return availableLabelSuggestions.filter((label) => label.toLowerCase().includes(query)).slice(0, 12)
  }, [availableLabelSuggestions, labelDraft])
  const canSubmit = Boolean(draft.accountId) && draft.handle.trim().length > 0 && availableAccounts.length > 0
  const [initialSignature] = useState(() =>
    createDirtySignature(createInitialDraft(source, seed, preferredProvider, preferredAccountId, snapshot), source ? [...source.labels] : [], '', source?.groupId ?? ''),
  )
  const currentSignature = useMemo(
    () => createDirtySignature(draft, selectedLabels, labelDraft, selectedGroupId),
    [draft, labelDraft, selectedGroupId, selectedLabels],
  )
  const isDirty = currentSignature !== initialSignature

  useEffect(() => {
    onDirtyChange?.(isDirty)
  }, [isDirty, onDirtyChange])

  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange])

  useEffect(() => {
    if (source || !draft.accountId || appliedDefaultsAccountId.current === draft.accountId) {
      return
    }

    const accountId = draft.accountId
    let disposed = false
    void loadProviderAccountEditor(accountId)
      .then((editor) => {
        if (disposed) {
          return
        }

        const accountDefaults = extractSourceDefaultsFromAccountSettings(
          editor.account.provider,
          buildProviderAccountSettingsDraft(editor.account.provider, editor.settings),
        )
        if (accountDefaults.readyForDownload !== undefined || accountDefaults.syncOptions) {
          setDraft((current) => ({
            ...current,
            readyForDownload: accountDefaults.readyForDownload ?? current.readyForDownload,
            syncOptions: accountDefaults.syncOptions ?? current.syncOptions,
          }))
        }
        if (accountDefaults.labels.length > 0) {
          setSelectedLabels(mergeLabels([], accountDefaults.labels))
          setLabelDraft('')
        }
        if (accountDefaults.labels.length > 0 || accountDefaults.readyForDownload !== undefined || accountDefaults.syncOptions) {
          setAccountDefaultsHint(`Defaults loaded from ${editor.account.displayName}`)
        }
        appliedDefaultsAccountId.current = accountId
      })
      .catch(() => undefined)

    return () => {
      disposed = true
    }
  }, [draft.accountId, loadProviderAccountEditor, source])

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key !== 'F2' || !draft.accountId) {
        return
      }

      event.preventDefault()
      if (onEditAccount) {
        onEditAccount(draft.accountId)
        return
      }

      onAdvancedAccountSettings?.(draft.accountId)
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [draft.accountId, onAdvancedAccountSettings, onEditAccount])

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!draft.accountId) {
      return
    }
    const submitter = (event.nativeEvent as SubmitEvent).submitter as HTMLButtonElement | null
    const syncAfterSave = !draft.id && submitter?.value === 'save-and-sync'

    const finalLabels = mergeLabels(selectedLabels, parseLabelCandidates(labelDraft))
    const payload: SourceProfileUpsert = {
      ...draft,
      handle: draft.handle.trim(),
      displayName: getSourceDisplayName(draft.handle, draft.displayName),
      accountId: draft.accountId,
      groupId: selectedGroupId || null,
      labels: finalLabels,
    }

    // Bloqueia duplicatas antes de chamar o backend: o handle normalizado não
    // pode colidir com outro perfil ativo do mesmo provider. Quando há colisão,
    // pedimos à janela principal que selecione o perfil existente.
    const existing = findDuplicateSource(snapshot.sources, payload.provider, payload.handle, source?.id)
    if (existing) {
      const existingLabel = existing.handle.trim() || existing.displayName
      setSubmitError(`O perfil "${existingLabel}" já existe nesta lista. Abrimos o perfil existente para você.`)
      void emitFocusSourceRequest(existing.id, { clearSearch: true })
      return
    }

    setSelectedLabels(finalLabels)
    setLabelDraft('')
    try {
      const savedSnapshot = await upsertSourceProfile(payload)
      const savedSource = resolveSavedSource(savedSnapshot.sources, payload)
      if (savedSource) {
        if (syncAfterSave) {
          await runSourceSync(savedSource.id, { trigger: 'manual' })
        }
        setSubmitError(undefined)
        onSaved(savedSource)
        onClose()
      }
    } catch (error) {
      setSubmitError(error instanceof Error ? error.message : String(error))
    }
  }

  async function handleForceImportedBackfill() {
    if (!source?.id) {
      return
    }

    await runSourceSync(source.id, {
      trigger: 'manual_force_imported_backfill',
      runMode: 'force_imported_backfill',
    })
  }

  async function handleTwitterFullTimelineBackfill() {
    if (!source?.id) {
      return
    }
    await runSourceSync(source.id, {
      trigger: 'manual_twitter_full_timeline_backfill',
      runMode: 'twitter_full_timeline_backfill',
    })
  }

  function commitLabelDraft() {
    const labelsToAdd = parseLabelCandidates(labelDraft)
    if (labelsToAdd.length === 0) {
      return
    }

    setSelectedLabels((current) => mergeLabels(current, labelsToAdd))
    setLabelDraft('')
  }

  function removeLabel(labelToRemove: string) {
    setSelectedLabels((current) => current.filter((label) => label.toLowerCase() !== labelToRemove.toLowerCase()))
  }

  function handleLabelInputKeyDown(event: ReactKeyboardEvent<HTMLInputElement>) {
    if (event.key === 'Enter' || event.key === ',') {
      event.preventDefault()
      commitLabelDraft()
      return
    }

    if (event.key === 'Backspace' && labelDraft.trim().length === 0 && selectedLabels.length > 0) {
      event.preventDefault()
      setSelectedLabels((current) => current.slice(0, current.length - 1))
    }
  }

  async function handleCreateGroup() {
    const name = newGroupName.trim()
    if (!name) return
    try {
      const criteria: import('../../domain/models').SchedulerPlanCriteria = {
        regular: false,
        temporary: false,
        favorite: false,
        readyForDownload: false,
        ignoreReadyForDownload: false,
        downloadUsers: false,
        downloadSubscriptions: false,
        userExists: false,
        userSuspended: false,
        userDeleted: false,
        labelsNo: false,
        labelsIncluded: [],
        labelsExcluded: [],
        ignoreExcludedLabels: false,
        sitesIncluded: [],
        sitesExcluded: [],
        groupIdsIncluded: [],
        groupIdsExcluded: [],
        daysIsDownloaded: false,
        dateInRange: true,
      }
      const newSnapshot = await upsertSchedulerGroup({ name, criteria })
      setLocalGroups(newSnapshot.schedulerGroups)
      const created = newSnapshot.schedulerGroups.find((g) => g.name === name)
      if (created) {
        setSelectedGroupId(created.id)
      }
      setNewGroupName('')
      setCreatingGroup(false)
    } catch (error) {
      console.error('Failed to create group:', error)
    }
  }

  function handleProviderChange(nextProvider: ProviderKey) {
    const nextAccountId = resolvePreferredAccountId(nextProvider, undefined, snapshot)
    appliedDefaultsAccountId.current = undefined
    setAccountDefaultsHint(undefined)
    setDraft((current) => ({
      ...current,
      provider: nextProvider,
      accountId:
        current.accountId && snapshot.accounts.some((account) => account.id === current.accountId && account.provider === nextProvider)
          ? current.accountId
          : nextAccountId,
      syncOptions: createSourceSyncOptions(nextProvider),
    }))
  }

  function handleAccountChange(nextAccountId: string) {
    appliedDefaultsAccountId.current = undefined
    setAccountDefaultsHint(undefined)
    setDraft((current) => ({
      ...current,
      accountId: nextAccountId.length > 0 ? nextAccountId : null,
    }))
  }

  function updateInstagramSyncOptions(mutate: (current: InstagramSyncUpsertOptions) => InstagramSyncUpsertOptions) {
    setDraft((current) => {
      const currentInstagram = resolveInstagramSourceSyncOptions(current.provider, current.syncOptions)
      if (!currentInstagram) {
        return current
      }

      return {
        ...current,
        syncOptions: {
          instagram: mutate(currentInstagram),
        },
      }
    })
  }

  function updateTwitterSyncOptions(mutate: (current: TwitterSyncUpsertOptions) => TwitterSyncUpsertOptions) {
    setDraft((current) => {
      const currentTwitter = resolveTwitterSourceSyncOptions(current.provider, current.syncOptions)
      if (!currentTwitter) {
        return current
      }

      return {
        ...current,
        syncOptions: {
          twitter: mutate(currentTwitter),
        },
      }
    })
  }

  function updateTikTokSyncOptions(mutate: (current: TikTokSyncUpsertOptions) => TikTokSyncUpsertOptions) {
    setDraft((current) => {
      const currentTikTok = resolveTikTokSourceSyncOptions(current.provider, current.syncOptions)
      if (!currentTikTok) {
        return current
      }

      return {
        ...current,
        syncOptions: {
          tiktok: mutate(currentTikTok),
        },
      }
    })
  }

  const headerTitle = draft.handle.trim().length > 0
    ? getSourceDisplayName(draft.handle, draft.displayName)
    : isEditMode
      ? (source?.handle ?? 'New profile')
      : 'New profile'
  const headerContextLine = isEditMode ? 'Editing profile' : 'New profile'
  const accountSummary = selectedAccount
    ? `${selectedAccount.authState.replaceAll('_', ' ')} · ${selectedAccount.authMode.replaceAll('_', ' ')}`
    : isEditMode
      ? 'No account linked to this profile.'
      : availableAccounts.length === 0
        ? `Create a ${providerDisplayName} account before saving this profile.`
        : 'Selecting an account applies its profile defaults.'
  return (
    <div className={`source-editor-shell source-editor-shell-provider-${draft.provider}`}>
      <form className="source-editor-form" onSubmit={handleSubmit}>
        <section className={`panel panel-accent source-editor-hero source-editor-hero-provider-${draft.provider}`}>
          <header className="source-editor-hero-header source-editor-hero-header-refresh">
            <div className="source-editor-provider-lockup">
              <div className="source-editor-provider-row">
                <span className="source-editor-provider-badge">{providerDisplayName}</span>
                <span className="source-editor-context-line">{headerContextLine}</span>
              </div>
              <div className="source-editor-hero-copy">
                <h2>{headerTitle}</h2>
              </div>
            </div>
            <span className={draft.readyForDownload ? 'status status-ready' : 'status status-degraded'}>
              {draft.readyForDownload ? 'Ready for download' : 'Paused'}
            </span>
          </header>

          {seed ? <p className="source-editor-context-note">Prefilled from clipboard URL.</p> : null}

          <div className="source-editor-context-grid">
            <label className="field source-editor-context-field">
              <span>Provider</span>
              {isEditMode ? (
                <div className="source-editor-static-field">{providerDisplayName}</div>
              ) : (
                <select onChange={(event) => handleProviderChange(event.target.value as ProviderKey)} value={draft.provider}>
                  {snapshot.providerCatalog.map((provider) => (
                    <option key={provider.key} value={provider.key}>
                      {provider.displayName}
                    </option>
                  ))}
                </select>
              )}
              <small>{isEditMode ? 'Locked while editing an existing profile.' : 'Choose the provider before saving.'}</small>
            </label>

            <label className="field source-editor-context-field">
              <span>Account</span>
              <div className="source-editor-account-row">
                <select
                  disabled={availableAccounts.length === 0}
                  onChange={(event) => handleAccountChange(event.target.value)}
                  required
                  value={draft.accountId ?? ''}
                >
                  <option disabled value="">
                    {availableAccounts.length === 0 ? 'No account available' : 'Select account'}
                  </option>
                  {availableAccounts.map((account) => (
                    <option key={account.id} value={account.id}>
                      {account.displayName}
                    </option>
                  ))}
                </select>
                <button
                  className="ghost-button source-editor-context-action-button"
                  disabled={!draft.accountId}
                  onClick={() => {
                    if (!draft.accountId) {
                      return
                    }

                    if (onEditAccount) {
                      onEditAccount(draft.accountId)
                      return
                    }

                    onAdvancedAccountSettings?.(draft.accountId)
                  }}
                  type="button"
                >
                  Edit account
                </button>
              </div>
              <small>{accountSummary}</small>
            </label>
          </div>

          {!isEditMode && accountDefaultsHint ? <p className="source-editor-context-note">{accountDefaultsHint}</p> : null}
        </section>

        <div className="source-editor-tab-bar" role="tablist" aria-label="Profile editor tabs">
          {([
            { key: 'profile', label: 'Profile' },
            { key: 'sync', label: 'Sync' },
            { key: 'history', label: 'History' },
          ] as const).map((tab) => (
            <button
              aria-controls={`source-editor-tab-${tab.key}`}
              aria-selected={activeTab === tab.key}
              className={activeTab === tab.key ? 'source-editor-tab source-editor-tab-active' : 'source-editor-tab'}
              id={`source-editor-tab-button-${tab.key}`}
              key={tab.key}
              onClick={() => setActiveTab(tab.key)}
              role="tab"
              type="button"
            >
              <span>{tab.label}</span>
              {tab.key === 'history' && selectedSourceRuns.length > 0 ? <small>{selectedSourceRuns.length}</small> : null}
            </button>
          ))}
        </div>

        {activeTab === 'profile' ? (
          <section
            aria-labelledby="source-editor-tab-button-profile"
            className="panel panel-accent source-editor-section source-editor-tab-panel source-editor-tab-panel-profile"
            id="source-editor-tab-profile"
            role="tabpanel"
          >
            <div className="form-grid source-editor-form-grid source-editor-form-grid-profile source-editor-profile-grid">
              <label className="field source-editor-profile-handle-field">
                <span>User URL</span>
                {isEditMode && !handleUnlocked ? (
                  <div className="source-editor-static-field source-editor-profile-readonly-field source-editor-handle-locked">
                    <span className="source-editor-handle-locked-value">{draft.handle}</span>
                    <button
                      type="button"
                      className="ghost-button source-editor-handle-edit-button"
                      onClick={() => setHandleUnlocked(true)}
                      title="Unlock the handle to fix a renamed profile"
                      aria-label="Edit handle"
                    >
                      Edit
                    </button>
                  </div>
                ) : (
                  <input
                    onChange={(event) => setDraft((current) => ({ ...current, handle: event.target.value }))}
                    placeholder="Enter user profile URL here..."
                    required
                    value={draft.handle}
                    autoFocus={handleUnlocked}
                  />
                )}
                {isEditMode ? (
                  handleUnlocked ? (
                    <small>
                      Manual override — use only to fix a renamed profile. New media is saved under the new handle;
                      already-downloaded files stay in the old folder.
                    </small>
                  ) : (
                    <small>User URL is locked for existing profiles. Use Edit to fix a renamed profile.</small>
                  )
                ) : null}
              </label>

              <label className="field source-editor-profile-name-field">
                <span>Friendly name</span>
                <input onChange={(event) => setDraft((current) => ({ ...current, displayName: event.target.value }))} placeholder="visual_lab" value={draft.displayName} />
                {isEditMode ? <small aria-hidden className="source-editor-field-spacer">&nbsp;</small> : null}
              </label>

              <label className="field">
                <span>Group</span>
                <div className="source-editor-group-row">
                  <select
                    onChange={(event) => {
                      const value = event.target.value
                      if (value === '__create__') {
                        setCreatingGroup(true)
                      } else {
                        setSelectedGroupId(value)
                        setCreatingGroup(false)
                      }
                    }}
                    value={creatingGroup ? '__create__' : selectedGroupId}
                  >
                    <option value="">No group</option>
                    {localGroups.map((group) => (
                      <option key={group.id} value={group.id}>{group.name}</option>
                    ))}
                    <option value="__create__">+ Create new group...</option>
                  </select>
                  {creatingGroup ? (
                    <div className="source-editor-group-create-row">
                      <input
                        onChange={(e) => setNewGroupName(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault()
                            void handleCreateGroup()
                          }
                        }}
                        placeholder="Group name"
                        type="text"
                        value={newGroupName}
                      />
                      <button
                        className="ghost-button"
                        disabled={newGroupName.trim().length === 0}
                        onClick={() => void handleCreateGroup()}
                        type="button"
                      >
                        Create
                      </button>
                    </div>
                  ) : null}
                </div>
              </label>

              <label className="field">
                <span>Labels</span>
                <div className="source-editor-label-input">
                  {selectedLabels.map((label) => (
                    <button aria-label={`Remove label ${label}`} className="source-editor-label-chip" key={label} onClick={() => removeLabel(label)} type="button">
                      {label}
                      <span aria-hidden>×</span>
                    </button>
                  ))}
                  <input
                    list={labelsSuggestionListId}
                    onBlur={() => commitLabelDraft()}
                    onChange={(event) => setLabelDraft(event.target.value)}
                    onKeyDown={handleLabelInputKeyDown}
                    placeholder="Type and press Enter"
                    value={labelDraft}
                  />
                  <button className="ghost-button source-editor-label-add" disabled={normalizeLabel(labelDraft).length === 0} onClick={() => commitLabelDraft()} type="button">
                    Add
                  </button>
                </div>
                <datalist id={labelsSuggestionListId}>
                  {filteredLabelSuggestions.map((label) => (
                    <option key={label} value={label} />
                  ))}
                </datalist>
                <small>Press Enter or comma to add labels.</small>
              </label>

              <label className="checkbox-row field-full source-editor-checkbox-inline">
                <input checked={draft.readyForDownload} onChange={(event) => setDraft((current) => ({ ...current, readyForDownload: event.target.checked }))} type="checkbox" />
                <span>Ready for download</span>
              </label>

              {instagramSyncOptions ? (
                <label className="field field-full source-editor-note-field">
                  <span>Profile note</span>
                  <textarea
                    onChange={(event) => updateInstagramSyncOptions((current) => ({ ...current, description: event.target.value }))}
                    placeholder="Optional profile note"
                    value={instagramSyncOptions.description ?? ''}
                  />
                </label>
              ) : null}
            </div>
          </section>
        ) : null}

        {activeTab === 'sync' ? (
          <section
            aria-labelledby="source-editor-tab-button-sync"
            className="panel panel-accent source-editor-section source-editor-tab-panel source-editor-tab-panel-sync"
            id="source-editor-tab-sync"
            role="tabpanel"
          >
            <SourceEditorSyncPanel
              onForceImportedBackfill={handleForceImportedBackfill}
              onTwitterFullTimelineBackfill={handleTwitterFullTimelineBackfill}
              onInstagramSyncOptionsChange={updateInstagramSyncOptions}
              onTikTokSyncOptionsChange={updateTikTokSyncOptions}
              onTwitterSyncOptionsChange={updateTwitterSyncOptions}
              provider={draft.provider}
              providerDisplayName={providerDisplayName}
              providerNote={providerDescriptor?.notes}
              source={source}
              syncOptions={draft.syncOptions}
            />
          </section>
        ) : null}

        {activeTab === 'history' ? (
          <section
            aria-labelledby="source-editor-tab-button-history"
            className="panel source-editor-section source-editor-runtime-section source-editor-tab-panel source-editor-tab-panel-history"
            id="source-editor-tab-history"
            role="tabpanel"
          >
            <div className="section-stack source-editor-history-stack">
              {selectedSourceRuns.length > 0 ? (
                selectedSourceRuns.map((run) => (
                  <article className="list-row source-editor-history-card" key={run.id}>
                    <header className="source-editor-history-meta">
                      <span className={`status ${historyStatusClass(run.status)}`}>{run.status}</span>
                      <small className="source-editor-history-date">{formatRunFinishedAt(run.finishedAt)}</small>
                      <small>{run.tool}</small>
                      <small>{run.trigger}</small>
                    </header>
                    <p className="source-editor-history-summary">{run.summary}</p>
                    <p className="source-editor-history-command" title={run.commandPreview}>{run.commandPreview}</p>
                  </article>
                ))
              ) : (
                <div className="empty-state source-editor-history-empty">No sync history for this profile yet.</div>
              )}
            </div>
          </section>
        ) : null}

        <footer className="source-editor-footer">
          {submitError ? (
            <p className="source-editor-submit-error" role="alert">
              {submitError}
            </p>
          ) : null}
          <div className="action-row">
            {isDirty ? (
              <div className="source-editor-dirty-indicator">
                <span aria-hidden className="source-editor-dirty-indicator-dot" />
                <span>Unsaved changes</span>
              </div>
            ) : null}
            <button className="ghost-button" disabled={Boolean(pendingCommand)} onClick={onClose} type="button">
              Cancel
            </button>
            <button className="primary-button" disabled={Boolean(pendingCommand) || !canSubmit} type="submit">
              {draft.id ? 'Save changes' : 'Create profile'}
            </button>
            {!draft.id ? (
              <button
                className="primary-button"
                disabled={Boolean(pendingCommand) || !canSubmit}
                type="submit"
                value="save-and-sync"
              >
                Save and Sync
              </button>
            ) : null}
          </div>
        </footer>
      </form>
    </div>
  )
}

function normalizeLabel(value: string): string {
  return value.trim()
}

function createDirtySignature(draft: SourceProfileUpsert, selectedLabels: string[], labelDraft: string, groupId: string): string {
  return JSON.stringify({
    draft: {
      ...draft,
      handle: draft.handle.trim(),
      displayName: draft.displayName.trim(),
      syncOptions: createSourceSyncOptions(draft.provider, draft.syncOptions),
    },
    selectedLabels: selectedLabels.map((label) => normalizeLabel(label)),
    labelDraft: labelDraft.trim(),
    groupId,
  })
}

function parseLabelCandidates(value: string): string[] {
  return value.split(',').map((candidate) => normalizeLabel(candidate)).filter((candidate) => candidate.length > 0)
}

function mergeLabels(base: string[], candidates: string[]): string[] {
  const merged = [...base]
  const known = new Set(base.map((label) => label.toLowerCase()))
  candidates.forEach((candidate) => {
    const key = candidate.toLowerCase()
    if (known.has(key)) {
      return
    }

    known.add(key)
    merged.push(candidate)
  })
  return merged
}

function createInitialDraft(
  source: SourceProfile | undefined,
  seed: ClipboardProfileSeed | undefined,
  preferredProvider: ProviderKey | undefined,
  preferredAccountId: string | undefined,
  snapshot: WorkspaceSnapshot,
): SourceProfileUpsert {
  if (source) {
    return mapSourceToDraft(source)
  }

  const draft = createSourceDraft(seed?.provider ?? preferredProvider)
  if (!seed) {
    return {
      ...draft,
      accountId: resolvePreferredAccountId(draft.provider, preferredAccountId, snapshot),
    }
  }

  return {
    ...draft,
    provider: seed.provider,
    handle: seed.handle,
    displayName: seed.displayName,
    accountId: resolvePreferredAccountId(seed.provider, preferredAccountId, snapshot),
  }
}

function resolvePreferredAccountId(provider: ProviderKey, preferredAccountId: string | undefined, snapshot: WorkspaceSnapshot): string | null {
  if (preferredAccountId && snapshot.accounts.some((account) => account.id === preferredAccountId && account.provider === provider)) {
    return preferredAccountId
  }

  const firstAccount = snapshot.accounts.find((account) => account.provider === provider)
  return firstAccount?.id ?? null
}

function historyStatusClass(status: string): string {
  if (status === 'succeeded') return 'status-succeeded'
  if (status === 'failed') return 'status-failed'
  return 'status-degraded'
}

function formatRunFinishedAt(value: string): string {
  const parsed = new Date(value)
  if (Number.isNaN(parsed.getTime())) {
    return value
  }
  return parsed.toLocaleString()
}

function resolveSavedSource(sources: SourceProfile[], payload: SourceProfileUpsert): SourceProfile | undefined {
  if (payload.id) {
    return sources.find((entry) => entry.id === payload.id)
  }

  return sources.find(
    (entry) =>
      entry.provider === payload.provider
      && entry.handle === payload.handle
      && (entry.accountId ?? null) === (payload.accountId ?? null),
  )
}
