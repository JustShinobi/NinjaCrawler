import { useEffect, useMemo, useState } from 'react'
import {
  subscribeToAccountsWindowIntent,
} from '../../bridge/desktop'
import type { AccountsWindowIntent, ProviderKey } from '../../domain/models'
import { useAppStore } from '../../state/appStore'
import { WindowShell } from '../brand/WindowShell'
import { WindowTitlebar } from '../brand/WindowTitlebar'
import { AccountsPage } from './AccountsPage'

const PROVIDERS: ProviderKey[] = ['instagram', 'tiktok', 'twitter']

function normalizeIntent(intent: AccountsWindowIntent | undefined): AccountsWindowIntent {
  const provider = intent?.initialProvider && PROVIDERS.includes(intent.initialProvider)
    ? intent.initialProvider
    : undefined
  const mode = intent?.initialMode === 'create' || intent?.initialMode === 'edit'
    ? intent.initialMode
    : undefined

  return {
    initialAccountId: intent?.initialAccountId?.trim() || undefined,
    initialProvider: provider,
    initialMode: mode,
  }
}

function intentSignature(intent: AccountsWindowIntent): string {
  return `${intent.initialAccountId ?? 'none'}:${intent.initialProvider ?? 'none'}:${intent.initialMode ?? 'none'}`
}

interface AccountsWindowPageProps {
  initialIntent?: AccountsWindowIntent
}

export function AccountsWindowPage({ initialIntent }: AccountsWindowPageProps) {
  const bootstrap = useAppStore((state) => state.bootstrap)
  const loading = useAppStore((state) => state.loading)
  const snapshot = useAppStore((state) => state.snapshot)
  const error = useAppStore((state) => state.error)
  const [activeIntent, setActiveIntent] = useState<AccountsWindowIntent>(() => normalizeIntent(initialIntent))
  const [intentRevision, setIntentRevision] = useState(0)
  const [isDirty, setIsDirty] = useState(false)
  const signature = useMemo(() => intentSignature(activeIntent), [activeIntent])

  useEffect(() => {
    void bootstrap()
  }, [bootstrap])

  useEffect(() => {
    let disposed = false
    let unsubscribe: (() => void) | undefined

    void subscribeToAccountsWindowIntent((incomingIntent) => {
      const nextIntent = normalizeIntent(incomingIntent)
      const nextSignature = intentSignature(nextIntent)
      if (nextSignature === signature) {
        return
      }

      const shouldSwitch = !isDirty
        || typeof window === 'undefined'
        || typeof window.confirm !== 'function'
        || window.confirm('You have unsaved account changes. Discard and switch context?')
      if (!shouldSwitch) {
        return
      }

      setIsDirty(false)
      setActiveIntent(nextIntent)
      setIntentRevision((current) => current + 1)
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
  }, [isDirty, signature])

  const titlebarTrailing = isDirty ? (
    <span className="window-titlebar-status-meta source-editor-titlebar-dirty">Unsaved changes</span>
  ) : null

  if (loading) {
    return (
      <WindowShell
        className="accounts-window-shell"
        contentClassName="accounts-window-content"
        titlebar={<WindowTitlebar title="Accounts editor" />}
      >
        <div className="loading-shell source-editor-loading">Loading accounts…</div>
      </WindowShell>
    )
  }

  if (!snapshot) {
    return (
      <WindowShell
        className="accounts-window-shell"
        contentClassName="accounts-window-content"
        titlebar={<WindowTitlebar title="Accounts editor" />}
      >
        <div className="loading-shell source-editor-loading" role="alert">
          Failed to load workspace: {error ?? 'missing snapshot'}
        </div>
      </WindowShell>
    )
  }

  return (
    <WindowShell
      className="accounts-window-shell"
      contentClassName="accounts-window-content"
      titlebar={<WindowTitlebar title="Accounts editor" trailing={titlebarTrailing} />}
    >
      <AccountsPage
        key={`${intentRevision}:${signature}`}
        initialAccountId={activeIntent.initialAccountId}
        initialMode={activeIntent.initialMode}
        initialProvider={activeIntent.initialProvider}
        onDirtyChange={setIsDirty}
      />
    </WindowShell>
  )
}
