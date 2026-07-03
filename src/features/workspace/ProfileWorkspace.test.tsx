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
  getPreviewSource: (source: { profileImagePath?: string; profileImageCustom: boolean; lastSyncedAt?: string }) => {
    if (!source.profileImagePath) return undefined
    if (!source.profileImageCustom && source.lastSyncedAt) {
      const version = encodeURIComponent(source.lastSyncedAt)
      return `${source.profileImagePath}?sync=${version}`
    }
    return source.profileImagePath
  },
}))

describe('ProfileWorkspace', () => {
  afterEach(() => {
    cleanup()
    localStorage.clear()
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
  })

  function buildSnapshot(options?: { withSyncIssue?: boolean }): WorkspaceSnapshot {
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
          readyForDownload: true,
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

    expect(container.querySelector('.profile-group.profile-group-framed')).toBeTruthy()
    expect(container.querySelector('.profile-group-content')).toBeTruthy()
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

    expect(container.querySelector('.profile-group.profile-group-collapsed')).toBeTruthy()
    expect(screen.queryByRole('listitem')).toBeNull()
  })

  it('uses profile image path directly and cache-busts synced provider avatars', () => {
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

    const image = screen.getByRole('img', { name: 'visual_lab' })
    expect(image.getAttribute('src')).toBe(
      'C:/temp/ProfilePicture.jpg?sync=2026-03-20T10%3A11%3A12Z',
    )
  })

  it('does not cache-bust custom profile avatars', () => {
    ;(window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {}
    const snapshot = buildSnapshot()
    snapshot.sources[0] = {
      ...snapshot.sources[0],
      profileImagePath: 'C:/temp/ProfilePicture.jpg',
      profileImageCustom: true,
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

    const image = screen.getByRole('img', { name: 'visual_lab' })
    expect(image.getAttribute('src')).toBe('C:/temp/ProfilePicture.jpg')
  })
})
