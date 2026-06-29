// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SourceMediaGallery } from '../../domain/models'
import { ProfileViewPage } from './ProfileViewPage'

const bridgeMocks = vi.hoisted(() => ({
  loadSourceMediaGallery: vi.fn(),
  deleteSourceMedia: vi.fn(),
  loadWorkspaceSnapshot: vi.fn(),
  openExternalTarget: vi.fn(),
  openMediaFile: vi.fn(),
  revealMediaInFolder: vi.fn(),
  subscribeToProfileViewSource: vi.fn(),
}))

vi.mock('../../bridge/desktop', () => bridgeMocks)
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (path: string) => `asset://${path}` }))

function galleryFixture(): SourceMediaGallery {
  // 2026-05-19 ~ capturedAt
  const day = Math.floor(Date.parse('2026-05-19T12:00:00Z') / 1000)
  return {
    sourceId: 'src-1',
    provider: 'tiktok',
    handle: 'gaaby.tls',
    profileUrl: 'https://www.tiktok.com/gaaby.tls',
    posts: [
      {
        postId: '7624199329925958920',
        postUrl: 'https://www.tiktok.com/@gaaby.tls/video/7624199329925958920',
        capturedAt: day,
        mediaType: 'video',
        section: 'timeline',
        files: [
          { relativePath: 'a.mp4', absolutePath: 'S:/x/a.mp4', mediaType: 'video' },
        ],
      },
      {
        postId: '7600000000000000000',
        postUrl: 'https://www.tiktok.com/@gaaby.tls/video/7600000000000000000',
        capturedAt: day - 60,
        mediaType: 'slideshow',
        section: 'timeline',
        files: [
          { relativePath: 'b_0.jpeg', absolutePath: 'S:/x/b_0.jpeg', mediaType: 'image' },
          { relativePath: 'b_1.jpeg', absolutePath: 'S:/x/b_1.jpeg', mediaType: 'image' },
        ],
      },
    ],
  }
}

function instagramGalleryFixture(): SourceMediaGallery {
  const day = Math.floor(Date.parse('2026-05-19T12:00:00Z') / 1000)
  return {
    sourceId: 'ig-1',
    provider: 'instagram',
    handle: 'bibiss.sz',
    profileUrl: 'https://www.instagram.com/bibiss.sz/',
    posts: [
      {
        postId: 'feed-1',
        postUrl: 'https://www.instagram.com/p/CyAbC-1_x/',
        capturedAt: day,
        mediaType: 'image',
        section: 'timeline',
        files: [{ relativePath: 'feed.jpg', absolutePath: 'S:/ig/feed.jpg', mediaType: 'image' }],
      },
      {
        postId: 'reel-1',
        postUrl: 'https://www.instagram.com/p/DzReel99/',
        capturedAt: day - 120,
        mediaType: 'video',
        section: 'reels',
        files: [{ relativePath: 'reel.mp4', absolutePath: 'S:/ig/reel.mp4', mediaType: 'video' }],
      },
    ],
  }
}

