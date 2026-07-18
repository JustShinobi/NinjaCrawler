import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'
import './styles.css'
import { applyTheme, watchTheme } from './theme'
import { MigrationGate } from './features/migration/MigrationGate'

applyTheme()
watchTheme()

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <MigrationGate>
      <App />
    </MigrationGate>
  </React.StrictMode>,
)
