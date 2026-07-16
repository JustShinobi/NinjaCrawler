import { useMemo, useState } from 'react'
import type { AppSetting } from '../../domain/models'
import { useAppStore } from '../../state/appStore'
import { HelpTip } from '../shared/HelpTip'

function useAppSetting(key: string): AppSetting | undefined {
  const snapshot = useAppStore((state) => state.snapshot)
  return useMemo(
    () => snapshot?.appSettings.find((entry) => entry.key === key),
    [key, snapshot?.appSettings],
  )
}

function useUpsertAppSetting() {
  return useAppStore((state) => state.upsertAppSetting)
}

function usePendingCommand() {
  return useAppStore((state) => state.pendingCommand)
}

interface SettingToggleProps {
  settingKey: string
  label: string
  hint?: string
}

export function SettingToggle({ settingKey, label, hint }: SettingToggleProps) {
  const setting = useAppSetting(settingKey)
  const upsert = useUpsertAppSetting()
  const pending = usePendingCommand()
  const [draft, setDraft] = useState<string>()

  if (!setting) return null

  const value = draft ?? setting.value
  const checked = value === 'true'
  const description = hint ?? setting.description

  return (
    <article className="preferences-entry workspace-policy-entry">
      <div className="preferences-entry-copy">
        <div className="preferences-entry-label-row">
          <label className="preferences-entry-label" htmlFor={`ws-${settingKey}`}>
            {label}
          </label>
          <HelpTip label={label} tooltip={`${description ?? label} (${settingKey})`} />
        </div>
        {description ? <p className="preferences-entry-hint">{description}</p> : null}
      </div>
      <input
        id={`ws-${settingKey}`}
        type="checkbox"
        className="settings-toggle"
        aria-label={label}
        title={settingKey}
        checked={checked}
        disabled={!setting.mutable || Boolean(pending)}
        onChange={() => {
          const next = checked ? 'false' : 'true'
          setDraft(next)
          void upsert({
            key: setting.key,
            value: next,
            category: setting.category,
            description: setting.description,
            mutable: setting.mutable,
          })
        }}
      />
    </article>
  )
}

interface SettingSelectProps {
  settingKey: string
  label: string
  options: Array<{ value: string; label: string }>
  hint?: string
}

export function SettingSelect({ settingKey, label, options, hint }: SettingSelectProps) {
  const setting = useAppSetting(settingKey)
  const upsert = useUpsertAppSetting()
  const pending = usePendingCommand()
  const [draft, setDraft] = useState<string>()

  if (!setting) return null

  const value = draft ?? setting.value
  const description = hint ?? setting.description

  return (
    <article className="preferences-entry preferences-entry-stack workspace-policy-entry">
      <div className="preferences-entry-copy">
        <div className="preferences-entry-label-row">
          <span className="preferences-entry-label">{label}</span>
          <HelpTip label={label} tooltip={`${description ?? label} (${settingKey})`} />
        </div>
        {description ? <p className="preferences-entry-hint">{description}</p> : null}
      </div>
      <select
        aria-label={label}
        className="preferences-select"
        disabled={!setting.mutable || Boolean(pending)}
        title={settingKey}
        value={value}
        onChange={(event) => {
          const next = event.target.value
          setDraft(next)
          void upsert({
            key: setting.key,
            value: next,
            category: setting.category,
            description: setting.description,
            mutable: setting.mutable,
          })
        }}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </article>
  )
}

interface SettingTextProps {
  settingKey: string
  label: string
  hint?: string
  mono?: boolean
}

export function SettingTextField({ settingKey, label, hint, mono }: SettingTextProps) {
  const setting = useAppSetting(settingKey)
  const upsert = useUpsertAppSetting()
  const pending = usePendingCommand()
  const [draft, setDraft] = useState<string>()

  if (!setting) return null

  const value = draft ?? setting.value
  const isDirty = value !== setting.value
  const description = hint ?? setting.description

  return (
    <article className="preferences-entry preferences-entry-stack workspace-policy-entry">
      <div className="preferences-entry-copy">
        <div className="preferences-entry-label-row">
          <label className="preferences-entry-label" htmlFor={`ws-${settingKey}`}>
            {label}
          </label>
          <HelpTip label={label} tooltip={`${description ?? label} (${settingKey})`} />
        </div>
        {description ? <p className="preferences-entry-hint">{description}</p> : null}
      </div>
      <div className="preferences-input-row">
        <input
          id={`ws-${settingKey}`}
          aria-label={label}
          className={mono ? 'preferences-input preferences-input-mono' : 'preferences-input'}
          disabled={!setting.mutable}
          title={settingKey}
          value={value}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && isDirty && setting.mutable) {
              event.preventDefault()
              void upsert({
                key: setting.key,
                value,
                category: setting.category,
                description: setting.description,
                mutable: setting.mutable,
              }).then(() => setDraft(undefined))
            }
          }}
        />
        <button
          className="ghost-button preferences-save"
          disabled={Boolean(pending) || !setting.mutable || !isDirty}
          onClick={() => {
            void upsert({
              key: setting.key,
              value,
              category: setting.category,
              description: setting.description,
              mutable: setting.mutable,
            }).then(() => setDraft(undefined))
          }}
          type="button"
        >
          Save
        </button>
      </div>
    </article>
  )
}
