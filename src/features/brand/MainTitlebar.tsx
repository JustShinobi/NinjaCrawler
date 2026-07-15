import { getCurrentWindow } from '@tauri-apps/api/window'
import { useEffect, useRef, useState, type MouseEvent, type ReactNode } from 'react'
import { BrandLockup } from './BrandLockup'

export interface MainWindowController {
  close: () => Promise<void>
  isMaximized: () => Promise<boolean>
  minimize: () => Promise<void>
  onResized: (handler: () => void) => Promise<() => void>
  startDragging: () => Promise<void>
  toggleMaximize: () => Promise<void>
}

interface MainTitlebarProps {
  children: ReactNode
  windowController?: MainWindowController
}

function createMainWindowController(): MainWindowController | undefined {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) {
    return undefined
  }

  const appWindow = getCurrentWindow()
  return {
    close: () => appWindow.close(),
    isMaximized: () => appWindow.isMaximized(),
    minimize: () => appWindow.minimize(),
    onResized: (handler) => appWindow.onResized(handler),
    startDragging: () => appWindow.startDragging(),
    toggleMaximize: () => appWindow.toggleMaximize(),
  }
}

export function MainTitlebar({ children, windowController }: MainTitlebarProps) {
  const [controller] = useState(() => windowController ?? createMainWindowController())
  const [maximized, setMaximized] = useState(false)
  const dragStartRef = useRef<{ x: number; y: number } | undefined>(undefined)
  const dragStartedRef = useRef(false)

  useEffect(() => {
    if (!controller) {
      return undefined
    }

    let mounted = true
    const syncMaximizedState = () => {
      void controller.isMaximized()
        .then((value) => {
          if (mounted) {
            setMaximized(value)
          }
        })
        .catch(() => undefined)
    }

    syncMaximizedState()
    const unlistenPromise = controller.onResized(syncMaximizedState).catch(() => undefined)

    return () => {
      mounted = false
      void unlistenPromise.then((unlisten) => unlisten?.())
    }
  }, [controller])

  const runWindowCommand = (command: (() => Promise<void>) | undefined) => {
    if (command) {
      void command().catch(() => undefined)
    }
  }

  const handleDragMouseDown = (event: MouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) {
      return
    }

    if (event.detail >= 2) {
      resetDragGesture()
      runWindowCommand(controller?.toggleMaximize)
      return
    }

    dragStartRef.current = { x: event.clientX, y: event.clientY }
    dragStartedRef.current = false
  }

  const handleDragMouseMove = (event: MouseEvent<HTMLDivElement>) => {
    const start = dragStartRef.current
    if (!controller || !start || dragStartedRef.current || event.buttons !== 1) {
      return
    }

    if (Math.hypot(event.clientX - start.x, event.clientY - start.y) < 3) {
      return
    }

    dragStartedRef.current = true
    runWindowCommand(controller.startDragging)
  }

  const resetDragGesture = () => {
    dragStartRef.current = undefined
    dragStartedRef.current = false
  }

  return (
    <header className="main-titlebar" data-menu-root>
      <BrandLockup compact />
      <nav aria-label="Application menu" className="main-titlebar-menu">
        {children}
      </nav>
      <div
        aria-hidden="true"
        className="main-titlebar-drag-region"
        onMouseDown={handleDragMouseDown}
        onMouseLeave={resetDragGesture}
        onMouseMove={handleDragMouseMove}
        onMouseUp={resetDragGesture}
      />
      <div aria-label="Window controls" className="main-titlebar-controls" role="group">
        <button
          aria-label="Minimize window"
          className="main-titlebar-control"
          onClick={() => runWindowCommand(controller?.minimize)}
          title="Minimize"
          type="button"
        >
          <svg aria-hidden="true" viewBox="0 0 16 16"><path d="M3 11.5h10v1H3z" /></svg>
        </button>
        <button
          aria-label={maximized ? 'Restore window' : 'Maximize window'}
          className="main-titlebar-control"
          onClick={() => runWindowCommand(controller?.toggleMaximize)}
          title={maximized ? 'Restore' : 'Maximize'}
          type="button"
        >
          {maximized ? (
            <svg aria-hidden="true" viewBox="0 0 16 16">
              <path d="M5 3h8v8h-2V5H5V3zm-2 3h8v7H3V6zm1 1v5h6V7H4z" />
            </svg>
          ) : (
            <svg aria-hidden="true" viewBox="0 0 16 16"><path d="M3 3h10v10H3V3zm1 1v8h8V4H4z" /></svg>
          )}
        </button>
        <button
          aria-label="Close window"
          className="main-titlebar-control main-titlebar-control-close"
          onClick={() => runWindowCommand(controller?.close)}
          title="Close"
          type="button"
        >
          <svg aria-hidden="true" viewBox="0 0 16 16"><path d="M4.15 3.44 8 7.29l3.85-3.85.71.71L8.71 8l3.85 3.85-.71.71L8 8.71l-3.85 3.85-.71-.71L7.29 8 3.44 4.15l.71-.71z" /></svg>
        </button>
      </div>
    </header>
  )
}
