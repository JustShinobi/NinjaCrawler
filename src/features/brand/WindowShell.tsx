import type { ReactNode } from 'react'

interface WindowShellProps {
  titlebar: ReactNode
  children: ReactNode
  /** Compact utility gutters vs default window padding. */
  density?: 'default' | 'compact' | 'inspector'
  className?: string
  contentClassName?: string
}

export function WindowShell({
  titlebar,
  children,
  density = 'default',
  className,
  contentClassName,
}: WindowShellProps) {
  const densityClass =
    density === 'compact'
      ? 'window-shell-compact'
      : density === 'inspector'
        ? 'window-shell-inspector'
        : ''

  return (
    <div className={['window-shell', densityClass, className].filter(Boolean).join(' ')}>
      {titlebar}
      <div className={['window-shell-content', contentClassName].filter(Boolean).join(' ')}>{children}</div>
    </div>
  )
}
