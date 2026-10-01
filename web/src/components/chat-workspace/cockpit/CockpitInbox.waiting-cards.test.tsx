import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import {
  c1WaitingItem,
  logicalCodebaseBootstrapItem,
  type CockpitInboxItem,
} from "../../../state/workspace-cockpit-projection";
import type {
  C1WaitingItem,
  LogicalCodebaseBootstrapNoticeDto,
} from "../../../api/types";
import { CockpitInbox } from "./CockpitInbox";
import { mockActions } from "./CockpitInbox.test-utils";

// 从 CockpitInbox.test.tsx 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// durable 等待项卡片测试族（C1 恢复 / C2 coding 等待 / C4 LC bootstrap / C5 repository 初始化）。
describe("C1 recovery cards", () => {
  it("renders durable identity and dispatches retry with a stable command id", async () => {
    const facade = mockActions();
    const item: CockpitInboxItem = {
      id: "c1:issue_0001:c1:advance_retry_failed:advance_0001",
      kind: "c1_recovery",
      severity: 2,
      title: "Failed advance 待显式重试",
      summary: "advance initialization failed · plan plan_0001 · attempt attempt_0001",
      triage: false,
      source: "c1_waiting",
      createdAt: null,
      gate: null,
      inlineError: null,
      choice: null,
      c1Info: {
        projectId: "project_0001",
        issueId: "issue_0001",
        itemId: "c1:advance_retry_failed:advance_0001",
        kind: "advance_retry_failed",
        reason: "advance initialization failed; original record stays failed",
        completedSteps: ["record_persisted"],
        targetLabel: "单仓 repo_physical_c1",
        planId: "plan_0001",
        sessionId: "wsp_0001",
        attemptId: "attempt_0001",
        gateId: null,
        possibleSideEffect: "provider start outcome unknown",
        actions: ["retry_initialization"],
        nextPhase: "journal_prepared",
        // C2 Task 12 additive（旧等待项缺省为空）。
        expectedVersion: null,
        actionContext: [],
        // C5 Task 6 additive（issue 级等待项无 operation/diagnostics）。
        operationId: null,
        diagnostics: null,
      },
    };
    render(<CockpitInbox items={[item]} actions={facade} />);
    const card = screen.getByTestId("c1-waiting-advance_retry_failed");
    expect(within(card).getByText(/已完成步骤/)).toBeVisible();
    expect(within(card).getByText(/可能副作用：provider start outcome unknown/)).toBeVisible();
    const retryButton = within(card).getByTestId("c1-action-retry_initialization");
    await userEvent.click(retryButton);
    expect(facade.retryInitialization).toHaveBeenCalledWith({
      kind: "retry_initialization",
      projectId: "project_0001",
      issueId: "issue_0001",
      planId: "plan_0001",
      commandId: "cmd-c1-retry-c1:advance_retry_failed:advance_0001",
      attemptId: "attempt_0001",
      checkpoint: "journal_prepared",
      confirmUnknownSideEffect: false,
    });
  });
});

