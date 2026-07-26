import { convertFileSrc } from "@tauri-apps/api/core";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type Dispatch,
  type KeyboardEvent as ReactKeyboardEvent,
  type SetStateAction,
} from "react";
import {
  applyMediaDedupe,
  cancelMediaDedupe,
  enqueueMediaDedupeScan,
  installMediaDedupeSimilarityEngine,
  installMediaToolRuntime,
  loadMediaDedupeStatus,
  loadWorkspaceHealth,
  openAccountsWindow,
  openProfileViewWindow,
  openRuntimeLogWindow,
  openSourceFolder,
  runSourceSync,
  subscribeToDesktopRuntimeEvents,
  subscribeToWorkspaceHealthWindowIntent,
  validateProviderAccount,
} from "../../bridge/desktop";
import type {
  MediaDedupeFile,
  MediaDedupeGroup,
  MediaDedupeJobStatus,
  MediaDedupeResourceProfile,
  MediaDedupeScanProfile,
  MediaDedupeSimilarSelection,
  SourceHealthItem,
  WorkspaceHealthIncident,
  WorkspaceHealthSeverity,
  WorkspaceHealthSnapshot,
  WorkspaceHealthWindowIntent,
} from "../../domain/models";
import { WindowShell } from "../brand/WindowShell";
import { WindowTitlebar } from "../brand/WindowTitlebar";
import { pickBestDedupeFile } from "./pickBestDedupeFile";

type HealthTab = "overview" | "sources" | "accounts" | "storage";
type SourceFilter = "all" | "attention" | "recurring" | "stale" | "never";

const healthTabs: ReadonlyArray<readonly [HealthTab, string]> = [
  ["overview", "Overview"],
  ["sources", "Sources"],
  ["accounts", "Accounts"],
  ["storage", "Storage & Cleanup"],
];

function formatBytes(value: number): string {
  if (value >= 1024 ** 3) return `${(value / 1024 ** 3).toFixed(1)} GB`;
  if (value >= 1024 ** 2) return `${(value / 1024 ** 2).toFixed(1)} MB`;
  return `${Math.round(value / 1024)} KB`;
}

function formatDate(value?: string): string {
  if (!value) return "Never";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const showYear = date.getFullYear() !== new Date().getFullYear();
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    ...(showYear ? { year: "numeric" as const } : {}),
    hour: "numeric",
    minute: "2-digit",
  });
}

function formatDuration(value?: number): string {
  if (value === undefined || !Number.isFinite(value)) return "Calculating…";
  const seconds = Math.max(0, Math.round(value));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (hours) return `${hours}h ${minutes}m`;
  if (minutes) return `${minutes}m`;
  return `${seconds}s`;
}

function displayHandle(value: string): string {
  return value.startsWith("@") ? value : `@${value}`;
}

function providerLabel(value?: string): string {
  if (value === "instagram") return "Instagram";
  if (value === "tiktok") return "TikTok";
  if (value === "twitter") return "X / Twitter";
  if (value === "youtube") return "YouTube";
  if (value === "vsco") return "VSCO";
  return "Entire library";
}

function severityLabel(value: WorkspaceHealthSeverity): string {
  if (value === "critical") return "Critical";
  if (value === "attention") return "Attention";
  return "Healthy";
}

function errorMessage(value: unknown, fallback: string): string {
  if (value instanceof Error && value.message.trim()) return value.message;
  if (typeof value === "string" && value.trim()) return value;
  return fallback;
}

const idleDedupeStatus: MediaDedupeJobStatus = {
  state: "idle",
  stage: "idle",
  resourceProfile: "balanced",
  scanProfile: "recommended",
  similarityScope: "source",
  filesProcessed: 0,
  filesTotal: 0,
  bytesProcessed: 0,
  bytesTotal: 0,
  cancellable: false,
  similarityEngine: {
    status: "not_installed",
    version: "unknown",
    installed: false,
    ffmpegAvailable: false,
    ffmpegStatus: "not_installed",
  },
  perceptualSourcesProcessed: 0,
  perceptualSourcesTotal: 0,
  perceptualSourcesFailed: 0,
  elapsedSeconds: 0,
  sourceJobs: [],
  updatedAt: "",
};

function dedupeStageLabel(status: MediaDedupeJobStatus): string {
  switch (status.stage) {
    case "starting":
      return "Starting library scan";
    case "inventory":
      return "Discovering media files";
    case "hashing_exact_candidates":
      return "Hashing exact duplicate candidates";
    case "perceptual_scan":
      return "Comparing similar media by source";
    case "grouping":
      return "Grouping duplicate candidates";
    case "acquiring_lock":
      return "Waiting for workspace maintenance lock";
    case "preparing":
      return "Preparing cleanup";
    case "consolidating_exact":
      return "Consolidating exact duplicates";
    case "recycling_similar":
      return "Moving reviewed files to Recycle Bin";
    case "cancelling":
      return "Cancelling cleanup";
    default:
      return status.stage.replaceAll("_", " ");
  }
}

function sourceProblemLabel(source: SourceHealthItem): string {
  if (source.problemCode) return "Sync blocked";
  if (source.recurringFailure) return "Repeated failures";
  if (source.latestStatus === "failed") return "Latest run failed";
  return "None";
}

function readableProblemCode(value?: string): string {
  if (!value) return "No active problem";
  return value.replaceAll("_", " ");
}

function incidentScope(kind: string): string {
  if (kind.startsWith("source_")) return "Source";
  if (kind.startsWith("account_")) return "Account";
  if (kind === "storage") return "Storage";
  return "Workspace";
}

function handleTabKeyDown(
  event: ReactKeyboardEvent<HTMLButtonElement>,
  index: number,
  selectTab: (tab: HealthTab) => void,
) {
  let nextIndex: number | undefined;
  if (event.key === "ArrowRight") nextIndex = (index + 1) % healthTabs.length;
  if (event.key === "ArrowLeft")
    nextIndex = (index - 1 + healthTabs.length) % healthTabs.length;
  if (event.key === "Home") nextIndex = 0;
  if (event.key === "End") nextIndex = healthTabs.length - 1;
  if (nextIndex === undefined) return;
  event.preventDefault();
  const nextTab = healthTabs[nextIndex][0];
  selectTab(nextTab);
  window.setTimeout(
    () => document.getElementById(`health-tab-${nextTab}`)?.focus(),
    0,
  );
}

function Progress({ status, scopeLabel }: { status: MediaDedupeJobStatus; scopeLabel: string }) {
  const active = ["queued", "scanning", "applying"].includes(status.state);
  if (!active) return null;
  const sourcePhase = status.stage === "perceptual_scan";
  const resourceProfile = status.resourceProfile ?? "balanced";
  const indeterminate =
    (sourcePhase
      ? status.perceptualSourcesTotal <= 0
      : status.filesTotal <= 0) ||
    ["starting", "inventory", "acquiring_lock", "preparing"].includes(
      status.stage,
    );
  const progressDone = sourcePhase
    ? status.perceptualSourcesProcessed
    : status.filesProcessed;
  const progressTotal = sourcePhase
    ? status.perceptualSourcesTotal
    : status.filesTotal;
  const percent =
    progressTotal > 0
      ? Math.min(
          100,
          Math.round((progressDone * 100) / progressTotal),
        )
      : 0;
  const fileLabel =
    sourcePhase
      ? `${status.perceptualSourcesProcessed.toLocaleString()} of ${status.perceptualSourcesTotal.toLocaleString()} sources`
      : indeterminate
        ? `${status.filesProcessed.toLocaleString()} files discovered`
        : `${status.filesProcessed.toLocaleString()} of ${status.filesTotal.toLocaleString()} candidates`;
  return (
    <section
      className="health-dedupe-progress"
      aria-label="Media cleanup progress"
      aria-live="polite"
      role="status"
    >
      <div className="health-progress-heading">
        <div className="health-progress-title">
          <span aria-hidden="true" className="health-activity-indicator" />
          <span>
            <small>Media cleanup</small>
            <strong>{dedupeStageLabel(status)}</strong>
          </span>
        </div>
        <span className="health-progress-count">{fileLabel}</span>
      </div>
      <div
        aria-label={dedupeStageLabel(status)}
        aria-valuemax={indeterminate ? undefined : 100}
        aria-valuemin={indeterminate ? undefined : 0}
        aria-valuenow={indeterminate ? undefined : percent}
        aria-valuetext={fileLabel}
        className={`queue-status-progress-track${indeterminate ? " indeterminate" : ""}`}
        role="progressbar"
      >
        <div
          className="queue-status-progress-fill"
          style={indeterminate ? undefined : { width: `${percent}%` }}
        />
      </div>
      {sourcePhase && status.filesTotal > 0 ? (
        <div className="health-progress-current-source">
          <span>
            Current source · {status.filesProcessed.toLocaleString()} of{" "}
            {status.filesTotal.toLocaleString()} videos
          </span>
          <div
            aria-label="Current source progress"
            aria-valuemax={100}
            aria-valuemin={0}
            aria-valuenow={Math.min(
              100,
              Math.round((status.filesProcessed * 100) / status.filesTotal),
            )}
            className="queue-status-progress-track health-progress-secondary"
            role="progressbar"
          >
            <div
              className="queue-status-progress-fill"
              style={{
                width: `${Math.min(100, Math.round((status.filesProcessed * 100) / status.filesTotal))}%`,
              }}
            />
          </div>
        </div>
      ) : null}
      <div className="health-progress-detail">
        <small title={status.currentPath}>
          {status.currentPath ??
            status.currentRoot ??
            "Preparing the media inventory…"}
        </small>
        <small>
          {sourcePhase
            ? `${status.perceptualSourcesFailed.toLocaleString()} failures`
            : `${formatBytes(status.bytesProcessed)} ${indeterminate ? "discovered" : "processed"}`}
        </small>
      </div>
      <div className="health-progress-metrics">
        <span>{scopeLabel}</span>
        <span>Similarity within source</span>
        <span>{resourceProfile[0].toUpperCase() + resourceProfile.slice(1)}</span>
        <span>{formatDuration(status.elapsedSeconds ?? 0)} elapsed</span>
        <span>{formatDuration(status.estimatedSecondsRemaining)} remaining</span>
        {status.throughputPerSecond ? (
          <span>
            {sourcePhase
              ? `${(status.throughputPerSecond * 60).toFixed(1)} sources/min`
              : `${status.throughputPerSecond.toFixed(1)} files/s`}
          </span>
        ) : null}
      </div>
      <small className="health-progress-background-note">
        The scan continues in the background while you review other tabs.
      </small>
    </section>
  );
}

