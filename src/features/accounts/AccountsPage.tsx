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
import { formatAuthState } from './formatAuthState'
import { ProviderAccountSettingsPanel } from './ProviderAccountSettingsPanel'
import {
  buildProviderAccountSettingsDraft,
  type ProviderAccountSettingsCategoryKey,
  serializeProviderAccountSettingsDraft,
} from './providerAccountSettings'
import { WorkspacePolicyPanel } from './WorkspacePolicyPanel'

const EMPTY_ACCOUNTS: ProviderAccount[] = []
const EMPTY_ACCOUNT_SESSIONS: ProviderAccountSession[] = []

type AccountsTabKey = 'account' | 'defaults' | 'provider' | 'workspace'

type PendingNavigation =
  | { kind: 'edit'; account: ProviderAccount }
  | { kind: 'create'; provider?: ProviderKey }

const ACCOUNTS_TABS: Array<{
  key: AccountsTabKey
  label: string
  categories?: ProviderAccountSettingsCategoryKey[]
  /** Workspace policy is global — available even with no account selected. */
  always?: boolean
}> = [
  { key: 'account', label: 'Account' },
  { key: 'defaults', label: 'Defaults', categories: ['defaults', 'extractVideo'] },
  { key: 'provider', label: 'Provider', categories: ['authorization', 'download', 'timers', 'errors'] },
  { key: 'workspace', label: 'Workspace', always: true },
]

const PRESET_SCOPE_OPTIONS: Array<{ key: keyof InstagramSourceSyncPreset['sections']; label: string }> = [
  { key: 'timeline', label: 'Timeline' },
  { key: 'reels', label: 'Reels' },
  { key: 'stories', label: 'Stories' },
  { key: 'storiesUser', label: 'Stories (user)' },
  { key: 'tagged', label: 'Tagged' },
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

function formatShortDateTime(value: string | undefined): string {
  if (!value) {
    return ''
  }
  const ms = Date.parse(value)
  if (Number.isNaN(ms)) {
    return value
  }
  try {
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'medium',
      timeStyle: 'short',
    }).format(new Date(ms))
  } catch {
    return value
  }
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

function serializeCookiesSignature(cookies: ProviderAccountCookie[]): string {
  return JSON.stringify(cookies)
}

function serializePresetsSignature(presets: Record<InstagramPresetSlot, InstagramSourceSyncPreset>): string {
  return JSON.stringify(presets)
}

