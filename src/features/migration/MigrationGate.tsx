import { useCallback, useEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import type { MigrationProgress, MigrationStatus } from '../../domain/models'
import {
  getMigrationStatus,
  openBackupsFolder,
  runPendingMigrations,
  subscribeToMigrationProgress,
} from '../../bridge/desktop'

type Phase = 'checking' | 'ready' | 'pending' | 'running' | 'error'

function formatBytes(bytes: number): string {
  if (!bytes || bytes <= 0) return '—'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value.toFixed(value >= 100 || unit === 0 ? 0 : 1)} ${units[unit]}`
}

function phaseLabel(progress: MigrationProgress | null): string {
  if (!progress) return 'Preparing…'
  if (progress.phase === 'backup') return progress.label || 'Backing up your database'
  return progress.label || 'Applying updates'
}

/**
 * Boot gate da janela principal: antes de qualquer acesso ao banco, checa se há
 * migrations pendentes. Se houver, exibe a tela de confirmação + progresso e só
 * renderiza o app quando terminar (o backend inicia os serviços de runtime ao
 * concluir). Um erro na CHECAGEM não trava o app (segue direto).
 */
export function MigrationGate({ children }: { children: ReactNode }) {
  const [phase, setPhase] = useState<Phase>('checking')
  const [status, setStatus] = useState<MigrationStatus | null>(null)
  const [progress, setProgress] = useState<MigrationProgress | null>(null)
  const [error, setError] = useState('')
  const disposeProgressRef = useRef<(() => void) | null>(null)

  useEffect(() => {
    let cancelled = false
    getMigrationStatus()
      .then((next) => {
        if (cancelled) return
        if (next && next.pendingCount > 0) {
          setStatus(next)
          setPhase('pending')
        } else {
          setPhase('ready')
        }
      })
      .catch(() => {
        // Falha na checagem não deve impedir o app de abrir.
        if (!cancelled) setPhase('ready')
      })
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => () => disposeProgressRef.current?.(), [])

  const handleConfirm = useCallback(() => {
    setPhase('running')
    setProgress(null)
    void subscribeToMigrationProgress(setProgress).then((dispose) => {
      disposeProgressRef.current = dispose
    })
    runPendingMigrations()
      .then(() => setPhase('ready'))
      .catch((runError) => {
        setError(runError instanceof Error ? runError.message : String(runError))
        setPhase('error')
      })
      .finally(() => {
        disposeProgressRef.current?.()
        disposeProgressRef.current = null
      })
  }, [])

  if (phase === 'ready') return <>{children}</>

  if (phase === 'checking') {
    return (
      <div className="migration-screen" role="status" aria-live="polite">
        <div className="migration-card">
          <div className="migration-spinner" aria-hidden="true" />
          <p className="migration-muted">Checking for updates…</p>
        </div>
      </div>
    )
  }

  const pct =
    progress && progress.total > 0
      ? Math.min(100, Math.round((progress.current / progress.total) * 100))
      : null

  return (
    <div className="migration-screen" role="dialog" aria-modal="true" aria-label="Database update">
      <div className="migration-card">
        <div className="migration-logo" aria-hidden="true">
          N
        </div>

        {phase === 'pending' && status ? (
          <>
            <h1 className="migration-title">Database update required</h1>
            <p className="migration-body">
              NinjaCrawler needs to update your local database before it can open
              {status.pendingCount > 1 ? ` (${status.pendingCount} updates)` : ''}. A full backup is
              taken first, so nothing is lost.
            </p>
            <dl className="migration-facts">
              <div>
                <dt>Database size</dt>
                <dd>{formatBytes(status.dbSizeBytes)}</dd>
              </div>
              <div>
                <dt>Version</dt>
                <dd>
                  {status.fromVersion} → {status.toVersion}
                </dd>
              </div>
            </dl>
            <p className="migration-hint">
              The backup is written to the <code>backups</code> folder. This can take a moment for
              large databases — please don’t close the app.
            </p>
            <div className="migration-actions">
              <button className="migration-primary" type="button" onClick={handleConfirm}>
                Back up &amp; update
              </button>
            </div>
          </>
        ) : null}

        {phase === 'running' ? (
          <>
            <h1 className="migration-title">Updating your database</h1>
            <p className="migration-body">{phaseLabel(progress)}</p>
            <div
              className="migration-progress"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={pct ?? undefined}
            >
              <div
                className={`migration-progress-fill${pct === null ? ' is-indeterminate' : ''}`}
                style={pct === null ? undefined : { width: `${pct}%` }}
              />
            </div>
            <p className="migration-muted">
              {progress?.phase === 'backup'
                ? pct === null
                  ? 'Starting backup…'
                  : `Backing up — ${pct}%`
                : pct === null
                  ? 'Applying updates…'
                  : `Applying updates — ${progress?.current ?? 0} of ${progress?.total ?? 0}`}
            </p>
            <p className="migration-hint">Please keep the app open until this finishes.</p>
          </>
        ) : null}

        {phase === 'error' ? (
          <>
            <h1 className="migration-title migration-title-error">Update failed</h1>
            <p className="migration-body">
              The database was not changed and your backup is safe. You can retry, or restore the
              backup manually from the backups folder.
            </p>
            {error ? <pre className="migration-error">{error}</pre> : null}
            <div className="migration-actions">
              <button className="migration-primary" type="button" onClick={handleConfirm}>
                Retry
              </button>
              <button
                className="migration-secondary"
                type="button"
                onClick={() => void openBackupsFolder()}
              >
                Open backups folder
              </button>
            </div>
          </>
        ) : null}
      </div>
    </div>
  )
}
