import ReactDOM from 'react-dom/client'
import './styles.css'
import { applyTheme, watchTheme } from './theme'
import { closeDesktopWindow } from './utils/closeDesktopWindow'

applyTheme()
watchTheme()

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    void closeDesktopWindow()
  }
})

const container = document.getElementById('root')

if (!container) {
  throw new Error('Scheduler window root element was not found.')
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
      <h1>Scheduler Window Failed</h1>
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

void import('./features/scheduler/SchedulerWindowPage')
  .then(({ SchedulerWindowPage }) => {
    bootstrapReady = true
    root.render(<SchedulerWindowPage />)
  })
  .catch((error) => {
    renderBootstrapFailure(error)
  })

window.setTimeout(() => {
  if (!bootstrapReady && !bootstrapFailureRendered) {
    renderBootstrapFailure(new Error('Scheduler window bootstrap timed out while loading UI modules.'))
  }
}, 6000)
