import { useEffect, useMemo, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import {
  batchUpdateSourceProfiles,
  loadWorkspaceSnapshot,
  subscribeToBatchEditorWindowIntent,
  upsertSchedulerGroup,
  type BatchInstagramSyncOptionsPatch,
  type BatchSourceSyncOptionsPatch,
} from '../../bridge/desktop'
import type { SchedulerGroup, WorkspaceSnapshot } from '../../domain/models'
import { TWITTER_SYNC_OPTION_GROUPS } from '../../domain/twitterSyncOptionDefinitions'
import { closeDesktopWindow } from '../../utils/closeDesktopWindow'

type TriState = 'unchanged' | 'on' | 'off'

function nextTriState(current: TriState): TriState {
  if (current === 'unchanged') return 'on'
  if (current === 'on') return 'off'
  return 'unchanged'
}

interface ToggleDef {
  key: string
  label: string
}

const SECTION_TOGGLES: ToggleDef[] = [
  { key: 'timeline', label: 'Timeline' },
  { key: 'reels', label: 'Reels' },
  { key: 'stories', label: 'Stories' },
  { key: 'storiesUser', label: 'Stories (user)' },
  { key: 'tagged', label: 'Tagged' },
]

const BEHAVIOR_TOGGLES: ToggleDef[] = [
  { key: 'temporary', label: 'Temporary' },
  { key: 'favorite', label: 'Favorite' },
  { key: 'getUserMediaOnly', label: 'User media only' },
  { key: 'verifiedProfile', label: 'Verified profile' },
  { key: 'forceUpdateUserName', label: 'Force update username' },
  { key: 'forceUpdateUserInformation', label: 'Force update user information' },
  { key: 'downloadText', label: 'Download text' },
  { key: 'downloadTextPosts', label: 'Download text posts' },
]

const MEDIA_TOGGLES: ToggleDef[] = [
  { key: 'downloadImages', label: 'Download images' },
  { key: 'downloadVideos', label: 'Download videos' },
  { key: 'placeExtractedImageIntoVideoFolder', label: 'Place extracted image in video folder' },
]

const EXTRACT_MEDIA_TOGGLES: ToggleDef[] = [
  { key: 'extractImageFromVideo.timeline', label: 'Extract timeline' },
  { key: 'extractImageFromVideo.reels', label: 'Extract reels' },
  { key: 'extractImageFromVideo.stories', label: 'Extract stories' },
  { key: 'extractImageFromVideo.storiesUser', label: 'Extract stories (user)' },
  { key: 'extractImageFromVideo.tagged', label: 'Extract tagged' },
]

const GROUP_UNCHANGED = '__unchanged__'
const GROUP_CLEAR = '__clear__'
const GROUP_CREATE = '__create__'

interface BatchEditorWindowPageProps {
  initialSourceIds: string[]
}

export function BatchEditorWindowPage({ initialSourceIds }: BatchEditorWindowPageProps) {
  const [sourceIds, setSourceIds] = useState(initialSourceIds)
  const [snapshot, setSnapshot] = useState<WorkspaceSnapshot | null>(null)
  const [labelsToAdd, setLabelsToAdd] = useState<string[]>([])
  const [labelsToRemove, setLabelsToRemove] = useState<string[]>([])
  const [addLabelDraft, setAddLabelDraft] = useState('')
  const [removeLabelDraft, setRemoveLabelDraft] = useState('')
  const [readyForDownload, setReadyForDownload] = useState<TriState>('unchanged')
  const [groupAction, setGroupAction] = useState<string>(GROUP_UNCHANGED)
  const [newGroupName, setNewGroupName] = useState('')
  const [syncToggles, setSyncToggles] = useState<Record<string, TriState>>({})
  const [syncValues, setSyncValues] = useState<Record<string, string>>({})
  const [changedSyncValues, setChangedSyncValues] = useState<Set<string>>(new Set())
  const [applying, setApplying] = useState(false)
  const [applyError, setApplyError] = useState<string | undefined>(undefined)
  const [collapsedSections, setCollapsedSections] = useState<Set<string>>(new Set())

  useEffect(() => {
    void loadWorkspaceSnapshot()
      .then(setSnapshot)
      .catch(() => undefined)
  }, [])

  useEffect(() => {
    let disposed = false
    let unsubscribe: (() => void) | undefined

    void subscribeToBatchEditorWindowIntent((intent) => {
      if (!disposed && intent.sourceIds.length > 0) {
        setSourceIds(intent.sourceIds)
      }
    })
      .then((teardown) => {
        if (disposed) {
          teardown()
          return
        }
        unsubscribe = teardown
      })
      .catch(() => undefined)

    return () => {
      disposed = true
      unsubscribe?.()
    }
  }, [])

  const allKnownLabels = useMemo(() => {
    if (!snapshot) return []
    const labels = new Set<string>()
    snapshot.sources.forEach((source) => {
      source.labels.forEach((label) => {
        const normalized = label.trim()
        if (normalized) labels.add(normalized)
      })
    })
    return Array.from(labels).sort((a, b) => a.localeCompare(b))
  }, [snapshot])

  const addLabelSuggestions = useMemo(() => {
    const query = addLabelDraft.trim().toLowerCase()
    return allKnownLabels
      .filter((label) => !labelsToAdd.some((l) => l.toLowerCase() === label.toLowerCase()))
      .filter((label) => !query || label.toLowerCase().includes(query))
      .slice(0, 12)
  }, [allKnownLabels, labelsToAdd, addLabelDraft])

  const removeLabelSuggestions = useMemo(() => {
    const query = removeLabelDraft.trim().toLowerCase()
    return allKnownLabels
      .filter((label) => !labelsToRemove.some((l) => l.toLowerCase() === label.toLowerCase()))
      .filter((label) => !query || label.toLowerCase().includes(query))
      .slice(0, 12)
  }, [allKnownLabels, labelsToRemove, removeLabelDraft])

  const schedulerGroups: SchedulerGroup[] = snapshot?.schedulerGroups ?? []
  const selectedProviders = useMemo(() => {
    const selectedIdSet = new Set(sourceIds)
    return new Set(
      snapshot?.sources
        .filter((source) => selectedIdSet.has(source.id))
        .map((source) => source.provider) ?? [],
    )
  }, [snapshot, sourceIds])
  const hasInstagramSources = selectedProviders.has('instagram')
  const hasTwitterSources = selectedProviders.has('twitter')

  function getTriState(key: string): TriState {
    return syncToggles[key] ?? 'unchanged'
  }

  function toggleSyncOption(key: string) {
    setSyncToggles((prev) => ({
      ...prev,
      [key]: nextTriState(prev[key] ?? 'unchanged'),
    }))
  }

  function toggleSyncValue(key: string) {
    setChangedSyncValues((current) => {
      const next = new Set(current)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }

  function toggleSection(section: string) {
    setCollapsedSections((prev) => {
      const next = new Set(prev)
      if (next.has(section)) {
        next.delete(section)
      } else {
        next.add(section)
      }
      return next
    })
  }

  function commitAddLabel() {
    const candidates = parseLabelCandidates(addLabelDraft)
    if (candidates.length === 0) return
    setLabelsToAdd((current) => mergeLabels(current, candidates))
    setAddLabelDraft('')
  }

  function commitRemoveLabel() {
    const candidates = parseLabelCandidates(removeLabelDraft)
    if (candidates.length === 0) return
    setLabelsToRemove((current) => mergeLabels(current, candidates))
    setRemoveLabelDraft('')
  }

  function handleAddLabelKeyDown(event: ReactKeyboardEvent<HTMLInputElement>) {
    if (event.key === 'Enter' || event.key === ',') {
      event.preventDefault()
      commitAddLabel()
      return
    }
    if (event.key === 'Backspace' && addLabelDraft.trim().length === 0 && labelsToAdd.length > 0) {
      event.preventDefault()
      setLabelsToAdd((current) => current.slice(0, current.length - 1))
    }
  }

  function handleRemoveLabelKeyDown(event: ReactKeyboardEvent<HTMLInputElement>) {
    if (event.key === 'Enter' || event.key === ',') {
      event.preventDefault()
      commitRemoveLabel()
      return
    }
    if (event.key === 'Backspace' && removeLabelDraft.trim().length === 0 && labelsToRemove.length > 0) {
      event.preventDefault()
      setLabelsToRemove((current) => current.slice(0, current.length - 1))
    }
  }

  async function handleCreateGroupInline() {
    const name = newGroupName.trim()
    if (!name) return
    const previousGroupIds = new Set((snapshot?.schedulerGroups ?? []).map((group) => group.id))
    try {
      const newSnapshot = await upsertSchedulerGroup({ name, criteria: defaultCriteria() })
      setSnapshot(newSnapshot)
      const created =
        newSnapshot.schedulerGroups.find((group) => !previousGroupIds.has(group.id)) ??
        newSnapshot.schedulerGroups.find((group) => group.name.trim().toLowerCase() === name.toLowerCase())
      if (created) {
        setGroupAction(created.id)
        setApplyError(undefined)
      } else {
        setApplyError('The new group was created, but it could not be selected automatically. Select a group before applying.')
      }
      setNewGroupName('')
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      setApplyError(`Failed to create group: ${message}`)
      console.error('Failed to create group:', error)
    }
  }

  async function handleApply() {
    setApplyError(undefined)
    setApplying(true)
    try {
      const patch: BatchSourceSyncOptionsPatch = {}
      let hasSyncChanges = false
      if (hasInstagramSources) {
        const instagramPatch: BatchInstagramSyncOptionsPatch = {}
        for (const toggle of [...SECTION_TOGGLES, ...BEHAVIOR_TOGGLES, ...MEDIA_TOGGLES]) {
          const state = getTriState(toggle.key)
          if (state !== 'unchanged') {
            (instagramPatch as Record<string, boolean>)[toggle.key] = state === 'on'
            hasSyncChanges = true
          }
        }
        for (const toggle of EXTRACT_MEDIA_TOGGLES) {
          const state = getTriState(toggle.key)
          if (state !== 'unchanged') {
            instagramPatch.extractImageFromVideo ??= {}
            const extractKey = toggle.key.replace('extractImageFromVideo.', '')
            const extractPatch = instagramPatch.extractImageFromVideo as Record<string, boolean>
            extractPatch[extractKey] = state === 'on'
            hasSyncChanges = true
          }
        }
        if (hasSyncChanges) patch.instagram = instagramPatch
      }

      if (hasTwitterSources) {
        const twitterPatch: Record<string, boolean | string> = {}
        for (const option of TWITTER_SYNC_OPTION_GROUPS.flatMap((group) => group.options)) {
          const stateKey = `twitter.${option.key}`
          if (option.type === 'boolean') {
            const state = getTriState(stateKey)
            if (state !== 'unchanged') twitterPatch[option.key] = state === 'on'
          } else if (changedSyncValues.has(stateKey)) {
            twitterPatch[option.key] = syncValues[stateKey] ?? ''
          }
        }
        if (Object.keys(twitterPatch).length > 0) {
          patch.twitter = twitterPatch
          hasSyncChanges = true
        }
      }

      let setGroupId: string | null | undefined = undefined
      if (groupAction === GROUP_CLEAR) {
        setGroupId = null
      } else if (groupAction !== GROUP_UNCHANGED && groupAction !== GROUP_CREATE) {
        setGroupId = groupAction
      }

      const nextSnapshot = await batchUpdateSourceProfiles({
        sourceIds: sourceIds,
        labelsToAdd,
        labelsToRemove,
        readyForDownload: readyForDownload === 'unchanged' ? undefined : readyForDownload === 'on',
        syncOptionsPatch: hasSyncChanges ? patch : undefined,
        setGroupId,
      })

      setSnapshot(nextSnapshot)
      void closeDesktopWindow()
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      setApplyError(`Failed to apply changes: ${message}`)
      console.error('Batch update failed:', error)
    } finally {
      setApplying(false)
    }
  }

  const hasChanges =
    labelsToAdd.length > 0 ||
    labelsToRemove.length > 0 ||
    readyForDownload !== 'unchanged' ||
    (groupAction !== GROUP_UNCHANGED) ||
    Object.values(syncToggles).some((v) => v !== 'unchanged') ||
    changedSyncValues.size > 0
  const applyBlockedByPendingGroupCreation = groupAction === GROUP_CREATE

  return (
    <div className="batch-editor-shell">
      <header className="batch-editor-header">
        <h2>Change Parameters</h2>
        <span className="batch-editor-count">{sourceIds.length} profiles selected</span>
      </header>

      <div className="batch-editor-body">
        <BatchSection
          collapsed={collapsedSections.has('labels')}
          onToggle={() => toggleSection('labels')}
          title="Labels"
        >
          <div className="batch-editor-chip-field">
            <label>Add labels</label>
            <div className="batch-editor-chip-input">
              {labelsToAdd.map((label) => (
                <button
                  aria-label={`Remove ${label}`}
                  className="batch-editor-chip"
                  key={label}
                  onClick={() => setLabelsToAdd((current) => current.filter((l) => l.toLowerCase() !== label.toLowerCase()))}
                  type="button"
                >
                  {label}
                  <span aria-hidden>&times;</span>
                </button>
              ))}
              <input
                list="batch-add-label-suggestions"
                onBlur={() => commitAddLabel()}
                onChange={(e) => setAddLabelDraft(e.target.value)}
                onKeyDown={handleAddLabelKeyDown}
                placeholder="Type and press Enter"
                value={addLabelDraft}
              />
              <button
                className="ghost-button batch-editor-chip-add"
                disabled={addLabelDraft.trim().length === 0}
                onClick={() => commitAddLabel()}
                type="button"
              >
                Add
              </button>
            </div>
            <datalist id="batch-add-label-suggestions">
              {addLabelSuggestions.map((label) => (
                <option key={label} value={label} />
              ))}
            </datalist>
          </div>

          <div className="batch-editor-chip-field">
            <label>Remove labels</label>
            <div className="batch-editor-chip-input">
              {labelsToRemove.map((label) => (
                <button
                  aria-label={`Remove ${label}`}
                  className="batch-editor-chip batch-editor-chip-remove"
                  key={label}
                  onClick={() => setLabelsToRemove((current) => current.filter((l) => l.toLowerCase() !== label.toLowerCase()))}
                  type="button"
                >
                  {label}
                  <span aria-hidden>&times;</span>
                </button>
              ))}
              <input
                list="batch-remove-label-suggestions"
                onBlur={() => commitRemoveLabel()}
                onChange={(e) => setRemoveLabelDraft(e.target.value)}
                onKeyDown={handleRemoveLabelKeyDown}
                placeholder="Type and press Enter"
                value={removeLabelDraft}
              />
              <button
                className="ghost-button batch-editor-chip-add"
                disabled={removeLabelDraft.trim().length === 0}
                onClick={() => commitRemoveLabel()}
                type="button"
              >
                Add
              </button>
            </div>
            <datalist id="batch-remove-label-suggestions">
              {removeLabelSuggestions.map((label) => (
                <option key={label} value={label} />
              ))}
            </datalist>
          </div>
        </BatchSection>

        <BatchSection
          collapsed={collapsedSections.has('group')}
          onToggle={() => toggleSection('group')}
          title="Group"
        >
          <div className="batch-editor-group-field">
            <select
              className="batch-editor-group-select"
              onChange={(e) => {
                setGroupAction(e.target.value)
                setApplyError(undefined)
              }}
              value={groupAction}
            >
              <option value={GROUP_UNCHANGED}>— No change —</option>
              <option value={GROUP_CLEAR}>Clear group</option>
              {schedulerGroups.map((group) => (
                <option key={group.id} value={group.id}>
                  {group.name}
                </option>
              ))}
              <option value={GROUP_CREATE}>+ Create new group...</option>
            </select>
            {groupAction === GROUP_CREATE ? (
              <div className="batch-editor-group-create-row">
                <input
                  onChange={(e) => setNewGroupName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault()
                      void handleCreateGroupInline()
                    }
                  }}
                  placeholder="Group name"
                  type="text"
                  value={newGroupName}
                />
                <button
                  className="ghost-button"
                  disabled={newGroupName.trim().length === 0}
                  onClick={() => void handleCreateGroupInline()}
                  type="button"
                >
                  Create
                </button>
              </div>
            ) : null}
          </div>
        </BatchSection>

        <BatchSection
          collapsed={collapsedSections.has('status')}
          onToggle={() => toggleSection('status')}
          title="Status"
        >
          <TriStateRow
            label="Ready for download"
            onChange={() => setReadyForDownload(nextTriState(readyForDownload))}
            value={readyForDownload}
          />
        </BatchSection>

        {hasInstagramSources ? <BatchSection
          collapsed={collapsedSections.has('sections')}
          onToggle={() => toggleSection('sections')}
          title="Instagram · Sections"
        >
          {SECTION_TOGGLES.map((toggle) => (
            <TriStateRow
              key={toggle.key}
              label={toggle.label}
              onChange={() => toggleSyncOption(toggle.key)}
              value={getTriState(toggle.key)}
            />
          ))}
        </BatchSection> : null}

        {hasInstagramSources ? <BatchSection
          collapsed={collapsedSections.has('behavior')}
          onToggle={() => toggleSection('behavior')}
          title="Instagram · Behavior"
        >
          {BEHAVIOR_TOGGLES.map((toggle) => (
            <TriStateRow
              key={toggle.key}
              label={toggle.label}
              onChange={() => toggleSyncOption(toggle.key)}
              value={getTriState(toggle.key)}
            />
          ))}
        </BatchSection> : null}

        {hasInstagramSources ? <BatchSection
          collapsed={collapsedSections.has('media')}
          onToggle={() => toggleSection('media')}
          title="Instagram · Media"
        >
          {MEDIA_TOGGLES.map((toggle) => (
            <TriStateRow
              key={toggle.key}
              label={toggle.label}
              onChange={() => toggleSyncOption(toggle.key)}
              value={getTriState(toggle.key)}
            />
          ))}
          {EXTRACT_MEDIA_TOGGLES.map((toggle) => (
            <TriStateRow
              key={toggle.key}
              label={toggle.label}
              onChange={() => toggleSyncOption(toggle.key)}
              value={getTriState(toggle.key)}
            />
          ))}
        </BatchSection> : null}

        {hasTwitterSources ? TWITTER_SYNC_OPTION_GROUPS.map((group) => {
          const sectionKey = `twitter.${group.title}`
          return (
            <BatchSection
              collapsed={collapsedSections.has(sectionKey)}
              key={sectionKey}
              onToggle={() => toggleSection(sectionKey)}
              title={`X / Twitter · ${group.title}`}
            >
              {group.options.map((option) => {
                const stateKey = `twitter.${option.key}`
                return option.type === 'boolean' ? (
                  <TriStateRow
                    key={option.key}
                    label={option.label}
                    onChange={() => toggleSyncOption(stateKey)}
                    value={getTriState(stateKey)}
                  />
                ) : (
                  <BatchValueRow
                    changed={changedSyncValues.has(stateKey)}
                    key={option.key}
                    label={option.label}
                    onChange={(value) => setSyncValues((current) => ({ ...current, [stateKey]: value }))}
                    onToggle={() => toggleSyncValue(stateKey)}
                    value={syncValues[stateKey] ?? ''}
                  />
                )
              })}
            </BatchSection>
          )
        }) : null}
      </div>

      <footer className="batch-editor-footer">
        {applyError ? (
          <p aria-live="polite" className="batch-editor-error">
            {applyError}
          </p>
        ) : null}
        <button
          className="batch-editor-cancel"
          onClick={() => void closeDesktopWindow()}
          type="button"
        >
          Cancel
        </button>
        <button
          className="batch-editor-apply"
          disabled={applying || !hasChanges || applyBlockedByPendingGroupCreation}
          onClick={() => void handleApply()}
          type="button"
        >
          {applying ? 'Applying...' : 'Apply changes'}
        </button>
      </footer>
    </div>
  )
}

// -- Helpers --

function parseLabelCandidates(value: string): string[] {
  return value.split(',').map((c) => c.trim()).filter((c) => c.length > 0)
}

function mergeLabels(base: string[], candidates: string[]): string[] {
  const merged = [...base]
  const known = new Set(base.map((l) => l.toLowerCase()))
  for (const candidate of candidates) {
    const key = candidate.toLowerCase()
    if (!known.has(key)) {
      known.add(key)
      merged.push(candidate)
    }
  }
  return merged
}

function defaultCriteria(): import('../../domain/models').SchedulerPlanCriteria {
  return {
    regular: false,
    temporary: false,
    favorite: false,
    readyForDownload: false,
    ignoreReadyForDownload: false,
    downloadUsers: false,
    downloadSubscriptions: false,
    userExists: false,
    userSuspended: false,
    userDeleted: false,
    labelsNo: false,
    labelsIncluded: [],
    labelsExcluded: [],
    ignoreExcludedLabels: false,
    sitesIncluded: [],
    sitesExcluded: [],
    groupIdsIncluded: [],
    groupIdsExcluded: [],
    daysIsDownloaded: false,
    dateInRange: true,
  }
}

// -- Sections --

interface BatchSectionProps {
  title: string
  collapsed: boolean
  onToggle: () => void
  children: React.ReactNode
}

function BatchSection({ title, collapsed, onToggle, children }: BatchSectionProps) {
  return (
    <div className="batch-editor-section">
      <button className="batch-editor-section-header" onClick={onToggle} type="button">
        <span className={collapsed ? 'batch-editor-chevron collapsed' : 'batch-editor-chevron'}>&#9656;</span>
        <strong>{title}</strong>
      </button>
      {!collapsed ? (
        <div className="batch-editor-section-body">{children}</div>
      ) : null}
    </div>
  )
}

// -- Tri-state toggle --

interface TriStateRowProps {
  label: string
  value: TriState
  onChange: () => void
}

function TriStateRow({ label, value, onChange }: TriStateRowProps) {
  return (
    <button aria-label={label} className="batch-editor-toggle-row" onClick={onChange} type="button">
      <span className={`tri-state-indicator tri-state-${value}`}>
        {value === 'on' ? '\u2713' : value === 'off' ? '\u2715' : '\u2014'}
      </span>
      <span>{label}</span>
    </button>
  )
}

interface BatchValueRowProps {
  label: string
  value: string
  changed: boolean
  onToggle: () => void
  onChange: (value: string) => void
}

function BatchValueRow({ label, value, changed, onToggle, onChange }: BatchValueRowProps) {
  return (
    <div className="batch-editor-value-row">
      <button
        aria-label={label}
        aria-pressed={changed}
        className="batch-editor-value-mode"
        onClick={onToggle}
        type="button"
      >
        <span className={`tri-state-indicator tri-state-${changed ? 'on' : 'unchanged'}`}>
          {changed ? '\u2713' : '\u2014'}
        </span>
        <span>{label}</span>
      </button>
      <input
        aria-label={`${label} value`}
        disabled={!changed}
        onChange={(event) => onChange(event.target.value)}
        type="text"
        value={value}
      />
    </div>
  )
}
