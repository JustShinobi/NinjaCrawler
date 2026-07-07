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
  previewAccount,
  syncSource,
} from './core.js'

const elements = {
  profileSummary: document.querySelector('#profileSummary'),
  activeTabMeta: document.querySelector('#activeTabMeta'),
  statusPill: document.querySelector('#statusPill'),
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
  targetButton: document.querySelector('#targetButton'),
  singleVideoButton: document.querySelector('#singleVideoButton'),
  syncButton: document.querySelector('#syncButton'),
  addButton: document.querySelector('#addButton'),
  importAccountButton: document.querySelector('#importAccountButton'),
  accountImportPanel: document.querySelector('#accountImportPanel'),
  accountImportSummary: document.querySelector('#accountImportSummary'),
  accountImportFields: document.querySelector('#accountImportFields'),
  accountDestination: document.querySelector('#accountDestination'),
  newAccountNameField: document.querySelector('#newAccountNameField'),
  newAccountName: document.querySelector('#newAccountName'),
  confirmAccountImport: document.querySelector('#confirmAccountImport'),
  cancelAccountImport: document.querySelector('#cancelAccountImport'),
  accountImportMessage: document.querySelector('#accountImportMessage'),
  message: document.querySelector('#message'),
}

const state = {
  tab: null,
  detected: null,
  provider: null,
  target: null,
  video: null,
  context: null,
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
  const activeTabPromise = chrome.tabs.query({ active: true, currentWindow: true })
  const allTabsPromise = chrome.tabs.query({})
  const [tab] = await activeTabPromise
  state.tab = tab
  state.detected = detectProfileFromUrl(tab?.url)
  state.provider = state.detected?.provider ?? detectProviderFromUrl(tab?.url)
  state.target = detectTargetFromUrl(tab?.url)
  state.video = detectVideoFromUrl(tab?.url)

  renderActiveLoading()
  const profilesTask = loadOpenProfiles(allTabsPromise)
  const activeContextTask = loadActiveContext()
  await Promise.allSettled([profilesTask, activeContextTask])
}

async function loadActiveContext() {
  if (!state.provider) return
  try {
    state.context = await loadContext(state.tab.url)
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
    elements.existingMeta.textContent = target
      ? `${existing.handle} · selected story ${target.storyId}`
      : `${existing.handle} · ${existing.lastSyncedAt ? `Last sync ${formatDate(existing.lastSyncedAt)}` : 'Never synced'}`
    elements.targetButton?.classList.toggle('hidden', !target)
    elements.syncButton.classList.remove('hidden')
    elements.addButton.classList.add('hidden')
  } else {
    setStatus('ready', 'Ready')
    elements.existingBanner.classList.add('hidden')
    elements.targetButton?.classList.add('hidden')
    elements.syncButton.classList.add('hidden')
    elements.addButton.classList.remove('hidden')
  }

  // "Save as single video" aparece sempre que a aba é uma URL de vídeo baixável,
  // independente de haver perfil rastreado (baixa avulso na estrutura própria).
  elements.singleVideoButton?.classList.toggle('hidden', !state.video)

  elements.importAccountButton?.classList.remove('hidden')
  setMessage('')
}

async function startAccountImport() {
  if (!state.tab?.id || !state.provider) return
  setBusy(true)
  setMessage('Capturing the signed-in browser account…')
  try {
    const response = await chrome.runtime.sendMessage({
      type: 'captureAccount',
      tabId: state.tab.id,
      provider: state.provider,
    })
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
  ]) {
    if (button) {
      button.disabled = isBusy
    }
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
