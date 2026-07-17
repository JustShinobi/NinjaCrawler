import {
  PROVIDER_LABELS,
  addSource,
  addSources,
  collectDetectedProfiles,
  detectProviderFromUrl,
  detectTargetFromUrl,
  detectProfileFromUrl,
  detectVideoFromUrl,
  downloadSingleVideo,
  downloadTarget,
  importAccount,
  loadContext,
  loadContexts,
  loadHealth,
  loadCompanionUpdateStatus,
  previewAccount,
  resolveLiveTabUrl,
  syncSource,
} from './core.js'

const elements = {
  profileSummary: document.querySelector('#profileSummary'),
  activeTabMeta: document.querySelector('#activeTabMeta'),
  statusPill: document.querySelector('#statusPill'),
  updatePanel: document.querySelector('#updatePanel'),
  updateTitle: document.querySelector('#updateTitle'),
  updateMeta: document.querySelector('#updateMeta'),
  copyInstallPathButton: document.querySelector('#copyInstallPathButton'),
  openExtensionsButton: document.querySelector('#openExtensionsButton'),
  viewReleaseButton: document.querySelector('#viewReleaseButton'),
  updateInstructions: document.querySelector('#updateInstructions'),
  updateInstallPath: document.querySelector('#updateInstallPath'),
  updateMessage: document.querySelector('#updateMessage'),
  unsupportedPanel: document.querySelector('#unsupportedPanel'),
  offlinePanel: document.querySelector('#offlinePanel'),
  profilesPanel: document.querySelector('#profilesPanel'),
  profilesMeta: document.querySelector('#profilesMeta'),
  profilesLoading: document.querySelector('#profilesLoading'),
  profilesLoadingText: document.querySelector('#profilesLoadingText'),
  profilesList: document.querySelector('#profilesList'),
  selectAllButton: document.querySelector('#selectAllButton'),
  addSelectedButton: document.querySelector('#addSelectedButton'),
  batchActions: document.querySelector('#batchActions'),
  batchMessage: document.querySelector('#batchMessage'),
  profileForm: document.querySelector('#profileForm'),
  existingBanner: document.querySelector('#existingBanner'),
  existingMeta: document.querySelector('#existingMeta'),
  targetMeta: document.querySelector('#targetMeta'),
  targetButton: document.querySelector('#targetButton'),
  singleVideoButton: document.querySelector('#singleVideoButton'),
  syncButton: document.querySelector('#syncButton'),
  addButton: document.querySelector('#addButton'),
  importAccountButton: document.querySelector('#importAccountButton'),
  accountImportPanel: document.querySelector('#accountImportPanel'),
  accountImportSummary: document.querySelector('#accountImportSummary'),
  accountImportFields: document.querySelector('#accountImportFields'),
  accountImportWarnings: document.querySelector('#accountImportWarnings'),
  accountDestination: document.querySelector('#accountDestination'),
  newAccountNameField: document.querySelector('#newAccountNameField'),
  newAccountName: document.querySelector('#newAccountName'),
  confirmAccountImport: document.querySelector('#confirmAccountImport'),
  cancelAccountImport: document.querySelector('#cancelAccountImport'),
  accountImportMessage: document.querySelector('#accountImportMessage'),
  message: document.querySelector('#message'),
  themeSelect: document.querySelector('#themeSelect'),
  autoReloadCompanionUpdates: document.querySelector('#autoReloadCompanionUpdates'),
  syncShortcut: document.querySelector('#syncShortcut'),
  storyShortcut: document.querySelector('#storyShortcut'),
  configureShortcutsButton: document.querySelector('#configureShortcutsButton'),
}

// Leave headroom above the probe's internal timeout so its real error arrives
// first. This remains the safety net if the service worker stops responding.
const ACCOUNT_CAPTURE_TIMEOUT_MS = 15_000

