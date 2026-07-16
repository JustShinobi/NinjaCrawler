import { useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { loadSystemShortDatePattern } from '../../bridge/desktop'

import type {
  PlanEditorWindowIntent,
  ProviderKey,
  SchedulerGroup,
  SchedulerPauseMode,
  SchedulerPlanCriteria,
  SchedulerPlanNotifications,
  SchedulerSet,
  SetSyncPlanPauseInput,
  SkipSyncPlanInput,
  SyncPlan,
  SyncPlanRun,
  SyncPlanUpsert,
} from '../../domain/models'
import { useAppStore } from '../../state/appStore'
import { SettingSelect } from '../settings/appSettingControls'
import { HelpTip } from '../shared/HelpTip'
import {
  PAUSE_PRESETS,
  PROVIDERS,
  checkbox,
  createPlanDraft,
  findPlanById,
  findRunsForPlan,
  findSetById,
  firstActiveSet,
  groupNameMap,
  mapPlanToDraft,
  runtimeStateLabel,
  statusLabel,
  syncNotificationMode,
} from './schedulerShared'

function PlanWorkspaceNotificationDefault() {
  return (
    <SettingSelect
      settingKey="policy.notifications.default"
      label="Default notification style for new plans"
      options={[
        { value: 'summary', label: 'Summary (simple)' },
        { value: 'detailed', label: 'Detailed' },
      ]}
      hint="Applied when creating new plans. This plan's style above is independent."
    />
  )
}

type EditorTab = 'general' | 'filters' | 'runtime'
type FilterBooleanKey =
  | 'regular'
  | 'temporary'
  | 'favorite'
  | 'userExists'
  | 'userSuspended'
  | 'userDeleted'
  | 'readyForDownload'
  | 'ignoreReadyForDownload'
  | 'downloadUsers'
  | 'downloadSubscriptions'

interface FilterToggleDefinition {
  key: FilterBooleanKey
  label: string
  tooltip?: string
  tone?: 'default' | 'warning' | 'danger'
}

interface SelectionOption {
  id: string
  label: string
}

const EMPTY_SETS: SchedulerSet[] = []
const EMPTY_GROUPS: SchedulerGroup[] = []
const EMPTY_RUNS: SyncPlanRun[] = []

const REMOTE_STATE_TOGGLES: FilterToggleDefinition[] = [
  { key: 'userExists', label: 'User exists' },
  { key: 'userSuspended', label: 'User suspended', tone: 'warning' },
  { key: 'userDeleted', label: 'User deleted', tone: 'danger' },
]

const DOWNLOAD_GATE_TOGGLES: FilterToggleDefinition[] = [
  { key: 'readyForDownload', label: 'Ready for download' },
  { key: 'ignoreReadyForDownload', label: 'Ignore ready for download', tooltip: 'Bypasses the ready gate and lets the other filters decide.' },
]

function appendUnique(values: string[], nextValue: string): string[] {
  const normalizedValue = nextValue.trim()
  if (!normalizedValue || values.includes(normalizedValue)) {
    return values
  }
  return [...values, normalizedValue]
}

function removeValue(values: string[], targetValue: string): string[] {
  return values.filter((value) => value !== targetValue)
}

function summarizeSelection(values: string[], fallback = 'Any'): string {
  return values.length > 0 ? values.join(', ') : fallback
}

function selectionSummary(values: string[], optionsMap: Map<string, string>, fallback = 'Any'): string {
  if (values.length === 0) {
    return fallback
  }
  return values.map((value) => optionsMap.get(value) ?? value).join(', ')
}

function helpButton(label: string, tooltip?: string) {
  return <HelpTip label={label} tooltip={tooltip} />
}

function countSummary(count: number, singular: string, plural: string): string {
  return count === 1 ? `1 ${singular}` : `${count} ${plural}`
}

interface LocalDateFormat {
  pattern: string
  order: Array<'day' | 'month' | 'year'>
  placeholder: string
}

type CalendarFieldKey = 'dateFrom' | 'dateTo' | 'pauseUntilDate' | 'skipUntilDate'

function detectLocalDateFormat(): LocalDateFormat {
  return localDateFormatFromPattern(Intl.DateTimeFormat().resolvedOptions().locale.toLowerCase().startsWith('pt')
    ? 'dd/MM/yyyy'
    : 'MM/dd/yyyy')
}

function localDateFormatFromPattern(pattern: string): LocalDateFormat {
  const normalizedPattern = pattern.trim() || 'yyyy-MM-dd'
  const tokens = normalizedPattern.match(/d+|M+|y+/gi) ?? []
  const order = tokens
    .map((token) => {
      const lowerToken = token.toLowerCase()
      if (lowerToken.startsWith('d')) return 'day'
      if (lowerToken.startsWith('m')) return 'month'
      if (lowerToken.startsWith('y')) return 'year'
      return null
    })
    .filter((token): token is 'day' | 'month' | 'year' => token !== null)

  const placeholder = normalizedPattern
    .replace(/d+/gi, 'dd')
    .replace(/m+/gi, 'mm')
    .replace(/y+/gi, 'aaaa')

  return {
    pattern: normalizedPattern,
    order: order.length === 3 ? order : ['day', 'month', 'year'],
    placeholder,
  }
}

function formatIsoDateForLocale(value: string | undefined, format: LocalDateFormat): string {
  if (!value) {
    return ''
  }

  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value)
  if (!match) {
    return value
  }

  const [, year, month, day] = match
  const parts = {
    day,
    month,
    year,
  }
  return format.pattern.replace(/d+|M+|y+/gi, (token) => {
    const lowerToken = token.toLowerCase()
    if (lowerToken.startsWith('d')) {
      return token.length === 1 ? String(Number(parts.day)) : parts.day
    }
    if (lowerToken.startsWith('m')) {
      return token.length === 1 ? String(Number(parts.month)) : parts.month
    }
    if (lowerToken.startsWith('y')) {
      if (token.length <= 2) {
        return parts.year.slice(-2)
      }
      return parts.year
    }
    return token
  })
}

function parseLocalizedDateInput(rawValue: string, format: LocalDateFormat): string | undefined {
  const normalized = rawValue.trim()
  if (!normalized) {
    return undefined
  }

  const chunks = normalized.split(/[.\-/\s]+/).filter(Boolean)
  if (chunks.length !== 3) {
    return undefined
  }

  const dateParts = { day: 0, month: 0, year: 0 }
  for (const [index, partType] of format.order.entries()) {
    dateParts[partType] = Number(chunks[index])
  }

  if (!Number.isInteger(dateParts.day) || !Number.isInteger(dateParts.month) || !Number.isInteger(dateParts.year)) {
    return undefined
  }
  if (dateParts.year < 1000 || dateParts.year > 9999) {
    return undefined
  }
  if (dateParts.month < 1 || dateParts.month > 12) {
    return undefined
  }

  const candidate = new Date(Date.UTC(dateParts.year, dateParts.month - 1, dateParts.day))
  if (
    candidate.getUTCFullYear() !== dateParts.year
    || candidate.getUTCMonth() !== dateParts.month - 1
    || candidate.getUTCDate() !== dateParts.day
  ) {
    return undefined
  }

  return `${String(dateParts.year).padStart(4, '0')}-${String(dateParts.month).padStart(2, '0')}-${String(dateParts.day).padStart(2, '0')}`
}

function parseIsoDateParts(value: string | undefined): { year: number, month: number, day: number } | null {
  if (!value) {
    return null
  }

  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value)
  if (!match) {
    return null
  }

  return {
    year: Number(match[1]),
    month: Number(match[2]),
    day: Number(match[3]),
  }
}

function splitLocalDateTimeValue(value: string | undefined): { date?: string, time: string } {
  if (!value) {
    return { date: undefined, time: '00:00' }
  }

  const match = /^(\d{4}-\d{2}-\d{2})(?:[T\s](\d{2}):(\d{2}))?/.exec(value)
  if (!match) {
    return { date: undefined, time: '00:00' }
  }

  return {
    date: match[1],
    time: match[2] && match[3] ? `${match[2]}:${match[3]}` : '00:00',
  }
}

function normalizeTimeInput(rawValue: string): string | undefined {
  const trimmed = rawValue.trim()
  if (!trimmed) {
    return undefined
  }

  const match = /^(\d{1,2}):(\d{2})$/.exec(trimmed)
  if (!match) {
    return undefined
  }

  const hours = Number(match[1])
  const minutes = Number(match[2])
  if (hours < 0 || hours > 23 || minutes < 0 || minutes > 59) {
    return undefined
  }

  return `${String(hours).padStart(2, '0')}:${String(minutes).padStart(2, '0')}`
}

