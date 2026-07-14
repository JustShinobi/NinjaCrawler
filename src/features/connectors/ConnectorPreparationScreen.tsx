import { useEffect, useRef, useState } from 'react'
import { useAppStore } from '../../state/appStore'
import type { ConnectorRuntimeStatus } from '../../domain/models'
import { connectorRuntimeStatusLabel } from './connectorRuntimeStatus'
import { connectorsNeedPreparation } from './connectorPreparation'

const EMPTY_CONNECTOR_RUNTIMES: ConnectorRuntimeStatus[] = []

export function ConnectorPreparationScreen() {
  const snapshot = useAppStore((state) => state.snapshot)
  const pendingCommand = useAppStore((state) => state.pendingCommand)
  const prepareConnectorRuntimes = useAppStore((state) => state.prepareConnectorRuntimes)
  const clearConnectorCustomOverride = useAppStore((state) => state.clearConnectorCustomOverride)
  const startedRef = useRef(false)
  const [attemptError, setAttemptError] = useState<string>()

  const runtimes = snapshot?.connectorRuntimes ?? EMPTY_CONNECTOR_RUNTIMES
  const preparing = pendingCommand === 'prepare_connector_runtimes'

  async function prepare() {
    setAttemptError(undefined)
    try {
      await prepareConnectorRuntimes()
    } catch (error) {
      setAttemptError(error instanceof Error ? error.message : String(error))
    }
  }

  useEffect(() => {
    if (startedRef.current || !connectorsNeedPreparation(runtimes)) {
      return
    }
    startedRef.current = true
    void prepareConnectorRuntimes().catch((error: unknown) => {
      setAttemptError(error instanceof Error ? error.message : String(error))
    })
  }, [prepareConnectorRuntimes, runtimes])

  async function restoreManaged(runtime: ConnectorRuntimeStatus) {
    setAttemptError(undefined)
    try {
      await clearConnectorCustomOverride(runtime.key)
      await prepareConnectorRuntimes()
    } catch (error) {
      setAttemptError(error instanceof Error ? error.message : String(error))
    }
  }

  return (
    <main className="connector-preparation-shell">
      <section className="connector-preparation-card" aria-labelledby="connector-preparation-title">
        <header className="connector-preparation-header">
          <p className="eyebrow">First-run preparation</p>
          <h1 id="connector-preparation-title">Preparing NinjaCrawler connectors</h1>
          <p>
            NinjaCrawler downloads the verified Windows connector runtimes before opening the workspace.
            Each asset must pass its GitHub SHA-256 digest and version probe.
          </p>
        </header>

        <div className="connector-preparation-list">
          {runtimes.map((runtime) => {
            const ready = Boolean(runtime.activeVersion)
            const progress = runtime.progressPercent ?? (ready ? 100 : 0)
            return (
              <article className="connector-preparation-item" key={runtime.key}>
                <div className="connector-preparation-item-heading">
                  <div>
                    <strong>{runtime.displayName}</strong>
                    <span>Required version {runtime.bundledVersion}</span>
                  </div>
                  <span className={ready ? 'status status-ready' : runtime.status === 'error' ? 'status status-failed' : 'status status-degraded'}>
                    {ready ? 'Ready' : connectorRuntimeStatusLabel(runtime)}
                  </span>
                </div>
                <div
                  aria-label={`${runtime.displayName} preparation progress`}
                  aria-valuemax={100}
                  aria-valuemin={0}
                  aria-valuenow={progress}
                  className="connector-preparation-progress"
                  role="progressbar"
                >
                  <span style={{ width: `${progress}%` }} />
                </div>
                <p>{runtime.lastError ?? runtime.progressDetail ?? 'Waiting to start.'}</p>
                {runtime.managementMode === 'custom' && !ready ? (
                  <button
                    className="toolbar-button"
                    disabled={Boolean(pendingCommand)}
                    onClick={() => void restoreManaged(runtime)}
                    type="button"
                  >
                    Use managed version
                  </button>
                ) : null}
              </article>
            )
          })}
        </div>

        {attemptError ? <p className="connector-preparation-error" role="alert">{attemptError}</p> : null}
        <footer className="connector-preparation-actions">
          <span>{preparing ? 'Downloading and verifying connectors…' : 'All connectors must be ready to continue.'}</span>
          <button
            className="primary-button"
            disabled={Boolean(pendingCommand)}
            onClick={() => void prepare()}
            type="button"
          >
            {preparing ? 'Preparing…' : 'Retry preparation'}
          </button>
        </footer>
      </section>
    </main>
  )
}