const state = {
  tab: null,
  detected: null,
  provider: null,
  target: null,
  video: null,
  context: null,
  compatibility: null,
  updateStatus: null,
  pendingCompanionVersion: null,
  accountCapture: null,
  accountPreview: null,
  profiles: [],
  profilesAreLoading: false,
  profilesError: '',
  isBusy: false,
}

boot()

async function boot() {
  bindEvents()
  await loadPreferences()
  await loadPendingCompanionUpdate()
  await renderShortcuts()
  const activeTabPromise = chrome.tabs.query({ active: true, currentWindow: true })
  const allTabsPromise = chrome.tabs.query({})
  const [tab] = await activeTabPromise
  const liveUrl = await resolveLiveTabUrl(tab)
  if (tab) tab.url = liveUrl
  state.tab = tab
  state.detected = detectProfileFromUrl(tab?.url)
  state.provider = state.detected?.provider ?? detectProviderFromUrl(tab?.url)
  state.target = detectTargetFromUrl(tab?.url)
  state.video = detectVideoFromUrl(tab?.url)

  renderActiveLoading()
  const compatibilityTask = loadCompatibility()
  const profilesTask = loadOpenProfiles(allTabsPromise)
  const activeContextTask = loadActiveContext()
  await Promise.allSettled([compatibilityTask, profilesTask, activeContextTask])
}

async function loadCompatibility() {
  try {
    const health = await loadHealth()
    rememberCompatibility(health?.companionCompatibility)
    try {
      state.updateStatus = await loadCompanionUpdateStatus()
    } catch {
      state.updateStatus = null
    }
    renderCompatibility()
  } catch {
    // The active context still owns desktop-offline feedback for supported tabs.
  }
}

async function loadActiveContext() {
  if (!state.provider) return
  try {
    state.context = await loadContext(state.tab.url)
    rememberCompatibility(state.context?.companionCompatibility)
    renderContext()
  } catch (error) {
    showOffline(error)
  }
}

async function loadOpenProfiles(allTabsPromise) {
  try {
    const tabs = await allTabsPromise
    state.profiles = collectDetectedProfiles(tabs).map((profile) => ({
      ...profile,
      context: null,
      selected: false,
    }))
    if (state.profiles.length === 0) {
      if (!state.provider) {
        showUnsupported()
      }
      return
    }

    state.profilesAreLoading = true
    renderProfiles()
    const contexts = await loadContexts(state.profiles.map((profile) => profile.url))
    if (!Array.isArray(contexts) || contexts.length !== state.profiles.length) {
      throw new Error('NinjaCrawler returned an incomplete profile list.')
    }
    state.profiles.forEach((profile, index) => {
      profile.context = contexts[index]
    })
    rememberCompatibility(contexts[0]?.companionCompatibility)
    state.profilesAreLoading = false
    state.profilesError = ''
    renderProfiles()
    if (!state.provider) {
      elements.profileSummary.textContent = 'No supported profile in the active tab'
      setStatus('ready', 'Profiles')
    }
  } catch (error) {
    state.profilesAreLoading = false
    state.profilesError = error.message
    if (state.profiles.length > 0) {
      renderProfiles()
      setBatchMessage(error.message, 'error')
    } else {
      showPopupError(error)
    }
  }
}

function renderActiveLoading() {
  const activeSupported = Boolean(state.provider)
  elements.unsupportedPanel.classList.add('hidden')
  elements.offlinePanel.classList.add('hidden')
  elements.profileForm.classList.toggle('hidden', !activeSupported)
  elements.existingBanner.classList.add('hidden')
  elements.targetMeta.classList.add('hidden')
  elements.accountImportPanel.classList.add('hidden')

  if (!activeSupported) {
    elements.profileSummary.textContent = 'Looking for open profile tabs…'
    setStatus('neutral', 'Loading')
    return
  }

  elements.profileSummary.textContent = state.detected
    ? `${PROVIDER_LABELS[state.provider]} ${state.detected.handle}`
    : `${PROVIDER_LABELS[state.provider]} account import`
  elements.activeTabMeta.textContent = state.detected
    ? `${PROVIDER_LABELS[state.provider]} · ${state.detected.handle}`
    : `${PROVIDER_LABELS[state.provider]} account`
  setStatus('neutral', 'Loading')
  elements.targetButton?.classList.toggle('hidden', !state.target)
  elements.targetButton.disabled = true
  elements.singleVideoButton?.classList.toggle('hidden', !state.video)
  elements.syncButton.classList.toggle('hidden', !state.detected)
  elements.syncButton.disabled = true
  elements.addButton.classList.add('hidden')
  elements.importAccountButton?.classList.remove('hidden')
  setMessage('Loading active profile…')
}

