import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { ProviderAccountCookie, ProviderKey } from '../../domain/models'
import { useAppStore } from '../../state/appStore'

interface CookieEditorDialogProps {
  accountId?: string
  initialCookies?: ProviderAccountCookie[]
  onSaveDraftCookies?: (cookies: ProviderAccountCookie[]) => void
  provider: ProviderKey
  providerLabel: string
  onClose: () => void
}

interface CookieImportDraft {
  format: 'json' | 'netscape'
  content: string
}

function createEmptyCookie(): ProviderAccountCookie {
  return {
    domain: '',
    name: '',
    value: '',
    path: '/',
    expiresAt: undefined,
    secure: false,
    httpOnly: false,
  }
}

function describeCookie(cookie: ProviderAccountCookie): string {
  const domain = cookie.domain.trim() || '(domain)'
  const name = cookie.name.trim() || '(name)'
  return `${name} @ ${domain}`
}

function normalizeImportedCookie(value: unknown): ProviderAccountCookie | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null
  }

  const record = value as Record<string, unknown>
  const name = typeof record.name === 'string' ? record.name.trim() : ''
  const domain = typeof record.domain === 'string' ? record.domain.trim() : ''
  const valueText = typeof record.value === 'string' ? record.value : ''

  if (!name || !domain) {
    return null
  }

  return {
    domain,
    name,
    value: valueText,
    path: typeof record.path === 'string' && record.path.trim() ? record.path.trim() : '/',
    expiresAt: typeof record.expiresAt === 'string' && record.expiresAt.trim() ? record.expiresAt.trim() : undefined,
    secure: Boolean(record.secure),
    httpOnly: Boolean(record.httpOnly),
  }
}

function parseJsonCookieImport(content: string): ProviderAccountCookie[] {
  const parsed = JSON.parse(content)
  if (!Array.isArray(parsed)) {
    throw new Error('JSON cookie import must be an array.')
  }

  const cookies = parsed
    .map((entry) => normalizeImportedCookie(entry))
    .filter((entry): entry is ProviderAccountCookie => entry !== null)

  if (cookies.length === 0) {
    throw new Error('No valid cookies were found in the JSON payload.')
  }

  return cookies
}

