import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  loadRuntimeLogContext,
  queryRuntimeLogs,
  reportRuntimeLogWindowReady,
  subscribeToDesktopRuntimeEvents,
} from '../../bridge/desktop'
import type {
  ProviderAccount,
  ProviderDescriptor,
  ProviderKey,
  RuntimeLogEntry,
} from '../../domain/models'
import { WindowShell } from '../brand/WindowShell'
import { WindowTitlebar } from '../brand/WindowTitlebar'

const EMPTY_LOGS: RuntimeLogEntry[] = []
const EMPTY_PROVIDERS: ProviderDescriptor[] = []
const EMPTY_ACCOUNTS: ProviderAccount[] = []
const MAX_RENDERED_LOGS = 500
const HIGHLIGHT_WORDS = new Set([
  'ready',
  'degraded',
  'expired',
  'queued',
  'started',
  'cancelled',
  'failed',
  'warning',
  'error',
  'update',
  'downloaded',
  'activation',
])

function formatTimestamp(value: string): string {
  const parsed = new Date(value)
  if (Number.isNaN(parsed.getTime())) {
    return value
  }

  return parsed.toLocaleString()
}

function buildLogQuery(
  level: string,
  scope: string,
  provider: string,
  accountId: string,
): {
  limit: number
  level?: 'info' | 'warning' | 'error' | 'debug'
  scope?: string
  provider?: ProviderKey
  accountId?: string
} {
  return {
    limit: MAX_RENDERED_LOGS,
    level: level === 'all' ? undefined : (level as 'info' | 'warning' | 'error' | 'debug'),
    scope: scope === 'all' ? undefined : scope,
    provider: provider === 'all' ? undefined : (provider as ProviderKey),
    accountId: accountId === 'all' ? undefined : accountId,
  }
}

function matchesFilters(
  entry: RuntimeLogEntry,
  filters: {
    level: string
    scope: string
    provider: string
    accountId: string
  },
): boolean {
  if (filters.level !== 'all' && entry.level !== filters.level) {
    return false
  }
  if (filters.scope !== 'all' && !entry.scope.toLowerCase().includes(filters.scope.toLowerCase())) {
    return false
  }
  if (filters.provider !== 'all' && entry.provider !== filters.provider) {
    return false
  }
  if (filters.accountId !== 'all' && entry.accountId !== filters.accountId) {
    return false
  }
  return true
}

function prependLiveEntry(entries: RuntimeLogEntry[], nextEntry: RuntimeLogEntry): RuntimeLogEntry[] {
  const deduped = entries.filter((entry) => entry.id !== nextEntry.id)
  return [nextEntry, ...deduped].slice(0, MAX_RENDERED_LOGS)
}

function renderHighlightedText(value: string) {
  const parts = value.split(/(@[A-Za-z0-9._-]+|'[^']+'|\b\d{3}\b|\b[a-z][a-z_]+\b)/gi)
  return parts.map((part, index) => {
    const normalized = part.toLowerCase()
    let className = 'runtime-log-token'

    if (!part) {
      return null
    }
    if (/^@/i.test(part)) {
      className += ' runtime-log-token-handle'
    } else if (/^'[^']+'$/i.test(part)) {
      className += ' runtime-log-token-string'
    } else if (/^\d{3}$/.test(part)) {
      className += ' runtime-log-token-number'
    } else if (HIGHLIGHT_WORDS.has(normalized)) {
      className += ' runtime-log-token-keyword'
    }

    return (
      <span className={className} key={`${part}-${index}`}>
        {part}
      </span>
    )
  })
}

function resolveAccountLabel(id: string, accounts: ProviderAccount[]): string {
  return accounts.find((a) => a.id === id)?.displayName ?? id.slice(0, 8)
}

function renderScope(scope: string) {
  return scope.split('.').map((segment) => (
    <span className="runtime-log-scope-segment" key={`${scope}-${segment}`}>
      {segment}
    </span>
  ))
}