function buildPresetsFromSettings(appSettings: Parameters<typeof resolveInstagramGlobalSyncPreset>[0]): Record<InstagramPresetSlot, InstagramSourceSyncPreset> {
  return {
    preset1: resolveInstagramGlobalSyncPreset(appSettings, 'preset1'),
    preset2: resolveInstagramGlobalSyncPreset(appSettings, 'preset2'),
  }
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
  const initialPresets = buildPresetsFromSettings(snapshot?.appSettings)

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
  const [savedCookiesSignature, setSavedCookiesSignature] = useState(() => serializeCookiesSignature([]))
  const [globalPresetsDraft, setGlobalPresetsDraft] = useState(initialPresets)
  const [savedPresetsSignature, setSavedPresetsSignature] = useState(() => serializePresetsSignature(initialPresets))
  const [savingGlobalPreset, setSavingGlobalPreset] = useState<InstagramPresetSlot>()
  const [justSaved, setJustSaved] = useState(false)
  const [importState, setImportState] = useState<ProviderAccountImportState | null>(null)
  const [pendingNavigation, setPendingNavigation] = useState<PendingNavigation | null>(null)
  const [fingerprintCopied, setFingerprintCopied] = useState(false)
  const settingsRequestRef = useRef(0)
  const isDirtyRef = useRef(false)

  const selectedAccount = useMemo(
    () => accounts.find((account) => account.id === selectedAccountId),
    [accounts, selectedAccountId],
  )
  const selectedSession = useMemo(
    () => accountSessions.find((session) => session.accountId === selectedAccountId),
    [accountSessions, selectedAccountId],
  )

  const applySavedSignatures = useCallback((
    nextDraft: ProviderAccountUpsert,
    nextSettingsDraft: Record<string, string>,
    nextCookies: ProviderAccountCookie[] = [],
  ) => {
    setSavedDraftSignature(serializeDraftSignature(nextDraft))
    setSavedSettingsSignature(serializeSettingsSignature(nextSettingsDraft))
    setSavedCookiesSignature(serializeCookiesSignature(nextCookies))
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
      applySavedSignatures(baselineDraft, nextSettingsDraft, [])
    } finally {
      if (settingsRequestRef.current === requestId) {
        setSettingsLoading(false)
      }
    }
  }, [applySavedSignatures, loadProviderAccountEditor])

  const openCreate = useCallback((provider?: ProviderKey, options?: { resetTab?: boolean }) => {
    const nextDraft = createDraft(providerCatalog, provider)
    setMode('create')
    if (options?.resetTab !== false) {
      setActiveTab('account')
    }
    setSelectedAccountId(undefined)
    setDraft(nextDraft)
    setDraftCookies([])
    setCookieDialogOpen(false)
    setImportState(null)
    setJustSaved(false)
    setPendingNavigation(null)
    const nextSettingsDraft = resetSettingsDraft(nextDraft.provider)
    applySavedSignatures(nextDraft, nextSettingsDraft, [])
  }, [applySavedSignatures, providerCatalog, resetSettingsDraft])

  const openEdit = useCallback((account: ProviderAccount, options?: { resetTab?: boolean }) => {
    const nextDraft = mapAccountToDraft(account, providerCatalog)
    setMode('edit')
    if (options?.resetTab) {
      setActiveTab('account')
    }
    setSelectedAccountId(account.id)
    setDraft(nextDraft)
    setDraftCookies([])
    setCookieDialogOpen(false)
    setImportState(null)
    setJustSaved(false)
    setPendingNavigation(null)
    const defaultSettings = resetSettingsDraft(account.provider)
    applySavedSignatures(nextDraft, defaultSettings, [])
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

      openEdit(launchAccount, { resetTab: true })
      setLaunchStateApplied(true)
      return
    }

    if (initialMode === 'create' || initialProvider) {
      openCreate(initialProvider, { resetTab: true })
      setLaunchStateApplied(true)
      return
    }

    if (accounts.length > 0) {
      openEdit(accounts[0], { resetTab: true })
    } else {
      openCreate(initialProvider, { resetTab: true })
    }

    setLaunchStateApplied(true)
  }, [accounts, initialAccountId, initialMode, initialProvider, launchStateApplied, openCreate, openEdit])

  useEffect(() => {
    if (mode !== 'edit' || !selectedAccountId) {
      return
    }

    const account = accounts.find((entry) => entry.id === selectedAccountId)
    if (!account) {
      openCreate(initialProvider, { resetTab: true })
    }
  }, [accounts, initialProvider, mode, openCreate, selectedAccountId])

  useEffect(() => {
    const next = buildPresetsFromSettings(snapshot?.appSettings)
    setGlobalPresetsDraft(next)
    setSavedPresetsSignature(serializePresetsSignature(next))
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
    setImportState(null)
    const nextSettingsDraft = resetSettingsDraft(provider)
    applySavedSignatures(nextDraft, nextSettingsDraft, [])
  }, [applySavedSignatures, draft, providerCatalog, resetSettingsDraft])

  const isCreateMode = mode === 'create'
  const providerLocked = !isCreateMode || Boolean(initialProvider)
  const canSaveName = draft.displayName.trim().length > 0
  const providerLabel = formatProviderLabel(draft.provider, providerCatalog)

  const accountDirty = useMemo(
    () =>
      serializeDraftSignature(draft) !== savedDraftSignature
      || serializeSettingsSignature(settingsDraft) !== savedSettingsSignature
      || serializeCookiesSignature(draftCookies) !== savedCookiesSignature,
    [draft, draftCookies, savedCookiesSignature, savedDraftSignature, savedSettingsSignature, settingsDraft],
  )

  const presetsDirty = useMemo(
    () => draft.provider === 'instagram' && serializePresetsSignature(globalPresetsDraft) !== savedPresetsSignature,
    [draft.provider, globalPresetsDraft, savedPresetsSignature],
  )

  const isDirty = accountDirty || presetsDirty

  useEffect(() => {
    isDirtyRef.current = isDirty
    onDirtyChange?.(isDirty)
  }, [isDirty, onDirtyChange])

  useEffect(() => {
    if (isDirty && justSaved) {
      setJustSaved(false)
    }
  }, [isDirty, justSaved])

  useEffect(() => () => {
    onDirtyChange?.(false)
  }, [onDirtyChange])

  async function saveDirtyPresets(): Promise<void> {
    if (draft.provider !== 'instagram' || !presetsDirty) {
      return
    }

    for (const slot of Object.keys(globalPresetsDraft) as InstagramPresetSlot[]) {
      const preset = globalPresetsDraft[slot]
      await upsertAppSetting({
        key: INSTAGRAM_GLOBAL_PRESET_SETTING_KEYS[slot],
        value: serializeInstagramGlobalSyncPreset(slot, preset),
        category: 'policy',
        description: slot === 'preset1' ? 'Instagram global sync preset 1' : 'Instagram global sync preset 2',
      })
    }
    setSavedPresetsSignature(serializePresetsSignature(globalPresetsDraft))
  }

  async function persistAccount(): Promise<boolean> {
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
        return false
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

      await saveDirtyPresets()

      const nextDraft = mapAccountToDraft(savedAccount, providerCatalog)
      setMode('edit')
      setSelectedAccountId(savedAccount.id)
      setDraft(nextDraft)
      applySavedSignatures(nextDraft, nextSettingsDraft, [])
      setJustSaved(true)
      return true
    } catch {
      return false
    }
  }

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (!canSaveName) {
      return
    }
    if (!isCreateMode && !isDirty) {
      return
    }
    await persistAccount()
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
      applySavedSignatures(draft, settingsDraft, [])
      return
    }

    if (typeof window !== 'undefined' && typeof window.confirm === 'function') {
      if (!window.confirm('Clear all stored cookies for this account?')) {
        return
      }
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
      setSelectedAccountId(savedAccount.id)
      setMode('edit')
      void loadSettings(savedAccount.id, savedAccount.provider, nextDraft)
    }
  }

  function handleResetChanges() {
    if (presetsDirty) {
      setGlobalPresetsDraft(buildPresetsFromSettings(snapshot?.appSettings))
      setSavedPresetsSignature(serializePresetsSignature(buildPresetsFromSettings(snapshot?.appSettings)))
    }
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
      setSavedPresetsSignature((currentSig) => {
        try {
          const previous = JSON.parse(currentSig) as Record<InstagramPresetSlot, InstagramSourceSyncPreset>
          return serializePresetsSignature({ ...previous, [slot]: preset })
        } catch {
          return serializePresetsSignature({ ...globalPresetsDraft, [slot]: preset })
        }
      })
    } finally {
      setSavingGlobalPreset(undefined)
    }
  }

  const displayedCookieCount = selectedAccount
    ? (selectedSession?.hasSecret ? selectedSession.cookieCount : 0)
    : draftCookies.length
  const sessionStatusText = selectedAccount
    ? selectedSession?.hasSecret
      ? `${selectedSession.cookieCount} cookies${selectedSession.importedAt ? ` · imported ${formatShortDateTime(selectedSession.importedAt)}` : ''}`
      : 'No stored cookies.'
    : draftCookies.length > 0
      ? `${draftCookies.length} draft cookies ready to save with the new account.`
      : 'Add cookies now or save the account first.'
  const sessionPillLabel = selectedAccount
    ? (selectedSession?.hasSecret ? 'Stored session' : 'No session')
    : (draftCookies.length > 0 ? 'Draft cookies' : 'No session')
  const identityTitle = isCreateMode
    ? (draft.displayName.trim() || 'New account draft')
    : (draft.displayName.trim() || providerLabel)
  const stateLabel = formatAuthState(selectedAccount?.authState ?? (isCreateMode ? 'draft' : draft.authState))
  const validatedMeta = selectedAccount?.lastValidatedAt
    ? `Validated ${formatShortDateTime(selectedAccount.lastValidatedAt)}`
    : 'Not validated yet'

  const accountsByProvider = useMemo(() => {
    return providerCatalog.map((descriptor) => ({
      descriptor,
      accounts: accounts.filter((account) => account.provider === descriptor.key),
    }))
  }, [accounts, providerCatalog])

  const showEmptyOnboarding = accounts.length === 0 && isCreateMode

  function requestNavigation(next: PendingNavigation) {
    if (!isDirtyRef.current) {
      if (next.kind === 'edit') {
        openEdit(next.account)
      } else {
        openCreate(next.provider, { resetTab: true })
      }
      return
    }
    setPendingNavigation(next)
  }

  function handleSelectAccount(account: ProviderAccount) {
    if (account.id === selectedAccountId && mode === 'edit') {
      return
    }
    requestNavigation({ kind: 'edit', account })
  }

  function handleNewAccount(provider?: ProviderKey) {
    requestNavigation({ kind: 'create', provider: provider ?? draft.provider })
  }

  async function handleDirtyDialogAction(action: 'cancel' | 'discard' | 'save') {
    if (!pendingNavigation) {
      return
    }
    if (action === 'cancel') {
      setPendingNavigation(null)
      return
    }
    if (action === 'save') {
      if (!canSaveName) {
        return
      }
      const ok = await persistAccount()
      if (!ok) {
        return
      }
    }
    const next = pendingNavigation
    setPendingNavigation(null)
    if (next.kind === 'edit') {
      openEdit(next.account)
    } else {
      openCreate(next.provider, { resetTab: true })
    }
  }

  async function copyFingerprint(value: string) {
    try {
      await navigator.clipboard.writeText(value)
      setFingerprintCopied(true)
      window.setTimeout(() => setFingerprintCopied(false), 1500)
    } catch {
      // Clipboard may be unavailable in some test / restricted contexts.
    }
  }

  const saveDisabled = Boolean(pendingCommand) || !canSaveName || (!isCreateMode && !isDirty)
  const saveTitle = !canSaveName
    ? 'Display name is required'
    : (!isCreateMode && !isDirty ? 'No changes to save' : undefined)

  return (
    <div className="accounts-shell">
      <aside className="accounts-sidebar" aria-label="Accounts list">
        <div className="accounts-sidebar-toolbar">
          <button
            className={isCreateMode ? 'accounts-sidebar-new accounts-sidebar-new-active' : 'accounts-sidebar-new'}
            disabled={Boolean(pendingCommand)}
            onClick={() => handleNewAccount()}
            type="button"
          >
            <span aria-hidden className="accounts-sidebar-new-icon">+</span>
            <span>New account</span>
          </button>
        </div>

        <div className="accounts-provider-groups">
          {accountsByProvider.map(({ descriptor, accounts: groupAccounts }) => {
            const providerActive = draft.provider === descriptor.key
            return (
              <section
                className={providerActive ? 'accounts-provider-group accounts-provider-group-active' : 'accounts-provider-group'}
                key={descriptor.key}
              >
                <div className="accounts-provider-header">
                  <span className="accounts-provider-heading">
                    {descriptor.displayName}
                    <span className="accounts-provider-count" aria-label={`${groupAccounts.length} accounts`}>
                      {groupAccounts.length}
                    </span>
                  </span>
                  <button
                    aria-label={`Add ${descriptor.displayName} account`}
                    className="accounts-provider-add"
                    disabled={Boolean(pendingCommand)}
                    onClick={() => handleNewAccount(descriptor.key)}
                    title={`Add ${descriptor.displayName} account`}
                    type="button"
                  >
                    <span aria-hidden>+</span>
                  </button>
                </div>
                {groupAccounts.length === 0 ? (
                  <p className="accounts-provider-empty">No accounts yet</p>
                ) : (
                  <ul className="accounts-entity-list">
                    {groupAccounts.map((account) => {
                      const active = mode === 'edit' && account.id === selectedAccountId
                      const showState = account.authState !== 'ready'
                      return (
                        <li key={account.id}>
                          <button
                            aria-current={active ? 'true' : undefined}
                            className={active ? 'accounts-entity-row accounts-entity-row-active' : 'accounts-entity-row'}
                            onClick={() => handleSelectAccount(account)}
                            type="button"
                          >
                            <span className="accounts-entity-name">{account.displayName}</span>
                            {showState ? (
                              <span className="accounts-entity-state">{formatAuthState(account.authState)}</span>
                            ) : (
                              <span className="accounts-entity-dot accounts-entity-dot-ready" title="Ready" aria-label="Ready" />
                            )}
                          </button>
                        </li>
                      )
                    })}
                  </ul>
                )}
              </section>
            )
          })}
        </div>
      </aside>

      <div className="accounts-workbench">
        {showEmptyOnboarding ? (
          <section className="accounts-empty-onboarding" aria-label="Get started">
            <h2>Add your first account</h2>
            <p>
              Choose a provider, set a display name and media path, then import cookies from the browser Companion
              or paste them manually. Validate when you are ready.
            </p>
            <div className="accounts-empty-provider-actions">
              {providerCatalog.map((descriptor) => (
                <button
                  className="ghost-button"
                  key={descriptor.key}
                  onClick={() => handleNewAccount(descriptor.key)}
                  type="button"
                >
                  {descriptor.displayName}
                </button>
              ))}
            </div>
          </section>
        ) : null}

        <section className="accounts-identity-strip" aria-label="Account summary">
          <div className="accounts-identity-main">
            <h2 className="accounts-identity-title">{identityTitle}</h2>
            <p className="accounts-identity-meta">
              {isCreateMode && !providerLocked ? (
                <label className="accounts-identity-provider-field">
                  <span className="visually-hidden">Provider</span>
                  <select
                    aria-label="Provider"
                    onChange={(event) => handleProviderChange(event.target.value as ProviderKey)}
                    value={draft.provider}
                  >
                    {providerCatalog.map((descriptor) => (
                      <option key={descriptor.key} value={descriptor.key}>
                        {descriptor.displayName}
                      </option>
                    ))}
                  </select>
                </label>
              ) : (
                <span className="accounts-identity-badge">{providerLabel}</span>
              )}
              <span className="accounts-identity-sep" aria-hidden>·</span>
              <span>{sessionStatusText}</span>
              <span className="accounts-identity-sep" aria-hidden>·</span>
              <span>{validatedMeta}</span>
            </p>
          </div>
          <div className="accounts-identity-status">
            <span className={stateLabel.toLowerCase() === 'ready' ? 'pill pill-ready' : 'pill'}>{stateLabel}</span>
            {sessionPillLabel !== 'Stored session' || !selectedAccount ? (
              <span className="pill accounts-session-pill">{sessionPillLabel}</span>
            ) : null}
          </div>
        </section>

        <div className="source-editor-tab-bar" role="tablist" aria-label="Accounts editor tabs">
          {ACCOUNTS_TABS.map((tab, index) => (
            <button
              aria-controls={`accounts-tab-${tab.key}`}
              aria-selected={activeTab === tab.key}
              className={activeTab === tab.key ? 'source-editor-tab source-editor-tab-active' : 'source-editor-tab'}
              id={`accounts-tab-button-${tab.key}`}
              key={tab.key}
              onClick={() => setActiveTab(tab.key)}
              onKeyDown={(event) => {
                if (event.key !== 'ArrowRight' && event.key !== 'ArrowLeft' && event.key !== 'Home' && event.key !== 'End') {
                  return
                }
                event.preventDefault()
                const last = ACCOUNTS_TABS.length - 1
                let nextIndex = index
                if (event.key === 'ArrowRight') {
                  nextIndex = index === last ? 0 : index + 1
                } else if (event.key === 'ArrowLeft') {
                  nextIndex = index === 0 ? last : index - 1
                } else if (event.key === 'Home') {
                  nextIndex = 0
                } else {
                  nextIndex = last
                }
                const nextTab = ACCOUNTS_TABS[nextIndex]
                if (!nextTab) {
                  return
                }
                setActiveTab(nextTab.key)
                requestAnimationFrame(() => {
                  document.getElementById(`accounts-tab-button-${nextTab.key}`)?.focus()
                })
              }}
              role="tab"
              tabIndex={activeTab === tab.key ? 0 : -1}
              type="button"
            >
              <span>{tab.label}</span>
            </button>
          ))}
        </div>

        <form className="accounts-tab-scroll" id="accounts-config-form" onSubmit={handleSubmit}>
          {activeTab === 'account' ? (
            <section
              aria-labelledby="accounts-tab-button-account"
              className="accounts-tab-panel"
              id="accounts-tab-account"
              role="tabpanel"
            >
              <section className="accounts-section accounts-section-compact">
                <header className="accounts-section-header">
                  <h3>Identity</h3>
                </header>
                <div className="form-grid accounts-settings-grid accounts-settings-grid-single">
                  <label className="field field-full">
                    <span>Display name</span>
                    <input
                      onChange={(event) => setDraft((current) => ({ ...current, displayName: event.target.value }))}
                      placeholder="Instagram Main"
                      required
                      value={draft.displayName}
                    />
                  </label>
                </div>
              </section>

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

              <section className="accounts-section">
                <header className="accounts-section-header">
                  <h3>Cookies</h3>
                </header>

                <div className="accounts-session-summary accounts-session-summary-single">
                  <div>
                    <span className="accounts-session-label">Stored cookies</span>
                    <strong className="accounts-session-count">{displayedCookieCount}</strong>
                    <p className="accounts-section-copy">
                      {selectedAccount
                        ? (selectedSession?.fingerprint
                          ? (
                              <span className="accounts-fingerprint-row">
                                <span className="accounts-field-mono" title={selectedSession.fingerprint}>
                                  {selectedSession.fingerprint}
                                </span>
                                <button
                                  className="ghost-button accounts-copy-button"
                                  onClick={() => void copyFingerprint(selectedSession.fingerprint ?? '')}
                                  type="button"
                                >
                                  {fingerprintCopied ? 'Copied' : 'Copy'}
                                </button>
                              </span>
                            )
                          : 'No stored cookie fingerprint yet.')
                        : (draftCookies.length > 0
                          ? 'Draft cookies will be persisted when you create the account.'
                          : 'No draft cookie fingerprint yet.')}
                    </p>
                    {selectedAccount ? (
                      <p className="accounts-section-copy accounts-inline-note">
                        Cookie edits save immediately for existing accounts.
                      </p>
                    ) : null}
                  </div>
                </div>

                <div className="action-row accounts-local-actions">
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

              <section className="accounts-section">
                <header className="accounts-section-header accounts-section-header-row">
                  <h3>Browser import</h3>
                  {importState?.providerUsername ? (
                    <span className="accounts-handle-chip">@{importState.providerUsername}</span>
                  ) : null}
                </header>
                {selectedAccount && importState ? (
                  <>
                    <p className="accounts-section-copy">
                      Last import {formatShortDateTime(importState.lastImportedAt)}.
                      {importState.backupImportedAt
                        ? ` Previous session from ${formatShortDateTime(importState.backupImportedAt)} is available.`
                        : ' No previous session backup is available.'}
                    </p>
                    {importState.canRevert ? (
                      <div className="action-row accounts-local-actions">
                        <button
                          className="ghost-button"
                          disabled={Boolean(pendingCommand)}
                          onClick={() => void handleRevertLastImport()}
                          type="button"
                        >
                          Revert last import
                        </button>
                      </div>
                    ) : null}
                  </>
                ) : (
                  <p className="accounts-section-copy">
                    Import a live browser session with the NinjaCrawler Companion while logged into the provider account,
                    or paste cookies via Edit cookies. After import, use Validate account to confirm the session.
                  </p>
                )}
              </section>
            </section>
          ) : null}

          {activeTab === 'defaults' ? (
            <section
              aria-labelledby="accounts-tab-button-defaults"
              className="accounts-tab-panel"
              id="accounts-tab-defaults"
              role="tabpanel"
            >
              <div className="accounts-banner" role="note">
                Applies to <strong>new profiles only</strong>. Existing sources keep their own sync options.
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
                visibleCategories={['defaults', 'extractVideo']}
              />
            </section>
          ) : null}

          {activeTab === 'provider' ? (
            <section
              aria-labelledby="accounts-tab-button-provider"
              className="accounts-tab-panel"
              id="accounts-tab-provider"
              role="tabpanel"
            >
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
                visibleCategories={['authorization', 'download', 'timers', 'errors']}
              />

              {draft.provider === 'instagram' ? (
                <section className="accounts-section">
                  <header className="accounts-section-header">
                    <h3>Global sync presets</h3>
                    <p className="accounts-section-copy">
                      Workspace-wide Instagram presets (not per-account). Save with Save changes or Save preset on each card.
                      {presetsDirty ? ' Unsaved preset edits.' : ''}
                    </p>
                  </header>
                  <div className="accounts-preset-stack">
                    {(Object.keys(globalPresetsDraft) as InstagramPresetSlot[]).map((slot) => {
                      const preset = globalPresetsDraft[slot]
                      const disabled = savingGlobalPreset === slot
                      const presetLabel = DEFAULT_INSTAGRAM_PRESET_LABELS[slot]
                      return (
                        <article className="accounts-preset-card" key={slot}>
                          <header className="accounts-preset-header">
                            <div>
                              <p className="accounts-preset-eyebrow">{presetLabel} · workspace</p>
                              <h4 className="accounts-preset-title">{preset.label.trim() || presetLabel}</h4>
                            </div>
                            <button
                              className="ghost-button accounts-preset-save"
                              disabled={disabled}
                              onClick={() => void handleSaveGlobalPreset(slot)}
                              type="button"
                            >
                              {disabled ? 'Saving…' : 'Save preset'}
                            </button>
                          </header>

                          <div className="accounts-preset-body">
                            <div className="accounts-toggle-row">
                              <input
                                checked={preset.enabled}
                                id={`${slot}-enabled`}
                                onChange={(event) => updateGlobalPreset(slot, (current) => ({
                                  ...current,
                                  enabled: event.target.checked,
                                }))}
                                type="checkbox"
                              />
                              <label className="accounts-setting-label" htmlFor={`${slot}-enabled`}>Enabled</label>
                            </div>

                            <label className="field" htmlFor={`${slot}-label`}>
                              <span>Label</span>
                              <input
                                id={`${slot}-label`}
                                onChange={(event) => updateGlobalPreset(slot, (current) => ({
                                  ...current,
                                  label: event.target.value,
                                }))}
                                onKeyDown={(event) => {
                                  if (event.key === 'Enter') {
                                    event.preventDefault()
                                  }
                                }}
                                value={preset.label}
                              />
                            </label>

                            <div className="accounts-scope-chips" role="group" aria-label={`${presetLabel} scopes`}>
                              {PRESET_SCOPE_OPTIONS.map((option) => {
                                const checked = Boolean(preset.sections[option.key])
                                return (
                                  <label
                                    className={checked ? 'accounts-scope-chip accounts-scope-chip-active' : 'accounts-scope-chip'}
                                    key={option.key}
                                  >
                                    <input
                                      checked={checked}
                                      onChange={(event) => updateGlobalPreset(slot, (current) => ({
                                        ...current,
                                        sections: {
                                          ...current.sections,
                                          [option.key]: event.target.checked,
                                        },
                                      }))}
                                      type="checkbox"
                                    />
                                    <span>{option.label}</span>
                                  </label>
                                )
                              })}
                            </div>
                          </div>
                        </article>
                      )
                    })}
                  </div>
                </section>
              ) : null}
            </section>
          ) : null}

          {activeTab === 'workspace' ? (
            <section
              aria-labelledby="accounts-tab-button-workspace"
              className="accounts-tab-panel"
              id="accounts-tab-workspace"
              role="tabpanel"
            >
              <WorkspacePolicyPanel />
            </section>
          ) : null}
        </form>

        <footer className="source-editor-footer accounts-footer">
          {storeError ? (
            <p className="source-editor-submit-error" role="alert">
              {storeError}
            </p>
          ) : null}
          <div className="action-row accounts-footer-actions">
            {isDirty ? (
              <div className="source-editor-dirty-indicator">
                <span aria-hidden className="source-editor-dirty-indicator-dot" />
                <span>
                  Unsaved changes
                  {presetsDirty && accountDirty ? ' (account + presets)' : presetsDirty ? ' (presets)' : ''}
                </span>
              </div>
            ) : justSaved ? (
              <div className="accounts-saved-indicator" role="status">
                <span aria-hidden>✓</span>
                <span>All changes saved</span>
              </div>
            ) : null}
            <button
              className="ghost-button accounts-footer-secondary"
              disabled={Boolean(pendingCommand) || !isDirty}
              onClick={handleResetChanges}
              type="button"
            >
              Reset changes
            </button>
            {selectedAccount ? (
              <button
                className="ghost-button accounts-footer-secondary"
                disabled={Boolean(pendingCommand)}
                onClick={() => void handleValidateAccount()}
                type="button"
              >
                Validate account
              </button>
            ) : null}
            <button
              className="primary-button source-editor-save-button accounts-footer-primary"
              disabled={saveDisabled}
              form="accounts-config-form"
              title={saveTitle}
              type="submit"
            >
              {isCreateMode ? 'Create account' : 'Save changes'}
            </button>
          </div>
        </footer>
      </div>

      {cookieDialogOpen ? (
        <CookieEditorDialog
          accountId={selectedAccount?.id}
          initialCookies={selectedAccount ? undefined : draftCookies}
          onClose={() => setCookieDialogOpen(false)}
          onSaveDraftCookies={(cookies) => {
            setDraftCookies(cookies)
          }}
          provider={selectedAccount?.provider ?? draft.provider}
          providerLabel={formatProviderLabel(selectedAccount?.provider ?? draft.provider, providerCatalog)}
        />
      ) : null}

      {pendingNavigation ? (
        <div className="accounts-dirty-dialog-backdrop" role="presentation">
          <section aria-labelledby="accounts-dirty-dialog-title" className="accounts-dirty-dialog" role="dialog">
            <h3 id="accounts-dirty-dialog-title">Unsaved changes</h3>
            <p>You have unsaved account changes. Save before switching, discard them, or keep editing.</p>
            <div className="action-row accounts-dirty-dialog-actions">
              <button className="ghost-button" onClick={() => void handleDirtyDialogAction('cancel')} type="button">
                Keep editing
              </button>
              <button className="ghost-button" onClick={() => void handleDirtyDialogAction('discard')} type="button">
                Discard
              </button>
              <button
                className="primary-button"
                disabled={!canSaveName || Boolean(pendingCommand)}
                onClick={() => void handleDirtyDialogAction('save')}
                type="button"
              >
                Save &amp; continue
              </button>
            </div>
          </section>
        </div>
      ) : null}
    </div>
  )
}