function bindEvents() {
  elements.addButton?.addEventListener('click', () => submitAdd())
  elements.syncButton?.addEventListener('click', () => submitSync())
  elements.targetButton?.addEventListener('click', () => submitTargetDownload())
  elements.singleVideoButton?.addEventListener('click', () => submitSingleVideoDownload())
  elements.importAccountButton?.addEventListener('click', () => startAccountImport())
  elements.confirmAccountImport?.addEventListener('click', () => submitAccountImport())
  elements.cancelAccountImport?.addEventListener('click', () => closeAccountImport())
  elements.accountDestination?.addEventListener('change', () => renderAccountDestination())
  elements.selectAllButton?.addEventListener('click', () => toggleSelectAll())
  elements.addSelectedButton?.addEventListener('click', () => submitSelectedProfiles())
  elements.themeSelect?.addEventListener('change', () => saveTheme(elements.themeSelect.value))
  elements.autoReloadCompanionUpdates?.addEventListener('change', () => saveUpdatePreferences())
  elements.configureShortcutsButton?.addEventListener('click', () => openShortcutSettings())
  elements.copyInstallPathButton?.addEventListener('click', () => copyManagedInstallPath())
  elements.viewReleaseButton?.addEventListener('click', () => openCompatibilityUrl('releasePageUrl'))
  elements.openExtensionsButton?.addEventListener('click', () => openChromeExtensions())
}

function renderProfiles() {
  const available = state.profiles.filter((profile) =>
    profile.context && !profile.context.existingSource)
  const existingCount = state.profiles.filter((profile) => profile.context?.existingSource).length
  const selectedCount = available.filter((profile) => profile.selected).length

  elements.profilesPanel.classList.toggle('hidden', state.profiles.length === 0)
  elements.profilesPanel.setAttribute('aria-busy', String(state.profilesAreLoading))
  elements.profilesLoading.classList.toggle('hidden', !state.profilesAreLoading)
  elements.profilesLoadingText.textContent =
    `Checking ${state.profiles.length} ${state.profiles.length === 1 ? 'profile' : 'profiles'} in NinjaCrawler…`
  const profilesUnavailable = state.profilesAreLoading || Boolean(state.profilesError)
  elements.profilesList.classList.toggle('hidden', profilesUnavailable)
  elements.batchActions.classList.toggle('hidden', profilesUnavailable)
  elements.profilesMeta.textContent =
    `${state.profiles.length} unique ${state.profiles.length === 1 ? 'profile' : 'profiles'}`
    + (existingCount ? ` · ${existingCount} already added` : '')
  elements.profilesList.replaceChildren()

  for (const profile of state.profiles) {
    const existing = Boolean(profile.context?.existingSource)
    const row = document.createElement('label')
    row.className = `profile-row${existing ? ' is-existing' : ''}`

    const checkbox = document.createElement('input')
    checkbox.type = 'checkbox'
    checkbox.checked = profile.selected
    checkbox.disabled = state.profilesAreLoading || !profile.context || existing
    checkbox.dataset.profileKey = profile.key
    checkbox.addEventListener('change', () => {
      profile.selected = checkbox.checked
      updateBatchControls()
    })

    const identity = document.createElement('span')
    identity.className = 'profile-identity'
    const name = document.createElement('span')
    name.className = 'profile-name'
    name.textContent = profile.handle
    const detail = document.createElement('span')
    detail.className = existing ? 'profile-state' : 'profile-provider'
    detail.textContent = existing
      ? 'Added'
      : profile.context
        ? PROVIDER_LABELS[profile.provider]
        : 'Checking…'
    identity.append(name, detail)
    row.append(checkbox, identity)
    elements.profilesList.append(row)
  }

  elements.selectAllButton.classList.toggle(
    'hidden',
    state.profilesAreLoading || Boolean(state.profilesError) || available.length === 0,
  )
  elements.selectAllButton.textContent = selectedCount === available.length && available.length > 0
    ? 'Clear'
    : 'Select all'
  updateBatchControls()
}

