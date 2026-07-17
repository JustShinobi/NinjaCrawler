import { useEffect, useState } from 'react'
import {
  getCompanionInstallStatus,
  installCompanion,
  openCompanionInstallFolder,
} from '../../bridge/desktop'
import type { CompanionInstallStatus } from '../../domain/models'

export function CompanionRuntimeCard() {
  const [status, setStatus] = useState<CompanionInstallStatus | null>(null)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const [error, setError] = useState('')

  useEffect(() => {
    let disposed = false
    void getCompanionInstallStatus()
      .then((nextStatus) => {
        if (!disposed) setStatus(nextStatus)
      })
      .catch((loadError) => {
        if (!disposed) setError(errorMessage(loadError))
      })
    return () => {
      disposed = true
    }
  }, [])

  async function handleInstall() {
    setBusy(true)
    setError('')
    setMessage('Downloading and preparing the managed Companion folder…')
    try {
      const nextStatus = await installCompanion()
      setStatus(nextStatus)
      setMessage(`Companion v${nextStatus.stagedVersion ?? nextStatus.availableVersion} is ready.`)
    } catch (installError) {
      setMessage('')
      setError(errorMessage(installError))
    } finally {
      setBusy(false)
    }
  }

  async function handleCopyPath() {
    if (!status?.installPath) return
    try {
      await navigator.clipboard.writeText(status.installPath)
      setMessage('Managed folder copied.')
      setError('')
    } catch (copyError) {
      setError(errorMessage(copyError))
    }
  }

  async function handleOpenFolder() {
    if (!status?.installPath) return
    try {
      await openCompanionInstallFolder(status.installPath)
      setError('')
    } catch (openError) {
      setError(errorMessage(openError))
    }
  }

  const installed = Boolean(status?.updateReady)
  const actionLabel = status?.stagedVersion ? 'Update Companion' : 'Download Companion'

  return (
    <article className="connector-runtime-row companion-runtime-row" aria-busy={busy}>
      <header className="connector-settings-card-header">
        <div className="connector-settings-card-heading">
          <p className="eyebrow">Managed browser extension</p>
          <h3>NinjaCrawler Companion</h3>
        </div>
        <span className={`status ${error ? 'bad' : installed ? 'good' : 'neutral'}`}>
          {error ? 'Error' : installed ? 'Ready' : status ? 'Not installed' : 'Checking'}
        </span>
      </header>

      <dl className="connector-settings-meta">
        <div>
          <dt>Managed</dt>
          <dd>{status?.stagedVersion ? `v${status.stagedVersion}` : 'Not installed'}</dd>
        </div>
        <div>
          <dt>Available</dt>
          <dd>{status ? `v${status.availableVersion}` : 'Checking'}</dd>
        </div>
        <div>
          <dt>Live reload</dt>
          <dd>{installed ? 'Automatic' : 'After setup'}</dd>
        </div>
      </dl>

      <div className="connector-settings-actions">
        {!installed ? (
          <button className="primary-button" disabled={busy || !status} onClick={() => void handleInstall()} type="button">
            {busy ? 'Downloading…' : actionLabel}
          </button>
        ) : null}
        <button className="ghost-button" disabled={busy || !installed} onClick={() => void handleOpenFolder()} type="button">
          Open folder
        </button>
        <button className="ghost-button" disabled={busy || !status?.installPath} onClick={() => void handleCopyPath()} type="button">
          Copy path
        </button>
      </div>

      {(message || error) ? (
        <p className={error ? 'connector-settings-note connector-settings-note-error' : 'connector-settings-note'} role={error ? 'alert' : 'status'}>
          {error || message}
        </p>
      ) : null}

      <details className="connector-settings-override" open>
        <summary>Chrome setup and managed path</summary>
        <p className="companion-runtime-instructions">
          In chrome://extensions, enable Developer mode and choose Load unpacked with this folder once.
          NinjaCrawler will keep its files current and the Companion will reload automatically.
        </p>
        <code className="companion-runtime-path">{status?.installPath || 'Resolving managed folder…'}</code>
      </details>
    </article>
  )
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
