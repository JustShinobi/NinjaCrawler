// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { RuntimeLogWindowPage } from './RuntimeLogWindowPage'

const bridgeMocks = vi.hoisted(() => ({
  loadRuntimeLogContext: vi.fn(),
  queryRuntimeLogs: vi.fn(),
  reportRuntimeLogWindowReady: vi.fn(),
  subscribeToDesktopRuntimeEvents: vi.fn(),
  subscribeToRuntimeLogWindowIntent: vi.fn(),
}))

const runtimeLogContext = {
  providerCatalog: [
    {
      key: 'instagram',
      displayName: 'Instagram',
      authModes: ['imported_session'],
      sourceKinds: ['profile'],
      supportsMultipleAccounts: true,
      defaultCapabilities: [],
      notes: '',
    },
  ],
  accounts: [
    {
      id: 'account-1',
      provider: 'instagram',
      displayName: 'Instagram Main',
      authMode: 'imported_session',
      authState: 'ready',
      capabilities: [],
      lastValidatedAt: '2026-03-10T00:00:00Z',
    },
  ],
}

vi.mock('../../bridge/desktop', () => bridgeMocks)

function listContainsText(text: string) {
  return screen
    .getAllByRole('listitem')
    .some((entry) => entry.textContent?.toLowerCase().includes(text.toLowerCase()))
}

describe('RuntimeLogWindowPage', () => {
  beforeEach(() => {
    bridgeMocks.loadRuntimeLogContext.mockReset()
    bridgeMocks.queryRuntimeLogs.mockReset()
    bridgeMocks.reportRuntimeLogWindowReady.mockReset()
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockReset()
    bridgeMocks.subscribeToRuntimeLogWindowIntent.mockReset()
    bridgeMocks.reportRuntimeLogWindowReady.mockResolvedValue(undefined)
    bridgeMocks.loadRuntimeLogContext.mockResolvedValue(runtimeLogContext)
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockResolvedValue(() => undefined)
    bridgeMocks.subscribeToRuntimeLogWindowIntent.mockResolvedValue(() => undefined)
  })

  afterEach(() => {
    cleanup()
  })

  it('loads runtime log entries on open and subscribes to live runtime log events', async () => {
    let appendedHandler: ((entry: {
      id: string
      timestamp: string
      scope: string
      level: 'info' | 'warning' | 'error' | 'debug'
      provider?: string
      accountId?: string
      sourceHandle?: string
      message: string
      detail?: string
    }) => void) | undefined

    bridgeMocks.subscribeToDesktopRuntimeEvents.mockImplementation(
      async (handlers: { onRuntimeLogAppended?: typeof appendedHandler }) => {
        appendedHandler = handlers.onRuntimeLogAppended
        return () => undefined
      },
    )
    bridgeMocks.queryRuntimeLogs.mockResolvedValueOnce([
      {
        id: 'log-1',
        timestamp: '2026-03-11T12:00:00Z',
        scope: 'sync.run',
        level: 'info',
        provider: 'instagram',
        accountId: 'account-1',
        sourceHandle: '@alpha',
        message: 'Initial log entry',
      },
    ])

    render(<RuntimeLogWindowPage />)

    await waitFor(() => {
      expect(bridgeMocks.loadRuntimeLogContext).toHaveBeenCalledTimes(1)
      expect(bridgeMocks.reportRuntimeLogWindowReady).toHaveBeenCalledTimes(1)
      expect(bridgeMocks.queryRuntimeLogs).toHaveBeenCalledWith({
        limit: 500,
        level: undefined,
        scope: undefined,
        provider: undefined,
        accountId: undefined,
      })
      expect(bridgeMocks.subscribeToDesktopRuntimeEvents).toHaveBeenCalledTimes(1)
    })

    await waitFor(() => {
      expect(listContainsText('Initial log entry')).toBe(true)
    })

    appendedHandler?.({
      id: 'log-2',
      timestamp: '2026-03-11T12:00:01Z',
      scope: 'sync.run',
      level: 'warning',
      provider: 'instagram',
      accountId: 'account-1',
      sourceHandle: '@alpha',
      message: 'Live runtime event',
      detail: 'Cancellation requested by user.',
    })

    await waitFor(() => {
      expect(listContainsText('Live runtime event')).toBe(true)
    })
    expect(listContainsText('Cancellation requested by user.')).toBe(true)
  })

  it('requeries when filters change and ignores live entries outside the active filters', async () => {
    let appendedHandler: ((entry: {
      id: string
      timestamp: string
      scope: string
      level: 'info' | 'warning' | 'error' | 'debug'
      provider?: string
      accountId?: string
      message: string
    }) => void) | undefined

    bridgeMocks.subscribeToDesktopRuntimeEvents.mockImplementation(
      async (handlers: { onRuntimeLogAppended?: typeof appendedHandler }) => {
        appendedHandler = handlers.onRuntimeLogAppended
        return () => undefined
      },
    )
    bridgeMocks.queryRuntimeLogs
      .mockResolvedValueOnce([
        {
          id: 'log-1',
          timestamp: '2026-03-11T12:00:00Z',
          scope: 'sync.run',
          level: 'info',
          provider: 'instagram',
          accountId: 'account-1',
          message: 'Initial log entry',
        },
      ])
      .mockResolvedValueOnce([
        {
          id: 'log-2',
          timestamp: '2026-03-11T12:00:01Z',
          scope: 'connector.runtime',
          level: 'info',
          provider: 'instagram',
          accountId: 'account-1',
          message: 'Filtered log entry',
        },
      ])

    render(<RuntimeLogWindowPage />)

    await waitFor(() => {
      expect(listContainsText('Initial log entry')).toBe(true)
    })

    fireEvent.change(screen.getByLabelText(/^scope$/i), {
      target: { value: 'connector' },
    })

    await waitFor(() => {
      expect(bridgeMocks.queryRuntimeLogs).toHaveBeenLastCalledWith({
        limit: 500,
        level: undefined,
        scope: 'connector',
        provider: undefined,
        accountId: undefined,
      })
    })

    appendedHandler?.({
      id: 'log-3',
      timestamp: '2026-03-11T12:00:02Z',
      scope: 'sync.run',
      level: 'info',
      provider: 'instagram',
      accountId: 'account-1',
      message: 'Ignored live entry',
    })

    expect(screen.queryByText(/ignored live entry/i)).toBeNull()
    await waitFor(() => {
      expect(listContainsText('Filtered log entry')).toBe(true)
    })
    expect(screen.queryByRole('button', { name: /^refresh$/i })).toBeNull()
  })
})
