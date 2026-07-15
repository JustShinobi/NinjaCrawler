import { useMemo, useRef, useState } from 'react'
import type { KeyboardEvent as ReactKeyboardEvent } from 'react'
import type { ProviderAccount, ProviderDescriptor, ProviderKey } from '../../domain/models'
import { formatAuthState } from '../accounts/formatAuthState'

type AccountMenuAction = 'edit' | 'clone' | 'delete'
type AccountActionKey = AccountMenuAction

interface AccountsMenuProps {
  accounts: ProviderAccount[]
  providerCatalog: ProviderDescriptor[]
  onAccountAction: (accountId: string, action: AccountMenuAction) => void
  onCreateAccount: (provider: ProviderKey) => void
  onOpenSettings: (provider: ProviderKey, accountId?: string) => void
}

const ACCOUNT_ACTIONS: AccountActionKey[] = ['edit', 'clone', 'delete']

export function AccountsMenu({
  accounts,
  providerCatalog,
  onAccountAction,
  onCreateAccount,
  onOpenSettings,
}: AccountsMenuProps) {
  const [activeProvider, setActiveProvider] = useState<ProviderKey>()
  const [activeAccountId, setActiveAccountId] = useState<string>()
  const [providerFlyoutTop, setProviderFlyoutTop] = useState(0)
  const [accountFlyoutTop, setAccountFlyoutTop] = useState(0)

  const providerButtonRefs = useRef<Record<string, HTMLButtonElement | null>>({})
  const accountButtonRefs = useRef<Record<string, HTMLButtonElement | null>>({})
  const commandButtonRefs = useRef<Record<'create-account', HTMLButtonElement | null>>({
    'create-account': null,
  })
  const accountActionButtonRefs = useRef<Record<AccountActionKey, HTMLButtonElement | null>>({
    edit: null,
    clone: null,
    delete: null,
  })

  const providerGroups = useMemo(
    () =>
      providerCatalog.map((provider) => ({
        provider,
        accounts: accounts.filter((account) => account.provider === provider.key),
      })),
    [accounts, providerCatalog],
  )

  const activeProviderGroup = providerGroups.find((group) => group.provider.key === activeProvider)
  const providerAccounts = activeProviderGroup?.accounts ?? []
  const activeAccount = providerAccounts.find((account) => account.id === activeAccountId)
  function focusProvider(provider: ProviderKey) {
    providerButtonRefs.current[provider]?.focus()
  }

  function focusAccount(accountId: string) {
    accountButtonRefs.current[accountId]?.focus()
  }

  function focusAccountAction(action: AccountActionKey) {
    accountActionButtonRefs.current[action]?.focus()
  }

  function setProviderHover(provider: ProviderKey, top: number) {
    setActiveProvider(provider)
    setActiveAccountId(undefined)
    setProviderFlyoutTop(top)
    setAccountFlyoutTop(0)
  }

  function openProvider(provider: ProviderKey, top: number) {
    const providerGroup = providerGroups.find((group) => group.provider.key === provider)
    if (!providerGroup) {
      return
    }

    if (providerGroup.accounts.length === 0) {
      onOpenSettings(provider)
      return
    }

    setActiveProvider(provider)
    setActiveAccountId(undefined)
    setProviderFlyoutTop(top)
    setAccountFlyoutTop(0)
  }

  function setAccountHover(accountId: string, top: number) {
    setActiveAccountId(accountId)
    setAccountFlyoutTop(top)
  }

  function handleProviderKeyDown(
    event: ReactKeyboardEvent<HTMLButtonElement>,
    provider: ProviderKey,
    index: number,
    hasAccounts: boolean,
  ) {
    switch (event.key) {
      case 'ArrowDown': {
        event.preventDefault()
        const nextProvider = providerCatalog[(index + 1) % providerCatalog.length]
        if (nextProvider) {
          focusProvider(nextProvider.key)
          setProviderHover(nextProvider.key, providerButtonRefs.current[nextProvider.key]?.offsetTop ?? 0)
        }
        break
      }
      case 'ArrowUp': {
        event.preventDefault()
        const previousProvider = providerCatalog[(index - 1 + providerCatalog.length) % providerCatalog.length]
        if (previousProvider) {
          focusProvider(previousProvider.key)
          setProviderHover(previousProvider.key, providerButtonRefs.current[previousProvider.key]?.offsetTop ?? 0)
        }
        break
      }
      case 'ArrowRight':
      case 'Enter':
      case ' ': {
        event.preventDefault()
        if (!hasAccounts) {
          onOpenSettings(provider)
          return
        }

        openProvider(provider, event.currentTarget.offsetTop)
        requestAnimationFrame(() => {
          const firstAccount = providerGroups.find((group) => group.provider.key === provider)?.accounts[0]
          if (firstAccount) {
            focusAccount(firstAccount.id)
          }
        })
        break
      }
      default:
        break
    }
  }

  function handleAccountKeyDown(
    event: ReactKeyboardEvent<HTMLButtonElement>,
    accountId: string,
    index: number,
  ) {
    switch (event.key) {
      case 'ArrowDown': {
        event.preventDefault()
        const nextAccount = providerAccounts[(index + 1) % providerAccounts.length]
        if (nextAccount) {
          focusAccount(nextAccount.id)
          setActiveAccountId(nextAccount.id)
        }
        break
      }
      case 'ArrowUp': {
        event.preventDefault()
        const previousAccount = providerAccounts[(index - 1 + providerAccounts.length) % providerAccounts.length]
        if (previousAccount) {
          focusAccount(previousAccount.id)
          setActiveAccountId(previousAccount.id)
        }
        break
      }
      case 'ArrowLeft': {
        event.preventDefault()
        if (activeProvider) {
          focusProvider(activeProvider)
        }
        break
      }
      case 'ArrowRight': {
        event.preventDefault()
        setAccountHover(accountId, event.currentTarget.offsetTop)
        requestAnimationFrame(() => focusAccountAction('edit'))
        break
      }
      case 'Enter':
      case ' ': {
        event.preventDefault()
        onAccountAction(accountId, 'edit')
        break
      }
      default:
        break
    }
  }

  function handleCommandKeyDown(
    event: ReactKeyboardEvent<HTMLButtonElement>,
  ) {
    switch (event.key) {
      case 'ArrowLeft':
        event.preventDefault()
        if (activeProvider) {
          focusProvider(activeProvider)
        }
        break
      default:
        break
    }
  }

  function handleAccountActionKeyDown(
    event: ReactKeyboardEvent<HTMLButtonElement>,
    action: AccountActionKey,
  ) {
    switch (event.key) {
      case 'ArrowLeft':
        event.preventDefault()
        if (activeAccountId) {
          focusAccount(activeAccountId)
        }
        break
      case 'ArrowDown':
      case 'ArrowUp': {
        event.preventDefault()
        const currentIndex = ACCOUNT_ACTIONS.indexOf(action)
        const nextIndex =
          event.key === 'ArrowDown'
            ? (currentIndex + 1) % ACCOUNT_ACTIONS.length
            : (currentIndex - 1 + ACCOUNT_ACTIONS.length) % ACCOUNT_ACTIONS.length
        focusAccountAction(ACCOUNT_ACTIONS[nextIndex])
        break
      }
      default:
        break
    }
  }

  return (
    <div className="accounts-menu-cascade" data-menu-root>
      <div className="accounts-menu-panel accounts-menu-panel-providers" role="menu">
        {providerGroups.map(({ provider, accounts: providerAccountsGroup }, index) => {
          const isActive = provider.key === activeProvider
          const hasAccounts = providerAccountsGroup.length > 0

          return (
            <button
              aria-expanded={hasAccounts ? isActive : undefined}
              aria-haspopup={hasAccounts ? 'menu' : undefined}
              className={isActive ? 'accounts-menu-entry accounts-menu-entry-active' : 'accounts-menu-entry'}
              key={provider.key}
              onClick={(event) => openProvider(provider.key, event.currentTarget.offsetTop)}
              onKeyDown={(event) => handleProviderKeyDown(event, provider.key, index, hasAccounts)}
              onMouseEnter={(event) => setProviderHover(provider.key, event.currentTarget.offsetTop)}
              ref={(element) => {
                providerButtonRefs.current[provider.key] = element
              }}
              type="button"
            >
              <strong>{provider.displayName}</strong>
              {hasAccounts ? <em aria-hidden="true">▶</em> : null}
            </button>
          )
        })}
      </div>

      {activeProviderGroup && providerAccounts.length > 0 ? (
        <div className="accounts-menu-flyout" style={{ top: `${providerFlyoutTop}px` }}>
          <div className="accounts-menu-panel accounts-menu-panel-accounts" role="menu">
            {providerAccounts.map((account, index) => (
              <button
                aria-expanded={account.id === activeAccountId}
                aria-haspopup="menu"
                className={
                  account.id === activeAccountId
                    ? 'accounts-menu-entry accounts-menu-entry-active'
                    : 'accounts-menu-entry'
                }
                key={account.id}
                onClick={() => onAccountAction(account.id, 'edit')}
                onKeyDown={(event) => handleAccountKeyDown(event, account.id, index)}
                onMouseEnter={(event) => setAccountHover(account.id, event.currentTarget.offsetTop)}
                ref={(element) => {
                  accountButtonRefs.current[account.id] = element
                }}
                type="button"
              >
                <strong>{account.displayName}</strong>
                <span>{formatAuthState(account.authState)}</span>
                <em aria-hidden="true">▶</em>
              </button>
            ))}

            <div className="accounts-menu-separator" />

            <button
              className="accounts-menu-entry accounts-menu-entry-command"
              onClick={() => onCreateAccount(activeProviderGroup.provider.key)}
              onKeyDown={(event) => handleCommandKeyDown(event)}
              onMouseEnter={() => setActiveAccountId(undefined)}
              ref={(element) => {
                commandButtonRefs.current['create-account'] = element
              }}
              type="button"
            >
              <strong>New account</strong>
            </button>
          </div>

          {activeAccount ? (
            <div className="accounts-menu-flyout accounts-menu-flyout-tertiary" style={{ top: `${accountFlyoutTop}px` }}>
              <div className="accounts-menu-panel accounts-menu-panel-actions" role="menu">
                <button
                  className="accounts-menu-entry accounts-menu-entry-command"
                  onClick={() => onAccountAction(activeAccount.id, 'edit')}
                  onKeyDown={(event) => handleAccountActionKeyDown(event, 'edit')}
                  ref={(element) => {
                    accountActionButtonRefs.current.edit = element
                  }}
                  type="button"
                >
                  <strong>Edit</strong>
                </button>
                <button
                  className="accounts-menu-entry accounts-menu-entry-command"
                  onClick={() => onAccountAction(activeAccount.id, 'clone')}
                  onKeyDown={(event) => handleAccountActionKeyDown(event, 'clone')}
                  ref={(element) => {
                    accountActionButtonRefs.current.clone = element
                  }}
                  type="button"
                >
                  <strong>Clone</strong>
                </button>
                <button
                  className="accounts-menu-entry accounts-menu-entry-command accounts-menu-entry-danger"
                  onClick={() => onAccountAction(activeAccount.id, 'delete')}
                  onKeyDown={(event) => handleAccountActionKeyDown(event, 'delete')}
                  ref={(element) => {
                    accountActionButtonRefs.current.delete = element
                  }}
                  type="button"
                >
                  <strong>Delete</strong>
                </button>
              </div>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}
