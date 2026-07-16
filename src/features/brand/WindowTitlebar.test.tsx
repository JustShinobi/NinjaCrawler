// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { WindowTitlebar, type WindowController } from './WindowTitlebar'

afterEach(cleanup)

function createWindowController() {
  let resizedHandler: (() => void) | undefined
  let focusHandler: ((focused: boolean) => void) | undefined
  const controller: WindowController = {
    close: vi.fn().mockResolvedValue(undefined),
    isFocused: vi.fn().mockResolvedValue(true),
    isMaximized: vi.fn().mockResolvedValue(false),
    minimize: vi.fn().mockResolvedValue(undefined),
    onFocusChanged: vi.fn().mockImplementation(async (handler: (focused: boolean) => void) => {
      focusHandler = handler
      return vi.fn()
    }),
    onResized: vi.fn().mockImplementation(async (handler: () => void) => {
      resizedHandler = handler
      return vi.fn()
    }),
    startDragging: vi.fn().mockResolvedValue(undefined),
    toggleMaximize: vi.fn().mockResolvedValue(undefined),
  }
  return {
    controller,
    resized: () => resizedHandler?.(),
    setFocused: (focused: boolean) => focusHandler?.(focused),
  }
}

describe('WindowTitlebar', () => {
  it('exposes brand, optional title, trailing, and accessible window controls', async () => {
    const { controller } = createWindowController()
    render(
      <WindowTitlebar
        title="Queue Status"
        trailing={<span className="queue-global-state">Idle</span>}
        windowController={controller}
      />,
    )

    expect(screen.getByLabelText('NinjaCrawler')).toBeTruthy()
    expect(screen.getByText('Queue Status')).toBeTruthy()
    expect(screen.getByText('Idle')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Minimize window' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Maximize window' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Close window' })).toBeTruthy()
    await waitFor(() => expect(controller.isMaximized).toHaveBeenCalled())
  })

  it('routes dragging, double-click, and window controls through the controller', async () => {
    const { controller } = createWindowController()
    const rendered = render(<WindowTitlebar title="Runtime Log" windowController={controller} />)
    const dragRegion = rendered.container.querySelector('.window-titlebar-drag-region') as HTMLElement

    fireEvent.mouseDown(dragRegion, { button: 0, clientX: 20, clientY: 20 })
    fireEvent.mouseMove(dragRegion, { buttons: 1, clientX: 28, clientY: 20 })
    fireEvent.mouseUp(dragRegion, { button: 0 })
    fireEvent.mouseDown(dragRegion, { button: 0, detail: 2, clientX: 20, clientY: 20 })
    fireEvent.click(screen.getByRole('button', { name: 'Minimize window' }))
    fireEvent.click(screen.getByRole('button', { name: 'Maximize window' }))
    fireEvent.click(screen.getByRole('button', { name: 'Close window' }))

    expect(controller.startDragging).toHaveBeenCalledTimes(1)
    expect(controller.toggleMaximize).toHaveBeenCalledTimes(2)
    expect(controller.minimize).toHaveBeenCalledTimes(1)
    expect(controller.close).toHaveBeenCalledTimes(1)
  })

  it('updates the maximize control after a resize', async () => {
    const { controller, resized } = createWindowController()
    vi.mocked(controller.isMaximized).mockResolvedValueOnce(false).mockResolvedValueOnce(true)

    render(<WindowTitlebar title="Connector Runtimes" windowController={controller} />)
    await waitFor(() => expect(controller.onResized).toHaveBeenCalled())

    resized()

    expect(await screen.findByRole('button', { name: 'Restore window' })).toBeTruthy()
  })

  it('renders application menus when children are provided', () => {
    const { controller } = createWindowController()
    render(
      <WindowTitlebar showWordmark windowController={controller}>
        <button type="button">File</button>
      </WindowTitlebar>,
    )

    expect(screen.getByRole('navigation', { name: 'Application menu' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'File' })).toBeTruthy()
  })

  it('marks the titlebar focused/blurred for stacked window chrome contrast', async () => {
    const { controller, setFocused } = createWindowController()
    const rendered = render(<WindowTitlebar title="Profile View" windowController={controller} />)
    const titlebar = rendered.container.querySelector('.window-titlebar') as HTMLElement

    await waitFor(() => expect(controller.onFocusChanged).toHaveBeenCalled())
    expect(titlebar.classList.contains('is-window-focused')).toBe(true)
    expect(titlebar.getAttribute('data-window-focused')).toBe('true')

    setFocused(false)
    await waitFor(() => {
      expect(titlebar.classList.contains('is-window-blurred')).toBe(true)
      expect(titlebar.getAttribute('data-window-focused')).toBe('false')
      expect(document.documentElement.dataset.windowFocused).toBe('false')
    })

    setFocused(true)
    await waitFor(() => {
      expect(titlebar.classList.contains('is-window-focused')).toBe(true)
      expect(document.documentElement.dataset.windowFocused).toBe('true')
    })
  })
})
