import { act, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  CodingFinalConfirmInfoItem,
  LogicalCodebaseBootstrapNoticeDto,
  RecentCompletionInfoItem,
  WorkspaceSessionSummary,
} from "../api/types";
import { notifyLifecycleInvalidated } from "../state/lifecycle-workbench-store";
import { INFO_TTL_MS } from "../state/recent-completion";
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

  // P3 WIGA Task 2（tasks.md §4.1 / REQ-WIGA-07）：K=8 观察窗外的近期
  // 完成事实经有界目录补读展示——第 9 个会话没有 observer socket，
  // 完成信息仍进 displayItems 且 actionableCount=0。
  it("surfaces K-external recent completions without an observer socket or actionable count", async () => {
    const replaceWatchedSessionIds = vi.fn();
    const sessions = Array.from({ length: 9 }, (_, index) => summary(`s${index + 1}`));
    const getIssueLifecycle = vi.fn(async () => ({
      workspace_sessions: sessions,
      coding_attempts: [],
      plan_confirmed_info: [],
      coding_final_confirm_info: [],
      recent_completion_info: [
        {
          kind: "coding_final_confirm" as const,
          key: "coding_final_confirm:attempt_9:node_9",
          project_id: "project_1",
          issue_id: "issue_1",
          plan_id: "plan_9",
          session_id: null,
          attempt_id: "attempt_9",
          occurred_at: new Date(Date.now() - 60_000).toISOString(),
          title: "编码执行完成，待最终确认",
          final_confirmed: false,
        },
      ],
    }));
    const view = renderObserverHook(
      observerOptions({
        currentSessionId: "s1",
        watchLimit: 8,
        getIssueLifecycle,
        createController: () => ({
          replaceWatchedSessionIds,
          updateRefreshIntervalMs: vi.fn(),
          refresh: vi.fn(),
          records: () => [],
          dispose: vi.fn(),
        }),
      }),
    );

    await waitFor(() => {
      expect(
        view.result.displayItems.filter((item) => item.source === "coding_final_confirm_info"),
      ).toHaveLength(1);
    });
    expect(view.result.actionableCount).toBe(0);
    expect(view.result.notificationCandidates).toHaveLength(0);
    // 第 9 个会话（K=8 + 当前 s1）没有 observer socket。
    expect(replaceWatchedSessionIds).toHaveBeenLastCalledWith([
      "s2", "s3", "s4", "s5", "s6", "s7", "s8",
    ]);

    await act(async () => {
      view.unmount();
    });
  });

  // 同 key 跨 project/issue 不合并；同 issue 双次返回同 key 只留一条。
  it("dedupes recent completions by full scope identity across projects and refreshes", async () => {
    const occurredAt = new Date(Date.now() - 60_000).toISOString();
    const entry = (projectId: string) => ({
      kind: "plan_confirmed" as const,
      key: "plan_confirmed:plan_x:compile_x",
      project_id: projectId,
      issue_id: "issue_1",
      plan_id: "plan_x",
      session_id: "session_x",
      attempt_id: null,
      occurred_at: occurredAt,
      title: "Work Item Plan 已确认",
      final_confirmed: null,
    });
    const getIssueLifecycle = vi.fn(
      async (_issueId: string, projectId: string) => ({
        workspace_sessions: [summary("s1")],
        coding_attempts: [],
        plan_confirmed_info: [],
        coding_final_confirm_info: [],
        recent_completion_info:
          projectId === "project_1" ? [entry("project_1"), entry("project_1")] : [entry("project_2")],
      }),
    );
    const view = renderObserverHook(
      observerOptions({
        currentSessionId: "s1",
        watchLimit: 2,
        getIssueLifecycle,
        listProjects: async () => ({
          projects: [
            { project_id: "project_1", name: "P1", description: null, created_at: "2026-09-14T00:00:00Z", updated_at: "2026-09-14T00:00:00Z", last_opened_at: null },
            { project_id: "project_2", name: "P2", description: null, created_at: "2026-09-14T00:00:00Z", updated_at: "2026-09-14T00:00:00Z", last_opened_at: null },
          ],
        }),
        listProductIssues: async (projectId: string) => ({
          issues: [{
            issue_id: "issue_1",
            project_id: projectId,
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
      }),
    );

    await waitFor(() => {
      expect(view.result.displayItems.filter((item) => item.source === "plan_confirmed_info"))
        .toHaveLength(2);
    });
    const identities = view.result.displayItems
      .filter((item) => item.source === "plan_confirmed_info")
      .map((item) => item.completionIdentity);
    expect(new Set(identities).size).toBe(2);
    expect(identities[0]).not.toBe(identities[1]);
  });

  // 首次 hydration（含空目录）只展示；失效唤醒补读；同 key 状态升级不
  // 产生新候选；fetch 失败保留已知目录与候选。
  it("keeps first hydration silent and tracks new identities after invalidation wakes", async () => {
    const recent = (key: string, confirmed = false) => ({
      kind: "coding_final_confirm" as const,
      key,
      project_id: "project_1",
      issue_id: "issue_1",
      plan_id: "plan_1",
      session_id: null,
      attempt_id: key,
      occurred_at: new Date(Date.now() - 60_000).toISOString(),
      title: confirmed ? "已最终确认" : "编码执行完成，待最终确认",
      final_confirmed: confirmed,
    });
    const lifecyclePage = (recentItems: ReturnType<typeof recent>[]) => async () => ({
      workspace_sessions: [summary("s1"), summary("s2")],
      coding_attempts: [],
      plan_confirmed_info: [],
      coding_final_confirm_info: [],
      recent_completion_info: recentItems,
    });
    const getIssueLifecycle = vi.fn(lifecyclePage([]));
    const view = renderObserverHook(
      observerOptions({ currentSessionId: "s1", watchLimit: 2, getIssueLifecycle }),
    );

    // 首次空成功 hydration：无展示、无候选（静默基线，失败不置基线）。
    await waitFor(() => expect(getIssueLifecycle).toHaveBeenCalled());

    // lifecycle 失效唤醒 → 补读发现新 key → 展示并成为候选。
    getIssueLifecycle.mockImplementation(lifecyclePage([recent("k1")]));
    await act(async () => {
      notifyLifecycleInvalidated("issue_1");
    });
    await waitFor(() => {
      expect(
        view.result.displayItems.filter((item) => item.source === "coding_final_confirm_info"),
      ).toHaveLength(1);
    });
    expect(view.result.notificationCandidates).toHaveLength(1);

    // 同 key waiting→Completed：文案更新，不产生第二条候选。
    getIssueLifecycle.mockImplementation(lifecyclePage([recent("k1", true)]));
    await act(async () => {
      notifyLifecycleInvalidated("issue_1");
    });
    await waitFor(() => {
      expect(view.result.displayItems.find((item) => item.completionIdentity)?.title)
        .toBe("已最终确认");
    });
    expect(view.result.notificationCandidates).toHaveLength(1);

    // fetch 失败：不清空已知目录，不设置新候选。
    getIssueLifecycle.mockRejectedValue(new Error("catalog unavailable"));
    await act(async () => {
      notifyLifecycleInvalidated("issue_1");
    });
    await waitFor(() => expect(getIssueLifecycle.mock.calls.length).toBeGreaterThan(3));
    expect(
      view.result.displayItems.filter((item) => item.source === "coding_final_confirm_info"),
    ).toHaveLength(1);
    expect(view.result.notificationCandidates).toHaveLength(1);

    await act(async () => {
      view.unmount();
    });
  });

  // 初始 GET 未完成时收到 invalidation：新事实归入首次补读静默批次，
  // 之后的 refresh 才开始产生候选。
  it("folds facts discovered by an invalidation during the initial fetch into the silent first batch", async () => {
    const recent = (key: string): RecentCompletionInfoItem => ({
      kind: "plan_confirmed",
      key,
      project_id: "project_1",
      issue_id: "issue_1",
      plan_id: key,
      session_id: `session_${key}`,
      attempt_id: null,
      occurred_at: new Date(Date.now() - 60_000).toISOString(),
      title: "Work Item Plan 已确认",
      final_confirmed: null,
    });
    interface LifecyclePage {
      workspace_sessions: WorkspaceSessionSummary[];
      coding_attempts: never[];
      plan_confirmed_info: never[];
      coding_final_confirm_info: never[];
      recent_completion_info: RecentCompletionInfoItem[];
    }
    const page = (keys: string[]) => async (): Promise<LifecyclePage> => ({
      workspace_sessions: [summary("s1")],
      coding_attempts: [],
      plan_confirmed_info: [],
      coding_final_confirm_info: [],
      recent_completion_info: keys.map(recent),
    });
    let releaseInitial: (value: LifecyclePage) => void = () => undefined;
    const initialGate = new Promise<LifecyclePage>((resolve) => {
      releaseInitial = resolve;
    });
    const getIssueLifecycle = vi.fn().mockImplementationOnce(() => initialGate);
    const view = renderObserverHook(
      observerOptions({ currentSessionId: "s1", watchLimit: 2, getIssueLifecycle }),
    );

    // 初始 GET 在途时收到失效唤醒（只标 dirty，不抢跑）。
    await act(async () => {
      notifyLifecycleInvalidated("issue_1");
    });
    // 初始 GET 完成时看到空目录；紧随的补读发现 k1 —— 仍在首次静默批次。
    getIssueLifecycle.mockImplementation(page(["k1"]));
    await act(async () => {
      releaseInitial(await page([])());
    });
    await waitFor(() => {
      expect(
        view.result.displayItems.filter((item) => item.source === "plan_confirmed_info"),
      ).toHaveLength(1);
    });
    expect(view.result.notificationCandidates).toHaveLength(0);

    // 首次批次关闭后的新事实才成为候选。
    getIssueLifecycle.mockImplementation(page(["k1", "k2"]));
    await act(async () => {
      notifyLifecycleInvalidated("issue_1");
    });
    await waitFor(() => {
      expect(view.result.notificationCandidates).toHaveLength(1);
      expect(view.result.notificationCandidates[0].completionIdentity).toContain("k2");
    });

    await act(async () => {
      view.unmount();
    });
  });

  // 观察者收到 durable snapshot 帧的唤醒回调后排队补读（仅排程，不当事实）。
  it("queues a catalog refresh when the controller signals a snapshot hint", async () => {
    let signalSnapshotHint: (() => void) | null = null;
    const recent = {
      kind: "coding_final_confirm" as const,
      key: "coding_final_confirm:attempt_h:node_h",
      project_id: "project_1",
      issue_id: "issue_1",
      plan_id: "plan_1",
      session_id: null,
      attempt_id: "attempt_h",
      occurred_at: new Date(Date.now() - 60_000).toISOString(),
      title: "编码执行完成，待最终确认",
      final_confirmed: false,
    };
    const getIssueLifecycle = vi.fn(async () => ({
      workspace_sessions: [summary("s1")],
      coding_attempts: [],
      plan_confirmed_info: [],
      coding_final_confirm_info: [],
      recent_completion_info: [] as RecentCompletionInfoItem[],
    }));
    const view = renderObserverHook(
      observerOptions({
        currentSessionId: "s1",
        watchLimit: 2,
        getIssueLifecycle,
        createController: (_onRecordsChange, onSnapshotHint) => {
          signalSnapshotHint = () => onSnapshotHint?.();
          return {
            replaceWatchedSessionIds: vi.fn(),
            updateRefreshIntervalMs: vi.fn(),
            refresh: vi.fn(),
            records: () => [],
            dispose: vi.fn(),
          };
        },
      }),
    );
    await waitFor(() => expect(getIssueLifecycle).toHaveBeenCalled());

    getIssueLifecycle.mockImplementation(async () => ({
      workspace_sessions: [summary("s1")],
      coding_attempts: [],
      plan_confirmed_info: [],
      coding_final_confirm_info: [],
      recent_completion_info: [recent],
    }));
    // 等同 durable snapshot 帧到达后的唤醒（测试注入 factory 主动触发）。
    await act(async () => {
      signalSnapshotHint?.();
    });
    await waitFor(() => {
      expect(
        view.result.displayItems.filter((item) => item.source === "coding_final_confirm_info"),
      ).toHaveLength(1);
    });

    await act(async () => {
      view.unmount();
    });
  });
  // P3 WIGA Task 3（tasks.md §4.1 / REQ-WIGA-07）：TTL 边界——未来/坏时间
  // 不展示；到期即使无新 REST 也从收件箱移除；刷新/重连返回同条目不续期；
  // 候选同样只收 TTL 内的新身份。
  it("expires recent completions by TTL without REST and never revives them on refresh", async () => {
    const recent = (key: string, occurredAt: string): RecentCompletionInfoItem => ({
      kind: "coding_final_confirm",
      key,
      project_id: "project_1",
      issue_id: "issue_1",
      plan_id: "plan_1",
      session_id: null,
      attempt_id: key,
      occurred_at: occurredAt,
      title: `完成事实 ${key}`,
      final_confirmed: false,
    });
    const healthy = recent("k-healthy", new Date(Date.now() - 60_000).toISOString());
    const expiring = recent(
      "k-expiring",
      new Date(Date.now() - INFO_TTL_MS + 500).toISOString(),
    );
    const future = recent("k-future", new Date(Date.now() + 60_000).toISOString());
    const badTime = recent("k-bad", "not-a-time");
    const getIssueLifecycle = vi.fn(async () => ({
      workspace_sessions: [summary("s1")],
      coding_attempts: [],
      plan_confirmed_info: [],
      coding_final_confirm_info: [],
      recent_completion_info: [healthy, expiring, future, badTime],
    }));
    const view = renderObserverHook(
      observerOptions({
        currentSessionId: "s1",
        watchLimit: 2,
        refreshIntervalMs: 600_000,
        getIssueLifecycle,
      }),
    );

    const infoTitles = () =>
      view.result.displayItems
        .filter((item) => item.kind === "info")
        .map((item) => item.title);
    // 初始 hydration：只有 TTL 内条目展示（未来/坏时间 fail-closed 不展示）。
    await waitFor(() => expect(infoTitles()).toEqual(["完成事实 k-healthy", "完成事实 k-expiring"]));
    expect(getIssueLifecycle).toHaveBeenCalledTimes(1);

    // 到期移除：无新 REST 也从收件箱消失（单次失效定时器）。
    await waitFor(
      () => expect(infoTitles()).toEqual(["完成事实 k-healthy"]),
      { timeout: 4_000 },
    );
    expect(getIssueLifecycle).toHaveBeenCalledTimes(1);

    // 刷新/重连不续期：同条目重新返回仍不展示，候选也不收过期身份。
    await act(async () => {
      notifyLifecycleInvalidated("issue_1");
    });
    await waitFor(() => expect(getIssueLifecycle.mock.calls.length).toBeGreaterThan(1));
    await waitFor(() => expect(infoTitles()).toEqual(["完成事实 k-healthy"]));

    // hydration 关闭后的新事实：TTL 内进候选，未来时刻不进。
    const lateOk = recent("k-late-ok", new Date(Date.now() - 30_000).toISOString());
    const lateFuture = recent("k-late-future", new Date(Date.now() + 30_000).toISOString());
    getIssueLifecycle.mockImplementation(async () => ({
      workspace_sessions: [summary("s1")],
      coding_attempts: [],
      plan_confirmed_info: [],
      coding_final_confirm_info: [],
      recent_completion_info: [healthy, expiring, lateOk, lateFuture],
    }));
    await act(async () => {
      notifyLifecycleInvalidated("issue_1");
    });
    await waitFor(() => {
      expect(view.result.notificationCandidates.map((item) => item.title)).toEqual([
        "完成事实 k-late-ok",
      ]);
    });

    await act(async () => {
      view.unmount();
    });
  });
});

describe("C4 logical codebase bootstrap notice source", () => {
  it("surfaces bootstrap waiting notices per LC and keeps them out of the error count", async () => {
    const bootstrapNotice: LogicalCodebaseBootstrapNoticeDto = {
      key: "bootstrap:identity:migration_0001:identity_migration_failed",
      step: "identity",
      object_id: "migration_0001",
      reason_code: "identity_migration_failed",
      summary: "identity mismatch: identity_registry repository_0001",
      external_side_effect: "none",
      allowed_actions: ["repair"],
      next_step: "manifest_checkout",
      created_at: "",
    };
    let bootstrapReads = 0;
    const view = renderObserverHook(observerOptions({
      listCodebases: async () => ({
        codebases: [
          {
            id: "lc_0001",
            name: "Platform",
            kind: "logical" as const,
            repository_id: null,
            logical_codebase_id: "lc_0001",
            member_count: 2,
          },
        ],
      }),
      getLogicalCodebaseBootstrap: async () => {
        bootstrapReads += 1;
        return {
          project_id: "project_1",
          logical_codebase_id: "lc_0001",
          authority_root: "/tmp/lc",
          membership_revision: 1,
          policy: null,
          steps: [],
          planning_ready: false,
          notices: [bootstrapNotice],
        };
      },
    }));

    await waitFor(() => expect(bootstrapReads).toBeGreaterThan(0));
    const inbox = view.result.inbox;
    const bootstrapItems = inbox.filter((item) => item.source === "logical_codebase_bootstrap");
    expect(bootstrapItems).toHaveLength(1);
    expect(bootstrapItems[0].kind).toBe("lc_bootstrap");
    expect(bootstrapItems[0].bootstrapInfo?.reasonCode).toBe("identity_migration_failed");
    expect(bootstrapItems[0].bootstrapInfo?.nextStep).toBe("manifest_checkout");
    // 通知不进入错误计数（与 C1/info 同一隔离规则）。
    expect(
      view.result.countedInbox.some((item) => item.source === "logical_codebase_bootstrap"),
    ).toBe(false);

    await act(async () => {
      view.unmount();
    });
  });
});

// C5 Task 6：project 级 repository 初始化失败等待项与目录同源补读——
// 无 issue_id 条目进入收件箱（id 即后端稳定 id）；读取失败保留上一轮
// durable 快照，不清空既有等待项。
describe("C5 project repository initialization waiting items", () => {
  const projectWaitingItem = {
    id: "c1:project:project_1:repository_init:op_init_0001",
    kind: "repository_initialization_failed",
    reason:
      "repository initialization failed at pre_check (provider_unavailable); awaiting gateway recovery",
    completed_steps: ["cadence_skills"],
    target: null,
    plan_id: null,
    session_id: null,
    attempt_id: null,
    gate_id: null,
    possible_side_effect: null,
    actions: ["resume_repository_initialization"],
    next_phase: "repository_registered",
    action_context: [],
    operation_id: "op_init_0001",
    diagnostics: {
      failed_step: "pre_check",
      reason_code: "provider_unavailable",
      provider: "claude_code",
      stderr_summary: null,
      changed_paths: [],
      retryable: true,
    },
    project_id: "project_1",
  };

  it("merges issue-less project waiting items into the inbox during catalog refresh", async () => {
    const reads: string[] = [];
    const view = renderObserverHook(
      observerOptions({
        listProjectRepositoryInitializationWaitingItems: async (projectId) => {
          reads.push(projectId);
          return [projectWaitingItem];
        },
      }),
    );

    await waitFor(() => expect(reads).toEqual(["project_1"]));
    const inbox = view.result.inbox;
    const waiting = inbox.filter(
      (item) =>
        item.source === "c1_waiting" &&
        item.c1Info?.kind === "repository_initialization_failed",
    );
    expect(waiting).toHaveLength(1);
    expect(waiting[0].id).toBe("c1:project:project_1:repository_init:op_init_0001");
    expect(waiting[0].id).not.toContain("undefined");
    expect(waiting[0].c1Info?.issueId).toBeNull();
    expect(waiting[0].c1Info?.operationId).toBe("op_init_0001");
    // 等待项不进入错误计数（与 C1/info 同一隔离规则）。
    expect(
      view.result.countedInbox.some(
        (item) => item.c1Info?.kind === "repository_initialization_failed",
      ),
    ).toBe(false);

    await act(async () => {
      view.unmount();
    });
  });

  it("keeps the previous project waiting snapshot when the refresh read fails", async () => {
    let failReads = false;
    let refreshes = 0;
    const view = renderObserverHook(
      observerOptions({
        getIssueLifecycle: async () => {
          refreshes += 1;
          return { workspace_sessions: [], coding_attempts: [] };
        },
        listProjectRepositoryInitializationWaitingItems: async () => {
          if (failReads) {
            throw new Error("project waiting-items unavailable");
          }
          return [projectWaitingItem];
        },
      }),
    );

    await waitFor(() =>
      expect(
        view.result.inbox.some(
          (item) =>
            item.c1Info?.kind === "repository_initialization_failed",
        ),
      ).toBe(true),
    );

    failReads = true;
    await act(async () => {
      notifyLifecycleInvalidated("issue_1");
    });
    await waitFor(() => expect(refreshes).toBeGreaterThanOrEqual(2));
    // 失败轮保留上一轮 durable 快照，不清空既有等待项。
    await waitFor(() =>
      expect(
        view.result.inbox.some(
          (item) =>
            item.c1Info?.kind === "repository_initialization_failed",
        ),
      ).toBe(true),
    );

    await act(async () => {
      view.unmount();
    });
  });
});