function updateBatchControls() {
  const available = state.profiles.filter((profile) => !profile.context?.existingSource)
  const selectedCount = available.filter((profile) => profile.selected).length
  elements.addSelectedButton.disabled =
    state.isBusy || state.profilesAreLoading || Boolean(state.profilesError) || selectedCount === 0
  elements.selectAllButton.disabled = state.isBusy
  elements.addSelectedButton.textContent = selectedCount > 0
    ? `Add selected profiles (${selectedCount})`
    : 'Add selected profiles'
  elements.selectAllButton.textContent = selectedCount === available.length && available.length > 0
    ? 'Clear'
    : 'Select all'
}

function toggleSelectAll() {
  const available = state.profiles.filter((profile) => !profile.context?.existingSource)
  const select = available.some((profile) => !profile.selected)
  available.forEach((profile) => {
    profile.selected = select
  })
  renderProfiles()
}

async function submitSelectedProfiles() {
  const selected = state.profiles.filter((profile) =>
    profile.selected && !profile.context?.existingSource)
  if (selected.length === 0) return

  setBusy(true)
  setBatchMessage(`Adding ${selected.length} ${selected.length === 1 ? 'profile' : 'profiles'}…`)
  try {
    const result = await addSources(selected.map(({ provider, handle, displayName }) => ({
      provider,
      handle,
      displayName,
    })))
    const contexts = await loadContexts(state.profiles.map((profile) => profile.url))
    state.profiles.forEach((profile, index) => {
      profile.context = contexts[index]
      profile.selected = false
    })
    if (state.detected) {
      const activeProfile = state.profiles.find((profile) =>
        profile.provider === state.detected.provider
        && profile.handle.toLocaleLowerCase() === state.detected.handle.toLocaleLowerCase())
      state.context = activeProfile?.context ?? state.context
    }
    setBatchMessage(
      `${result.addedCount} ${result.addedCount === 1 ? 'profile added' : 'profiles added'}`
      + (result.skippedCount ? ` · ${result.skippedCount} skipped` : ''),
      'ok',
    )
    renderProfiles()
    if (state.provider) {
      renderContext()
    }
    void chrome.runtime.sendMessage({ type: 'refreshBadges' })
  } catch (error) {
    setBatchMessage(error.message, 'error')
  } finally {
    setBusy(false)
    updateBatchControls()
  }
}

