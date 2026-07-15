import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent as ReactKeyboardEvent,
} from 'react'
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
  const profileNoteRef = useRef<HTMLTextAreaElement>(null)
  const isEditMode = Boolean(source)

  // Profile note grows with content and caps only at remaining tab/window space.
  const fitProfileNoteHeight = useCallback(() => {
    const el = profileNoteRef.current
    if (!el || activeTab !== 'profile') {
      return
    }

    const minHeight = 72
    el.style.height = '0px'
    el.style.overflowY = 'hidden'
    const contentHeight = el.scrollHeight

    const panel = el.closest('.source-editor-tab-panel-profile') as HTMLElement | null
    let maxHeight = Math.max(minHeight, Math.floor(window.innerHeight * 0.55))
    if (panel) {
      const panelRect = panel.getBoundingClientRect()
      const styles = window.getComputedStyle(panel)
      const padBottom = Number.parseFloat(styles.paddingBottom) || 0
      // Measure from textarea top so the field can use leftover panel space.
      const top = el.getBoundingClientRect().top
      maxHeight = Math.max(minHeight, Math.floor(panelRect.bottom - top - padBottom - 8))
    }

    const nextHeight = Math.min(Math.max(contentHeight, minHeight), maxHeight)
    el.style.height = `${nextHeight}px`
    el.style.overflowY = contentHeight > maxHeight ? 'auto' : 'hidden'
  }, [activeTab])

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

  useLayoutEffect(() => {
    fitProfileNoteHeight()
  }, [fitProfileNoteHeight, instagramSyncOptions?.description, activeTab])

  useEffect(() => {
    if (activeTab !== 'profile') {
      return undefined
    }

    const onResize = () => fitProfileNoteHeight()
    window.addEventListener('resize', onResize)
    const panel = profileNoteRef.current?.closest('.source-editor-tab-panel-profile') ?? null
    const Observer = typeof ResizeObserver === 'undefined' ? undefined : ResizeObserver
    const observer = panel && Observer ? new Observer(onResize) : undefined
    if (panel && observer) {
      observer.observe(panel)
    }

    return () => {
      window.removeEventListener('resize', onResize)
      observer?.disconnect()
    }
  }, [activeTab, fitProfileNoteHeight])

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

  // Title mirrors User URL (keep @); do not strip via getSourceDisplayName.
  const headerTitle = draft.handle.trim()
    || (isEditMode ? (source?.handle ?? 'Profile') : 'New profile')
  const headerSubtitle =
    draft.displayName.trim()
    && draft.displayName.trim().replace(/^@+/, '').toLowerCase()
      !== draft.handle.trim().replace(/^@+/, '').toLowerCase()
      ? draft.displayName.trim()
      : undefined
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
        <section className="source-editor-identity-strip" aria-label="Profile identity">
          <div className="source-editor-identity-main">
            <div className="source-editor-provider-row">
              <span className="source-editor-provider-badge">{providerDisplayName}</span>
              <span className="source-editor-context-line">{headerContextLine}</span>
              <button
                type="button"
                role="switch"
                aria-checked={draft.readyForDownload}
                aria-label="Ready for download"
                title={draft.readyForDownload ? 'Ready for download — click to pause' : 'Paused — click to mark ready for download'}
                className={
                  draft.readyForDownload
                    ? 'source-editor-ready-switch is-on'
                    : 'source-editor-ready-switch is-off'
                }
                onClick={() =>
                  setDraft((current) => ({ ...current, readyForDownload: !current.readyForDownload }))
                }
              >
                <span className="source-editor-ready-switch-label">Ready for download</span>
                <span className="source-editor-ready-switch-track" aria-hidden="true">
                  <span className="source-editor-ready-switch-thumb" />
                </span>
              </button>
            </div>
            <h2 className="source-editor-identity-title">{headerTitle}</h2>
            {headerSubtitle ? <p className="source-editor-identity-subtitle">{headerSubtitle}</p> : null}
            {seed ? <p className="source-editor-context-note">Prefilled from clipboard URL.</p> : null}
          </div>

          <div
            className={
              isEditMode
                ? 'source-editor-context-grid source-editor-context-grid-edit'
                : 'source-editor-context-grid'
            }
          >
            {!isEditMode ? (
              <label className="field source-editor-context-field">
                <span>Provider</span>
                <select onChange={(event) => handleProviderChange(event.target.value as ProviderKey)} value={draft.provider}>
                  {snapshot.providerCatalog.map((provider) => (
                    <option key={provider.key} value={provider.key}>
                      {provider.displayName}
                    </option>
                  ))}
                </select>
                <small>Choose the provider before saving.</small>
              </label>
            ) : null}

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
              {tab.key === 'history' && selectedSourceRuns.length > 0 ? (
                <span className="source-editor-tab-count">{selectedSourceRuns.length}</span>
              ) : null}
            </button>
          ))}
        </div>

        <div className="source-editor-tab-scroll">
        {activeTab === 'profile' ? (
          <section
            aria-labelledby="source-editor-tab-button-profile"
            className="panel source-editor-section source-editor-tab-panel source-editor-tab-panel-profile"
            id="source-editor-tab-profile"
            role="tabpanel"
          >
            <div className="source-editor-form-grid source-editor-form-grid-profile source-editor-profile-grid">
              <div className="source-editor-profile-pair">
                <label className="field source-editor-profile-handle-field">
                  <span>Handle</span>
                  {isEditMode && !handleUnlocked ? (
                    <div className="source-editor-static-field source-editor-profile-readonly-field source-editor-handle-locked is-locked">
                      <span className="source-editor-handle-locked-value" title={draft.handle}>{draft.handle}</span>
                      <button
                        type="button"
                        className="ghost-button source-editor-handle-edit-button"
                        onClick={() => setHandleUnlocked(true)}
                        title="Unlock to fix a renamed profile"
                        aria-label="Edit handle"
                      >
                        Edit
                      </button>
                    </div>
                  ) : (
                    <input
                      onChange={(event) => setDraft((current) => ({ ...current, handle: event.target.value }))}
                      placeholder="@handle or profile URL"
                      required
                      value={draft.handle}
                      autoFocus={handleUnlocked}
                    />
                  )}
                  {isEditMode ? (
                    handleUnlocked ? (
                      <small className="source-editor-field-hint">
                        Manual override — use only to fix a renamed profile. New media is saved under the new handle;
                        already-downloaded files stay in the old folder.
                      </small>
                    ) : (
                      <small className="source-editor-field-hint">
                        Locked for existing profiles. Use Edit to fix a rename.
                      </small>
                    )
                  ) : (
                    <small className="source-editor-field-hint source-editor-field-hint-empty" aria-hidden="true" />
                  )}
                </label>

                <label className="field source-editor-profile-name-field">
                  <span>Friendly name</span>
                  <input
                    onChange={(event) => setDraft((current) => ({ ...current, displayName: event.target.value }))}
                    placeholder="visual_lab"
                    value={draft.displayName}
                  />
                  <small className="source-editor-field-hint source-editor-field-hint-empty" aria-hidden="true" />
                </label>
              </div>

              <div className="source-editor-profile-pair">
                <label className="field source-editor-profile-group-field">
                  <span>Group</span>
                  <div className="source-editor-group-row source-editor-control-slot">
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
                  <small className="source-editor-field-hint source-editor-field-hint-empty" aria-hidden="true" />
                </label>

                <label className="field source-editor-profile-labels-field">
                  <span>Labels</span>
                  <div className="source-editor-label-input source-editor-control-slot">
                    {selectedLabels.map((label) => (
                      <button
                        aria-label={`Remove label ${label}`}
                        className="source-editor-label-chip"
                        key={label}
                        onClick={() => removeLabel(label)}
                        type="button"
                      >
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
                    <button
                      className="ghost-button source-editor-label-add"
                      disabled={normalizeLabel(labelDraft).length === 0}
                      onClick={() => commitLabelDraft()}
                      type="button"
                    >
                      Add
                    </button>
                  </div>
                  <datalist id={labelsSuggestionListId}>
                    {filteredLabelSuggestions.map((label) => (
                      <option key={label} value={label} />
                    ))}
                  </datalist>
                  <small className="source-editor-field-hint source-editor-field-hint-empty" aria-hidden="true" />
                </label>
              </div>

              {instagramSyncOptions ? (
                <label className="field source-editor-note-field">
                  <span>Profile note</span>
                  <textarea
                    ref={profileNoteRef}
                    onChange={(event) => {
                      updateInstagramSyncOptions((current) => ({ ...current, description: event.target.value }))
                      // Grow immediately before React re-renders description into effect.
                      requestAnimationFrame(() => fitProfileNoteHeight())
                    }}
                    onInput={() => fitProfileNoteHeight()}
                    placeholder="Optional profile note"
                    rows={3}
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
            className="panel source-editor-section source-editor-tab-panel source-editor-tab-panel-sync"
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
            <div className="source-editor-history-stack">
              {selectedSourceRuns.length > 0 ? (
                selectedSourceRuns.map((run) => {
                  const summaryPrimary = historySummaryPrimary(run.summary)
                  const summaryDetail = historySummaryDetail(run.summary, summaryPrimary)
                  return (
                    <article
                      className={`source-editor-history-card source-editor-history-${run.status}`}
                      key={run.id}
                    >
                      <span className={`status source-editor-history-status ${historyStatusClass(run.status)}`}>
                        {historyStatusLabel(run.status)}
                      </span>
                      <div className="source-editor-history-body">
                        <p className="source-editor-history-summary" title={run.summary}>
                          {summaryPrimary}
                        </p>
                        {summaryDetail ? (
                          <p className="source-editor-history-summary-detail">{summaryDetail}</p>
                        ) : null}
                        <div className="source-editor-history-meta">
                          <time className="source-editor-history-date" dateTime={run.finishedAt}>
                            {formatRunFinishedAt(run.finishedAt)}
                          </time>
                          <span className="source-editor-history-meta-data" title={run.tool}>{run.tool}</span>
                          <span className="source-editor-history-meta-data" title={run.trigger}>{run.trigger}</span>
                        </div>
                        {run.commandPreview ? (
                          <p className="source-editor-history-command" title={run.commandPreview}>
                            {run.commandPreview}
                          </p>
                        ) : null}
                      </div>
                    </article>
                  )
                })
              ) : (
                <div className="empty-state source-editor-history-empty">No sync history for this profile yet.</div>
              )}
            </div>
          </section>
        ) : null}
        </div>

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
            <button
              className="primary-button source-editor-save-button"
              disabled={Boolean(pendingCommand) || !canSubmit || (Boolean(draft.id) && !isDirty)}
              type="submit"
              title={draft.id && !isDirty ? 'No changes to save' : undefined}
            >
              {draft.id ? 'Save changes' : 'Create profile'}
            </button>
            {!draft.id ? (
              <button
                className="primary-button source-editor-save-button"
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
  if (status === 'succeeded') return 'status-succeeded source-editor-history-status-quiet'
  if (status === 'failed') return 'status-failed'
  return 'status-degraded'
}

function historyStatusLabel(status: string): string {
  if (status === 'succeeded') return 'Succeeded'
  if (status === 'failed') return 'Failed'
  if (!status) return 'Unknown'
  return status.charAt(0).toUpperCase() + status.slice(1)
}

function historySummaryPrimary(summary: string): string {
  const trimmed = summary.trim()
  if (!trimmed) return 'Sync finished'
  const sentenceEnd = trimmed.search(/[.!?]\s/)
  if (sentenceEnd > 0 && sentenceEnd < 120) {
    return trimmed.slice(0, sentenceEnd + 1)
  }
  if (trimmed.length <= 120) return trimmed
  return `${trimmed.slice(0, 117).trimEnd()}…`
}

function historySummaryDetail(summary: string, primary: string): string | undefined {
  const trimmed = summary.trim()
  if (!trimmed || trimmed === primary || trimmed.startsWith(primary.replace(/…$/, ''))) {
    const rest = trimmed.slice(primary.replace(/…$/, '').length).trim()
    return rest.length > 0 ? rest : undefined
  }
  return undefined
}

function formatRunFinishedAt(value: string): string {
  const parsed = new Date(value)
  if (Number.isNaN(parsed.getTime())) {
    return value
  }
  return parsed.toLocaleString(undefined, {
    year: 'numeric',
    month: 'numeric',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
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
