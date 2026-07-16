// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SchedulerWindowPage } from './SchedulerWindowPage'

const bootstrapMock = vi.fn()
const refreshSnapshotMock = vi.fn()
const useAppStoreMock = vi.fn()
const deleteSyncPlanMock = vi.fn()
const runSyncPlanNowMock = vi.fn()
const setSyncPlanPauseMock = vi.fn()
const applySyncPlanSkipMock = vi.fn()
const moveSyncPlanMock = vi.fn()
const upsertSchedulerSetMock = vi.fn()

const bridgeMocks = vi.hoisted(() => ({
  openPlansWindow: vi.fn(),
  subscribeToDesktopRuntimeEvents: vi.fn(),
}))

vi.mock('../../bridge/desktop', () => bridgeMocks)
vi.mock('../../state/appStore', () => ({
  useAppStore: (selector: (state: Record<string, unknown>) => unknown) => useAppStoreMock(selector),
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    close: vi.fn(),
    isFocused: () => Promise.resolve(true),
    isMaximized: () => Promise.resolve(false),
    minimize: vi.fn(),
    onFocusChanged: () => Promise.resolve(() => undefined),
    onResized: () => Promise.resolve(() => undefined),
    startDragging: vi.fn(),
    toggleMaximize: vi.fn(),
    setTitle: vi.fn(() => Promise.resolve()),
  }),
}))

function renderWindow(snapshot?: {
  schedulerSets: Array<{
    id: string
    name: string
    active: boolean
    plans: Array<{
      id: string
      schedulerSetId: string
      name: string
      enabled: boolean
      paused: boolean
      pauseMode: string
      mode: 'automatic' | 'manual'
      lastRunStatus: 'idle' | 'succeeded' | 'failed' | 'skipped'
      lastRunAt?: string
      nextDueAt?: string
      lastRunSummary?: string
    }>
  }>
}) {
  const store = {
    bootstrap: bootstrapMock,
    refreshSnapshot: refreshSnapshotMock,
    snapshot,
    pendingCommand: undefined,
    deleteSyncPlan: deleteSyncPlanMock,
    runSyncPlanNow: runSyncPlanNowMock,
    setSyncPlanPause: setSyncPlanPauseMock,
    applySyncPlanSkip: applySyncPlanSkipMock,
    moveSyncPlan: moveSyncPlanMock,
    upsertSchedulerSet: upsertSchedulerSetMock,
  }

  useAppStoreMock.mockImplementation((selector: (state: typeof store) => unknown) => selector(store))
  return render(<SchedulerWindowPage />)
}

