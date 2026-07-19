// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  getMigrationStatus: vi.fn(),
  runPendingMigrations: vi.fn(),
  subscribeToMigrationProgress: vi.fn(),
  subscribeToMigrationCompletion: vi.fn(),
  openBackupsFolder: vi.fn(),
  closeDesktopWindow: vi.fn(),
}))

vi.mock('../../bridge/desktop', () => ({
  getMigrationStatus: mocks.getMigrationStatus,
  runPendingMigrations: mocks.runPendingMigrations,
  subscribeToMigrationProgress: mocks.subscribeToMigrationProgress,
  subscribeToMigrationCompletion: mocks.subscribeToMigrationCompletion,
  openBackupsFolder: mocks.openBackupsFolder,
}))
vi.mock('../../utils/closeDesktopWindow', () => ({
  closeDesktopWindow: mocks.closeDesktopWindow,
}))

import { MigrationGate } from './MigrationGate'

describe('MigrationGate', () => {
  beforeEach(() => {
    for (const mock of Object.values(mocks)) mock.mockReset()
    mocks.subscribeToMigrationProgress.mockResolvedValue(() => undefined)
  })
  afterEach(() => cleanup())

  it('renders the app directly when no migration is pending', async () => {
    mocks.getMigrationStatus.mockResolvedValue(null)

    render(
      <MigrationGate>
        <div>APP READY</div>
      </MigrationGate>,
    )

    expect(await screen.findByText('APP READY')).toBeTruthy()
    expect(mocks.runPendingMigrations).not.toHaveBeenCalled()
  })

  it('shows the confirm screen, runs the migration, then renders the app', async () => {
    mocks.getMigrationStatus.mockResolvedValue({
      fromVersion: 41,
      toVersion: 42,
      pendingCount: 1,
      dbSizeBytes: 1073741824,
    })
    mocks.runPendingMigrations.mockResolvedValue(undefined)

    render(
      <MigrationGate>
        <div>APP READY</div>
      </MigrationGate>,
    )

    // App stays hidden behind the confirm screen; DB size is shown.
    const confirm = await screen.findByRole('button', { name: /back up.*update/i })
    expect(screen.queryByText('APP READY')).toBeNull()
    expect(screen.getByText(/1\.0 GB/)).toBeTruthy()

    fireEvent.click(confirm)

    await waitFor(() => expect(mocks.runPendingMigrations).toHaveBeenCalled())
    expect(await screen.findByText('APP READY')).toBeTruthy()
  })

  it('shows an error screen with a retry when the migration fails', async () => {
    mocks.getMigrationStatus.mockResolvedValue({
      fromVersion: 41,
      toVersion: 42,
      pendingCount: 1,
      dbSizeBytes: 512,
    })
    mocks.runPendingMigrations.mockRejectedValueOnce(new Error('disk full'))

    render(
      <MigrationGate>
        <div>APP READY</div>
      </MigrationGate>,
    )

    fireEvent.click(await screen.findByRole('button', { name: /back up.*update/i }))

    expect(await screen.findByText(/update failed/i)).toBeTruthy()
    expect(screen.getByText(/disk full/)).toBeTruthy()
    expect(screen.getByRole('button', { name: /retry/i })).toBeTruthy()
    const openBackups = screen.getByRole('button', { name: /open backups folder/i })
    fireEvent.click(openBackups)
    await waitFor(() => expect(mocks.openBackupsFolder).toHaveBeenCalledTimes(1))

    fireEvent.click(screen.getByRole('button', { name: /close ninjacrawler/i }))
    expect(mocks.closeDesktopWindow).toHaveBeenCalledTimes(1)
  })

  it('reports a failure to open the backups folder', async () => {
    mocks.getMigrationStatus.mockResolvedValue({
      fromVersion: 41,
      toVersion: 42,
      pendingCount: 1,
      dbSizeBytes: 512,
    })
    mocks.runPendingMigrations.mockRejectedValueOnce(new Error('migration failed'))
    mocks.openBackupsFolder.mockRejectedValueOnce(new Error('Explorer is unavailable'))

    render(
      <MigrationGate>
        <div>APP READY</div>
      </MigrationGate>,
    )

    fireEvent.click(await screen.findByRole('button', { name: /back up.*update/i }))
    fireEvent.click(await screen.findByRole('button', { name: /open backups folder/i }))

    expect((await screen.findByRole('alert')).textContent).toContain('Explorer is unavailable')
  })
})
