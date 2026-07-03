// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type {
  ProviderAccount,
  ProviderAccountEditor,
  ProviderAccountSettingValue,
  SourceProfile,
  WorkspaceSnapshot,
} from '../../domain/models'
import { createEmptyWorkspaceSnapshot } from '../../domain/workspaceSnapshot'
import { SourceEditorDialog } from './SourceEditorDialog'

const useAppStoreMock = vi.fn()

vi.mock('../../state/appStore', () => ({
  useAppStore: (selector: (state: Record<string, unknown>) => unknown) => useAppStoreMock(selector),
}))

vi.mock('../../bridge/desktop', () => ({
  loadSystemShortDatePattern: vi.fn(async () => 'dd/MM/yyyy'),
}))

function buildAccount(overrides: Partial<ProviderAccount> = {}): ProviderAccount {
  return {
    id: 'account-1',
    provider: 'instagram',
    displayName: 'Instagram Main',
    authMode: 'imported_session',
    authState: 'ready',
    capabilities: ['posts'],
    lastValidatedAt: '2026-03-10T00:00:00Z',
    ...overrides,
  }
}

function buildEditor(account: ProviderAccount, settings: ProviderAccountSettingValue[] = []): ProviderAccountEditor {
  return {
    account,
    session: null,
    settings,
  }
}

function buildSource(overrides: Partial<SourceProfile> = {}): SourceProfile {
  return {
    id: 'source-1',
    provider: 'instagram',
    sourceKind: 'profile',
    handle: '@alpha',
    displayName: 'alpha',
    accountId: 'account-1',
    labels: ['reference'],
    readyForDownload: true,
    remoteState: 'exists',
    isSubscription: false,
    profileImageCustom: false,
    importerId: undefined,
    importedAt: undefined,
    ...overrides,
  }
}

function buildSnapshot(snapshotOverrides: Partial<WorkspaceSnapshot> = {}): WorkspaceSnapshot {
  const snapshot = createEmptyWorkspaceSnapshot()
  const accounts = snapshotOverrides.accounts ?? [buildAccount()]

  return {
    ...snapshot,
    ...snapshotOverrides,
    accounts,
  }
}

function renderDialog(
  snapshotOverrides: Partial<WorkspaceSnapshot> = {},
  options: {
    editorSettings?: ProviderAccountSettingValue[]
    preferredAccountId?: string
    preferredProvider?: SourceProfile['provider']
    source?: SourceProfile
  } = {},
) {
  const snapshot = buildSnapshot(snapshotOverrides)
  const initialAccount = snapshot.accounts.find((account) => account.id === 'account-1') ?? snapshot.accounts[0]

  const store = {
    pendingCommand: undefined,
    loadProviderAccountEditor: vi.fn<(accountId: string) => Promise<ProviderAccountEditor>>(async (accountId: string) =>
      buildEditor(
        snapshot.accounts.find((account) => account.id === accountId)!,
        accountId === initialAccount?.id ? (options.editorSettings ?? []) : [],
      ),
    ),
    runSourceSync: vi.fn(),
    upsertSourceProfile: vi.fn(),
  }

  useAppStoreMock.mockImplementation((selector: (state: typeof store) => unknown) => selector(store))

  const onAdvancedAccountSettings = vi.fn()
  const onClose = vi.fn()
  const onEditAccount = vi.fn()
  const onSaved = vi.fn()

  render(
    <SourceEditorDialog
      onAdvancedAccountSettings={onAdvancedAccountSettings}
      onClose={onClose}
      onEditAccount={onEditAccount}
      onSaved={onSaved}
      preferredAccountId={options.preferredAccountId ?? 'account-1'}
      preferredProvider={options.preferredProvider}
      snapshot={snapshot}
      source={options.source}
    />,
  )

  return { onAdvancedAccountSettings, onClose, onEditAccount, onSaved, snapshot, store }
}

