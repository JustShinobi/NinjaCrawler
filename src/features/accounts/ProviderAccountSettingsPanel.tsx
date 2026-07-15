import { useMemo, useState } from 'react'
import type { ProviderKey } from '../../domain/models'
import { HelpTip } from '../shared/HelpTip'
import {
  type ProviderAccountSettingsCategoryKey,
  type ProviderAccountSettingsField,
  getProviderAccountSettingsFields,
  getProviderAccountSettingsLayout,
} from './providerAccountSettings'

interface ProviderAccountSettingsPanelProps {
  provider: ProviderKey
  loading: boolean
  draft: Record<string, string>
  onFieldChange: (key: string, value: string) => void
  visibleCategories?: ProviderAccountSettingsCategoryKey[]
}

function SettingsFieldControl({
  field,
  value,
  onFieldChange,
}: {
  field: ProviderAccountSettingsField
  value: string
  onFieldChange: (key: string, value: string) => void
}) {
  const inputId = `provider-setting-${field.key}`
  const monoClass = field.mono ? 'accounts-field-mono' : undefined

  if (field.kind === 'toggle') {
    return (
      <div className="accounts-toggle-row">
        <input
          checked={value === 'true'}
          id={inputId}
          onChange={(event) => onFieldChange(field.key, event.target.checked ? 'true' : 'false')}
          type="checkbox"
        />
        <div className="accounts-toggle-copy">
          <label className="accounts-setting-label" htmlFor={inputId}>
            <span>{field.label}</span>
          </label>
          {field.tooltip ? <HelpTip label={field.label} tooltip={field.tooltip} /> : null}
          {field.description ? <small className="accounts-toggle-hint">{field.description}</small> : null}
        </div>
      </div>
    )
  }

  if (field.kind === 'textarea') {
    return (
      <div className="field field-full">
        <span className="accounts-setting-label">
          <label htmlFor={inputId}>{field.label}</label>
          {field.tooltip ? <HelpTip label={field.label} tooltip={field.tooltip} /> : null}
        </span>
        <textarea
          className={monoClass}
          id={inputId}
          onChange={(event) => onFieldChange(field.key, event.target.value)}
          placeholder={field.placeholder}
          rows={3}
          value={value}
        />
        {field.description ? <small>{field.description}</small> : null}
      </div>
    )
  }

  return (
    <div className="field">
      <span className="accounts-setting-label">
        <label htmlFor={inputId}>{field.label}</label>
        {field.tooltip ? <HelpTip label={field.label} tooltip={field.tooltip} /> : null}
      </span>
      <input
        className={monoClass}
        id={inputId}
        onChange={(event) => onFieldChange(field.key, event.target.value)}
        placeholder={field.placeholder}
        type={field.kind === 'number' ? 'number' : 'text'}
        value={value}
      />
      {field.description ? <small>{field.description}</small> : null}
    </div>
  )
}

function CategoryFields({
  fields,
  draft,
  onFieldChange,
}: {
  fields: ProviderAccountSettingsField[]
  draft: Record<string, string>
  onFieldChange: (key: string, value: string) => void
}) {
  const primary = fields.filter((field) => !field.advanced)
  const advanced = fields.filter((field) => field.advanced)
  const [advancedOpen, setAdvancedOpen] = useState(false)

  const toggles = primary.filter((field) => field.kind === 'toggle')
  const others = primary.filter((field) => field.kind !== 'toggle')

  return (
    <>
      {others.length > 0 ? (
        <div className="form-grid accounts-settings-grid">
          {others.map((field) => (
            <SettingsFieldControl
              field={field}
              key={field.key}
              onFieldChange={onFieldChange}
              value={draft[field.key] ?? field.defaultValue}
            />
          ))}
        </div>
      ) : null}

      {toggles.length > 0 ? (
        <div className="accounts-toggle-list" role="group">
          {toggles.map((field) => (
            <SettingsFieldControl
              field={field}
              key={field.key}
              onFieldChange={onFieldChange}
              value={draft[field.key] ?? field.defaultValue}
            />
          ))}
        </div>
      ) : null}

      {advanced.length > 0 ? (
        <div className="accounts-advanced-block">
          <button
            aria-expanded={advancedOpen}
            className="accounts-advanced-toggle"
            onClick={() => setAdvancedOpen((current) => !current)}
            type="button"
          >
            {advancedOpen ? 'Hide advanced fields' : 'Show advanced fields'}
          </button>
          {advancedOpen ? (
            <div className="form-grid accounts-settings-grid accounts-advanced-fields">
              {advanced.map((field) => (
                <SettingsFieldControl
                  field={field}
                  key={field.key}
                  onFieldChange={onFieldChange}
                  value={draft[field.key] ?? field.defaultValue}
                />
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
    </>
  )
}

export function ProviderAccountSettingsPanel({
  provider,
  loading,
  draft,
  onFieldChange,
  visibleCategories,
}: ProviderAccountSettingsPanelProps) {
  const layout = getProviderAccountSettingsLayout(provider)

  const categories = useMemo(() => {
    if (!layout) {
      return []
    }
    return visibleCategories
      ? layout.categories.filter((category) => visibleCategories.includes(category.key))
      : layout.categories
  }, [layout, visibleCategories])

  if (!layout) {
    return (
      <section className="accounts-section">
        <header className="accounts-section-header">
          <h3>Provider settings</h3>
        </header>
        <p className="accounts-section-copy">This provider has no settings schema yet.</p>
      </section>
    )
  }

  if (loading) {
    return (
      <section className="accounts-section accounts-section-loading">
        <header className="accounts-section-header">
          <h3>Loading configuration</h3>
        </header>
        <p className="accounts-section-copy">Loading provider account settings…</p>
      </section>
    )
  }

  return (
    <>
      {categories.map((category) => {
        const fields = getProviderAccountSettingsFields(provider, category.key)
        if (fields.length === 0) {
          return null
        }

        return (
          <section className="accounts-section" key={category.key}>
            <header className="accounts-section-header">
              <h3>{category.label}</h3>
              {category.description ? <p className="accounts-section-copy">{category.description}</p> : null}
            </header>
            <CategoryFields draft={draft} fields={fields} onFieldChange={onFieldChange} />
          </section>
        )
      })}
    </>
  )
}
