// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { InternalDialog } from './InternalDialog'

describe('InternalDialog', () => {
  afterEach(() => cleanup())

  it('uses native dialog semantics and restores focus when unmounted', () => {
    const onClose = vi.fn()
    const trigger = document.createElement('button')
    document.body.append(trigger)
    trigger.focus()

    const view = render(
      <InternalDialog onClose={onClose} title="Accessible dialog">
        <button type="button">First action</button>
      </InternalDialog>,
    )

    const dialog = screen.getByRole('dialog', { name: 'Accessible dialog' })
    expect(dialog.tagName).toBe('DIALOG')
    expect(screen.getByRole('button', { name: 'Close' })).toBe(document.activeElement)

    view.unmount()
    expect(trigger).toBe(document.activeElement)
    trigger.remove()
  })

  it('routes cancel through the close callback', () => {
    const onClose = vi.fn()
    render(<InternalDialog onClose={onClose} title="Cancelable"><p>Body</p></InternalDialog>)
    fireEvent(screen.getByRole('dialog'), new Event('cancel', { bubbles: false, cancelable: true }))
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
