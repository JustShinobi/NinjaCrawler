// @vitest-environment jsdom

import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MediaDedupeFile, WorkspaceHealthSnapshot } from "../../domain/models";
import { pickBestDedupeFile } from "./pickBestDedupeFile";
import { WorkspaceHealthWindowPage } from "./WorkspaceHealthWindowPage";

const bridgeMocks = vi.hoisted(() => ({
  applyMediaDedupe: vi.fn(),
  cancelMediaDedupe: vi.fn(),
  enqueueMediaDedupeScan: vi.fn(),
  installMediaDedupeSimilarityEngine: vi.fn(),
  installMediaToolRuntime: vi.fn(),
  loadMediaDedupeStatus: vi.fn(),
  loadWorkspaceHealth: vi.fn(),
  openAccountsWindow: vi.fn(),
  openProfileViewWindow: vi.fn(),
  openRuntimeLogWindow: vi.fn(),
  openSourceFolder: vi.fn(),
  runSourceSync: vi.fn(),
  subscribeToDesktopRuntimeEvents: vi.fn(),
  subscribeToWorkspaceHealthWindowIntent: vi.fn(),
  validateProviderAccount: vi.fn(),
}));

vi.mock("../../bridge/desktop", () => bridgeMocks);
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => path,
}));

function healthFixture(): WorkspaceHealthSnapshot {
  return {
    overallStatus: "critical",
    generatedAt: "2026-07-17T12:00:00Z",
    counts: {
      sourceCount: 1,
      affectedSourceCount: 1,
      recurringFailureCount: 1,
      degradedAccountCount: 1,
      criticalAccountCount: 0,
      storageAttentionCount: 1,
      criticalIssueCount: 1,
      attentionIssueCount: 2,
    },
    incidents: [
      {
        id: "storage:C:",
        severity: "critical",
        kind: "storage",
        title: "Media storage C: needs attention",
        detail: "4.0 GB available.",
        volumeKey: "C:",
        evidence: ["C:\\Media"],
        availableActions: ["open_storage_cleanup"],
      },
    ],
    sources: [
      {
        sourceId: "source-1",
        provider: "instagram",
        handle: "profile",
        displayName: "Profile",
        accountId: "account-1",
        lastSyncedAt: "2026-07-16T12:00:00Z",
        latestStatus: "failed",
        consecutiveFailures: 3,
        recurringFailure: true,
        freshness: "stale",
        severity: "attention",
        recentRuns: [],
      },
    ],
    accounts: [
      {
        accountId: "account-1",
        provider: "instagram",
        displayName: "Instagram Main",
        authState: "degraded",
        hasSession: true,
        hasSecret: true,
        lastValidatedAt: "2026-07-17T11:00:00Z",
        impactedSourceCount: 1,
        severity: "attention",
      },
    ],
    volumes: [
      {
        volumeKey: "C:",
        totalBytes: 100 * 1024 ** 3,
        availableBytes: 4 * 1024 ** 3,
        usedBytes: 96 * 1024 ** 3,
        availablePercent: 4,
        severity: "critical",
        roots: [
          {
            path: "C:\\Media",
            sourceCount: 1,
            primary: true,
            accessible: true,
          },
        ],
      },
    ],
  };
}