export function WorkspaceHealthWindowPage({
  initialIntent,
}: {
  initialIntent?: WorkspaceHealthWindowIntent;
}) {
  const [health, setHealth] = useState<WorkspaceHealthSnapshot>();
  const [dedupe, setDedupe] = useState<MediaDedupeJobStatus>();
  const [tab, setTab] = useState<HealthTab>(initialIntent?.initialTab ?? "overview");
  const [selectedIncident, setSelectedIncident] =
    useState<WorkspaceHealthIncident>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const [cleanupError, setCleanupError] = useState<string>();
  const [busyAction, setBusyAction] = useState<string>();
  const [sourceFilter, setSourceFilter] = useState<SourceFilter>("all");
  const [providerFilter, setProviderFilter] = useState("all");
  const [cleanupProvider, setCleanupProvider] = useState("all");
  const [cleanupResourceProfile, setCleanupResourceProfile] =
    useState<MediaDedupeResourceProfile>("balanced");
  const [cleanupScanProfile, setCleanupScanProfile] =
    useState<MediaDedupeScanProfile>("recommended");
  const [sourceSearch, setSourceSearch] = useState("");
  const [reviewSelections, setReviewSelections] = useState<
    Record<string, { keepPath: string; removePaths: string[] }>
  >({});
  const incidentReturnFocusRef = useRef<HTMLElement | null>(null);

  const openIncident = useCallback((incident: WorkspaceHealthIncident) => {
    if (document.activeElement instanceof HTMLElement) {
      incidentReturnFocusRef.current = document.activeElement;
    }
    setSelectedIncident(incident);
  }, []);

  const closeIncident = useCallback(() => {
    setSelectedIncident(undefined);
    window.setTimeout(() => incidentReturnFocusRef.current?.focus(), 0);
  }, []);

  const refresh = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    const [healthResult, dedupeResult] = await Promise.allSettled([
      loadWorkspaceHealth(),
      loadMediaDedupeStatus(),
    ]);
    if (healthResult.status === "fulfilled") {
      setHealth(healthResult.value);
      setError(undefined);
    } else {
      setError(
        errorMessage(healthResult.reason, "Failed to load workspace health."),
      );
    }
    if (dedupeResult.status === "fulfilled") {
      setDedupe(dedupeResult.value);
      setCleanupError(undefined);
    } else {
      setCleanupError(
        errorMessage(dedupeResult.reason, "Failed to load media cleanup."),
      );
    }
    if (!silent) setLoading(false);
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    void subscribeToWorkspaceHealthWindowIntent((intent) => {
      if (intent.initialTab) setTab(intent.initialTab);
    }).then((value) => {
      unsubscribe = value;
    }).catch(() => undefined);
    return () => unsubscribe?.();
  }, []);
  useEffect(() => {
    const timer = window.setInterval(() => {
      if (document.visibilityState !== "hidden") void refresh(true);
    }, 30_000);
    return () => window.clearInterval(timer);
  }, [refresh]);
  useEffect(() => {
    if (!selectedIncident) return undefined;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeIncident();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [closeIncident, selectedIncident]);
  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    void subscribeToDesktopRuntimeEvents({
      onWorkspaceSnapshotChanged: () => void refresh(true),
      onMediaDedupeStatusChanged: (status) => setDedupe(status),
    })
      .then((value) => {
        unsubscribe = value;
      })
      .catch(() => undefined);
    return () => unsubscribe?.();
  }, [refresh]);

  const runAction = useCallback(
    async (key: string, action: () => Promise<unknown>) => {
      setBusyAction(key);
      try {
        await action();
        await refresh(true);
      } catch (actionError) {
        setError(errorMessage(actionError, "Workspace action failed."));
      } finally {
        setBusyAction(undefined);
      }
    },
    [refresh],
  );

  const runCleanupAction = useCallback(
    async (key: string, action: () => Promise<MediaDedupeJobStatus>) => {
      setBusyAction(key);
      setCleanupError(undefined);
      if (key === "scan") {
        setDedupe((current) => ({
          ...(current ?? idleDedupeStatus),
          state: "queued",
          stage: "starting",
          filesProcessed: 0,
          filesTotal: 0,
          bytesProcessed: 0,
          bytesTotal: 0,
          currentPath: undefined,
          currentRoot: undefined,
          cancellable: false,
          error: undefined,
          updatedAt: new Date().toISOString(),
        }));
      }
      try {
        const status = await action();
        setDedupe(status);
        await refresh(true);
      } catch (actionError) {
        setCleanupError(
          errorMessage(actionError, "Media cleanup action failed."),
        );
      } finally {
        setBusyAction(undefined);
      }
    },
    [refresh],
  );

  if (loading && !health) {
    return (
      <WindowShell
        density="compact"
        titlebar={<WindowTitlebar title="Workspace Health" />}
      >
        <div className="health-loading">Inspecting workspace health…</div>
      </WindowShell>
    );
  }
  if (!health) {
    return (
      <WindowShell
        density="compact"
        titlebar={<WindowTitlebar title="Workspace Health" />}
      >
        <div className="health-loading health-error">
          {error ?? "Workspace health is unavailable."}
        </div>
      </WindowShell>
    );
  }

  const criticalDisk = health.volumes.some(
    (volume) => volume.severity === "critical",
  );
  return (
    <WindowShell
      density="compact"
      contentClassName="health-window-shell-content"
      titlebar={
        <WindowTitlebar
          title="Workspace Health"
          trailing={
            <span
              className={`health-title-status health-tone-${health.overallStatus}`}
            >
              {severityLabel(health.overallStatus)}
            </span>
          }
        />
      }
    >
      <div className="health-window-body">
        <header className="health-header">
          <div>
            <span className="eyebrow">Operator view</span>
            <p aria-live="polite">Updated {formatDate(health.generatedAt)}</p>
          </div>
          <button
            className="ghost-button"
            disabled={loading}
            onClick={() => void refresh()}
            type="button"
          >
            {loading ? "Refreshing…" : "Refresh"}
          </button>
        </header>

        {criticalDisk ? (
          <div className="health-critical-banner" role="alert">
            <strong>Media storage needs immediate attention.</strong>
            <span>
              A primary root is unavailable or a volume has less than 5 GB free.
            </span>
            <button onClick={() => setTab("storage")} type="button">
              Review storage
            </button>
          </div>
        ) : null}
        {error ? (
          <div className="maintenance-error" role="alert">
            {error}
          </div>
        ) : null}
        {cleanupError ? (
          <div className="maintenance-error" role="alert">
            <strong>Media cleanup is unavailable.</strong> {cleanupError}
          </div>
        ) : null}

        <section
          className="health-summary-grid"
          aria-label="Workspace health summary"
        >
          <SummaryCard
            label="Overall status"
            value={severityLabel(health.overallStatus)}
            detail={`${health.counts.criticalIssueCount} critical · ${health.counts.attentionIssueCount} attention`}
            severity={health.overallStatus}
          />
          <SummaryCard
            label="Sources"
            value={`${health.counts.affectedSourceCount} affected`}
            detail={`${health.counts.recurringFailureCount} recurring failures`}
            severity={
              health.counts.affectedSourceCount ? "attention" : "healthy"
            }
          />
          <SummaryCard
            label="Accounts"
            value={`${health.counts.degradedAccountCount + health.counts.criticalAccountCount} affected`}
            detail={`${health.counts.criticalAccountCount} critical`}
            severity={
              health.counts.criticalAccountCount
                ? "critical"
                : health.counts.degradedAccountCount
                  ? "attention"
                  : "healthy"
            }
          />
          <SummaryCard
            label="Storage"
            value={
              health.volumes.length
                ? `${formatBytes(Math.min(...health.volumes.map((volume) => volume.availableBytes)))} free`
                : "Unavailable"
            }
            detail={`${health.volumes.length} volume(s)`}
            severity={
              health.volumes.some((volume) => volume.severity === "critical")
                ? "critical"
                : health.volumes.some(
                      (volume) => volume.severity === "attention",
                    )
                  ? "attention"
                  : "healthy"
            }
          />
        </section>

        <nav
          className="health-tabs"
          aria-label="Workspace health sections"
          role="tablist"
        >
          {healthTabs.map(([key, label], index) => (
            <button
              aria-controls={`health-panel-${key}`}
              aria-selected={tab === key}
              className={
                tab === key ? "health-tab health-tab-active" : "health-tab"
              }
              id={`health-tab-${key}`}
              key={key}
              onClick={() => setTab(key)}
              onKeyDown={(event) => handleTabKeyDown(event, index, setTab)}
              role="tab"
              tabIndex={tab === key ? 0 : -1}
              type="button"
            >
              {label}
            </button>
          ))}
        </nav>

        {tab !== "storage" &&
        dedupe &&
        ["queued", "scanning", "applying"].includes(dedupe.state) ? (
          <button
            className="health-cleanup-running-strip"
            onClick={() => setTab("storage")}
            type="button"
          >
            <span>
              <span aria-hidden="true" className="health-activity-indicator" />
              {dedupeStageLabel(dedupe)}
            </span>
            <strong>View progress</strong>
          </button>
        ) : null}

        <main className="health-content">
          {tab === "overview" ? (
            <Overview health={health} onSelect={openIncident} />
          ) : null}
          {tab === "sources" ? (
            <Sources
              health={health}
              filter={sourceFilter}
              provider={providerFilter}
              search={sourceSearch}
              onFilter={setSourceFilter}
              onProvider={setProviderFilter}
              onSearch={setSourceSearch}
              busyAction={busyAction}
              runAction={runAction}
            />
          ) : null}
          {tab === "accounts" ? (
            <Accounts
              health={health}
              busyAction={busyAction}
              runAction={runAction}
            />
          ) : null}
          {tab === "storage" ? (
            <Storage
              health={health}
              dedupe={dedupe}
              cleanupError={cleanupError}
              busyAction={busyAction}
              runCleanupAction={runCleanupAction}
              reviewSelections={reviewSelections}
              setReviewSelections={setReviewSelections}
              provider={cleanupProvider}
              onProvider={setCleanupProvider}
              resourceProfile={cleanupResourceProfile}
              onResourceProfile={setCleanupResourceProfile}
              scanProfile={cleanupScanProfile}
              onScanProfile={setCleanupScanProfile}
            />
          ) : null}
        </main>

        {selectedIncident ? (
          <div
            className="health-drawer-backdrop"
            onMouseDown={(event) => {
              if (event.target === event.currentTarget) closeIncident();
            }}
          >
            <IncidentDrawer
              incident={selectedIncident}
              onClose={closeIncident}
              onOpenStorage={() => {
                setTab("storage");
                setSelectedIncident(undefined);
              }}
              busyAction={busyAction}
              runAction={runAction}
            />
          </div>
        ) : null}
      </div>
    </WindowShell>
  );
}

