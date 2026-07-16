import { useEffect } from 'react'
import { subscribeToDesktopRuntimeEvents } from '../../bridge/desktop'
import { useAppStore } from '../../state/appStore'
import { WindowShell } from '../brand/WindowShell'
import { WindowTitlebar } from '../brand/WindowTitlebar'
import { ConnectorRuntimesPanel } from './ConnectorRuntimesPanel'

export function ConnectorRuntimesWindowPage() {
  const bootstrap = useAppStore((state) => state.bootstrap)
  const refreshSnapshot = useAppStore((state) => state.refreshSnapshot)
  const pendingCommand = useAppStore((state) => state.pendingCommand)
  const checkConnectorUpdates = useAppStore((state) => state.checkConnectorUpdates)
  const snapshot = useAppStore((state) => state.snapshot)
  const connectorRuntimes = snapshot?.connectorRuntimes ?? []
  const managedCount = connectorRuntimes.filter((runtime) => runtime.managementMode === 'managed').length
  const customCount = connectorRuntimes.filter((runtime) => runtime.managementMode === 'custom').length
  const updateCount = connectorRuntimes.filter((runtime) => runtime.updateAvailable).length
  const attentionCount = connectorRuntimes.filter((runtime) => runtime.status === 'error' || runtime.status === 'pending_activation').length

  useEffect(() => {
    void bootstrap()
  }, [bootstrap])

  useEffect(() => {
    let disposed = false
    let unsubscribe: (() => void) | undefined

    void subscribeToDesktopRuntimeEvents({
      onConnectorRuntimeChanged: () => {
        if (!disposed) {
          void refreshSnapshot().catch(() => undefined)
        }
      },
    })
      .then((teardown) => {
        if (disposed) {
          teardown()
          return
        }

        unsubscribe = teardown
      })
      .catch(() => undefined)

    return () => {
      disposed = true
      unsubscribe?.()
    }
  }, [refreshSnapshot])

  return (
    <WindowShell
      density="compact"
      titlebar={<WindowTitlebar title="Connector Runtimes" />}
    >
      <div className="connector-runtime-window-body">
        <header className="connector-runtime-window-header">
          <div className="connector-runtime-window-tools">
            <div className="connector-runtime-window-summary" role="list" aria-label="Connector runtime summary">
              <span role="listitem">
                <strong>{managedCount}</strong> managed
              </span>
              <span role="listitem">
                <strong>{customCount}</strong> custom
              </span>
              <span className={updateCount > 0 ? 'is-attention' : undefined} role="listitem">
                <strong>{updateCount}</strong> updates
              </span>
              <span className={attentionCount > 0 ? 'is-danger' : undefined} role="listitem">
                <strong>{attentionCount}</strong> attention
              </span>
            </div>
            <button
              className="primary-button"
              disabled={Boolean(pendingCommand)}
              onClick={() => void checkConnectorUpdates()}
              type="button"
            >
              Check all
            </button>
          </div>
        </header>

        {snapshot ? (
          <ConnectorRuntimesPanel />
        ) : (
          <div className="panel runtime-log-window-empty" role="status">
            Loading connector runtimes…
          </div>
        )}
      </div>
    </WindowShell>
  )
}
