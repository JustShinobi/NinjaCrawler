import ReactDOM from 'react-dom/client'
import './styles.css'
import { applyTheme, watchTheme } from './theme'
import { closeDesktopWindow } from './utils/closeDesktopWindow'

applyTheme()
watchTheme()

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') void closeDesktopWindow()
})

const container = document.getElementById('root')
if (!container) throw new Error('Library root element was not found.')

const root = ReactDOM.createRoot(container)

void import('./features/library/LibraryWindowPage')
  .then(({ LibraryWindowPage }) => root.render(<LibraryWindowPage />))
  .catch((error) => {
    const message = error instanceof Error ? error.message : String(error)
    root.render(
      <div className="runtime-log-bootstrap-failure">
        <h1>Library Failed</h1>
        <pre>{message}</pre>
      </div>,
    )
  })