function SummaryCard({
  label,
  value,
  detail,
  severity,
}: {
  label: string;
  value: string;
  detail: string;
  severity: WorkspaceHealthSeverity;
}) {
  return (
    <article className={`health-summary-card health-tone-${severity}`}>
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{detail}</small>
    </article>
  );
}

function Overview({
  health,
  onSelect,
}: {
  health: WorkspaceHealthSnapshot;
  onSelect: (incident: WorkspaceHealthIncident) => void;
}) {
  return (
    <section
      aria-labelledby="health-tab-overview"
      className="panel health-issues-panel health-tab-panel"
      id="health-panel-overview"
      role="tabpanel"
    >
      <div className="panel-header compact-header">
        <div>
          <span className="eyebrow">Prioritized issues</span>
          <h2 id="health-overview-heading">What needs attention</h2>
        </div>
        <span className="pill">{health.incidents.length.toLocaleString()}</span>
      </div>
      <div className="health-issue-list health-scroll-region">
        {health.incidents.map((incident) => (
          <button
            className="health-issue-row"
            key={incident.id}
            onClick={() => onSelect(incident)}
            type="button"
          >
            <span
              className={`health-severity-dot health-tone-${incident.severity}`}
            />
            <span className="health-issue-copy">
              <span className="health-issue-meta">
                {incidentScope(incident.kind)} ·{" "}
                {severityLabel(incident.severity)}
              </span>
              <strong>{incident.title}</strong>
              <span className="health-issue-detail">{incident.detail}</span>
            </span>
            <span aria-hidden="true" className="health-row-chevron">
              ›
            </span>
          </button>
        ))}
        {health.incidents.length === 0 ? (
          <div className="health-empty">
            <strong>Workspace is healthy</strong>
            <p>No source, account, or storage incidents were detected.</p>
          </div>
        ) : null}
      </div>
    </section>
  );
}

function Sources({
  health,
  filter,
  provider,
  search,
  onFilter,
  onProvider,
  onSearch,
  busyAction,
  runAction,
}: {
  health: WorkspaceHealthSnapshot;
  filter: SourceFilter;
  provider: string;
  search: string;
  onFilter: (value: SourceFilter) => void;
  onProvider: (value: string) => void;
  onSearch: (value: string) => void;
  busyAction?: string;
  runAction: (key: string, action: () => Promise<unknown>) => Promise<void>;
}) {
  const filtered = useMemo(
    () =>
      health.sources.filter((source) => {
        if (provider !== "all" && source.provider !== provider) return false;
        if (
          search &&
          !`${source.handle} ${source.displayName}`
            .toLowerCase()
            .includes(search.toLowerCase())
        )
          return false;
        if (filter === "attention" && source.severity === "healthy")
          return false;
        if (filter === "recurring" && !source.recurringFailure) return false;
        if (
          filter === "stale" &&
          !["stale", "old", "ancient"].includes(source.freshness)
        )
          return false;
        if (filter === "never" && source.freshness !== "never") return false;
        return true;
      }),
    [filter, health.sources, provider, search],
  );
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: filtered.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 70,
    overscan: 8,
  });
  return (
    <section
      aria-labelledby="health-tab-sources"
      className="panel health-table-panel health-tab-panel"
      id="health-panel-sources"
      role="tabpanel"
    >
      <div className="health-filter-bar">
        <label className="health-search-field">
          <span className="sr-only">Search sources</span>
          <input
            aria-label="Search sources"
            onChange={(event) => onSearch(event.target.value)}
            placeholder="Search handle or name"
            value={search}
          />
        </label>
        <select
          aria-label="Source health filter"
          onChange={(event) => onFilter(event.target.value as SourceFilter)}
          value={filter}
        >
          <option value="all">All sources</option>
          <optgroup label="Health">
            <option value="attention">Needs attention</option>
            <option value="recurring">Recurring failures</option>
          </optgroup>
          <optgroup label="Sync history">
            <option value="stale">Stale sync date</option>
            <option value="never">Never synced</option>
          </optgroup>
        </select>
        <select
          aria-label="Provider filter"
          onChange={(event) => onProvider(event.target.value)}
          value={provider}
        >
          <option value="all">All providers</option>
          <option value="instagram">Instagram</option>
          <option value="tiktok">TikTok</option>
          <option value="twitter">X / Twitter</option>
          <option value="youtube">YouTube</option>
          <option value="vsco">VSCO</option>
        </select>
        <span aria-live="polite" className="health-filter-count">
          <strong>{filtered.length.toLocaleString()}</strong> of{" "}
          {health.sources.length.toLocaleString()}
        </span>
      </div>
      <div aria-hidden="true" className="health-source-head">
        <span>Source</span>
        <span>Last sync</span>
        <span>Runs</span>
        <span>Current problem</span>
        <span>Actions</span>
      </div>
      <div
        className="health-source-virtual health-scroll-region"
        ref={scrollRef}
      >
        {filtered.length > 0 ? (
          <div
            style={{ height: virtualizer.getTotalSize(), position: "relative" }}
          >
            {virtualizer.getVirtualItems().map((row) => {
              const source = filtered[row.index];
              return (
                <SourceRow
                  key={source.sourceId}
                  source={source}
                  style={{ transform: `translateY(${row.start}px)` }}
                  busy={busyAction}
                  runAction={runAction}
                />
              );
            })}
          </div>
        ) : (
          <div className="health-empty">
            <strong>No matching sources</strong>
            <p>Adjust the search or filters to see more sources.</p>
          </div>
        )}
      </div>
    </section>
  );
}

function SourceRow({
  source,
  style,
  busy,
  runAction,
}: {
  source: SourceHealthItem;
  style: CSSProperties;
  busy?: string;
  runAction: (key: string, action: () => Promise<unknown>) => Promise<void>;
}) {
  return (
    <div className="health-source-row" style={style}>
      <div className="health-source-identity">
        <span
          className={`health-severity-dot health-tone-${source.severity}`}
        />
        <span>
          <strong>{displayHandle(source.handle)}</strong>
          <small>
            {source.provider} · {source.displayName}
          </small>
        </span>
      </div>
      <span className="health-source-date">
        <strong>{formatDate(source.lastSyncedAt)}</strong>
        <small>{source.freshness}</small>
      </span>
      <span className="health-source-runs">
        <strong>{source.consecutiveFailures}</strong>
        <small>{source.latestStatus ?? "no runs"}</small>
      </span>
      <span
        className="health-source-problem"
        title={source.problemMessage ?? source.problemCode}
      >
        <strong>{sourceProblemLabel(source)}</strong>
        <small>{readableProblemCode(source.problemCode)}</small>
      </span>
      <span className="health-row-actions">
        <button
          className="ghost-button"
          disabled={busy === `sync:${source.sourceId}`}
          onClick={() =>
            void runAction(`sync:${source.sourceId}`, () =>
              runSourceSync(source.sourceId),
            )
          }
          type="button"
        >
          {busy === `sync:${source.sourceId}` ? "Retrying…" : "Retry"}
        </button>
        <button
          className="ghost-button"
          onClick={() => void openProfileViewWindow(source.sourceId)}
          type="button"
        >
          Profile
        </button>
        <button
          className="ghost-button"
          onClick={() => void openSourceFolder(source.sourceId)}
          type="button"
        >
          Folder
        </button>
        <button
          className="ghost-button"
          onClick={() =>
            void openRuntimeLogWindow({ sourceId: source.sourceId })
          }
          type="button"
        >
          Log
        </button>
      </span>
    </div>
  );
}

function Accounts({
  health,
  busyAction,
  runAction,
}: {
  health: WorkspaceHealthSnapshot;
  busyAction?: string;
  runAction: (key: string, action: () => Promise<unknown>) => Promise<void>;
}) {
  return (
    <section
      aria-labelledby="health-tab-accounts"
      className="health-account-grid health-tab-panel health-scroll-region"
      id="health-panel-accounts"
      role="tabpanel"
    >
      {health.accounts.map((account) => (
        <article className="panel health-account-card" key={account.accountId}>
          <header>
            <div>
              <span className="eyebrow">{account.provider}</span>
              <h2>{account.displayName}</h2>
            </div>
            <span className={`status status-${account.authState}`}>
              {account.authState}
            </span>
          </header>
          <dl>
            <div>
              <dt>Last validation</dt>
              <dd>{formatDate(account.lastValidatedAt)}</dd>
            </div>
            <div>
              <dt>Linked sources</dt>
              <dd>{account.impactedSourceCount}</dd>
            </div>
            <div>
              <dt>Session secret</dt>
              <dd>{account.hasSecret ? "Available" : "Missing"}</dd>
            </div>
          </dl>
          {account.lastValidationError ? (
            <p className="health-account-error">
              {account.lastValidationError}
            </p>
          ) : null}
          <footer>
            <button
              className="primary-button"
              disabled={busyAction === `validate:${account.accountId}`}
              onClick={() =>
                void runAction(`validate:${account.accountId}`, () =>
                  validateProviderAccount(account.accountId),
                )
              }
              type="button"
            >
              {busyAction === `validate:${account.accountId}`
                ? "Validating…"
                : "Validate"}
            </button>
            <button
              className="ghost-button"
              onClick={() =>
                void openAccountsWindow({
                  initialAccountId: account.accountId,
                  initialMode: "edit",
                })
              }
              type="button"
            >
              Open account
            </button>
            <button
              className="ghost-button"
              onClick={() =>
                void openRuntimeLogWindow({ accountId: account.accountId })
              }
              type="button"
            >
              Log
            </button>
          </footer>
        </article>
      ))}
      {health.accounts.length === 0 ? (
        <div className="health-empty">
          <strong>No provider accounts</strong>
          <p>Add an account to validate connector sessions.</p>
        </div>
      ) : null}
    </section>
  );
}

