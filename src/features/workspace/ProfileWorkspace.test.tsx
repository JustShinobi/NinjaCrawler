// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { WorkspaceSnapshot } from '../../domain/models'
import { createEmptyWorkspaceSnapshot } from '../../domain/workspaceSnapshot'
import { ProfileWorkspace } from './ProfileWorkspace'

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (filePath: string) => filePath,
}))

vi.mock('./thumbnailCache', () => ({
  // Espelha o fallback real: asset URL direta, sem query string (o
  // cache-buster mora no nome do arquivo do thumb, testado no Rust).
  getPreviewSource: (source: { profileImagePath?: string }) =>
    source.profileImagePath ?? undefined,
  subscribeToAvatarThumbnails: () => () => undefined,
  getAvatarThumbnailsEpoch: () => 0,
}))

// jsdom não faz layout, então a virtualização real (que depende de medir a
// viewport/linhas) renderizaria zero linhas. Trocamos o virtualizer por um que
// renderiza todas as linhas — os testes checam conteúdo, não o windowing.
vi.mock('@tanstack/react-virtual', () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 100,
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({ index, key: index, start: index * 100 })),
    measureElement: () => undefined,
    measure: () => undefined,
    scrollToIndex: () => undefined,
    isScrolling: false,
  }),
}))

// O componente usa ResizeObserver p/ medir a largura; jsdom não o tem.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal('ResizeObserver', ResizeObserverStub)

