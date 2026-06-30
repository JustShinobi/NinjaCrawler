import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { DEFAULT_PROVIDER_CATALOG } from '../../domain/defaults'
import {
  DEFAULT_INSTAGRAM_PRESET_LABELS,
  INSTAGRAM_GLOBAL_PRESET_SETTING_KEYS,
  resolveInstagramGlobalSyncPreset,
  serializeInstagramGlobalSyncPreset,
} from '../../domain/sourceSyncOptions'
import type {
  InstagramPresetSlot,
  InstagramSourceSyncPreset,
  ProviderAccount,
  ProviderAccountCookie,
  ProviderAccountImportState,
  ProviderAccountSession,
  ProviderAccountUpsert,
  ProviderDescriptor,
  ProviderKey,
} from '../../domain/models'
import { useAppStore } from '../../state/appStore'
import { CookieEditorDialog } from './CookieEditorDialog'
import { ProviderAccountSettingsPanel } from './ProviderAccountSettingsPanel'
import {
  buildProviderAccountSettingsDraft,
  type ProviderAccountSettingsCategoryKey,
  serializeProviderAccountSettingsDraft,
} from './providerAccountSettings'

const EMPTY_ACCOUNTS: ProviderAccount[] = []
const EMPTY_ACCOUNT_SESSIONS: ProviderAccountSession[] = []

type AccountsTabKey = 'account' | 'defaults' | 'provider'

const ACCOUNTS_TABS: Array<{
  key: AccountsTabKey
  label: string
  categories?: ProviderAccountSettingsCategoryKey[]
}> = [
  { key: 'account', label: 'Account' },
  { key: 'defaults', label: 'Defaults', categories: ['defaults', 'extractVideo'] },
  { key: 'provider', label: 'Provider', categories: ['authorization', 'download', 'timers', 'errors', 'diagnostics'] },
]

function createDraft(providerCatalog: ProviderDescriptor[], provider?: ProviderKey): ProviderAccountUpsert {
  const descriptor = providerCatalog.find((entry) => entry.key === provider) ?? providerCatalog[0] ?? DEFAULT_PROVIDER_CATALOG[0]

  return {
    provider: descriptor.key,
    displayName: '',
    authMode: descriptor.authModes[0] ?? 'imported_session',
    authState: 'ready',
    capabilities: [...descriptor.defaultCapabilities],
  }
}

function mapAccountToDraft(account: ProviderAccount, providerCatalog: ProviderDescriptor[]): ProviderAccountUpsert {
  const descriptor = providerCatalog.find((entry) => entry.key === account.provider) ?? providerCatalog[0] ?? DEFAULT_PROVIDER_CATALOG[0]
  const authMode = descriptor.authModes.includes(account.authMode)
    ? account.authMode
    : descriptor.authModes[0] ?? 'imported_session'

  return {
    id: account.id,
    provider: account.provider,
    displayName: account.displayName,
    authMode,
    authState: account.authState,
    capabilities: [...account.capabilities],
    lastValidatedAt: account.lastValidatedAt,
  }
}

function formatProviderLabel(provider: ProviderKey, providerCatalog: ProviderDescriptor[]): string {
  return providerCatalog.find((descriptor) => descriptor.key === provider)?.displayName ?? provider
}

function describeSession(session?: ProviderAccountSession): string {
  if (!session || !session.hasSecret) {
    return 'No stored cookies.'
  }

  const parts = [`${session.cookieCount} cookies`]
  if (session.importedAt) {
    parts.push(`imported ${session.importedAt}`)
  }
  if (session.fingerprint) {
    parts.push(session.fingerprint)
  }

  return parts.join(' · ')
}

function resolveSavedAccount(
  accounts: ProviderAccount[],
  payload: ProviderAccountUpsert,
  knownIds: Set<string>,
): ProviderAccount | undefined {
  if (payload.id) {
    return accounts.find((account) => account.id === payload.id)
  }

  return accounts.find((account) => !knownIds.has(account.id))
}

