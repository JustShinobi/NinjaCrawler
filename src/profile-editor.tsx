import ReactDOM from 'react-dom/client'
import type { SourceEditorWindowIntent, ProviderKey } from './domain/models'
import { SourceEditorWindowPage } from './features/workspace/SourceEditorWindowPage'
import { applyTheme, watchTheme } from './theme'
import { closeDesktopWindow } from './utils/closeDesktopWindow'
import './styles.css'

const PROVIDERS: ProviderKey[] = ['instagram', 'tiktok', 'twitter']

applyTheme()
watchTheme()

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    void closeDesktopWindow()
  }
})

function parseInitialIntentFromQuery(): SourceEditorWindowIntent {
  const query = new URLSearchParams(window.location.search)
  const provider = query.get('preferredProvider')
  const seedProvider = query.get('seedProvider')
  const initialProvider = provider && PROVIDERS.includes(provider as ProviderKey)
    ? (provider as ProviderKey)
    : undefined
  const sourceId = query.get('sourceId')?.trim() || undefined
  const preferredAccountId = query.get('preferredAccountId')?.trim() || undefined
  const seedHandle = query.get('seedHandle')?.trim() || undefined
  const seed = seedProvider
    && PROVIDERS.includes(seedProvider as ProviderKey)
    && seedHandle
    ? {
        provider: seedProvider as ProviderKey,
        handle: seedHandle,
        displayName: query.get('seedDisplayName')?.trim() || seedHandle.replace(/^@+/, ''),
      }
    : undefined

  return {
    sourceId,
    preferredProvider: initialProvider,
    preferredAccountId,
    seed,
  }
}

const container = document.getElementById('root')

if (!container) {
  throw new Error('Profile editor root element was not found.')
}

const rootContainer = container
const root = ReactDOM.createRoot(rootContainer)
let bootstrapFailureRendered = false
let bootstrapReady = false

function renderBootstrapFailure(error: unknown) {
  if (bootstrapFailureRendered) {
    return
  }

  bootstrapFailureRendered = true
  const message = error instanceof Error ? `${error.name}: ${error.message}` : String(error)
  const escapedMessage = message
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')

  rootContainer.innerHTML = `
    <div class="runtime-log-bootstrap-failure">
      <h1>Profile Editor Failed</h1>
      <pre>${escapedMessage}</pre>
    </div>
  `
}

window.addEventListener('error', (event) => {
  event.preventDefault()
  renderBootstrapFailure(event.error ?? event.message)
})

window.addEventListener('unhandledrejection', (event) => {
  event.preventDefault()
  renderBootstrapFailure(event.reason)
})

try {
  const initialIntent = parseInitialIntentFromQuery()
  bootstrapReady = true
  root.render(<SourceEditorWindowPage initialIntent={initialIntent} />)
} catch (error) {
  renderBootstrapFailure(error)
}

window.setTimeout(() => {
  if (!bootstrapReady && !bootstrapFailureRendered) {
    renderBootstrapFailure(new Error('Profile editor bootstrap timed out while loading UI modules.'))
  }
}, 6000)