function renderContext() {
  const { detected, context } = state
  const target = context.detectedTarget ?? state.target
  const existing = context.existingSource

  elements.unsupportedPanel.classList.add('hidden')
  elements.offlinePanel.classList.add('hidden')
  elements.profileForm.classList.remove('hidden')
  elements.profileForm.classList.toggle('is-existing', Boolean(existing))
  elements.profileSummary.textContent = detected
    ? `${PROVIDER_LABELS[state.provider]} ${detected.handle}`
    : `${PROVIDER_LABELS[state.provider]} account import`
  elements.activeTabMeta.textContent = detected
    ? `${PROVIDER_LABELS[state.provider]} · ${detected.handle}`
    : `${PROVIDER_LABELS[state.provider]} account`
  elements.targetButton.disabled = state.isBusy
  elements.singleVideoButton.disabled = state.isBusy
  elements.syncButton.disabled = state.isBusy
  elements.addButton.disabled = state.isBusy
  elements.importAccountButton.disabled = state.isBusy

  if (!detected) {
    setStatus('ready', 'Account')
    elements.existingBanner.classList.add('hidden')
    elements.targetButton?.classList.add('hidden')
    elements.syncButton.classList.add('hidden')
    elements.addButton.classList.add('hidden')
  } else if (existing) {
    setStatus('good', target ? 'Story' : 'Added')
    elements.existingBanner.classList.remove('hidden')
    elements.existingMeta.textContent = `${existing.handle} · ${existing.lastSyncedAt ? `Last sync ${formatDate(existing.lastSyncedAt)}` : 'Never synced'}`
    elements.targetMeta.textContent = target ? `Selected story ${target.storyId}` : ''
    elements.targetMeta.classList.toggle('hidden', !target)
    elements.targetButton?.classList.toggle('hidden', !target)
    elements.syncButton.classList.remove('hidden')
    elements.addButton.classList.add('hidden')
  } else {
    setStatus('ready', 'Ready')
    elements.existingBanner.classList.add('hidden')
    elements.targetMeta.classList.add('hidden')
    elements.targetButton?.classList.add('hidden')
    elements.syncButton.classList.add('hidden')
    elements.addButton.classList.remove('hidden')
  }

  // "Save as single video" is available for every downloadable video URL,
  // even when no tracked profile exists for that standalone download.
  elements.singleVideoButton?.classList.toggle('hidden', !state.video)

  elements.importAccountButton?.classList.remove('hidden')
  setMessage('')
}

async function startAccountImport() {
  if (!state.tab?.id || !state.provider) return
  setBusy(true)
  setMessage('Capturing the signed-in browser account…')
  try {
    const response = await withTimeout(
      chrome.runtime.sendMessage({
        type: 'captureAccount',
        tabId: state.tab.id,
        provider: state.provider,
      }),
      ACCOUNT_CAPTURE_TIMEOUT_MS,
      'Account capture timed out. Reload the tab and try again.',
    )
    if (!response?.ok) throw new Error(response?.error || 'Account capture failed.')
    state.accountCapture = response.capture
    state.accountPreview = await previewAccount(response.capture)
    renderAccountImportReview()
    setMessage('')
  } catch (error) {
    setMessage(error.message, 'error')
  } finally {
    setBusy(false)
  }
}

function renderAccountImportReview() {
  const preview = state.accountPreview
  if (!preview) return
  elements.accountImportPanel.classList.remove('hidden')
  elements.accountImportSummary.textContent =
    `${PROVIDER_LABELS[preview.provider]} @${preview.username} · ${preview.cookieCount} cookies`
  elements.accountImportFields.textContent = preview.authorizationFields.length
    ? `Authorization: ${preview.authorizationFields.join(', ')}`
    : 'No additional authorization parameters detected.'

  const missing = preview.missingRequiredFields ?? []
  elements.accountImportWarnings.classList.toggle('hidden', missing.length === 0)
  elements.accountImportWarnings.textContent = missing.length
    ? `Incomplete browser session — missing ${missing.map(formatMissingField).join(', ')}. `
      + 'Sign in on this tab and capture again.'
    : ''

  elements.accountDestination.replaceChildren()

  const createOption = document.createElement('option')
  createOption.value = '__new__'
  createOption.textContent = 'Create a new account'
  elements.accountDestination.append(createOption)
  for (const candidate of preview.candidates) {
    const option = document.createElement('option')
    option.value = candidate.accountId
    option.textContent = `Update ${candidate.displayName}${candidate.matchKind === 'provider_user_id' ? ' (matched)' : ''}`
    elements.accountDestination.append(option)
  }
  elements.accountDestination.value = preview.suggestedAccountId || '__new__'
  elements.newAccountName.value = preview.username
  elements.accountImportMessage.textContent = ''
  renderAccountDestination()
}

