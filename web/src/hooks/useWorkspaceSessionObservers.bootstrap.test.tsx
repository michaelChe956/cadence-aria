// useWorkspaceSessionObservers 拆分测试（C4 LC bootstrap 通知 + C5 项目仓
// 初始化等待项）。
//
// 2026-11-13：useWorkspaceSessionObservers.test.tsx 超出 large_file_guard
// 登记上限（tests/it_core/large_file_guard.rs）⇒ 按主题拆分，本文件承接
// C4/C5 两组用例；用例与断言逐字保留（拆分自原文件 821-1041 行），行为
// 零变化。公共夹具见 useWorkspaceSessionObservers.test-utils.tsx。
import { act, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { LogicalCodebaseBootstrapNoticeDto } from "../api/types";
import { notifyLifecycleInvalidated } from "../state/lifecycle-workbench-store";
import {
  observerOptions,
  renderObserverHook,
} from "./useWorkspaceSessionObservers.test-utils";

afterEach(() => {
  vi.useRealTimers();
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

  // P4（无限轮询修复）：bootstrap 是 LC 级端点——只有 kind === "logical"
  // 的条目请求；普通单仓 codebase 用仓库 id 打 LC 端点必然 404 且被静默
  // 吞掉，不应产生请求。
  it("requests bootstrap only for logical codebases", async () => {
    const getLogicalCodebaseBootstrap = vi.fn(async () => ({
      project_id: "project_1",
      logical_codebase_id: "lc_0001",
      authority_root: "/tmp/lc",
      membership_revision: 1,
      policy: null,
      steps: [],
      planning_ready: false,
      notices: [],
    }));
    const view = renderObserverHook(observerOptions({
      listCodebases: async () => ({
        codebases: [
          {
            id: "repo_0001",
            name: "Repo",
            kind: "single_repo" as const,
            repository_id: "repo_0001",
            logical_codebase_id: null,
            member_count: 1,
          },
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
      getLogicalCodebaseBootstrap,
    }));

    await waitFor(() => expect(getLogicalCodebaseBootstrap).toHaveBeenCalled());
    // 只对 logical 条目请求一次，且用其 logical_codebase_id。
    expect(getLogicalCodebaseBootstrap).toHaveBeenCalledTimes(1);
    expect(getLogicalCodebaseBootstrap).toHaveBeenCalledWith("project_1", "lc_0001");

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
