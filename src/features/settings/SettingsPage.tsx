import { useMemo, useState } from 'react'
import type { AppSetting } from '../../domain/models'
import { useAppStore } from '../../state/appStore'
import { getStoredTheme, toggleTheme, type Theme } from '../../theme'
import { HelpTip } from '../shared/HelpTip'

/**
 * Preferences dialog (Tools → Preferences) — cross-cutting hub only.
 *
 * Domain homes (do not re-add here):
 * - imports.* → Import window
 * - session/sync delay/duplicate/retention → Accounts → Workspace
 * - plan notification default → Plans editor
 * - storage.media_root → About
 * - tool/runtime/instagram.sync internals → hidden
 */

type PreferenceSectionId = 'appearance' | 'desktop' | 'media'

interface PreferenceSection {
  id: PreferenceSectionId
  title: string
  description: string
}

const SECTIONS: PreferenceSection[] = [
  {
    id: 'appearance',
    title: 'Appearance',
    description: 'Theme for the desktop UI.',
  },
  {
    id: 'desktop',
    title: 'Desktop',
    description: 'Window and background behaviour on this machine.',
  },
  {
    id: 'media',
    title: 'Media naming',
    description: 'Instagram file naming defaults for downloads.',
  },
]

/** Keys with domain homes or internal — never list in Preferences. */
function shouldHideSetting(key: string): boolean {
  if (key.startsWith('tool.') && key.endsWith('.path')) return true
  if (key.startsWith('instagram.sync.')) return true
  if (key.startsWith('runtime.')) return true
  if (key.startsWith('imports.')) return true
  if (key === 'storage.media_root') return true
  if (key.startsWith('policy.session_')) return true
  if (key.startsWith('policy.sync.')) return true
  if (key.startsWith('policy.feed.')) return true
  if (key.startsWith('policy.notifications.')) return true
  return false
}

function sectionForKey(key: string): PreferenceSectionId | null {
  if (shouldHideSetting(key)) return null
  if (key.startsWith('policy.desktop.')) return 'desktop'
  if (key.startsWith('naming.')) return 'media'
  return null
}

const SETTING_LABELS: Record<string, string> = {
  'policy.desktop.close_to_tray': 'Close to tray',
  'policy.desktop.silent_mode': 'Silent mode',
  'naming.instagram.media_file_pattern_mode': 'Instagram file naming',
  'naming.instagram.media_file_pattern_template': 'Custom Instagram name template',
}

const ENUM_OPTIONS: Record<string, Array<{ value: string; label: string }>> = {
  'naming.instagram.media_file_pattern_mode': [
    { value: 'preset_new_default', label: 'New default' },
    { value: 'preset_legacy_url_basename', label: 'Legacy URL basename' },
    { value: 'custom', label: 'Custom template' },
  ],
}

function humanizeKey(key: string): string {
  if (SETTING_LABELS[key]) return SETTING_LABELS[key]
  const tail = key.split('.').pop() ?? key
  return tail
    .replace(/[_-]+/g, ' ')
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .replace(/\b\w/g, (char) => char.toUpperCase())
}

function isBooleanSetting(value: string): boolean {
  return value === 'true' || value === 'false'
}

