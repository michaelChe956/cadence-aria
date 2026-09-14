import { act, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceSessionSummary } from "../api/types";
import {
  useWorkspaceSessionObservers,
  type WorkspaceSessionObserverOptions,
  type WorkspaceSessionObserverResult,
} from "./useWorkspaceSessionObservers";

function summary(workspace_session_id: string): WorkspaceSessionSummary {
  return {
    workspace_session_id,
    issue_id: "issue_1",
    entity_id: `entity_${workspace_session_id}`,
    workspace_type: "work_item",
    status: "running",
    author_provider: "claude_code",
    reviewer_provider: "codex",
    review_rounds: 1,
    superpowers_enabled: false,
    openspec_enabled: false,
  };
}

function observerOptions(overrides: Partial<WorkspaceSessionObserverOptions> = {}) {
  return {
    currentSessionId: "s1",
    watchLimit: 2,
    refreshIntervalMs: 15_000,
    listProjects: async () => ({
      projects: [{
        project_id: "project_1",
        name: "Project",
        description: null,
        created_at: "2026-09-14T00:00:00Z",
        updated_at: "2026-09-14T00:00:00Z",
        last_opened_at: null,
      }],
    }),
    listProductIssues: async () => ({
      issues: [{
        issue_id: "issue_1",
        project_id: "project_1",
        repo_id: null,
        workspace_id: null,
        task_id: null,
        session_id: null,
        title: "Issue",
        description: null,
        change_id: "change_1",
        phase: "development" as const,
        status: "in_progress" as const,
        active_binding_id: null,
        created_at: "2026-09-14T00:00:00Z",
        updated_at: "2026-09-14T00:00:00Z",
      }],
    }),
    getIssueLifecycle: async () => ({
      workspace_sessions: [summary("s1"), summary("s2"), summary("s3")],
    }),
    ...overrides,
  } satisfies WorkspaceSessionObserverOptions;
}

function renderObserverHook(initialOptions: WorkspaceSessionObserverOptions) {
  let result: WorkspaceSessionObserverResult | undefined;

  function Harness({ options }: { options: WorkspaceSessionObserverOptions }) {
    result = useWorkspaceSessionObservers(options);
    return null;
  }

  const view = render(<Harness options={initialOptions} />);
  return {
    ...view,
    rerender(options: WorkspaceSessionObserverOptions) {
      view.rerender(<Harness options={options} />);
    },
    get result() {
      if (!result) throw new Error("hook result unavailable");
      return result;
    },
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("useWorkspaceSessionObservers", () => {
  it("enumerates REST sessions and replaces the watch window immediately when K changes", async () => {
    const replaceWatchedSessionIds = vi.fn();
    const updateRefreshIntervalMs = vi.fn();
    const options = observerOptions({
      createController: () => ({
        replaceWatchedSessionIds,
        updateRefreshIntervalMs,
        refresh: vi.fn(),
        records: () => [],
        dispose: vi.fn(),
      }),
    });
    const view = renderObserverHook(options);

    await waitFor(() => expect(view.result.watchedSessionIds).toEqual(["s1", "s2"]));
    expect(replaceWatchedSessionIds).toHaveBeenLastCalledWith(["s2"]);

    view.rerender({ ...options, watchLimit: 3, refreshIntervalMs: 5_000 });

    await waitFor(() => expect(view.result.watchedSessionIds).toEqual(["s1", "s2", "s3"]));
    expect(replaceWatchedSessionIds).toHaveBeenLastCalledWith(["s2", "s3"]);
    expect(updateRefreshIntervalMs).toHaveBeenLastCalledWith(5_000);

    await act(async () => {
      view.unmount();
    });
  });

  it("does not open a duplicate observer socket for the current workspace session", async () => {
    const replaceWatchedSessionIds = vi.fn();
    const view = renderObserverHook(observerOptions({
      currentSessionId: "s1",
      createController: () => ({
        replaceWatchedSessionIds,
        refresh: vi.fn(),
        records: () => [],
        updateRefreshIntervalMs: vi.fn(),
        dispose: vi.fn(),
      }),
    }));

    await waitFor(() => expect(view.result.watchedSessionIds).toEqual(["s1", "s2"]));
    expect(replaceWatchedSessionIds).toHaveBeenLastCalledWith(["s2"]);
  });

  it("keeps the current session visible outside K without adding it to global counts", async () => {
    const replaceWatchedSessionIds = vi.fn();
    const view = renderObserverHook(observerOptions({
      currentSessionId: "outside",
      currentSessionState: {
        sessionId: "outside",
        sessionStatus: "stopped_needs_human",
        stage: "running",
        humanGateSnapshot: null,
        humanGateTurn: null,
        humanGateClosure: null,
        flowKind: null,
        pendingReviewerSummary: null,
        chatEntries: [],
        protocolError: null,
        error: null,
        advanceCommands: {},
      } as never,
      watchLimit: 1,
      getIssueLifecycle: async () => ({
        workspace_sessions: [summary("watched"), summary("outside")],
      }),
      createController: () => ({
        replaceWatchedSessionIds,
        updateRefreshIntervalMs: vi.fn(),
        refresh: vi.fn(),
        records: () => [],
        dispose: vi.fn(),
      }),
    }));

    await waitFor(() => expect(view.result.watchedSessionIds).toEqual(["watched"]));
    expect(view.result.records).toHaveLength(1);
    expect(view.result.records[0]?.sessionId).toBe("outside");
    expect(view.result.inbox).toHaveLength(1);
    expect(view.result.countedInbox).toEqual([]);
    expect(replaceWatchedSessionIds).toHaveBeenLastCalledWith(["watched"]);
  });

  it("refreshes the catalog and opens an observer for a session newly admitted to K", async () => {
    let sessions = [summary("s1"), summary("s2")];
    const replaceWatchedSessionIds = vi.fn();
    let refreshCatalog: (() => void) | undefined;
    const view = renderObserverHook(observerOptions({
      createController: () => ({
        replaceWatchedSessionIds,
        updateRefreshIntervalMs: vi.fn(),
        refresh: vi.fn(),
        records: () => [],
        dispose: vi.fn(),
      }),
      getIssueLifecycle: async () => ({ workspace_sessions: sessions }),
      scheduleCatalogRefresh: (callback) => {
        refreshCatalog = callback;
        return 0;
      },
      cancelCatalogRefresh: vi.fn(),
    }));

    await waitFor(() => expect(view.result.watchedSessionIds).toEqual(["s1", "s2"]));
    sessions = [summary("s1"), summary("newly-admitted")];
    await act(async () => {
      refreshCatalog?.();
    });

    await waitFor(() => expect(view.result.watchedSessionIds).toEqual(["s1", "newly-admitted"]));
    expect(replaceWatchedSessionIds).toHaveBeenLastCalledWith(["newly-admitted"]);
  });

  it("adds a manually watched child session to the observer controller", async () => {
    const replaceWatchedSessionIds = vi.fn();
    const view = renderObserverHook(observerOptions({
      watchLimit: 0,
      refreshIntervalMs: 0,
      createController: () => ({
        replaceWatchedSessionIds,
        updateRefreshIntervalMs: vi.fn(),
        refresh: vi.fn(),
        records: () => [],
        dispose: vi.fn(),
      }),
    }));
    await act(async () => {
      view.result.watchSession("child_001");
    });

    await waitFor(() => expect(view.result.watchedSessionIds).toEqual(["child_001"]));
    expect(replaceWatchedSessionIds).toHaveBeenLastCalledWith(["child_001"]);
  });
});
