import { getCurrentWindow } from '@tauri-apps/api/window'
import { useEffect, useRef, useState, type MouseEvent, type ReactNode } from 'react'
import { BrandLockup } from './BrandLockup'

export interface WindowController {
  close: () => Promise<void>
  isFocused?: () => Promise<boolean>
  isMaximized: () => Promise<boolean>
  minimize: () => Promise<void>
  onFocusChanged?: (handler: (focused: boolean) => void) => Promise<() => void>
  onResized: (handler: () => void) => Promise<() => void>
  startDragging: () => Promise<void>
  toggleMaximize: () => Promise<void>
}

/** @deprecated Prefer WindowController — alias retained for main-window call sites. */
export type MainWindowController = WindowController

interface WindowTitlebarProps {
  children?: ReactNode
  title?: string
  trailing?: ReactNode
  compactLockup?: boolean
  showWordmark?: boolean
  windowController?: WindowController
}

function createWindowController(): WindowController | undefined {
  if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) {
    return undefined
  }

  const appWindow = getCurrentWindow()
  return {
    close: () => appWindow.close(),
    isFocused: () => appWindow.isFocused(),
    isMaximized: () => appWindow.isMaximized(),
    minimize: () => appWindow.minimize(),
    onFocusChanged: (handler) => appWindow.onFocusChanged((event) => handler(event.payload)),
    onResized: (handler) => appWindow.onResized(handler),
    startDragging: () => appWindow.startDragging(),
    toggleMaximize: () => appWindow.toggleMaximize(),
  }
}

function setDocumentWindowFocus(focused: boolean) {
  if (typeof document === 'undefined') return
  document.documentElement.dataset.windowFocused = focused ? 'true' : 'false'
}

export function WindowTitlebar({
  children,
  title,
  trailing,
  compactLockup = true,
  showWordmark = false,
  windowController,
}: WindowTitlebarProps) {
  const [controller] = useState(() => windowController ?? createWindowController())
  const [maximized, setMaximized] = useState(false)
  // Defaults to focused so first paint / browser tests keep full contrast chrome.
  const [windowFocused, setWindowFocused] = useState(true)
  const dragStartRef = useRef<{ x: number; y: number } | undefined>(undefined)
  const dragStartedRef = useRef(false)

  useEffect(() => {
    if (!controller) {
      return undefined
    }

    let mounted = true
    const syncMaximizedState = () => {
      void controller
        .isMaximized()
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

  // Focused vs blurred titlebar: critical when undecorated windows stack
  // (Profile View over the main shell looks like one continuous charcoal field).
  useEffect(() => {
    let mounted = true
    const applyFocus = (focused: boolean) => {
      if (!mounted) return
      setWindowFocused(focused)
      setDocumentWindowFocus(focused)
    }

    applyFocus(typeof document !== 'undefined' ? document.hasFocus() : true)

    const onDomFocus = () => applyFocus(true)
    const onDomBlur = () => applyFocus(false)
    window.addEventListener('focus', onDomFocus)
    window.addEventListener('blur', onDomBlur)

    let unlistenFocus: (() => void) | undefined
    if (controller?.onFocusChanged) {
      void controller
        .onFocusChanged((focused) => applyFocus(focused))
        .then((dispose) => {
          if (mounted) unlistenFocus = dispose
          else dispose()
        })
        .catch(() => undefined)
    }
    if (controller?.isFocused) {
      void controller
        .isFocused()
        .then((focused) => applyFocus(focused))
        .catch(() => undefined)
    }

    return () => {
      mounted = false
      window.removeEventListener('focus', onDomFocus)
      window.removeEventListener('blur', onDomBlur)
      unlistenFocus?.()
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

  const hasMenus = Boolean(children)

  return (
    <header
      className={[
        'window-titlebar',
        'main-titlebar',
        windowFocused ? 'is-window-focused' : 'is-window-blurred',
      ].join(' ')}
      data-menu-root
      data-window-focused={windowFocused ? 'true' : 'false'}
    >
      <BrandLockup compact={compactLockup} showWordmark={showWordmark} />
      {hasMenus ? (
        <nav aria-label="Application menu" className="window-titlebar-menu main-titlebar-menu">
          {children}
        </nav>
      ) : null}
      {title ? (
        <div className="window-titlebar-title">
          <span className="window-titlebar-title-text">{title}</span>
        </div>
      ) : null}
      <div
        aria-hidden="true"
        className="window-titlebar-drag-region main-titlebar-drag-region"
        onMouseDown={handleDragMouseDown}
        onMouseLeave={resetDragGesture}
        onMouseMove={handleDragMouseMove}
        onMouseUp={resetDragGesture}
      />
      {trailing ? <div className="window-titlebar-trailing">{trailing}</div> : null}
      <div aria-label="Window controls" className="window-titlebar-controls main-titlebar-controls" role="group">
        <button
          aria-label="Minimize window"
          className="window-titlebar-control main-titlebar-control"
          onClick={() => runWindowCommand(controller?.minimize)}
          title="Minimize"
          type="button"
        >
          <svg aria-hidden="true" viewBox="0 0 16 16">
            <path d="M3 11.5h10v1H3z" />
          </svg>
        </button>
        <button
          aria-label={maximized ? 'Restore window' : 'Maximize window'}
          className="window-titlebar-control main-titlebar-control"
          onClick={() => runWindowCommand(controller?.toggleMaximize)}
          title={maximized ? 'Restore' : 'Maximize'}
          type="button"
        >
          {maximized ? (
            <svg aria-hidden="true" viewBox="0 0 16 16">
              <path d="M5 3h8v8h-2V5H5V3zm-2 3h8v7H3V6zm1 1v5h6V7H4z" />
            </svg>
          ) : (
            <svg aria-hidden="true" viewBox="0 0 16 16">
              <path d="M3 3h10v10H3V3zm1 1v8h8V4H4z" />
            </svg>
          )}
        </button>
        <button
          aria-label="Close window"
          className="window-titlebar-control window-titlebar-control-close main-titlebar-control main-titlebar-control-close"
          onClick={() => runWindowCommand(controller?.close)}
          title="Close"
          type="button"
        >
          <svg aria-hidden="true" viewBox="0 0 16 16">
            <path d="M4.15 3.44 8 7.29l3.85-3.85.71.71L8.71 8l3.85 3.85-.71.71L8 8.71l-3.85 3.85-.71-.71L7.29 8 3.44 4.15l.71-.71z" />
          </svg>
        </button>
      </div>
    </header>
  )
}
