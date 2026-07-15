// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { MainTitlebar, type MainWindowController } from './MainTitlebar'

afterEach(cleanup)

function createWindowController() {
  let resizedHandler: (() => void) | undefined
  const controller: MainWindowController = {
    close: vi.fn().mockResolvedValue(undefined),
    isMaximized: vi.fn().mockResolvedValue(false),
    minimize: vi.fn().mockResolvedValue(undefined),
    onResized: vi.fn().mockImplementation(async (handler: () => void) => {
      resizedHandler = handler
      return vi.fn()
    }),
    startDragging: vi.fn().mockResolvedValue(undefined),
    toggleMaximize: vi.fn().mockResolvedValue(undefined),
  }
  return { controller, resized: () => resizedHandler?.() }
}

describe('MainTitlebar', () => {
  it('exposes the brand, application menu, and accessible window controls', async () => {
    const { controller } = createWindowController()
    render(
      <MainTitlebar windowController={controller}>
        <button type="button">File</button>
      </MainTitlebar>,
    )

    expect(screen.getByLabelText('NinjaCrawler')).toBeTruthy()
    expect(screen.getByRole('navigation', { name: 'Application menu' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Minimize window' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Maximize window' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Close window' })).toBeTruthy()
    await waitFor(() => expect(controller.isMaximized).toHaveBeenCalled())
  })

  it('routes dragging, double-click, and window controls through the controller', async () => {
    const controller = createWindowController()
    const rendered = render(
      <MainTitlebar windowController={controller.controller}>
        <button type="button">File</button>
      </MainTitlebar>,
    )
    const dragRegion = rendered.container.querySelector('.main-titlebar-drag-region') as HTMLElement

    fireEvent.mouseDown(dragRegion, { button: 0, clientX: 20, clientY: 20 })
    fireEvent.mouseMove(dragRegion, { buttons: 1, clientX: 28, clientY: 20 })
    fireEvent.mouseUp(dragRegion, { button: 0 })
    fireEvent.mouseDown(dragRegion, { button: 0, detail: 2, clientX: 20, clientY: 20 })
    fireEvent.click(screen.getByRole('button', { name: 'Minimize window' }))
    fireEvent.click(screen.getByRole('button', { name: 'Maximize window' }))
    fireEvent.click(screen.getByRole('button', { name: 'Close window' }))

    expect(controller.controller.startDragging).toHaveBeenCalledTimes(1)
    expect(controller.controller.toggleMaximize).toHaveBeenCalledTimes(2)
    expect(controller.controller.minimize).toHaveBeenCalledTimes(1)
    expect(controller.controller.close).toHaveBeenCalledTimes(1)
  })

  it('updates the maximize control after a resize', async () => {
    const { controller, resized } = createWindowController()
    vi.mocked(controller.isMaximized)
      .mockResolvedValueOnce(false)
      .mockResolvedValueOnce(true)

    render(
      <MainTitlebar windowController={controller}>
        <button type="button">File</button>
      </MainTitlebar>,
    )
    await waitFor(() => expect(controller.onResized).toHaveBeenCalled())

    resized()

    expect(await screen.findByRole('button', { name: 'Restore window' })).toBeTruthy()
  })
})