function Storage({
  health,
  dedupe,
  cleanupError,
  busyAction,
  runCleanupAction,
  reviewSelections,
  setReviewSelections,
  provider,
  onProvider,
  resourceProfile,
  onResourceProfile,
  scanProfile,
  onScanProfile,
}: {
  health: WorkspaceHealthSnapshot;
  dedupe?: MediaDedupeJobStatus;
  cleanupError?: string;
  busyAction?: string;
  runCleanupAction: (
    key: string,
    action: () => Promise<MediaDedupeJobStatus>,
  ) => Promise<void>;
  reviewSelections: Record<string, { keepPath: string; removePaths: string[] }>;
  setReviewSelections: Dispatch<
    SetStateAction<Record<string, { keepPath: string; removePaths: string[] }>>
  >;
  provider: string;
  onProvider: (provider: string) => void;
  resourceProfile: MediaDedupeResourceProfile;
  onResourceProfile: (profile: MediaDedupeResourceProfile) => void;
  scanProfile: MediaDedupeScanProfile;
  onScanProfile: (profile: MediaDedupeScanProfile) => void;
}) {
  const scan = dedupe?.latestScan;
  const similarityEngine =
    dedupe?.similarityEngine ?? idleDedupeStatus.similarityEngine;
  const runtimeNeedsAttention =
    !similarityEngine.installed ||
    !similarityEngine.ffmpegAvailable ||
    Boolean(similarityEngine.error || similarityEngine.ffmpegError);
  // User toggle only; force-open when tools need attention so we never setState in an effect.
  const [runtimeExpandedByUser, setRuntimeExpandedByUser] = useState(false);
  const runtimeExpanded = runtimeNeedsAttention || runtimeExpandedByUser;
  // Collapsed by default: results are the focus now. Force-open the whole
  // "Scan details" disclosure when something nested inside needs attention,
  // otherwise the attention indicator would be hidden behind the collapse.
  const [scanDetailsExpandedByUser, setScanDetailsExpandedByUser] =
    useState(false);
  const sourceJobs = dedupe?.sourceJobs ?? [];
  const completedSourceJobs = sourceJobs.filter(
    (job) => job.status === "completed",
  ).length;
  const cachedSourceJobs = sourceJobs.filter(
    (job) => job.stage === "cached",
  ).length;
  const runningSourceJobs = sourceJobs.filter(
    (job) => job.status === "running",
  ).length;
  const failedSourceJobs = sourceJobs.filter(
    (job) => job.status === "failed",
  ).length;
  const queuedSourceJobs =
    sourceJobs.filter((job) => job.status === "queued").length +
    Math.max(0, (dedupe?.perceptualSourcesTotal ?? 0) - sourceJobs.length);
  const active = Boolean(
    dedupe && ["queued", "scanning", "applying"].includes(dedupe.state),
  );
  const consolidatableExactGroups =
    scan?.exactGroups.filter((group) => group.consolidatable) ?? [];
  const similarSelections: MediaDedupeSimilarSelection[] = Object.entries(
    reviewSelections,
  )
    .map(([groupId, value]) => ({
      groupId,
      keepPath: value.keepPath,
      removePaths: value.removePaths,
    }))
    .filter((selection) => selection.removePaths.length > 0);
  const scanButtonLabel =
    busyAction === "scan" || dedupe?.stage === "starting"
      ? "Starting scan…"
      : "Scan library";
  const availableProviders = Array.from(
    new Set(health.sources.map((source) => source.provider)),
  ).sort();
  const selectedSourceCount = health.sources.filter(
    (source) => provider === "all" || source.provider === provider,
  ).length;
  const scopedSourceId = dedupe?.sourceScope ?? scan?.sourceScope;
  const scopedSource = scopedSourceId
    ? health.sources.find((source) => source.sourceId === scopedSourceId)
    : undefined;
  const scopeLabel = scopedSource
    ? `${displayHandle(scopedSource.handle)} · ${providerLabel(scopedSource.provider)}`
    : providerLabel(dedupe?.providerScope ?? scan?.providerScope);
  const activeSourceScopeValue = active && dedupe?.sourceScope
    ? `source:${dedupe.sourceScope}`
    : undefined;
  const scanDetailsNeedsAttention = runtimeNeedsAttention || failedSourceJobs > 0;
  const scanDetailsExpanded =
    scanDetailsNeedsAttention || scanDetailsExpandedByUser;
  return (
    <div
      aria-labelledby="health-tab-storage"
      className="health-storage-stack health-tab-panel health-scroll-region"
      id="health-panel-storage"
      role="tabpanel"
    >
      <section aria-busy={active} className="panel health-cleanup-panel">
        <div className="panel-header compact-header">
          <div>
            <span className="eyebrow">Media cleanup</span>
            <h2>Find duplicate media</h2>
            <p>
              Scan safely first. Nothing is changed until you explicitly apply
              reviewed results.
            </p>
          </div>
          <div className="health-cleanup-actions">
            <label className="health-scan-scope">
              <span>Scan scope</span>
              <select
                aria-label="Media scan provider"
                disabled={active || busyAction === "scan"}
                onChange={(event) => onProvider(event.target.value)}
                value={activeSourceScopeValue ?? provider}
              >
                {activeSourceScopeValue ? (
                  <option value={activeSourceScopeValue}>{scopeLabel} (profile)</option>
                ) : null}
                <option value="all">Entire library</option>
                {availableProviders.map((value) => (
                  <option key={value} value={value}>
                    {providerLabel(value)}
                  </option>
                ))}
              </select>
            </label>
            <label className="health-scan-scope">
              <span>Resource use</span>
              <select
                aria-label="Media scan resource use"
                disabled={active || busyAction === "scan"}
                onChange={(event) =>
                  onResourceProfile(
                    event.target.value as MediaDedupeResourceProfile,
                  )
                }
                value={active ? dedupe?.resourceProfile ?? resourceProfile : resourceProfile}
              >
                <option value="quiet">Quiet</option>
                <option value="balanced">Balanced</option>
                <option value="fast">Fast</option>
              </select>
            </label>
            <label className="health-scan-scope">
              <span>Scan profile</span>
              <select
                aria-label="Media scan profile"
                disabled={active || busyAction === "scan"}
                onChange={(event) =>
                  onScanProfile(event.target.value as MediaDedupeScanProfile)
                }
                title="Recommended matches VDF's 'Edited & altered copies' preset. AI scan and Deep add neural and audio passes (downloads ~100 MB on first use) and take longer."
                value={active ? dedupe?.scanProfile ?? scanProfile : scanProfile}
              >
                <option value="recommended">Recommended</option>
                <option value="ai">AI scan</option>
                <option value="deep">Deep</option>
              </select>
            </label>
            {!active || dedupe?.stage === "starting" ? (
              <button
                className="primary-button"
                disabled={
                  Boolean(cleanupError) || active || busyAction === "scan"
                }
                onClick={() =>
                  void runCleanupAction("scan", () =>
                    enqueueMediaDedupeScan(
                      {
                        ...(provider === "all"
                          ? {}
                          : {
                              provider: provider as
                                | "instagram"
                                | "tiktok"
                                | "twitter"
                                | "youtube"
                                | "vsco",
                            }),
                        resourceProfile,
                        scanProfile,
                      },
                    ),
                  )
                }
                type="button"
              >
                {scanButtonLabel}
              </button>
            ) : null}
            {dedupe?.cancellable ? (
              <button
                className="ghost-button"
                disabled={busyAction === "cancel"}
                onClick={() =>
                  void runCleanupAction("cancel", cancelMediaDedupe)
                }
                type="button"
              >
                {busyAction === "cancel" ? "Cancelling…" : "Cancel scan"}
              </button>
            ) : null}
          </div>
        </div>
        {!active && scan ? (
          <div className="health-last-scan">
            <strong>Last scan</strong>
            <span>
              {scan.finishedAt ? formatDate(scan.finishedAt) : "completed"} ·{" "}
              {scopeLabel} · {scan.filesScanned.toLocaleString()} files
              scanned
            </span>
          </div>
        ) : null}
        <Progress status={dedupe ?? idleDedupeStatus} scopeLabel={scopeLabel} />
        {cleanupError ? (
          <div className="runtime-log-window-error" role="alert">
            Cleanup controls are unavailable. {cleanupError}
          </div>
        ) : null}
        {dedupe?.error ? (
          <div className="runtime-log-window-error" role="alert">
            {dedupe.error}
          </div>
        ) : null}
        <details
          className="health-runtime-disclosure health-scan-details-disclosure"
          onToggle={(event) =>
            setScanDetailsExpandedByUser(event.currentTarget.open)
          }
          open={scanDetailsExpanded}
        >
          <summary>
            <span>
              <strong>Scan details</strong>
              <small>
                Scan engine, media tools, per-source jobs, and scan
                statistics · Click to {scanDetailsExpanded ? "collapse" : "review"}
              </small>
            </span>
            <span className="health-runtime-disclosure-affordance">
              {scanDetailsNeedsAttention ? (
                <span className="status status-attention">Attention</span>
              ) : null}
              <svg aria-hidden="true" className="health-runtime-chevron" viewBox="0 0 20 20">
                <path d="m6 8 4 4 4-4" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" />
              </svg>
            </span>
          </summary>
          <div className="health-runtime-details health-scan-details-body">
            {!active ? (
              <div className="health-scan-semantics">
                <span>{selectedSourceCount.toLocaleString()} configured sources</span>
                <span>Exact SHA-256 across selected scope on each volume</span>
                <span>Visual similarity within each source</span>
              </div>
            ) : null}
            {dedupe ? (
              <details
                className="health-runtime-disclosure"
                onToggle={(event) => setRuntimeExpandedByUser(event.currentTarget.open)}
                open={runtimeExpanded}
              >
                <summary>
                  <span>
                    <strong>Scan engine &amp; media tools</strong>
                    <small>
                      {similarityEngine.installed && similarityEngine.ffmpegAvailable
                        ? `Ready · VDF ${similarityEngine.version} · FFmpeg ${similarityEngine.ffmpegVersion ?? "detected"} (${similarityEngine.ffmpegSource === "managed" ? "Managed" : "System"}) · Click to review`
                        : "Setup requires attention · Click to manage"}
                    </small>
                  </span>
                  <span className="health-runtime-disclosure-affordance">
                    <span className={`status ${similarityEngine.installed && similarityEngine.ffmpegAvailable ? "status-ready" : "status-attention"}`}>
                      {similarityEngine.installed && similarityEngine.ffmpegAvailable
                        ? "Ready"
                        : "Attention"}
                    </span>
                    <svg aria-hidden="true" className="health-runtime-chevron" viewBox="0 0 20 20">
                      <path d="m6 8 4 4 4-4" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8" />
                    </svg>
                  </span>
                </summary>
                <div className="health-runtime-details">
                  <div className="health-runtime-row">
                    <span>
                      <strong>
                        {similarityEngine.installed
                          ? `Video Duplicate Finder ${similarityEngine.version}`
                          : similarityEngine.status === "installing"
                            ? "Installing Video Duplicate Finder…"
                            : "Video similarity is not installed"}
                      </strong>
                      <small>
                        Exact copies use SHA-256. Images use NinjaCrawler aHash/dHash;
                        VDF compares videos within each source.
                      </small>
                      {similarityEngine.error ? (
                        <small className="health-tone-attention">
                          {similarityEngine.error}
                        </small>
                      ) : null}
                    </span>
                    {!similarityEngine.installed ? (
                      <button
                        className="ghost-button"
                        disabled={
                          active ||
                          busyAction === "install-engine" ||
                          similarityEngine.status === "installing" ||
                          similarityEngine.status === "unsupported"
                        }
                        onClick={() =>
                          void runCleanupAction(
                            "install-engine",
                            installMediaDedupeSimilarityEngine,
                          )
                        }
                        type="button"
                      >
                        {busyAction === "install-engine" ||
                        similarityEngine.status === "installing"
                          ? "Installing…"
                          : "Install similarity engine"}
                      </button>
                    ) : (
                      <span className="status status-ready">Installed</span>
                    )}
                  </div>
                  <div className="health-runtime-row">
                    <span>
                      <strong>
                        {similarityEngine.ffmpegStatus === "installing"
                          ? "Installing FFmpeg and FFprobe…"
                          : similarityEngine.ffmpegAvailable
                            ? `FFmpeg ${similarityEngine.ffmpegVersion ?? "detected"}`
                            : "FFmpeg and FFprobe were not found"}
                      </strong>
                      <small>
                        {similarityEngine.ffmpegSource === "managed"
                          ? "Private NinjaCrawler runtime; the system PATH is unchanged."
                          : similarityEngine.ffmpegSource === "system"
                            ? "Using the tools detected on the system PATH."
                            : "Required for video similarity and video thumbnails."}
                      </small>
                      {similarityEngine.ffmpegError ? (
                        <small className="health-tone-attention">
                          {similarityEngine.ffmpegError}
                        </small>
                      ) : null}
                    </span>
                    {similarityEngine.ffmpegAvailable ? (
                      <span className="status status-ready">
                        {similarityEngine.ffmpegSource === "managed" ? "Managed" : "System"}
                      </span>
                    ) : (
                      <button
                        className="ghost-button"
                        disabled={
                          active ||
                          busyAction === "install-media-tools" ||
                          similarityEngine.ffmpegStatus === "installing"
                        }
                        onClick={() =>
                          void runCleanupAction(
                            "install-media-tools",
                            installMediaToolRuntime,
                          )
                        }
                        type="button"
                      >
                        {busyAction === "install-media-tools" ||
                        similarityEngine.ffmpegStatus === "installing"
                          ? "Installing…"
                          : "Install private runtime"}
                      </button>
                    )}
                  </div>
                </div>
              </details>
            ) : null}
            {sourceJobs.length || (dedupe?.perceptualSourcesTotal ?? 0) > 0 ? (
              <details
                className="health-source-jobs"
                open={(dedupe?.perceptualSourcesFailed ?? 0) > 0}
              >
                <summary>
                  Per-source similarity jobs
                  <span>
                    {completedSourceJobs} completed
                    {cachedSourceJobs
                      ? ` · ${cachedSourceJobs} cached`
                      : ""}
                    {runningSourceJobs
                      ? ` · ${runningSourceJobs} running`
                      : ""}
                    {queuedSourceJobs
                      ? ` · ${queuedSourceJobs} queued`
                      : ""}
                    {failedSourceJobs
                      ? ` · ${failedSourceJobs} failed`
                      : ""}
                  </span>
                </summary>
                <ul>
                  {sourceJobs.slice(0, 100).map((job) => (
                    <li key={job.sourceId}>
                      <span>
                        <strong>{job.sourcePath.split(/[\\/]/).pop()}</strong>
                        <small title={job.currentPath ?? job.sourcePath}>
                          {job.provider} · {job.stage.replaceAll("_", " ")}
                        </small>
                      </span>
                      <span className={job.status === "failed" ? "health-tone-attention" : undefined}>
                        {job.error ??
                          (job.progressPercent === undefined
                            ? job.status
                            : `${job.progressPercent}%`)}
                      </span>
                    </li>
                  ))}
                </ul>
                {sourceJobs.length > 100 ? (
                  <small>Showing the first 100 source jobs.</small>
                ) : null}
              </details>
            ) : null}
            {!active && scan ? (
              <div className="health-scan-summary">
                <SummaryCard
                  label="Files scanned"
                  value={scan.filesScanned.toLocaleString()}
                  detail={formatBytes(scan.bytesScanned)}
                  severity="healthy"
                />
                <SummaryCard
                  label="Exact groups"
                  value={scan.exactGroupCount.toLocaleString()}
                  detail={`${formatBytes(scan.reclaimableBytes)} reclaimable`}
                  severity={scan.exactGroupCount ? "attention" : "healthy"}
                />
                <SummaryCard
                  label="Similar groups"
                  value={scan.similarGroupCount.toLocaleString()}
                  detail="Review required"
                  severity={scan.similarGroupCount ? "attention" : "healthy"}
                />
                <SummaryCard
                  label="Video similarity"
                  value={`${scan.skippedVideoSimilarityCount} skipped`}
                  detail="FFmpeg/decoding unavailable"
                  severity={
                    scan.skippedVideoSimilarityCount ? "attention" : "healthy"
                  }
                />
              </div>
            ) : null}
          </div>
        </details>
      </section>
      {!active && scan ? (
        <>
          <DedupeResultsList
            exactGroups={scan.exactGroups}
            selections={reviewSelections}
            setSelections={setReviewSelections}
            similarGroups={scan.similarGroups}
          />
          {consolidatableExactGroups.length > 0 ? (
            <button
              className="primary-button health-apply-button"
              disabled={busyAction === "apply-exact"}
              onClick={() => {
                if (
                  window.confirm(
                    `Consolidate ${consolidatableExactGroups.length} exact duplicate groups using hardlinks? All paths will be preserved.`,
                  )
                )
                  void runCleanupAction("apply-exact", () =>
                    applyMediaDedupe({
                      scanId: scan.scanId,
                      consolidateExact: true,
                      similarSelections: [],
                    }),
                  );
              }}
              type="button"
            >
              {busyAction === "apply-exact"
                ? "Starting consolidation…"
                : `Consolidate exact duplicates · ${formatBytes(scan.reclaimableBytes)}`}
            </button>
          ) : null}
          {similarSelections.length > 0 ? (
            <button
              className="danger-button health-apply-button"
              disabled={busyAction === "apply-similar"}
              onClick={() => {
                if (
                  window.confirm(
                    `Move ${similarSelections.reduce((count, item) => count + item.removePaths.length, 0)} reviewed files to the Recycle Bin?`,
                  )
                )
                  void runCleanupAction("apply-similar", () =>
                    applyMediaDedupe({
                      scanId: scan.scanId,
                      consolidateExact: false,
                      similarSelections,
                    }),
                  );
              }}
              type="button"
            >
              {busyAction === "apply-similar"
                ? "Starting cleanup…"
                : "Move reviewed copies to Recycle Bin"}
            </button>
          ) : null}
        </>
      ) : null}
      {!active && !scan ? (
        <div className="health-cleanup-empty empty-state">
          <strong>No scan has been run</strong>
          <p>
            Start a read-only scan to inventory recognized images and videos,
            group candidates, and estimate reclaimable space.
          </p>
        </div>
      ) : null}
      <section className="health-storage-section">
        <div className="panel-header compact-header">
          <div>
            <span className="eyebrow">Capacity</span>
            <h2>Storage volumes</h2>
          </div>
          <span className="pill">{health.volumes.length}</span>
        </div>
        <p className="health-storage-policy">
          Critical is reserved for an unavailable primary root or less than 5 GB
          free. Attention begins below 20 GB.
        </p>
        <div className="health-volume-grid">
          {health.volumes.map((volume) => (
            <StorageVolumeCard key={volume.volumeKey} volume={volume} />
          ))}
        </div>
      </section>
    </div>
  );
}