function parseNetscapeCookieImport(content: string): ProviderAccountCookie[] {
  const cookies: ProviderAccountCookie[] = []

  for (const rawLine of content.split(/\r?\n/)) {
    const line = rawLine.trim()
    if (!line || (line.startsWith('#') && !line.startsWith('#HttpOnly_'))) {
      continue
    }

    const parts = rawLine.split('\t')
    if (parts.length < 7) {
      continue
    }

    const httpOnly = parts[0].startsWith('#HttpOnly_')
    const domain = parts[0].replace(/^#HttpOnly_/, '').trim()
    const path = parts[2]?.trim() || '/'
    const secure = parts[3]?.trim().toUpperCase() === 'TRUE'
    const expires = parts[4]?.trim()
    const name = parts[5]?.trim()
    const value = parts.slice(6).join('\t')

    if (!domain || !name) {
      continue
    }

    const expiresNumber = Number(expires)
    cookies.push({
      domain,
      name,
      value,
      path,
      expiresAt:
        Number.isFinite(expiresNumber) && expiresNumber > 0
          ? new Date(expiresNumber * 1000).toISOString()
          : undefined,
      secure,
      httpOnly,
    })
  }

  if (cookies.length === 0) {
    throw new Error('No valid Netscape cookies were found in the pasted text.')
  }

  return cookies
}

function parseImportedCookies(
  format: CookieImportDraft['format'],
  content: string,
): ProviderAccountCookie[] {
  return format === 'json'
    ? parseJsonCookieImport(content)
    : parseNetscapeCookieImport(content)
}

export function CookieEditorDialog({
  accountId,
  initialCookies,
  onSaveDraftCookies,
  provider,
  providerLabel,
  onClose,
}: CookieEditorDialogProps) {
  const pendingCommand = useAppStore((state) => state.pendingCommand)
  const loadProviderAccountCookies = useAppStore((state) => state.loadProviderAccountCookies)
  const saveProviderAccountCookies = useAppStore((state) => state.saveProviderAccountCookies)
  const importProviderAccountCookies = useAppStore((state) => state.importProviderAccountCookies)
  const clearProviderAccountCookies = useAppStore((state) => state.clearProviderAccountCookies)

  const draftMode = !accountId
  const persistedAccountId = accountId ?? ''
  const initialDraftCookiesSnapshot = useMemo(
    () => (initialCookies ?? []).map((cookie) => ({ ...cookie })),
    [initialCookies],
  )
  const [cookies, setCookies] = useState<ProviderAccountCookie[]>(
    () => (draftMode ? initialDraftCookiesSnapshot : []),
  )
  const [selectedIndex, setSelectedIndex] = useState(
    draftMode
      ? (initialDraftCookiesSnapshot.length > 0 ? 0 : -1)
      : 0,
  )
  const [loading, setLoading] = useState(!draftMode)
  const [error, setError] = useState<string>()
  const [importMenuOpen, setImportMenuOpen] = useState(false)
  const [importDraft, setImportDraft] = useState<CookieImportDraft>()
  const importTextareaRef = useRef<HTMLTextAreaElement>(null)

  const handleClose = useCallback(() => {
    setImportMenuOpen(false)
    setImportDraft(undefined)
    onClose()
  }, [onClose])

  useEffect(() => {
    let cancelled = false

    if (draftMode) {
      return () => {
        cancelled = true
      }
    }

    void loadProviderAccountCookies(persistedAccountId)
      .then((loaded) => {
        if (cancelled) {
          return
        }

        setCookies(loaded)
        setSelectedIndex(loaded.length > 0 ? 0 : -1)
      })
      .catch((loadError) => {
        if (cancelled) {
          return
        }

        setError(loadError instanceof Error ? loadError.message : 'Failed to load stored cookies.')
        setCookies([])
        setSelectedIndex(-1)
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false)
        }
      })

    return () => {
      cancelled = true
    }
  }, [draftMode, loadProviderAccountCookies, persistedAccountId])

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        if (importMenuOpen) {
          setImportMenuOpen(false)
          return
        }

        if (importDraft) {
          setImportDraft(undefined)
          return
        }

        handleClose()
      }
    }

    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [handleClose, importDraft, importMenuOpen])

  useEffect(() => {
    if (!importDraft) {
      return
    }

    importTextareaRef.current?.focus()
  }, [importDraft])

  const selectedCookie = selectedIndex >= 0 ? cookies[selectedIndex] : undefined
  const busy = !draftMode && (
    pendingCommand === 'load_provider_account_cookies'
      || pendingCommand === 'save_provider_account_cookies'
      || pendingCommand === 'import_provider_account_cookies'
      || pendingCommand === 'clear_provider_account_cookies'
  )

  const cookieCountLabel = useMemo(() => {
    const count = cookies.length
    return `${count} cookie${count === 1 ? '' : 's'}`
  }, [cookies.length])

  function updateSelectedCookie(patch: Partial<ProviderAccountCookie>) {
    if (selectedIndex < 0) {
      return
    }

    setCookies((current) => current.map((cookie, index) => (
      index === selectedIndex
        ? { ...cookie, ...patch }
        : cookie
    )))
  }

  function handleAddCookie() {
    setCookies((current) => {
      const next = [...current, createEmptyCookie()]
      setSelectedIndex(next.length - 1)
      return next
    })
    setError(undefined)
  }

  function handleDeleteCookie() {
    if (selectedIndex < 0) {
      return
    }

    setCookies((current) => {
      const next = current.filter((_, index) => index !== selectedIndex)
      setSelectedIndex(next.length === 0 ? -1 : Math.min(selectedIndex, next.length - 1))
      return next
    })
    setError(undefined)
  }

  async function handleSaveCookies() {
    setError(undefined)

    if (draftMode) {
      onSaveDraftCookies?.(cookies)
      handleClose()
      return
    }

    if (!persistedAccountId) {
      setError('Missing persisted account context for cookie saving.')
      return
    }

    try {
      await saveProviderAccountCookies(persistedAccountId, cookies)
      handleClose()
    } catch (saveError) {
      setError(saveError instanceof Error ? saveError.message : 'Failed to save cookies.')
    }
  }

  async function handleClearStoredCookies() {
    setError(undefined)

    if (draftMode) {
      setCookies([])
      setSelectedIndex(-1)
      return
    }

    if (!persistedAccountId) {
      setError('Missing persisted account context for cookie clearing.')
      return
    }

    try {
      await clearProviderAccountCookies(persistedAccountId)
      setCookies([])
      setSelectedIndex(-1)
      handleClose()
    } catch (clearError) {
      setError(clearError instanceof Error ? clearError.message : 'Failed to clear cookies.')
    }
  }

  function openImportPrompt(importFormat: 'json' | 'netscape') {
    setImportMenuOpen(false)
    setError(undefined)
    setImportDraft({
      format: importFormat,
      content: '',
    })
  }

  async function handleImportContent() {
    if (!importDraft) {
      return
    }

    const content = importDraft.content.trim()
    if (!content) {
      setError('Cookie import content cannot be empty.')
      return
    }

    setError(undefined)

    try {
      if (draftMode) {
        const parsedCookies = parseImportedCookies(importDraft.format, content)
        setCookies(parsedCookies)
        setSelectedIndex(parsedCookies.length > 0 ? 0 : -1)
        setImportDraft(undefined)
        return
      }

      if (!persistedAccountId) {
        setError('Missing persisted account context for cookie import.')
        return
      }

      await importProviderAccountCookies({
        accountId: persistedAccountId,
        importFormat: importDraft.format,
        content,
      })
      const loaded = await loadProviderAccountCookies(persistedAccountId)
      setCookies(loaded)
      setSelectedIndex(loaded.length > 0 ? 0 : -1)
      setImportDraft(undefined)
    } catch (importError) {
      setError(importError instanceof Error ? importError.message : 'Failed to import cookies.')
    }
  }

  return (
    <div className="accounts-cookie-dialog-backdrop" onMouseDown={(event) => {
      if (event.target === event.currentTarget) {
        handleClose()
      }
    }}>
      <section className="accounts-cookie-dialog" role="dialog" aria-modal="true" aria-label={`${providerLabel} cookies`}>
        <header className="accounts-cookie-dialog-header">
          <div>
            <h3>{providerLabel} cookies</h3>
            <p className="accounts-cookie-dialog-note">
              {draftMode
                ? 'Draft cookies are saved when you create the account.'
                : 'Changes save immediately for this account when you click Save cookies.'}
            </p>
          </div>
          <div className="accounts-cookie-dialog-summary">
            <span>{provider}</span>
            <strong>{cookieCountLabel}</strong>
          </div>
        </header>

        <div className="accounts-cookie-toolbar">
          <button className="ghost-button" disabled={busy} onClick={handleAddCookie} type="button">Add</button>
          <button className="ghost-button" disabled={busy || selectedIndex < 0} onClick={handleDeleteCookie} type="button">Delete</button>
          <div className="accounts-cookie-import" data-menu-root>
            <button className="ghost-button" disabled={busy} onClick={() => setImportMenuOpen((current) => !current)} type="button">
              Import cookies
            </button>
            {importMenuOpen ? (
              <div className="accounts-cookie-import-menu">
                <button className="menu-item-button" onClick={() => openImportPrompt('json')} type="button">Paste JSON</button>
                <button className="menu-item-button" onClick={() => openImportPrompt('netscape')} type="button">Paste Netscape</button>
              </div>
            ) : null}
          </div>
          <button className="danger-button" disabled={busy} onClick={() => void handleClearStoredCookies()} type="button">Clear</button>
        </div>

        <div className="accounts-cookie-body">
          <div className="accounts-cookie-list-panel">
            {loading ? (
              <div className="accounts-cookie-empty">Loading cookies...</div>
            ) : cookies.length === 0 ? (
              <div className="accounts-cookie-empty">
                {draftMode ? 'No draft cookies yet.' : 'No cookies stored for this account.'}
              </div>
            ) : (
              <div className="accounts-cookie-list" role="listbox" aria-label="Stored cookies">
                {cookies.map((cookie, index) => (
                  <button
                    aria-selected={index === selectedIndex}
                    className={`accounts-cookie-list-item${index === selectedIndex ? ' accounts-cookie-list-item-active' : ''}`}
                    key={`${cookie.domain}-${cookie.name}-${index}`}
                    onClick={() => setSelectedIndex(index)}
                    role="option"
                    type="button"
                  >
                    <strong>{describeCookie(cookie)}</strong>
                    <span>{cookie.path || '/'}</span>
                  </button>
                ))}
              </div>
            )}
          </div>

          <div className="accounts-cookie-editor-panel">
            {selectedCookie ? (
              <div className="accounts-cookie-form-grid">
                <label className="accounts-config-field">
                  <span>Name</span>
                  <input onChange={(event) => updateSelectedCookie({ name: event.target.value })} value={selectedCookie.name} />
                </label>
                <label className="accounts-config-field">
                  <span>Domain</span>
                  <input onChange={(event) => updateSelectedCookie({ domain: event.target.value })} value={selectedCookie.domain} />
                </label>
                <label className="accounts-config-field accounts-cookie-form-wide">
                  <span>Value</span>
                  <textarea onChange={(event) => updateSelectedCookie({ value: event.target.value })} rows={6} spellCheck={false} value={selectedCookie.value} />
                </label>
                <label className="accounts-config-field">
                  <span>Path</span>
                  <input onChange={(event) => updateSelectedCookie({ path: event.target.value })} value={selectedCookie.path} />
                </label>
                <label className="accounts-config-field">
                  <span>Expires at</span>
                  <input onChange={(event) => updateSelectedCookie({ expiresAt: event.target.value || undefined })} placeholder="2026-03-31T00:00:00Z" value={selectedCookie.expiresAt ?? ''} />
                </label>
                <label className="accounts-config-row accounts-config-row-toggle">
                  <span>Secure</span>
                  <div className="accounts-config-row-toggle-value">
                    <input checked={selectedCookie.secure} onChange={(event) => updateSelectedCookie({ secure: event.target.checked })} type="checkbox" />
                  </div>
                </label>
                <label className="accounts-config-row accounts-config-row-toggle">
                  <span>HTTP only</span>
                  <div className="accounts-config-row-toggle-value">
                    <input checked={selectedCookie.httpOnly} onChange={(event) => updateSelectedCookie({ httpOnly: event.target.checked })} type="checkbox" />
                  </div>
                </label>
              </div>
            ) : (
              <div className="accounts-cookie-empty">Select a cookie or add a new one.</div>
            )}
          </div>
        </div>

        {error ? <div className="accounts-cookie-error">{error}</div> : null}

        <footer className="accounts-cookie-dialog-footer">
          <button className="ghost-button" disabled={busy} onClick={handleClose} type="button">Cancel</button>
          <button className="primary-button" disabled={busy || cookies.length === 0} onClick={() => void handleSaveCookies()} type="button">
            Save cookies
          </button>
        </footer>

        {importDraft ? (
          <div className="accounts-cookie-import-overlay">
            <section
              aria-label="Import cookies"
              className="accounts-cookie-import-dialog"
              role="dialog"
            >
              <header className="accounts-cookie-import-dialog-header">
                <div>
                  <h4>Paste cookie text</h4>
                </div>
                <span className="accounts-cookie-import-dialog-format">
                  {importDraft.format === 'json' ? 'JSON' : 'Netscape'}
                </span>
              </header>
              <div className="accounts-cookie-import-dialog-body">
                <div className="accounts-cookie-import-label">
                  Cookie text
                  <span>
                    {importDraft.format === 'json'
                      ? 'Paste JSON cookie data.'
                      : 'Paste Netscape cookie lines.'}
                  </span>
                </div>
                <label className="accounts-config-field">
                  <textarea
                    aria-label="Cookie text"
                    onChange={(event) =>
                      setImportDraft((current) =>
                        current
                          ? {
                              ...current,
                              content: event.target.value,
                            }
                          : current,
                      )
                    }
                    placeholder={
                      importDraft.format === 'json'
                        ? '[{"domain":".instagram.com","name":"sessionid","value":"..."}]'
                        : '# Netscape HTTP Cookie File'
                    }
                    ref={importTextareaRef}
                    rows={12}
                    spellCheck={false}
                    value={importDraft.content}
                  />
                </label>
              </div>
              <footer className="accounts-cookie-import-dialog-footer">
                <button
                  className="ghost-button"
                  disabled={busy}
                  onClick={() => setImportDraft(undefined)}
                  type="button"
                >
                  Cancel
                </button>
                <button className="primary-button" disabled={busy} onClick={() => void handleImportContent()} type="button">
                  OK
                </button>
              </footer>
            </section>
          </div>
        ) : null}
      </section>
    </div>
  )
}
