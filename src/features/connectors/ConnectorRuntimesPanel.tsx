import { useState } from 'react'
import type { ConnectorRuntimeStatus } from '../../domain/models'
import { useAppStore } from '../../state/appStore'
import { connectorRuntimeStatusClassName, connectorRuntimeStatusLabel } from './connectorRuntimeStatus'

export function ConnectorRuntimesPanel() {
  const snapshot = useAppStore((state) => state.snapshot)
  const pendingCommand = useAppStore((state) => state.pendingCommand)
  const checkConnectorUpdates = useAppStore((state) => state.checkConnectorUpdates)
  const updateConnectorRuntime = useAppStore((state) => state.updateConnectorRuntime)
  const setConnectorCustomOverride = useAppStore((state) => state.setConnectorCustomOverride)
  const clearConnectorCustomOverride = useAppStore((state) => state.clearConnectorCustomOverride)
  const [customPaths, setCustomPaths] = useState<Record<string, string>>({})

  const connectorRuntimes = snapshot?.connectorRuntimes ?? []

  async function handleApplyCustomPath(runtime: ConnectorRuntimeStatus) {
    const customPath = (customPaths[runtime.key] ?? runtime.customPath ?? '').trim()
    if (!customPath) {
      return
    }

    await setConnectorCustomOverride(runtime.key, customPath)
  }

  return (
    <section className="panel connector-runtime-panel">
      <div className="connector-runtime-panel-header">
        <button className="toolbar-button" disabled={Boolean(pendingCommand)} onClick={() => void checkConnectorUpdates()} type="button">
          Check all
        </button>
      </div>

      <div className="settings-grid settings-grid-wide connector-runtime-grid">
        {connectorRuntimes.map((runtime) => (
          <article className="settings-card connector-settings-card" key={runtime.key}>
            <header className="connector-settings-card-header">
              <div className="connector-settings-card-heading">
                <p className="eyebrow">{runtime.managementMode === 'custom' ? 'Custom' : 'Managed'}</p>
                <div className="connector-settings-card-title-row">
                  <h3>{runtime.displayName}</h3>
                  <span className="connector-settings-version-pill">
                    {runtime.activeVersion ? `v${runtime.activeVersion}` : 'Not installed'}
                  </span>
                </div>
              </div>
              <span className={`status ${connectorRuntimeStatusClassName(runtime)}`}>
                {connectorRuntimeStatusLabel(runtime)}
              </span>
            </header>

            <dl className="connector-settings-meta">
              <div>
                <dt>Bundled</dt>
                <dd>v{runtime.bundledVersion}</dd>
              </div>
              <div>
                <dt>Latest</dt>
                <dd>{runtime.latestVersion ? `v${runtime.latestVersion}` : 'Unchecked'}</dd>
              </div>
              <div>
                <dt>Mode</dt>
                <dd>{runtime.managementMode === 'custom' ? 'Custom path' : 'Managed'}</dd>
              </div>
            </dl>

            {(runtime.progressDetail ?? runtime.lastError) ? (
              <p className="connector-settings-note">{runtime.progressDetail ?? runtime.lastError}</p>
            ) : null}

            <div className="connector-settings-actions">
              <button className="toolbar-button" disabled={Boolean(pendingCommand)} onClick={() => void checkConnectorUpdates(runtime.key)} type="button">
                Check
              </button>
              {runtime.managementMode === 'custom' ? (
                <button className="toolbar-button" disabled={Boolean(pendingCommand)} onClick={() => void clearConnectorCustomOverride(runtime.key)} type="button">
                  Use managed
                </button>
              ) : (
                <button
                  className="toolbar-button toolbar-button-primary"
                  disabled={Boolean(pendingCommand) || !runtime.updateAvailable}
                  onClick={() => void updateConnectorRuntime(runtime.key)}
                  type="button"
                >
                  Update
                </button>
              )}
            </div>

            <div className="connector-settings-override">
              <div className="setting-input-row">
                <input
                  id={`connector-path-${runtime.key}`}
                  placeholder={runtime.managementMode === 'custom' ? runtime.customPath ?? '' : 'Custom executable path…'}
                  value={customPaths[runtime.key] ?? runtime.customPath ?? ''}
                  onChange={(event) =>
                    setCustomPaths((current) => ({
                      ...current,
                      [runtime.key]: event.target.value,
                    }))
                  }
                />
                <button className="primary-button" disabled={Boolean(pendingCommand)} onClick={() => void handleApplyCustomPath(runtime)} type="button">
                  Apply
                </button>
              </div>
            </div>
          </article>
        ))}
      </div>
    </section>
  )
}
