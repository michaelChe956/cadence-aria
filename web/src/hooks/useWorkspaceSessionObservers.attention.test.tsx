// useWorkspaceSessionObservers 拆分测试（P4-d 全局 coding attention 待办）。
//
// 2026-11-13：useWorkspaceSessionObservers.test.tsx 超出 large_file_guard
// 登记上限（tests/it_core/large_file_guard.rs）⇒ 按主题拆分，本文件承接
// P4-d 组用例；用例与断言逐字保留（拆分自原文件 1043-1295 行），行为
// 零变化。公共夹具见 useWorkspaceSessionObservers.test-utils.tsx。
import { act, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  CodingAttentionItemDto,
  WorkspaceSessionSummary,
} from "../api/types";
import { notifyLifecycleInvalidated } from "../state/lifecycle-workbench-store";
import { INFO_TTL_MS } from "../state/recent-completion";
import { codingAttentionIdentity } from "../state/workspace-cockpit-projection";
import {
  observerOptions,
  renderObserverHook,
  summary,
} from "./useWorkspaceSessionObservers.test-utils";
import type { WorkspaceSessionObserverOptions } from "./useWorkspaceSessionObservers";

afterEach(() => {
  vi.useRealTimers();
});

// P4-d（REQ-UI37-19）：durable 待处理事项全局消费——issue 不在 watched 窗口
// 内其 choice/blocked/stalled 等仍进 display/count；同 [project,issue,key]
// 身份不重复提醒；事实关闭后条目消失；不受近期完成 TTL/32 条上限约束。
describe("P4-d global coding attention items", () => {
  const attention = (
    overrides: Partial<CodingAttentionItemDto> = {},
  ): CodingAttentionItemDto => ({
    key: "gate:attempt_A1:gate_A1",
    kind: "stalled",
    project_id: "project_1",
    issue_id: "issue_A",
    plan_id: null,
    attempt_id: "attempt_A1",
    logical_work_item_id: null,
    repository_name: null,
    reason_code: "coder_provider_stalled",
    title: "AI 运行挂起，需要处理",
    summary: "provider 零活动挂起",
    occurred_at: new Date(Date.now() - 60_000).toISOString(),
    target: { type: "coding_attempt", attempt_id: "attempt_A1" },
    focus: { gate_id: "gate_A1" },
    ...overrides,
  });

  const summaryFor = (
    issueId: string,
    workspaceSessionId: string,
  ): WorkspaceSessionSummary => ({
    ...summary(workspaceSessionId),
    issue_id: issueId,
  });

  function twoIssueObserverOptions(
    getIssueLifecycle: WorkspaceSessionObserverOptions["getIssueLifecycle"],
    watchLimit = 1,
  ) {
    return observerOptions({
      currentSessionId: "sB",
      watchLimit,
      getIssueLifecycle,
      listProductIssues: async () => ({
        issues: [
          {
            issue_id: "issue_B",
            project_id: "project_1",
            repo_id: null,
            base_branch: null,
            workspace_id: null,
            task_id: null,
            session_id: null,
            title: "Issue B",
            description: null,
            change_id: "change_B",
            phase: "development" as const,
            status: "in_progress" as const,
            active_binding_id: null,
            created_at: "2026-09-14T00:00:00Z",
            updated_at: "2026-09-14T00:00:00Z",
          },
          {
            issue_id: "issue_A",
            project_id: "project_1",
            repo_id: null,
            base_branch: null,
            workspace_id: null,
            task_id: null,
            session_id: null,
            title: "Issue A",
            description: null,
            change_id: "change_A",
            phase: "development" as const,
            status: "in_progress" as const,
            active_binding_id: null,
            created_at: "2026-09-14T00:00:00Z",
            updated_at: "2026-09-14T00:00:00Z",
          },
        ],
      }),
      createController: () => ({
        replaceWatchedSessionIds: vi.fn(),
        updateRefreshIntervalMs: vi.fn(),
        refresh: vi.fn(),
        records: () => [],
        dispose: vi.fn(),
      }),
    });
  }

  it("surfaces unwatched issues' choice/blocked/stalled attention items in display and count", async () => {
    // 人在 plan B（issue_B 当前会话 watched）；issue_A 的会话不在 K 内。
    const getIssueLifecycle = vi.fn(async (issueId: string) =>
      issueId === "issue_B"
        ? {
            workspace_sessions: [summaryFor("issue_B", "sB")],
            coding_attempts: [],
          }
        : {
            workspace_sessions: [summaryFor("issue_A", "sA1")],
            coding_attempts: [],
            coding_attention_items: [
              attention({ kind: "choice", key: "choice:attempt_A1:choice_A1" }),
              attention({ kind: "blocked", key: "gate:attempt_A1:gate_A2" }),
              attention({ kind: "stalled" }),
            ],
          },
    );
    const view = renderObserverHook(twoIssueObserverOptions(getIssueLifecycle));

    await waitFor(() => {
      expect(
        view.result.displayItems.filter((item) => item.source === "coding_attention"),
      ).toHaveLength(3);
    });
    // 三条均进可操作计数（无 watched 过滤）。
    expect(view.result.actionableCount).toBe(3);
    expect(
      view.result.countedInbox.filter((item) => item.source === "coding_attention"),
    ).toHaveLength(3);
    // issue_A 的会话确实未被观察。
    expect(view.result.watchedSessionIds).toEqual(["sB"]);

    await act(async () => {
      view.unmount();
    });
  });

  it("dedupes attention items by [project,issue,key] and reminds each key once", async () => {
    const getIssueLifecycle = vi.fn(async () => ({
      workspace_sessions: [summaryFor("issue_B", "sB")],
      coding_attempts: [],
      coding_attention_items: [
        attention(),
        attention(), // 同 key 重复条目
      ],
    }));
    const view = renderObserverHook(twoIssueObserverOptions(getIssueLifecycle));

    // 首次 hydration 静默基线：展示但不提醒。
    await waitFor(() => {
      expect(
        view.result.displayItems.filter((item) => item.source === "coding_attention"),
      ).toHaveLength(1);
    });
    expect(view.result.actionableCount).toBe(1);
    expect(view.result.notificationCandidates).toHaveLength(0);

    // 失效唤醒返回同 key：仍一条、不产生新候选。
    await act(async () => {
      notifyLifecycleInvalidated("issue_B");
    });
    await waitFor(() => expect(getIssueLifecycle.mock.calls.length).toBeGreaterThan(2));
    expect(
      view.result.displayItems.filter((item) => item.source === "coding_attention"),
    ).toHaveLength(1);
    expect(view.result.notificationCandidates).toHaveLength(0);

    // 新 key（同 attempt 新门）成为候选恰一次；再次返回同 key 不重复提醒。
    getIssueLifecycle.mockImplementation(async () => ({
      workspace_sessions: [summaryFor("issue_B", "sB")],
      coding_attempts: [],
      coding_attention_items: [
        attention(),
        attention({ key: "gate:attempt_A1:gate_NEW", reason_code: "upstream_source_unavailable" }),
      ],
    }));
    await act(async () => {
      notifyLifecycleInvalidated("issue_B");
    });
    await waitFor(() => {
      expect(
        view.result.displayItems.filter((item) => item.source === "coding_attention"),
      ).toHaveLength(2);
    });
    expect(view.result.notificationCandidates).toHaveLength(1);
    expect(view.result.notificationCandidates[0].attentionInfo?.identity).toBe(
      codingAttentionIdentity(attention({ key: "gate:attempt_A1:gate_NEW" })),
    );
    await act(async () => {
      notifyLifecycleInvalidated("issue_B");
    });
    await waitFor(() => expect(getIssueLifecycle.mock.calls.length).toBeGreaterThan(4));
    expect(view.result.notificationCandidates).toHaveLength(1);

    await act(async () => {
      view.unmount();
    });
  });

  it("removes attention items once the durable fact closes", async () => {
    const getIssueLifecycle = vi.fn(async () => ({
      workspace_sessions: [summaryFor("issue_B", "sB")],
      coding_attempts: [],
      coding_attention_items: [attention()],
    }));
    const view = renderObserverHook(twoIssueObserverOptions(getIssueLifecycle));

    await waitFor(() => {
      expect(
        view.result.displayItems.some((item) => item.source === "coding_attention"),
      ).toBe(true);
    });
    expect(view.result.actionableCount).toBe(1);

    // 事实关闭（作答/resolve/确认）后投影消失。
    getIssueLifecycle.mockImplementation(async () => ({
      workspace_sessions: [summaryFor("issue_B", "sB")],
      coding_attempts: [],
      coding_attention_items: [],
    }));
    await act(async () => {
      notifyLifecycleInvalidated("issue_B");
    });
    await waitFor(() => {
      expect(
        view.result.displayItems.some((item) => item.source === "coding_attention"),
      ).toBe(false);
    });
    expect(view.result.actionableCount).toBe(0);

    await act(async () => {
      view.unmount();
    });
  });

  it("keeps attention items beyond the recent-completion TTL and 32-item catalog cap", async () => {
    const staleOccurredAt = new Date(
      Date.now() - INFO_TTL_MS * 2,
    ).toISOString();
    const items: CodingAttentionItemDto[] = Array.from({ length: 33 }, (_, index) =>
      attention({
        key: `gate:attempt_A1:gate_${index}`,
        occurred_at: staleOccurredAt,
      }),
    );
    const getIssueLifecycle = vi.fn(async () => ({
      workspace_sessions: [summaryFor("issue_B", "sB")],
      coding_attempts: [],
      coding_attention_items: items,
    }));
    const view = renderObserverHook(twoIssueObserverOptions(getIssueLifecycle));

    await waitFor(() => {
      expect(
        view.result.displayItems.filter((item) => item.source === "coding_attention"),
      ).toHaveLength(33);
    });
    expect(view.result.actionableCount).toBe(33);

    await act(async () => {
      view.unmount();
    });
  });
});
