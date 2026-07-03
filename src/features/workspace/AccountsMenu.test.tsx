// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { DEFAULT_PROVIDER_CATALOG } from '../../domain/defaults'
import type { ProviderAccount } from '../../domain/models'
import { AccountsMenu } from './AccountsMenu'

const ACCOUNTS: ProviderAccount[] = [
  {
    id: 'account-1',
    provider: 'instagram',
    displayName: 'Instagram Main',
    authMode: 'imported_session',
    authState: 'ready',
    capabilities: ['posts'],
    lastValidatedAt: '2026-03-10T00:00:00Z',
  },
  {
    id: 'account-2',
    provider: 'tiktok',
    displayName: 'TikTok Main',
    authMode: 'imported_session',
    authState: 'ready',
    capabilities: ['videos'],
    lastValidatedAt: '2026-03-10T00:00:00Z',
  },
]

describe('AccountsMenu', () => {
  afterEach(() => {
    cleanup()
  })

  it('routes new-account and account actions through the provider-first cascade', () => {
    const onAccountAction = vi.fn()
    const onCreateAccount = vi.fn()
    const onOpenSettings = vi.fn()

    render(
      <AccountsMenu
        accounts={ACCOUNTS}
        onAccountAction={onAccountAction}
        onCreateAccount={onCreateAccount}
        onOpenSettings={onOpenSettings}
        providerCatalog={DEFAULT_PROVIDER_CATALOG}
      />, 
    )

    expect(screen.queryByRole('button', { name: /^new account$/i })).toBeNull()
    expect(screen.queryByRole('button', { name: /^edit$/i })).toBeNull()

    fireEvent.mouseEnter(screen.getByRole('button', { name: /^instagram$/i }))
    expect(screen.getByRole('button', { name: /^new account$/i })).toBeTruthy()
    expect(screen.queryByRole('button', { name: /^edit$/i })).toBeNull()

    fireEvent.mouseEnter(screen.getByRole('button', { name: /instagram main/i }))
    expect(screen.getByRole('button', { name: /^edit$/i })).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: /^new account$/i }))
    fireEvent.click(screen.getByRole('button', { name: /^edit$/i }))

    expect(onCreateAccount).toHaveBeenCalledWith('instagram')
    expect(screen.queryByRole('button', { name: /^settings$/i })).toBeNull()
    expect(onOpenSettings).not.toHaveBeenCalled()
    expect(onAccountAction).toHaveBeenCalledWith('account-1', 'edit')
  })

  it('opens provider settings in create mode when a provider has no accounts', () => {
    const onOpenSettings = vi.fn()

    render(
      <AccountsMenu
        accounts={ACCOUNTS}
        onAccountAction={vi.fn()}
        onCreateAccount={vi.fn()}
        onOpenSettings={onOpenSettings}
        providerCatalog={DEFAULT_PROVIDER_CATALOG}
      />,
    )

    fireEvent.mouseEnter(screen.getByRole('button', { name: /^X \/ Twitter$/i }))
    expect(screen.queryByRole('button', { name: /^new account$/i })).toBeNull()

    fireEvent.click(screen.getByRole('button', { name: /^X \/ Twitter$/i }))
    expect(onOpenSettings).toHaveBeenCalledWith('twitter')
  })
})