function renderAccountDestination() {
  const creating = elements.accountDestination.value === '__new__'
  elements.newAccountNameField.classList.toggle('hidden', !creating)
  elements.confirmAccountImport.textContent = creating ? 'Create and import' : 'Update account'
  // The desktop rejects incomplete sessions; disable this action so the user
  // does not have to click it only to receive the same validation error.
  const incomplete = (state.accountPreview?.missingRequiredFields ?? []).length > 0
  elements.confirmAccountImport.disabled = state.isBusy || incomplete
}

function formatMissingField(field) {
  if (field === 'identity.username') return 'the account username'
  const cookieMatch = field.match(/^cookie:(.+)$/)
  if (cookieMatch) {
    return `cookie ${cookieMatch[1].split('|').join(' or ')}`
  }
  return field
}

async function submitAccountImport() {
  if (!state.accountCapture || !state.accountPreview) return
  const creating = elements.accountDestination.value === '__new__'
  const createDisplayName = elements.newAccountName.value.trim()
  if (creating && !createDisplayName) {
    setAccountImportMessage('Enter a name for the new account.', 'error')
    return
  }
  if (!creating) {
    const selected = state.accountPreview.candidates
      .find((candidate) => candidate.accountId === elements.accountDestination.value)
    if (!globalThis.confirm(`Replace the browser session for "${selected?.displayName ?? 'this account'}"?`)) {
      return
    }
  }

  setBusy(true)
  setAccountImportMessage('Saving and validating…')
  try {
    const result = await importAccount({
      capture: state.accountCapture,
      targetAccountId: creating ? null : elements.accountDestination.value,
      createDisplayName: creating ? createDisplayName : null,
    })
    if (result.validationError) {
      setAccountImportMessage(`Imported, but validation is degraded: ${result.validationError}`, 'error')
    } else {
      setAccountImportMessage(result.created ? 'Account created and validated.' : 'Account updated and validated.', 'ok')
    }
  } catch (error) {
    setAccountImportMessage(error.message, 'error')
  } finally {
    setBusy(false)
  }
}

function closeAccountImport() {
  state.accountCapture = null
  state.accountPreview = null
  elements.accountImportPanel.classList.add('hidden')
  setAccountImportMessage('')
}

async function submitAdd() {
  const { detected } = state
  setBusy(true)
  setMessage('')

  try {
    const result = await addSource({
      provider: detected.provider,
      handle: detected.handle,
      displayName: detected.displayName,
    })
    setMessage(result.opened ? 'Sent to NinjaCrawler.' : 'Request completed.', 'ok')
    state.context = await loadContext(state.tab.url)
    renderContext()
  } catch (error) {
    setMessage(error.message, 'error')
  } finally {
    setBusy(false)
  }
}

async function submitSync() {
  const existing = state.context?.existingSource
  if (!existing) return

  setBusy(true)
  setMessage('')

  try {
    await syncSource({
      sourceId: existing.id,
    })
    setMessage('Sync queued.', 'ok')
  } catch (error) {
    setMessage(error.message, 'error')
  } finally {
    setBusy(false)
  }
}

async function submitTargetDownload() {
  const existing = state.context?.existingSource
  const target = state.context?.detectedTarget ?? state.target
  if (!existing || !target) return

  setBusy(true)
  setMessage('')

  try {
    await downloadTarget({
      sourceId: existing.id,
      target,
    })
    setMessage('Selected story queued.', 'ok')
  } catch (error) {
    setMessage(error.message, 'error')
  } finally {
    setBusy(false)
  }
}

async function submitSingleVideoDownload() {
  const video = state.video
  if (!video?.url) return

  setBusy(true)
  setMessage('Downloading video…')

  try {
    await downloadSingleVideo(video.url)
    setMessage('Saved to Single videos.', 'ok')
  } catch (error) {
    setMessage(error.message, 'error')
  } finally {
    setBusy(false)
  }
}

function showUnsupported() {
  elements.profileSummary.textContent = 'No supported profile detected'
  setStatus('neutral', 'Idle')
  elements.unsupportedPanel.classList.remove('hidden')
  elements.offlinePanel.classList.add('hidden')
  elements.profilesPanel.classList.add('hidden')
  elements.profileForm.classList.add('hidden')
}

