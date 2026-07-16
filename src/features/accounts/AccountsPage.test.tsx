// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type {
  ProviderAccount,
  ProviderAccountCookie,
  ProviderAccountEditor,
  ProviderAccountSession,
  ProviderAccountSettingValue,
  ProviderAccountUpsert,
  WorkspaceSnapshot,
} from '../../domain/models'
import { createEmptyWorkspaceSnapshot } from '../../domain/workspaceSnapshot'
import { AccountsPage, type AccountsPageProps } from './AccountsPage'

const useAppStoreMock = vi.fn()

vi.mock('../../state/appStore', () => ({
  useAppStore: (selector: (state: Record<string, unknown>) => unknown) => useAppStoreMock(selector),
}))

function createAccount(overrides: Partial<ProviderAccount> = {}): ProviderAccount {
  return {
    id: 'account-1',
    provider: 'instagram',
    displayName: 'Instagram Main',
    authMode: 'imported_session',
    authState: 'ready',
    capabilities: ['posts', 'stories'],
    lastValidatedAt: '2026-03-10T00:00:00Z',
    ...overrides,
  }
}

function createSession(overrides: Partial<ProviderAccountSession> = {}): ProviderAccountSession {
  return {
    accountId: 'account-1',
    authMode: 'imported_session',
    sessionFormat: 'cookie_json',
    fingerprint: 'sha256:abc123',
    cookieCount: 1,
    importedAt: '2026-03-10T00:00:00Z',
    lastValidatedAt: '2026-03-10T00:00:00Z',
    lastValidationError: undefined,
    hasSecret: true,
    ...overrides,
  }
}

function createCookie(overrides: Partial<ProviderAccountCookie> = {}): ProviderAccountCookie {
  return {
    domain: '.instagram.com',
    name: 'sessionid',
    value: 'abc123',
    path: '/',
    expiresAt: undefined,
    secure: true,
    httpOnly: true,
    ...overrides,
  }
}

function createEditor(account: ProviderAccount, settings: ProviderAccountSettingValue[] = []): ProviderAccountEditor {
  return {
    account,
    session: null,
    settings,
  }
}

function renderPage(
  snapshotOverrides: Partial<WorkspaceSnapshot> = {},
  pageProps: AccountsPageProps = {},
  storeOverrides: Partial<Record<string, unknown>> = {},
) {
  const snapshot: WorkspaceSnapshot = {
    ...createEmptyWorkspaceSnapshot(),
    ...snapshotOverrides,
  }
  const fallbackAccount = snapshot.accounts[0] ?? createAccount()

  const store = {
    snapshot,
    pendingCommand: undefined,
    upsertProviderAccount: vi.fn<(draft: ProviderAccountUpsert) => Promise<WorkspaceSnapshot>>(),
    clearProviderAccountCookies: vi.fn<(accountId: string) => Promise<WorkspaceSnapshot>>(),
    loadProviderAccountCookies: vi.fn<(accountId: string) => Promise<ProviderAccountCookie[]>>().mockResolvedValue([]),
    saveProviderAccountCookies: vi.fn<(accountId: string, cookies: ProviderAccountCookie[]) => Promise<WorkspaceSnapshot>>(),
    importProviderAccountCookies: vi.fn(),
    loadProviderAccountEditor: vi.fn<(accountId: string) => Promise<ProviderAccountEditor>>().mockResolvedValue(
      createEditor(fallbackAccount),
    ),
    saveProviderAccountSettings: vi.fn<(accountId: string, values: ProviderAccountSettingValue[]) => Promise<ProviderAccountEditor>>().mockResolvedValue(
      createEditor(fallbackAccount),
    ),
    upsertAppSetting: vi.fn(),
    validateProviderAccount: vi.fn<(id: string) => Promise<WorkspaceSnapshot>>(),
    revertProviderAccountImport: vi.fn<(accountId: string) => Promise<WorkspaceSnapshot>>(),
    ...storeOverrides,
  }

  useAppStoreMock.mockImplementation((selector: (state: typeof store) => unknown) => selector(store))
  return { store, ...render(<AccountsPage {...pageProps} />) }
}

