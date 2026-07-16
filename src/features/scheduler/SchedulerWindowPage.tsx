import type { ReactNode } from 'react'
import { useEffect, useMemo, useState } from 'react'
import { openPlansWindow, subscribeToDesktopRuntimeEvents } from '../../bridge/desktop'
import type { SchedulerPauseMode, SchedulerSetUpsert, SetSyncPlanPauseInput, SkipSyncPlanInput, SyncPlan } from '../../domain/models'
import { useAppStore } from '../../state/appStore'
import { WindowShell } from '../brand/WindowShell'
import { WindowTitlebar } from '../brand/WindowTitlebar'
import {
  PAUSE_PRESETS,
  createSetDraft,
  findSetById,
  firstActiveSet,
  formatPlanRow,
  runtimeStateLabel,
} from './schedulerShared'

type SchedulerMenuKey = 'pause' | 'skip' | 'settings' | null

interface ToolbarMenuProps {
  disabled?: boolean
  label: string
  menuKey: Exclude<SchedulerMenuKey, null>
  openMenu: SchedulerMenuKey
  onToggle?: (nextOpen: boolean) => void
  setOpenMenu: (value: SchedulerMenuKey) => void
  children: ReactNode
}

function ToolbarMenu({ disabled, label, menuKey, openMenu, onToggle, setOpenMenu, children }: ToolbarMenuProps) {
  const isOpen = openMenu === menuKey

  return (
    <div className="menu-root scheduler-toolbar-menu-root">
      <button
        aria-expanded={isOpen}
        className={isOpen ? 'toolbar-button scheduler-toolbar-menu-button menu-button-open' : 'toolbar-button scheduler-toolbar-menu-button'}
        disabled={disabled}
        onClick={() => {
          const nextOpen = !isOpen
          onToggle?.(nextOpen)
          setOpenMenu(nextOpen ? menuKey : null)
        }}
        type="button"
      >
        {label}
      </button>
      {isOpen && !disabled ? <div className="menu-dropdown scheduler-toolbar-menu">{children}</div> : null}
    </div>
  )
}

function menuAction(
  key: string,
  label: string,
  action: () => void | Promise<void>,
  setOpenMenu: (value: SchedulerMenuKey) => void,
  hint?: string,
) {
  return (
    <button
      key={key}
      className="menu-item"
      onClick={() => {
        void action()
        setOpenMenu(null)
      }}
      type="button"
    >
      <strong>{label}</strong>
      {hint ? <span>{hint}</span> : null}
    </button>
  )
}

function skipSummary(plan?: SyncPlan) {
  if (!plan?.skipUntil) {
    return 'no delay'
  }
  return `delayed until ${plan.skipUntil}`
}