function StorageVolumeCard({
  volume,
}: {
  volume: WorkspaceHealthSnapshot["volumes"][number];
}) {
  const orderedRoots = [...volume.roots].sort(
    (left, right) =>
      Number(left.accessible) - Number(right.accessible) ||
      Number(right.primary) - Number(left.primary) ||
      left.path.localeCompare(right.path),
  );
  const previewRoots = orderedRoots.slice(0, 4);
  const remainingRoots = orderedRoots.slice(4);
  const unavailableCount = volume.roots.filter(
    (root) => !root.accessible,
  ).length;
  const sourceCount = volume.roots.reduce(
    (count, root) => count + root.sourceCount,
    0,
  );
  const usedPercent = Math.max(0, Math.min(100, 100 - volume.availablePercent));
  const statusClass =
    volume.severity === "healthy"
      ? "status-ready"
      : volume.severity === "attention"
        ? "status-warning"
        : "status-failed";
  return (
    <article className="panel health-volume-card">
      <header>
        <div>
          <span className="eyebrow">Volume</span>
          <h2>{volume.volumeKey}</h2>
        </div>
        <div className="health-volume-status">
          <strong className={`status ${statusClass}`}>
            {severityLabel(volume.severity)}
          </strong>
          <span>{volume.availablePercent.toFixed(1)}% free</span>
        </div>
      </header>
      <div
        aria-label={`${usedPercent.toFixed(1)}% used`}
        aria-valuemax={100}
        aria-valuemin={0}
        aria-valuenow={Math.round(usedPercent)}
        className="health-volume-meter"
        role="progressbar"
      >
        <span
          className={`health-capacity-fill health-tone-${volume.severity}`}
          style={{ width: `${usedPercent}%` }}
        />
      </div>
      <p>
        <strong>{formatBytes(volume.availableBytes)} free</strong>
        <span>
          {formatBytes(volume.usedBytes)} used ·{" "}
          {formatBytes(volume.totalBytes)} total
        </span>
      </p>
      <div className="health-volume-root-summary">
        <span>{volume.roots.length} configured root(s)</span>
        <span>{sourceCount.toLocaleString()} linked sources</span>
        {unavailableCount ? (
          <span className="health-tone-attention">
            {unavailableCount} unavailable
          </span>
        ) : (
          <span className="health-tone-healthy">
            All destinations reachable
          </span>
        )}
      </div>
      <ul>
        {previewRoots.map((root) => (
          <StorageRootRow key={root.path} root={root} />
        ))}
      </ul>
      {remainingRoots.length ? (
        <details className="health-volume-paths">
          <summary>Browse {remainingRoots.length} more path(s)</summary>
          <ul>
            {remainingRoots.map((root) => (
              <StorageRootRow key={root.path} root={root} />
            ))}
          </ul>
        </details>
      ) : null}
    </article>
  );
}

