import {
  PROVIDER_LABELS,
  addSource,
  detectTargetFromUrl,
  detectProfileFromUrl,
  downloadTarget,
  loadContext,
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
  syncButton: document.querySelector('#syncButton'),
  addButton: document.querySelector('#addButton'),
  message: document.querySelector('#message'),
}

const state = {
  tab: null,
  detected: null,
  target: null,
  context: null,
}

boot()

async function boot() {
  bindEvents()
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true })
  state.tab = tab
  state.detected = detectProfileFromUrl(tab?.url)
  state.target = detectTargetFromUrl(tab?.url)

  if (!state.detected) {
    showUnsupported()
    return
  }

  elements.profileSummary.textContent = `${PROVIDER_LABELS[state.detected.provider]} ${state.detected.handle}`

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
}

function renderContext() {
  const { detected, context } = state
  const target = context.detectedTarget ?? state.target
  const existing = context.existingSource

  elements.unsupportedPanel.classList.add('hidden')
  elements.offlinePanel.classList.add('hidden')
  elements.profileForm.classList.remove('hidden')
  elements.profileForm.classList.toggle('is-existing', Boolean(existing))

  if (existing) {
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

  setMessage('')
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

function showUnsupported() {
  elements.profileSummary.textContent = 'No supported profile detected'
  setStatus('neutral', 'Idle')
  elements.unsupportedPanel.classList.remove('hidden')
  elements.offlinePanel.classList.add('hidden')
  elements.profileForm.classList.add('hidden')
}

function showOffline(error) {
  setStatus('bad', 'Offline')
  elements.profileSummary.textContent = `${PROVIDER_LABELS[state.detected.provider]} ${state.detected.handle}`
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
  for (const button of [elements.addButton, elements.syncButton, elements.targetButton]) {
    if (button) {
      button.disabled = isBusy
    }
  }
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