export function SettingsPage() {
  const snapshot = useAppStore((state) => state.snapshot)
  const pendingCommand = useAppStore((state) => state.pendingCommand)
  const upsertAppSetting = useAppStore((state) => state.upsertAppSetting)
  const [draftValues, setDraftValues] = useState<Record<string, string>>({})
  const [theme, setThemeState] = useState<Theme>(getStoredTheme)
  const [activeSection, setActiveSection] = useState<PreferenceSectionId>('appearance')

  const settingsBySection = useMemo(() => {
    const grouped: Record<PreferenceSectionId, AppSetting[]> = {
      appearance: [],
      desktop: [],
      media: [],
    }

    for (const setting of snapshot?.appSettings ?? []) {
      const section = sectionForKey(setting.key)
      if (!section) continue
      grouped[section].push(setting)
    }

    for (const id of Object.keys(grouped) as PreferenceSectionId[]) {
      grouped[id].sort((left, right) => humanizeKey(left.key).localeCompare(humanizeKey(right.key)))
    }

    return grouped
  }, [snapshot])

  const visibleSections = useMemo(
    () =>
      SECTIONS.filter(
        (section) => section.id === 'appearance' || settingsBySection[section.id].length > 0,
      ),
    [settingsBySection],
  )

  const resolvedSection =
    visibleSections.find((section) => section.id === activeSection)?.id ??
    visibleSections[0]?.id ??
    'appearance'

  async function handleSave(key: string, explicitValue?: string) {
    const setting = snapshot?.appSettings.find((entry) => entry.key === key)
    if (!setting) return

    await upsertAppSetting({
      key,
      value: explicitValue ?? draftValues[key] ?? setting.value,
      category: setting.category,
      description: setting.description,
      mutable: setting.mutable,
    })
  }

  if (!snapshot) {
    return null
  }

  const sectionMeta = SECTIONS.find((section) => section.id === resolvedSection) ?? SECTIONS[0]
  const sectionSettings = settingsBySection[resolvedSection]

  return (
    <div className="preferences-shell">
      <nav className="preferences-nav" aria-label="Preference sections">
        {visibleSections.map((section) => {
          const isActive = section.id === resolvedSection
          return (
            <button
              key={section.id}
              type="button"
              className={isActive ? 'preferences-nav-item is-active' : 'preferences-nav-item'}
              aria-current={isActive ? 'page' : undefined}
              onClick={() => setActiveSection(section.id)}
            >
              {section.title}
            </button>
          )
        })}
      </nav>

      <section className="preferences-panel" aria-labelledby="preferences-section-title">
        <header className="preferences-panel-header">
          <div>
            <h2 id="preferences-section-title">{sectionMeta.title}</h2>
            <p className="muted-text">{sectionMeta.description}</p>
          </div>
        </header>

        <div className="preferences-entry-list">
          {resolvedSection === 'appearance' ? (
            <article className="preferences-entry">
              <div className="preferences-entry-copy">
                <label className="preferences-entry-label" htmlFor="pref-theme">
                  Dark theme
                </label>
                <p className="preferences-entry-hint">Use the dark Precision Stealth palette.</p>
              </div>
              <input
                id="pref-theme"
                type="checkbox"
                className="settings-toggle"
                aria-label="Dark theme"
                checked={theme === 'dark'}
                onChange={() => setThemeState(toggleTheme())}
              />
            </article>
          ) : null}

          {sectionSettings.map((setting) => {
            const currentValue = draftValues[setting.key] ?? setting.value
            const isDirty = currentValue !== setting.value
            const label = humanizeKey(setting.key)
            const enumOptions = ENUM_OPTIONS[setting.key]
            const description = setting.description?.trim() || undefined

            if (enumOptions) {
              return (
                <article
                  className="preferences-entry preferences-entry-stack"
                  data-locked={!setting.mutable || undefined}
                  key={setting.key}
                >
                  <div className="preferences-entry-copy">
                    <div className="preferences-entry-label-row">
                      <span className="preferences-entry-label">{label}</span>
                      <HelpTip label={label} tooltip={`${description ?? label} (${setting.key})`} />
                    </div>
                    {description ? <p className="preferences-entry-hint">{description}</p> : null}
                  </div>
                  <select
                    aria-label={label}
                    className="preferences-select"
                    disabled={!setting.mutable || Boolean(pendingCommand)}
                    title={setting.key}
                    value={currentValue}
                    onChange={(event) => {
                      setDraftValues((current) => ({ ...current, [setting.key]: event.target.value }))
                      void handleSave(setting.key, event.target.value)
                    }}
                  >
                    {enumOptions.map((option) => (
                      <option key={option.value} value={option.value}>
                        {option.label}
                      </option>
                    ))}
                  </select>
                </article>
              )
            }

            if (isBooleanSetting(setting.value)) {
              return (
                <article
                  className="preferences-entry"
                  data-locked={!setting.mutable || undefined}
                  key={setting.key}
                >
                  <div className="preferences-entry-copy">
                    <div className="preferences-entry-label-row">
                      <label className="preferences-entry-label" htmlFor={`pref-${setting.key}`}>
                        {label}
                      </label>
                      <HelpTip label={label} tooltip={`${description ?? label} (${setting.key})`} />
                    </div>
                    {description ? <p className="preferences-entry-hint">{description}</p> : null}
                  </div>
                  <input
                    id={`pref-${setting.key}`}
                    type="checkbox"
                    className="settings-toggle"
                    aria-label={label}
                    title={setting.key}
                    checked={currentValue === 'true'}
                    disabled={!setting.mutable || Boolean(pendingCommand)}
                    onChange={() => {
                      const next = currentValue === 'true' ? 'false' : 'true'
                      setDraftValues((current) => ({ ...current, [setting.key]: next }))
                      void handleSave(setting.key, next)
                    }}
                  />
                </article>
              )
            }

            return (
              <article
                className="preferences-entry preferences-entry-stack"
                data-locked={!setting.mutable || undefined}
                key={setting.key}
              >
                <div className="preferences-entry-copy">
                  <div className="preferences-entry-label-row">
                    <label className="preferences-entry-label" htmlFor={`pref-${setting.key}`}>
                      {label}
                    </label>
                    <HelpTip label={label} tooltip={`${description ?? label} (${setting.key})`} />
                  </div>
                  {description ? <p className="preferences-entry-hint">{description}</p> : null}
                </div>
                <div className="preferences-input-row">
                  <input
                    id={`pref-${setting.key}`}
                    aria-label={label}
                    className="preferences-input"
                    disabled={!setting.mutable}
                    title={setting.key}
                    value={currentValue}
                    onChange={(event) =>
                      setDraftValues((current) => ({
                        ...current,
                        [setting.key]: event.target.value,
                      }))
                    }
                    onKeyDown={(event) => {
                      if (event.key === 'Enter' && isDirty && setting.mutable) {
                        event.preventDefault()
                        void handleSave(setting.key)
                      }
                    }}
                  />
                  <button
                    className="ghost-button preferences-save"
                    disabled={Boolean(pendingCommand) || !setting.mutable || !isDirty}
                    onClick={() => void handleSave(setting.key)}
                    type="button"
                  >
                    Save
                  </button>
                </div>
              </article>
            )
          })}

          {resolvedSection !== 'appearance' && sectionSettings.length === 0 ? (
            <div className="preferences-empty muted-text">No preferences in this section.</div>
          ) : null}
        </div>
      </section>
    </div>
  )
}
