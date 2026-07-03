import ReactDOM from 'react-dom/client'
import './styles.css'
import { applyTheme, watchTheme } from './theme'
import { ConnectorDebugWindowPage } from './features/workspace/ConnectorDebugWindowPage'

applyTheme()
watchTheme()

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') window.close()
})

const container = document.getElementById('root')
if (!container) throw new Error('Connector debugger root element was not found.')

ReactDOM.createRoot(container).render(<ConnectorDebugWindowPage />)