function StorageRootRow({
  root,
}: {
  root: WorkspaceHealthSnapshot["volumes"][number]["roots"][number];
}) {
  return (
    <li>
      <span title={root.path}>{root.path}</span>
      <small className={root.accessible ? undefined : "health-tone-attention"}>
        {root.primary
          ? "Primary root"
          : `${root.sourceCount.toLocaleString()} sources`}{" "}
        · {root.accessible ? "Reachable" : "Unavailable"}
      </small>
    </li>
  );
}

/** File name without its parent directories, mirroring VDF's results list. */
function fileNameOf(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

/** Parent directory of a file path, for the muted secondary line under the name. */
function parentPathOf(path: string): string {
  const parts = path.split(/[\\/]/);
  parts.pop();
  return parts.join(parts[0] === "" ? "/" : "\\") || path;
}

function formatMediaDuration(durationMs?: number): string {
  if (!durationMs || durationMs <= 0) return "—";
  const totalSeconds = Math.round(durationMs / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

function formatResolution(width?: number, height?: number): string {
  if (!width || !height) return "—";
  return `${width}×${height}`;
}

function formatBitrateKbps(bitrateKbps?: number): string {
  if (!bitrateKbps || bitrateKbps <= 0) return "—";
  if (bitrateKbps >= 1000) return `${(bitrateKbps / 1000).toFixed(1)} Mb/s`;
  return `${Math.round(bitrateKbps)} kb/s`;
}

/** Combines video codec + frame rate into one compact column, e.g. "H264 · 29.97fps". */
function formatVideoCodec(videoCodec?: string, frameRate?: number): string {
  const codec = videoCodec ? videoCodec.toUpperCase() : undefined;
  const fps =
    frameRate && frameRate > 0
      ? `${Number.isInteger(frameRate) ? frameRate : frameRate.toFixed(2)}fps`
      : undefined;
  if (codec && fps) return `${codec} · ${fps}`;
  if (codec) return codec;
  if (fps) return fps;
  return "—";
}

function formatModifiedAt(modifiedAt?: number): string {
  if (!modifiedAt) return "—";
  const date = new Date(modifiedAt);
  if (Number.isNaN(date.getTime())) return "—";
  const showYear = date.getFullYear() !== new Date().getFullYear();
  return date.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    ...(showYear ? { year: "numeric" as const } : {}),
  });
}

type DedupeSelectionMap = Record<
  string,
  { keepPath: string; removePaths: string[] }
>;

type DedupeSortKey = "wasted" | "totalSize" | "fileCount" | "matchPercent";
type DedupeSortDir = "desc" | "asc";
type DedupeMediaFilter = "all" | "video" | "image";

const dedupeSortOptions: ReadonlyArray<readonly [DedupeSortKey, string]> = [
  ["wasted", "Wasted space"],
  ["totalSize", "Total size"],
  ["fileCount", "File count"],
  ["matchPercent", "Match %"],
];

function groupMatchPercent(group: MediaDedupeGroup): number {
  return group.kind === "exact" ? 100 : (group.confidencePercent ?? 0);
}

function groupTotalSize(group: MediaDedupeGroup): number {
  return group.files.reduce((sum, file) => sum + file.sizeBytes, 0);
}

function groupMediaType(group: MediaDedupeGroup): "image" | "video" | undefined {
  return group.files[0]?.mediaType;
}

function dedupeSortValue(group: MediaDedupeGroup, key: DedupeSortKey): number {
  switch (key) {
    case "totalSize":
      return groupTotalSize(group);
    case "fileCount":
      return group.files.length;
    case "matchPercent":
      return groupMatchPercent(group);
    case "wasted":
    default:
      return group.reclaimableBytes;
  }
}

/**
 * VDF-inspired unified duplicate results list: exact and similar groups render
 * through the same header + row layout so scan output reads as one list instead
 * of two disconnected sections. Exact groups are consolidated in bulk elsewhere
 * (see the "Consolidate exact duplicates" action) and stay display-only here —
 * they have no per-file keep/recycle selection today. Similar groups keep the
 * existing Keep/Recycle radio+checkbox flow and selection state shape.
 *
 * The toolbar (sort, filters, auto-select, compact rows) and the sticky footer
 * status bar are all client-side — they slice/reorder what the scan already
 * returned and never touch the backend.
 */
function DedupeResultsList({
  exactGroups,
  similarGroups,
  selections,
  setSelections,
}: {
  exactGroups: MediaDedupeGroup[];
  similarGroups: MediaDedupeGroup[];
  selections: DedupeSelectionMap;
  setSelections: Dispatch<SetStateAction<DedupeSelectionMap>>;
}) {
  const [sortKey, setSortKey] = useState<DedupeSortKey>("wasted");
  const [sortDir, setSortDir] = useState<DedupeSortDir>("desc");
  const [mediaFilter, setMediaFilter] = useState<DedupeMediaFilter>("all");
  const [minMatchPercent, setMinMatchPercent] = useState(0);
  const [pathFilter, setPathFilter] = useState("");
  const [onlyWithSelections, setOnlyWithSelections] = useState(false);
  const [compactRows, setCompactRows] = useState(false);

  const combined = useMemo(
    () => [...exactGroups, ...similarGroups],
    [exactGroups, similarGroups],
  );

  const visible = useMemo(() => {
    const needle = pathFilter.trim().toLowerCase();
    const filtered = combined.filter((group) => {
      if (mediaFilter !== "all" && groupMediaType(group) !== mediaFilter)
        return false;
      if (groupMatchPercent(group) < minMatchPercent) return false;
      if (
        needle &&
        !group.files.some((file) => file.path.toLowerCase().includes(needle))
      )
        return false;
      if (
        onlyWithSelections &&
        (selections[group.id]?.removePaths.length ?? 0) === 0
      )
        return false;
      return true;
    });
    const sorted = [...filtered].sort(
      (left, right) => dedupeSortValue(right, sortKey) - dedupeSortValue(left, sortKey),
    );
    if (sortDir === "asc") sorted.reverse();
    return sorted;
  }, [
    combined,
    mediaFilter,
    minMatchPercent,
    onlyWithSelections,
    pathFilter,
    selections,
    sortDir,
    sortKey,
  ]);

  const visibleSimilar = useMemo(
    () => visible.filter((group) => group.kind === "similar"),
    [visible],
  );

  const [compareGroup, setCompareGroup] = useState<MediaDedupeGroup | null>(
    null,
  );

  const footerStats = useMemo(() => {
    const duplicateFiles = visible.reduce(
      (sum, group) => sum + group.files.length,
      0,
    );
    const reclaimable = visible.reduce(
      (sum, group) => sum + group.reclaimableBytes,
      0,
    );
    const selectedForRecycle = visible.reduce(
      (sum, group) => sum + (selections[group.id]?.removePaths.length ?? 0),
      0,
    );
    return { duplicateFiles, reclaimable, selectedForRecycle };
  }, [selections, visible]);

  const applyAutoSelect = useCallback(
    (
      mode:
        | "keep-largest"
        | "keep-oldest"
        | "keep-newest"
        | "keep-best-quality"
        | "clear",
    ) => {
      setSelections((current) => {
        const next = { ...current };
        for (const group of visibleSimilar) {
          if (mode === "clear") {
            delete next[group.id];
            continue;
          }
          let keep: MediaDedupeFile;
          if (mode === "keep-best-quality") {
            keep = pickBestDedupeFile(group.files) ?? group.files[0];
          } else if (mode === "keep-largest") {
            keep = group.files.reduce((best, file) =>
              file.sizeBytes > best.sizeBytes ? file : best,
            );
          } else {
            // Files without a known modifiedAt are never preferred over ones
            // that do — they only win when nothing in the group has a date.
            const dated = group.files.filter(
              (file) => file.modifiedAt != null,
            );
            const pool = dated.length > 0 ? dated : group.files;
            keep = pool.reduce((best, file) => {
              const fileTime = file.modifiedAt ?? 0;
              const bestTime = best.modifiedAt ?? 0;
              return mode === "keep-oldest"
                ? fileTime < bestTime
                  ? file
                  : best
                : fileTime > bestTime
                  ? file
                  : best;
            });
          }
          next[group.id] = {
            keepPath: keep.path,
            removePaths: group.files
              .filter((file) => file.path !== keep.path)
              .map((file) => file.path),
          };
        }
        return next;
      });
    },
    [setSelections, visibleSimilar],
  );

  if (!combined.length) {
    return (
      <section className="health-dedupe-groups">
        <div className="panel-header">
          <div>
            <span className="eyebrow">Results</span>
            <h3>Duplicate candidates</h3>
          </div>
          <span className="pill">0</span>
        </div>
        <p className="health-dedupe-empty">
          No exact or similar duplicates found in this scope.
        </p>
      </section>
    );
  }
  return (
    <section className="health-dedupe-groups">
      <div className="panel-header">
        <div>
          <span className="eyebrow">Results</span>
          <h3>Duplicate candidates</h3>
        </div>
        <span className="pill">{visible.length}</span>
      </div>
      <div className="health-dedupe-toolbar">
        <label className="health-dedupe-toolbar-field">
          <span>Sort by</span>
          <div className="health-dedupe-sort-control">
            <select
              aria-label="Sort duplicate candidates"
              onChange={(event) =>
                setSortKey(event.target.value as DedupeSortKey)
              }
              value={sortKey}
            >
              {dedupeSortOptions.map(([key, label]) => (
                <option key={key} value={key}>
                  {label}
                </option>
              ))}
            </select>
            <button
              aria-label={
                sortDir === "desc" ? "Sort descending" : "Sort ascending"
              }
              className="ghost-button health-dedupe-sort-direction"
              onClick={() =>
                setSortDir((current) => (current === "desc" ? "asc" : "desc"))
              }
              title={sortDir === "desc" ? "Descending" : "Ascending"}
              type="button"
            >
              {sortDir === "desc" ? "↓" : "↑"}
            </button>
          </div>
        </label>
        <label className="health-dedupe-toolbar-field">
          <span>Media type</span>
          <select
            aria-label="Filter by media type"
            onChange={(event) =>
              setMediaFilter(event.target.value as DedupeMediaFilter)
            }
            value={mediaFilter}
          >
            <option value="all">All</option>
            <option value="video">Video</option>
            <option value="image">Image</option>
          </select>
        </label>
        <label
          className="health-dedupe-toolbar-field health-dedupe-toolbar-field-slider"
          title="Hide groups whose files match below this similarity"
        >
          <span className="health-dedupe-toolbar-field-label-row">
            <span>Minimum similarity</span>
            <span className="health-dedupe-toolbar-slider-value">
              ≥ {minMatchPercent}%
            </span>
          </span>
          <input
            aria-label="Minimum similarity"
            max={100}
            min={0}
            onChange={(event) =>
              setMinMatchPercent(
                Math.min(100, Math.max(0, Number(event.target.value) || 0)),
              )
            }
            title="Hide groups whose files match below this similarity"
            type="range"
            value={minMatchPercent}
          />
        </label>
        <label className="health-dedupe-toolbar-field health-dedupe-toolbar-field-grow">
          <span>Path contains</span>
          <input
            aria-label="Filter by path substring"
            onChange={(event) => setPathFilter(event.target.value)}
            placeholder="Filter by folder or profile…"
            type="text"
            value={pathFilter}
          />
        </label>
        <label className="health-dedupe-toolbar-toggle">
          <input
            checked={onlyWithSelections}
            onChange={(event) => setOnlyWithSelections(event.target.checked)}
            type="checkbox"
          />
          Only groups with selections
        </label>
        <label className="health-dedupe-toolbar-toggle">
          <input
            checked={compactRows}
            onChange={(event) => setCompactRows(event.target.checked)}
            type="checkbox"
          />
          Compact rows
        </label>
        <label className="health-dedupe-toolbar-field">
          <span>Auto-select</span>
          <select
            aria-label="Auto-select for similar groups"
            disabled={visibleSimilar.length === 0}
            onChange={(event) => {
              const value = event.target.value;
              event.target.value = "";
              if (value === "keep-best-quality")
                applyAutoSelect("keep-best-quality");
              if (value === "keep-largest") applyAutoSelect("keep-largest");
              if (value === "keep-oldest") applyAutoSelect("keep-oldest");
              if (value === "keep-newest") applyAutoSelect("keep-newest");
              if (value === "clear") applyAutoSelect("clear");
            }}
            title="Applies to visible similar groups only — exact groups are display-only"
            value=""
          >
            <option value="">Choose action…</option>
            <option value="keep-best-quality">Keep best quality</option>
            <option value="keep-largest">Keep largest</option>
            <option value="keep-oldest">Keep oldest</option>
            <option value="keep-newest">Keep newest</option>
            <option value="clear">Clear selections</option>
          </select>
        </label>
      </div>
      {visible.slice(0, 100).map((group, index) => (
        <DedupeGroupCard
          compact={compactRows}
          group={group}
          index={index + 1}
          key={group.id}
          onCompare={setCompareGroup}
          selection={selections[group.id]}
          setSelections={setSelections}
        />
      ))}
      {visible.length > 100 ? (
        <small>Showing the 100 largest visible groups.</small>
      ) : null}
      {compareGroup ? (
        <DedupeCompareModal
          group={compareGroup}
          onClose={() => setCompareGroup(null)}
          selection={selections[compareGroup.id]}
          setSelections={setSelections}
        />
      ) : null}
      <div className="health-dedupe-footer-bar">
        <span>
          <strong>{visible.length.toLocaleString()}</strong> groups
        </span>
        <span>
          <strong>{footerStats.duplicateFiles.toLocaleString()}</strong>{" "}
          duplicate files
        </span>
        <span>
          <strong>{formatBytes(footerStats.reclaimable)}</strong> reclaimable
        </span>
        <span>
          <strong>{footerStats.selectedForRecycle.toLocaleString()}</strong>{" "}
          selected for recycle
        </span>
      </div>
    </section>
  );
}

function DedupeGroupCard({
  group,
  index,
  selection,
  setSelections,
  compact,
  onCompare,
}: {
  group: MediaDedupeGroup;
  index: number;
  selection: { keepPath: string; removePaths: string[] } | undefined;
  setSelections: Dispatch<SetStateAction<DedupeSelectionMap>>;
  compact: boolean;
  onCompare: (group: MediaDedupeGroup) => void;
}) {
  const isExact = group.kind === "exact";
  const matchPercent = isExact ? 100 : (group.confidencePercent ?? 0);
  const resolvedSelection = selection ?? {
    keepPath: group.files[0]?.path ?? "",
    removePaths: [],
  };
  const bestFile = pickBestDedupeFile(group.files);
  return (
    <article className="health-dedupe-group" key={group.id}>
      <header className="health-dedupe-group-header">
        <span className="health-dedupe-group-index">#{index}</span>
        <div className="health-dedupe-group-heading">
          <strong>
            {group.files.length} file{group.files.length === 1 ? "" : "s"}
          </strong>
          <small>
            {formatBytes(
              group.files.reduce((sum, file) => sum + file.sizeBytes, 0),
            )}{" "}
            total · save up to {formatBytes(group.reclaimableBytes)}
          </small>
        </div>
        <span
          className={`pill health-dedupe-match-badge ${isExact ? "health-tone-attention" : ""}`}
        >
          {isExact ? "Exact · 100%" : `Similar · ${matchPercent}%`}
        </span>
        {!isExact && bestFile ? (
          <button
            className="ghost-button health-dedupe-keepbest-button"
            onClick={() =>
              setSelections((current) => ({
                ...current,
                [group.id]: {
                  keepPath: bestFile.path,
                  removePaths: group.files
                    .filter((file) => file.path !== bestFile.path)
                    .map((file) => file.path),
                },
              }))
            }
            title="Keep the highest-quality file (resolution, then bitrate, then size, then most recent) and recycle the rest"
            type="button"
          >
            Keep best
          </button>
        ) : null}
        <button
          className="ghost-button health-dedupe-compare-button"
          onClick={() => onCompare(group)}
          type="button"
        >
          Compare
        </button>
      </header>
      <div
        className={
          compact ? "health-dedupe-rows is-compact" : "health-dedupe-rows"
        }
      >
        {group.files.map((file) => {
          const keep = !isExact && resolvedSelection.keepPath === file.path;
          const remove =
            !isExact && resolvedSelection.removePaths.includes(file.path);
          const rowClass = [
            "health-dedupe-row",
            keep ? "is-keep" : remove ? "is-remove" : "",
            compact ? "is-compact" : "",
          ]
            .filter(Boolean)
            .join(" ");
          return (
            <div className={rowClass} key={file.path}>
              {file.mediaType === "image" ? (
                <img
                  alt=""
                  className="health-dedupe-row-thumb"
                  src={convertFileSrc(file.path)}
                />
              ) : file.thumbnailPath ? (
                <img
                  alt=""
                  className="health-dedupe-row-thumb"
                  src={convertFileSrc(file.thumbnailPath)}
                />
              ) : (
                <div className="health-dedupe-row-thumb health-video-placeholder">
                  VIDEO
                </div>
              )}
              <div className="health-dedupe-row-name">
                <span className="health-dedupe-row-name-line">
                  <strong title={file.path}>{fileNameOf(file.path)}</strong>
                  {bestFile && file.path === bestFile.path ? (
                    <span
                      className="health-dedupe-best-badge"
                      title="Best quality in this group: highest resolution, then bitrate, then size, then most recent"
                    >
                      BEST
                    </span>
                  ) : null}
                </span>
                {compact ? null : (
                  <small title={file.path}>{parentPathOf(file.path)}</small>
                )}
              </div>
              <span className="health-dedupe-row-col">
                {formatMediaDuration(file.durationMs)}
              </span>
              <span className="health-dedupe-row-col">
                {formatResolution(file.width, file.height)}
              </span>
              {compact ? null : (
                <>
                  <span
                    className="health-dedupe-row-col"
                    title="Bitrate (ffprobe)"
                  >
                    {formatBitrateKbps(file.bitrateKbps)}
                  </span>
                  <span
                    className="health-dedupe-row-col"
                    title="Video codec / frame rate (ffprobe)"
                  >
                    {formatVideoCodec(file.videoCodec, file.frameRate)}
                  </span>
                  <span
                    className="health-dedupe-row-col"
                    title="Audio stream (ffprobe)"
                  >
                    {file.audioSummary ?? "—"}
                  </span>
                </>
              )}
              <span className="health-dedupe-row-col">
                {formatBytes(file.sizeBytes)}
              </span>
              <span className="health-dedupe-row-col">
                {formatModifiedAt(file.modifiedAt)}
              </span>
              <span className="health-dedupe-row-col">{matchPercent}%</span>
              {isExact ? (
                <span className="health-dedupe-row-selection health-dedupe-row-selection-placeholder" />
              ) : (
                <div className="health-dedupe-row-selection">
                  <label>
                    <input
                      checked={keep}
                      name={`keep-${group.id}`}
                      onChange={() =>
                        setSelections((current) => ({
                          ...current,
                          [group.id]: {
                            keepPath: file.path,
                            removePaths: (
                              current[group.id]?.removePaths ?? []
                            ).filter((path) => path !== file.path),
                          },
                        }))
                      }
                      type="radio"
                    />{" "}
                    Keep
                  </label>
                  <label>
                    <input
                      checked={remove}
                      disabled={keep}
                      onChange={(event) =>
                        setSelections((current) => {
                          const existing =
                            current[group.id] ?? resolvedSelection;
                          return {
                            ...current,
                            [group.id]: {
                              ...existing,
                              removePaths: event.target.checked
                                ? [...existing.removePaths, file.path]
                                : existing.removePaths.filter(
                                    (path) => path !== file.path,
                                  ),
                            },
                          };
                        })
                      }
                      type="checkbox"
                    />{" "}
                    Recycle
                  </label>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </article>
  );
}

/**
 * Side-by-side compare view for one duplicate group (VDF-inspired): a large
 * preview + full metadata per file, with the same Keep/Recycle controls as the
 * row list. Similar groups can be resolved here; exact groups are display-only
 * (they're consolidated in bulk elsewhere, same as the row list).
 */
function DedupeCompareModal({
  group,
  selection,
  setSelections,
  onClose,
}: {
  group: MediaDedupeGroup;
  selection: { keepPath: string; removePaths: string[] } | undefined;
  setSelections: Dispatch<SetStateAction<DedupeSelectionMap>>;
  onClose: () => void;
}) {
  const isExact = group.kind === "exact";
  const matchPercent = isExact ? 100 : (group.confidencePercent ?? 0);
  const resolvedSelection = selection ?? {
    keepPath: group.files[0]?.path ?? "",
    removePaths: [],
  };
  const bestFile = pickBestDedupeFile(group.files);

  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose]);

  return (
    <div
      aria-label={`Compare ${group.files.length} files`}
      aria-modal="true"
      className="health-dedupe-compare-backdrop"
      onClick={onClose}
      role="dialog"
    >
      <div
        className="health-dedupe-compare-modal"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="health-dedupe-compare-header">
          <div>
            <span className="eyebrow">
              {isExact ? "Exact · 100%" : `Similar · ${matchPercent}%`}
            </span>
            <h3>
              {group.files.length} file{group.files.length === 1 ? "" : "s"} ·
              save up to {formatBytes(group.reclaimableBytes)}
            </h3>
          </div>
          <button
            aria-label="Close compare view"
            className="health-dedupe-compare-close"
            onClick={onClose}
            type="button"
          >
            ✕
          </button>
        </header>
        <div className="health-dedupe-compare-grid">
          {group.files.map((file) => {
            const keep = !isExact && resolvedSelection.keepPath === file.path;
            const remove =
              !isExact && resolvedSelection.removePaths.includes(file.path);
            return (
              <article
                className={[
                  "health-dedupe-compare-item",
                  keep ? "is-keep" : remove ? "is-remove" : "",
                ]
                  .filter(Boolean)
                  .join(" ")}
                key={file.path}
              >
                <div className="health-dedupe-compare-preview">
                  {file.mediaType === "video" ? (
                    <video controls src={convertFileSrc(file.path)} />
                  ) : (
                    <img alt="" src={convertFileSrc(file.path)} />
                  )}
                </div>
                <dl className="health-dedupe-compare-meta">
                  <dt>Name</dt>
                  <dd title={file.path}>
                    {fileNameOf(file.path)}
                    {bestFile && file.path === bestFile.path ? (
                      <span
                        className="health-dedupe-best-badge"
                        title="Best quality in this group: highest resolution, then bitrate, then size, then most recent"
                      >
                        BEST
                      </span>
                    ) : null}
                  </dd>
                  <dt>Path</dt>
                  <dd title={file.path}>{parentPathOf(file.path)}</dd>
                  <dt>Size</dt>
                  <dd>{formatBytes(file.sizeBytes)}</dd>
                  <dt>Resolution</dt>
                  <dd>{formatResolution(file.width, file.height)}</dd>
                  <dt>Duration</dt>
                  <dd>{formatMediaDuration(file.durationMs)}</dd>
                  <dt>Bitrate</dt>
                  <dd>{formatBitrateKbps(file.bitrateKbps)}</dd>
                  <dt>Codec</dt>
                  <dd>{formatVideoCodec(file.videoCodec, file.frameRate)}</dd>
                  <dt>Audio</dt>
                  <dd>{file.audioSummary ?? "—"}</dd>
                  <dt>Modified</dt>
                  <dd>{formatModifiedAt(file.modifiedAt)}</dd>
                  <dt>Match</dt>
                  <dd>{matchPercent}%</dd>
                </dl>
                {isExact ? (
                  <p className="health-dedupe-compare-readonly">
                    Exact duplicates are consolidated in bulk — no per-file
                    selection here.
                  </p>
                ) : (
                  <div className="health-dedupe-row-selection health-dedupe-compare-selection">
                    <label>
                      <input
                        checked={keep}
                        name={`compare-keep-${group.id}`}
                        onChange={() =>
                          setSelections((current) => ({
                            ...current,
                            [group.id]: {
                              keepPath: file.path,
                              removePaths: (
                                current[group.id]?.removePaths ?? []
                              ).filter((path) => path !== file.path),
                            },
                          }))
                        }
                        type="radio"
                      />{" "}
                      Keep
                    </label>
                    <label>
                      <input
                        checked={remove}
                        disabled={keep}
                        onChange={(event) =>
                          setSelections((current) => {
                            const existing =
                              current[group.id] ?? resolvedSelection;
                            return {
                              ...current,
                              [group.id]: {
                                ...existing,
                                removePaths: event.target.checked
                                  ? [...existing.removePaths, file.path]
                                  : existing.removePaths.filter(
                                      (path) => path !== file.path,
                                    ),
                              },
                            };
                          })
                        }
                        type="checkbox"
                      />{" "}
                      Recycle
                    </label>
                  </div>
                )}
              </article>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function IncidentDrawer({
  incident,
  onClose,
  onOpenStorage,
  busyAction,
  runAction,
}: {
  incident: WorkspaceHealthIncident;
  onClose: () => void;
  onOpenStorage: () => void;
  busyAction?: string;
  runAction: (key: string, action: () => Promise<unknown>) => Promise<void>;
}) {
  const hasAction = (action: string) =>
    incident.availableActions.includes(action);
  return (
    <aside
      aria-labelledby="health-incident-title"
      aria-modal="true"
      className="health-incident-drawer"
      role="dialog"
    >
      <header>
        <div>
          <span className={`pill health-tone-${incident.severity}`}>
            {severityLabel(incident.severity)}
          </span>
          <h2 id="health-incident-title">{incident.title}</h2>
        </div>
        <button
          aria-label="Close incident"
          autoFocus
          onClick={onClose}
          type="button"
        >
          ×
        </button>
      </header>
      <p>{incident.detail}</p>
      <section>
        <span className="eyebrow">Evidence</span>
        <ul>
          {incident.evidence.map((item) => (
            <li key={item}>{item}</li>
          ))}
        </ul>
      </section>
      <footer>
        {incident.sourceId && hasAction("retry_sync") ? (
          <button
            className="primary-button"
            disabled={busyAction === `sync:${incident.sourceId}`}
            onClick={() =>
              void runAction(`sync:${incident.sourceId}`, () =>
                runSourceSync(incident.sourceId!),
              )
            }
            type="button"
          >
            Retry sync
          </button>
        ) : null}
        {incident.sourceId && hasAction("open_profile") ? (
          <button
            className="ghost-button"
            onClick={() => void openProfileViewWindow(incident.sourceId!)}
            type="button"
          >
            Open profile
          </button>
        ) : null}
        {incident.sourceId && hasAction("open_folder") ? (
          <button
            className="ghost-button"
            onClick={() => void openSourceFolder(incident.sourceId!)}
            type="button"
          >
            Open folder
          </button>
        ) : null}
        {incident.accountId && hasAction("validate_account") ? (
          <button
            className="primary-button"
            disabled={busyAction === `validate:${incident.accountId}`}
            onClick={() =>
              void runAction(`validate:${incident.accountId}`, () =>
                validateProviderAccount(incident.accountId!),
              )
            }
            type="button"
          >
            Validate account
          </button>
        ) : null}
        {incident.accountId && hasAction("open_account") ? (
          <button
            className="ghost-button"
            onClick={() =>
              void openAccountsWindow({
                initialAccountId: incident.accountId,
                initialMode: "edit",
              })
            }
            type="button"
          >
            Open account
          </button>
        ) : null}
        {hasAction("open_filtered_log") ? (
          <button
            className="ghost-button"
            onClick={() =>
              void openRuntimeLogWindow({
                sourceId: incident.sourceId,
                accountId: incident.accountId,
              })
            }
            type="button"
          >
            Open log
          </button>
        ) : null}
        {hasAction("open_storage_cleanup") ? (
          <button
            className="primary-button"
            onClick={onOpenStorage}
            type="button"
          >
            Open Storage &amp; Cleanup
          </button>
        ) : null}
      </footer>
    </aside>
  );
}
