// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { SourceMediaGallery } from '../../domain/models'
import { ProfileViewPage } from './ProfileViewPage'

const bridgeMocks = vi.hoisted(() => ({
  loadSourceMediaGallery: vi.fn(),
  loadMediaThumbnails: vi.fn(),
  deleteSourceMedia: vi.fn(),
  loadWorkspaceSnapshot: vi.fn(),
  openExternalTarget: vi.fn(),
  openMediaFile: vi.fn(),
  revealMediaInFolder: vi.fn(),
  runSourceSync: vi.fn(),
  subscribeToProfileViewSource: vi.fn(),
  subscribeToSourceSyncQueue: vi.fn(),
}))

vi.mock('../../bridge/desktop', () => bridgeMocks)
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (path: string) => `asset://${path}` }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    close: vi.fn(),
    isMaximized: () => Promise.resolve(false),
    minimize: vi.fn(),
    onResized: () => Promise.resolve(() => undefined),
    startDragging: vi.fn(),
    toggleMaximize: vi.fn(),
    setTitle: vi.fn(() => Promise.resolve()),
  }),
}))

// jsdom não faz layout, então a virtualização real (que depende de medir a
// viewport/linhas) renderizaria zero linhas. Trocamos o virtualizer por um que
// renderiza todas as linhas — os testes checam ordem/contagem, não o windowing.
vi.mock('@tanstack/react-virtual', () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 200,
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({ index, key: index, start: index * 200 })),
    measureElement: () => undefined,
    measure: () => undefined,
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
        viewCount: 10,
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
        viewCount: 100,
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

// TikTok com Timeline + Likes: datas de criação e download distintas e autores
// nos likes, para exercitar filtro de seção, ordenação e busca por autor.
function tiktokMixedFixture(): SourceMediaGallery {
  const day = Math.floor(Date.parse('2026-05-19T12:00:00Z') / 1000)
  const img = (name: string) => [
    { relativePath: `${name}.jpg`, absolutePath: `S:/${name}.jpg`, mediaType: 'image' },
  ]
  return {
    sourceId: 'tk-1',
    provider: 'tiktok',
    handle: 'creator',
    profileUrl: 'https://www.tiktok.com/@creator',
    posts: [
      { postId: 't1', capturedAt: day, downloadedAt: day + 100, mediaType: 'image', section: 'timeline', files: img('t1') },
      { postId: 't2', capturedAt: day - 1000, downloadedAt: day + 200, mediaType: 'image', section: 'timeline', files: img('t2') },
      { postId: 'l1', capturedAt: day - 500, downloadedAt: day + 50, author: 'alice', mediaType: 'image', section: 'likes', files: img('l1') },
      { postId: 'l2', capturedAt: day - 200, downloadedAt: day + 300, author: 'bob', mediaType: 'image', section: 'likes', files: img('l2') },
    ],
  } as SourceMediaGallery
}

/** Ordem (por id abreviado) das miniaturas montadas, na sequência do DOM. */
function thumbOrder(container: HTMLElement): string[] {
  return Array.from(container.querySelectorAll('.profile-view-thumb img')).map(
    (img) =>
      (img as HTMLImageElement)
        .getAttribute('src')
        ?.replace('asset://S:/', '')
        .replace('.jpg', '') ?? '',
  )
}

