// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { WorkspaceSnapshot } from '../../domain/models'
import { createEmptyWorkspaceSnapshot } from '../../domain/workspaceSnapshot'
import { SettingsPage } from './SettingsPage'

const useAppStoreMock = vi.fn()

vi.mock('../../state/appStore', () => ({
  useAppStore: (selector: (state: Record<string, unknown>) => unknown) => useAppStoreMock(selector),
}))

function renderPage(snapshotOverrides: Partial<WorkspaceSnapshot> = {}) {
  const snapshot: WorkspaceSnapshot = {
    ...createEmptyWorkspaceSnapshot(),
    ...snapshotOverrides,
  }

  const store = {
    snapshot,
    pendingCommand: undefined,
    upsertAppSetting: vi.fn(),
  }

  useAppStoreMock.mockImplementation((selector: (state: typeof store) => unknown) => selector(store))
  return { store, ...render(<SettingsPage />) }
}

describe('SettingsPage', () => {
  beforeEach(() => {
    useAppStoreMock.mockReset()
    localStorage.clear()
    document.documentElement.removeAttribute('data-theme')
  })

  afterEach(() => {
    cleanup()
  })

  it('keeps only cross-cutting preferences and hides domain homes', () => {
    renderPage({
      appSettings: [
        {
          key: 'runtime.scheduler.launch_at',
          value: '2026-03-12T00:00:00Z',
          category: 'general',
          description: 'Internal timestamp.',
          mutable: true,
        },
        {
          key: 'imports.instagram.scrawler.disabledRoots',
          value: '[]',
          category: 'general',
          description: 'Owned by Import.',
          mutable: true,
        },
        {
          key: 'policy.session_import.enabled',
          value: 'true',
          category: 'policy',
          description: 'Owned by Accounts Workspace.',
          mutable: true,
        },
        {
          key: 'policy.notifications.default',
          value: 'summary',
          category: 'policy',
          description: 'Owned by Plans.',
          mutable: true,
        },
        {
          key: 'storage.media_root',
          value: 'F:\\Data',
          category: 'storage',
          description: 'Owned by About.',
          mutable: true,
        },
        {
          key: 'policy.desktop.close_to_tray',
          value: 'true',
          category: 'policy',
          description: 'Close to tray instead of exiting.',
          mutable: true,
        },
        {
          key: 'naming.instagram.media_file_pattern_mode',
          value: 'preset_new_default',
          category: 'storage',
          description: 'Instagram naming.',
          mutable: true,
        },
      ],
    })

    expect(screen.getByRole('navigation', { name: /preference sections/i })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Appearance' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Desktop' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Media naming' })).toBeTruthy()

    expect(screen.queryByText(/session import/i)).toBeNull()
    expect(screen.queryByText(/plan notifications/i)).toBeNull()
    expect(screen.queryByDisplayValue('F:\\Data')).toBeNull()
    expect(screen.queryByDisplayValue('2026-03-12T00:00:00Z')).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'Desktop' }))
    expect(screen.getByRole('checkbox', { name: 'Close to tray' })).not.toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'Media naming' }))
    expect(screen.getByLabelText('Instagram file naming')).not.toBeNull()
  })

  it('saves Instagram naming enum immediately', () => {
    const { store } = renderPage({
      appSettings: [
        {
          key: 'naming.instagram.media_file_pattern_mode',
          value: 'preset_new_default',
          category: 'storage',
          description: 'Controls how Instagram media file names are generated.',
          mutable: true,
        },
      ],
    })

    fireEvent.click(screen.getByRole('button', { name: 'Media naming' }))
    fireEvent.change(screen.getByLabelText('Instagram file naming'), {
      target: { value: 'custom' },
    })

    expect(store.upsertAppSetting).toHaveBeenCalledWith({
      key: 'naming.instagram.media_file_pattern_mode',
      value: 'custom',
      category: 'storage',
      description: 'Controls how Instagram media file names are generated.',
      mutable: true,
    })
  })

  it('toggles dark mode from Appearance', () => {
    renderPage()

    fireEvent.click(screen.getByRole('checkbox', { name: 'Dark theme' }))

    expect(document.documentElement.getAttribute('data-theme')).toBe('dark')
    expect(localStorage.getItem('nc-theme')).toBe('dark')
  })
})