function buildLocalDateTimeValue(dateValue: string | undefined, timeValue: string | undefined): string | undefined {
  if (!dateValue) {
    return undefined
  }

  return `${dateValue}T${normalizeTimeInput(timeValue ?? '') ?? '00:00'}`
}

function shiftMonth(month: Date, delta: number): Date {
  return new Date(month.getFullYear(), month.getMonth() + delta, 1)
}

function startOfCalendarGrid(month: Date): Date {
  const firstDay = new Date(month.getFullYear(), month.getMonth(), 1)
  const weekDay = firstDay.getDay()
  return new Date(month.getFullYear(), month.getMonth(), 1 - weekDay)
}

function calendarDays(month: Date): Date[] {
  const start = startOfCalendarGrid(month)
  return Array.from({ length: 42 }, (_, index) => new Date(start.getFullYear(), start.getMonth(), start.getDate() + index))
}

function isoDateFromCalendarDate(value: Date): string {
  return `${value.getFullYear()}-${String(value.getMonth() + 1).padStart(2, '0')}-${String(value.getDate()).padStart(2, '0')}`
}

function monthLabel(value: Date): string {
  return new Intl.DateTimeFormat(undefined, { month: 'long', year: 'numeric' }).format(value)
}

function weekDayLabels(): string[] {
  const sunday = new Date(Date.UTC(2026, 0, 4))
  return Array.from({ length: 7 }, (_, index) => (
    new Intl.DateTimeFormat(undefined, { weekday: 'short', timeZone: 'UTC' })
      .format(new Date(sunday.getTime() + (index * 24 * 60 * 60 * 1000)))
  ))
}

function CalendarIcon() {
  return (
    <svg aria-hidden="true" className="plans-date-picker-icon" viewBox="0 0 16 16">
      <path d="M4 1.5a.75.75 0 0 1 .75.75V3h6.5v-.75a.75.75 0 0 1 1.5 0V3h.5A1.75 1.75 0 0 1 15 4.75v8.5A1.75 1.75 0 0 1 13.25 15h-10.5A1.75 1.75 0 0 1 1 13.25v-8.5A1.75 1.75 0 0 1 2.75 3h.5v-.75A.75.75 0 0 1 4 1.5Zm9.5 5h-11v6.75c0 .138.112.25.25.25h10.5a.25.25 0 0 0 .25-.25V6.5Zm-10.75-2a.25.25 0 0 0-.25.25V5h11v-.25a.25.25 0 0 0-.25-.25h-.5v.25a.75.75 0 0 1-1.5 0V4.5h-6.5v.25a.75.75 0 0 1-1.5 0V4.5h-.5Z" />
    </svg>
  )
}

interface SchedulerPageProps {
  initialIntent?: PlanEditorWindowIntent
}

function normalizeIntent(intent?: PlanEditorWindowIntent): PlanEditorWindowIntent {
  return {
    mode: intent?.mode ?? 'edit',
    planId: intent?.planId?.trim() || undefined,
    schedulerSetId: intent?.schedulerSetId?.trim() || undefined,
  }
}

function trimProviderList(values: string[]): ProviderKey[] {
  return values.filter((value): value is ProviderKey => PROVIDERS.includes(value as ProviderKey))
}

function pickPlanForIntent(schedulerSets: SchedulerSet[], intent: PlanEditorWindowIntent): SyncPlan | undefined {
  if (intent.planId) {
    return findPlanById(schedulerSets, intent.planId)
  }

  const candidateSet = findSetById(schedulerSets, intent.schedulerSetId) ?? firstActiveSet(schedulerSets)
  return candidateSet?.plans[0]
}