describe('ProfileViewPage', () => {
  beforeEach(() => {
    localStorage.clear()
    for (const mock of Object.values(bridgeMocks)) {
      mock.mockReset()
    }
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue(galleryFixture())
    // ffmpeg "disponível" mas sem thumbs prontos → cards de vídeo viram
    // placeholder (sem <video> no grid), o comportamento padrão do app.
    bridgeMocks.loadMediaThumbnails.mockResolvedValue({ available: true, thumbs: {} })
    bridgeMocks.loadWorkspaceSnapshot.mockResolvedValue({ sources: [] })
    bridgeMocks.subscribeToProfileViewSource.mockResolvedValue(() => undefined)
    bridgeMocks.subscribeToSourceSyncQueue.mockResolvedValue(() => undefined)
    bridgeMocks.openExternalTarget.mockResolvedValue(undefined)
    bridgeMocks.openMediaFile.mockResolvedValue(undefined)
    bridgeMocks.revealMediaInFolder.mockResolvedValue(undefined)
    bridgeMocks.runSourceSync.mockResolvedValue(undefined)
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

  it('uses the generated thumbnail for a photo card, falling back to the original', async () => {
    // Thumb pronto só para o 1º frame do slideshow; o vídeo continua sem thumb.
    bridgeMocks.loadMediaThumbnails.mockResolvedValue({
      available: true,
      thumbs: { 'S:/x/b_0.jpeg': 'S:/x/.thumbs/b_0.jpeg.jpg' },
    })
    const { container } = render(<ProfileViewPage initialSourceId="src-1" />)
    await screen.findByRole('heading', { name: /gaaby\.tls/i })

    await waitFor(() => {
      const imgs = Array.from(
        container.querySelectorAll('.profile-view-thumb img'),
      ) as HTMLImageElement[]
      const photo = imgs.find((img) => img.getAttribute('src')?.includes('b_0'))
      expect(photo?.getAttribute('src')).toBe('asset://S:/x/.thumbs/b_0.jpeg.jpg')
    })
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

  it('reveals the media folder from the card action', async () => {
    render(<ProfileViewPage initialSourceId="src-1" />)
    const folderButtons = await screen.findAllByRole('button', { name: 'Folder' })

    fireEvent.click(folderButtons[0])

    await waitFor(() => {
      expect(bridgeMocks.revealMediaInFolder).toHaveBeenCalledWith('S:/x/a.mp4')
    })
  })

  it('opens the profile link from the header', async () => {
    render(<ProfileViewPage initialSourceId="src-1" />)

    fireEvent.click(await screen.findByRole('button', { name: /open profile online/i }))

    await waitFor(() => {
      expect(bridgeMocks.openExternalTarget).toHaveBeenCalledWith('https://www.tiktok.com/gaaby.tls')
    })
  })

  it('sorts media by TikTok view count when Popularity is selected', async () => {
    render(<ProfileViewPage initialSourceId="src-1" />)
    await screen.findAllByRole('button', { name: 'Online' })

    // Abre o menu de ordenação e escolhe o eixo de popularidade (views).
    fireEvent.click(screen.getByRole('button', { name: 'Sort order' }))
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Popularity' }))
    fireEvent.click(screen.getAllByRole('button', { name: 'Online' })[0])

    await waitFor(() => {
      expect(bridgeMocks.openExternalTarget).toHaveBeenCalledWith(
        'https://www.tiktok.com/@gaaby.tls/video/7600000000000000000',
      )
    })
    expect(localStorage.getItem('profileView.sortField')).toBe('popularity')
  })

  it('switches to the flat grid and persists the choice', async () => {
    const { unmount } = render(<ProfileViewPage initialSourceId="src-1" />)

    // Default mode is grouped by day.
    expect(await screen.findByRole('button', { name: /by day/i })).toHaveProperty('ariaPressed', 'true')

    fireEvent.click(screen.getByRole('button', { name: /flat grid/i }))

    // Grid mode is active, both posts still rendered, choice persisted.
    expect(screen.getByRole('button', { name: /flat grid/i })).toHaveProperty('ariaPressed', 'true')
    expect(screen.getByRole('button', { name: /by day/i })).toHaveProperty('ariaPressed', 'false')
    expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(2)
    expect(localStorage.getItem('profileView.mode')).toBe('grid')

    // Re-mounting restores the saved mode.
    unmount()
    render(<ProfileViewPage initialSourceId="src-1" />)
    expect(await screen.findByRole('button', { name: /flat grid/i })).toHaveProperty('ariaPressed', 'true')
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
    expect(await screen.findByRole('button', { name: /^Feed /i })).toBeTruthy()
    const reelsChip = screen.getByRole('button', { name: /^Reels /i })
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

  it('opens the lightbox when a thumbnail is clicked', async () => {
    render(<ProfileViewPage initialSourceId="src-1" />)
    const thumbs = await screen.findAllByRole('button', { name: /open preview/i })
    fireEvent.click(thumbs[0])
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByRole('button', { name: /open online/i })).toBeTruthy()
    expect(within(dialog).getByRole('button', { name: /reveal in folder/i })).toBeTruthy()
  })

  it('runs lightbox online, open file, and reveal actions through the bridge', async () => {
    render(<ProfileViewPage initialSourceId="src-1" />)
    const thumbs = await screen.findAllByRole('button', { name: /open preview/i })
    fireEvent.click(thumbs[0])
    const dialog = await screen.findByRole('dialog')

    fireEvent.click(within(dialog).getByRole('button', { name: /open online/i }))
    fireEvent.click(within(dialog).getByRole('button', { name: /open file/i }))
    fireEvent.click(within(dialog).getByRole('button', { name: /reveal in folder/i }))

    await waitFor(() => {
      expect(bridgeMocks.openExternalTarget).toHaveBeenCalledWith(
        'https://www.tiktok.com/@gaaby.tls/video/7624199329925958920',
      )
      expect(bridgeMocks.openMediaFile).toHaveBeenCalledWith('S:/x/a.mp4')
      expect(bridgeMocks.revealMediaInFolder).toHaveBeenCalledWith('S:/x/a.mp4')
    })
  })

  it('keeps horizontal arrows for video seeking instead of Profile View navigation', async () => {
    render(<ProfileViewPage initialSourceId="src-1" />)
    const thumbs = await screen.findAllByRole('button', { name: /open preview/i })
    fireEvent.click(thumbs[0])
    const dialog = await screen.findByRole('dialog')

    fireEvent.keyDown(document, { key: 'ArrowRight' })

    expect(within(dialog).getByRole('button', { name: /open online/i })).toBeTruthy()
    fireEvent.click(within(dialog).getByRole('button', { name: /open online/i }))
    await waitFor(() => {
      expect(bridgeMocks.openExternalTarget).toHaveBeenCalledWith(
        'https://www.tiktok.com/@gaaby.tls/video/7624199329925958920',
      )
    })
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

  it('enters selection from a checkbox alone and ranges with shift+click', async () => {
    render(<ProfileViewPage initialSourceId="src-1" />)
    await screen.findAllByRole('button', { name: /open preview/i })

    const checkboxes = screen.getAllByRole('button', { name: /select media/i })

    // Checking a box (without the "Select" toggle) reveals the delete action.
    fireEvent.click(checkboxes[0])
    expect(screen.getByText(/1 selected/i)).toBeTruthy()
    expect(screen.getByRole('button', { name: /delete selected/i })).toBeTruthy()

    // Shift+click the last box selects the whole range in between.
    fireEvent.click(screen.getAllByRole('button', { name: /select media/i }).at(-1)!, {
      shiftKey: true,
    })
    expect(screen.getByText(/2 selected/i)).toBeTruthy()
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

  it('hides the Online link for live stories but keeps it for highlights', async () => {
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
          // Highlights (backend section `stories`) persist → keep the Online link.
          postId: 'highlight',
          postUrl: undefined,
          capturedAt: day - 30,
          mediaType: 'image',
          section: 'stories',
          files: [{ relativePath: 'h.jpg', absolutePath: 'S:/h.jpg', mediaType: 'image' }],
        },
        {
          // Live stories (backend section `stories_user`) are ephemeral → no link.
          postId: 'story',
          postUrl: undefined,
          capturedAt: day - 60,
          mediaType: 'image',
          section: 'stories_user',
          files: [{ relativePath: 's.jpg', absolutePath: 'S:/s.jpg', mediaType: 'image' }],
        },
      ],
    } as SourceMediaGallery)

    render(<ProfileViewPage initialSourceId="ig-1" />)
    await waitFor(() =>
      expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(3),
    )
    // Feed + highlight expose Online; the ephemeral live story does not.
    expect(screen.getAllByRole('button', { name: 'Online' }).length).toBe(2)
    // The two story-like sections render distinct chips (Stories vs Highlights).
    expect(screen.getByRole('button', { name: /^Highlights /i })).toBeTruthy()
    expect(screen.getByRole('button', { name: /^Stories /i })).toBeTruthy()
  })

  it('groups highlights by album and can switch back to by-day', async () => {
    const day = Math.floor(Date.parse('2026-05-19T12:00:00Z') / 1000)
    const story = (id: string, albums: string[], at: number) => ({
      postId: id,
      capturedAt: at,
      mediaType: 'image' as const,
      section: 'stories',
      albums,
      files: [{ relativePath: `${id}.jpg`, absolutePath: `S:/${id}.jpg`, mediaType: 'image' }],
    })
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue({
      sourceId: 'ig-1',
      provider: 'instagram',
      handle: 'someone',
      profileUrl: 'https://www.instagram.com/someone/',
      posts: [
        {
          // Plain feed post — not a highlight member, stays out of Highlights.
          postId: 'feed',
          postUrl: 'https://www.instagram.com/p/AAA/',
          capturedAt: day,
          mediaType: 'image',
          section: 'timeline',
          albums: [],
          files: [{ relativePath: 'f.jpg', absolutePath: 'S:/f.jpg', mediaType: 'image' }],
        },
        {
          // Feed post that is ALSO a highlight member (cross-ref, file lives in
          // the feed) — must appear under the "Venda" album in Highlights.
          postId: 'feedmember',
          postUrl: 'https://www.instagram.com/p/BBB/',
          capturedAt: day - 5,
          mediaType: 'image',
          section: 'timeline',
          albums: ['Venda'],
          files: [{ relativePath: 'b.jpg', absolutePath: 'S:/b.jpg', mediaType: 'image' }],
        },
        story('v1', ['Venda'], day - 10),
        story('c1', ['CATSU'], day - 20),
        story('c2', ['CATSU'], day - 30),
      ],
    } as SourceMediaGallery)

    render(<ProfileViewPage initialSourceId="ig-1" />)
    await screen.findByRole('button', { name: /^Highlights /i })

    // Enter the Highlights section → defaults to album grouping.
    fireEvent.click(screen.getByRole('button', { name: /^Highlights /i }))
    expect(screen.getByRole('button', { name: /by album/i })).toBeTruthy()
    expect(screen.getByText('Venda')).toBeTruthy()
    expect(screen.getByText('CATSU')).toBeTruthy()
    // Highlights shows the 4 album members (v1, c1, c2, feed-member) but not the
    // plain feed post → 4 preview tiles, not 5.
    expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(4)

    // Switching to "By day" leaves album mode (album headers gone).
    fireEvent.click(screen.getByRole('button', { name: /by day/i }))
    expect(screen.queryByText('Venda')).toBeNull()
  })

  it('labels the TikTok timeline chip "Timeline" and filters Likes', async () => {
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue(tiktokMixedFixture())
    render(<ProfileViewPage initialSourceId="tk-1" />)

    // Section chips read Timeline (not "Posts") + Likes.
    expect(await screen.findByRole('button', { name: /^Timeline /i })).toBeTruthy()
    expect(screen.queryByRole('button', { name: /^Posts /i })).toBeNull()
    const likesChip = screen.getByRole('button', { name: /^Likes /i })

    // "All" shows the four posts; filtering to Likes keeps only the two likes.
    expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(4)
    fireEvent.click(likesChip)
    expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(2)
  })

  it('searches Likes by author via the inline field', async () => {
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue(tiktokMixedFixture())
    render(<ProfileViewPage initialSourceId="tk-1" />)

    fireEvent.click(await screen.findByRole('button', { name: /^Likes /i }))
    // The magnifier only exists on the Likes tab; expand it and search.
    fireEvent.click(screen.getByRole('button', { name: /search likes by author/i }))
    fireEvent.change(screen.getByRole('searchbox', { name: /search likes by author/i }), {
      target: { value: 'bob' },
    })
    // Only bob's like survives; alice's is filtered out.
    expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(1)

    // The search field is absent outside the Likes tab.
    fireEvent.click(screen.getByRole('button', { name: /^Timeline /i }))
    expect(screen.queryByRole('button', { name: /search likes by author/i })).toBeNull()
  })

  it('matches authors typed with @ and falls back to the file name', async () => {
    const fixture = tiktokMixedFixture()
    // Like without a backend author — search must fall back to the file name.
    fixture.posts.push({
      postId: 'l3',
      capturedAt: Math.floor(Date.parse('2026-05-19T12:00:00Z') / 1000) - 300,
      mediaType: 'image',
      section: 'likes',
      files: [
        { relativePath: 'Liked/carol_777.jpg', absolutePath: 'S:/carol_777.jpg', mediaType: 'image' },
      ],
    } as SourceMediaGallery['posts'][number])
    const alicePost = fixture.posts.find((post) => post.author === 'alice')
    if (!alicePost) throw new Error('fixture must contain alice')
    alicePost.files[0].relativePath =
      'Liked/.alice_1779997681_7645031804658814215.mp4'
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue(fixture)
    render(<ProfileViewPage initialSourceId="tk-1" />)

    fireEvent.click(await screen.findByRole('button', { name: /^Likes /i }))
    fireEvent.click(screen.getByRole('button', { name: /search likes by author/i }))
    const input = screen.getByRole('searchbox', { name: /search likes by author/i })

    // Leading @ is ignored (users paste @handles).
    fireEvent.change(input, { target: { value: '@alice' } })
    expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(1)

    // No author on the post → the file name (which carries the uploader) matches.
    fireEvent.change(input, { target: { value: 'carol' } })
    expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(1)

    // Mesmo quando há author no ledger, colar o basename completo precisa achar
    // a mídia (incluindo nomes válidos que começam com ponto).
    fireEvent.change(input, {
      target: { value: '.alice_1779997681_7645031804658814215.mp4' },
    })
    expect(screen.getAllByRole('button', { name: /open preview/i }).length).toBe(1)
  })

  it('groups Likes by user with per-author headers, most liked first', async () => {
    const fixture = tiktokMixedFixture()
    // Second like from alice so her group outranks bob's.
    fixture.posts.push({
      postId: 'l4',
      capturedAt: Math.floor(Date.parse('2026-05-19T12:00:00Z') / 1000) - 400,
      author: 'alice',
      mediaType: 'image',
      section: 'likes',
      files: [{ relativePath: 'l4.jpg', absolutePath: 'S:/l4.jpg', mediaType: 'image' }],
    } as SourceMediaGallery['posts'][number])
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue(fixture)
    render(<ProfileViewPage initialSourceId="tk-1" />)

    // The Likes tab defaults to grouping by user.
    fireEvent.click(await screen.findByRole('button', { name: /^Likes /i }))
    expect(screen.getByRole('button', { name: /by user/i })).toHaveProperty('ariaPressed', 'true')
    const alice = screen.getByText('@alice')
    const bob = screen.getByText('@bob')
    // Alice (2 likes) ranks above bob (1) in the DOM.
    expect(alice.compareDocumentPosition(bob) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()

    // Switching to Flat grid drops the author headers.
    fireEvent.click(screen.getByRole('button', { name: /flat grid/i }))
    expect(screen.queryByText('@alice')).toBeNull()
  })

  it('deletes the active post with Shift+Del in the lightbox, without a dialog', async () => {
    const remaining = galleryFixture()
    remaining.posts = remaining.posts.slice(1)
    bridgeMocks.deleteSourceMedia.mockResolvedValue(remaining)
    render(<ProfileViewPage initialSourceId="src-1" />)

    const thumbs = await screen.findAllByRole('button', { name: /open preview/i })
    fireEvent.click(thumbs[0])
    await screen.findByRole('dialog')

    fireEvent.keyDown(document, { key: 'Delete', shiftKey: true })
    await waitFor(() =>
      expect(bridgeMocks.deleteSourceMedia).toHaveBeenCalledWith('src-1', ['a.mp4']),
    )
    // No confirmation dialog: Shift is the confirmation.
    expect(screen.queryByText(/Delete media\?/i)).toBeNull()
    // The lightbox stays open showing the next item (the slideshow post).
    await waitFor(() => {
      const dialog = screen.getByRole('dialog')
      expect(within(dialog).getByRole('button', { name: /open online/i })).toBeTruthy()
    })
  })

  it('shows the author above the media in the lightbox', async () => {
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue(tiktokMixedFixture())
    render(<ProfileViewPage initialSourceId="tk-1" />)

    fireEvent.click(await screen.findByRole('button', { name: /^Likes /i }))
    fireEvent.click(screen.getAllByRole('button', { name: /open preview/i })[0])
    const dialog = await screen.findByRole('dialog')
    // Newest like first → bob's; his @handle heads the lightbox.
    expect(within(dialog).getByText('@bob')).toBeTruthy()
  })

  it('sorts by creation and download date in both directions', async () => {
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue(tiktokMixedFixture())
    const { container } = render(<ProfileViewPage initialSourceId="tk-1" />)
    await screen.findByRole('button', { name: /^Timeline /i })

    // Grid mode drops the day headers, so the thumbnail order is the sort order.
    fireEvent.click(screen.getByRole('button', { name: /flat grid/i }))
    // Default: creation date, newest first.
    expect(thumbOrder(container)).toEqual(['t1', 'l2', 'l1', 't2'])

    // Switch the axis to download date (menu stays open for both toggles).
    fireEvent.click(screen.getByRole('button', { name: /sort order/i }))
    fireEvent.click(screen.getByRole('menuitemradio', { name: /download date/i }))
    expect(thumbOrder(container)).toEqual(['l2', 't2', 't1', 'l1'])

    // Flip the direction to oldest first.
    fireEvent.click(screen.getByRole('menuitemradio', { name: /oldest first/i }))
    expect(thumbOrder(container)).toEqual(['l1', 't1', 't2', 'l2'])
  })

  it('places TikTok stats refresh in the header and queues a manual stats refresh', async () => {
    bridgeMocks.loadSourceMediaGallery.mockResolvedValue(tiktokMixedFixture())
    const { container } = render(<ProfileViewPage initialSourceId="tk-1" />)

    const refreshButton = await screen.findByRole('button', { name: /refresh media stats/i })
    expect(refreshButton.closest('.profile-view-header-actions')).toBeTruthy()
    const toolbar = container.querySelector('.profile-view-toolbar')
    expect(toolbar).toBeTruthy()
    expect(within(toolbar as HTMLElement).queryByRole('button', { name: /refresh media stats/i })).toBeNull()

    fireEvent.click(refreshButton)

    await waitFor(() =>
      expect(bridgeMocks.runSourceSync).toHaveBeenCalledWith('tk-1', {
        trigger: 'manual_stats_refresh',
        runMode: 'refresh_media_stats',
      }),
    )
    await waitFor(() => expect(refreshButton.textContent).toMatch(/queued/i))
  })
})