describe('SourceEditorDialog', () => {
  beforeEach(() => {
    useAppStoreMock.mockReset()
  })

  afterEach(() => {
    cleanup()
  })

  it('shows a single edit-account action in the header context', () => {
    const { onAdvancedAccountSettings, onEditAccount } = renderDialog()

    fireEvent.click(screen.getByRole('button', { name: /edit account/i }))

    expect(onEditAccount).toHaveBeenCalledWith('account-1')
    expect(onAdvancedAccountSettings).not.toHaveBeenCalled()
    expect(screen.queryByRole('button', { name: /options/i })).toBeNull()
  })

  it('locks provider but keeps account editable in edit mode', () => {
    renderDialog(
      {
        accounts: [
          buildAccount(),
          buildAccount({
            id: 'account-2',
            displayName: 'Instagram Backup',
          }),
        ],
      },
      {
        source: buildSource(),
      },
    )

    expect(screen.getByText('Locked while editing an existing profile.')).toBeTruthy()
    expect(screen.queryByRole('combobox', { name: /provider/i })).toBeNull()

    const accountSelect = screen.getByRole('combobox', { name: /account/i }) as HTMLSelectElement
    expect(accountSelect.value).toBe('account-1')

    fireEvent.change(accountSelect, { target: { value: 'account-2' } })

    expect(accountSelect.value).toBe('account-2')
    expect(screen.getByRole('button', { name: /edit account/i })).toBeTruthy()
  })

  it('locks the user url and hides accent color in edit mode', () => {
    renderDialog(
      {},
      {
        source: buildSource(),
      },
    )

    expect(screen.queryByRole('textbox', { name: /user url/i })).toBeNull()
    expect(screen.getByText('@alpha')).toBeTruthy()
    expect(screen.getByText(/User URL is locked for existing profiles/)).toBeTruthy()
    expect(screen.queryByLabelText(/accent color/i)).toBeNull()
  })

  it.each(['instagram', 'tiktok', 'twitter'] as const)(
    'unlocks the %s user url for manual editing via the Edit button',
    (provider) => {
      renderDialog(
        {
          accounts: [
            buildAccount({
              provider,
              displayName: `${provider} account`,
            }),
          ],
        },
        {
          source: buildSource({ provider }),
        },
      )

      // Locked by default: value shown, no input.
      expect(screen.queryByRole('textbox', { name: /user url/i })).toBeNull()

      fireEvent.click(screen.getByRole('button', { name: 'Edit handle' }))

      // Now editable: an input with the current handle that accepts a new value.
      const input = screen.getByRole('textbox', { name: /user url/i })
      expect((input as HTMLInputElement).value).toBe('@alpha')
      fireEvent.change(input, { target: { value: '@renamed' } })
      expect((input as HTMLInputElement).value).toBe('@renamed')
      expect(screen.getByText(/Manual override/)).toBeTruthy()
    },
  )

  it('uses dedicated layout panels for profile, sync, and history tabs', () => {
    renderDialog(
      {},
      {
        source: buildSource(),
      },
    )

    expect(document.getElementById('source-editor-tab-profile')?.className).toContain('source-editor-tab-panel-profile')

    fireEvent.click(screen.getByRole('tab', { name: /sync/i }))
    expect(document.getElementById('source-editor-tab-sync')?.className).toContain('source-editor-tab-panel-sync')

    fireEvent.click(screen.getByRole('tab', { name: /history/i }))
    expect(document.getElementById('source-editor-tab-history')?.className).toContain('source-editor-tab-panel-history')
  })

  it('shows legacy import metadata and runs force backfill for imported profiles', async () => {
    const { store } = renderDialog(
      {},
      {
        source: buildSource({
          importerId: 'instagram.scrawler',
          importedAt: '2026-03-20T12:00:00Z',
        }),
      },
    )
    store.runSourceSync.mockResolvedValue(buildSnapshot())

    fireEvent.click(screen.getByRole('tab', { name: /sync/i }))

    expect(screen.getByText('Legacy Import')).toBeTruthy()
    expect(screen.getByText('2026-03-20T12:00:00Z')).toBeTruthy()
    expect(screen.getByText('instagram.scrawler')).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: /force legacy backfill/i }))

    await waitFor(() =>
      expect(store.runSourceSync).toHaveBeenCalledWith('source-1', {
        trigger: 'manual_force_imported_backfill',
        runMode: 'force_imported_backfill',
      }),
    )
  })

  it('applies account defaults when creating a profile from a bound account', async () => {
    renderDialog(
      {},
      {
        editorSettings: [
          {
            settingKey: 'instagram.defaults.labels',
            valueKind: 'string',
            stringValue: 'priority, reference',
          },
          {
            settingKey: 'instagram.defaults.readyForDownload',
            valueKind: 'string',
            stringValue: 'false',
          },
        ],
      },
    )

    expect(await screen.findByRole('button', { name: /remove label priority/i })).toBeTruthy()
    expect(screen.getByRole('button', { name: /remove label reference/i })).toBeTruthy()
    expect((screen.getByRole('checkbox', { name: /ready for download/i }) as HTMLInputElement).checked).toBe(false)

    fireEvent.click(screen.getByRole('tab', { name: /sync/i }))

    expect((screen.getByRole('checkbox', { name: /^timeline/i }) as HTMLInputElement).checked).toBe(true)
    expect((screen.getByRole('checkbox', { name: /^reels/i }) as HTMLInputElement).checked).toBe(false)
    expect((screen.getByRole('checkbox', { name: /^stories$/i }) as HTMLInputElement).checked).toBe(false)
  })

  it('applies instagram section defaults from account settings', async () => {
    renderDialog(
      {},
      {
        editorSettings: [
          {
            settingKey: 'instagram.defaults.downloadTimeline',
            valueKind: 'string',
            stringValue: 'false',
          },
          {
            settingKey: 'instagram.defaults.downloadReels',
            valueKind: 'string',
            stringValue: 'true',
          },
          {
            settingKey: 'instagram.defaults.downloadStoriesUser',
            valueKind: 'string',
            stringValue: 'true',
          },
          {
            settingKey: 'instagram.defaults.downloadTaggedPosts',
            valueKind: 'string',
            stringValue: 'true',
          },
        ],
      },
    )

    await screen.findByText(/defaults loaded from/i)
    fireEvent.click(screen.getByRole('tab', { name: /sync/i }))

    expect((screen.getByRole('checkbox', { name: /^timeline/i }) as HTMLInputElement).checked).toBe(false)
    expect((screen.getByRole('checkbox', { name: /^reels/i }) as HTMLInputElement).checked).toBe(true)
    expect((screen.getByRole('checkbox', { name: /^stories \(user\)/i }) as HTMLInputElement).checked).toBe(true)
    expect((screen.getByRole('checkbox', { name: /^tagged/i }) as HTMLInputElement).checked).toBe(true)
  })

  it('applies text and extract-image defaults from account settings for new profiles', async () => {
    renderDialog(
      {},
      {
        editorSettings: [
          {
            settingKey: 'instagram.defaults.downloadText',
            valueKind: 'string',
            stringValue: 'true',
          },
          {
            settingKey: 'instagram.defaults.downloadTextPosts',
            valueKind: 'string',
            stringValue: 'true',
          },
          {
            settingKey: 'instagram.defaults.textSpecialFolder',
            valueKind: 'string',
            stringValue: 'false',
          },
          {
            settingKey: 'instagram.defaults.extractImageFromVideo.timeline',
            valueKind: 'string',
            stringValue: 'false',
          },
          {
            settingKey: 'instagram.defaults.extractImageFromVideo.reels',
            valueKind: 'string',
            stringValue: 'false',
          },
          {
            settingKey: 'instagram.defaults.extractImageFromVideo.stories',
            valueKind: 'string',
            stringValue: 'false',
          },
          {
            settingKey: 'instagram.defaults.extractImageFromVideo.storiesUser',
            valueKind: 'string',
            stringValue: 'false',
          },
          {
            settingKey: 'instagram.defaults.extractImageFromVideo.tagged',
            valueKind: 'string',
            stringValue: 'false',
          },
          {
            settingKey: 'instagram.defaults.placeExtractedImageIntoVideoFolder',
            valueKind: 'string',
            stringValue: 'true',
          },
        ],
      },
    )

    await screen.findByText(/defaults loaded from/i)
    fireEvent.click(screen.getByRole('tab', { name: /sync/i }))

    expect((screen.getAllByRole('checkbox', { name: /download text/i })[0] as HTMLInputElement).checked).toBe(true)
    expect((screen.getByRole('checkbox', { name: /download text posts/i }) as HTMLInputElement).checked).toBe(true)
    expect((screen.getByRole('checkbox', { name: /text special folder/i }) as HTMLInputElement).checked).toBe(false)
    expect((screen.getByRole('checkbox', { name: /extract timeline/i }) as HTMLInputElement).checked).toBe(false)
    expect((screen.getByRole('checkbox', { name: /extract reels/i }) as HTMLInputElement).checked).toBe(false)
    expect((screen.getByRole('checkbox', { name: /^extract stories$/i }) as HTMLInputElement).checked).toBe(false)
    expect((screen.getByRole('checkbox', { name: /extract stories \(user\)/i }) as HTMLInputElement).checked).toBe(false)
    expect((screen.getByRole('checkbox', { name: /extract tagged/i }) as HTMLInputElement).checked).toBe(false)
    expect((screen.getByRole('checkbox', { name: /place extracted image in video folder/i }) as HTMLInputElement).checked).toBe(true)
  })

  it('reports dirty state and shows footer indicator', () => {
    const onDirtyChange = vi.fn()
    const snapshot = buildSnapshot()
    const store = {
      pendingCommand: undefined,
      loadProviderAccountEditor: vi.fn(async (accountId: string) =>
        buildEditor(snapshot.accounts.find((account) => account.id === accountId)!),
      ),
      upsertSourceProfile: vi.fn(),
    }

    useAppStoreMock.mockImplementation((selector: (state: typeof store) => unknown) => selector(store))

    render(
      <SourceEditorDialog
        onClose={vi.fn()}
        onDirtyChange={onDirtyChange}
        onSaved={vi.fn()}
        preferredAccountId="account-1"
        snapshot={snapshot}
      />,
    )

    expect(screen.queryByText('No pending changes')).toBeNull()
    expect(screen.queryByText('Unsaved changes')).toBeNull()
    fireEvent.change(screen.getByLabelText(/friendly name/i), { target: { value: 'edited' } })
    expect(screen.getByText('Unsaved changes')).toBeTruthy()
    expect(onDirtyChange).toHaveBeenLastCalledWith(true)
  })

  it('saves an existing profile and closes after a successful submit', async () => {
    const source = buildSource()
    const savedSource = { ...source, displayName: 'edited-name' }
    const { onClose, onSaved, store } = renderDialog(
      {
        sources: [savedSource],
      },
      {
        source,
      },
    )
    store.upsertSourceProfile.mockResolvedValue(
      buildSnapshot({
        sources: [savedSource],
      }),
    )

    fireEvent.change(screen.getByLabelText(/friendly name/i), { target: { value: 'edited-name' } })
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }))

    await waitFor(() => {
      expect(store.upsertSourceProfile).toHaveBeenCalledWith(
        expect.objectContaining({
          id: source.id,
          accountId: source.accountId,
          displayName: 'edited-name',
          handle: source.handle,
        }),
      )
    })
    await waitFor(() => {
      expect(onSaved).toHaveBeenCalledWith(savedSource)
      expect(onClose).toHaveBeenCalledTimes(1)
    })
  })

  it('saves and starts a manual sync for a new profile', async () => {
    const savedSource = buildSource({
      id: 'source-new',
      handle: '@new-profile',
      displayName: 'new-profile',
    })
    const { onClose, onSaved, store } = renderDialog()
    store.upsertSourceProfile.mockResolvedValue(
      buildSnapshot({
        sources: [savedSource],
      }),
    )
    store.runSourceSync.mockResolvedValue(
      buildSnapshot({
        sources: [savedSource],
      }),
    )

    fireEvent.change(screen.getByRole('textbox', { name: /user url/i }), {
      target: { value: '@new-profile' },
    })
    fireEvent.click(screen.getByRole('button', { name: /save and sync/i }))

    await waitFor(() => {
      expect(store.runSourceSync).toHaveBeenCalledWith('source-new', { trigger: 'manual' })
      expect(onSaved).toHaveBeenCalledWith(savedSource)
      expect(onClose).toHaveBeenCalledTimes(1)
    })
  })

  it('closes without submitting when cancel is clicked', () => {
    const { onClose, store } = renderDialog(
      {},
      {
        source: buildSource(),
      },
    )

    fireEvent.click(screen.getByRole('button', { name: /cancel/i }))

    expect(onClose).toHaveBeenCalledTimes(1)
    expect(store.upsertSourceProfile).not.toHaveBeenCalled()
  })

  it('does not mark the form as dirty when only switching tabs', () => {
    const onDirtyChange = vi.fn()
    const snapshot = buildSnapshot()
    const store = {
      pendingCommand: undefined,
      loadProviderAccountEditor: vi.fn(async (accountId: string) =>
        buildEditor(snapshot.accounts.find((account) => account.id === accountId)!),
      ),
      upsertSourceProfile: vi.fn(),
    }

    useAppStoreMock.mockImplementation((selector: (state: typeof store) => unknown) => selector(store))

    render(
      <SourceEditorDialog
        onClose={vi.fn()}
        onDirtyChange={onDirtyChange}
        onSaved={vi.fn()}
        preferredAccountId="account-1"
        snapshot={snapshot}
      />,
    )

    fireEvent.click(screen.getByRole('tab', { name: /sync/i }))
    fireEvent.click(screen.getByRole('tab', { name: /history/i }))
    fireEvent.click(screen.getByRole('tab', { name: /profile/i }))

    expect(screen.queryByText('Unsaved changes')).toBeNull()
    expect(onDirtyChange).not.toHaveBeenCalledWith(true)
  })

  it('keeps operational actions out of the editor flow', () => {
    renderDialog(
      {},
      {
        source: buildSource(),
      },
    )

    expect(screen.queryByRole('button', { name: /run sync/i })).toBeNull()
    expect(screen.queryByRole('button', { name: /^delete$/i })).toBeNull()
  })

  it('applies dependent enabled state for special path and script fields', () => {
    renderDialog()

    fireEvent.click(screen.getByRole('tab', { name: /sync/i }))

    const specialPathInput = screen.getByLabelText(/^special path/i) as HTMLInputElement
    const textSpecialFolderToggle = screen.getByRole('checkbox', { name: /text special folder/i }) as HTMLInputElement
    const scriptInput = screen.getByLabelText(/^script/i) as HTMLInputElement
    const scriptToggle = screen.getByRole('checkbox', { name: /enable post-sync script/i }) as HTMLInputElement

    expect(textSpecialFolderToggle.checked).toBe(true)
    expect(specialPathInput.disabled).toBe(false)
    fireEvent.click(textSpecialFolderToggle)
    expect(specialPathInput.disabled).toBe(true)

    expect(scriptToggle.checked).toBe(false)
    expect(scriptInput.disabled).toBe(true)
    fireEvent.click(scriptToggle)
    expect(scriptInput.disabled).toBe(false)
  })

  it('uses the shared visual calendar pattern for sync date filters', () => {
    renderDialog(
      {},
      {
        source: buildSource({
          syncOptions: {
            instagram: {
              timeline: true,
              reels: true,
              stories: true,
              storiesUser: true,
              tagged: true,
              dateFrom: '2026-03-01',
              dateTo: '2026-03-12',
            },
          },
        }),
      },
    )

    fireEvent.click(screen.getByRole('tab', { name: /sync/i }))

    const dateFromInput = screen.getByLabelText(/^date from$/i) as HTMLInputElement
    expect(dateFromInput.type).toBe('text')
    expect(screen.getByRole('button', { name: /pick date from/i })).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: /pick date from/i }))
    expect(screen.getByRole('dialog', { name: /date from calendar/i })).toBeTruthy()
  })

  it('renders the TikTok sync editor with its download toggles', () => {
    renderDialog(
      {
        accounts: [buildAccount({ id: 'account-2', provider: 'tiktok', displayName: 'TikTok Main' })],
      },
      {
        preferredAccountId: 'account-2',
        preferredProvider: 'tiktok',
      },
    )

    fireEvent.click(screen.getByRole('tab', { name: /sync/i }))

    expect(screen.queryByText('TikTok sync editor not modeled yet')).toBeNull()
    expect(screen.getByRole('checkbox', { name: 'Download videos' })).toBeTruthy()
    expect(screen.getByRole('checkbox', { name: 'Download photos' })).toBeTruthy()
  })
})