export function SchedulerPage({ initialIntent }: SchedulerPageProps) {
  const snapshot = useAppStore((state) => state.snapshot)
  const pendingCommand = useAppStore((state) => state.pendingCommand)
  const upsertSyncPlan = useAppStore((state) => state.upsertSyncPlan)
  const previewSyncPlanTarget = useAppStore((state) => state.previewSyncPlanTarget)
  const deleteSyncPlan = useAppStore((state) => state.deleteSyncPlan)
  const setSyncPlanPause = useAppStore((state) => state.setSyncPlanPause)
  const clearSyncPlanPause = useAppStore((state) => state.clearSyncPlanPause)
  const applySyncPlanSkip = useAppStore((state) => state.applySyncPlanSkip)
  const moveSyncPlan = useAppStore((state) => state.moveSyncPlan)
  const cloneSyncPlan = useAppStore((state) => state.cloneSyncPlan)

  const schedulerSets = snapshot?.schedulerSets ?? EMPTY_SETS
  const schedulerGroups = snapshot?.schedulerGroups ?? EMPTY_GROUPS
  const syncPlanRuns = snapshot?.syncPlanRuns ?? EMPTY_RUNS
  const normalizedIntent = useMemo(() => normalizeIntent(initialIntent), [initialIntent])
  const initialPlan = pickPlanForIntent(schedulerSets, normalizedIntent)
  const initialSet = findSetById(schedulerSets, normalizedIntent.schedulerSetId)
    ?? findSetById(schedulerSets, initialPlan?.schedulerSetId)
    ?? firstActiveSet(schedulerSets)
  const [activeTab, setActiveTab] = useState<EditorTab>(() => normalizedIntent.mode === 'edit' ? 'runtime' : 'general')
  const [clonePending, setClonePending] = useState(() => normalizedIntent.mode === 'clone')
  const [selectedPlanId, setSelectedPlanId] = useState<string | undefined>(() => normalizedIntent.mode === 'new' ? undefined : initialPlan?.id)
  const [planDraft, setPlanDraft] = useState<SyncPlanUpsert>(() => {
    if (normalizedIntent.mode === 'new') {
      return createPlanDraft(initialSet?.id ?? '')
    }
    return initialPlan ? mapPlanToDraft(initialPlan) : createPlanDraft(initialSet?.id ?? '')
  })
  const [pauseInput, setPauseInput] = useState<SetSyncPlanPauseInput>(() => ({
    id: initialPlan?.id ?? '',
    pauseMode: (initialPlan?.pauseMode as SchedulerPauseMode | undefined) ?? 'disabled',
    pauseUntil: initialPlan?.pauseUntil,
  }))
  const [skipInput, setSkipInput] = useState<SkipSyncPlanInput>(() => ({
    id: initialPlan?.id ?? '',
    mode: 'default',
    minutes: 60,
  }))
  const [labelEntry, setLabelEntry] = useState('')
  const [groupEntry, setGroupEntry] = useState('')
  const [previewCount, setPreviewCount] = useState<number>()
  const [previewHandles, setPreviewHandles] = useState<string[]>([])
  const [localDateFormat, setLocalDateFormat] = useState<LocalDateFormat>(() => detectLocalDateFormat())
  const [activeCalendar, setActiveCalendar] = useState<CalendarFieldKey | null>(null)
  const [calendarMonth, setCalendarMonth] = useState(() => new Date())
  const dateFromFieldRef = useRef<HTMLDivElement | null>(null)
  const dateToFieldRef = useRef<HTMLDivElement | null>(null)
  const pauseUntilFieldRef = useRef<HTMLDivElement | null>(null)
  const skipUntilFieldRef = useRef<HTMLDivElement | null>(null)

  const selectedPlan = useMemo(
    () => findPlanById(schedulerSets, selectedPlanId) ?? pickPlanForIntent(schedulerSets, normalizedIntent),
    [normalizedIntent, schedulerSets, selectedPlanId],
  )
  const selectedSet = useMemo(
    () => findSetById(schedulerSets, planDraft.schedulerSetId || selectedPlan?.schedulerSetId)
      ?? findSetById(schedulerSets, normalizedIntent.schedulerSetId)
      ?? firstActiveSet(schedulerSets),
    [normalizedIntent.schedulerSetId, planDraft.schedulerSetId, schedulerSets, selectedPlan?.schedulerSetId],
  )
  const selectedPlanRuns = useMemo(
    () => findRunsForPlan(syncPlanRuns, selectedPlan?.id),
    [selectedPlan?.id, syncPlanRuns],
  )
  const currentPauseInput = useMemo(
    () => (selectedPlan && pauseInput.id !== selectedPlan.id
      ? { id: selectedPlan.id, pauseMode: selectedPlan.pauseMode as SchedulerPauseMode, pauseUntil: selectedPlan.pauseUntil }
      : pauseInput),
    [pauseInput, selectedPlan],
  )
  const currentSkipInput = useMemo(
    () => (selectedPlan && skipInput.id !== selectedPlan.id
      ? { ...skipInput, id: selectedPlan.id, until: selectedPlan.skipUntil }
      : skipInput),
    [selectedPlan, skipInput],
  )
  const schedulerGroupNames = useMemo(() => groupNameMap(schedulerGroups), [schedulerGroups])
  const schedulerGroupOptions = useMemo<SelectionOption[]>(
    () => schedulerGroups.map((group) => ({ id: group.id, label: group.name })),
    [schedulerGroups],
  )
  const schedulerGroupOptionsMap = useMemo(
    () => new Map(schedulerGroupOptions.map((group) => [group.id, group.label])),
    [schedulerGroupOptions],
  )
  const availableLabels = useMemo(
    () => Array.from(new Set((snapshot?.sources ?? []).flatMap((source) => source.labels).map((label) => label.trim()).filter(Boolean))).sort((left, right) => left.localeCompare(right)),
    [snapshot?.sources],
  )
  const isManualMode = planDraft.mode === 'manual'
  const notificationStyle = planDraft.notifications.simple ? 'simple' : 'detailed'
  const planStatusHint = planDraft.enabled
    ? 'This plan can run automatically when its schedule matches.'
    : 'This plan stays saved but will not run automatically.'
  const groupControlsDisabled = schedulerGroupOptions.length === 0
  const availabilitySummary = useMemo(() => {
    const active = REMOTE_STATE_TOGGLES.filter((entry) => planDraft.criteria[entry.key]).map((entry) => entry.label)
    if (active.length === 0) return 'No state'
    if (active.length === 1) return active[0]
    return countSummary(active.length, 'state', 'states')
  }, [planDraft.criteria])
  const readinessSummary = planDraft.criteria.ignoreReadyForDownload
    ? 'Gate ignored'
    : planDraft.criteria.readyForDownload
      ? 'Ready required'
      : 'Ready off'
  const rangeSummary = planDraft.criteria.dateInRange ? 'Inside range' : 'Outside range'
  const freshnessSummary = [
    planDraft.criteria.daysNumber
      ? (planDraft.criteria.daysIsDownloaded ? `Recent ${planDraft.criteria.daysNumber}d` : `Stale ${planDraft.criteria.daysNumber}d`)
      : 'No freshness limit',
    planDraft.criteria.usersCount ? `Max ${planDraft.criteria.usersCount}` : 'No cap',
  ].join(' · ')
  const labelsSummary = [
    planDraft.criteria.labelsIncluded.length > 0 ? `${planDraft.criteria.labelsIncluded.length} include` : null,
    planDraft.criteria.labelsExcluded.length > 0 ? `${planDraft.criteria.labelsExcluded.length} exclude` : null,
  ].filter(Boolean).join(' · ') || 'No label rules'
  const groupsSummary = [
    planDraft.criteria.groupIdsIncluded.length > 0 ? `${planDraft.criteria.groupIdsIncluded.length} include` : null,
    planDraft.criteria.groupIdsExcluded.length > 0 ? `${planDraft.criteria.groupIdsExcluded.length} exclude` : null,
  ].filter(Boolean).join(' · ') || 'No groups'
  const calendarWeekdays = useMemo(() => weekDayLabels(), [])
  const calendarMonthDays = useMemo(() => calendarDays(calendarMonth), [calendarMonth])
  const currentPauseUntil = currentPauseInput.pauseUntil
  const currentPauseUntilParts = useMemo(() => splitLocalDateTimeValue(currentPauseUntil), [currentPauseUntil])
  const currentSkipUntil = currentSkipInput.until
  const currentSkipUntilParts = useMemo(() => splitLocalDateTimeValue(currentSkipUntil), [currentSkipUntil])

  useEffect(() => {
    void loadSystemShortDatePattern()
      .then((pattern) => setLocalDateFormat(localDateFormatFromPattern(pattern)))
      .catch(() => undefined)
  }, [])

  useEffect(() => {
    if (!activeCalendar) {
      return
    }

    function handlePointerDown(event: MouseEvent) {
      const target = event.target
      if (!(target instanceof Node)) {
        return
      }

      const clickedInsideFrom = dateFromFieldRef.current?.contains(target) ?? false
      const clickedInsideTo = dateToFieldRef.current?.contains(target) ?? false
      const clickedInsidePause = pauseUntilFieldRef.current?.contains(target) ?? false
      const clickedInsideSkip = skipUntilFieldRef.current?.contains(target) ?? false
      if (!clickedInsideFrom && !clickedInsideTo && !clickedInsidePause && !clickedInsideSkip) {
        setActiveCalendar(null)
      }
    }

    document.addEventListener('mousedown', handlePointerDown)
    return () => document.removeEventListener('mousedown', handlePointerDown)
  }, [activeCalendar])

  useEffect(() => {
    if (!clonePending || !normalizedIntent.planId) {
      return
    }

    const cloneSource = findPlanById(schedulerSets, normalizedIntent.planId)
    if (!cloneSource) {
      return
    }

    const knownIds = new Set(schedulerSets.flatMap((entry) => entry.plans).map((entry) => entry.id))
    void cloneSyncPlan(cloneSource.id)
      .then((saved) => {
        const clonedPlan = saved.schedulerSets
          .flatMap((entry) => entry.plans)
          .find((entry) => !knownIds.has(entry.id))
        if (!clonedPlan) {
          setClonePending(false)
          return
        }
        setSelectedPlanId(clonedPlan.id)
        setPlanDraft(mapPlanToDraft(clonedPlan))
        setPauseInput({ id: clonedPlan.id, pauseMode: clonedPlan.pauseMode, pauseUntil: clonedPlan.pauseUntil })
        setSkipInput({ id: clonedPlan.id, mode: 'default', minutes: 60 })
        setClonePending(false)
      })
      .catch(() => {
        setClonePending(false)
      })
  }, [clonePending, cloneSyncPlan, normalizedIntent.planId, schedulerSets])

  function updateCriteria<K extends keyof SchedulerPlanCriteria>(key: K, value: SchedulerPlanCriteria[K]) {
    setPlanDraft((current) => ({ ...current, criteria: { ...current.criteria, [key]: value } }))
  }

  function updateNotifications<K extends keyof SchedulerPlanNotifications>(key: K, value: SchedulerPlanNotifications[K]) {
    setPlanDraft((current) => ({ ...current, notifications: { ...current.notifications, [key]: value } }))
  }

  function updateNotificationStyle(style: 'simple' | 'detailed') {
    setPlanDraft((current) => ({
      ...current,
      notifications: syncNotificationMode({
        ...current.notifications,
        simple: style === 'simple',
      }),
    }))
  }

  function toggleProviderFilter(targetKey: 'sitesIncluded' | 'sitesExcluded', provider: ProviderKey, enabled: boolean) {
    updateCriteria(
      targetKey,
      enabled
        ? trimProviderList([...planDraft.criteria[targetKey], provider])
        : planDraft.criteria[targetKey].filter((entry) => entry !== provider),
    )
  }

  function addLabelTo(targetKey: 'labelsIncluded' | 'labelsExcluded') {
    if (!labelEntry.trim()) {
      return
    }
    updateCriteria(targetKey, appendUnique(planDraft.criteria[targetKey], labelEntry))
    setLabelEntry('')
  }

  function removeLabelFrom(targetKey: 'labelsIncluded' | 'labelsExcluded', value: string) {
    updateCriteria(targetKey, removeValue(planDraft.criteria[targetKey], value))
  }

  function addGroupTo(targetKey: 'groupIdsIncluded' | 'groupIdsExcluded') {
    const normalizedGroupId = groupEntry.trim()
    if (!normalizedGroupId) {
      return
    }
    updateCriteria(targetKey, appendUnique(planDraft.criteria[targetKey], normalizedGroupId))
    setGroupEntry('')
  }

  function removeGroupFrom(targetKey: 'groupIdsIncluded' | 'groupIdsExcluded', value: string) {
    updateCriteria(targetKey, removeValue(planDraft.criteria[targetKey], value))
  }

  function handleLocalizedDateChange(
    key: 'dateFrom' | 'dateTo',
    rawValue: string,
  ) {
    if (!rawValue.trim()) {
      updateCriteria(key, undefined)
      return
    }

    const parsed = parseLocalizedDateInput(rawValue, localDateFormat)
    if (parsed) {
      updateCriteria(key, parsed)
    }
  }

  function normalizeLocalizedDateInput(
    key: 'dateFrom' | 'dateTo',
    input: HTMLInputElement,
  ) {
    const rawValue = input.value
    const parsed = parseLocalizedDateInput(rawValue, localDateFormat)
    if (!rawValue.trim()) {
      input.value = ''
      updateCriteria(key, undefined)
      return
    }
    if (!parsed) {
      return
    }

    updateCriteria(key, parsed)
    input.value = formatIsoDateForLocale(parsed, localDateFormat)
  }

  function updatePauseUntil(dateValue?: string, timeValue?: string) {
    setPauseInput((current) => {
      const currentValue = current.id === (selectedPlan?.id ?? '') ? current.pauseUntil : selectedPlan?.pauseUntil
      const currentParts = splitLocalDateTimeValue(currentValue)
      return {
        ...current,
        id: selectedPlan?.id ?? '',
        pauseMode: 'until',
        pauseUntil: buildLocalDateTimeValue(
          dateValue !== undefined ? dateValue : currentParts.date,
          timeValue !== undefined ? timeValue : currentParts.time,
        ),
      }
    })
  }

  function updateSkipUntil(dateValue?: string, timeValue?: string) {
    setSkipInput((current) => {
      const currentValue = current.id === (selectedPlan?.id ?? '') ? current.until : selectedPlan?.skipUntil
      const currentParts = splitLocalDateTimeValue(currentValue)
      return {
        ...current,
        id: selectedPlan?.id ?? '',
        mode: 'until',
        until: buildLocalDateTimeValue(
          dateValue !== undefined ? dateValue : currentParts.date,
          timeValue !== undefined ? timeValue : currentParts.time,
        ),
      }
    })
  }

  function handleRuntimeLocalizedDateChange(kind: 'pause' | 'skip', rawValue: string) {
    if (!rawValue.trim()) {
      if (kind === 'pause') {
        updatePauseUntil(undefined, currentPauseUntilParts.time)
      } else {
        updateSkipUntil(undefined, currentSkipUntilParts.time)
      }
      return
    }

    const parsed = parseLocalizedDateInput(rawValue, localDateFormat)
    if (!parsed) {
      return
    }

    if (kind === 'pause') {
      updatePauseUntil(parsed, currentPauseUntilParts.time)
    } else {
      updateSkipUntil(parsed, currentSkipUntilParts.time)
    }
  }

  function normalizeRuntimeLocalizedDateInput(kind: 'pause' | 'skip', input: HTMLInputElement) {
    const parsed = parseLocalizedDateInput(input.value, localDateFormat)
    if (!input.value.trim()) {
      input.value = ''
      if (kind === 'pause') {
        updatePauseUntil(undefined, currentPauseUntilParts.time)
      } else {
        updateSkipUntil(undefined, currentSkipUntilParts.time)
      }
      return
    }
    if (!parsed) {
      return
    }

    input.value = formatIsoDateForLocale(parsed, localDateFormat)
    if (kind === 'pause') {
      updatePauseUntil(parsed, currentPauseUntilParts.time)
    } else {
      updateSkipUntil(parsed, currentSkipUntilParts.time)
    }
  }

  function openCalendar(key: CalendarFieldKey) {
    const currentValue = key === 'dateFrom'
      ? planDraft.criteria.dateFrom
      : key === 'dateTo'
        ? planDraft.criteria.dateTo
        : key === 'pauseUntilDate'
          ? currentPauseUntilParts.date
          : currentSkipUntilParts.date
    const parts = parseIsoDateParts(currentValue)
    setCalendarMonth(parts ? new Date(parts.year, parts.month - 1, 1) : new Date())
    setActiveCalendar(key)
  }

  function selectCalendarDate(key: CalendarFieldKey, value: Date) {
    const isoValue = isoDateFromCalendarDate(value)
    if (key === 'dateFrom' || key === 'dateTo') {
      updateCriteria(key, isoValue)
    } else if (key === 'pauseUntilDate') {
      updatePauseUntil(isoValue, currentPauseUntilParts.time)
    } else {
      updateSkipUntil(isoValue, currentSkipUntilParts.time)
    }
    setActiveCalendar(null)
    setCalendarMonth(new Date(value.getFullYear(), value.getMonth(), 1))
  }

  async function handlePreview() {
    const preview = await previewSyncPlanTarget({
      schedulerSetId: planDraft.schedulerSetId,
      planId: planDraft.id,
      criteria: planDraft.criteria,
    })
    setPreviewCount(preview.sourceCount)
    setPreviewHandles(preview.sources.slice(0, 12).map((entry) => `${entry.provider}:${entry.handle}`))
  }

  async function handleSave(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const notifications = syncNotificationMode(planDraft.notifications)
    const payload: SyncPlanUpsert = {
      ...planDraft,
      name: planDraft.name.trim(),
      schedulerSetId: planDraft.schedulerSetId || selectedSet?.id || '',
      notificationMode: notifications.simple ? 'summary' : 'detailed',
      targetFilter: planDraft.criteria.advancedExpression?.trim() ?? '',
      notifications,
    }
    const saved = await upsertSyncPlan(payload)
    const next = saved.schedulerSets.flatMap((entry) => entry.plans).find((entry) => entry.id === payload.id)
      ?? saved.schedulerSets.flatMap((entry) => entry.plans).find((entry) => entry.name === payload.name)
    if (!next) {
      return
    }
    setSelectedPlanId(next.id)
    setPlanDraft(mapPlanToDraft(next))
    setPauseInput({ id: next.id, pauseMode: next.pauseMode, pauseUntil: next.pauseUntil })
    setSkipInput({ id: next.id, mode: 'default', minutes: 60 })
    setActiveTab('runtime')
  }

  if (!snapshot) {
    return <div className="plans-window-empty panel">Loading plan editor...</div>
  }

  return (
    <div className="plans-editor-shell">
      <header className="plans-editor-header">
        <div>
          <h1 className="plans-editor-plan-title">
            {planDraft.name.trim() || (normalizedIntent.mode === 'new' ? 'New plan' : selectedPlan?.name ?? 'Plan')}
          </h1>
        </div>
        <div className="plans-editor-header-meta">
          <span className="pill">{selectedSet?.name ?? 'No set'}</span>
          <span className={`pill pill-${selectedPlan?.lastRunStatus ?? 'idle'}`}>{selectedPlan ? runtimeStateLabel(selectedPlan) : normalizedIntent.mode}</span>
        </div>
      </header>

      <div className="settings-tab-bar plans-tab-bar" role="tablist" aria-label="Plan editor tabs">
        {[
          { key: 'general', label: 'General' },
          { key: 'filters', label: 'Filters' },
          { key: 'runtime', label: 'Runtime' },
        ].map((tab) => {
          const isActive = activeTab === tab.key
          return (
            <button
              key={tab.key}
              aria-controls={`plans-tab-${tab.key}`}
              aria-selected={isActive}
              className={isActive ? 'settings-tab settings-tab-active' : 'settings-tab'}
              id={`plans-tab-button-${tab.key}`}
              onClick={() => setActiveTab(tab.key as EditorTab)}
              role="tab"
              type="button"
            >
              <span>{tab.label}</span>
            </button>
          )
        })}
      </div>

      <form className="plans-editor-body panel panel-accent settings-tab-panel" onSubmit={(event) => void handleSave(event)}>
        {activeTab === 'general' ? (
          <section className="plans-editor-tab" aria-labelledby="plans-tab-button-general" id="plans-tab-general" role="tabpanel">
            <article className="panel plans-section-card plans-status-card">
              <div className="panel-header compact-header">
                <div>
                  <p className="eyebrow">Plan status</p>
                  <h2>Execution state</h2>
                </div>
                <span className={`pill pill-${selectedPlan?.lastRunStatus ?? 'idle'}`}>{selectedPlan ? runtimeStateLabel(selectedPlan) : 'draft'}</span>
              </div>
              <div className="plans-status-toggle" role="group" aria-label="Plan status">
                <button
                  aria-pressed={planDraft.enabled}
                  className={planDraft.enabled ? 'plans-status-button plans-status-button-selected plans-status-button-enabled' : 'plans-status-button'}
                  onClick={() => setPlanDraft((current) => ({ ...current, enabled: true }))}
                  type="button"
                >
                  Enabled
                </button>
                <button
                  aria-pressed={!planDraft.enabled}
                  className={!planDraft.enabled ? 'plans-status-button plans-status-button-selected plans-status-button-disabled' : 'plans-status-button'}
                  onClick={() => setPlanDraft((current) => ({ ...current, enabled: false }))}
                  type="button"
                >
                  Disabled
                </button>
              </div>
              <p className="plans-help-text">{planStatusHint}</p>
            </article>

            <article className="panel plans-section-card">
              <div className="panel-header compact-header">
                <div>
                  <p className="eyebrow">Configuration</p>
                  <h2>Identity &amp; schedule</h2>
                </div>
              </div>
              <div className="plans-grid-two">
                <label className="field">
                  <span>Name</span>
                  <input type="text" value={planDraft.name} onChange={(event) => setPlanDraft((current) => ({ ...current, name: event.target.value }))} />
                </label>
                <label className="field">
                  <span>Scheduler set</span>
                  <select value={planDraft.schedulerSetId || selectedSet?.id || ''} onChange={(event) => setPlanDraft((current) => ({ ...current, schedulerSetId: event.target.value }))}>
                    {schedulerSets.map((entry) => <option key={entry.id} value={entry.id}>{entry.name}</option>)}
                  </select>
                </label>
                <label className="field">
                  <span>Mode</span>
                  <select value={planDraft.mode} onChange={(event) => setPlanDraft((current) => ({ ...current, mode: event.target.value as SyncPlanUpsert['mode'] }))}>
                    <option value="automatic">Automatic</option>
                    <option value="manual">Manual</option>
                  </select>
                </label>
              </div>
              {isManualMode ? (
                <p className="plans-help-text">Runs only when started manually from the Runtime tab using Start or Start (force).</p>
              ) : (
                <>
                  <div className="plans-grid-two">
                    <label className="field">
                      <span>Run every (minutes)</span>
                      <input type="number" value={planDraft.intervalMinutes} onChange={(event) => setPlanDraft((current) => ({ ...current, intervalMinutes: Number(event.target.value) }))} />
                    </label>
                    <label className="field">
                      <span>Initial delay after app start (minutes)</span>
                      <input type="number" value={planDraft.startupDelayMinutes} onChange={(event) => setPlanDraft((current) => ({ ...current, startupDelayMinutes: Number(event.target.value) }))} />
                    </label>
                  </div>
                  <p className="plans-help-text">The initial delay only affects the first automatic run after opening the app.</p>
                </>
              )}
            </article>

            <article className="panel plans-section-card">
              <div className="panel-header compact-header">
                <div>
                  <p className="eyebrow">Notifications</p>
                  <h2>Desktop alerts</h2>
                </div>
              </div>
              <div className="plans-grid-two">
                <label className="toggle-card plans-secondary-toggle">
                  <input checked={planDraft.notifications.enabled} onChange={(event) => updateNotifications('enabled', checkbox(event))} type="checkbox" />
                  <span>Show notifications</span>
                </label>
              </div>
              {planDraft.notifications.enabled ? (
                <>
                  <div className="plans-grid-two">
                    <label className="field">
                      <span>Notification style</span>
                      <select value={notificationStyle} onChange={(event) => updateNotificationStyle(event.target.value as 'simple' | 'detailed')}>
                        <option value="simple">Simple</option>
                        <option value="detailed">Detailed</option>
                      </select>
                    </label>
                  </div>
                  {notificationStyle === 'detailed' ? (
                    <div className="plans-toggle-grid plans-toggle-grid-tight">
                      <label className="toggle-card plans-secondary-toggle"><input checked={planDraft.notifications.showImage} onChange={(event) => updateNotifications('showImage', checkbox(event))} type="checkbox" /><span>Include preview image</span></label>
                      <label className="toggle-card plans-secondary-toggle"><input checked={planDraft.notifications.showUserIcon} onChange={(event) => updateNotifications('showUserIcon', checkbox(event))} type="checkbox" /><span>Include user icon</span></label>
                    </div>
                  ) : null}
                </>
              ) : (
                <p className="plans-help-text">Notification details stay hidden until desktop alerts are enabled.</p>
              )}
              <div className="plans-workspace-default">
                <p className="plans-help-text">
                  Workspace default for new plans (summary vs detailed). Existing plans keep their own style above.
                </p>
                <PlanWorkspaceNotificationDefault />
              </div>
            </article>

            <article className="panel plans-section-card">
              <div className="panel-header compact-header">
                <div>
                  <p className="eyebrow">Summary</p>
                  <h2>Runtime facts</h2>
                </div>
              </div>
              <div className="plans-inline-summary">
                <div><strong>Last download date</strong><span>{selectedPlan?.lastRunAt ?? 'never'}</span></div>
                <div><strong>Next run</strong><span>{selectedPlan?.nextDueAt ?? 'pending'}</span></div>
                <div><strong>State</strong><span>{selectedPlan ? statusLabel(selectedPlan) : 'draft'}</span></div>
              </div>
            </article>
          </section>
        ) : null}

        {activeTab === 'filters' ? (
          <section className="plans-editor-tab" aria-labelledby="plans-tab-button-filters" id="plans-tab-filters" role="tabpanel">
            <div className="plans-filter-layout">
              <div className="plans-filter-grid">
                <article className="panel plans-section-card plans-filter-card plans-filter-card-compact">
                  <div className="panel-header compact-header">
                    <h2 className="plans-filter-heading">Availability</h2>
                    <span className="pill plans-filter-summary-pill">{availabilitySummary}</span>
                  </div>
                  <div className="plans-toggle-grid plans-toggle-grid-tight">
                    {REMOTE_STATE_TOGGLES.map((entry) => (
                      <label className={`toggle-card plans-secondary-toggle ${entry.tone ? `plans-filter-toggle-${entry.tone}` : ''}`} key={entry.key}>
                        <input checked={Boolean(planDraft.criteria[entry.key])} onChange={(event) => updateCriteria(entry.key, checkbox(event))} type="checkbox" />
                        <span className="plans-toggle-label">{entry.label}{helpButton(entry.label, entry.tooltip)}</span>
                      </label>
                    ))}
                  </div>
                </article>

                <article className="panel plans-section-card plans-filter-card plans-filter-card-compact">
                  <div className="panel-header compact-header">
                    <h2 className="plans-filter-heading">Readiness</h2>
                    <span className="pill plans-filter-summary-pill">{readinessSummary}</span>
                  </div>
                  <div className="plans-toggle-grid plans-toggle-grid-tight">
                    {DOWNLOAD_GATE_TOGGLES.map((entry) => (
                      <label className="toggle-card plans-secondary-toggle" key={entry.key}>
                        <input checked={Boolean(planDraft.criteria[entry.key])} onChange={(event) => updateCriteria(entry.key, checkbox(event))} type="checkbox" />
                        <span className="plans-toggle-label">{entry.label}{helpButton(entry.label, entry.tooltip)}</span>
                      </label>
                    ))}
                  </div>
                </article>

              </div>

              <article className="panel plans-section-card plans-filter-card plans-filter-card-condensed">
                <div className="panel-header compact-header">
                  <h2 className="plans-filter-heading">Date range {helpButton('Date range', 'When enabled, the source date must fall inside the interval. When disabled, the source date must stay outside the interval.')}</h2>
                  <span className="pill plans-filter-summary-pill" title={planDraft.criteria.dateInRange ? 'Keeps dates inside the selected interval.' : 'Keeps dates outside the selected interval.'}>{rangeSummary}</span>
                </div>
                <div className="plans-filter-inline-grid plans-filter-inline-grid-date">
                  <label className="toggle-card plans-secondary-toggle plans-filter-inline-toggle">
                    <input checked={planDraft.criteria.dateInRange} onChange={(event) => updateCriteria('dateInRange', checkbox(event))} type="checkbox" />
                    <span className="plans-toggle-label">In range {helpButton('In range', 'Matches inside the interval when checked, or outside the interval when unchecked.')}</span>
                  </label>
                  <label className="field">
                    <span>From</span>
                    <div className="plans-date-field-shell" ref={dateFromFieldRef}>
                      <div className="plans-date-input-row">
                        <input
                          defaultValue={formatIsoDateForLocale(planDraft.criteria.dateFrom, localDateFormat)}
                          inputMode="numeric"
                          key={`date-from-${planDraft.criteria.dateFrom ?? 'empty'}`}
                          placeholder={localDateFormat.placeholder}
                          type="text"
                          onBlur={(event) => normalizeLocalizedDateInput('dateFrom', event.currentTarget)}
                          onChange={(event) => handleLocalizedDateChange('dateFrom', event.target.value)}
                        />
                        <button
                          aria-expanded={activeCalendar === 'dateFrom'}
                          aria-label="Pick From date"
                          className="ghost-button plans-date-picker-button"
                          onClick={() => openCalendar('dateFrom')}
                          type="button"
                        >
                          <CalendarIcon />
                        </button>
                      </div>
                      {activeCalendar === 'dateFrom' ? (
                        <div className="panel plans-date-picker-popover" role="dialog" aria-label="From date calendar">
                          <div className="plans-date-picker-header">
                            <button aria-label="Previous month" className="ghost-button plans-date-picker-nav" onClick={() => setCalendarMonth((current) => shiftMonth(current, -1))} type="button">‹</button>
                            <strong>{monthLabel(calendarMonth)}</strong>
                            <button aria-label="Next month" className="ghost-button plans-date-picker-nav" onClick={() => setCalendarMonth((current) => shiftMonth(current, 1))} type="button">›</button>
                          </div>
                          <div className="plans-date-picker-weekdays">
                            {calendarWeekdays.map((weekday) => <span key={`from-weekday-${weekday}`}>{weekday}</span>)}
                          </div>
                          <div className="plans-date-picker-grid">
                            {calendarMonthDays.map((day) => {
                              const isoValue = isoDateFromCalendarDate(day)
                              const isCurrentMonth = day.getMonth() === calendarMonth.getMonth()
                              const isSelected = isoValue === planDraft.criteria.dateFrom
                              const isToday = isoValue === isoDateFromCalendarDate(new Date())
                              return (
                                <button
                                  aria-label={`Choose ${isoValue}`}
                                  className={`plans-date-picker-day${isCurrentMonth ? '' : ' is-outside'}${isSelected ? ' is-selected' : ''}${isToday ? ' is-today' : ''}`}
                                  key={`from-day-${isoValue}`}
                                  onClick={() => selectCalendarDate('dateFrom', day)}
                                  type="button"
                                >
                                  {day.getDate()}
                                </button>
                              )
                            })}
                          </div>
                        </div>
                      ) : null}
                    </div>
                  </label>
                  <label className="field">
                    <span>To</span>
                    <div className="plans-date-field-shell" ref={dateToFieldRef}>
                      <div className="plans-date-input-row">
                        <input
                          defaultValue={formatIsoDateForLocale(planDraft.criteria.dateTo, localDateFormat)}
                          inputMode="numeric"
                          key={`date-to-${planDraft.criteria.dateTo ?? 'empty'}`}
                          placeholder={localDateFormat.placeholder}
                          type="text"
                          onBlur={(event) => normalizeLocalizedDateInput('dateTo', event.currentTarget)}
                          onChange={(event) => handleLocalizedDateChange('dateTo', event.target.value)}
                        />
                        <button
                          aria-expanded={activeCalendar === 'dateTo'}
                          aria-label="Pick To date"
                          className="ghost-button plans-date-picker-button"
                          onClick={() => openCalendar('dateTo')}
                          type="button"
                        >
                          <CalendarIcon />
                        </button>
                      </div>
                      {activeCalendar === 'dateTo' ? (
                        <div className="panel plans-date-picker-popover" role="dialog" aria-label="To date calendar">
                          <div className="plans-date-picker-header">
                            <button aria-label="Previous month" className="ghost-button plans-date-picker-nav" onClick={() => setCalendarMonth((current) => shiftMonth(current, -1))} type="button">‹</button>
                            <strong>{monthLabel(calendarMonth)}</strong>
                            <button aria-label="Next month" className="ghost-button plans-date-picker-nav" onClick={() => setCalendarMonth((current) => shiftMonth(current, 1))} type="button">›</button>
                          </div>
                          <div className="plans-date-picker-weekdays">
                            {calendarWeekdays.map((weekday) => <span key={`to-weekday-${weekday}`}>{weekday}</span>)}
                          </div>
                          <div className="plans-date-picker-grid">
                            {calendarMonthDays.map((day) => {
                              const isoValue = isoDateFromCalendarDate(day)
                              const isCurrentMonth = day.getMonth() === calendarMonth.getMonth()
                              const isSelected = isoValue === planDraft.criteria.dateTo
                              const isToday = isoValue === isoDateFromCalendarDate(new Date())
                              return (
                                <button
                                  aria-label={`Choose ${isoValue}`}
                                  className={`plans-date-picker-day${isCurrentMonth ? '' : ' is-outside'}${isSelected ? ' is-selected' : ''}${isToday ? ' is-today' : ''}`}
                                  key={`to-day-${isoValue}`}
                                  onClick={() => selectCalendarDate('dateTo', day)}
                                  type="button"
                                >
                                  {day.getDate()}
                                </button>
                              )
                            })}
                          </div>
                        </div>
                      ) : null}
                    </div>
                  </label>
                </div>
              </article>

              <article className="panel plans-section-card plans-filter-card plans-filter-card-condensed">
                <div className="panel-header compact-header">
                  <h2 className="plans-filter-heading">Freshness &amp; limits {helpButton('Freshness & limits', 'Matches sources by their last synced date. Checked means recently downloaded; unchecked means stale or not yet synced.')}</h2>
                  <span className="pill plans-filter-summary-pill" title="Combines the freshness rule and the optional users cap.">{freshnessSummary}</span>
                </div>
                <div className="plans-filter-inline-grid plans-filter-inline-grid-freshness">
                  <label className="toggle-card plans-secondary-toggle plans-filter-inline-toggle">
                    <input checked={planDraft.criteria.daysIsDownloaded} onChange={(event) => updateCriteria('daysIsDownloaded', checkbox(event))} type="checkbox" />
                    <span className="plans-toggle-label">
                      {planDraft.criteria.daysIsDownloaded ? 'Downloaded recently' : 'Not downloaded recently'}
                      {helpButton('Freshness direction', 'The checkbox changes the direction of the freshness test instead of changing which date field is used.')}
                    </span>
                  </label>
                  <label className="field">
                    <span>Days</span>
                    <input type="number" value={planDraft.criteria.daysNumber ?? ''} onChange={(event) => updateCriteria('daysNumber', event.target.value ? Number(event.target.value) : undefined)} />
                  </label>
                  <label className="field">
                    <span>Users cap</span>
                    <input type="number" value={planDraft.criteria.usersCount ?? ''} onChange={(event) => updateCriteria('usersCount', event.target.value ? Number(event.target.value) : undefined)} />
                  </label>
                  <button className="ghost-button" onClick={() => updateCriteria('daysNumber', undefined)} type="button">Reset</button>
                </div>
              </article>

              <article className="panel plans-section-card plans-filter-card">
                <div className="panel-header compact-header">
                  <h2 className="plans-filter-heading">Labels</h2>
                  <span className="pill plans-filter-summary-pill">{labelsSummary}</span>
                </div>
                <div className="plans-toggle-grid plans-toggle-grid-tight">
                  <label className="toggle-card plans-secondary-toggle">
                    <input checked={planDraft.criteria.labelsNo} onChange={(event) => updateCriteria('labelsNo', checkbox(event))} type="checkbox" />
                    <span className="plans-toggle-label">No labels</span>
                  </label>
                  <label className="toggle-card plans-secondary-toggle">
                    <input checked={planDraft.criteria.ignoreExcludedLabels} onChange={(event) => updateCriteria('ignoreExcludedLabels', checkbox(event))} type="checkbox" />
                    <span className="plans-toggle-label">Ignore excluded labels {helpButton('Ignore excluded labels', 'Skips the exclusion list when the rest of the filter already matches.')}</span>
                  </label>
                </div>
                <div className="plans-composer-toolbar">
                  <label className="field field-full">
                    <span>Label entry</span>
                    <input list="scheduler-label-suggestions" type="text" value={labelEntry} onChange={(event) => setLabelEntry(event.target.value)} />
                    <datalist id="scheduler-label-suggestions">
                      {availableLabels.map((label) => <option key={label} value={label} />)}
                    </datalist>
                  </label>
                  <div className="action-row">
                    <button className="ghost-button" onClick={() => addLabelTo('labelsIncluded')} type="button">Include</button>
                    <button className="ghost-button" onClick={() => addLabelTo('labelsExcluded')} type="button">Exclude</button>
                    <button className="ghost-button" onClick={() => { updateCriteria('labelsIncluded', []); updateCriteria('labelsExcluded', []); }} type="button">Clear</button>
                  </div>
                </div>
                <div className="plans-composer-summary">
                  <div><strong>Include</strong><span>{summarizeSelection(planDraft.criteria.labelsIncluded)}</span></div>
                  <div><strong>Exclude</strong><span>{summarizeSelection(planDraft.criteria.labelsExcluded)}</span></div>
                </div>
                {planDraft.criteria.labelsIncluded.length > 0 ? (
                  <div className="plans-chip-list">
                    {planDraft.criteria.labelsIncluded.map((label) => (
                      <button className="pill" key={`label-include-${label}`} onClick={() => removeLabelFrom('labelsIncluded', label)} type="button">{label} ×</button>
                    ))}
                  </div>
                ) : null}
                {planDraft.criteria.labelsExcluded.length > 0 ? (
                  <div className="plans-chip-list">
                    {planDraft.criteria.labelsExcluded.map((label) => (
                      <button className="pill" key={`label-exclude-${label}`} onClick={() => removeLabelFrom('labelsExcluded', label)} type="button">{label} ×</button>
                    ))}
                  </div>
                ) : null}
              </article>

              <article className="panel plans-section-card plans-filter-card">
                <div className="panel-header compact-header">
                  <h2 className="plans-filter-heading">Providers {helpButton('Providers', 'Check the providers this plan should include. Leave all unchecked to include every provider.')}</h2>
                  <span className="pill plans-filter-summary-pill">{summarizeSelection(planDraft.criteria.sitesIncluded, 'All providers')}</span>
                </div>
                <div className="plans-checkbox-column plans-provider-include-row">
                  {PROVIDERS.map((provider) => (
                    <label className="toggle-card plans-secondary-toggle" key={`provider-include-${provider}`}>
                      <input checked={planDraft.criteria.sitesIncluded.includes(provider)} onChange={(event) => toggleProviderFilter('sitesIncluded', provider, event.target.checked)} type="checkbox" />
                      <span>{provider}</span>
                    </label>
                  ))}
                </div>
              </article>

              <article className="panel plans-section-card plans-filter-card">
                <div className="panel-header compact-header">
                  <h2 className="plans-filter-heading">Scheduler groups {helpButton('Scheduler groups', 'Include = the source must belong to at least one of these groups. Exclude = drop sources that belong to any of these groups. Group rules are combined with the other filters (e.g. provider), so Twitter + Favorito matches only the Twitter profiles in Favorito.')}</h2>
                  <span className="pill plans-filter-summary-pill">{groupsSummary}</span>
                </div>
                {groupControlsDisabled ? (
                  <p className="plans-help-text">No scheduler groups are available yet. Create a group first to use this filter block.</p>
                ) : null}
                <div className="plans-composer-toolbar">
                  <label className="field field-full">
                    <span>Group entry</span>
                    <select disabled={groupControlsDisabled} value={groupEntry} onChange={(event) => setGroupEntry(event.target.value)}>
                      <option value="">Select a group</option>
                      {schedulerGroupOptions.map((group) => <option key={group.id} value={group.id}>{group.label}</option>)}
                    </select>
                  </label>
                  <div className="action-row">
                    <button className="ghost-button" disabled={groupControlsDisabled} onClick={() => addGroupTo('groupIdsIncluded')} type="button">Include</button>
                    <button className="ghost-button" disabled={groupControlsDisabled} onClick={() => addGroupTo('groupIdsExcluded')} type="button">Exclude</button>
                    <button className="ghost-button" disabled={groupControlsDisabled} onClick={() => { updateCriteria('groupIdsIncluded', []); updateCriteria('groupIdsExcluded', []); }} type="button">Clear</button>
                  </div>
                </div>
                <div className="plans-composer-summary">
                  <div><strong>Include</strong><span>{selectionSummary(planDraft.criteria.groupIdsIncluded, schedulerGroupOptionsMap)}</span></div>
                  <div><strong>Exclude</strong><span>{selectionSummary(planDraft.criteria.groupIdsExcluded, schedulerGroupOptionsMap, 'None')}</span></div>
                </div>
                {planDraft.criteria.groupIdsIncluded.length > 0 ? (
                  <div className="plans-chip-list">
                    {planDraft.criteria.groupIdsIncluded.map((groupId) => (
                      <button className="pill" key={`group-include-${groupId}`} onClick={() => removeGroupFrom('groupIdsIncluded', groupId)} type="button">{schedulerGroupNames.get(groupId) ?? groupId} ×</button>
                    ))}
                  </div>
                ) : null}
                {planDraft.criteria.groupIdsExcluded.length > 0 ? (
                  <div className="plans-chip-list">
                    {planDraft.criteria.groupIdsExcluded.map((groupId) => (
                      <button className="pill" key={`group-exclude-${groupId}`} onClick={() => removeGroupFrom('groupIdsExcluded', groupId)} type="button">{schedulerGroupNames.get(groupId) ?? groupId} ×</button>
                    ))}
                  </div>
                ) : null}
              </article>

              <article className="panel plans-section-card plans-filter-card">
                <div className="panel-header compact-header">
                  <h2 className="plans-filter-heading">Expression {helpButton('Advanced expression', 'Optional clause filter for provider, label, handle, account, kind, state or subscription.')}</h2>
                </div>
                <label className="field">
                  <span>Advanced expression</span>
                  <textarea rows={5} value={planDraft.criteria.advancedExpression ?? ''} onChange={(event) => updateCriteria('advancedExpression', event.target.value)} />
                </label>
              </article>
            </div>
          </section>
        ) : null}

        {activeTab === 'runtime' ? (
          <section className="plans-editor-tab" aria-labelledby="plans-tab-button-runtime" id="plans-tab-runtime" role="tabpanel">
            <div className="plans-runtime-grid">
              <article className="panel plans-runtime-card">
                <div className="panel-header compact-header">
                  <h2 className="plans-filter-heading">Temporary overrides</h2>
                  <span className={`pill pill-${selectedPlan?.lastRunStatus ?? 'idle'}`}>{selectedPlan ? runtimeStateLabel(selectedPlan) : 'draft'}</span>
                </div>
                <div className="plans-runtime-stack">
                  <section className="plans-runtime-subsection">
                    <div className="plans-runtime-subsection-header">
                      <strong>Pause execution</strong>
                    </div>
                    <div className="plans-grid-two">
                      <label className="field">
                        <span>Pause preset</span>
                        <select
                          value={currentPauseInput.pauseMode}
                          onChange={(event) => {
                            const nextMode = event.target.value as SchedulerPauseMode
                            setPauseInput((current) => ({
                              ...current,
                              id: selectedPlan?.id ?? '',
                              pauseMode: nextMode,
                              pauseUntil: nextMode === 'until'
                                ? buildLocalDateTimeValue(currentPauseUntilParts.date, currentPauseUntilParts.time)
                                : undefined,
                            }))
                          }}
                        >
                          {PAUSE_PRESETS.map((entry) => <option key={entry} value={entry}>{entry}</option>)}
                        </select>
                      </label>
                    </div>
                    {currentPauseInput.pauseMode === 'until' ? (
                      <div className="plans-grid-two">
                        <div className="field plans-date-picker-field" ref={pauseUntilFieldRef}>
                          <span>Pause until</span>
                          <div className="plans-date-picker-control">
                            <input
                              defaultValue={formatIsoDateForLocale(currentPauseUntilParts.date, localDateFormat)}
                              onBlur={(event) => normalizeRuntimeLocalizedDateInput('pause', event.currentTarget)}
                              onChange={(event) => handleRuntimeLocalizedDateChange('pause', event.target.value)}
                              placeholder={localDateFormat.placeholder}
                            />
                            <button
                              aria-expanded={activeCalendar === 'pauseUntilDate'}
                              aria-label="Pick Pause until date"
                              className="ghost-button plans-date-picker-button"
                              onClick={() => openCalendar('pauseUntilDate')}
                              type="button"
                            >
                              <CalendarIcon />
                            </button>
                          </div>
                          {activeCalendar === 'pauseUntilDate' ? (
                            <div aria-label="Pause until date calendar" className="plans-date-picker-popover" role="dialog">
                              <div className="plans-date-picker-header">
                                <button aria-label="Previous month" className="ghost-button plans-date-picker-nav" onClick={() => setCalendarMonth((current) => shiftMonth(current, -1))} type="button">‹</button>
                                <strong>{monthLabel(calendarMonth)}</strong>
                                <button aria-label="Next month" className="ghost-button plans-date-picker-nav" onClick={() => setCalendarMonth((current) => shiftMonth(current, 1))} type="button">›</button>
                              </div>
                              <div className="plans-date-picker-weekdays">
                                {calendarWeekdays.map((label) => <span key={`pause-weekday-${label}`}>{label}</span>)}
                              </div>
                              <div className="plans-date-picker-grid">
                                {calendarMonthDays.map((day) => {
                                  const isoValue = isoDateFromCalendarDate(day)
                                  const isCurrentMonth = day.getMonth() === calendarMonth.getMonth()
                                  const isSelected = currentPauseUntilParts.date === isoValue
                                  const isToday = isoValue === isoDateFromCalendarDate(new Date())
                                  return (
                                    <button
                                      className={`plans-date-picker-day${isCurrentMonth ? '' : ' is-outside'}${isSelected ? ' is-selected' : ''}${isToday ? ' is-today' : ''}`}
                                      key={`pause-day-${isoValue}`}
                                      onClick={() => selectCalendarDate('pauseUntilDate', day)}
                                      type="button"
                                    >
                                      {day.getDate()}
                                    </button>
                                  )
                                })}
                              </div>
                            </div>
                          ) : null}
                        </div>
                        <label className="field">
                          <span>Pause time</span>
                          <input
                            inputMode="numeric"
                            onBlur={(event) => {
                              const normalized = normalizeTimeInput(event.target.value)
                              event.target.value = normalized ?? currentPauseUntilParts.time
                              updatePauseUntil(currentPauseUntilParts.date, normalized ?? currentPauseUntilParts.time)
                            }}
                            onChange={(event) => updatePauseUntil(currentPauseUntilParts.date, event.target.value)}
                            placeholder="00:00"
                            value={currentPauseUntilParts.time}
                          />
                        </label>
                      </div>
                    ) : null}
                    <div className="action-row">
                      <button className="ghost-button" disabled={!selectedPlan || (currentPauseInput.pauseMode === 'until' && !currentPauseInput.pauseUntil)} onClick={() => selectedPlan && void setSyncPlanPause({ ...currentPauseInput, id: selectedPlan.id })} type="button">Apply pause</button>
                      <button className="ghost-button" disabled={!selectedPlan} onClick={() => selectedPlan && void clearSyncPlanPause(selectedPlan.id)} type="button">Clear pause</button>
                    </div>
                  </section>

                  <section className="plans-runtime-subsection">
                    <div className="plans-runtime-subsection-header">
                      <strong>Skip next window</strong>
                    </div>
                    <div className="plans-grid-two">
                      <label className="field">
                        <span>Skip mode</span>
                        <select value={currentSkipInput.mode} onChange={(event) => setSkipInput((current) => ({ ...current, id: selectedPlan?.id ?? '', mode: event.target.value as SkipSyncPlanInput['mode'] }))}>
                          <option value="default">default</option>
                          <option value="minutes">minutes</option>
                          <option value="until">until</option>
                          <option value="reset">reset</option>
                        </select>
                      </label>
                      {currentSkipInput.mode === 'minutes' ? (
                        <label className="field">
                          <span>Skip minutes</span>
                          <input type="number" value={currentSkipInput.minutes ?? ''} onChange={(event) => setSkipInput((current) => ({ ...current, id: selectedPlan?.id ?? '', mode: 'minutes', minutes: event.target.value ? Number(event.target.value) : undefined }))} />
                        </label>
                      ) : (
                        <div className="field plans-runtime-placeholder-field">
                          <span>Skip minutes</span>
                          <div className="plans-runtime-placeholder">Used only in `minutes` mode.</div>
                        </div>
                      )}
                    </div>
                    {currentSkipInput.mode === 'until' ? (
                      <div className="plans-grid-two">
                        <div className="field plans-date-picker-field" ref={skipUntilFieldRef}>
                          <span>Skip until</span>
                          <div className="plans-date-picker-control">
                            <input
                              defaultValue={formatIsoDateForLocale(currentSkipUntilParts.date, localDateFormat)}
                              onBlur={(event) => normalizeRuntimeLocalizedDateInput('skip', event.currentTarget)}
                              onChange={(event) => handleRuntimeLocalizedDateChange('skip', event.target.value)}
                              placeholder={localDateFormat.placeholder}
                            />
                            <button
                              aria-expanded={activeCalendar === 'skipUntilDate'}
                              aria-label="Pick Skip until date"
                              className="ghost-button plans-date-picker-button"
                              onClick={() => openCalendar('skipUntilDate')}
                              type="button"
                            >
                              <CalendarIcon />
                            </button>
                          </div>
                          {activeCalendar === 'skipUntilDate' ? (
                            <div aria-label="Skip until date calendar" className="plans-date-picker-popover" role="dialog">
                              <div className="plans-date-picker-header">
                                <button aria-label="Previous month" className="ghost-button plans-date-picker-nav" onClick={() => setCalendarMonth((current) => shiftMonth(current, -1))} type="button">‹</button>
                                <strong>{monthLabel(calendarMonth)}</strong>
                                <button aria-label="Next month" className="ghost-button plans-date-picker-nav" onClick={() => setCalendarMonth((current) => shiftMonth(current, 1))} type="button">›</button>
                              </div>
                              <div className="plans-date-picker-weekdays">
                                {calendarWeekdays.map((label) => <span key={`skip-weekday-${label}`}>{label}</span>)}
                              </div>
                              <div className="plans-date-picker-grid">
                                {calendarMonthDays.map((day) => {
                                  const isoValue = isoDateFromCalendarDate(day)
                                  const isCurrentMonth = day.getMonth() === calendarMonth.getMonth()
                                  const isSelected = currentSkipUntilParts.date === isoValue
                                  const isToday = isoValue === isoDateFromCalendarDate(new Date())
                                  return (
                                    <button
                                      className={`plans-date-picker-day${isCurrentMonth ? '' : ' is-outside'}${isSelected ? ' is-selected' : ''}${isToday ? ' is-today' : ''}`}
                                      key={`skip-day-${isoValue}`}
                                      onClick={() => selectCalendarDate('skipUntilDate', day)}
                                      type="button"
                                    >
                                      {day.getDate()}
                                    </button>
                                  )
                                })}
                              </div>
                            </div>
                          ) : null}
                        </div>
                        <label className="field">
                          <span>Skip time</span>
                          <input
                            inputMode="numeric"
                            onBlur={(event) => {
                              const normalized = normalizeTimeInput(event.target.value)
                              event.target.value = normalized ?? currentSkipUntilParts.time
                              updateSkipUntil(currentSkipUntilParts.date, normalized ?? currentSkipUntilParts.time)
                            }}
                            onChange={(event) => updateSkipUntil(currentSkipUntilParts.date, event.target.value)}
                            placeholder="00:00"
                            value={currentSkipUntilParts.time}
                          />
                        </label>
                      </div>
                    ) : null}
                    <div className="action-row">
                      <button className="ghost-button" disabled={!selectedPlan || (currentSkipInput.mode === 'minutes' && !currentSkipInput.minutes) || (currentSkipInput.mode === 'until' && !currentSkipInput.until)} onClick={() => selectedPlan && void applySyncPlanSkip({ ...currentSkipInput, id: selectedPlan.id })} type="button">Apply skip</button>
                    </div>
                  </section>
                </div>
              </article>

              <article className="panel plans-runtime-card">
                <div className="panel-header compact-header">
                  <h2 className="plans-filter-heading">Matched sources</h2>
                  <div className="action-row">
                    <span className="pill">{previewCount ?? '-'}</span>
                    <button className="ghost-button" onClick={() => void handlePreview()} type="button">Preview</button>
                  </div>
                </div>
                {previewHandles.length > 0 ? (
                  <div className="plans-chip-list">
                    {previewHandles.map((entry) => <span className="pill" key={entry}>{entry}</span>)}
                  </div>
                ) : (
                  <p className="plans-empty-note">Run preview to inspect the current targeting result.</p>
                )}
              </article>

              <article className="panel plans-runtime-card plans-runtime-card-wide">
                <div className="panel-header compact-header"><h2 className="plans-filter-heading">Recent runs</h2><span className="pill">{selectedPlanRuns.length}</span></div>
                <div className="plans-runtime-list">
                  {selectedPlanRuns.length > 0 ? selectedPlanRuns.map((entry) => (
                    <div className="list-row" key={entry.id}>
                      <div>
                        <strong>{entry.summary}</strong>
                        <p>{entry.trigger} · {entry.finishedAt}</p>
                      </div>
                      <span className={`pill pill-${entry.status}`}>{entry.sourceCount}</span>
                    </div>
                  )) : <p className="plans-empty-note">No run history yet.</p>}
                </div>
              </article>
            </div>
          </section>
        ) : null}

        <footer className="plans-editor-actions">
          <div className="action-row">
            <button className="primary-button" disabled={Boolean(pendingCommand)} type="submit">Save</button>
            <button className="ghost-button" disabled={!selectedPlan} onClick={() => selectedPlan && void cloneSyncPlan(selectedPlan.id)} type="button">Clone</button>
            <button className="danger-button" disabled={!selectedPlan} onClick={() => selectedPlan && void deleteSyncPlan(selectedPlan.id)} type="button">Delete</button>
            <button className="ghost-button" disabled={!selectedPlan} onClick={() => selectedPlan && void moveSyncPlan(selectedPlan.id, 'up')} type="button">Up</button>
            <button className="ghost-button" disabled={!selectedPlan} onClick={() => selectedPlan && void moveSyncPlan(selectedPlan.id, 'down')} type="button">Down</button>
          </div>
        </footer>
      </form>
    </div>
  )
}
