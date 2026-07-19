import ReactDOM from 'react-dom/client'
import type { AccountsWindowIntent } from './domain/models'
import { AccountsWindowPage } from './features/accounts/AccountsWindowPage'
import './styles.css'
import { applyTheme, watchTheme } from './theme'

applyTheme()
watchTheme()

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    window.close()
  }
})

function parseInitialIntentFromQuery(): AccountsWindowIntent {
  const query = new URLSearchParams(window.location.search)
  const provider = query.get('initialProvider')
  const mode = query.get('initialMode')
  const initialProvider =
    provider === 'instagram'
    || provider === 'tiktok'
    || provider === 'twitter'
    || provider === 'youtube'
    || provider === 'vsco'
      ? provider
      : undefined
  const initialMode = mode === 'create' || mode === 'edit' ? mode : undefined
  const initialAccountId = query.get('initialAccountId')?.trim() || undefined

  return {
    initialAccountId,
    initialProvider,
    initialMode,
  }
}

const container = document.getElementById('root')

if (!container) {
  throw new Error('Accounts root element was not found.')
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
      <h1>Accounts Failed</h1>
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
  root.render(<AccountsWindowPage initialIntent={initialIntent} />)
} catch (error) {
  renderBootstrapFailure(error)
}

window.setTimeout(() => {
  if (!bootstrapReady && !bootstrapFailureRendered) {
    renderBootstrapFailure(new Error('Accounts bootstrap timed out while loading UI modules.'))
  }
}, 6000)