export function RuntimeLogWindowPage() {
  const [accounts, setAccounts] = useState<ProviderAccount[]>(EMPTY_ACCOUNTS)
  const [providerCatalog, setProviderCatalog] = useState<ProviderDescriptor[]>(EMPTY_PROVIDERS)
  const [logs, setLogs] = useState<RuntimeLogEntry[]>(EMPTY_LOGS)
  const [contextReady, setContextReady] = useState(false)
  const [contextLoading, setContextLoading] = useState(false)
  const [loading, setLoading] = useState(false)
  const [contextError, setContextError] = useState<string>()
  const [queryError, setQueryError] = useState<string>()
  const [level, setLevel] = useState<string>('all')
  const [scope, setScope] = useState<string>('all')
  const [provider, setProvider] = useState<string>('all')
  const [accountId, setAccountId] = useState<string>('all')
  const contextRequestRef = useRef(0)
  const queryRequestRef = useRef(0)

  const accountOptions = useMemo(() => {
    return accounts
      .filter((account) => provider === 'all' || account.provider === provider)
      .map((account) => ({
        id: account.id,
        label: `${account.displayName} [${account.provider}]`,
      }))
  }, [accounts, provider])

  const providerOptions = useMemo(() => providerCatalog, [providerCatalog])
  const activeError = contextError ?? queryError

  const loadContext = useCallback(async () => {
    const requestId = contextRequestRef.current + 1
    contextRequestRef.current = requestId
    setContextLoading(true)
    setContextError(undefined)

    try {
      const context = await loadRuntimeLogContext()
      if (contextRequestRef.current !== requestId) {
        return
      }
      setProviderCatalog(context.providerCatalog)
      setAccounts(context.accounts)
      setContextReady(true)
    } catch (contextError) {
      if (contextRequestRef.current !== requestId) {
        return
      }
      setContextError(
        contextError instanceof Error ? contextError.message : 'Failed to load runtime log context.',
      )
      setProviderCatalog(EMPTY_PROVIDERS)
      setAccounts(EMPTY_ACCOUNTS)
      setContextReady(true)
    } finally {
      if (contextRequestRef.current === requestId) {
        setContextLoading(false)
      }
    }
  }, [])

  const runQuery = useCallback(
    async (silent = false) => {
      const requestId = queryRequestRef.current + 1
      queryRequestRef.current = requestId
      if (!silent) {
        setLoading(true)
      }
      setQueryError(undefined)

      try {
        const entries = await queryRuntimeLogs(buildLogQuery(level, scope, provider, accountId))
        if (queryRequestRef.current !== requestId) {
          return
        }
        setLogs(entries)
      } catch (queryError) {
        if (queryRequestRef.current !== requestId) {
          return
        }
        setQueryError(
          queryError instanceof Error ? queryError.message : 'Failed to query runtime log.',
        )
        if (!silent) {
          setLogs(EMPTY_LOGS)
        }
      } finally {
        if (!silent && queryRequestRef.current === requestId) {
          setLoading(false)
        }
      }
    },
    [accountId, level, provider, scope],
  )

  useEffect(() => {
    void reportRuntimeLogWindowReady()
  }, [])

  useEffect(() => {
    void loadContext()
  }, [loadContext])

  useEffect(() => {
    if (!contextReady) {
      return
    }

    void runQuery()
  }, [contextReady, runQuery])

  useEffect(() => {
    if (accountId !== 'all' && !accountOptions.some((account) => account.id === accountId)) {
      setAccountId('all')
    }
  }, [accountId, accountOptions])

  useEffect(() => {
    let disposed = false
    let unsubscribe: (() => void) | undefined

    void subscribeToDesktopRuntimeEvents({
      onRuntimeLogAppended: (entry) => {
        if (
          disposed ||
          !matchesFilters(entry, {
            level,
            scope,
            provider,
            accountId,
          })
        ) {
          return
        }

        setLogs((current) => prependLiveEntry(current, entry))
        setQueryError(undefined)
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
  }, [accountId, level, provider, scope])

  useEffect(() => {
    if (!contextReady) {
      return
    }

    const reconcile = () => {
      if (document.visibilityState === 'hidden') {
        return
      }
      void runQuery(true)
    }

    window.addEventListener('focus', reconcile)
    document.addEventListener('visibilitychange', reconcile)

    return () => {
      window.removeEventListener('focus', reconcile)
      document.removeEventListener('visibilitychange', reconcile)
    }
  }, [contextReady, runQuery])

  return (
    <WindowShell
      density="compact"
      titlebar={
        <WindowTitlebar
          title="Runtime Log"
          trailing={
            <span className="window-titlebar-status-meta">
              {logs.length} · {loading ? 'Syncing' : 'Watching'}
            </span>
          }
        />
      }
    >
      <div className="runtime-log-window-body">
        <section className="runtime-log-toolbar panel">
          <div className="runtime-log-window-filters">
            <label className="accounts-config-field">
              <span>Type</span>
              <select onChange={(event) => setLevel(event.target.value)} value={level}>
                <option value="all">All</option>
                <option value="info">Info</option>
                <option value="warning">Warning</option>
                <option value="error">Error</option>
                <option value="debug">Debug</option>
              </select>
            </label>

            <label className="accounts-config-field">
              <span>Scope</span>
              <input
                onChange={(event) => setScope(event.target.value.trim() || 'all')}
                placeholder="all"
                value={scope === 'all' ? '' : scope}
              />
            </label>

            <label className="accounts-config-field">
              <span>Provider</span>
              <select
                disabled={contextLoading}
                onChange={(event) => setProvider(event.target.value)}
                value={provider}
              >
                <option value="all">All</option>
                {providerOptions.map((entry) => (
                  <option key={entry.key} value={entry.key}>
                    {entry.displayName}
                  </option>
                ))}
              </select>
            </label>

            <label className="accounts-config-field">
              <span>Account</span>
              <select
                disabled={contextLoading}
                onChange={(event) => setAccountId(event.target.value)}
                value={accountId}
              >
                <option value="all">All</option>
                {accountOptions.map((entry) => (
                  <option key={entry.id} value={entry.id}>
                    {entry.label}
                  </option>
                ))}
              </select>
            </label>
          </div>
        </section>

        <section className="runtime-log-window-table panel">
          {activeError ? <div className="runtime-log-window-error">{activeError}</div> : null}
          {loading ? <div className="runtime-log-window-empty">Loading runtime log...</div> : null}
          {!loading && logs.length === 0 ? (
            <div className="runtime-log-window-empty">No log entries matched the current filters.</div>
          ) : null}
          {!loading && logs.length > 0 ? (
            <div className="runtime-log-feed" role="list">
              {logs.map((entry) => (
                <article className="runtime-log-entry" data-level={entry.level} key={entry.id} role="listitem">
                  <div className="runtime-log-entry-meta">
                    <span className={`runtime-log-level runtime-log-level-${entry.level}`}>
                      {entry.level}
                    </span>
                    <span className="runtime-log-entry-time">{formatTimestamp(entry.timestamp)}</span>
                    <span className="runtime-log-entry-scope">{renderScope(entry.scope)}</span>
                  </div>
                  <p className="runtime-log-entry-message">{renderHighlightedText(entry.message)}</p>
                  <div className="runtime-log-entry-facts">
                    {entry.provider ? <span className="runtime-log-fact">{entry.provider}</span> : null}
                    {entry.accountId ? (
                      <span className="runtime-log-fact">
                        {resolveAccountLabel(entry.accountId, accounts)}
                      </span>
                    ) : null}
                    {entry.sourceHandle ? (
                      <span className="runtime-log-fact runtime-log-fact-handle">{entry.sourceHandle}</span>
                    ) : null}
                  </div>
                  {entry.detail ? (
                    <pre className="runtime-log-entry-detail">{renderHighlightedText(entry.detail)}</pre>
                  ) : null}
                </article>
              ))}
            </div>
          ) : null}
        </section>
      </div>
    </WindowShell>
  )
}