describe('ProfileWorkspace', () => {
  afterEach(() => {
    cleanup()
    localStorage.clear()
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
  })

  function buildSnapshot(options?: { withSyncIssue?: boolean; paused?: boolean }): WorkspaceSnapshot {
    return {
      ...createEmptyWorkspaceSnapshot(),
      sources: [
        {
          id: 'source-1',
          provider: 'instagram' as const,
          sourceKind: 'profile' as const,
          handle: '@visual_lab',
          displayName: 'visual_lab',
          accountId: 'account-1',
          labels: ['priority'],
          readyForDownload: !options?.paused,
          remoteState: 'exists' as const,
          isSubscription: false,
          profileImageCustom: false,
          ...(options?.withSyncIssue
            ? {
                syncProblemCode: 'auth_required',
                syncProblemMessage: 'Reconnect account',
              }
            : {}),
        },
      ],
    }
  }

  it('opens a custom source context menu from profile cards', () => {
    const onOpenSourceContextMenu = vi.fn()
    const onSelectSource = vi.fn()

    render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={onOpenSourceContextMenu}
        onSelectSource={onSelectSource}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={buildSnapshot()}
      />,
    )

    fireEvent.contextMenu(screen.getByRole('listitem'), {
      clientX: 164,
      clientY: 212,
    })

    expect(screen.getByText('visual_lab')).toBeTruthy()
    expect(screen.queryByText('@visual_lab')).toBeNull()
    expect(onSelectSource).toHaveBeenCalledWith('source-1')
    expect(onOpenSourceContextMenu).toHaveBeenCalledWith('source-1', 164, 212, false)
  })

  it('marks paused profiles (not ready for download) with a badge and card class', () => {
    render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={buildSnapshot({ paused: true })}
      />,
    )

    expect(screen.getByText('Paused')).toBeTruthy()
    expect(screen.getByRole('listitem').className).toContain('profile-card-paused')
  })

  it('does not render a ready profile as paused', () => {
    render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={buildSnapshot()}
      />,
    )

    expect(screen.queryByText('Paused')).toBeNull()
    expect(screen.getByRole('listitem').className).not.toContain('profile-card-paused')
  })

  it('does not stack a paused badge on top of a sync issue badge', () => {
    render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={buildSnapshot({ withSyncIssue: true, paused: true })}
      />,
    )

    // O card recua (classe de pausa) mas o pill "Paused" não empilha com o
    // badge de sync issue — só o aviso de sync aparece.
    expect(screen.getByRole('listitem').className).toContain('profile-card-paused')
    expect(screen.queryByText('Paused')).toBeNull()
    expect(screen.getByText('Auth required')).toBeTruthy()
  })

  it('hides the sync sections fingerprint by default (sections mode off)', () => {
    render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={buildSnapshot()}
      />,
    )

    // Sem chips e o card não recebe a classe de hover.
    expect(screen.queryByText('TL')).toBeNull()
    expect(screen.getByRole('listitem').className).not.toContain('profile-card-sections-hover')
  })

  it('renders the sync sections fingerprint when the overlay is set to always on', () => {
    localStorage.setItem('nc-sections-mode', 'on')

    render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={buildSnapshot()}
      />,
    )

    // Instagram: 5 slots, Timeline ligada por padrão.
    expect(screen.getByText('TL')).toBeTruthy()
    expect(screen.getByText('RE')).toBeTruthy()
    expect(screen.getByText('TG')).toBeTruthy()
    const timelineChip = screen.getByText('TL')
    expect(timelineChip.className).toContain('profile-section-chip-on')
    const reelsChip = screen.getByText('RE')
    expect(reelsChip.className).toContain('profile-section-chip-off')
  })

  it('adds the hover class to cards when the overlay is set to on-hover', () => {
    localStorage.setItem('nc-sections-mode', 'hover')

    render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={buildSnapshot()}
      />,
    )

    expect(screen.getByRole('listitem').className).toContain('profile-card-sections-hover')
    expect(screen.getByText('TL')).toBeTruthy()
  })

  it('filters profiles from different providers by their save path', () => {
    const onSavePathFilterChange = vi.fn()
    const snapshot = buildSnapshot()
    snapshot.sources = [
      snapshot.sources[0],
      {
        ...snapshot.sources[0],
        id: 'source-2',
        provider: 'tiktok',
        handle: '@short_video',
        displayName: 'short_video',
      },
      {
        ...snapshot.sources[0],
        id: 'source-3',
        provider: 'twitter',
        handle: '@news_feed',
        displayName: 'news_feed',
      },
    ]
    snapshot.sourceMediaPaths = {
      'source-1': 'D:\\Media\\Instagram\\visual_lab',
      'source-2': 'E:\\Media\\TikTok\\short_video',
      'source-3': 'E:\\Media\\TikTok\\news_feed',
    }

    render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSavePathFilterChange={onSavePathFilterChange}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        savePathFilter={'E:\\Media\\TikTok'}
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={snapshot}
      />,
    )

    const savePathFilter = screen.getByRole('combobox', { name: 'Filter by save path' })
    expect(savePathFilter).toBeTruthy()
    expect(screen.queryByText('visual_lab')).toBeNull()
    expect(screen.getByText('short_video')).toBeTruthy()
    expect(screen.getByText('news_feed')).toBeTruthy()

    fireEvent.change(savePathFilter, { target: { value: 'D:\\Media\\Instagram' } })
    expect(onSavePathFilterChange).toHaveBeenCalledWith('D:\\Media\\Instagram')
  })

  it('hides the save-path filter when every profile has the same base directory', () => {
    const snapshot = buildSnapshot()
    snapshot.sources = [
      snapshot.sources[0],
      {
        ...snapshot.sources[0],
        id: 'source-2',
        provider: 'tiktok',
        handle: '@short_video',
        displayName: 'short_video',
      },
    ]
    snapshot.sourceMediaPaths = {
      'source-1': 'D:\\Media\\Shared\\visual_lab',
      'source-2': 'D:\\Media\\Shared\\short_video',
    }

    render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={snapshot}
      />,
    )

    expect(screen.queryByRole('combobox', { name: 'Filter by save path' })).toBeNull()
  })

  it('shows a selected marker in grid cards', () => {
    const { container } = render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={['source-1']}
        serviceTab="all"
        snapshot={buildSnapshot()}
      />,
    )

    expect(container.querySelector('.profile-card-selected .profile-selection-indicator')).toBeTruthy()
  })

  it('shows a selected marker in list rows', () => {
    const { container } = render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={['source-1']}
        serviceTab="all"
        snapshot={buildSnapshot()}
      />,
    )

    fireEvent.click(screen.getByRole('button', { name: 'List view' }))

    expect(container.querySelector('.profile-list-row-selected .profile-selection-indicator-inline')).toBeTruthy()
  })

  it('shows sync issue badges in grid cards', () => {
    const { container } = render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={buildSnapshot({ withSyncIssue: true })}
      />,
    )

    const badge = container.querySelector('.profile-sync-issue-badge')
    expect(badge?.textContent).toContain('Auth required')
    expect(badge?.getAttribute('title')).toBe('Reconnect account')
  })

  it('shows sync issue badges in list rows', () => {
    const { container } = render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={buildSnapshot({ withSyncIssue: true })}
      />,
    )

    fireEvent.click(screen.getByRole('button', { name: 'List view' }))

    const badge = container.querySelector('.profile-sync-issue-pill')
    expect(badge?.textContent).toContain('Auth required')
    expect(badge?.getAttribute('title')).toBe('Reconnect account')
  })

  it('marks never-synced profiles with a stale-age badge in grid cards', () => {
    const { container } = render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={buildSnapshot()}
      />,
    )

    const badge = container.querySelector('.profile-sync-age-badge.profile-sync-age-never')
    expect(badge?.textContent).toBe('Never')
    expect(badge?.getAttribute('title')).toBe('Never synced')
  })

  it('shows the last-sync age in list row status column', () => {
    const { container } = render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={buildSnapshot()}
      />,
    )

    fireEvent.click(screen.getByRole('button', { name: 'List view' }))

    const inline = container.querySelector('.profile-sync-age-inline.profile-sync-age-never')
    expect(inline?.textContent).toBe('Never')
  })

  it('shows private profile badge label when code is private/restricted', () => {
    const snapshot = {
      ...createEmptyWorkspaceSnapshot(),
      sources: [
        {
          id: 'source-1',
          provider: 'instagram' as const,
          sourceKind: 'profile' as const,
          handle: '@visual_lab',
          displayName: 'visual_lab',
          accountId: 'account-1',
          labels: ['priority'],
          readyForDownload: true,
          remoteState: 'exists' as const,
          isSubscription: false,
          profileImageCustom: false,
          syncProblemCode: 'instagram_profile_private_or_restricted',
          syncProblemMessage: 'Private profile',
        },
      ],
    }
    const { container } = render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={snapshot}
      />,
    )

    const badge = container.querySelector('.profile-sync-issue-badge')
    expect(badge?.textContent).toContain('Private profile')
  })

  it('clears selection only when clicking empty workspace background', () => {
    const onClearSelection = vi.fn()
    const { container } = render(
      <ProfileWorkspace
        onClearSelection={onClearSelection}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={['source-1']}
        serviceTab="all"
        snapshot={buildSnapshot()}
      />,
    )

    const gridShell = container.querySelector('.profile-grid-shell')
    expect(gridShell).toBeTruthy()
    fireEvent.mouseDown(gridShell as Element)
    expect(onClearSelection).toHaveBeenCalledTimes(1)

    fireEvent.mouseDown(screen.getByRole('listitem'))
    expect(onClearSelection).toHaveBeenCalledTimes(1)
  })

  it('renders framed group containers when grouping headers are visible', () => {
    const { container } = render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={buildSnapshot()}
      />,
    )

    fireEvent.change(screen.getByRole('combobox', { name: 'Group by' }), {
      target: { value: 'category' },
    })

    expect(container.querySelector('.workspace-vframe.workspace-vframe-start')).toBeTruthy()
    expect(container.querySelector('.workspace-vframe .profile-grid')).toBeTruthy()
  })

  it('hides group content when the group is collapsed', () => {
    const { container } = render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={buildSnapshot()}
      />,
    )

    fireEvent.change(screen.getByRole('combobox', { name: 'Group by' }), {
      target: { value: 'category' },
    })

    fireEvent.click(screen.getByRole('button', { name: /regular/i }))

    expect(container.querySelector('.workspace-vframe.profile-group-collapsed')).toBeTruthy()
    expect(screen.queryByRole('listitem')).toBeNull()
  })

  it('renders the avatar image from the preview source', () => {
    ;(window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {}
    const snapshot = buildSnapshot()
    snapshot.sources[0] = {
      ...snapshot.sources[0],
      profileImagePath: 'C:/temp/ProfilePicture.jpg',
      profileImageCustom: false,
      lastSyncedAt: '2026-03-20T10:11:12Z',
    }

    render(
      <ProfileWorkspace
        onClearSelection={vi.fn()}
        onEditSource={vi.fn()}
        onOpenSourceContextMenu={vi.fn()}
        onSelectSource={vi.fn()}
        onServiceTabChange={vi.fn()}
        onSavePathFilterChange={vi.fn()}
        savePathFilter=""
        searchText=""
        selectedSourceIds={[]}
        serviceTab="all"
        snapshot={snapshot}
      />,
    )

    // Sem query string: o asset protocol do Windows não a ignora e falharia
    // ao abrir o arquivo. O cache-buster vive no nome do thumb versionado.
    const image = screen.getByRole('img', { name: 'visual_lab' })
    expect(image.getAttribute('src')).toBe('C:/temp/ProfilePicture.jpg')
    expect(image.getAttribute('loading')).toBe('lazy')
  })
})
