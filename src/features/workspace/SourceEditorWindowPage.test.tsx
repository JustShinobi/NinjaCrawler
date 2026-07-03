// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createEmptyWorkspaceSnapshot } from '../../domain/workspaceSnapshot'
import { SourceEditorWindowPage } from './SourceEditorWindowPage'

const {
  bootstrapMock,
  closeWindowMock,
  emitFocusSourceRequestMock,
  openAccountsWindowMock,
  subscribeToSourceEditorWindowIntentMock,
  useAppStoreMock,
} = vi.hoisted(() => ({
  bootstrapMock: vi.fn(),
  closeWindowMock: vi.fn(),
  emitFocusSourceRequestMock: vi.fn(),
  openAccountsWindowMock: vi.fn(),
  subscribeToSourceEditorWindowIntentMock: vi.fn(),
  useAppStoreMock: vi.fn(),
}))

let sourceEditorIntentHandler: ((intent: {
  sourceId?: string
  preferredProvider?: 'instagram' | 'tiktok' | 'twitter'
  preferredAccountId?: string
  seed?: { provider: 'instagram' | 'tiktok' | 'twitter'; handle: string; displayName: string }
}) => void) | undefined

vi.mock('../../bridge/desktop', () => ({
  emitFocusSourceRequest: emitFocusSourceRequestMock,
  openAccountsWindow: openAccountsWindowMock,
  subscribeToSourceEditorWindowIntent: subscribeToSourceEditorWindowIntentMock,
}))

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: closeWindowMock }),
}))

vi.mock('../../state/appStore', () => ({
  useAppStore: (selector: (state: Record<string, unknown>) => unknown) => useAppStoreMock(selector),
}))

vi.mock('./SourceEditorDialog', () => ({
  SourceEditorDialog: ({
    onClose,
    onDirtyChange,
    onSaved,
    source,
  }: {
    onClose?: () => void
    onDirtyChange?: (dirty: boolean) => void
    onSaved?: (source: {
      id: string
      provider: 'instagram'
      sourceKind: 'profile'
      handle: string
      displayName: string
      labels: string[]
      readyForDownload: boolean
      remoteState: 'exists'
      isSubscription: boolean
      profileImageCustom: boolean
    }) => void
    source?: { handle: string }
  }) => (
    <div>
      <span>{source?.handle ?? 'new-profile'}</span>
      <button onClick={() => onClose?.()} type="button">
        close editor
      </button>
      <button onClick={() => onDirtyChange?.(true)} type="button">
        mark dirty
      </button>
      <button onClick={() => onDirtyChange?.(false)} type="button">
        mark clean
      </button>
      <button
        onClick={() =>
          onSaved?.({
            id: 'source-2',
            provider: 'instagram',
            sourceKind: 'profile',
            handle: '@beta',
            displayName: 'beta',
            labels: [],
            readyForDownload: true,
            remoteState: 'exists',
            isSubscription: false,
            profileImageCustom: false,
          })}
        type="button"
      >
        save editor
      </button>
    </div>
  ),
}))

describe('SourceEditorWindowPage', () => {
  beforeEach(() => {
    bootstrapMock.mockReset()
    closeWindowMock.mockReset()
    closeWindowMock.mockResolvedValue(undefined)
    emitFocusSourceRequestMock.mockReset()
    emitFocusSourceRequestMock.mockResolvedValue(undefined)
    openAccountsWindowMock.mockReset()
    subscribeToSourceEditorWindowIntentMock.mockReset()
    sourceEditorIntentHandler = undefined

    subscribeToSourceEditorWindowIntentMock.mockImplementation(
      async (handler: typeof sourceEditorIntentHandler) => {
        sourceEditorIntentHandler = handler
        return () => {
          sourceEditorIntentHandler = undefined
        }
      },
    )

    const snapshot = createEmptyWorkspaceSnapshot()
    snapshot.sources = [
      {
        id: 'source-1',
        provider: 'instagram',
        sourceKind: 'profile',
        handle: '@alpha',
        displayName: 'alpha',
        accountId: 'account-1',
        labels: [],
        readyForDownload: true,
        remoteState: 'exists',
        isSubscription: false,
        profileImageCustom: false,
      },
      {
        id: 'source-2',
        provider: 'instagram',
        sourceKind: 'profile',
        handle: '@beta',
        displayName: 'beta',
        accountId: 'account-1',
        labels: [],
        readyForDownload: true,
        remoteState: 'exists',
        isSubscription: false,
        profileImageCustom: false,
      },
    ]

    useAppStoreMock.mockImplementation((selector: (state: Record<string, unknown>) => unknown) =>
      selector({
        bootstrap: bootstrapMock,
        loading: false,
        snapshot,
        error: undefined,
      }))
  })

  afterEach(() => {
    cleanup()
    vi.restoreAllMocks()
  })

  it('switches context when receiving a new intent while clean', () => {
    render(<SourceEditorWindowPage initialIntent={{ sourceId: 'source-1' }} />)

    expect(screen.getByText('@alpha')).toBeTruthy()
    sourceEditorIntentHandler?.({ sourceId: 'source-2' })
    return waitFor(() => {
      expect(screen.getByText('@beta')).toBeTruthy()
    })
  })

  it('asks for confirmation before switching when editor is dirty', () => {
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false)
    render(<SourceEditorWindowPage initialIntent={{ sourceId: 'source-1' }} />)

    fireEvent.click(screen.getByRole('button', { name: 'mark dirty' }))
    sourceEditorIntentHandler?.({ sourceId: 'source-2' })

    expect(confirmSpy).toHaveBeenCalledWith('Discard and switch profile?')
    expect(screen.getByText('@alpha')).toBeTruthy()
  })

  it('does not expose run-sync or delete actions in this editor flow', () => {
    render(<SourceEditorWindowPage initialIntent={{ sourceId: 'source-1' }} />)

    expect(screen.queryByRole('button', { name: /run sync/i })).toBeNull()
    expect(screen.queryByRole('button', { name: /^delete$/i })).toBeNull()
  })

  it('closes the tauri webview when the dialog requests close', () => {
    render(<SourceEditorWindowPage initialIntent={{ sourceId: 'source-1' }} />)

    fireEvent.click(screen.getByRole('button', { name: 'close editor' }))

    expect(closeWindowMock).toHaveBeenCalledTimes(1)
  })

  it('asks the main window to select the profile saved by the editor', () => {
    render(<SourceEditorWindowPage initialIntent={{ sourceId: 'source-1' }} />)

    fireEvent.click(screen.getByRole('button', { name: 'save editor' }))

    expect(emitFocusSourceRequestMock).toHaveBeenCalledWith('source-2', {
      clearSearch: false,
    })
  })

  it('asks the main window to reveal a newly created profile', () => {
    render(<SourceEditorWindowPage />)

    fireEvent.click(screen.getByRole('button', { name: 'save editor' }))

    expect(emitFocusSourceRequestMock).toHaveBeenCalledWith('source-2', {
      clearSearch: true,
    })
  })
})