describe('AccountsPage', () => {
  beforeEach(() => {
    useAppStoreMock.mockReset()
  })

  afterEach(() => {
    cleanup()
  })

  it('renders the unified cookie-based configuration surface without raw session import controls', async () => {
    const account = createAccount()
    const editor = createEditor(account, [
      {
        settingKey: 'instagram.account.mediaPath',
        valueKind: 'string',
        stringValue: 'D:/Media/Instagram/Main',
        jsonValue: undefined,
      },
      {
        settingKey: 'instagram.auth.appId',
        valueKind: 'string',
        stringValue: '936619743392459',
        jsonValue: undefined,
      },
    ])

    renderPage(
      {
        accounts: [account],
        accountSessions: [createSession()],
      },
      { initialAccountId: account.id },
      {
        loadProviderAccountEditor: vi.fn<(accountId: string) => Promise<ProviderAccountEditor>>().mockResolvedValue(editor),
      },
    )

    expect(await screen.findByDisplayValue('Instagram Main')).toBeTruthy()
    expect(screen.queryByRole('heading', { name: /account configuration/i })).toBeNull()
    expect(screen.getByText(/1 cookies/i)).toBeTruthy()
    expect(screen.getByRole('button', { name: /^edit cookies$/i })).toBeTruthy()
    expect(await screen.findByDisplayValue('D:/Media/Instagram/Main')).toBeTruthy()
    expect(screen.queryByRole('button', { name: /^run saved posts$/i })).toBeNull()
    fireEvent.click(screen.getByRole('tab', { name: /^provider$/i }))
    expect(screen.getAllByText(/^authorization$/i).length).toBeGreaterThan(0)
    expect(screen.getByRole('textbox', { name: /^x-csrftoken/i })).toBeTruthy()
    expect(screen.getByRole('textbox', { name: /^x-ig-app-id/i })).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /show advanced fields/i }))
    expect(screen.getByRole('textbox', { name: /^user agent/i })).toBeTruthy()
    expect(screen.queryByRole('button', { name: /^import session$/i })).toBeNull()
    expect(screen.queryByRole('button', { name: /^login$/i })).toBeNull()
    expect(screen.queryByLabelText(/session payload/i)).toBeNull()
  })

  it('saves edited account data and provider settings through the same unified form', async () => {
    const account = createAccount()
    const loadedEditor = createEditor(account, [
      {
        settingKey: 'instagram.account.mediaPath',
        valueKind: 'string',
        stringValue: 'D:/Media/Instagram/Main',
        jsonValue: undefined,
      },
    ])
    const savedSnapshot: WorkspaceSnapshot = {
      ...createEmptyWorkspaceSnapshot(),
      accounts: [
        createAccount({
          displayName: 'Instagram Primary',
        }),
      ],
    }
    const savedEditor = createEditor(savedSnapshot.accounts[0], [
      {
        settingKey: 'instagram.account.mediaPath',
        valueKind: 'string',
        stringValue: 'D:/Media/Instagram/Primary',
        jsonValue: undefined,
      },
    ])

    const upsertProviderAccount = vi.fn<(draft: ProviderAccountUpsert) => Promise<WorkspaceSnapshot>>().mockResolvedValue(savedSnapshot)
    const saveProviderAccountSettings = vi
      .fn<(accountId: string, values: ProviderAccountSettingValue[]) => Promise<ProviderAccountEditor>>()
      .mockResolvedValue(savedEditor)

    renderPage(
      { accounts: [account] },
      { initialAccountId: account.id },
      {
        upsertProviderAccount,
        loadProviderAccountEditor: vi.fn<(accountId: string) => Promise<ProviderAccountEditor>>().mockResolvedValue(loadedEditor),
        saveProviderAccountSettings,
      },
    )

    fireEvent.change(await screen.findByLabelText(/display name/i), {
      target: { value: 'Instagram Primary' },
    })
    fireEvent.change(await screen.findByLabelText(/media path/i), {
      target: { value: 'D:/Media/Instagram/Primary' },
    })
    fireEvent.click(screen.getByRole('button', { name: /^save changes$/i }))

    await waitFor(() => {
      expect(upsertProviderAccount).toHaveBeenCalledWith(
        expect.objectContaining({
          id: 'account-1',
          displayName: 'Instagram Primary',
        }),
      )
    })

    await waitFor(() => {
      expect(saveProviderAccountSettings).toHaveBeenCalledWith(
        'account-1',
        expect.arrayContaining([
          expect.objectContaining({
            settingKey: 'instagram.account.mediaPath',
            stringValue: 'D:/Media/Instagram/Primary',
          }),
        ]),
      )
    })
  })

  it('opens provider-scoped create mode when settings are launched for a provider with no accounts', () => {
    const { store } = renderPage(
      {},
      { initialProvider: 'twitter', initialMode: 'create' },
    )

    expect(screen.getByText(/new account draft/i)).toBeTruthy()
    expect(screen.getAllByText(/^X \/ Twitter$/i).length).toBeGreaterThan(0)
    expect(screen.getByText(/add cookies now or save the account first/i)).toBeTruthy()
    expect(screen.getByRole('button', { name: /^edit cookies$/i })).toBeTruthy()
    fireEvent.click(screen.getByRole('tab', { name: /^provider$/i }))
    expect(screen.getByText('Use user agent')).toBeTruthy()
    expect(store.loadProviderAccountEditor).not.toHaveBeenCalled()
  })

  it('uses fixed tabs and exposes instagram account defaults separately from provider runtime', async () => {
    const account = createAccount()
    const editor = createEditor(account, [
      {
        settingKey: 'instagram.defaults.downloadText',
        valueKind: 'string',
        stringValue: 'true',
        jsonValue: undefined,
      },
      {
        settingKey: 'instagram.defaults.extractImageFromVideo.reels',
        valueKind: 'string',
        stringValue: 'false',
        jsonValue: undefined,
      },
      {
        settingKey: 'instagram.defaults.placeExtractedImageIntoVideoFolder',
        valueKind: 'string',
        stringValue: 'true',
        jsonValue: undefined,
      },
    ])

    renderPage(
      {
        accounts: [account],
      },
      { initialAccountId: account.id },
      {
        loadProviderAccountEditor: vi.fn<(accountId: string) => Promise<ProviderAccountEditor>>().mockResolvedValue(editor),
      },
    )

    expect(await screen.findByRole('tab', { name: /^account$/i })).toBeTruthy()
    expect(screen.getByRole('tab', { name: /^defaults$/i })).toBeTruthy()
    expect(screen.getByRole('tab', { name: /^provider$/i })).toBeTruthy()
    expect(screen.queryByRole('tab', { name: /^history$/i })).toBeNull()
    expect(screen.queryByRole('button', { name: /^show advanced$/i })).toBeNull()
    expect(screen.queryByText(/identity, session state, validation, and saved-post actions/i)).toBeNull()

    fireEvent.click(screen.getByRole('tab', { name: /^defaults$/i }))

    expect(await screen.findByLabelText(/^download text$/i)).toBeTruthy()
    expect((screen.getByLabelText(/^download text$/i) as HTMLInputElement).checked).toBe(true)
    expect((screen.getByLabelText(/^from reels$/i) as HTMLInputElement).checked).toBe(false)
    expect((screen.getByLabelText(/^place extracted image into the video folder$/i) as HTMLInputElement).checked).toBe(true)
  })

  it('allows cookies to be prepared before the account is created', async () => {
    const createdAccount = createAccount({
      id: 'account-9',
      displayName: 'Instagram Draft',
    })
    const upsertProviderAccount = vi
      .fn<(draft: ProviderAccountUpsert) => Promise<WorkspaceSnapshot>>()
      .mockResolvedValue({
        ...createEmptyWorkspaceSnapshot(),
        accounts: [createdAccount],
      })
    const saveProviderAccountCookies = vi
      .fn<(accountId: string, cookies: ProviderAccountCookie[]) => Promise<WorkspaceSnapshot>>()
      .mockResolvedValue({
        ...createEmptyWorkspaceSnapshot(),
        accounts: [createdAccount],
        accountSessions: [createSession({ accountId: 'account-9' })],
      })

    renderPage(
      {},
      { initialProvider: 'instagram', initialMode: 'create' },
      {
        upsertProviderAccount,
        saveProviderAccountCookies,
      },
    )

    fireEvent.click(screen.getByRole('button', { name: /^edit cookies$/i }))
    expect(await screen.findByRole('dialog', { name: /instagram cookies/i })).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: /^import cookies$/i }))
    fireEvent.click(screen.getByRole('button', { name: /^paste json$/i }))
    fireEvent.change(screen.getByLabelText(/^cookie text$/i), {
      target: {
        value: '[{"domain":".instagram.com","name":"sessionid","value":"abc123","path":"/","secure":true,"httpOnly":true}]',
      },
    })
    fireEvent.click(screen.getByRole('button', { name: /^ok$/i }))
    fireEvent.click(screen.getByRole('button', { name: /^save cookies$/i }))

    expect(await screen.findByText(/1 draft cookies ready to save with the new account/i)).toBeTruthy()

    fireEvent.change(screen.getByLabelText(/display name/i), {
      target: { value: 'Instagram Draft' },
    })
    fireEvent.click(screen.getAllByRole('button', { name: /^create account$/i })[0])

    await waitFor(() => {
      expect(upsertProviderAccount).toHaveBeenCalledWith(
        expect.objectContaining({
          displayName: 'Instagram Draft',
          provider: 'instagram',
        }),
      )
    })

    await waitFor(() => {
      expect(saveProviderAccountCookies).toHaveBeenCalledWith(
        'account-9',
        expect.arrayContaining([
          expect.objectContaining({
            domain: '.instagram.com',
            name: 'sessionid',
          }),
        ]),
      )
    })
  })

  it('opens the cookie editor and loads stored cookies for the selected account', async () => {
    const account = createAccount()
    const loadProviderAccountCookies = vi
      .fn<(accountId: string) => Promise<ProviderAccountCookie[]>>()
      .mockResolvedValue([createCookie()])

    renderPage(
      {
        accounts: [account],
        accountSessions: [createSession()],
      },
      { initialAccountId: account.id },
      {
        loadProviderAccountCookies,
      },
    )

    fireEvent.click(await screen.findByRole('button', { name: /^edit cookies$/i }))

    await waitFor(() => {
      expect(loadProviderAccountCookies).toHaveBeenCalledWith('account-1')
    })

    expect(await screen.findByRole('dialog', { name: /instagram cookies/i })).toBeTruthy()
    expect(screen.getByText(/sessionid @ \.instagram\.com/i)).toBeTruthy()
  })

  it('imports pasted cookie content through the cookie prompt instead of a file picker', async () => {
    const account = createAccount()
    const loadProviderAccountCookies = vi
      .fn<(accountId: string) => Promise<ProviderAccountCookie[]>>()
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([createCookie()])
    const importProviderAccountCookies = vi.fn().mockResolvedValue({
      ...createEmptyWorkspaceSnapshot(),
      accounts: [account],
      accountSessions: [createSession()],
    })

    renderPage(
      {
        accounts: [account],
        accountSessions: [createSession()],
      },
      { initialAccountId: account.id },
      {
        loadProviderAccountCookies,
        importProviderAccountCookies,
      },
    )

    fireEvent.click(await screen.findByRole('button', { name: /^edit cookies$/i }))
    expect(await screen.findByRole('dialog', { name: /instagram cookies/i })).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: /^import cookies$/i }))
    fireEvent.click(screen.getByRole('button', { name: /^paste json$/i }))

    expect(await screen.findByRole('dialog', { name: /^import cookies$/i })).toBeTruthy()

    fireEvent.change(screen.getByLabelText(/^cookie text$/i), {
      target: {
        value: '[{"domain":".instagram.com","name":"sessionid","value":"abc123","path":"/","secure":true,"httpOnly":true}]',
      },
    })
    fireEvent.click(screen.getByRole('button', { name: /^ok$/i }))

    await waitFor(() => {
      expect(importProviderAccountCookies).toHaveBeenCalledWith({
        accountId: 'account-1',
        importFormat: 'json',
        content: '[{"domain":".instagram.com","name":"sessionid","value":"abc123","path":"/","secure":true,"httpOnly":true}]',
      })
    })
  })

  it('clears stored cookies through the store mutation', async () => {
    const account = createAccount()
    const clearedSnapshot: WorkspaceSnapshot = {
      ...createEmptyWorkspaceSnapshot(),
      accounts: [
        createAccount({
          authState: 'expired',
        }),
      ],
      accountSessions: [
        createSession({
          cookieCount: 0,
          hasSecret: false,
          lastValidationError: 'No session is stored for this provider account.',
        }),
      ],
    }
    const clearProviderAccountCookies = vi
      .fn<(accountId: string) => Promise<WorkspaceSnapshot>>()
      .mockResolvedValue(clearedSnapshot)

    renderPage(
      {
        accounts: [account],
        accountSessions: [createSession()],
      },
      { initialAccountId: account.id },
      {
        clearProviderAccountCookies,
      },
    )

    const confirm = vi.spyOn(globalThis, 'confirm').mockReturnValue(true)
    fireEvent.click(await screen.findByRole('button', { name: /^clear cookies$/i }))

    await waitFor(() => {
      expect(clearProviderAccountCookies).toHaveBeenCalledWith('account-1')
    })
    confirm.mockRestore()
  })

  it('keeps account actions in the footer without saved-post or history controls', async () => {
    const account = createAccount()

    renderPage(
      {
        accounts: [account],
        accountSessions: [createSession()],
      },
      { initialAccountId: account.id },
    )

    expect(await screen.findByRole('button', { name: /^validate account$/i })).toBeTruthy()
    expect(screen.queryByRole('button', { name: /^run saved posts$/i })).toBeNull()
    expect(screen.queryByRole('tab', { name: /^history$/i })).toBeNull()
  })

  it('offers the single Companion backup and reverts it after confirmation', async () => {
    const account = createAccount()
    const editor: ProviderAccountEditor = {
      ...createEditor(account),
      importState: {
        accountId: account.id,
        providerUserId: '42',
        providerUsername: 'ninja',
        lastImportedAt: '2026-06-30T10:00:00Z',
        canRevert: true,
        backupImportedAt: '2026-06-29T10:00:00Z',
      },
    }
    const revertedSnapshot: WorkspaceSnapshot = {
      ...createEmptyWorkspaceSnapshot(),
      accounts: [account],
      accountSessions: [createSession()],
    }
    const revertProviderAccountImport = vi
      .fn<(accountId: string) => Promise<WorkspaceSnapshot>>()
      .mockResolvedValue(revertedSnapshot)
    const confirm = vi.spyOn(globalThis, 'confirm').mockReturnValue(true)

    renderPage(
      { accounts: [account], accountSessions: [createSession()] },
      { initialAccountId: account.id },
      {
        loadProviderAccountEditor: vi.fn().mockResolvedValue(editor),
        revertProviderAccountImport,
      },
    )

    fireEvent.click(await screen.findByRole('button', { name: /^revert last import$/i }))
    await waitFor(() => {
      expect(revertProviderAccountImport).toHaveBeenCalledWith(account.id)
    })
    confirm.mockRestore()
  })
})
