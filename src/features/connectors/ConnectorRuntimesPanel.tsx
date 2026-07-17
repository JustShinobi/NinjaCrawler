import { useState } from 'react'
import type { ConnectorRuntimeStatus } from '../../domain/models'
import { useAppStore } from '../../state/appStore'
import { connectorRuntimeStatusClassName, connectorRuntimeStatusLabel } from './connectorRuntimeStatus'
import { CompanionRuntimeCard } from './CompanionRuntimeCard'

export function ConnectorRuntimesPanel() {
  const snapshot = useAppStore((state) => state.snapshot)
  const pendingCommand = useAppStore((state) => state.pendingCommand)
  const checkConnectorUpdates = useAppStore((state) => state.checkConnectorUpdates)
  const updateConnectorRuntime = useAppStore((state) => state.updateConnectorRuntime)
  const setConnectorCustomOverride = useAppStore((state) => state.setConnectorCustomOverride)
  const clearConnectorCustomOverride = useAppStore((state) => state.clearConnectorCustomOverride)
  const [customPaths, setCustomPaths] = useState<Record<string, string>>({})
  const [pathErrors, setPathErrors] = useState<Record<string, string>>({})

  const connectorRuntimes = snapshot?.connectorRuntimes ?? []

  async function handleApplyCustomPath(runtime: ConnectorRuntimeStatus) {
    const customPath = (customPaths[runtime.key] ?? runtime.customPath ?? '').trim()
    if (!customPath) {
      setPathErrors((current) => ({ ...current, [runtime.key]: 'Enter the full path to an executable.' }))
      return
    }

    setPathErrors((current) => ({ ...current, [runtime.key]: '' }))
    try {
      await setConnectorCustomOverride(runtime.key, customPath)
    } catch (error) {
      setPathErrors((current) => ({
        ...current,
        [runtime.key]: error instanceof Error ? error.message : String(error),
      }))
    }
  }

  return (
    <section className="connector-runtime-panel" aria-label="Connector runtimes" aria-busy={Boolean(pendingCommand)}>
      {connectorRuntimes.length > 0 ? (
        <div className="connector-runtime-list">
          <CompanionRuntimeCard />
          {connectorRuntimes.map((runtime) => (
            <article className="connector-runtime-row" key={runtime.key}>
              <header className="connector-settings-card-header">
                <div className="connector-settings-card-heading">
                  <p className="eyebrow">{runtime.managementMode === 'custom' ? 'Custom runtime' : 'Managed runtime'}</p>
                  <h3>{runtime.displayName}</h3>
                </div>
                <span className={`status ${connectorRuntimeStatusClassName(runtime)}`}>
                  {connectorRuntimeStatusLabel(runtime)}
                </span>
              </header>

              <dl className="connector-settings-meta">
                <div>
                  <dt>Active</dt>
                  <dd>{runtime.activeVersion ? `v${runtime.activeVersion}` : 'Not installed'}</dd>
                </div>
                <div>
                  <dt>Latest</dt>
                  <dd>{runtime.latestVersion ? `v${runtime.latestVersion}` : 'Unchecked'}</dd>
                </div>
                <div>
                  <dt>Bundled</dt>
                  <dd>v{runtime.bundledVersion}</dd>
                </div>
              </dl>

              {(runtime.progressDetail ?? runtime.lastError) ? (
                <p
                  className={runtime.lastError ? 'connector-settings-note connector-settings-note-error' : 'connector-settings-note'}
                  role={runtime.lastError ? 'alert' : 'status'}
                >
                  {runtime.progressDetail ?? runtime.lastError}
                </p>
              ) : null}

              <div className="connector-settings-actions">
                <button className="ghost-button" disabled={Boolean(pendingCommand)} onClick={() => void checkConnectorUpdates(runtime.key)} type="button">
                  Check
                </button>
                {runtime.managementMode === 'custom' ? (
                  <button className="ghost-button" disabled={Boolean(pendingCommand)} onClick={() => void clearConnectorCustomOverride(runtime.key)} type="button">
                    Use managed
                  </button>
                ) : runtime.updateAvailable ? (
                  <button className="primary-button" disabled={Boolean(pendingCommand)} onClick={() => void updateConnectorRuntime(runtime.key)} type="button">
                    Update
                  </button>
                ) : null}
              </div>

              <details className="connector-settings-override" open={runtime.managementMode === 'custom' ? true : undefined}>
                <summary>Custom executable</summary>
                <div className="setting-input-row">
                  <input
                    aria-label={`${runtime.displayName} custom executable path`}
                    aria-describedby={pathErrors[runtime.key] ? `connector-path-error-${runtime.key}` : undefined}
                    aria-invalid={Boolean(pathErrors[runtime.key])}
                    id={`connector-path-${runtime.key}`}
                    placeholder="Custom executable path…"
                    value={customPaths[runtime.key] ?? runtime.customPath ?? ''}
                    onChange={(event) => {
                      setPathErrors((current) => ({ ...current, [runtime.key]: '' }))
                      setCustomPaths((current) => ({ ...current, [runtime.key]: event.target.value }))
                    }}
                  />
                  <button className="primary-button" disabled={Boolean(pendingCommand)} onClick={() => void handleApplyCustomPath(runtime)} type="button">
                    Apply
                  </button>
                </div>
                {pathErrors[runtime.key] ? (
                  <p className="connector-path-error" id={`connector-path-error-${runtime.key}`} role="alert">{pathErrors[runtime.key]}</p>
                ) : null}
              </details>
            </article>
          ))}
        </div>
      ) : (
        <div className="connector-runtime-list">
          <CompanionRuntimeCard />
          <div className="connector-runtime-empty">
            <h2>No connector runtimes</h2>
            <p>Check again after the workspace finishes loading.</p>
            <button className="primary-button" disabled={Boolean(pendingCommand)} onClick={() => void checkConnectorUpdates()} type="button">Check again</button>
          </div>
        </div>
      )}
    </section>
  )
}