// C2 Task 12（REQ-CRO-06）：coding 链等待项卡片——动作按钮从服务端
// action_context（稳定 command_id＋expected 版本）出站，经 facade 透传
// REST 发送器；前端不自行生成 command_id、不判定业务成功。
describe("C2 waiting cards", () => {
  const c2Item = (
    overrides: Partial<{
      kind: string;
      actions: string[];
      actionContext: { action: string; commandId: string; expectedVersion: number }[];
      gateId: string | null;
      attemptId: string;
      nextPhase: string;
    }>,
  ): CockpitInboxItem => ({
    id: "c1:issue_0001:c2:coding_restart_available:attempt_0002",
    kind: "c1_recovery",
    severity: 2,
    title: "Coding 终态可显式重启",
    summary: "coding attempt reached terminal state · attempt attempt_0002",
    triage: false,
    source: "c1_waiting",
    createdAt: null,
    gate: null,
    inlineError: null,
    choice: null,
    c1Info: {
      projectId: "project_0001",
      issueId: "issue_0001",
      itemId: "c2:coding_restart_available:attempt_0002",
      kind: overrides.kind ?? "coding_restart_available",
      reason: "coding attempt reached terminal state Aborted",
      completedSteps: [],
      targetLabel: "单仓 repo_physical_c1",
      planId: "plan_0001",
      sessionId: null,
      attemptId: overrides.attemptId ?? "attempt_0002",
      gateId: overrides.gateId ?? null,
      possibleSideEffect: null,
      actions: overrides.actions ?? ["restart_coding"],
      nextPhase: overrides.nextPhase ?? "coding_restarted",
      expectedVersion: 3,
      actionContext:
        overrides.actionContext ??
        [
          {
            action: "restart_coding",
            commandId: "cmd-c2-restart-attempt_0002",
            expectedVersion: 3,
          },
        ],
      // C5 Task 6 additive（issue 级等待项无 operation/diagnostics）。
      operationId: null,
      diagnostics: null,
    },
  });

  it("dispatches restart_coding with the server-issued command id and version", async () => {
    const facade = { ...mockActions(), restartCoding: vi.fn(async () => undefined) };
    render(<CockpitInbox items={[c2Item({})]} actions={facade} />);
    const card = screen.getByTestId("c1-waiting-coding_restart_available");
    const button = within(card).getByTestId("c1-action-restart_coding");
    expect(button).toHaveTextContent("重启 Coding");
    await userEvent.click(button);
    expect(facade.restartCoding).toHaveBeenCalledWith({
      kind: "restart_coding",
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "attempt_0002",
      commandId: "cmd-c2-restart-attempt_0002",
      expectedVersion: 3,
    });
  });

  it("answers gate actions from action_context via the gate REST facade", async () => {
    const facade = { ...mockActions(), respondGate: vi.fn(async () => undefined) };
    render(
      <CockpitInbox
        items={[
          c2Item({
            kind: "reviewer_configuration_missing",
            gateId: "gate_0007",
            actions: ["retry_review"],
            actionContext: [
              {
                action: "retry_review",
                commandId: "cmd-c2-gate-gate_0007-retry_review",
                expectedVersion: 7,
              },
            ],
          }),
        ]}
        actions={facade}
      />,
    );
    const card = screen.getByTestId("c1-waiting-reviewer_configuration_missing");
    const button = within(card).getByTestId("c1-action-retry_review");
    expect(button).toHaveTextContent("重试代码审查");
    await userEvent.click(button);
    expect(facade.respondGate).toHaveBeenCalledWith({
      kind: "gate_response",
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "attempt_0002",
      gateId: "gate_0007",
      actionId: "retry_review",
      commandId: "cmd-c2-gate-gate_0007-retry_review",
      expectedVersion: 7,
    });
  });

  it("keeps read-only waiting kinds actionless (completion unconfirmed)", () => {
    render(
      <CockpitInbox
        items={[
          c2Item({
            kind: "coding_completion_unconfirmed",
            actions: [],
            actionContext: [],
            nextPhase: "manual_recovery",
          }),
        ]}
        actions={mockActions()}
      />,
    );
    const card = screen.getByTestId("c1-waiting-coding_completion_unconfirmed");
    expect(card.querySelector("button")).toBeNull();
    expect(within(card).getByText(/下一阶段：manual_recovery/)).toBeVisible();
  });
});

