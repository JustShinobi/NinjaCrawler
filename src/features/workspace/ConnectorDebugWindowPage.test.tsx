// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ConnectorDebugEntry } from '../../domain/models'
import { ConnectorDebugWindowPage } from './ConnectorDebugWindowPage'

const bridgeMocks = vi.hoisted(() => ({
  clearConnectorDebug: vi.fn(),
  queryConnectorDebug: vi.fn(),
  subscribeToConnectorDebug: vi.fn(),
}))

vi.mock('../../bridge/desktop', () => bridgeMocks)

const entry = (overrides: Partial<ConnectorDebugEntry> = {}): ConnectorDebugEntry => ({
  id: 'debug-1',
  timestamp: '2026-07-02T22:00:00.000Z',
  sourceId: 'source-1',
  provider: 'tiktok',
  sourceHandle: '@profile',
  connector: 'yt-dlp',
  eventType: 'stdout',
  operation: 'process.output',
  raw: '[download] 42.0% of 10MiB',
  ...overrides,
})

describe('ConnectorDebugWindowPage', () => {
  beforeEach(() => {
    for (const mock of Object.values(bridgeMocks)) mock.mockReset()
    bridgeMocks.queryConnectorDebug.mockResolvedValue([entry()])
    bridgeMocks.clearConnectorDebug.mockResolvedValue(undefined)
    bridgeMocks.subscribeToConnectorDebug.mockResolvedValue(() => undefined)
    Element.prototype.scrollTo = vi.fn()
  })

  afterEach(cleanup)

  it('renders raw history and appends connector events live', async () => {
    let liveHandler: ((entry: ConnectorDebugEntry) => void) | undefined
    bridgeMocks.subscribeToConnectorDebug.mockImplementation(
      async (handler: (entry: ConnectorDebugEntry) => void) => {
        liveHandler = handler
        return () => undefined
      },
    )

    render(<ConnectorDebugWindowPage />)
    expect(await screen.findByText('[download] 42.0% of 10MiB')).toBeTruthy()

    liveHandler?.(entry({
      id: 'debug-2',
      eventType: 'stderr',
      raw: 'ERROR: HTTP Error 403: Forbidden',
    }))

    expect(await screen.findByText('ERROR: HTTP Error 403: Forbidden')).toBeTruthy()
  })

  it('filters events and clears the dedicated debug buffer', async () => {
    bridgeMocks.queryConnectorDebug.mockResolvedValue([
      entry(),
      entry({ id: 'debug-2', provider: 'instagram', eventType: 'response', raw: 'HTTP 200' }),
    ])
    render(<ConnectorDebugWindowPage />)
    await screen.findByText('HTTP 200')

    fireEvent.change(screen.getByLabelText('Provider'), { target: { value: 'instagram' } })
    expect(screen.queryByText('[download] 42.0% of 10MiB')).toBeNull()
    expect(screen.getByText('HTTP 200')).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: 'Clear' }))
    await waitFor(() => expect(bridgeMocks.clearConnectorDebug).toHaveBeenCalledTimes(1))
    expect(screen.getByText(/waiting for connector activity/i)).toBeTruthy()
  })

  it('reconciles missed live events without reopening the window', async () => {
    bridgeMocks.queryConnectorDebug
      .mockResolvedValueOnce([entry()])
      .mockResolvedValue([
        entry({ id: 'debug-2', eventType: 'response', raw: 'batch completed automatically' }),
        entry(),
      ])
    bridgeMocks.subscribeToConnectorDebug.mockResolvedValue(() => undefined)

    render(<ConnectorDebugWindowPage />)
    expect(await screen.findByText('[download] 42.0% of 10MiB')).toBeTruthy()

    expect(
      await screen.findByText('batch completed automatically', {}, { timeout: 2000 }),
    ).toBeTruthy()
    expect(bridgeMocks.queryConnectorDebug.mock.calls.length).toBeGreaterThanOrEqual(2)
  })
})
