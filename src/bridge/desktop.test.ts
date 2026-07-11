import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createEmptyWorkspaceSnapshot } from '../domain/workspaceSnapshot'

type EventHandler = (event: { payload: unknown }) => void

const { invokeMock, listenMock, eventHandlers } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
  eventHandlers: new Map<string, EventHandler>(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: listenMock,
}))

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  WebviewWindow: {
    getByLabel: vi.fn(),
  },
}))

describe('loadWorkspaceSnapshot', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    listenMock.mockReset()
    eventHandlers.clear()
    listenMock.mockImplementation(async (eventName: string, handler: EventHandler) => {
      eventHandlers.set(eventName, handler)
      return () => {
        eventHandlers.delete(eventName)
      }
    })
    vi.resetModules()
  })

  it('rejects when the tauri bootstrap command fails', async () => {
    invokeMock.mockRejectedValueOnce(new Error('bootstrap failed'))

    const desktop = await import('./desktop')

    await expect(desktop.loadWorkspaceSnapshot()).rejects.toThrow('bootstrap failed')
  })

  it('normalizes desktop runtime state from snake_case backend payloads', async () => {
    invokeMock.mockResolvedValueOnce({
      desktop_runtime: {
        close_to_tray: true,
        silent_mode: true,
        tray_available: true,
      },
      sources: [
        {
          id: 'source-1',
          provider: 'instagram',
          source_kind: 'profile',
          handle: '@alpha',
          display_name: 'Alpha',
          ready_for_download: true,
          sync_options: {
            instagram: {
              timeline: true,
              reels: true,
              stories: false,
              stories_user: true,
              tagged_posts: true,
              extract_image_from_video: {
                timeline: false,
                reels: false,
                stories: false,
                stories_user: false,
                tagged: false,
              },
              place_extracted_image_into_video_folder: true,
            },
          },
        },
      ],
      account_sync_runs: [
        {
          id: 'account-run-1',
          account_id: 'account-1',
          provider: 'instagram',
          tool: 'instagram',
          trigger: 'manual',
          status: 'succeeded',
          summary: 'Saved posts sync completed',
          command_preview: 'saved-posts',
          degraded_capabilities: [],
          started_at: '2026-03-11T11:00:00Z',
          finished_at: '2026-03-11T11:01:00Z',
        },
      ],
      source_sync_runs: [
        {
          id: 'source-run-1',
          source_id: 'source-1',
          account_id: 'account-1',
          provider: 'instagram',
          tool: 'internal.instagram',
          trigger: 'manual',
          status: 'skipped',
          summary: 'Instagram sync skipped because provider cooldown is active.',
          command_preview: 'internal.instagram profile alpha',
          degraded_capabilities: [],
          started_at: '2026-03-11T11:00:00Z',
          finished_at: '2026-03-11T11:01:00Z',
        },
      ],
    })

    const desktop = await import('./desktop')
    const snapshot = await desktop.loadWorkspaceSnapshot()

    expect(snapshot.desktopRuntime).toEqual({
      closeToTray: true,
      silentMode: true,
      trayAvailable: true,
      reportedByBackend: true,
    })
    expect(snapshot.sources[0].syncOptions?.instagram).toEqual(
      expect.objectContaining({
        timeline: true,
        reels: true,
        stories: false,
        storiesUser: true,
        tagged: true,
        extractImageFromVideo: {
          timeline: false,
          reels: false,
          stories: false,
          storiesUser: false,
          tagged: false,
        },
        placeExtractedImageIntoVideoFolder: true,
      }),
    )
    expect(snapshot.accountSyncRuns[0]).toEqual(
      expect.objectContaining({
        accountId: 'account-1',
        summary: 'Saved posts sync completed',
      }),
    )
    expect(snapshot.sourceSyncRuns[0]).toEqual(
      expect.objectContaining({
        status: 'skipped',
        tool: 'internal.instagram',
      }),
    )
  })

  it('subscribes to scheduler, queue, import, route, and runtime-log events from tauri', async () => {
    const desktop = await import('./desktop')
    const onSchedulerTick = vi.fn()
    const onWorkspaceSnapshotChanged = vi.fn()
    const onRouteActivation = vi.fn()
    const onSourceSyncQueueChanged = vi.fn()
    const onImportQueueChanged = vi.fn()
    const onRuntimeLogAppended = vi.fn()

    const unsubscribe = await desktop.subscribeToDesktopRuntimeEvents({
      onSchedulerTick,
      onWorkspaceSnapshotChanged,
      onRouteActivation,
      onSourceSyncQueueChanged,
      onImportQueueChanged,
      onRuntimeLogAppended,
    })

    eventHandlers.get('runtime://scheduler-tick')?.({ payload: undefined })
    const snapshot = createEmptyWorkspaceSnapshot()
    snapshot.desktopRuntime.reportedByBackend = true
    eventHandlers.get('runtime://workspace-snapshot-changed')?.({ payload: snapshot })
    eventHandlers.get('runtime://source-sync-queue-changed')?.({
      payload: {
        queuedCount: 1,
        runningCount: 1,
        completedCount: 2,
        failedCount: 0,
        totalCount: 4,
        activeSourceId: 'source-1',
        activeHandle: '@active',
        activeProvider: 'instagram',
        providers: [
          {
            provider: 'instagram',
            displayName: 'Instagram',
            queued: 1,
            running: 1,
            completed: 2,
            failed: 0,
            total: 4,
          },
        ],
        queuedItems: [],
        runningItems: [],
        recentResults: [],
        updatedAt: '2026-03-11T12:00:00Z',
      },
    })
    eventHandlers.get('runtime://import-queue-changed')?.({
      payload: {
        queued_count: 0,
        running_count: 1,
        completed_count: 0,
        failed_count: 0,
        total_count: 1,
        active_importer_id: 'instagram.scrawler',
        active_provider: 'instagram',
        active_method_label: 'SCrawler',
        active_job_kind: 'preview',
        running_items: [
          {
            job_id: 'job-1',
            importer_id: 'instagram.scrawler',
            provider: 'instagram',
            method_label: 'SCrawler',
            job_kind: 'preview',
            queued_at: '2026-03-11T12:00:00Z',
            started_at: '2026-03-11T12:00:01Z',
            progress_label: 'Scanning folders',
            progress_detail: 'Scanning legacy folders.',
            progress_indeterminate: true,
          },
        ],
        queued_items: [],
        recent_results: [],
        updated_at: '2026-03-11T12:00:02Z',
      },
    })
    eventHandlers.get('runtime://foreground-route')?.({ payload: { route: 'scheduler' } })
    eventHandlers.get('runtime://runtime-log-appended')?.({
      payload: {
        id: 'log-1',
        timestamp: '2026-03-11T12:00:01Z',
        scope: 'sync.run',
        level: 'warning',
        provider: 'instagram',
        account_id: 'account-1',
        source_handle: '@active',
        message: 'Cancellation requested',
        detail: 'Requested by user.',
      },
    })

    expect(onSchedulerTick).toHaveBeenCalledTimes(1)
    expect(onWorkspaceSnapshotChanged).toHaveBeenCalledWith(
      expect.objectContaining({
        desktopRuntime: expect.objectContaining({ reportedByBackend: true }),
      }),
    )
    expect(onRouteActivation).toHaveBeenCalledWith('scheduler')
    expect(onSourceSyncQueueChanged).toHaveBeenCalledWith(
      expect.objectContaining({
        queuedCount: 1,
        activeHandle: '@active',
      }),
    )
    expect(onImportQueueChanged).toHaveBeenCalledWith(
      expect.objectContaining({
        runningCount: 1,
        activeMethodLabel: 'SCrawler',
      }),
    )
    expect(onRuntimeLogAppended).toHaveBeenCalledWith(
      expect.objectContaining({
        id: 'log-1',
        accountId: 'account-1',
        sourceHandle: '@active',
        detail: 'Requested by user.',
      }),
    )

    unsubscribe()
    expect(eventHandlers.size).toBe(0)
  })

  it('loads source sync queue status via tauri command', async () => {
    invokeMock.mockResolvedValueOnce({
      queuedCount: 0,
      runningCount: 0,
      completedCount: 3,
      failedCount: 1,
      totalCount: 4,
      providers: [],
      queuedItems: [],
      runningItems: [
        {
          job_key: 'source-1:instagram-story:123',
          source_id: 'source-1',
          provider: 'instagram',
          handle: '@active',
          state: 'running',
          queued_at: '2026-03-11T11:58:00Z',
          started_at: '2026-03-11T12:00:00Z',
          progress_percent: 64,
          progress_label: 'Downloading profile',
          progress_detail: '64 files downloaded',
          progress_indeterminate: false,
          downloaded_items: 64,
        },
      ],
      recentResults: [
        {
          source_id: 'source-2',
          provider: 'instagram',
          handle: '@cooldown',
          status: 'skipped',
          summary: 'Instagram sync skipped because provider cooldown is active.',
          finished_at: '2026-03-11T11:59:00Z',
        },
      ],
      updatedAt: '2026-03-11T12:00:00Z',
    })
    const desktop = await import('./desktop')

    const status = await desktop.loadSourceSyncQueueStatus()

    expect(invokeMock).toHaveBeenCalledWith('source_sync_queue_status')
    expect(status.completedCount).toBe(3)
    expect(status.failedCount).toBe(1)
    expect(status.runningItems[0]).toEqual(
      expect.objectContaining({
        jobKey: 'source-1:instagram-story:123',
        sourceId: 'source-1',
        progressPercent: 64,
        progressLabel: 'Downloading profile',
        progressDetail: '64 files downloaded',
        progressIndeterminate: false,
        downloadedItems: 64,
      }),
    )
    expect(status.recentResults[0]).toEqual(
      expect.objectContaining({
        handle: '@cooldown',
        status: 'skipped',
      }),
    )
  })

  it('invokes profile and provider queue cancel commands', async () => {
    invokeMock.mockResolvedValue({})
    const desktop = await import('./desktop')

    await desktop.cancelSourceSyncProfile('source-1')
    await desktop.cancelSourceSyncProvider('instagram')

    expect(invokeMock).toHaveBeenCalledWith(
      'cancel_source_sync_profile',
      expect.objectContaining({
        sourceId: 'source-1',
        source_id: 'source-1',
      }),
    )
    expect(invokeMock).toHaveBeenCalledWith(
      'cancel_source_sync_provider',
      expect.objectContaining({
        provider: 'instagram',
      }),
    )
  })

  it('invokes the instagram saved-posts sync command with account aliases', async () => {
    invokeMock.mockResolvedValue({})
    const desktop = await import('./desktop')

    await desktop.runInstagramSavedPostsSync('account-1')

    expect(invokeMock).toHaveBeenCalledWith(
      'run_instagram_saved_posts_sync',
      expect.objectContaining({
        accountId: 'account-1',
        account_id: 'account-1',
      }),
    )
  })

  it('invokes the open source folder command with source aliases', async () => {
    invokeMock.mockResolvedValue({})
    const desktop = await import('./desktop')

    await desktop.openSourceFolder('source-1')

    expect(invokeMock).toHaveBeenCalledWith(
      'open_source_folder',
      expect.objectContaining({
        sourceId: 'source-1',
        source_id: 'source-1',
      }),
    )
  })

  it('reports runtime-log readiness telemetry without throwing', async () => {
    invokeMock.mockResolvedValueOnce({
      windowOpen: true,
      openRequests: 1,
      readySignals: 1,
      lastReadyAt: '2026-03-11T12:00:00Z',
      lastFailure: null,
    })
    const desktop = await import('./desktop')

    await expect(desktop.reportRuntimeLogWindowReady()).resolves.toBeUndefined()
    expect(invokeMock).toHaveBeenCalledWith('report_runtime_log_window_ready')
  })

  it('reports runtime-log bootstrap failures with sanitized payload', async () => {
    invokeMock.mockResolvedValueOnce({
      windowOpen: false,
      openRequests: 1,
      readySignals: 0,
      lastReadyAt: null,
      lastFailure: 'boom',
    })
    const desktop = await import('./desktop')

    await expect(
      desktop.reportRuntimeLogWindowBootstrapFailure('  boom  '),
    ).resolves.toBeUndefined()

    expect(invokeMock).toHaveBeenCalledWith(
      'report_runtime_log_window_bootstrap_failure',
      { message: 'boom' },
    )
  })

  it('opens source-sync queue window through tauri command', async () => {
    invokeMock.mockResolvedValueOnce(undefined)
    const desktop = await import('./desktop')

    await expect(desktop.openSourceSyncQueueWindow()).resolves.toBeUndefined()
    expect(invokeMock).toHaveBeenCalledWith('open_source_sync_queue_window')
  })

  it('opens connector runtimes window through tauri command', async () => {
    invokeMock.mockResolvedValueOnce(undefined)
    const desktop = await import('./desktop')

    await expect(desktop.openConnectorRuntimesWindow()).resolves.toBeUndefined()
    expect(invokeMock).toHaveBeenCalledWith('open_connector_runtimes_window')
  })

  it('opens accounts window through tauri command with optional intent', async () => {
    invokeMock.mockResolvedValue(undefined)
    const desktop = await import('./desktop')

    await expect(desktop.openAccountsWindow()).resolves.toBeUndefined()
    await expect(
      desktop.openAccountsWindow({
        initialAccountId: ' account-1 ',
        initialProvider: 'instagram',
        initialMode: 'edit',
      }),
    ).resolves.toBeUndefined()

    expect(invokeMock).toHaveBeenCalledWith('open_accounts_window')
    expect(invokeMock).toHaveBeenCalledWith('open_accounts_window', {
      intent: {
        initialAccountId: 'account-1',
        initialProvider: 'instagram',
        initialMode: 'edit',
      },
    })
  })

  it('opens source editor window through tauri command with optional intent', async () => {
    invokeMock.mockResolvedValue(undefined)
    const desktop = await import('./desktop')

    await expect(desktop.openSourceEditorWindow()).resolves.toBeUndefined()
    await expect(
      desktop.openSourceEditorWindow({
        sourceId: ' source-1 ',
        preferredProvider: 'instagram',
        preferredAccountId: ' account-1 ',
        seed: {
          provider: 'instagram',
          handle: ' @seed ',
          displayName: '  Seed Profile  ',
        },
      }),
    ).resolves.toBeUndefined()

    expect(invokeMock).toHaveBeenCalledWith('open_source_editor_window')
    expect(invokeMock).toHaveBeenCalledWith('open_source_editor_window', {
      intent: {
        sourceId: 'source-1',
        preferredProvider: 'instagram',
        preferredAccountId: 'account-1',
        seed: {
          provider: 'instagram',
          handle: '@seed',
          displayName: 'Seed Profile',
        },
      },
    })
  })

  it('keeps openProfileEditorWindow as alias for source editor command', async () => {
    invokeMock.mockResolvedValue(undefined)
    const desktop = await import('./desktop')

    await expect(desktop.openProfileEditorWindow()).resolves.toBeUndefined()

    expect(invokeMock).toHaveBeenCalledWith('open_source_editor_window')
  })

  it('subscribes to source editor window intent and normalizes snake_case payload', async () => {
    const desktop = await import('./desktop')
    const handler = vi.fn()

    const unsubscribe = await desktop.subscribeToSourceEditorWindowIntent(handler)
    eventHandlers.get('runtime://source-editor-window-intent')?.({
      payload: {
        source_id: 'source-1',
        preferred_provider: 'instagram',
        preferred_account_id: 'account-1',
        seed: {
          provider: 'instagram',
          handle: '@seed',
          display_name: 'Seed',
        },
      },
    })

    expect(handler).toHaveBeenCalledWith({
      sourceId: 'source-1',
      preferredProvider: 'instagram',
      preferredAccountId: 'account-1',
      seed: {
        provider: 'instagram',
        handle: '@seed',
        displayName: 'Seed',
      },
    })

    unsubscribe()
  })

  it('opens import window through tauri command', async () => {
    invokeMock.mockResolvedValueOnce(undefined)
    const desktop = await import('./desktop')

    await expect(desktop.openImportWindow()).resolves.toBeUndefined()
    expect(invokeMock).toHaveBeenCalledWith('open_import_window')
  })

  it('loads and normalizes import providers, methods, preview, and run result', async () => {
    invokeMock
      .mockResolvedValueOnce([
        {
          key: 'instagram',
          display_name: 'Instagram',
          description: 'Legacy imports',
        },
      ])
      .mockResolvedValueOnce([
        {
          importer_id: 'instagram.scrawler',
          provider: 'instagram',
          label: 'SCrawler',
          description: 'Imports from legacy folders',
        },
      ])
      .mockResolvedValueOnce([
        {
          path: 'D:\\Media\\Instagram',
          source: 'default',
          label: 'Media root',
          removable: false,
        },
        {
          path: 'D:\\Legacy\\Instagram',
          source: 'manual',
          label: 'Manual root',
          removable: true,
        },
      ])
      .mockResolvedValueOnce({
        importer_id: 'instagram.scrawler',
        provider: 'instagram',
        method_label: 'SCrawler',
        force_reimport: false,
        roots: ['D:\\Media\\Instagram'],
        profiles: [
          {
            profile_root: 'D:\\Media\\Instagram\\alpha',
            user_xml_path: 'D:\\Media\\Instagram\\alpha\\Settings\\User_Instagram_alpha.xml',
            handle: 'alpha',
            display_name: 'Alpha',
            already_imported: false,
            import_state: 'needs_account_link',
            file_count: 12,
            already_cataloged_count: 4,
            new_file_count: 8,
            problems: [
              {
                severity: 'error',
                code: 'account-match-missing',
                message: 'Link an account.',
              },
            ],
          },
        ],
        summary: {
          detected_profiles: 1,
          ready_profiles: 0,
          blocked_profiles: 1,
          already_imported_profiles: 0,
          importable_files: 12,
        },
      })
      .mockResolvedValueOnce({
        importer_id: 'instagram.scrawler',
        imported_profiles: 1,
        skipped_profiles: 0,
        failed_profiles: 0,
        imported_media_count: 8,
        already_cataloged_count: 4,
        profiles: [
          {
            profile_root: 'D:\\Media\\Instagram\\alpha',
            handle: 'alpha',
            status: 'imported',
            source_id: 'source-1',
            imported_media_count: 8,
            already_cataloged_count: 4,
            message: 'Imported successfully.',
          },
        ],
      })
      .mockResolvedValueOnce('D:\\Legacy\\Instagram')
      .mockResolvedValueOnce({
        queued_count: 0,
        running_count: 1,
        completed_count: 0,
        failed_count: 0,
        total_count: 1,
        running_items: [
          {
            job_id: 'job-1',
            importer_id: 'instagram.scrawler',
            provider: 'instagram',
            method_label: 'SCrawler',
            job_kind: 'preview',
            queued_at: '2026-03-11T12:10:00Z',
            started_at: '2026-03-11T12:10:01Z',
            progress_label: 'Scanning folders',
            progress_detail: 'Scanning legacy folders.',
            progress_indeterminate: true,
          },
        ],
        queued_items: [],
        recent_results: [],
        updated_at: '2026-03-11T12:10:02Z',
      })
      .mockResolvedValueOnce({
        queued_count: 0,
        running_count: 1,
        completed_count: 0,
        failed_count: 0,
        total_count: 1,
        running_items: [
          {
            job_id: 'job-2',
            importer_id: 'instagram.scrawler',
            provider: 'instagram',
            method_label: 'SCrawler',
            job_kind: 'import',
            queued_at: '2026-03-11T12:11:00Z',
            started_at: '2026-03-11T12:11:01Z',
            progress_label: 'Applying import',
            progress_detail: 'Cataloging reviewed media.',
            progress_indeterminate: true,
          },
        ],
        queued_items: [],
        recent_results: [],
        updated_at: '2026-03-11T12:11:02Z',
      })
      .mockResolvedValueOnce({
        queued_count: 0,
        running_count: 0,
        completed_count: 1,
        failed_count: 0,
        total_count: 1,
        queued_items: [],
        running_items: [],
        recent_results: [],
        updated_at: '2026-03-11T12:11:03Z',
      })

    const desktop = await import('./desktop')

    await expect(desktop.listImportProviders()).resolves.toEqual([
      {
        key: 'instagram',
        displayName: 'Instagram',
        description: 'Legacy imports',
      },
    ])
    await expect(desktop.listImportMethods('instagram')).resolves.toEqual([
      {
        importerId: 'instagram.scrawler',
        provider: 'instagram',
        label: 'SCrawler',
        description: 'Imports from legacy folders',
      },
    ])
    await expect(desktop.listImportRoots('instagram.scrawler', ['D:\\Legacy\\Instagram'], [])).resolves.toEqual([
      {
        path: 'D:\\Media\\Instagram',
        source: 'default',
        label: 'Media root',
        removable: false,
      },
      {
        path: 'D:\\Legacy\\Instagram',
        source: 'manual',
        label: 'Manual root',
        removable: true,
      },
    ])
    await expect(
      desktop.previewImportMethod('instagram.scrawler', {
        forceReimport: false,
        manualRoots: ['D:\\Legacy\\Instagram'],
        disabledRoots: [],
      }),
    ).resolves.toEqual(
      expect.objectContaining({
        importerId: 'instagram.scrawler',
        roots: ['D:\\Media\\Instagram'],
        profiles: [
          expect.objectContaining({
            importState: 'needs_account_link',
            fileCount: 12,
            newFileCount: 8,
          }),
        ],
      }),
    )
    await expect(
      desktop.runImportMethod('instagram.scrawler', {
        forceReimport: false,
        manualRoots: ['D:\\Legacy\\Instagram'],
        disabledRoots: [],
        resolutions: [
          {
            profileRoot: 'D:\\Media\\Instagram\\alpha',
            action: 'import',
            accountId: 'account-1',
          },
        ],
      }),
    ).resolves.toEqual(
      expect.objectContaining({
        importedProfiles: 1,
        importedMediaCount: 8,
      }),
    )
    await expect(desktop.pickImportRootFolder()).resolves.toBe('D:\\Legacy\\Instagram')
    await expect(
      desktop.enqueueImportPreview('instagram.scrawler', {
        forceReimport: false,
        manualRoots: ['D:\\Legacy\\Instagram'],
        disabledRoots: [],
      }),
    ).resolves.toEqual(
      expect.objectContaining({
        runningCount: 1,
        runningItems: [
          expect.objectContaining({
            importerId: 'instagram.scrawler',
            jobKind: 'preview',
          }),
        ],
      }),
    )
    await expect(
      desktop.enqueueImportRun('instagram.scrawler', {
        forceReimport: false,
        manualRoots: ['D:\\Legacy\\Instagram'],
        disabledRoots: [],
        resolutions: [],
      }),
    ).resolves.toEqual(
      expect.objectContaining({
        runningCount: 1,
        runningItems: [
          expect.objectContaining({
            importerId: 'instagram.scrawler',
            jobKind: 'import',
          }),
        ],
      }),
    )
    await expect(desktop.loadImportQueueStatus()).resolves.toEqual(
      expect.objectContaining({
        completedCount: 1,
      }),
    )

    expect(invokeMock).toHaveBeenCalledWith('list_import_providers')
    expect(invokeMock).toHaveBeenCalledWith('list_import_methods', { provider: 'instagram' })
    expect(invokeMock).toHaveBeenCalledWith('list_import_roots', {
      importerId: 'instagram.scrawler',
      importer_id: 'instagram.scrawler',
      manualRoots: ['D:\\Legacy\\Instagram'],
      manual_roots: ['D:\\Legacy\\Instagram'],
      disabledRoots: [],
      disabled_roots: [],
    })
    expect(invokeMock).toHaveBeenCalledWith('preview_import_method', {
      importerId: 'instagram.scrawler',
      importer_id: 'instagram.scrawler',
      options: { forceReimport: false, manualRoots: ['D:\\Legacy\\Instagram'], disabledRoots: [] },
      forceReimport: false,
      force_reimport: false,
      manualRoots: ['D:\\Legacy\\Instagram'],
      manual_roots: ['D:\\Legacy\\Instagram'],
      disabledRoots: [],
      disabled_roots: [],
    })
    expect(invokeMock).toHaveBeenCalledWith(
      'run_import_method',
      expect.objectContaining({
        importerId: 'instagram.scrawler',
        importer_id: 'instagram.scrawler',
        input: expect.objectContaining({
          forceReimport: false,
          manualRoots: ['D:\\Legacy\\Instagram'],
          disabledRoots: [],
          resolutions: [
            expect.objectContaining({
              profileRoot: 'D:\\Media\\Instagram\\alpha',
              accountId: 'account-1',
            }),
          ],
        }),
      }),
    )
    expect(invokeMock).toHaveBeenCalledWith('pick_import_root_folder')
    expect(invokeMock).toHaveBeenCalledWith('enqueue_import_preview', {
      importerId: 'instagram.scrawler',
      importer_id: 'instagram.scrawler',
      options: { forceReimport: false, manualRoots: ['D:\\Legacy\\Instagram'], disabledRoots: [] },
      forceReimport: false,
      force_reimport: false,
      manualRoots: ['D:\\Legacy\\Instagram'],
      manual_roots: ['D:\\Legacy\\Instagram'],
      disabledRoots: [],
      disabled_roots: [],
    })
    expect(invokeMock).toHaveBeenCalledWith(
      'enqueue_import_run',
      expect.objectContaining({
        importerId: 'instagram.scrawler',
        importer_id: 'instagram.scrawler',
        input: expect.objectContaining({
          forceReimport: false,
          manualRoots: ['D:\\Legacy\\Instagram'],
          disabledRoots: [],
        }),
      }),
    )
    expect(invokeMock).toHaveBeenCalledWith('import_queue_status')
  })
})
