import type { ReactNode } from 'react'
import { WindowTitlebar, type WindowController } from './WindowTitlebar'

export type { WindowController as MainWindowController } from './WindowTitlebar'

interface MainTitlebarProps {
  children: ReactNode
  windowController?: WindowController
}

export function MainTitlebar({ children, windowController }: MainTitlebarProps) {
  return (
    <WindowTitlebar compactLockup showWordmark windowController={windowController}>
      {children}
    </WindowTitlebar>
  )
}
