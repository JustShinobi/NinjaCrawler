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
import type { WorkspaceHealthSnapshot } from "../../domain/models";
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
      }),
    );
    expect(screen.getByText(/vdf compares videos within each source/i)).toBeTruthy();
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
});
