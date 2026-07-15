import { useEffect, useId, useRef, type ReactNode } from 'react'

interface InternalDialogProps {
  children: ReactNode
  closeVariant?: 'icon' | 'text'
  headerDensity?: 'compact' | 'default'
  height?: 'fit' | 'full'
  subtitle?: string
  title: string
  width?: 'medium' | 'large' | 'wide'
  onClose: () => void
}

export function InternalDialog({
  children,
  closeVariant = 'text',
  headerDensity = 'default',
  height = 'full',
  onClose,
  subtitle,
  title,
  width = 'large',
}: InternalDialogProps) {
  const dialogRef = useRef<HTMLDialogElement>(null)
  const titleId = useId()
  const subtitleId = useId()

  useEffect(() => {
    const dialog = dialogRef.current
    const returnFocusTo = document.activeElement instanceof HTMLElement ? document.activeElement : undefined
    if (!dialog) {
      return
    }

    if (typeof dialog.showModal === 'function') {
      if (dialog.open && typeof dialog.close === 'function') {
        dialog.close()
      }
      dialog.showModal()
    } else {
      dialog.setAttribute('open', '')
    }

    const firstFocusable = dialog.querySelector<HTMLElement>(
      '[autofocus], button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])',
    )
    firstFocusable?.focus()

    return () => {
      if (dialog.open && typeof dialog.close === 'function') {
        dialog.close()
      } else {
        dialog.removeAttribute('open')
      }
      returnFocusTo?.focus()
    }
  }, [])

  return (
    <dialog
      aria-describedby={subtitle ? subtitleId : undefined}
      aria-labelledby={titleId}
      className="dialog-overlay"
      open
      onCancel={(event) => {
        event.preventDefault()
        onClose()
      }}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose()
        }
      }}
      ref={dialogRef}
    >
      <section
        className={`dialog-shell dialog-shell-${width} dialog-shell-height-${height}`}
      >
        <header className={`dialog-header ${headerDensity === 'compact' ? 'dialog-header-compact' : ''}`}>
          <div className="dialog-title-block">
            <h2 id={titleId}>{title}</h2>
            {subtitle ? <p className="dialog-subtitle" id={subtitleId}>{subtitle}</p> : null}
          </div>
          {closeVariant === 'icon' ? (
            <button aria-label="Close" className="ghost-button dialog-close-button dialog-close-button-icon" onClick={onClose} type="button">
              <span aria-hidden>×</span>
            </button>
          ) : (
            <button className="ghost-button dialog-close-button" onClick={onClose} type="button">
              Close
            </button>
          )}
        </header>
        <div className="dialog-body">{children}</div>
      </section>
    </dialog>
  )
}
