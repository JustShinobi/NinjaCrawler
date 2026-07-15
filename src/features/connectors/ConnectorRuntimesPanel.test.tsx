// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ConnectorRuntimeStatus } from '../../domain/models'
import { createEmptyWorkspaceSnapshot } from '../../domain/workspaceSnapshot'
import { ConnectorRuntimesPanel } from './ConnectorRuntimesPanel'

const useAppStoreMock = vi.fn()

vi.mock('../../state/appStore', () => ({
  useAppStore: (selector: (state: Record<string, unknown>) => unknown) => useAppStoreMock(selector),
}))

function runtime(overrides: Partial<ConnectorRuntimeStatus> = {}): ConnectorRuntimeStatus {
  return {
    key: 'gallery-dl',
    displayName: 'gallery-dl',
    managementMode: 'managed',
    activeVersion: '1.31.9',
    bundledVersion: '1.31.9',
    latestVersion: '1.31.10',
    updateAvailable: true,
    status: 'update_available',
    ...overrides,
  }
}

function renderPanel(runtimes: ConnectorRuntimeStatus[]) {
  const store = {
    snapshot: { ...createEmptyWorkspaceSnapshot(), connectorRuntimes: runtimes },
    pendingCommand: undefined,
    checkConnectorUpdates: vi.fn(),
    updateConnectorRuntime: vi.fn(),
    setConnectorCustomOverride: vi.fn().mockResolvedValue(undefined),
    clearConnectorCustomOverride: vi.fn(),
  }
  useAppStoreMock.mockImplementation((selector: (state: typeof store) => unknown) => selector(store))
  return { store, ...render(<ConnectorRuntimesPanel />) }
}

describe('ConnectorRuntimesPanel', () => {
  beforeEach(() => {
    useAppStoreMock.mockReset()
  })
  afterEach(() => cleanup())

  it('prioritizes update actions only when an update is available', () => {
    const { store } = renderPanel([
      runtime(),
      runtime({ key: 'yt-dlp', displayName: 'yt-dlp', updateAvailable: false, status: 'up_to_date' }),
    ])

    expect(screen.getAllByRole('button', { name: 'Update' })).toHaveLength(1)
    fireEvent.click(screen.getByRole('button', { name: 'Update' }))
    expect(store.updateConnectorRuntime).toHaveBeenCalledWith('gallery-dl')
  })

  it('shows custom mode controls and validates the path next to the field', () => {
    const { store } = renderPanel([
      runtime({ managementMode: 'custom', status: 'custom_override', updateAvailable: false, customPath: '' }),
    ])

    expect(screen.getByRole('button', { name: 'Use managed' })).not.toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Apply' }))
    expect(screen.getByRole('alert').textContent).toContain('Enter the full path')

    fireEvent.change(screen.getByPlaceholderText('Custom executable path…'), { target: { value: 'D:\\Tools\\gallery-dl.exe' } })
    fireEvent.click(screen.getByRole('button', { name: 'Apply' }))
    expect(store.setConnectorCustomOverride).toHaveBeenCalledWith('gallery-dl', 'D:\\Tools\\gallery-dl.exe')
  })

  it('provides one recovery action when no runtimes are registered', () => {
    const { store } = renderPanel([])
    fireEvent.click(screen.getByRole('button', { name: 'Check again' }))
    expect(store.checkConnectorUpdates).toHaveBeenCalledWith()
  })
})