function showOffline(error) {
  setStatus('bad', 'Offline')
  elements.profileSummary.textContent = state.detected
    ? `${PROVIDER_LABELS[state.provider]} ${state.detected.handle}`
    : state.provider
      ? `${PROVIDER_LABELS[state.provider]} account import`
      : 'NinjaCrawler desktop unavailable'
  elements.unsupportedPanel.classList.add('hidden')
  elements.offlinePanel.classList.remove('hidden')
  elements.profilesPanel.classList.add('hidden')
  elements.profileForm.classList.add('hidden')
  elements.offlinePanel.querySelector('.muted').textContent = error?.message || 'Start NinjaCrawler and keep it running.'
}

function showPopupError(error) {
  setStatus('bad', 'Error')
  elements.unsupportedPanel.classList.add('hidden')
  elements.offlinePanel.classList.remove('hidden')
  elements.profilesPanel.classList.add('hidden')
  elements.profileForm.classList.add('hidden')
  elements.offlinePanel.querySelector('h2').textContent = 'Popup Error'
  elements.offlinePanel.querySelector('.muted').textContent = error?.message || 'Unexpected popup error.'
}

function setBusy(isBusy) {
  state.isBusy = isBusy
  for (const button of [
    elements.addButton,
    elements.syncButton,
    elements.targetButton,
    elements.singleVideoButton,
    elements.importAccountButton,
    elements.confirmAccountImport,
    elements.cancelAccountImport,
    elements.selectAllButton,
    elements.addSelectedButton,
    elements.copyInstallPathButton,
  ]) {
    if (button) {
      button.disabled = isBusy
    }
  }
  // Finishing a busy operation must not re-enable an incomplete session import.
  if (!isBusy && (state.accountPreview?.missingRequiredFields ?? []).length > 0) {
    elements.confirmAccountImport.disabled = true
  }
}

function setBatchMessage(text, kind = '') {
  elements.batchMessage.textContent = text
  elements.batchMessage.className = `message ${kind}`.trim()
}

function setAccountImportMessage(text, kind = '') {
  elements.accountImportMessage.textContent = text
  elements.accountImportMessage.className = `message ${kind}`.trim()
}

function setStatus(kind, text) {
  elements.statusPill.className = `status ${kind}`
  elements.statusPill.textContent = text
}

function setMessage(text, kind = '') {
  elements.message.textContent = text
  elements.message.className = `message ${kind}`.trim()
}

function rememberCompatibility(compatibility) {
  if (!compatibility) return
  state.compatibility = compatibility
  renderCompatibility()
}

function renderCompatibility() {
  const compatibility = state.compatibility
  const needsUpdate = compatibility?.status === 'update_available'
    || compatibility?.status === 'incompatible'
  elements.updatePanel.classList.toggle('hidden', !needsUpdate)
  if (!needsUpdate) return

  const installed = compatibility.installedVersion || chrome.runtime.getManifest().version
  const updateStatus = state.updateStatus
  const ready = Boolean(updateStatus?.updateReady)
  const installPath = updateStatus?.installPath || compatibility.installPath || ''

  elements.updateTitle.textContent = compatibility.status === 'incompatible'
    ? 'Companion update required'
    : ready
      ? state.pendingCompanionVersion
        ? 'Chrome is using another Companion folder'
        : 'Managed update is ready'
      : 'Companion update available'
  elements.updateMeta.textContent = ready
    ? `Installed ${installed} · staged ${updateStatus.stagedVersion || compatibility.availableVersion}`
    : `Installed ${installed} · available ${compatibility.availableVersion}`

  if (elements.updateInstructions) {
    elements.updateInstructions.textContent = ready
      ? state.pendingCompanionVersion
        ? 'The managed folder is current, but Chrome is still running an older copy. Open extensions and choose Load unpacked with the folder below once.'
        : 'The managed folder is current. The Companion will reload automatically.'
      : 'Open NinjaCrawler > Connector Runtimes to download the update into the managed folder.'
  }

  if (elements.updateInstallPath) {
    elements.updateInstallPath.textContent = installPath ? `Install path: ${installPath}` : ''
    elements.updateInstallPath.classList.toggle('hidden', !installPath)
  }

  if (elements.copyInstallPathButton) {
    elements.copyInstallPathButton.classList.toggle('hidden', !ready || !installPath)
    elements.copyInstallPathButton.disabled = state.isBusy
  }
}

