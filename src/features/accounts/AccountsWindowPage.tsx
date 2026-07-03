import { useEffect, useMemo, useState } from 'react'
import {
  subscribeToAccountsWindowIntent,
} from '../../bridge/desktop'
import type { AccountsWindowIntent, ProviderKey } from '../../domain/models'
import { useAppStore } from '../../state/appStore'
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

  return (
    <div className="accounts-window-shell">
      <AccountsPage
        key={`${intentRevision}:${signature}`}
        initialAccountId={activeIntent.initialAccountId}
        initialMode={activeIntent.initialMode}
        initialProvider={activeIntent.initialProvider}
        onDirtyChange={setIsDirty}
      />
    </div>
  )
}