function serializeDraftSignature(draft: ProviderAccountUpsert): string {
  return JSON.stringify({
    id: draft.id ?? null,
    provider: draft.provider,
    displayName: draft.displayName.trim(),
    authMode: draft.authMode,
    authState: draft.authState,
    capabilities: [...draft.capabilities].sort(),
    lastValidatedAt: draft.lastValidatedAt ?? null,
  })
}

function serializeSettingsSignature(settingsDraft: Record<string, string>): string {
  return JSON.stringify(
    Object.entries(settingsDraft)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, value]) => [key, value]),
  )
}

export interface AccountsPageProps {
  initialAccountId?: string
  initialProvider?: ProviderKey
  initialMode?: 'create' | 'edit'
  onDirtyChange?: (dirty: boolean) => void
}

export function AccountsPage({
  initialAccountId,
  initialProvider,
  initialMode,
  onDirtyChange,
}: AccountsPageProps = {}) {
  const snapshot = useAppStore((state) => state.snapshot)
  const pendingCommand = useAppStore((state) => state.pendingCommand)
  const storeError = useAppStore((state) => state.error)
  const upsertProviderAccount = useAppStore((state) => state.upsertProviderAccount)
  const clearProviderAccountCookies = useAppStore((state) => state.clearProviderAccountCookies)
  const saveProviderAccountCookies = useAppStore((state) => state.saveProviderAccountCookies)
  const loadProviderAccountEditor = useAppStore((state) => state.loadProviderAccountEditor)
  const saveProviderAccountSettings = useAppStore((state) => state.saveProviderAccountSettings)
  const upsertAppSetting = useAppStore((state) => state.upsertAppSetting)
  const validateProviderAccount = useAppStore((state) => state.validateProviderAccount)
  const revertProviderAccountImport = useAppStore((state) => state.revertProviderAccountImport)

  const accounts = snapshot?.accounts ?? EMPTY_ACCOUNTS
  const accountSessions = snapshot?.accountSessions ?? EMPTY_ACCOUNT_SESSIONS
  const providerCatalog = snapshot?.providerCatalog ?? DEFAULT_PROVIDER_CATALOG

  const [launchStateApplied, setLaunchStateApplied] = useState(false)
  const [mode, setMode] = useState<'create' | 'edit'>('create')
  const [activeTab, setActiveTab] = useState<AccountsTabKey>('account')
  const [selectedAccountId, setSelectedAccountId] = useState<string>()
  const [draft, setDraft] = useState<ProviderAccountUpsert>(() => createDraft(providerCatalog, initialProvider))
  const [settingsDraft, setSettingsDraft] = useState<Record<string, string>>(() => buildProviderAccountSettingsDraft(initialProvider ?? 'instagram', []))
  const [settingsLoading, setSettingsLoading] = useState(false)
  const [cookieDialogOpen, setCookieDialogOpen] = useState(false)
  const [savedDraftSignature, setSavedDraftSignature] = useState(() => serializeDraftSignature(createDraft(providerCatalog, initialProvider)))
  const [savedSettingsSignature, setSavedSettingsSignature] = useState(() => serializeSettingsSignature(buildProviderAccountSettingsDraft(initialProvider ?? 'instagram', [])))
  const [draftCookies, setDraftCookies] = useState<ProviderAccountCookie[]>([])
  const [globalPresetsDraft, setGlobalPresetsDraft] = useState<Record<InstagramPresetSlot, InstagramSourceSyncPreset>>(() => ({
    preset1: resolveInstagramGlobalSyncPreset(snapshot?.appSettings, 'preset1'),
    preset2: resolveInstagramGlobalSyncPreset(snapshot?.appSettings, 'preset2'),
  }))
  const [savingGlobalPreset, setSavingGlobalPreset] = useState<InstagramPresetSlot>()
  const [justSaved, setJustSaved] = useState(false)
  const [importState, setImportState] = useState<ProviderAccountImportState | null>(null)
  const settingsRequestRef = useRef(0)

  const selectedAccount = useMemo(
    () => accounts.find((account) => account.id === selectedAccountId),
    [accounts, selectedAccountId],
  )
  const selectedSession = useMemo(
    () => accountSessions.find((session) => session.accountId === selectedAccountId),
    [accountSessions, selectedAccountId],
  )
  const applySavedSignatures = useCallback((nextDraft: ProviderAccountUpsert, nextSettingsDraft: Record<string, string>) => {
    setSavedDraftSignature(serializeDraftSignature(nextDraft))
    setSavedSettingsSignature(serializeSettingsSignature(nextSettingsDraft))
  }, [])

  const resetSettingsDraft = useCallback((provider: ProviderKey) => {
    settingsRequestRef.current += 1
    setSettingsLoading(false)
    const nextDraft = buildProviderAccountSettingsDraft(provider, [])
    setSettingsDraft(nextDraft)
    return nextDraft
  }, [])

  const loadSettings = useCallback(async (accountId: string, provider: ProviderKey, baselineDraft: ProviderAccountUpsert) => {
    const requestId = settingsRequestRef.current + 1
    settingsRequestRef.current = requestId
    setSettingsLoading(true)

    try {
      const editor = await loadProviderAccountEditor(accountId)
      if (settingsRequestRef.current !== requestId) {
        return
      }

      const nextSettingsDraft = buildProviderAccountSettingsDraft(provider, editor.settings)
      setSettingsDraft(nextSettingsDraft)
      setImportState(editor.importState ?? null)
      applySavedSignatures(baselineDraft, nextSettingsDraft)
    } finally {
      if (settingsRequestRef.current === requestId) {
        setSettingsLoading(false)
      }
    }
  }, [applySavedSignatures, loadProviderAccountEditor])

  const openCreate = useCallback((provider?: ProviderKey) => {
    const nextDraft = createDraft(providerCatalog, provider)
    setMode('create')
    setActiveTab('account')
    setSelectedAccountId(undefined)
    setDraft(nextDraft)
    setDraftCookies([])
    setCookieDialogOpen(false)
    setImportState(null)
    const nextSettingsDraft = resetSettingsDraft(nextDraft.provider)
    applySavedSignatures(nextDraft, nextSettingsDraft)
  }, [applySavedSignatures, providerCatalog, resetSettingsDraft])

  const openEdit = useCallback((account: ProviderAccount) => {
    const nextDraft = mapAccountToDraft(account, providerCatalog)
    setMode('edit')
    setActiveTab('account')
    setSelectedAccountId(account.id)
    setDraft(nextDraft)
    setDraftCookies([])
    setCookieDialogOpen(false)
    setImportState(null)
    const defaultSettings = resetSettingsDraft(account.provider)
    applySavedSignatures(nextDraft, defaultSettings)
    void loadSettings(account.id, account.provider, nextDraft)
  }, [applySavedSignatures, loadSettings, providerCatalog, resetSettingsDraft])

  useEffect(() => {
    if (launchStateApplied) {
      return
    }

    if (initialAccountId) {
      const launchAccount = accounts.find((account) => account.id === initialAccountId)
      if (!launchAccount) {
        return
      }

      openEdit(launchAccount)
      setLaunchStateApplied(true)
      return
    }

    if (initialMode === 'create' || initialProvider) {
      openCreate(initialProvider)
      setLaunchStateApplied(true)
      return
    }

    if (accounts.length > 0) {
      openEdit(accounts[0])
    } else {
      openCreate(initialProvider)
    }

    setLaunchStateApplied(true)
  }, [accounts, initialAccountId, initialMode, initialProvider, launchStateApplied, openCreate, openEdit])

  useEffect(() => {
    if (mode !== 'edit' || !selectedAccountId) {
      return
    }

    const account = accounts.find((entry) => entry.id === selectedAccountId)
    if (!account) {
      openCreate(initialProvider)
    }
  }, [accounts, initialProvider, mode, openCreate, selectedAccountId])

  useEffect(() => {
    setGlobalPresetsDraft({
      preset1: resolveInstagramGlobalSyncPreset(snapshot?.appSettings, 'preset1'),
      preset2: resolveInstagramGlobalSyncPreset(snapshot?.appSettings, 'preset2'),
    })
  }, [snapshot?.appSettings])

  const handleProviderChange = useCallback((provider: ProviderKey) => {
    const descriptor = providerCatalog.find((entry) => entry.key === provider) ?? providerCatalog[0] ?? DEFAULT_PROVIDER_CATALOG[0]
    const nextDraft = {
      ...draft,
      id: undefined,
      provider,
      authMode: descriptor.authModes.includes(draft.authMode) ? draft.authMode : (descriptor.authModes[0] ?? 'imported_session'),
      capabilities: [...descriptor.defaultCapabilities],
      lastValidatedAt: undefined,
    }
    setDraft(nextDraft)
    setDraftCookies([])
    const nextSettingsDraft = resetSettingsDraft(provider)
    applySavedSignatures(nextDraft, nextSettingsDraft)
  }, [applySavedSignatures, draft, providerCatalog, resetSettingsDraft])

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setJustSaved(false)

    const knownIds = new Set(accounts.map((account) => account.id))
    const payload: ProviderAccountUpsert = {
      ...draft,
      displayName: draft.displayName.trim(),
      capabilities: [...draft.capabilities],
    }

    try {
      const savedSnapshot = await upsertProviderAccount(payload)
      const savedAccount = resolveSavedAccount(savedSnapshot.accounts, payload, knownIds)
      if (!savedAccount) {
        return
      }

      const serializedSettings = serializeProviderAccountSettingsDraft(savedAccount.provider, settingsDraft)
      let nextSettingsDraft = settingsDraft
      if (serializedSettings.length > 0) {
        const editor = await saveProviderAccountSettings(savedAccount.id, serializedSettings)
        nextSettingsDraft = buildProviderAccountSettingsDraft(savedAccount.provider, editor.settings)
        setSettingsDraft(nextSettingsDraft)
        setImportState(editor.importState ?? null)
      } else {
        nextSettingsDraft = resetSettingsDraft(savedAccount.provider)
      }

      if (draftCookies.length > 0) {
        await saveProviderAccountCookies(savedAccount.id, draftCookies)
        setDraftCookies([])
      }
      const nextDraft = mapAccountToDraft(savedAccount, providerCatalog)
      setMode('edit')
      setSelectedAccountId(savedAccount.id)
      setDraft(nextDraft)
      applySavedSignatures(nextDraft, nextSettingsDraft)
      setJustSaved(true)
    } catch {
      // O store já registrou a mensagem em `storeError`, exibida no rodapé.
    }
  }

  async function handleValidateAccount() {
    if (!selectedAccount) {
      return
    }

    const savedSnapshot = await validateProviderAccount(selectedAccount.id)
    const savedAccount = savedSnapshot.accounts.find((account) => account.id === selectedAccount.id)
    if (!savedAccount) {
      return
    }

    const nextDraft = mapAccountToDraft(savedAccount, providerCatalog)
    setDraft(nextDraft)
    setSelectedAccountId(savedAccount.id)
    setMode('edit')
    void loadSettings(savedAccount.id, savedAccount.provider, nextDraft)
  }

  async function handleClearCookies() {
    if (!selectedAccount) {
      setDraftCookies([])
      return
    }

    const savedSnapshot = await clearProviderAccountCookies(selectedAccount.id)
    const savedAccount = savedSnapshot.accounts.find((account) => account.id === selectedAccount.id)
    if (savedAccount) {
      const nextDraft = mapAccountToDraft(savedAccount, providerCatalog)
      setDraft(nextDraft)
      setSelectedAccountId(savedAccount.id)
      setMode('edit')
      void loadSettings(savedAccount.id, savedAccount.provider, nextDraft)
    }
  }

  async function handleRevertLastImport() {
    if (!selectedAccount || !importState?.canRevert) {
      return
    }
    if (!globalThis.confirm('Revert this account to the session from before the last Companion import?')) {
      return
    }

    const savedSnapshot = await revertProviderAccountImport(selectedAccount.id)
    const savedAccount = savedSnapshot.accounts.find((account) => account.id === selectedAccount.id)
    if (savedAccount) {
      const nextDraft = mapAccountToDraft(savedAccount, providerCatalog)
      setDraft(nextDraft)
      void loadSettings(savedAccount.id, savedAccount.provider, nextDraft)
    }
  }

  function handleResetChanges() {
    if (selectedAccount) {
      openEdit(selectedAccount)
      return
    }

    openCreate(draft.provider)
  }

  function updateGlobalPreset(
    slot: InstagramPresetSlot,
    mutate: (current: InstagramSourceSyncPreset) => InstagramSourceSyncPreset,
  ) {
    setGlobalPresetsDraft((current) => ({
      ...current,
      [slot]: mutate(current[slot]),
    }))
  }

  async function handleSaveGlobalPreset(slot: InstagramPresetSlot) {
    if (draft.provider !== 'instagram') {
      return
    }

    const preset = globalPresetsDraft[slot]
    setSavingGlobalPreset(slot)
    try {
      await upsertAppSetting({
        key: INSTAGRAM_GLOBAL_PRESET_SETTING_KEYS[slot],
        value: serializeInstagramGlobalSyncPreset(slot, preset),
        category: 'policy',
        description: slot === 'preset1' ? 'Instagram global sync preset 1' : 'Instagram global sync preset 2',
      })
    } finally {
      setSavingGlobalPreset(undefined)
    }
  }

  const isCreateMode = mode === 'create'
  const providerLocked = !isCreateMode || Boolean(initialProvider)
  const canSave = draft.displayName.trim().length > 0
  const providerLabel = formatProviderLabel(draft.provider, providerCatalog)
  const activeTabConfig = ACCOUNTS_TABS.find((tab) => tab.key === activeTab) ?? ACCOUNTS_TABS[0]
  const isDirty = useMemo(
    () =>
      serializeDraftSignature(draft) !== savedDraftSignature
      || serializeSettingsSignature(settingsDraft) !== savedSettingsSignature,
    [draft, savedDraftSignature, savedSettingsSignature, settingsDraft],
  )

  useEffect(() => {
    onDirtyChange?.(isDirty)
  }, [isDirty, onDirtyChange])

  // Some a confirmação "Saved" assim que o usuário volta a editar.
  useEffect(() => {
    if (isDirty && justSaved) {
      setJustSaved(false)
    }
  }, [isDirty, justSaved])

  useEffect(() => () => {
    onDirtyChange?.(false)
  }, [onDirtyChange])
  const displayedCookieCount = selectedAccount
    ? (selectedSession?.hasSecret ? selectedSession.cookieCount : 0)
    : draftCookies.length
  const sessionStatusText = selectedAccount
    ? describeSession(selectedSession)
    : draftCookies.length > 0
      ? `${draftCookies.length} draft cookie${draftCookies.length === 1 ? '' : 's'} ready to save with the new account.`
      : 'Add cookies now or save the account first.'
  const sessionPillLabel = selectedAccount
    ? (selectedSession?.hasSecret ? 'Stored session' : 'No session')
    : (draftCookies.length > 0 ? 'Draft cookies' : 'No session')

  return (
    <div className="accounts-shell">
      <section className="panel panel-accent accounts-panel accounts-summary-strip">
        <div className="accounts-hero-metrics" role="list" aria-label="Account summary">
          <article className="accounts-hero-metric" role="listitem">
            <span>Account</span>
            <strong>{draft.displayName || providerLabel}</strong>
            <p>{isCreateMode ? 'New account draft' : 'Existing account configuration'}</p>
          </article>
          <article className="accounts-hero-metric" role="listitem">
            <span>Provider</span>
            <strong>{providerLabel}</strong>
            <p>{providerLocked ? 'Provider locked by current context.' : 'Provider can still be changed before save.'}</p>
          </article>
          <article className="accounts-hero-metric" role="listitem">
            <span>Session</span>
            <strong>{displayedCookieCount > 0 ? `${displayedCookieCount} cookies` : 'No cookies'}</strong>
            <p>{sessionStatusText}</p>
          </article>
          <article className="accounts-hero-metric" role="listitem">
            <span>State</span>
            <strong>{selectedAccount?.authState ?? 'Draft'}</strong>
            <p>{selectedAccount?.lastValidatedAt ?? 'Not validated yet'}</p>
          </article>
        </div>
      </section>

      <form className="accounts-workbench" id="accounts-config-form" onSubmit={handleSubmit}>
        <div className="source-editor-tab-bar" role="tablist" aria-label="Account editor tabs">
          {ACCOUNTS_TABS.map((tab) => (
            <button
              aria-controls={`accounts-tab-${tab.key}`}
              aria-selected={activeTab === tab.key}
              className={activeTab === tab.key ? 'source-editor-tab source-editor-tab-active' : 'source-editor-tab'}
              id={`accounts-tab-button-${tab.key}`}
              key={tab.key}
              onClick={() => setActiveTab(tab.key)}
              role="tab"
              type="button"
            >
              <span>{tab.label}</span>
            </button>
          ))}
        </div>

        {activeTab === 'account' ? (
          <section
            aria-labelledby="accounts-tab-button-account"
            className="panel panel-accent source-editor-tab-panel source-editor-tab-panel-profile accounts-panel"
            id="accounts-tab-account"
            role="tabpanel"
          >
            <div className="panel-header compact-header">
              <div>
                <p className="eyebrow">{activeTabConfig.label}</p>
                <h2>Identity</h2>
              </div>
            </div>

            <div className="form-grid">
              <label className="field">
                <span>Provider</span>
                {providerLocked ? (
                  <div className="accounts-static-field">{providerLabel}</div>
                ) : (
                  <select onChange={(event) => handleProviderChange(event.target.value as ProviderKey)} value={draft.provider}>
                    {providerCatalog.map((descriptor) => (
                      <option key={descriptor.key} value={descriptor.key}>
                        {descriptor.displayName}
                      </option>
                    ))}
                  </select>
                )}
              </label>

              <label className="field">
                <span>Display name</span>
                <input
                  onChange={(event) => setDraft((current) => ({ ...current, displayName: event.target.value }))}
                  placeholder="Instagram Main"
                  required
                  value={draft.displayName}
                />
              </label>
            </div>

            <ProviderAccountSettingsPanel
              draft={settingsDraft}
              loading={settingsLoading}
              onFieldChange={(key, value) =>
                setSettingsDraft((current) => ({
                  ...current,
                  [key]: value,
                }))
              }
              provider={draft.provider}
              visibleCategories={['account']}
            />

            <section className="panel accounts-panel">
              <div className="panel-header compact-header">
                <div>
                  <p className="eyebrow">Session</p>
                  <h2>Cookies</h2>
                </div>
                <span className="pill">{sessionPillLabel}</span>
              </div>

              <div className="stat-grid compact-grid">
                <article className="stat-card">
                  <span>Stored cookies</span>
                  <strong>{displayedCookieCount}</strong>
                  <small>
                    {selectedAccount
                      ? (selectedSession?.fingerprint ?? 'No stored cookie fingerprint yet.')
                      : (draftCookies.length > 0 ? 'Draft cookies will be persisted after account creation.' : 'No draft cookie fingerprint yet.')}
                  </small>
                </article>
                <article className="stat-card muted-card">
                  <span>Validation</span>
                  <strong>{selectedSession?.lastValidationError ? 'Review' : 'Ready'}</strong>
                  <small>{selectedSession?.lastValidationError ?? 'Cookies and manual auth fields validate together.'}</small>
                </article>
              </div>

              <div className="action-row">
                <button className="ghost-button" disabled={Boolean(pendingCommand)} onClick={() => setCookieDialogOpen(true)} type="button">
                  Edit cookies
                </button>
                <button
                  className="danger-button"
                  disabled={
                    Boolean(pendingCommand)
                    || (selectedAccount ? !selectedSession?.hasSecret : draftCookies.length === 0)
                  }
                  onClick={() => void handleClearCookies()}
                  type="button"
                >
                  Clear cookies
                </button>
              </div>
            </section>

            {selectedAccount && importState ? (
              <section className="panel accounts-panel">
                <div className="panel-header compact-header">
                  <div>
                    <p className="eyebrow">Companion</p>
                    <h2>Browser import</h2>
                  </div>
                  <span className="pill">{importState.providerUsername ? `@${importState.providerUsername}` : 'Imported'}</span>
                </div>
                <p className="source-editor-note">
                  Last import {importState.lastImportedAt}.
                  {importState.backupImportedAt ? ` Previous session from ${importState.backupImportedAt} is available.` : ' No previous session is available.'}
                </p>
                <div className="action-row">
                  <button
                    className="ghost-button"
                    disabled={Boolean(pendingCommand) || !importState.canRevert}
                    onClick={() => void handleRevertLastImport()}
                    type="button"
                  >
                    Revert last import
                  </button>
                </div>
              </section>
            ) : null}
          </section>
        ) : null}

        {activeTab === 'defaults' ? (
          <section
            aria-labelledby="accounts-tab-button-defaults"
            className="panel panel-accent source-editor-tab-panel source-editor-tab-panel-sync accounts-panel"
            id="accounts-tab-defaults"
            role="tabpanel"
          >
            <div className="panel-header compact-header">
              <div>
                <p className="eyebrow">{activeTabConfig.label}</p>
                <h2>New profile defaults</h2>
              </div>
            </div>

            <ProviderAccountSettingsPanel
              draft={settingsDraft}
              loading={settingsLoading}
              onFieldChange={(key, value) =>
                setSettingsDraft((current) => ({
                  ...current,
                  [key]: value,
                }))
              }
              provider={draft.provider}
              visibleCategories={activeTabConfig.categories}
            />
          </section>
        ) : null}

        {activeTab === 'provider' ? (
          <section
            aria-labelledby="accounts-tab-button-provider"
            className="panel panel-accent source-editor-tab-panel source-editor-tab-panel-sync accounts-panel"
            id="accounts-tab-provider"
            role="tabpanel"
          >
            <div className="panel-header compact-header">
              <div>
                <p className="eyebrow">{activeTabConfig.label}</p>
                <h2>Provider runtime</h2>
              </div>
            </div>

            <ProviderAccountSettingsPanel
              draft={settingsDraft}
              loading={settingsLoading}
              onFieldChange={(key, value) =>
                setSettingsDraft((current) => ({
                  ...current,
                  [key]: value,
                }))
              }
              provider={draft.provider}
              visibleCategories={activeTabConfig.categories}
            />
            <div className="panel section-stack">
              <div className="panel-header compact-header">
                <div>
                  <p className="eyebrow">Provider presets</p>
                  <h2>Global sync presets</h2>
                </div>
              </div>
              {draft.provider !== 'instagram' ? (
                <p className="source-editor-note">Global presets are currently available for Instagram only.</p>
              ) : (
                <div className="section-stack">
                  {(Object.keys(globalPresetsDraft) as InstagramPresetSlot[]).map((slot) => {
                    const preset = globalPresetsDraft[slot]
                    const disabled = savingGlobalPreset === slot
                    const presetLabel = DEFAULT_INSTAGRAM_PRESET_LABELS[slot]
                    return (
                      <article className="panel" key={slot}>
                        <div className="panel-header compact-header">
                          <div>
                            <p className="eyebrow">{presetLabel}</p>
                            <h2>{preset.label.trim() || presetLabel}</h2>
                          </div>
                          <button
                            className="ghost-button"
                            disabled={disabled}
                            onClick={() => void handleSaveGlobalPreset(slot)}
                            type="button"
                          >
                            {disabled ? 'Saving...' : 'Save preset'}
                          </button>
                        </div>
                        <div className="source-editor-setting-list">
                          <div className="source-editor-setting-row">
                            <div className="source-editor-setting-copy">
                              <label htmlFor={`${slot}-enabled`}>Enabled</label>
                            </div>
                            <input
                              checked={preset.enabled}
                              id={`${slot}-enabled`}
                              onChange={(event) => updateGlobalPreset(slot, (current) => ({
                                ...current,
                                enabled: event.target.checked,
                              }))}
                              type="checkbox"
                            />
                          </div>
                          <label className="field">
                            <span>Label</span>
                            <input
                              onChange={(event) => updateGlobalPreset(slot, (current) => ({
                                ...current,
                                label: event.target.value,
                              }))}
                              type="text"
                              value={preset.label}
                            />
                          </label>
                          <div className="accounts-toggle-grid">
                            {([
                              ['timeline', 'Timeline'],
                              ['reels', 'Reels'],
                              ['stories', 'Stories'],
                              ['storiesUser', 'Stories (user)'],
                              ['tagged', 'Tagged'],
                            ] as Array<[keyof InstagramSourceSyncPreset['sections'], string]>).map(([sectionKey, sectionLabel]) => (
                              <label className="checkbox-inline" key={`${slot}-${sectionKey}`}>
                                <input
                                  checked={preset.sections[sectionKey]}
                                  onChange={(event) => updateGlobalPreset(slot, (current) => ({
                                    ...current,
                                    sections: {
                                      ...current.sections,
                                      [sectionKey]: event.target.checked,
                                    },
                                  }))}
                                  type="checkbox"
                                />
                                <span>{sectionLabel}</span>
                              </label>
                            ))}
                          </div>
                        </div>
                      </article>
                    )
                  })}
                </div>
              )}
            </div>
          </section>
        ) : null}

        <footer className="source-editor-footer">
          {storeError ? (
            <p className="source-editor-submit-error" role="alert">
              {storeError}
            </p>
          ) : null}
          <div className="action-row">
            {isDirty ? (
              <div className="source-editor-dirty-indicator">
                <span aria-hidden className="source-editor-dirty-indicator-dot" />
                <span>Unsaved changes</span>
              </div>
            ) : justSaved ? (
              <div className="accounts-saved-indicator" role="status">
                <span aria-hidden>✓</span>
                <span>All changes saved</span>
              </div>
            ) : null}
            <button className="ghost-button" disabled={Boolean(pendingCommand) || !isDirty} onClick={handleResetChanges} type="button">
              Reset changes
            </button>
            {selectedAccount ? (
              <button className="ghost-button" disabled={Boolean(pendingCommand)} onClick={() => void handleValidateAccount()} type="button">
                Validate account
              </button>
            ) : null}
            <button className="primary-button" disabled={Boolean(pendingCommand) || !canSave} type="submit">
              {isCreateMode ? 'Create account' : 'Save changes'}
            </button>
          </div>
        </footer>
      </form>

      {cookieDialogOpen ? (
        <CookieEditorDialog
          accountId={selectedAccount?.id}
          initialCookies={selectedAccount ? undefined : draftCookies}
          onClose={() => setCookieDialogOpen(false)}
          onSaveDraftCookies={(cookies) => setDraftCookies(cookies)}
          provider={selectedAccount?.provider ?? draft.provider}
          providerLabel={formatProviderLabel(selectedAccount?.provider ?? draft.provider, providerCatalog)}
        />
      ) : null}
    </div>
  )
}
