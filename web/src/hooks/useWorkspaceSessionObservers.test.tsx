import { act, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  CodingFinalConfirmInfoItem,
  WorkspaceSessionSummary,
} from "../api/types";
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
    automation: { owner: "client", enrollment_id: null, policy_revision: null, enabled: false },
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
        base_branch: null,
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
      coding_attempts: [],
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
        coding_attempts: [],
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
      getIssueLifecycle: async () => ({ workspace_sessions: sessions, coding_attempts: [] }),
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

  it("retains the last successful watch window and counted inbox when a catalog refresh fails", async () => {
    const replaceWatchedSessionIds = vi.fn();
    const getIssueLifecycle = vi.fn(async () => {
      if (getIssueLifecycle.mock.calls.length > 1) {
        throw new Error("temporary catalog failure");
      }
      return { workspace_sessions: [summary("s1"), summary("s2")], coding_attempts: [] as never[] };
    });
    let refreshCatalog: (() => void) | undefined;
    let onRecordsChange: ((records: readonly { sessionId: string; state: never }[]) => void) | undefined;
    const view = renderObserverHook(observerOptions({
      getIssueLifecycle,
      createController: (onChange) => {
        onRecordsChange = onChange as typeof onRecordsChange;
        return {
          replaceWatchedSessionIds,
          updateRefreshIntervalMs: vi.fn(),
          refresh: vi.fn(),
          records: () => [],
          dispose: vi.fn(),
        };
      },
      scheduleCatalogRefresh: (callback) => {
        refreshCatalog = callback;
        return 0;
      },
      cancelCatalogRefresh: vi.fn(),
    }));

    await waitFor(() => expect(view.result.watchedSessionIds).toEqual(["s1", "s2"]));
    act(() => {
      onRecordsChange?.([{
        sessionId: "s2",
        state: {
          sessionId: "s2",
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
      }]);
    });
    await waitFor(() => expect(view.result.countedInbox).toHaveLength(1));

    act(() => refreshCatalog?.());

    await waitFor(() => expect(getIssueLifecycle).toHaveBeenCalledTimes(2));
    expect(view.result.watchedSessionIds).toEqual(["s1", "s2"]);
    expect(view.result.countedInbox).toHaveLength(1);
    expect(replaceWatchedSessionIds).toHaveBeenLastCalledWith(["s2"]);
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

  // P1 WIGA Task 9（REQ-WIGA-07）：issue lifecycle 的 plan_confirmed_info 只
  // 为 watched session 投影只读 info 条目：按 durable key 去重、新 key 追加、
  // 不进 countedInbox（待处理计数不含 info）。
  it("projects watched plan_confirmed_info into the display inbox only", async () => {
    const info = (key: string, session_id: string) => ({
      key,
      plan_id: "plan_1",
      session_id,
      occurred_at: "2026-09-27T00:00:00Z",
      title: "Work Item Plan 已确认",
    });
    const getIssueLifecycle = vi.fn(async () => ({
      workspace_sessions: [summary("s1"), summary("s2"), summary("s3")],
      coding_attempts: [],
      // s1/s2 在 watch 窗口内（K=2 + current s1）；s9 不在。
      plan_confirmed_info: [
        info("plan_confirmed:plan_1:compile_1", "s1"),
        info("plan_confirmed:plan_1:compile_1", "s1"),
        info("plan_confirmed:plan_9:compile_9", "s9"),
      ],
    }));
    const view = renderObserverHook(
      observerOptions({ currentSessionId: "s1", watchLimit: 2, getIssueLifecycle }),
    );

    await waitFor(() => {
      expect(view.result.inbox).toHaveLength(1);
    });
    expect(view.result.inbox[0]).toMatchObject({
      id: "s1:info:plan_confirmed:plan_1:compile_1",
      kind: "info",
      title: "Work Item Plan 已确认",
    });
    expect(view.result.countedInbox).toHaveLength(0);

    // 同 key 重复刷新不重复；新 key 追加为第二条。
    getIssueLifecycle.mockResolvedValue({
      workspace_sessions: [summary("s1"), summary("s2"), summary("s3")],
      coding_attempts: [],
      plan_confirmed_info: [
        info("plan_confirmed:plan_1:compile_1", "s1"),
        info("plan_confirmed:plan_1:compile_2", "s1"),
        info("plan_confirmed:plan_9:compile_9", "s9"),
      ],
    });
    view.rerender(observerOptions({ currentSessionId: "s1", watchLimit: 2, getIssueLifecycle }));
    await waitFor(() => {
      expect(view.result.inbox).toHaveLength(2);
    });
    expect(view.result.countedInbox).toHaveLength(0);
  });

  // P2 WIGA Task 9（REQ-WIGA-07/R5）：durable coding FinalConfirm 等待信息
  // 只进展示收件箱——同 key 去重、不进 countedInbox（待处理计数不含 info）。
  it("shows one final-confirm coding info without increasing actionable count", async () => {
    const info: CodingFinalConfirmInfoItem = {
      key: "coding_final_confirm:coding_attempt_001:coding_node_0004",
      project_id: "project_1",
      issue_id: "issue_1",
      plan_id: "work_item_plan_0001",
      attempt_id: "coding_attempt_001",
      occurred_at: "2026-09-27T03:20:00Z",
      title: "编码执行完成，待最终确认",
      final_confirmed: false,
    };
    const getIssueLifecycle = vi.fn(async () => ({
      workspace_sessions: [summary("s1"), summary("s2")],
      coding_attempts: [],
      coding_final_confirm_info: [info, info],
    }));
    const view = renderObserverHook(
      observerOptions({ currentSessionId: "s1", watchLimit: 2, getIssueLifecycle }),
    );

    await waitFor(() => {
      expect(
        view.result.inbox.filter((item) => item.source === "coding_final_confirm_info"),
      ).toHaveLength(1);
    });
    const codingItem = view.result.inbox.find(
      (item) => item.source === "coding_final_confirm_info",
    );
    expect(codingItem).toMatchObject({
      kind: "info",
      title: "编码执行完成，待最终确认",
    });
    // 不进 countedInbox（待处理计数隔离）。
    expect(view.result.countedInbox).toHaveLength(0);

    // 人工确认后同 key 改文案（final_confirmed），仍只有一条、仍不计数。
    getIssueLifecycle.mockResolvedValue({
      workspace_sessions: [summary("s1"), summary("s2")],
      coding_attempts: [],
      coding_final_confirm_info: [
        { ...info, title: "已最终确认", final_confirmed: true },
      ],
    });
    view.rerender(observerOptions({ currentSessionId: "s1", watchLimit: 2, getIssueLifecycle }));
    await waitFor(() => {
      expect(
        view.result.inbox.filter((item) => item.source === "coding_final_confirm_info"),
      ).toHaveLength(1);
    });
    expect(view.result.inbox.find((item) => item.source === "coding_final_confirm_info"))
      .toMatchObject({ title: "已最终确认" });
    expect(view.result.countedInbox).toHaveLength(0);
  });
});
