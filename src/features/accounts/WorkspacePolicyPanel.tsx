import { SettingTextField, SettingToggle } from '../settings/appSettingControls'

/**
 * Global account/session/sync policy — lives on Accounts (domain home),
 * not in Preferences.
 */
export function WorkspacePolicyPanel() {
  return (
    <section className="accounts-section workspace-policy-panel" aria-label="Workspace policy">
      <header className="accounts-section-header">
        <div>
          <p className="eyebrow">Workspace</p>
          <h3>Session &amp; sync policy</h3>
        </div>
      </header>
      <p className="accounts-section-copy muted-text">
        Defaults for session import and the sync queue. Per-account timers still win when set.
      </p>
      <div className="workspace-policy-list">
        <SettingToggle
          settingKey="policy.session_import.enabled"
          label="Manual session import"
          hint="Show session import actions in the UI when enabled."
        />
        <SettingToggle
          settingKey="policy.sync.blockDuplicateUserId"
          label="Block duplicate user IDs"
          hint="On a profile's first sync, cancel and remove the new profile if its user id already belongs to another source."
        />
        <SettingTextField
          settingKey="policy.sync.delayBetweenProfilesSecs"
          label="Global delay between profiles (seconds)"
          hint="Fallback when an account does not set its own delay. 0 disables."
        />
        <SettingTextField
          settingKey="policy.feed.archived_session_retention_limit"
          label="Archived session retention"
          hint="How many archived session records to keep before pruning."
        />
      </div>
    </section>
  )
}
