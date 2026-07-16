// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SchedulerPage } from './SchedulerPage'
import { createEmptyWorkspaceSnapshot } from '../../domain/workspaceSnapshot'

const useAppStoreMock = vi.fn()
const closeDesktopWindowMock = vi.hoisted(() => vi.fn())

vi.mock('../../utils/closeDesktopWindow', () => ({
  closeDesktopWindow: closeDesktopWindowMock,
}))

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

function createSnapshot() {
  return {
    ...createEmptyWorkspaceSnapshot(),
    sources: [
      {
        id: 'source-1',
        provider: 'instagram',
        accountId: 'account-1',
        handle: '@priority_feed',
        displayName: 'Priority Feed',
        sourceKind: 'profile',
        remoteState: 'exists',
        readyForDownload: true,
        labels: ['priority', 'vip'],
        isSubscription: false,
        lastSyncedAt: '2026-03-15T10:00:00Z',
      },
    ],
    schedulerGroups: [
      {
        id: 'group-1',
        name: 'Priority Group',
        sortIndex: 0,
        criteria: {
          regular: true,
          temporary: false,
          favorite: false,
          readyForDownload: true,
          ignoreReadyForDownload: false,
          downloadUsers: true,
          downloadSubscriptions: true,
          userExists: true,
          userSuspended: false,
          userDeleted: false,
          labelsNo: false,
          labelsIncluded: [],
          labelsExcluded: [],
          ignoreExcludedLabels: false,
          sitesIncluded: [],
          sitesExcluded: [],
          groupIdsIncluded: [],
          groupIdsExcluded: [],
          usersCount: undefined,
          daysNumber: undefined,
          daysIsDownloaded: false,
          dateFrom: undefined,
          dateTo: undefined,
          dateInRange: true,
          advancedExpression: '',
        },
      },
    ],
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
            mode: 'automatic',
            intervalMinutes: 30,
            startupDelayMinutes: 5,
            notificationMode: 'summary',
            targetFilter: '',
            sortIndex: 0,
            paused: false,
            pauseMode: 'disabled',
            lastRunStatus: 'idle',
            lastRunAt: '2026-03-16T10:00:00Z',
            nextDueAt: '2026-03-16T10:30:00Z',
            notifications: { enabled: true, simple: true, showImage: false, showUserIcon: false },
            criteria: {
              regular: true,
              temporary: false,
              favorite: false,
              readyForDownload: true,
              ignoreReadyForDownload: false,
              downloadUsers: true,
              downloadSubscriptions: true,
              userExists: true,
              userSuspended: false,
              userDeleted: false,
              labelsNo: false,
              labelsIncluded: [],
              labelsExcluded: [],
              ignoreExcludedLabels: false,
              sitesIncluded: [],
              sitesExcluded: [],
              groupIdsIncluded: [],
              groupIdsExcluded: [],
              usersCount: undefined,
              daysNumber: undefined,
              daysIsDownloaded: false,
              dateFrom: undefined,
              dateTo: undefined,
              dateInRange: true,
              advancedExpression: '',
            },
          },
        ],
      },
    ],
    syncPlanRuns: [
      {
        id: 'run-1',
        planId: 'plan-1',
        schedulerSetId: 'set-1',
        trigger: 'scheduler',
        status: 'failed',
        summary: 'Ran 2 source syncs with 1 failures.',
        sourceCount: 2,
        startedAt: '2026-03-16T10:00:00Z',
        finishedAt: '2026-03-16T10:02:00Z',
      },
    ],
  }
}

function renderPage() {
  const snapshot = createSnapshot()
  const store = {
    snapshot,
    pendingCommand: undefined,
    upsertSyncPlan: vi.fn().mockResolvedValue(snapshot),
    previewSyncPlanTarget: vi.fn().mockResolvedValue({
      sourceCount: 1,
      sources: [{ id: 'source-1', handle: '@priority_feed', provider: 'instagram', labels: ['priority'], readyForDownload: true, remoteState: 'exists', subscription: false }],
    }),
    deleteSyncPlan: vi.fn().mockResolvedValue(snapshot),
    runSyncPlanNow: vi.fn().mockResolvedValue(snapshot),
    setSyncPlanPause: vi.fn().mockResolvedValue(snapshot),
    clearSyncPlanPause: vi.fn().mockResolvedValue(snapshot),
    applySyncPlanSkip: vi.fn().mockResolvedValue(snapshot),
    moveSyncPlan: vi.fn().mockResolvedValue(snapshot),
    cloneSyncPlan: vi.fn().mockResolvedValue(snapshot),
  }

  useAppStoreMock.mockImplementation((selector: (state: typeof store) => unknown) => selector(store))
  return { store, ...render(<SchedulerPage initialIntent={{ mode: 'edit', planId: 'plan-1', schedulerSetId: 'set-1' }} />) }
}