export function SchedulerWindowPage() {
  const bootstrap = useAppStore((state) => state.bootstrap)
  const refreshSnapshot = useAppStore((state) => state.refreshSnapshot)
  const snapshot = useAppStore((state) => state.snapshot)
  const pendingCommand = useAppStore((state) => state.pendingCommand)
  const deleteSyncPlan = useAppStore((state) => state.deleteSyncPlan)
  const runSyncPlanNow = useAppStore((state) => state.runSyncPlanNow)
  const setSyncPlanPause = useAppStore((state) => state.setSyncPlanPause)
  const applySyncPlanSkip = useAppStore((state) => state.applySyncPlanSkip)
  const moveSyncPlan = useAppStore((state) => state.moveSyncPlan)
  const upsertSchedulerSet = useAppStore((state) => state.upsertSchedulerSet)

  const schedulerSets = useMemo(() => snapshot?.schedulerSets ?? [], [snapshot])
  const [selectedSetId, setSelectedSetId] = useState<string>()
  const [selectedPlanId, setSelectedPlanId] = useState<string>()
  const [openMenu, setOpenMenu] = useState<SchedulerMenuKey>(null)
  const [schedulerSetDraft, setSchedulerSetDraft] = useState<SchedulerSetUpsert>(() => createSetDraft())
  const [pauseInput, setPauseInput] = useState<SetSyncPlanPauseInput>({ id: '', pauseMode: 'disabled' })
  const [skipInput, setSkipInput] = useState<SkipSyncPlanInput>({ id: '', mode: 'default', minutes: 60 })

  useEffect(() => {
    void bootstrap()
  }, [bootstrap])

  useEffect(() => {
    let disposed = false
    let unsubscribe: (() => void) | undefined

    void subscribeToDesktopRuntimeEvents({
      onSchedulerTick: () => {
        if (!disposed) {
          void refreshSnapshot().catch(() => undefined)
        }
      },
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
  }, [refreshSnapshot])

  useEffect(() => {
    function handlePointerDown(event: MouseEvent) {
      const target = event.target
      if (target instanceof HTMLElement && target.closest('.scheduler-toolbar-menu-root')) {
        return
      }
      setOpenMenu(null)
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key !== 'Escape' || openMenu === null) return
      event.preventDefault()
      event.stopImmediatePropagation()
      setOpenMenu(null)
    }

    document.addEventListener('mousedown', handlePointerDown)
    document.addEventListener('keydown', handleEscape, true)
    return () => {
      document.removeEventListener('mousedown', handlePointerDown)
      document.removeEventListener('keydown', handleEscape, true)
    }
  }, [openMenu])

  const effectiveSetId = selectedSetId ?? firstActiveSet(schedulerSets)?.id
  const selectedSet = useMemo(
    () => findSetById(schedulerSets, effectiveSetId) ?? firstActiveSet(schedulerSets),
    [effectiveSetId, schedulerSets],
  )
  const plans = useMemo(() => selectedSet?.plans ?? [], [selectedSet])
  const effectivePlanId = selectedPlanId ?? plans[0]?.id
  const selectedPlan = useMemo(
    () => plans.find((entry) => entry.id === effectivePlanId) ?? plans[0],
    [effectivePlanId, plans],
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
  const showSetSelector = schedulerSets.length > 1

  async function applyPauseMode(mode: SchedulerPauseMode) {
    if (!selectedPlan) {
      return
    }
    const nextInput: SetSyncPlanPauseInput = {
      id: selectedPlan.id,
      pauseMode: mode,
      pauseUntil: mode === 'until' ? currentPauseInput.pauseUntil : undefined,
    }
    setPauseInput(nextInput)
    await setSyncPlanPause(nextInput)
    setOpenMenu(null)
  }

  async function applyPauseUntil() {
    if (!selectedPlan || !currentPauseInput.pauseUntil) {
      return
    }
    const nextInput: SetSyncPlanPauseInput = {
      id: selectedPlan.id,
      pauseMode: 'until',
      pauseUntil: currentPauseInput.pauseUntil,
    }
    setPauseInput(nextInput)
    await setSyncPlanPause(nextInput)
    setOpenMenu(null)
  }

  async function applySkipMode(mode: SkipSyncPlanInput['mode']) {
    if (!selectedPlan) {
      return
    }
    const nextInput: SkipSyncPlanInput = {
      id: selectedPlan.id,
      mode,
      minutes: mode === 'minutes' ? currentSkipInput.minutes : undefined,
      until: mode === 'until' ? currentSkipInput.until : undefined,
    }
    setSkipInput(nextInput)
    await applySyncPlanSkip(nextInput)
    setOpenMenu(null)
  }

  async function applySkipMinutes() {
    if (!selectedPlan || !currentSkipInput.minutes) {
      return
    }
    const nextInput: SkipSyncPlanInput = { id: selectedPlan.id, mode: 'minutes', minutes: currentSkipInput.minutes }
    setSkipInput(nextInput)
    await applySyncPlanSkip(nextInput)
    setOpenMenu(null)
  }

  async function applySkipUntil() {
    if (!selectedPlan || !currentSkipInput.until) {
      return
    }
    const nextInput: SkipSyncPlanInput = { id: selectedPlan.id, mode: 'until', until: currentSkipInput.until }
    setSkipInput(nextInput)
    await applySyncPlanSkip(nextInput)
    setOpenMenu(null)
  }

  async function saveSchedulerSetDraft(createNew: boolean) {
    const name = schedulerSetDraft.name.trim()
    if (!name) {
      return
    }

    const payload: SchedulerSetUpsert = {
      id: createNew ? undefined : selectedSet?.id,
      name,
      active: schedulerSetDraft.active || schedulerSets.length === 0,
    }

    const saved = await upsertSchedulerSet(payload)
    const nextSet = createNew
      ? saved.schedulerSets.find((entry) => entry.name === payload.name)
      : saved.schedulerSets.find((entry) => entry.id === payload.id)

    if (nextSet) {
      setSelectedSetId(nextSet.id)
    }

    setOpenMenu(null)
  }

  if (!snapshot) {
    return (
      <WindowShell
        className="scheduler-window-frame"
        contentClassName="scheduler-window-content"
        density="compact"
        titlebar={<WindowTitlebar title="Scheduler" />}
      >
        <div className="panel runtime-log-window-empty">Loading scheduler...</div>
      </WindowShell>
    )
  }

  return (
    <WindowShell
      className="scheduler-window-frame"
      contentClassName="scheduler-window-content"
      density="compact"
      titlebar={
        <WindowTitlebar
          title="Scheduler"
          trailing={
            <span className="window-titlebar-status-meta">
              {selectedSet ? `${selectedSet.name} · ${plans.length}` : 'No set'}
            </span>
          }
        />
      }
    >
    <div className="scheduler-window-shell scheduler-window-shell-compact">
      <header className="scheduler-legacy-header panel scheduler-legacy-header-compact">
        <div className="scheduler-legacy-title-row scheduler-legacy-title-row-compact">
          {showSetSelector ? (
            <label className="field scheduler-legacy-set-picker scheduler-legacy-set-picker-compact">
              <span>Scheduler set</span>
              <select value={selectedSet?.id ?? ''} onChange={(event) => setSelectedSetId(event.target.value)}>
                {schedulerSets.map((entry) => <option key={entry.id} value={entry.id}>{entry.name}</option>)}
              </select>
            </label>
          ) : (
            <span className="muted-text">{selectedSet?.name ?? 'Scheduler set'}</span>
          )}
        </div>

        <div className="scheduler-toolbar-strip">
          <div className="toolbar-group scheduler-toolbar-group-compact">
            <ToolbarMenu
              label="Settings"
              menuKey="settings"
              onToggle={(nextOpen) => {
                if (!nextOpen) {
                  return
                }

                setSchedulerSetDraft(selectedSet
                  ? {
                    id: selectedSet.id,
                    name: selectedSet.name,
                    active: selectedSet.active,
                  }
                  : createSetDraft())
              }}
              openMenu={openMenu}
              setOpenMenu={setOpenMenu}
            >
              <div className="scheduler-toolbar-menu-section scheduler-toolbar-menu-section-first">
                <strong>Scheduler set</strong>
                <input
                  placeholder="Default Scheduler"
                  type="text"
                  value={schedulerSetDraft.name}
                  onChange={(event) => setSchedulerSetDraft((current) => ({ ...current, name: event.target.value }))}
                />
                <label className="scheduler-toolbar-menu-checkbox">
                  <input
                    checked={schedulerSetDraft.active}
                    onChange={(event) => setSchedulerSetDraft((current) => ({ ...current, active: event.target.checked }))}
                    type="checkbox"
                  />
                  <span>Active scheduler</span>
                </label>
                <div className="scheduler-toolbar-menu-actions">
                  <button className="toolbar-button" disabled={!schedulerSetDraft.name.trim()} onClick={() => void saveSchedulerSetDraft(false)} type="button">Save current</button>
                  <button className="toolbar-button" disabled={!schedulerSetDraft.name.trim()} onClick={() => void saveSchedulerSetDraft(true)} type="button">Create new</button>
                </div>
              </div>
            </ToolbarMenu>
            <button className="toolbar-button toolbar-button-primary" onClick={() => void openPlansWindow({ mode: 'new', schedulerSetId: selectedSet?.id })} type="button">Add</button>
            <button className="toolbar-button" disabled={!selectedPlan} onClick={() => selectedPlan && void openPlansWindow({ mode: 'clone', planId: selectedPlan.id, schedulerSetId: selectedPlan.schedulerSetId })} type="button">Clone</button>
            <button className="toolbar-button" disabled={!selectedPlan} onClick={() => selectedPlan && void openPlansWindow({ mode: 'edit', planId: selectedPlan.id, schedulerSetId: selectedPlan.schedulerSetId })} type="button">Edit</button>
            <button className="toolbar-button" disabled={!selectedPlan} onClick={() => selectedPlan && void deleteSyncPlan(selectedPlan.id)} type="button">Delete</button>
            <button className="toolbar-button" onClick={() => void refreshSnapshot()} type="button">Update</button>
            <button className="toolbar-button" disabled={!selectedPlan} onClick={() => selectedPlan && void moveSyncPlan(selectedPlan.id, 'up')} type="button">Up</button>
            <button className="toolbar-button" disabled={!selectedPlan} onClick={() => selectedPlan && void moveSyncPlan(selectedPlan.id, 'down')} type="button">Down</button>
            <button className="toolbar-button" disabled={!selectedPlan} onClick={() => selectedPlan && void runSyncPlanNow({ id: selectedPlan.id, force: false })} type="button">Start</button>
            <button className="toolbar-button" disabled={!selectedPlan} onClick={() => selectedPlan && void runSyncPlanNow({ id: selectedPlan.id, force: true })} type="button">Start (force)</button>
            <ToolbarMenu disabled={!selectedPlan} label="Pause" menuKey="pause" openMenu={openMenu} setOpenMenu={setOpenMenu}>
              {PAUSE_PRESETS.filter((mode) => mode !== 'until').map((mode) => menuAction(mode, mode, () => applyPauseMode(mode), setOpenMenu))}
              <div className="scheduler-toolbar-menu-section">
                <strong>Pause until</strong>
                <input
                  type="datetime-local"
                  value={currentPauseInput.pauseUntil ?? ''}
                  onChange={(event) => setPauseInput((current) => ({ ...current, id: selectedPlan?.id ?? '', pauseMode: 'until', pauseUntil: event.target.value || undefined }))}
                />
                <button className="toolbar-button" disabled={!currentPauseInput.pauseUntil} onClick={() => void applyPauseUntil()} type="button">Apply</button>
              </div>
            </ToolbarMenu>
            <ToolbarMenu disabled={!selectedPlan} label="Skip" menuKey="skip" openMenu={openMenu} setOpenMenu={setOpenMenu}>
              {menuAction('skip-default', 'Skip', () => applySkipMode('default'), setOpenMenu, 'Use the plan interval')}
              {menuAction('skip-reset', 'Delay reset', () => applySkipMode('reset'), setOpenMenu)}
              <div className="scheduler-toolbar-menu-section">
                <strong>Delay for minutes</strong>
                <input
                  type="number"
                  min="1"
                  value={currentSkipInput.minutes ?? ''}
                  onChange={(event) => setSkipInput((current) => ({ ...current, id: selectedPlan?.id ?? '', mode: 'minutes', minutes: event.target.value ? Number(event.target.value) : undefined }))}
                />
                <button className="toolbar-button" disabled={!currentSkipInput.minutes} onClick={() => void applySkipMinutes()} type="button">Apply</button>
              </div>
              <div className="scheduler-toolbar-menu-section">
                <strong>Delay by date/time</strong>
                <input
                  type="datetime-local"
                  value={currentSkipInput.until ?? ''}
                  onChange={(event) => setSkipInput((current) => ({ ...current, id: selectedPlan?.id ?? '', mode: 'until', until: event.target.value || undefined }))}
                />
                <button className="toolbar-button" disabled={!currentSkipInput.until} onClick={() => void applySkipUntil()} type="button">Apply</button>
              </div>
            </ToolbarMenu>
          </div>
        </div>

        <div className="scheduler-inline-status-strip">
          <span>{selectedPlan ? `${runtimeStateLabel(selectedPlan)} · ${selectedPlan.lastRunAt ?? 'never run'}` : 'No plan selected'}</span>
          <span>{selectedPlan ? (selectedPlan.pauseUntil ? `pause until ${selectedPlan.pauseUntil}` : selectedPlan.pauseMode) : 'pause disabled'}</span>
          <span>{selectedPlan ? skipSummary(selectedPlan) : 'no delay'}</span>
        </div>
      </header>

      <section className="scheduler-legacy-list panel" aria-label="Scheduler plans list">
        {plans.length > 0 ? plans.map((plan) => {
          const isSelected = plan.id === selectedPlan?.id
          return (
            <button
              key={plan.id}
              className={isSelected ? 'scheduler-plan-row scheduler-plan-row-selected' : 'scheduler-plan-row'}
              onClick={() => setSelectedPlanId(plan.id)}
              onDoubleClick={() => void openPlansWindow({ mode: 'edit', planId: plan.id, schedulerSetId: plan.schedulerSetId })}
              type="button"
            >
              <span className="scheduler-plan-row-main">{plan.name}</span>
              <span className="scheduler-plan-row-meta">{formatPlanRow(plan)}</span>
            </button>
          )
        }) : <div className="scheduler-empty scheduler-empty-large"><strong>No plans</strong><p>Create the first plan for this scheduler set.</p></div>}
      </section>

      <footer className="scheduler-legacy-footer">
        <span>{selectedSet ? `${selectedSet.name} · ${plans.length} plans` : 'No scheduler set'}</span>
        <span>{selectedPlan ? selectedPlan.lastRunSummary ?? 'No execution summary.' : 'Select a plan to inspect runtime state.'}</span>
        <span>{pendingCommand ?? 'idle'}</span>
      </footer>
    </div>
    </WindowShell>
  )
}
