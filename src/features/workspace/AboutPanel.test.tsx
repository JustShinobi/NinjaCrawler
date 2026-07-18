// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { AboutPanel } from './AboutPanel'

const buildInfo = {
  version: '0.18.2',
  commitSha: 'abc1234',
  dirty: false,
  channel: 'release' as const,
  displayVersion: 'v0.18.2',
}

function renderPanel(overrides: Partial<Parameters<typeof AboutPanel>[0]> = {}) {
  const props: Parameters<typeof AboutPanel>[0] = {
    accountCount: 6,
    buildInfo,
    databasePath: 'C:\\Users\\ninja\\AppData\\Local\\NinjaCrawler\\data\\ninjacrawler.db',
    mediaRoot: 'F:\\SCrawler\\Data',
    onCheckUpdate: vi.fn(),
    onInstallUpdate: vi.fn(),
    onOpenRelease: vi.fn(),
    planCount: 2,
    profileCount: 1322,
    updateChecking: false,
    updateInstalling: false,
    updateStatus: {
      build: buildInfo,
      latestVersion: '0.19.0',
      releaseUrl: 'https://github.com/JustShinobi/NinjaCrawler/releases/tag/v0.19.0',
      updateAvailable: true,
    },
    workspaceRoot: 'C:\\Users\\ninja\\AppData\\Local\\NinjaCrawler',
    ...overrides,
  }
  return { props, ...render(<AboutPanel {...props} />) }
}

describe('AboutPanel', () => {
  beforeEach(() => {
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
    })
  })
  afterEach(() => cleanup())

  it('shows build identity and the lightweight GitHub release action', () => {
    const { props } = renderPanel()
    expect(screen.getByText('v0.18.2')).not.toBeNull()
    expect(screen.getByText('A newer release is ready')).not.toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'View / Download v0.19.0 on GitHub' }))
    expect(props.onOpenRelease).toHaveBeenCalledWith(props.updateStatus?.releaseUrl)
  })

  it('copies long paths without exposing them as layout-sized controls', async () => {
    const { props } = renderPanel()
    fireEvent.click(screen.getAllByRole('button', { name: 'Copy' })[0])

    await waitFor(() => expect(screen.getByRole('button', { name: 'Copied' })).not.toBeNull())
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(props.workspaceRoot)
  })

  it('announces update errors next to the update action', () => {
    renderPanel({ updateError: 'GitHub is unavailable.', updateStatus: undefined })
    expect(screen.getByRole('alert').textContent).toContain('GitHub is unavailable')
  })
})