async function copyManagedInstallPath() {
  const installPath = state.updateStatus?.installPath || state.compatibility?.installPath || ''
  if (!installPath) return
  try {
    await navigator.clipboard.writeText(installPath)
    setUpdateMessage('Managed folder copied. Use it with Load unpacked in Chrome Extensions.', 'ok')
  } catch (error) {
    setUpdateMessage(error?.message || 'Could not copy the managed folder.', 'error')
  }
}

async function loadPendingCompanionUpdate() {
  const installedVersion = chrome.runtime.getManifest().version
  const { pendingCompanionVersion = null } = await chrome.storage.local.get({
    pendingCompanionVersion: null,
  })
  if (pendingCompanionVersion === installedVersion) {
    await chrome.storage.local.remove(['pendingCompanionVersion', 'pendingCompanionReloadAt'])
    state.pendingCompanionVersion = null
    return
  }
  state.pendingCompanionVersion = pendingCompanionVersion
}

function setUpdateMessage(text, kind = '') {
  if (!elements.updateMessage) return
  elements.updateMessage.textContent = text
  elements.updateMessage.className = `message ${kind}`.trim()
}

async function openCompatibilityUrl(field) {
  const url = state.compatibility?.[field]
  if (!url) return
  try {
    await chrome.tabs.create({ url })
  } catch (error) {
    setMessage(error?.message || 'Could not open the Companion update.', 'error')
  }
}

async function openChromeExtensions() {
  try {
    await chrome.tabs.create({ url: 'chrome://extensions' })
    window.close()
  } catch (error) {
    setMessage(error?.message || 'Could not open Chrome Extensions.', 'error')
  }
}

function withTimeout(promise, timeoutMs, message) {
  let timer
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(message)), timeoutMs)
  })
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer))
}

function formatDate(value) {
  try {
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: 'short',
      timeStyle: 'short',
    }).format(new Date(value))
  } catch {
    return value
  }
}

async function loadPreferences() {
  const {
    theme = 'system',
    autoReloadCompanionUpdates = false,
  } = await chrome.storage.sync.get({
    theme: 'system',
    autoReloadCompanionUpdates: false,
  })
  elements.themeSelect.value = theme
  if (elements.autoReloadCompanionUpdates) {
    elements.autoReloadCompanionUpdates.checked = autoReloadCompanionUpdates
  }
  applyTheme(theme)
}

async function saveTheme(theme) {
  applyTheme(theme)
  await chrome.storage.sync.set({ theme })
}

async function saveUpdatePreferences() {
  await chrome.storage.sync.set({
    autoReloadCompanionUpdates: Boolean(elements.autoReloadCompanionUpdates?.checked),
  })
}

function applyTheme(theme) {
  if (theme === 'light' || theme === 'dark') {
    document.documentElement.dataset.theme = theme
  } else {
    delete document.documentElement.dataset.theme
  }
}

async function renderShortcuts() {
  const commands = await chrome.commands.getAll()
  const shortcuts = new Map(commands.map((command) => [command.name, command.shortcut]))
  elements.syncShortcut.textContent = shortcuts.get('sync-profile') || 'Not set'
  elements.storyShortcut.textContent = shortcuts.get('download-story') || 'Not set'
}

async function openShortcutSettings() {
  try {
    await chrome.tabs.create({ url: 'chrome://extensions/shortcuts' })
    window.close()
  } catch (error) {
    setMessage(error?.message || 'Could not open Chrome shortcut settings.', 'error')
  }
}
