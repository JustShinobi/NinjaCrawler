import type { ChangeEvent } from 'react'
import type {
  ProviderKey,
  SchedulerGroup,
  SchedulerPlanCriteria,
  SchedulerPlanNotifications,
  SchedulerPauseMode,
  SchedulerSet,
  SchedulerSetUpsert,
  SyncPlan,
  SyncPlanRun,
  SyncPlanUpsert,
} from '../../domain/models'

export const PROVIDERS: ProviderKey[] = ['instagram', 'tiktok', 'twitter']
export const PAUSE_PRESETS: SchedulerPauseMode[] = ['disabled', 'unlimited', '1h', '2h', '4h', '6h', '12h', 'until']

export function createNotifications(): SchedulerPlanNotifications {
  return { enabled: true, simple: true, showImage: false, showUserIcon: false }
}

export function createCriteria(): SchedulerPlanCriteria {
  return {
    regular: true,
    temporary: false,
    favorite: false,
    readyForDownload: true,
    ignoreReadyForDownload: false,
    downloadUsers: true,
    downloadSubscriptions: true,
    userExists: true,
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
    groupsOnly: false,
    usersCount: undefined,
    daysNumber: undefined,
    daysIsDownloaded: false,
    dateFrom: undefined,
    dateTo: undefined,
    dateInRange: true,
    dateMode: undefined,
    advancedExpression: '',
  }
}

export function createSetDraft(): SchedulerSetUpsert {
  return { name: '', active: false }
}

export function createPlanDraft(setId = ''): SyncPlanUpsert {
  return {
    schedulerSetId: setId,
    name: '',
    enabled: true,
    mode: 'automatic',
    intervalMinutes: 30,
    startupDelayMinutes: 1,
    notificationMode: 'summary',
    targetFilter: '',
    notifications: createNotifications(),
    criteria: createCriteria(),
  }
}

export function csvToList(value: string): string[] {
  return value
    .split(',')
    .map((entry) => entry.trim())
    .filter(Boolean)
}

export function listToCsv(value: string[] | undefined): string {
  return (value ?? []).join(', ')
}

export function mapPlanToDraft(value: SyncPlan): SyncPlanUpsert {
  const notifications = { ...createNotifications(), ...structuredClone(value.notifications ?? createNotifications()) }
  const criteria = { ...createCriteria(), ...structuredClone(value.criteria ?? createCriteria()) }
  return {
    id: value.id,
    schedulerSetId: value.schedulerSetId,
    name: value.name,
    enabled: value.enabled,
    mode: value.mode,
    intervalMinutes: value.intervalMinutes,
    startupDelayMinutes: value.startupDelayMinutes,
    notificationMode: value.notificationMode,
    targetFilter: value.targetFilter,
    sortIndex: value.sortIndex,
    pauseMode: value.pauseMode,
    pauseUntil: value.pauseUntil,
    notifications,
    criteria,
  }
}

export function syncNotificationMode(notifications: SchedulerPlanNotifications): SchedulerPlanNotifications {
  return {
    ...notifications,
    simple: notifications.simple,
    showImage: notifications.simple ? false : notifications.showImage,
    showUserIcon: notifications.simple ? false : notifications.showUserIcon,
  }
}

export function statusLabel(plan: SyncPlan): string {
  if (!plan.enabled) return 'disabled'
  if (plan.paused) return plan.pauseUntil ? `paused until ${plan.pauseUntil}` : 'paused'
  if (plan.mode === 'manual') return 'manual'
  return plan.nextDueAt ? `next ${plan.nextDueAt}` : plan.lastRunStatus
}

export function runtimeStateLabel(plan: SyncPlan): string {
  if (!plan.enabled) return 'disabled'
  if (plan.paused) return 'paused'
  if (plan.mode === 'manual') return 'manual'
  if (plan.lastRunStatus === 'failed') return 'failed'
  if (plan.lastRunStatus === 'skipped') return 'skipped'
  if (plan.lastRunStatus === 'succeeded') return 'ready'
  return 'idle'
}

export function formatPlanRow(plan: SyncPlan): string {
  const parts = [`${plan.name} (${runtimeStateLabel(plan)})`]
  parts.push(`last run: ${plan.lastRunAt ?? 'never'}`)
  if (plan.mode === 'automatic') {
    parts.push(`next run: ${plan.nextDueAt ?? 'pending'}`)
  }
  return parts.join('; ')
}

export function checkbox(event: ChangeEvent<HTMLInputElement>): boolean {
  return event.target.checked
}

export function firstActiveSet(schedulerSets: SchedulerSet[]): SchedulerSet | undefined {
  return schedulerSets.find((entry) => entry.active) ?? schedulerSets[0]
}

export function findPlanById(schedulerSets: SchedulerSet[], planId?: string): SyncPlan | undefined {
  if (!planId) {
    return undefined
  }
  return schedulerSets.flatMap((entry) => entry.plans).find((entry) => entry.id === planId)
}

export function findSetById(schedulerSets: SchedulerSet[], setId?: string): SchedulerSet | undefined {
  if (!setId) {
    return undefined
  }
  return schedulerSets.find((entry) => entry.id === setId)
}

export function findRunsForPlan(syncPlanRuns: SyncPlanRun[], planId?: string): SyncPlanRun[] {
  if (!planId) {
    return []
  }
  return syncPlanRuns.filter((entry) => entry.planId === planId).slice(0, 10)
}

export function groupNameMap(groups: SchedulerGroup[]): Map<string, string> {
  return new Map(groups.map((group) => [group.id, group.name]))
}