describe('SchedulerWindowPage', () => {
  beforeEach(() => {
    bootstrapMock.mockReset()
    bootstrapMock.mockResolvedValue(undefined)
    refreshSnapshotMock.mockReset()
    refreshSnapshotMock.mockResolvedValue(undefined)
    useAppStoreMock.mockReset()
    deleteSyncPlanMock.mockReset()
    deleteSyncPlanMock.mockResolvedValue(undefined)
    runSyncPlanNowMock.mockReset()
    runSyncPlanNowMock.mockResolvedValue(undefined)
    setSyncPlanPauseMock.mockReset()
    setSyncPlanPauseMock.mockResolvedValue(undefined)
    applySyncPlanSkipMock.mockReset()
    applySyncPlanSkipMock.mockResolvedValue(undefined)
    moveSyncPlanMock.mockReset()
    moveSyncPlanMock.mockResolvedValue(undefined)
    upsertSchedulerSetMock.mockReset()
    upsertSchedulerSetMock.mockImplementation(async (draft) => ({
      schedulerSets: [
        {
          id: draft.id ?? 'set-created',
          name: draft.name,
          active: draft.active,
          plans: [],
        },
      ],
    }))
    bridgeMocks.openPlansWindow.mockReset()
    bridgeMocks.openPlansWindow.mockResolvedValue(undefined)
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockReset()
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockResolvedValue(() => undefined)
  })

  afterEach(() => {
    cleanup()
  })

  it('bootstraps the scheduler window and shows a loading state before snapshot is ready', async () => {
    renderWindow(undefined)

    await waitFor(() => {
      expect(bootstrapMock).toHaveBeenCalledTimes(1)
      expect(bridgeMocks.subscribeToDesktopRuntimeEvents).toHaveBeenCalledTimes(1)
    })

    expect(screen.getByText(/loading scheduler/i)).toBeTruthy()
  })

  it('renders the compact list and opens the plans editor from toolbar actions', async () => {
    renderWindow({
      schedulerSets: [
        {
          id: 'set-1',
          name: 'Default Scheduler',
          active: true,
          plans: [
            {
              id: 'plan-1',
              schedulerSetId: 'set-1',
              name: 'Instagram Sweep',
              enabled: true,
              paused: false,
              pauseMode: 'disabled',
              mode: 'automatic',
              lastRunStatus: 'failed',
              lastRunAt: '2026-03-16T10:00:00Z',
              nextDueAt: '2026-03-16T10:30:00Z',
            },
          ],
        },
      ],
    })

    expect(screen.getByText('Scheduler')).toBeTruthy()
    expect(screen.getByText('Instagram Sweep')).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: 'Edit' }))

    await waitFor(() => {
      expect(bridgeMocks.openPlansWindow).toHaveBeenCalledWith({
        mode: 'edit',
        planId: 'plan-1',
        schedulerSetId: 'set-1',
      })
    })
  })

  it('runs start and start-force actions separately and refreshes on scheduler ticks', async () => {
    let schedulerTick: (() => void) | undefined
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockImplementation(
      async (handlers: { onSchedulerTick?: () => void }) => {
        schedulerTick = handlers.onSchedulerTick
        return () => undefined
      },
    )

    renderWindow({
      schedulerSets: [
        {
          id: 'set-1',
          name: 'Default Scheduler',
          active: true,
          plans: [
            {
              id: 'plan-1',
              schedulerSetId: 'set-1',
              name: 'Instagram Sweep',
              enabled: true,
              paused: false,
              pauseMode: 'disabled',
              mode: 'automatic',
              lastRunStatus: 'idle',
            },
          ],
        },
      ],
    })

    fireEvent.click(screen.getByRole('button', { name: 'Start' }))
    fireEvent.click(screen.getByRole('button', { name: 'Start (force)' }))

    expect(runSyncPlanNowMock).toHaveBeenNthCalledWith(1, { id: 'plan-1', force: false })
    expect(runSyncPlanNowMock).toHaveBeenNthCalledWith(2, { id: 'plan-1', force: true })

    schedulerTick?.()

    await waitFor(() => {
      expect(refreshSnapshotMock).toHaveBeenCalledTimes(1)
    })
  })

  it('opens pause and skip menus and applies dropdown actions', async () => {
    renderWindow({
      schedulerSets: [
        {
          id: 'set-1',
          name: 'Default Scheduler',
          active: true,
          plans: [
            {
              id: 'plan-1',
              schedulerSetId: 'set-1',
              name: 'Instagram Sweep',
              enabled: true,
              paused: false,
              pauseMode: 'disabled',
              mode: 'automatic',
              lastRunStatus: 'idle',
            },
          ],
        },
      ],
    })

    fireEvent.click(screen.getByRole('button', { name: 'Pause' }))
    fireEvent.click(screen.getByRole('button', { name: '1h' }))

    await waitFor(() => {
      expect(setSyncPlanPauseMock).toHaveBeenCalledWith({ id: 'plan-1', pauseMode: '1h', pauseUntil: undefined })
    })

    fireEvent.click(screen.getByRole('button', { name: 'Skip' }))
    fireEvent.click(screen.getByRole('button', { name: 'Delay reset' }))

    await waitFor(() => {
      expect(applySyncPlanSkipMock).toHaveBeenCalledWith({ id: 'plan-1', mode: 'reset', minutes: undefined, until: undefined })
    })
  })

  it('hides the set selector for a single set and exposes scheduler set creation in settings', async () => {
    renderWindow({
      schedulerSets: [
        {
          id: 'set-1',
          name: 'Default Scheduler',
          active: true,
          plans: [],
        },
      ],
    })

    expect(screen.queryByText('Scheduler set')).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: 'Settings' }))
    fireEvent.change(screen.getByPlaceholderText('Default Scheduler'), { target: { value: 'Night Run' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create new' }))

    await waitFor(() => {
      expect(upsertSchedulerSetMock).toHaveBeenCalledWith({ id: undefined, name: 'Night Run', active: true })
    })
  })
})
