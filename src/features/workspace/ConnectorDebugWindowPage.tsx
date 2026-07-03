import { useEffect, useMemo, useRef, useState } from 'react'
import {
  clearConnectorDebug,
  queryConnectorDebug,
  subscribeToConnectorDebug,
} from '../../bridge/desktop'
import type {
  ConnectorDebugEntry,
  ConnectorDebugEventType,
  ProviderKey,
} from '../../domain/models'

const EVENT_TYPES: Array<ConnectorDebugEventType | 'all'> = [
  'all',
  'call',
  'stdout',
  'stderr',
  'response',
  'error',
  'system',
]

function timestamp(value: string): string {
  const parsed = new Date(value)
  return Number.isNaN(parsed.getTime())
    ? value
    : parsed.toLocaleTimeString(undefined, { hour12: false, fractionalSecondDigits: 3 })
}

function mergeEntries(
  current: ConnectorDebugEntry[],
  incoming: ConnectorDebugEntry[],
): ConnectorDebugEntry[] {
  const byId = new Map(current.map((entry) => [entry.id, entry]))
  for (const entry of incoming) byId.set(entry.id, entry)
  return Array.from(byId.values())
    .sort((left, right) => Date.parse(left.timestamp) - Date.parse(right.timestamp))
    .slice(-5000)
}

export function ConnectorDebugWindowPage() {
  const [entries, setEntries] = useState<ConnectorDebugEntry[]>([])
  const [paused, setPaused] = useState(false)
  const [provider, setProvider] = useState<ProviderKey | 'all'>('all')
  const [eventType, setEventType] = useState<ConnectorDebugEventType | 'all'>('all')
  const [search, setSearch] = useState('')
  const [error, setError] = useState<string>()
  const [copied, setCopied] = useState(false)
  const feedRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let disposed = false
    let firstLoad = true
    let reconciling = false
    const reconcile = async () => {
      if (reconciling) return
      reconciling = true
      try {
        const result = await queryConnectorDebug({ limit: firstLoad ? 5000 : 1000 })
        firstLoad = false
        if (!disposed) {
          setEntries((current) => mergeEntries(current, [...result].reverse()))
          setError(undefined)
        }
      } catch (loadError) {
        if (!disposed) {
          setError(loadError instanceof Error ? loadError.message : 'Failed to load connector debug.')
        }
      } finally {
        reconciling = false
      }
    }
    void reconcile()
    const reconcileTimer = window.setInterval(() => void reconcile(), 750)

    let teardown: (() => void) | undefined
    void subscribeToConnectorDebug((entry) => {
      if (!disposed) {
        setEntries((current) => mergeEntries(current, [entry]))
      }
    }).then((nextTeardown) => {
      if (disposed) nextTeardown()
      else teardown = nextTeardown
    })
    return () => {
      disposed = true
      window.clearInterval(reconcileTimer)
      teardown?.()
    }
  }, [])

  const visibleEntries = useMemo(() => {
    const needle = search.trim().toLocaleLowerCase()
    return entries.filter((entry) => {
      if (provider !== 'all' && entry.provider !== provider) return false
      if (eventType !== 'all' && entry.eventType !== eventType) return false
      if (!needle) return true
      return `${entry.connector} ${entry.operation} ${entry.raw} ${entry.sourceHandle ?? ''}`
        .toLocaleLowerCase()
        .includes(needle)
    })
  }, [entries, eventType, provider, search])

  useEffect(() => {
    if (!paused) {
      feedRef.current?.scrollTo({ top: feedRef.current.scrollHeight })
    }
  }, [paused, visibleEntries])

  const copyVisible = async () => {
    const raw = visibleEntries
      .map(
        (entry) =>
          `[${entry.timestamp}] ${entry.eventType.toUpperCase()} ${entry.connector} ${entry.operation}\n${entry.raw}`,
      )
      .join('\n\n')
    await navigator.clipboard.writeText(raw)
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1200)
  }

  const clear = async () => {
    await clearConnectorDebug()
    setEntries([])
  }

  return (
    <main className="connector-debug-shell">
      <header className="connector-debug-toolbar">
        <div className="connector-debug-title">
          <span className={`connector-debug-live-dot${paused ? ' is-paused' : ''}`} />
          <div>
            <strong>Realtime Connector Debugger</strong>
            <small>Raw connector I/O · credentials are redacted</small>
          </div>
        </div>
        <div className="connector-debug-filters">
          <select aria-label="Provider" value={provider} onChange={(event) => setProvider(event.target.value as ProviderKey | 'all')}>
            <option value="all">All providers</option>
            <option value="instagram">Instagram</option>
            <option value="tiktok">TikTok</option>
            <option value="twitter">Twitter</option>
            <option value="reddit">Reddit</option>
          </select>
          <select aria-label="Event type" value={eventType} onChange={(event) => setEventType(event.target.value as ConnectorDebugEventType | 'all')}>
            {EVENT_TYPES.map((type) => <option key={type} value={type}>{type === 'all' ? 'All events' : type.toUpperCase()}</option>)}
          </select>
          <input aria-label="Search raw output" onChange={(event) => setSearch(event.target.value)} placeholder="Search raw output…" value={search} />
        </div>
        <div className="connector-debug-actions">
          <span>{visibleEntries.length} events</span>
          <button className="ghost-button" onClick={() => setPaused((value) => !value)} type="button">
            {paused ? 'Resume' : 'Pause'}
          </button>
          <button className="ghost-button" disabled={visibleEntries.length === 0} onClick={() => void copyVisible()} type="button">
            {copied ? 'Copied' : 'Copy raw'}
          </button>
          <button className="ghost-button" onClick={() => void clear()} type="button">Clear</button>
        </div>
      </header>

      {error ? <div className="runtime-log-window-error">{error}</div> : null}
      <div className="connector-debug-feed" ref={feedRef} role="log" aria-live={paused ? 'off' : 'polite'}>
        {visibleEntries.length === 0 ? (
          <div className="connector-debug-empty">Waiting for connector activity…</div>
        ) : visibleEntries.map((entry) => (
          <article className="connector-debug-line" data-event={entry.eventType} key={entry.id}>
            <div className="connector-debug-line-meta">
              <time>{timestamp(entry.timestamp)}</time>
              <b>{entry.eventType.toUpperCase()}</b>
              <span>{entry.connector}</span>
              {entry.provider ? <span>{entry.provider}</span> : null}
              {entry.sourceHandle ? <span>{entry.sourceHandle}</span> : null}
              <strong>{entry.operation}</strong>
            </div>
            <pre>{entry.raw}</pre>
          </article>
        ))}
      </div>
    </main>
  )
}
