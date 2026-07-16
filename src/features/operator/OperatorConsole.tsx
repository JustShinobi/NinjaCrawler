import { useMemo } from 'react'
import { useAppStore } from '../../state/appStore'
import type {
  DesktopRuntimeState,
  SourceSyncRun,
  SyncPlanRun,
} from '../../domain/models'
const EMPTY_SOURCE_SYNC_RUNS: SourceSyncRun[] = []
const EMPTY_SYNC_PLAN_RUNS: SyncPlanRun[] = []
const EMPTY_DESKTOP_RUNTIME: DesktopRuntimeState = {
  closeToTray: false,
  silentMode: false,
  trayAvailable: false,
  reportedByBackend: false,
}

function runStatusClassName(status: SourceSyncRun['status'] | SyncPlanRun['status']): string {
  switch (status) {
    case 'failed':
      return 'status status-failed'
    case 'warning':
      return 'status status-warning'
    case 'skipped':
      return 'status status-skipped'
    case 'idle':
      return 'status status-ready'
    default:
      return 'status status-succeeded'
  }
}

function labelForCommand(command?: string): string {
  if (!command) {
    return 'Idle'
  }

  return command
    .split('_')
    .filter((part) => part.length > 0)
    .map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`)
    .join(' ')
}

export function OperatorConsole() {
  const snapshot = useAppStore((state) => state.snapshot)
  const pendingCommand = useAppStore((state) => state.pendingCommand)
  const operatorSilentMode = useAppStore((state) => state.operatorSilentMode)
  const toggleOperatorSilentMode = useAppStore((state) => state.toggleOperatorSilentMode)
  const routeAction = useAppStore((state) => state.routeAction)

  const sourceSyncRuns = snapshot?.sourceSyncRuns ?? EMPTY_SOURCE_SYNC_RUNS
  const syncPlanRuns = snapshot?.syncPlanRuns ?? EMPTY_SYNC_PLAN_RUNS
  const desktopRuntime = snapshot?.desktopRuntime ?? EMPTY_DESKTOP_RUNTIME

  const failedRunCount = useMemo(
    () =>
      sourceSyncRuns.filter((run) => run.status === 'failed').length
      + syncPlanRuns.filter((run) => run.status === 'failed').length,
    [sourceSyncRuns, syncPlanRuns],
  )
  const runtimeManagedSilentMode = desktopRuntime.reportedByBackend ?? false
  const effectiveSilentMode = runtimeManagedSilentMode ? desktopRuntime.silentMode : operatorSilentMode
  const desktopRuntimeLabel = desktopRuntime.trayAvailable
    ? desktopRuntime.closeToTray
      ? 'Close-to-tray enabled'
      : 'Foreground window mode'
    : 'Tray bridge unavailable'

  return (
    <section className="panel operator-shell">
      <div className="operator-column">
        <div className="panel-header">
          <div>
            <p className="eyebrow">Operator rail</p>
            <h2>Foreground state, queue lanes, and desktop routing</h2>
          </div>
          <button
            className="ghost-button"
            onClick={toggleOperatorSilentMode}
            type="button"
          >
            {effectiveSilentMode ? 'Disable silent mode' : 'Enable silent mode'}
          </button>
        </div>

        {runtimeManagedSilentMode ? (
          <div className="operator-banner">
            Silent mode and tray behavior are being persisted by the desktop runtime. Changes here
            write through the Tauri bridge and stay aligned with the tray menu.
          </div>
        ) : effectiveSilentMode ? (
          <div className="operator-banner">
            Silent mode is local to this shell for now. Runtime events stay visible here while backend
            desktop persistence catches up.
          </div>
        ) : null}

        <div className="operator-summary-grid">
          <article className="operator-summary-card">
            <span>Foreground operation</span>
            <strong>{labelForCommand(pendingCommand)}</strong>
            <small>{pendingCommand ? 'Current mutation is running through the Tauri bridge.' : 'No active foreground mutation.'}</small>
          </article>
          <article className="operator-summary-card">
            <span>Plan runs</span>
            <strong>{syncPlanRuns.length}</strong>
            <small>Recent scheduler executions persisted in the workspace</small>
          </article>
          <article className="operator-summary-card">
            <span>Desktop runtime</span>
            <strong>{desktopRuntimeLabel}</strong>
            <small>{effectiveSilentMode ? 'Silent mode active' : 'Audible mode active'}</small>
          </article>
          <article className="operator-summary-card">
            <span>Failed runs</span>
            <strong>{failedRunCount}</strong>
            <small>{sourceSyncRuns.length + syncPlanRuns.length} recent runtime entries</small>
          </article>
          <article className="operator-summary-card">
            <span>Source queue</span>
            <strong>{sourceSyncRuns.length}</strong>
            <small>Recent account-bound source sync results</small>
          </article>
        </div>

        <div className="operator-lanes">
          <article className="operator-card">
            <div className="panel-header compact-header">
              <div>
                <p className="eyebrow">Source lane</p>
                <h2>Recent source sync queue</h2>
              </div>
              <button className="ghost-button operator-mini-button" onClick={() => routeAction('sources')} type="button">
                Open sources page
              </button>
            </div>
            <div className="operator-list">
              {sourceSyncRuns.slice(0, 4).map((run) => (
                <button
                  key={run.id}
                  className="operator-list-row"
                  onClick={() => routeAction('sources')}
                  type="button"
                >
                  <div>
                    <strong>{run.summary}</strong>
                    <small>{run.provider} · {run.trigger} · {run.finishedAt}</small>
                  </div>
                  <span className={runStatusClassName(run.status)}>
                    {run.status}
                  </span>
                </button>
              ))}
              {sourceSyncRuns.length === 0 ? (
                <div className="operator-empty">No source sync history yet.</div>
              ) : null}
            </div>
          </article>

          <article className="operator-card">
            <div className="panel-header compact-header">
              <div>
                <p className="eyebrow">Scheduler lane</p>
                <h2>Recent plan execution results</h2>
              </div>
              <button className="ghost-button operator-mini-button" onClick={() => routeAction('scheduler')} type="button">
                Open scheduler page
              </button>
            </div>
            <div className="operator-list">
              {syncPlanRuns.slice(0, 4).map((run) => (
                <button
                  key={run.id}
                  className="operator-list-row"
                  onClick={() => routeAction('scheduler')}
                  type="button"
                >
                  <div>
                    <strong>{run.summary}</strong>
                    <small>{run.trigger} · {run.sourceCount} sources · {run.finishedAt}</small>
                  </div>
                  <span className={runStatusClassName(run.status)}>
                    {run.status}
                  </span>
                </button>
              ))}
              {syncPlanRuns.length === 0 ? (
                <div className="operator-empty">No scheduler execution history yet.</div>
              ) : null}
            </div>
          </article>
        </div>
      </div>
    </section>
  )
}
