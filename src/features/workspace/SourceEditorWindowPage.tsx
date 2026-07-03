import { useEffect, useMemo, useState } from 'react'
import { closeDesktopWindow } from '../../utils/closeDesktopWindow'
import {
  emitFocusSourceRequest,
  openAccountsWindow,
  subscribeToSourceEditorWindowIntent,
} from '../../bridge/desktop'
import type {
  SourceEditorWindowIntent,
  ProviderKey,
} from '../../domain/models'
import { useAppStore } from '../../state/appStore'
import { SourceEditorDialog } from './SourceEditorDialog'

const PROVIDERS: ProviderKey[] = ['instagram', 'tiktok', 'reddit', 'twitter']

function normalizeIntent(
  intent: SourceEditorWindowIntent | undefined,
): SourceEditorWindowIntent {
  const preferredProvider = intent?.preferredProvider
    && PROVIDERS.includes(intent.preferredProvider)
    ? intent.preferredProvider
    : undefined
  const sourceId = intent?.sourceId?.trim() || undefined
  const preferredAccountId = intent?.preferredAccountId?.trim() || undefined
  const seed = intent?.seed
    && PROVIDERS.includes(intent.seed.provider)
    && intent.seed.handle.trim().length > 0
    ? {
        provider: intent.seed.provider,
        handle: intent.seed.handle.trim(),
        displayName: intent.seed.displayName.trim() || intent.seed.handle.trim().replace(/^@+/, ''),
      }
    : undefined

  return {
    sourceId,
    preferredProvider,
    preferredAccountId,
    seed,
  }
}

function intentSignature(intent: SourceEditorWindowIntent): string {
  return [
    intent.sourceId ?? 'none',
    intent.preferredProvider ?? 'none',
    intent.preferredAccountId ?? 'none',
    intent.seed?.provider ?? 'none',
    intent.seed?.handle ?? 'none',
    intent.seed?.displayName ?? 'none',
  ].join(':')
}

interface SourceEditorWindowPageProps {
  initialIntent?: SourceEditorWindowIntent
}

export function SourceEditorWindowPage({
  initialIntent,
}: SourceEditorWindowPageProps) {
  const bootstrap = useAppStore((state) => state.bootstrap)
  const loading = useAppStore((state) => state.loading)
  const snapshot = useAppStore((state) => state.snapshot)
  const error = useAppStore((state) => state.error)
  const [activeIntent, setActiveIntent] = useState<SourceEditorWindowIntent>(
    () => normalizeIntent(initialIntent),
  )
  const [intentRevision, setIntentRevision] = useState(0)
  const [isDirty, setIsDirty] = useState(false)
  const signature = useMemo(() => intentSignature(activeIntent), [activeIntent])

  useEffect(() => {
    void bootstrap()
  }, [bootstrap])

  useEffect(() => {
    let disposed = false
    let unsubscribe: (() => void) | undefined

    void subscribeToSourceEditorWindowIntent((incomingIntent) => {
      const nextIntent = normalizeIntent(incomingIntent)
      const nextSignature = intentSignature(nextIntent)
      if (nextSignature === signature) {
        return
      }

      const shouldSwitch = !isDirty
        || typeof window === 'undefined'
        || typeof window.confirm !== 'function'
        || window.confirm('Discard and switch profile?')
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

  if (loading) {
    return <div className="app-shell loading-shell">Loading profile editor...</div>
  }

  if (!snapshot) {
    return <div className="app-shell loading-shell">Failed to load workspace: {error ?? 'missing snapshot'}</div>
  }

  const source = activeIntent.sourceId
    ? snapshot.sources.find((entry) => entry.id === activeIntent.sourceId)
    : undefined

  return (
    <div className="profile-editor-window-shell">
      <SourceEditorDialog
        key={`${intentRevision}:${signature}`}
        preferredProvider={activeIntent.preferredProvider}
        preferredAccountId={activeIntent.preferredAccountId}
        onAdvancedAccountSettings={(accountId) =>
          void openAccountsWindow({ initialAccountId: accountId, initialMode: 'edit' })}
        onClose={() => void closeDesktopWindow()}
        onEditAccount={(accountId) =>
          void openAccountsWindow({ initialAccountId: accountId, initialMode: 'edit' })}
        onDirtyChange={setIsDirty}
        onSaved={(savedSource) =>
          void emitFocusSourceRequest(savedSource.id, {
            clearSearch: !activeIntent.sourceId,
          })}
        seed={activeIntent.seed}
        snapshot={snapshot}
        source={source}
      />
    </div>
  )
}
