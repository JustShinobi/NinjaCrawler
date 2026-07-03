import {
  PROVIDER_LABELS,
  addSource,
  detectProviderFromUrl,
  detectTargetFromUrl,
  detectProfileFromUrl,
  detectVideoFromUrl,
  downloadSingleVideo,
  downloadTarget,
  importAccount,
  loadContext,
  previewAccount,
  syncSource,
} from './core.js'

const elements = {
  profileSummary: document.querySelector('#profileSummary'),
  statusPill: document.querySelector('#statusPill'),
  unsupportedPanel: document.querySelector('#unsupportedPanel'),
  offlinePanel: document.querySelector('#offlinePanel'),
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
}

boot()

async function boot() {
  bindEvents()
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true })
  state.tab = tab
  state.detected = detectProfileFromUrl(tab?.url)
  state.provider = state.detected?.provider ?? detectProviderFromUrl(tab?.url)
  state.target = detectTargetFromUrl(tab?.url)
  state.video = detectVideoFromUrl(tab?.url)

  if (!state.provider) {
    showUnsupported()
    return
  }

  elements.profileSummary.textContent = state.detected
    ? `${PROVIDER_LABELS[state.provider]} ${state.detected.handle}`
    : `${PROVIDER_LABELS[state.provider]} account import`

  try {
    state.context = await loadContext(tab.url)
  } catch (error) {
    showOffline(error)
    return
  }

  try {
    renderContext()
  } catch (error) {
    showPopupError(error)
  }
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
}

function renderContext() {
  const { detected, context } = state
  const target = context.detectedTarget ?? state.target
  const existing = context.existingSource

  elements.unsupportedPanel.classList.add('hidden')
  elements.offlinePanel.classList.add('hidden')
  elements.profileForm.classList.remove('hidden')
  elements.profileForm.classList.toggle('is-existing', Boolean(existing))

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
    elements.syncButton.classList.toggle('hidden', Boolean(target))
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
  elements.profileForm.classList.add('hidden')
}

function showOffline(error) {
  setStatus('bad', 'Offline')
  elements.profileSummary.textContent = state.detected
    ? `${PROVIDER_LABELS[state.provider]} ${state.detected.handle}`
    : `${PROVIDER_LABELS[state.provider]} account import`
  elements.unsupportedPanel.classList.add('hidden')
  elements.offlinePanel.classList.remove('hidden')
  elements.profileForm.classList.add('hidden')
  elements.offlinePanel.querySelector('.muted').textContent = error?.message || 'Start NinjaCrawler and keep it running.'
}

function showPopupError(error) {
  setStatus('bad', 'Error')
  elements.unsupportedPanel.classList.add('hidden')
  elements.offlinePanel.classList.remove('hidden')
  elements.profileForm.classList.add('hidden')
  elements.offlinePanel.querySelector('h2').textContent = 'Popup Error'
  elements.offlinePanel.querySelector('.muted').textContent = error?.message || 'Unexpected popup error.'
}

function setBusy(isBusy) {
  for (const button of [
    elements.addButton,
    elements.syncButton,
    elements.targetButton,
    elements.singleVideoButton,
    elements.importAccountButton,
    elements.confirmAccountImport,
    elements.cancelAccountImport,
  ]) {
    if (button) {
      button.disabled = isBusy
    }
  }
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