describe('SchedulerPage', () => {
  beforeEach(() => {
    useAppStoreMock.mockReset()
    closeDesktopWindowMock.mockReset()
    closeDesktopWindowMock.mockResolvedValue(undefined)
  })

  afterEach(() => {
    cleanup()
  })

  it('renders the tabbed plan editor with general, filters, and runtime tabs', async () => {
    renderPage()

    expect(screen.getByRole('tab', { name: 'General' })).toBeTruthy()
    expect(screen.getByRole('tab', { name: 'Filters' })).toBeTruthy()
    expect(screen.getByRole('tab', { name: 'Runtime' })).toBeTruthy()
    expect(screen.getByRole('heading', { name: 'Instagram Sweep' })).toBeTruthy()
  })

  it('keeps summary in general and exposes a primary plan status control', async () => {
    renderPage()

    fireEvent.click(screen.getByRole('tab', { name: 'General' }))

    expect(screen.getByRole('group', { name: 'Plan status' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Enabled' }).getAttribute('aria-pressed')).toBe('true')
    expect(screen.getByText('Last download date')).toBeTruthy()
    expect(screen.queryByText('Sort index')).toBeNull()
    expect(screen.queryByText('Run this task manually')).toBeNull()
  })

  it('hides automatic schedule fields when mode is manual', async () => {
    renderPage()

    fireEvent.click(screen.getByRole('tab', { name: 'General' }))
    fireEvent.change(screen.getByLabelText('Mode'), { target: { value: 'manual' } })

    expect(screen.queryByLabelText('Run every (minutes)')).toBeNull()
    expect(screen.queryByLabelText('Initial delay after app start (minutes)')).toBeNull()
    expect(screen.getByText('Runs only when started manually from the Runtime tab using Start or Start (force).')).toBeTruthy()
  })

  it('shows notification details only when notifications are enabled and detailed', async () => {
    renderPage()

    fireEvent.click(screen.getByRole('tab', { name: 'General' }))
    expect(screen.getByLabelText('Notification style')).toBeTruthy()
    expect(screen.queryByLabelText('Include preview image')).toBeNull()

    fireEvent.change(screen.getByLabelText('Notification style'), { target: { value: 'detailed' } })

    expect(screen.getByLabelText('Include preview image')).toBeTruthy()
    expect(screen.getByLabelText('Include user icon')).toBeTruthy()

    fireEvent.click(screen.getByRole('checkbox', { name: 'Show notifications' }))

    expect(screen.queryByLabelText('Notification style')).toBeNull()
  })

  it('preserves sort index when saving without exposing the field', async () => {
    const { store } = renderPage()

    fireEvent.click(screen.getByRole('tab', { name: 'General' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))

    await waitFor(() => {
      expect(store.upsertSyncPlan).toHaveBeenCalled()
    })

    expect(store.upsertSyncPlan).toHaveBeenCalledWith(expect.objectContaining({
      id: 'plan-1',
      sortIndex: 0,
    }))
  })

  it('previews targets from the runtime tab', async () => {
    const { store } = renderPage()

    fireEvent.click(screen.getByRole('tab', { name: 'Runtime' }))
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }))

    await waitFor(() => {
      expect(store.previewSyncPlanTarget).toHaveBeenCalledWith({
        schedulerSetId: 'set-1',
        planId: 'plan-1',
        criteria: expect.any(Object),
      })
    })

    expect(screen.getByText('@priority_feed', { exact: false })).toBeTruthy()
  })

  it('keeps up and down outside the runtime panel', async () => {
    renderPage()

    fireEvent.click(screen.getByRole('tab', { name: 'Runtime' }))

    const runtimePanel = screen.getByRole('tabpanel')
    expect(within(runtimePanel).queryByRole('button', { name: 'Up' })).toBeNull()
    expect(within(runtimePanel).queryByRole('button', { name: 'Down' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Up' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Down' })).toBeTruthy()
  })

  it('reorganizes filters into grouped sections with contextual controls', async () => {
    renderPage()

    fireEvent.click(screen.getByRole('tab', { name: 'Filters' }))

    expect(screen.getByRole('heading', { name: /Providers/i })).toBeTruthy()
    expect(screen.queryByRole('heading', { name: /Source class/i })).toBeNull()
    expect(screen.queryByRole('heading', { name: /Users & subscriptions/i })).toBeNull()
    expect(screen.getByRole('heading', { name: /Availability/i })).toBeTruthy()
    expect(screen.getByRole('heading', { name: /Date range/i })).toBeTruthy()
    expect(screen.getByRole('heading', { name: /Freshness & limits/i })).toBeTruthy()
    expect(screen.queryByRole('heading', { name: /Users count/i })).toBeNull()
    expect(screen.queryByText('Labels include')).toBeNull()
    expect(screen.queryByText('Groups include')).toBeNull()
    expect(screen.queryByText('Profile class')).toBeNull()
    expect(screen.queryByText('Remote state')).toBeNull()
    expect(screen.queryByRole('button', { name: 'Labels help' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Groups help' })).toBeNull()
    expect(screen.queryByText('Regular')).toBeNull()
    expect(screen.getByText('Inside range')).toBeTruthy()
  })

  it('renders temporary overrides without execution controls or notification feed', async () => {
    renderPage()

    fireEvent.click(screen.getByRole('tab', { name: 'Runtime' }))

    expect(screen.getByRole('heading', { name: /Temporary overrides/i })).toBeTruthy()
    expect(screen.getByText('Pause execution')).toBeTruthy()
    expect(screen.getByText('Skip next window')).toBeTruthy()
    expect(screen.queryByRole('heading', { name: /Execution controls/i })).toBeNull()
    expect(screen.queryByRole('heading', { name: /Notification feed/i })).toBeNull()
  })

  it('lets the filters tab switch date range semantics and compose labels', async () => {
    renderPage()

    fireEvent.click(screen.getByRole('tab', { name: 'Filters' }))
    fireEvent.click(screen.getByRole('checkbox', { name: /In range/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Pick From date' }))
    fireEvent.change(screen.getByLabelText('Label entry'), { target: { value: 'vip' } })
    const labelsCard = screen.getByRole('heading', { name: /Labels/i }).closest('article')
    expect(labelsCard).toBeTruthy()
    fireEvent.click(within(labelsCard as HTMLElement).getByRole('button', { name: 'Include' }))

    expect(screen.getByRole('dialog', { name: 'From date calendar' })).toBeTruthy()
    expect(screen.getByText('Outside range')).toBeTruthy()
    expect(screen.getByText('vip ×')).toBeTruthy()
  })

  it('shows groups as a composed control and disables the block when no groups exist', async () => {
    const snapshot = createSnapshot()
    snapshot.schedulerGroups = []
    const store = {
      snapshot,
      pendingCommand: undefined,
      upsertSyncPlan: vi.fn().mockResolvedValue(snapshot),
      previewSyncPlanTarget: vi.fn().mockResolvedValue({ sourceCount: 0, sources: [] }),
      deleteSyncPlan: vi.fn().mockResolvedValue(snapshot),
      runSyncPlanNow: vi.fn().mockResolvedValue(snapshot),
      setSyncPlanPause: vi.fn().mockResolvedValue(snapshot),
      clearSyncPlanPause: vi.fn().mockResolvedValue(snapshot),
      applySyncPlanSkip: vi.fn().mockResolvedValue(snapshot),
      moveSyncPlan: vi.fn().mockResolvedValue(snapshot),
      cloneSyncPlan: vi.fn().mockResolvedValue(snapshot),
    }

    useAppStoreMock.mockImplementation((selector: (state: typeof store) => unknown) => selector(store))
    render(<SchedulerPage initialIntent={{ mode: 'edit', planId: 'plan-1', schedulerSetId: 'set-1' }} />)

    fireEvent.click(screen.getByRole('tab', { name: 'Filters' }))

    expect(screen.getByText('No scheduler groups are available yet. Create a group first to use this filter block.')).toBeTruthy()
    const groupsCard = screen.getByRole('heading', { name: /Scheduler groups/i }).closest('article')
    expect(groupsCard).toBeTruthy()
    expect(within(groupsCard as HTMLElement).getByRole('button', { name: 'Include' }).getAttribute('disabled')).not.toBeNull()
  })
})