describe("C4 logical codebase bootstrap cards", () => {
  it("renders waiting reason/side effect/next step and replays the same stable command without duplicating notices", async () => {
    const facade = mockActions();
    // 同一 notice key 两次到达（轮询重放）：投影层按稳定 key 去重为一条。
    const notice: LogicalCodebaseBootstrapNoticeDto = {
      key: "bootstrap:member_index:op_0001:aggregate_initialization_failed",
      step: "member_index",
      object_id: "op_0001",
      reason_code: "aggregate_initialization_failed",
      summary: "stage provider_turn: exit 1",
      external_side_effect: "aggregate_initialization_provider_turn",
      allowed_actions: ["retry"],
      next_step: "aggregate_index_active",
      created_at: "",
    };
    const projection = {
      project_id: "project_0001",
      logical_codebase_id: "lc_0001",
      membership_revision: 3,
    };
    const first = logicalCodebaseBootstrapItem(notice, projection);
    const duplicate = logicalCodebaseBootstrapItem(notice, projection);
    expect(duplicate.id).toBe(first.id);
    const deduped = new Map([[first.id, first]]);
    expect(deduped.size).toBe(1);

    render(<CockpitInbox items={[...deduped.values()]} actions={facade} />);
    const card = screen.getByTestId("lc-bootstrap-member_index");
    expect(within(card).getByText(/aggregate_initialization_failed/)).toBeVisible();
    expect(
      within(card).getByText(/可能外部副作用：aggregate_initialization_provider_turn/),
    ).toBeVisible();
    expect(within(card).getByText(/下一步：aggregate_index_active/)).toBeVisible();

    const button = within(card).getByTestId("lc-bootstrap-action-retry");
    await userEvent.click(button);
    // 一次点击派发一个稳定 command id；提交后按钮禁用，重复点击零出站
    //（服务端再按同 command 幂等重放，provider/action 计数不增）。
    expect(facade.sendBootstrapAction).toHaveBeenCalledTimes(1);
    const call = vi.mocked(facade.sendBootstrapAction).mock.calls[0][0];
    expect(call.commandId).toContain("bootstrap_member_index_op_0001");
    expect(call.step).toBe("member_index");
    expect(call.action).toBe("retry");
    expect(call.expectedRevision).toBe(3);
    expect(call.expectedObjectId).toBe("op_0001");
    expect(button).toBeDisabled();
    await userEvent.click(button);
    expect(facade.sendBootstrapAction).toHaveBeenCalledTimes(1);
    expect(screen.queryAllByTestId("lc-bootstrap-member_index")).toHaveLength(1);
  });
});

// C5 Task 6/7：project 级 repository 初始化失败等待项——经 c1WaitingItem
// 投影（无 issueId，id 即后端稳定 id）渲染结构化 diagnostics 与
// “网关恢复后继续”动作（稳定 command id cmd-repo-init-resume-{operationId}）。
describe("repository initialization waiting cards", () => {
  const failedProjectItem: C1WaitingItem = {
    id: "c1:project:project_0001:repository_init:op_init_0001",
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
      stderr_summary: "claude code gateway refused connection",
      changed_paths: ["repo-a/.claude/settings.json"],
      retryable: true,
    },
    project_id: "project_0001",
  };

  it("renders structured diagnostics and dispatches resume with a stable command id", async () => {
    const facade = mockActions();
    render(
      <CockpitInbox
        items={[c1WaitingItem(failedProjectItem, "project_0001")]}
        actions={facade}
      />,
    );
    const card = screen.getByTestId("c1-waiting-repository_initialization_failed");
    const diagnostics = within(card).getByTestId("repo-init-failure-diagnostics");
    expect(diagnostics).toHaveTextContent("pre_check");
    expect(diagnostics).toHaveTextContent("provider_unavailable");
    expect(diagnostics).toHaveTextContent("claude_code");
    expect(diagnostics).toHaveTextContent("claude code gateway refused connection");
    expect(diagnostics).toHaveTextContent("repo-a/.claude/settings.json");

    const button = within(card).getByTestId("repo-init-resume-action");
    expect(button).toHaveTextContent("网关恢复后继续");
    await userEvent.click(button);
    expect(facade.resumeRepositoryInitialization).toHaveBeenCalledWith({
      kind: "resume_repository_initialization",
      projectId: "project_0001",
      operationId: "op_init_0001",
      commandId: "cmd-repo-init-resume-op_init_0001",
    });
  });

  it("keeps the successor running item read-only without diagnostics", () => {
    const runningItem: C1WaitingItem = {
      ...failedProjectItem,
      id: "c1:project:project_0001:repository_init:op_init_0002",
      reason: "repository initialization resumed; successor operation running",
      actions: [],
      operation_id: "op_init_0002",
      diagnostics: null,
    };
    render(
      <CockpitInbox
        items={[c1WaitingItem(runningItem, "project_0001")]}
        actions={mockActions()}
      />,
    );
    const card = screen.getByTestId("c1-waiting-repository_initialization_failed");
    expect(card.querySelector("button")).toBeNull();
    expect(
      within(card).queryByTestId("repo-init-failure-diagnostics"),
    ).toBeNull();
  });
});