describe("WorkspaceHealthWindowPage", () => {
  beforeEach(() => {
    for (const mock of Object.values(bridgeMocks)) mock.mockReset();
    bridgeMocks.loadWorkspaceHealth.mockResolvedValue(healthFixture());
    bridgeMocks.loadMediaDedupeStatus.mockResolvedValue({
      state: "idle",
      stage: "idle",
      filesProcessed: 0,
      filesTotal: 0,
      bytesProcessed: 0,
      bytesTotal: 0,
      cancellable: false,
      similarityEngine: {
        status: "not_installed",
        version: "4.1.x+test",
        installed: false,
        ffmpegAvailable: true,
        ffmpegStatus: "ready",
        ffmpegSource: "system",
      },
      perceptualSourcesProcessed: 0,
      perceptualSourcesTotal: 0,
      perceptualSourcesFailed: 0,
      sourceJobs: [],
      updatedAt: "",
    });
    bridgeMocks.subscribeToDesktopRuntimeEvents.mockResolvedValue(
      () => undefined,
    );
    bridgeMocks.subscribeToWorkspaceHealthWindowIntent.mockResolvedValue(
      () => undefined,
    );
    bridgeMocks.enqueueMediaDedupeScan.mockResolvedValue({});
    bridgeMocks.installMediaDedupeSimilarityEngine.mockResolvedValue({});
    bridgeMocks.installMediaToolRuntime.mockResolvedValue({});
  });

  afterEach(() => cleanup());

  it("shows prioritized evidence and opens the incident drawer", async () => {
    render(<WorkspaceHealthWindowPage />);

    fireEvent.click(
      await screen.findByRole("button", {
        name: /media storage c: needs attention/i,
      }),
    );

    expect(
      screen.getByRole("dialog", { name: /media storage c: needs attention/i }),
    ).toBeTruthy();
    expect(screen.getByText("C:\\Media")).toBeTruthy();
    fireEvent.click(
      screen.getByRole("button", { name: /^open storage & cleanup$/i }),
    );
    expect(
      screen.getByRole("heading", { name: /duplicate media/i }),
    ).toBeTruthy();
  });

  it("keeps cleanup on demand in Storage & Cleanup", async () => {
    render(<WorkspaceHealthWindowPage />);

    fireEvent.click(
      await screen.findByRole("tab", { name: /storage & cleanup/i }),
    );
    const cleanupHeading = screen.getByRole("heading", {
      name: /find duplicate media/i,
    });
    const volumesHeading = screen.getByRole("heading", {
      name: /storage volumes/i,
    });
    expect(
      cleanupHeading.compareDocumentPosition(volumesHeading) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(document.querySelector(".health-window-shell-content")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /scan library/i }));

    await waitFor(() =>
      expect(bridgeMocks.enqueueMediaDedupeScan).toHaveBeenCalledTimes(1),
    );
    expect(bridgeMocks.enqueueMediaDedupeScan).toHaveBeenCalledWith({
      resourceProfile: "balanced",
      scanProfile: "recommended",
    });
    expect(screen.getByText(/no scan has been run/i)).toBeTruthy();
  });

  it("opens directly on Storage & Cleanup when launched from Profile View", async () => {
    render(<WorkspaceHealthWindowPage initialIntent={{ initialTab: "storage" }} />);

    expect(
      await screen.findByRole("heading", { name: /find duplicate media/i }),
    ).toBeTruthy();
    expect(
      screen.getByRole("tab", { name: /storage & cleanup/i }).getAttribute("aria-selected"),
    ).toBe("true");
  });

  it("limits a media scan to the selected provider", async () => {
    render(<WorkspaceHealthWindowPage />);
    fireEvent.click(
      await screen.findByRole("tab", { name: /storage & cleanup/i }),
    );
    fireEvent.change(screen.getByRole("combobox", { name: /media scan provider/i }), {
      target: { value: "instagram" },
    });
    fireEvent.click(screen.getByRole("button", { name: /scan library/i }));

    await waitFor(() =>
      expect(bridgeMocks.enqueueMediaDedupeScan).toHaveBeenCalledWith({
        provider: "instagram",
        resourceProfile: "balanced",
        scanProfile: "recommended",
      }),
    );
  });

  it("selects a resource profile and keeps similarity scope explicit", async () => {
    render(<WorkspaceHealthWindowPage />);
    fireEvent.click(
      await screen.findByRole("tab", { name: /storage & cleanup/i }),
    );
    fireEvent.change(
      screen.getByRole("combobox", { name: /media scan resource use/i }),
      { target: { value: "fast" } },
    );
    fireEvent.click(screen.getByRole("button", { name: /scan library/i }));

    await waitFor(() =>
      expect(bridgeMocks.enqueueMediaDedupeScan).toHaveBeenCalledWith({
        resourceProfile: "fast",
        scanProfile: "recommended",
      }),
    );
    expect(screen.getByText(/vdf compares videos within each source/i)).toBeTruthy();
  });

  it("selects a deeper scan profile for the media scan", async () => {
    render(<WorkspaceHealthWindowPage />);
    fireEvent.click(
      await screen.findByRole("tab", { name: /storage & cleanup/i }),
    );
    fireEvent.change(
      screen.getByRole("combobox", { name: /media scan profile/i }),
      { target: { value: "deep" } },
    );
    fireEvent.click(screen.getByRole("button", { name: /scan library/i }));

    await waitFor(() =>
      expect(bridgeMocks.enqueueMediaDedupeScan).toHaveBeenCalledWith({
        resourceProfile: "balanced",
        scanProfile: "deep",
      }),
    );
  });

  it("offers the managed similarity runtime without blocking exact scans", async () => {
    render(<WorkspaceHealthWindowPage />);

    fireEvent.click(
      await screen.findByRole("tab", { name: /storage & cleanup/i }),
    );
    expect(screen.getByRole("button", { name: /scan library/i })).toBeTruthy();
    fireEvent.click(
      screen.getByRole("button", { name: /install similarity engine/i }),
    );

    await waitFor(() =>
      expect(
        bridgeMocks.installMediaDedupeSimilarityEngine,
      ).toHaveBeenCalledTimes(1),
    );
  });

  it("keeps ready runtime details collapsed until the operator expands them", async () => {
    bridgeMocks.loadMediaDedupeStatus.mockResolvedValue({
      state: "idle",
      stage: "idle",
      filesProcessed: 0,
      filesTotal: 0,
      bytesProcessed: 0,
      bytesTotal: 0,
      cancellable: false,
      similarityEngine: {
        status: "ready",
        version: "4.1.x+test",
        installed: true,
        ffmpegAvailable: true,
        ffmpegStatus: "ready",
        ffmpegSource: "system",
        ffmpegVersion: "8.1.2",
      },
      perceptualSourcesProcessed: 0,
      perceptualSourcesTotal: 0,
      perceptualSourcesFailed: 0,
      sourceJobs: [],
      updatedAt: "",
    });
    render(<WorkspaceHealthWindowPage />);
    fireEvent.click(
      await screen.findByRole("tab", { name: /storage & cleanup/i }),
    );

    const summary = screen.getByText(/scan engine & media tools/i).closest("summary");
    const disclosure = summary?.closest("details");
    expect(disclosure?.hasAttribute("open")).toBe(false);
    fireEvent.click(summary!);
    expect(disclosure?.hasAttribute("open")).toBe(true);
  });

  it("offers a private FFmpeg runtime when system tools are missing", async () => {
    bridgeMocks.loadMediaDedupeStatus.mockResolvedValue({
      state: "idle",
      stage: "idle",
      filesProcessed: 0,
      filesTotal: 0,
      bytesProcessed: 0,
      bytesTotal: 0,
      cancellable: false,
      similarityEngine: {
        status: "ready",
        version: "4.1.x+test",
        installed: true,
        ffmpegAvailable: false,
        ffmpegStatus: "not_installed",
      },
      perceptualSourcesProcessed: 0,
      perceptualSourcesTotal: 0,
      perceptualSourcesFailed: 0,
      sourceJobs: [],
      updatedAt: "",
    });
    render(<WorkspaceHealthWindowPage />);
    fireEvent.click(
      await screen.findByRole("tab", { name: /storage & cleanup/i }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: /install private runtime/i }),
    );

    await waitFor(() =>
      expect(bridgeMocks.installMediaToolRuntime).toHaveBeenCalledTimes(1),
    );
  });

  it("keeps health usable when the cleanup backend is unavailable", async () => {
    bridgeMocks.loadMediaDedupeStatus.mockRejectedValue(
      "no such table: media_dedupe_scans",
    );
    render(<WorkspaceHealthWindowPage />);

    expect(
      await screen.findByText(/^workspace health$/i, {
        selector: ".window-titlebar-title-text",
      }),
    ).toBeTruthy();
    expect(screen.getByText(/no such table: media_dedupe_scans/i)).toBeTruthy();

    fireEvent.click(screen.getByRole("tab", { name: /storage & cleanup/i }));
    expect(
      screen
        .getByRole("button", { name: /scan library/i })
        .hasAttribute("disabled"),
    ).toBe(true);
  });

  it("shows string errors returned by the desktop command", async () => {
    bridgeMocks.loadWorkspaceHealth.mockRejectedValue(
      "workspace database is locked",
    );
    render(<WorkspaceHealthWindowPage />);

    expect(
      await screen.findByText("workspace database is locked"),
    ).toBeTruthy();
  });

  it("exposes source filters and account actions", async () => {
    render(<WorkspaceHealthWindowPage />);

    fireEvent.click(await screen.findByRole("tab", { name: /^sources$/i }));
    expect(
      screen.getByRole("combobox", { name: /source health filter/i }),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: /^accounts$/i }));
    expect(screen.getByRole("button", { name: /^validate$/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /open account/i })).toBeTruthy();
  });

  it("gives immediate indeterminate feedback while a library scan starts", async () => {
    let resolveScan: (
      status: Awaited<ReturnType<typeof bridgeMocks.enqueueMediaDedupeScan>>,
    ) => void = () => undefined;
    bridgeMocks.enqueueMediaDedupeScan.mockReturnValue(
      new Promise((resolve) => {
        resolveScan = resolve;
      }),
    );
    render(<WorkspaceHealthWindowPage />);

    fireEvent.click(
      await screen.findByRole("tab", { name: /storage & cleanup/i }),
    );
    fireEvent.click(screen.getByRole("button", { name: /scan library/i }));

    expect(
      screen
        .getByRole("button", { name: /starting scan/i })
        .hasAttribute("disabled"),
    ).toBe(true);
    expect(
      screen.getByRole("status", { name: /media cleanup progress/i })
        .textContent,
    ).toContain("Starting library scan");
    expect(
      screen
        .getByRole("progressbar", { name: /starting library scan/i })
        .classList.contains("indeterminate"),
    ).toBe(true);

    await act(async () =>
      resolveScan({
        state: "queued",
        stage: "inventory",
        filesProcessed: 0,
        filesTotal: 0,
        bytesProcessed: 0,
        bytesTotal: 0,
        cancellable: true,
        updatedAt: "2026-07-17T12:01:00Z",
      }),
    );
  });

  it("shows ffprobe metadata columns and opens the compare view", async () => {
    bridgeMocks.loadMediaDedupeStatus.mockResolvedValue({
      state: "idle",
      stage: "idle",
      filesProcessed: 0,
      filesTotal: 0,
      bytesProcessed: 0,
      bytesTotal: 0,
      cancellable: false,
      similarityEngine: {
        status: "ready",
        version: "4.1.x+test",
        installed: true,
        ffmpegAvailable: true,
        ffmpegStatus: "ready",
        ffmpegSource: "system",
      },
      perceptualSourcesProcessed: 0,
      perceptualSourcesTotal: 0,
      perceptualSourcesFailed: 0,
      sourceJobs: [],
      updatedAt: "",
      latestScan: {
        scanId: "scan-1",
        resourceProfile: "balanced",
        similarityScope: "source",
        status: "completed",
        filesScanned: 2,
        bytesScanned: 200,
        exactGroupCount: 0,
        similarGroupCount: 1,
        reclaimableBytes: 100,
        skippedVideoSimilarityCount: 0,
        startedAt: "2026-07-17T12:00:00Z",
        exactGroups: [],
        similarGroups: [
          {
            id: "similar:source-1:0",
            kind: "similar",
            confidencePercent: 92,
            reclaimableBytes: 100,
            consolidatable: false,
            files: [
              {
                path: "C:\\Media\\a.mp4",
                sourceId: "source-1",
                provider: "instagram",
                mediaType: "video",
                sizeBytes: 200,
                durationMs: 5000,
                bitrateKbps: 3512,
                videoCodec: "h264",
                frameRate: 29.97,
                audioSummary: "aac (stereo)",
              },
              {
                path: "C:\\Media\\b.mp4",
                sourceId: "source-1",
                provider: "instagram",
                mediaType: "video",
                sizeBytes: 100,
                durationMs: 5000,
                bitrateKbps: 1200,
                videoCodec: "vp9",
                frameRate: 30,
                audioSummary: "No audio",
              },
            ],
          },
        ],
      },
    });
    render(<WorkspaceHealthWindowPage />);

    fireEvent.click(
      await screen.findByRole("tab", { name: /storage & cleanup/i }),
    );

    expect(await screen.findByText("H264 · 29.97fps")).toBeTruthy();
    expect(screen.getByText("3.5 Mb/s")).toBeTruthy();
    expect(screen.getByText("aac (stereo)")).toBeTruthy();
    expect(screen.getByText("No audio")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /^compare$/i }));
    const dialog = screen.getByRole("dialog", { name: /compare 2 files/i });
    expect(dialog).toBeTruthy();
    expect(dialog.textContent).toContain("VP9 · 30fps");

    fireEvent.keyDown(window, { key: "Escape" });
    expect(
      screen.queryByRole("dialog", { name: /compare 2 files/i }),
    ).toBeNull();
  });

  it("uses dedicated scroll regions for large tabs", async () => {
    render(<WorkspaceHealthWindowPage />);

    expect(
      (
        await screen.findByRole("tabpanel", { name: /overview/i })
      ).querySelector(".health-scroll-region"),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: /^sources$/i }));
    expect(
      screen
        .getByRole("tabpanel", { name: /sources/i })
        .querySelector(".health-source-virtual.health-scroll-region"),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: /storage & cleanup/i }));
    expect(
      screen
        .getByRole("tabpanel", { name: /storage & cleanup/i })
        .classList.contains("health-scroll-region"),
    ).toBe(true);
  });

  describe("pickBestDedupeFile", () => {
    function file(overrides: Partial<MediaDedupeFile>): MediaDedupeFile {
      return {
        path: "C:\\Media\\file.mp4",
        mediaType: "video",
        sizeBytes: 100,
        ...overrides,
      };
    }

    it("prefers higher resolution first", () => {
      const low = file({ path: "low", width: 640, height: 480 });
      const high = file({ path: "high", width: 1920, height: 1080 });
      expect(pickBestDedupeFile([low, high])?.path).toBe("high");
    });

    it("falls back to bitrate when resolution ties", () => {
      const low = file({
        path: "low-bitrate",
        width: 1920,
        height: 1080,
        bitrateKbps: 1200,
      });
      const high = file({
        path: "high-bitrate",
        width: 1920,
        height: 1080,
        bitrateKbps: 4500,
      });
      expect(pickBestDedupeFile([low, high])?.path).toBe("high-bitrate");
    });

    it("falls back to size when resolution and bitrate tie", () => {
      const small = file({ path: "small", sizeBytes: 100 });
      const big = file({ path: "big", sizeBytes: 5000 });
      expect(pickBestDedupeFile([small, big])?.path).toBe("big");
    });

    it("falls back to the newest modifiedAt when all else ties", () => {
      const older = file({ path: "older", sizeBytes: 100, modifiedAt: 1000 });
      const newer = file({ path: "newer", sizeBytes: 100, modifiedAt: 2000 });
      expect(pickBestDedupeFile([older, newer])?.path).toBe("newer");
    });

    it("never lets a file with missing metrics beat one that has them", () => {
      const unknown = file({ path: "unknown", sizeBytes: 100 });
      const known = file({
        path: "known",
        sizeBytes: 100,
        modifiedAt: 500,
      });
      expect(pickBestDedupeFile([unknown, known])?.path).toBe("known");
    });

    it("returns undefined for an empty group", () => {
      expect(pickBestDedupeFile([])).toBeUndefined();
    });
  });

  it("marks the best-quality file and applies keep-best selection per group", async () => {
    bridgeMocks.loadMediaDedupeStatus.mockResolvedValue({
      state: "idle",
      stage: "idle",
      filesProcessed: 0,
      filesTotal: 0,
      bytesProcessed: 0,
      bytesTotal: 0,
      cancellable: false,
      similarityEngine: {
        status: "ready",
        version: "4.1.x+test",
        installed: true,
        ffmpegAvailable: true,
        ffmpegStatus: "ready",
        ffmpegSource: "system",
      },
      perceptualSourcesProcessed: 0,
      perceptualSourcesTotal: 0,
      perceptualSourcesFailed: 0,
      sourceJobs: [],
      updatedAt: "",
      latestScan: {
        scanId: "scan-1",
        resourceProfile: "balanced",
        similarityScope: "source",
        status: "completed",
        filesScanned: 2,
        bytesScanned: 300,
        exactGroupCount: 0,
        similarGroupCount: 1,
        reclaimableBytes: 100,
        skippedVideoSimilarityCount: 0,
        startedAt: "2026-07-17T12:00:00Z",
        exactGroups: [],
        similarGroups: [
          {
            id: "similar:source-1:0",
            kind: "similar",
            confidencePercent: 92,
            reclaimableBytes: 100,
            consolidatable: false,
            files: [
              {
                path: "C:\\Media\\low.mp4",
                sourceId: "source-1",
                provider: "instagram",
                mediaType: "video",
                sizeBytes: 100,
                width: 640,
                height: 480,
              },
              {
                path: "C:\\Media\\high.mp4",
                sourceId: "source-1",
                provider: "instagram",
                mediaType: "video",
                sizeBytes: 200,
                width: 1920,
                height: 1080,
              },
            ],
          },
        ],
      },
    });
    render(<WorkspaceHealthWindowPage />);

    fireEvent.click(
      await screen.findByRole("tab", { name: /storage & cleanup/i }),
    );

    expect(await screen.findByText("BEST")).toBeTruthy();
    const highRow = screen.getByText("high.mp4").closest(".health-dedupe-row");
    expect(highRow?.querySelector(".health-dedupe-best-badge")).toBeTruthy();
    const lowRow = screen.getByText("low.mp4").closest(".health-dedupe-row");
    expect(lowRow?.querySelector(".health-dedupe-best-badge")).toBeFalsy();

    fireEvent.click(screen.getByRole("button", { name: /keep best/i }));

    const highKeepRadio = highRow?.querySelector(
      'input[type="radio"]',
    ) as HTMLInputElement;
    const lowRecycleCheckbox = lowRow?.querySelector(
      'input[type="checkbox"]',
    ) as HTMLInputElement;
    expect(highKeepRadio.checked).toBe(true);
    expect(lowRecycleCheckbox.checked).toBe(true);
  });

  it("collapses scan details by default and uses a similarity slider with a neutral path placeholder", async () => {
    bridgeMocks.loadMediaDedupeStatus.mockResolvedValue({
      state: "idle",
      stage: "idle",
      filesProcessed: 0,
      filesTotal: 0,
      bytesProcessed: 0,
      bytesTotal: 0,
      cancellable: false,
      similarityEngine: {
        status: "ready",
        version: "4.1.x+test",
        installed: true,
        ffmpegAvailable: true,
        ffmpegStatus: "ready",
        ffmpegSource: "system",
      },
      perceptualSourcesProcessed: 0,
      perceptualSourcesTotal: 0,
      perceptualSourcesFailed: 0,
      sourceJobs: [],
      updatedAt: "",
      latestScan: {
        scanId: "scan-1",
        resourceProfile: "balanced",
        similarityScope: "source",
        status: "completed",
        filesScanned: 2,
        bytesScanned: 200,
        exactGroupCount: 0,
        similarGroupCount: 1,
        reclaimableBytes: 100,
        skippedVideoSimilarityCount: 0,
        startedAt: "2026-07-17T12:00:00Z",
        finishedAt: "2026-07-17T12:05:00Z",
        exactGroups: [],
        similarGroups: [
          {
            id: "similar:source-1:0",
            kind: "similar",
            confidencePercent: 92,
            reclaimableBytes: 100,
            consolidatable: false,
            files: [
              {
                path: "C:\\Media\\a.mp4",
                sourceId: "source-1",
                provider: "instagram",
                mediaType: "video",
                sizeBytes: 200,
              },
              {
                path: "C:\\Media\\b.mp4",
                sourceId: "source-1",
                provider: "instagram",
                mediaType: "video",
                sizeBytes: 100,
              },
            ],
          },
        ],
      },
    });
    render(<WorkspaceHealthWindowPage />);

    fireEvent.click(
      await screen.findByRole("tab", { name: /storage & cleanup/i }),
    );

    // Scan details (engine, per-source jobs, stat cards) is collapsed by default.
    const scanDetailsSummary = (
      await screen.findByText(/^scan details$/i)
    ).closest("summary");
    const scanDetailsDisclosure = scanDetailsSummary?.closest("details");
    expect(scanDetailsDisclosure?.hasAttribute("open")).toBe(false);
    // Stat cards remain reachable inside it.
    expect(screen.getByText("Files scanned")).toBeTruthy();

    // Results are the dominant, non-collapsed area.
    expect(screen.getByText("Duplicate candidates")).toBeTruthy();
    expect(
      screen.getByPlaceholderText(/filter by folder or profile/i),
    ).toBeTruthy();

    const slider = screen.getByRole("slider", { name: /minimum similarity/i });
    expect(slider).toBeTruthy();
    fireEvent.change(slider, { target: { value: "95" } });
    expect(screen.getByText("≥ 95%")).toBeTruthy();

    // The compact control bar shows the one-line last-scan summary.
    expect(screen.getByText(/2 files scanned/i)).toBeTruthy();
  });
});
