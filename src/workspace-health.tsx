import ReactDOM from 'react-dom/client'
import './styles.css'
import { applyTheme, watchTheme } from './theme'
import { closeDesktopWindow } from './utils/closeDesktopWindow'
import type { WorkspaceHealthWindowIntent } from './domain/models'

applyTheme()
watchTheme()

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') void closeDesktopWindow()
})

const container = document.getElementById('root')
if (!container) throw new Error('Workspace Health root element was not found.')

const root = ReactDOM.createRoot(container)

function parseInitialIntentFromQuery(): WorkspaceHealthWindowIntent {
  const initialTab = new URLSearchParams(window.location.search).get('initialTab')
  return initialTab === 'overview' || initialTab === 'sources' || initialTab === 'accounts' || initialTab === 'storage'
    ? { initialTab }
    : {}
}

void import('./features/workspace/WorkspaceHealthWindowPage')
  .then(({ WorkspaceHealthWindowPage }) => root.render(<WorkspaceHealthWindowPage initialIntent={parseInitialIntentFromQuery()} />))
  .catch((error) => {
    const message = error instanceof Error ? error.message : String(error)
    root.render(<div className="runtime-log-bootstrap-failure"><h1>Workspace Health Failed</h1><pre>{message}</pre></div>)
  })