describe('ProfileViewPage', () => {
  beforeEach(() => {
    localStorage.clear()
    for (const mock of Object.values(bridgeMocks)) {
      mock.mockReset()
    }
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue(galleryFixture())
    bridgeMocks.loadWorkspaceSnapshot.mockResolvedValue({ sources: [] })
    bridgeMocks.subscribeToProfileViewSource.mockResolvedValue(() => undefined)
    bridgeMocks.openExternalTarget.mockResolvedValue(undefined)
  })

  afterEach(() => cleanup())

  it('renders the profile header and a day section with posts', async () => {
    render(<ProfileViewPage initialSourceId="src-1" />)

    expect(await screen.findByRole('heading', { name: /gaaby\.tls/i })).toBeTruthy()
    await waitFor(() => expect(bridgeMocks.loadSourceMediaGallery).toHaveBeenCalledWith('src-1'))
    // 2 posts on the same day → one day section, slideshow badge present
    expect(screen.getByText('▣ 2')).toBeTruthy()
    expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(2)
  })

  it('opens the original post link online', async () => {
    render(<ProfileViewPage initialSourceId="src-1" />)
    const onlineButtons = await screen.findAllByRole('button', { name: 'Online' })
    fireEvent.click(onlineButtons[0])
    await waitFor(() => {
      expect(bridgeMocks.openExternalTarget).toHaveBeenCalledWith(
        'https://www.tiktok.com/@gaaby.tls/video/7624199329925958920',
      )
    })
  })

  it('switches to the "all media" grid and persists the choice', async () => {
    const { unmount } = render(<ProfileViewPage initialSourceId="src-1" />)

    // Default mode is grouped by day.
    expect(await screen.findByRole('button', { name: /by day/i })).toHaveProperty('ariaPressed', 'true')

    fireEvent.click(screen.getByRole('button', { name: /all media/i }))

    // Grid mode is active, both posts still rendered, choice persisted.
    expect(screen.getByRole('button', { name: /all media/i })).toHaveProperty('ariaPressed', 'true')
    expect(screen.getByRole('button', { name: /by day/i })).toHaveProperty('ariaPressed', 'false')
    expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(2)
    expect(localStorage.getItem('profileView.mode')).toBe('grid')

    // Re-mounting restores the saved mode.
    unmount()
    render(<ProfileViewPage initialSourceId="src-1" />)
    expect(await screen.findByRole('button', { name: /all media/i })).toHaveProperty('ariaPressed', 'true')
  })

  it('adjusts and persists the thumbnail density', async () => {
    render(<ProfileViewPage initialSourceId="src-1" />)
    await screen.findAllByRole('button', { name: /open preview/i })

    fireEvent.click(screen.getByRole('button', { name: /larger thumbnails/i }))
    expect(localStorage.getItem('profileView.density')).toBe('3')

    fireEvent.click(screen.getByRole('button', { name: /smaller thumbnails/i }))
    fireEvent.click(screen.getByRole('button', { name: /smaller thumbnails/i }))
    expect(localStorage.getItem('profileView.density')).toBe('1')
  })

  it('differentiates Instagram feed and reels with a section filter', async () => {
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue(instagramGalleryFixture())
    render(<ProfileViewPage initialSourceId="ig-1" />)

    // Filter chips: All + Feed + Reels (timeline is labelled "Feed" on Instagram).
    expect(await screen.findByRole('button', { name: /^Feed$/ })).toBeTruthy()
    const reelsChip = screen.getByRole('button', { name: /^Reels$/ })
    // Both posts visible up front.
    expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(2)

    // Filtering to Reels keeps only the reel post.
    fireEvent.click(reelsChip)
    expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(1)
    expect(reelsChip).toHaveProperty('ariaPressed', 'true')
  })

  it('rebuilds the Instagram post link from the shortcode', async () => {
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue(instagramGalleryFixture())
    render(<ProfileViewPage initialSourceId="ig-1" />)
    const onlineButtons = await screen.findAllByRole('button', { name: 'Online' })
    fireEvent.click(onlineButtons[0])
    await waitFor(() => {
      expect(bridgeMocks.openExternalTarget).toHaveBeenCalledWith('https://www.instagram.com/p/CyAbC-1_x/')
    })
  })

  it('renders media progressively and grows the window on scroll', async () => {
    // Capture IntersectionObserver instances (jsdom has none) so we can fire it.
    const observers: Array<(entries: Array<{ isIntersecting: boolean }>) => void> = []
    class MockIntersectionObserver {
      constructor(cb: (entries: Array<{ isIntersecting: boolean }>) => void) {
        observers.push(cb)
      }
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    vi.stubGlobal('IntersectionObserver', MockIntersectionObserver)

    const day = Math.floor(Date.parse('2026-05-19T12:00:00Z') / 1000)
    // Image posts keep the DOM light (an <img> per card vs a heavier <video>).
    const posts = Array.from({ length: 140 }, (_, index) => ({
      postId: `p-${index}`,
      postUrl: `https://www.tiktok.com/@bulk/photo/${index}`,
      capturedAt: day - index,
      mediaType: 'image' as const,
      section: 'timeline',
      files: [{ relativePath: `${index}.jpg`, absolutePath: `S:/x/${index}.jpg`, mediaType: 'image' }],
    }))
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue({
      sourceId: 'bulk',
      provider: 'tiktok',
      handle: 'bulk',
      profileUrl: 'https://www.tiktok.com/@bulk',
      posts,
    } satisfies SourceMediaGallery)

    render(<ProfileViewPage initialSourceId="bulk" />)

    // First window only mounts the initial batch, not all 150 posts.
    await waitFor(
      () => expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(120),
      { timeout: 5000 },
    )

    // The sentinel becoming visible grows the window to cover the rest.
    expect(observers.length).toBeGreaterThan(0)
    act(() => observers[observers.length - 1]([{ isIntersecting: true }]))
    await waitFor(
      () => expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(140),
      { timeout: 5000 },
    )

    vi.unstubAllGlobals()
  }, 20000)

  it('opens the lightbox when a thumbnail is clicked', async () => {
    render(<ProfileViewPage initialSourceId="src-1" />)
    const thumbs = await screen.findAllByRole('button', { name: /open preview/i })
    fireEvent.click(thumbs[0])
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByRole('button', { name: /open online/i })).toBeTruthy()
    expect(within(dialog).getByRole('button', { name: /reveal in folder/i })).toBeTruthy()
  })

  it('multi-selects posts and deletes them via the backend', async () => {
    // After deletion the backend returns the gallery without the first post.
    const remaining = galleryFixture()
    remaining.posts = remaining.posts.slice(1)
    bridgeMocks.deleteSourceMedia.mockResolvedValue(remaining)

    render(<ProfileViewPage initialSourceId="src-1" />)
    await screen.findAllByRole('button', { name: /open preview/i })

    // Enter selection mode and pick the first post.
    fireEvent.click(screen.getByRole('button', { name: /^select$/i }))
    fireEvent.click(screen.getAllByRole('button', { name: /select media/i })[0])
    expect(screen.getByText(/1 selected/i)).toBeTruthy()

    // Delete selected → confirm dialog → confirm.
    fireEvent.click(screen.getByRole('button', { name: /delete selected/i }))
    const dialog = await screen.findByRole('dialog')
    fireEvent.click(within(dialog).getByRole('button', { name: /^delete$/i }))

    await waitFor(() =>
      expect(bridgeMocks.deleteSourceMedia).toHaveBeenCalledWith('src-1', ['a.mp4']),
    )
    // Gallery refreshed to the backend result (first post gone).
    await waitFor(() =>
      expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(1),
    )
  })

  it('deletes a single post from its card action', async () => {
    bridgeMocks.deleteSourceMedia.mockResolvedValue({ ...galleryFixture(), posts: [] })
    render(<ProfileViewPage initialSourceId="src-1" />)
    await screen.findAllByRole('button', { name: /open preview/i })

    // Card action delete (accessible name is the visible text "Delete").
    fireEvent.click(screen.getAllByRole('button', { name: 'Delete' })[0])
    const dialog = await screen.findByRole('dialog')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Delete' }))

    await waitFor(() =>
      expect(bridgeMocks.deleteSourceMedia).toHaveBeenCalledWith('src-1', ['a.mp4']),
    )
  })

  it('hides the Online link for stories (ephemeral, 24h)', async () => {
    const day = Math.floor(Date.parse('2026-05-19T12:00:00Z') / 1000)
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue({
      sourceId: 'ig-1',
      provider: 'instagram',
      handle: 'someone',
      profileUrl: 'https://www.instagram.com/someone/',
      posts: [
        {
          postId: 'feed',
          postUrl: 'https://www.instagram.com/p/AAA/',
          capturedAt: day,
          mediaType: 'image',
          section: 'timeline',
          files: [{ relativePath: 'f.jpg', absolutePath: 'S:/f.jpg', mediaType: 'image' }],
        },
        {
          postId: 'story',
          postUrl: undefined,
          capturedAt: day - 60,
          mediaType: 'image',
          section: 'stories',
          files: [{ relativePath: 's.jpg', absolutePath: 'S:/s.jpg', mediaType: 'image' }],
        },
      ],
    } as SourceMediaGallery)

    render(<ProfileViewPage initialSourceId="ig-1" />)
    await waitFor(() =>
      expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(2),
    )
    // Only the feed post offers an Online link; the story does not.
    expect(screen.getAllByRole('button', { name: 'Online' }).length).toBe(1)
  })
})
