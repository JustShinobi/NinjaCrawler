// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SingleVideosPage } from './SingleVideosPage'

const listSingleVideos = vi.fn()
const loadSingleVideosRootStatus = vi.fn()
const setSingleVideosRootMock = vi.fn()
const pickImportRootFolder = vi.fn()

vi.mock('../../bridge/desktop', () => ({
  deleteSingleVideo: vi.fn(),
  enqueueSingleVideoDownload: vi.fn(),
  listSingleVideos: () => listSingleVideos(),
  loadSingleVideosRootStatus: () => loadSingleVideosRootStatus(),
  loadWorkspaceSnapshot: vi.fn(),
  openExternalTarget: vi.fn(),
  pickImportRootFolder: () => pickImportRootFolder(),
  revealMediaInFolder: vi.fn(),
  setSingleVideosRoot: (path: string, move: boolean) => setSingleVideosRootMock(path, move),
  subscribeToSingleVideosChanged: () => Promise.resolve(() => {}),
  upsertSourceProfile: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (path: string) => `asset://${path}` }))

const sampleVideo = {
  id: 'video-1',
  provider: 'tiktok',
  sourceUrl: 'https://www.tiktok.com/@a/video/1',
  relativePath: '1.mp4',
  absolutePath: 'S:/NinjaCrawler/Single videos/1.mp4',
  mediaType: 'video',
  downloadedAt: '2026-07-20T00:00:00Z',
  files: [
    {
      relativePath: '1.mp4',
      absolutePath: 'S:/NinjaCrawler/Single videos/1.mp4',
      mediaType: 'video',
    },
  ],
}

beforeEach(() => {
  vi.clearAllMocks()
  listSingleVideos.mockResolvedValue([sampleVideo])
  setSingleVideosRootMock.mockResolvedValue([sampleVideo])
})

afterEach(cleanup)

describe('SingleVideosPage folder handling', () => {
  it('warns when the catalog points at a folder without the media', async () => {
    loadSingleVideosRootStatus.mockResolvedValue({
      root: 'S:/NinjaCrawler/Single videos',
      mediaRootDefault: 'S:/NinjaCrawler/Single videos',
      totalCount: 12,
      missingCount: 12,
    })

    render(<SingleVideosPage />)

    expect(await screen.findByText(/12 of 12 media files were not found/i)).toBeTruthy()
  })

  it('relocates without moving when media is missing, and moves otherwise', async () => {
    loadSingleVideosRootStatus.mockResolvedValue({
      root: 'S:/NinjaCrawler/Single videos',
      mediaRootDefault: 'S:/NinjaCrawler/Single videos',
      totalCount: 12,
      missingCount: 12,
    })
    pickImportRootFolder.mockResolvedValue('F:/SCrawler/Data/Single videos')

    render(<SingleVideosPage />)
    fireEvent.click(await screen.findByRole('button', { name: /change folder/i }))

    // Com mídia faltando o padrão é relocalizar, não mover por cima.
    const moveCheckbox = await screen.findByRole('checkbox')
    expect((moveCheckbox as HTMLInputElement).checked).toBe(false)

    fireEvent.click(screen.getByRole('button', { name: /^apply$/i }))
    await waitFor(() =>
      expect(setSingleVideosRootMock).toHaveBeenCalledWith('F:/SCrawler/Data/Single videos', false),
    )
  })

  it('defaults to moving the media when nothing is missing', async () => {
    loadSingleVideosRootStatus.mockResolvedValue({
      root: 'F:/SCrawler/Data/Single videos',
      mediaRootDefault: 'S:/NinjaCrawler/Single videos',
      totalCount: 12,
      missingCount: 0,
    })
    pickImportRootFolder.mockResolvedValue('S:/NinjaCrawler')

    render(<SingleVideosPage />)
    fireEvent.click(await screen.findByRole('button', { name: /folder…/i }))

    const moveCheckbox = await screen.findByRole('checkbox')
    expect((moveCheckbox as HTMLInputElement).checked).toBe(true)

    fireEvent.click(screen.getByRole('button', { name: /^apply$/i }))
    await waitFor(() =>
      expect(setSingleVideosRootMock).toHaveBeenCalledWith('S:/NinjaCrawler', true),
    )
  })
})
