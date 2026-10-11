import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  c1WaitingItem,
  logicalCodebaseBootstrapItem,
  type CockpitInboxItem,
  type LcResumeRecoveryInboxProjection,
} from "../../../state/workspace-cockpit-projection";
import { useProviderAvailabilityStore } from "../../../state/provider-availability-store";
import { useWorkspaceStore } from "../../../state/workspace-ws-store";
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

// D（Task 7.1 / anchor_t21，设计 §6）：LC 恢复等待卡动作面——显式 fresh
// 「启动新会话继续修订」＋既有守卫（observer 只读、断连/provider fail-closed、
// inflight 禁双击、失败回 waiting 复原、missing map 不猜漂移维度）。
// 卡面数据源是 C6.1 投影（item.recovery，wire allowed_actions/last_operation
// 的只读镜像）；operation_id 由页面一次用户动作生成（Page 层用例覆盖 wire）。
describe("anchor_t21 lc resume recovery cards", () => {
  beforeEach(() => {
    useProviderAvailabilityStore.setState({ snapshot: null });
    useWorkspaceStore.getState().reset();
  });

  const recoveryItem = (
    overrides: Partial<LcResumeRecoveryInboxProjection> & { id?: string } = {},
  ): CockpitInboxItem => ({
    id: overrides.id ?? "session_001:lc_resume_recovery:rec-1",
    kind: "lc_resume_recovery",
    severity: 2,
    // 标题随 state 派生（与 C6.1 selectCockpitInbox 同判据）。
    title: (overrides.state ?? "waiting") === "waiting"
      ? "旧原生会话已失效"
      : "正在启动新原生会话…",
    summary: `${overrides.reasonCode ?? "resume_fingerprint_mismatch"} · ${overrides.provider ?? "codex"} · ${overrides.runKind ?? "legacy_draft"}`,
    triage: false,
    source: "lc_resume_recovery",
    createdAt: "2026-10-11T00:02:00Z",
    gate: null,
    inlineError: null,
    choice: null,
    recovery: {
      recoveryId: "rec-1",
      state: "waiting",
      reasonCode: "resume_fingerprint_mismatch",
      provider: "codex",
      failedNodeId: "node-1",
      runKind: "legacy_draft",
      detectedAt: "2026-10-11T00:02:00Z",
      updatedAt: "2026-10-11T00:04:00Z",
      allowedActions: [],
      lastOperation: null,
      ...(overrides as Partial<LcResumeRecoveryInboxProjection>),
    },
  });

  it("anchor_t21_inbox_explicit_native_fresh_and_existing_guards", async () => {
    const onStartLcResumeFresh = vi.fn(() => true);
    const user = userEvent.setup();
    const view = render(
      <CockpitInbox
        items={[recoveryItem()]}
        onStartLcResumeFresh={onStartLcResumeFresh}
        actionableSessionId="session_001"
      />,
    );
    const card = screen.getByTestId("cockpit-inbox-item-lc_resume_recovery");
    // 设计 §6 文案：保留 workspace 事实 + 动作说明（不叫安全重试）。
    expect(
      within(card).getByText(
        "运行环境已变化，不能继续旧原生会话。当前工作区、产物、反馈与评审意见均保留。",
      ),
    ).toBeVisible();
    expect(
      within(card).getByText("只更换 AI 原生会话，将重新校验权限并继续当前任务"),
    ).toBeVisible();

    const button = screen.getByRole("button", { name: "启动新会话继续修订" });
    expect(button).toBeEnabled();
    await user.click(button);
    expect(onStartLcResumeFresh).toHaveBeenCalledTimes(1);
    expect(onStartLcResumeFresh).toHaveBeenCalledWith("rec-1");
    // inflight 禁双击：durable 事实未推进前第二次点击零出站。
    expect(button).toBeDisabled();
    await user.click(button);
    expect(onStartLcResumeFresh).toHaveBeenCalledTimes(1);

    // 受理（typed 事实推进 updated_at → reserved）：卡转「正在启动新原生会话…」，
    // 无写动作面。
    view.rerender(
      <CockpitInbox
        items={[
          recoveryItem({
            state: "fresh_reserved",
            updatedAt: "2026-10-11T00:05:00Z",
            lastOperation: {
              operation_id: "op-1",
              state: "reserved",
              reason: null,
              fresh_node: null,
            },
          }),
        ]}
        onStartLcResumeFresh={onStartLcResumeFresh}
        actionableSessionId="session_001"
      />,
    );
    const reservedCard = screen.getByTestId("cockpit-inbox-item-lc_resume_recovery");
    expect(reservedCard).toHaveTextContent("正在启动新原生会话…");
    expect(
      within(reservedCard).queryByRole("button", { name: "启动新会话继续修订" }),
    ).toBeNull();

    // prepare 失败回 waiting（updated_at 再推进）：回执 metadata 上卡面，
    // 按钮复原可再次显式发起（新用户动作＝新 operation_id，由页面生成）。
    view.rerender(
      <CockpitInbox
        items={[
          recoveryItem({
            state: "waiting",
            updatedAt: "2026-10-11T00:06:00Z",
            lastOperation: {
              operation_id: "op-1",
              state: "failed",
              reason: "prepare_failed",
              fresh_node: null,
            },
          }),
        ]}
        onStartLcResumeFresh={onStartLcResumeFresh}
        actionableSessionId="session_001"
      />,
    );
    const failedCard = screen.getByTestId("cockpit-inbox-item-lc_resume_recovery");
    expect(within(failedCard).getByText(/prepare_failed/)).toBeVisible();
    expect(screen.getByRole("button", { name: "启动新会话继续修订" })).toBeEnabled();
  });

  it("anchor_t21_recovery_card_observer_shows_facts_without_write_action", () => {
    // observer（非当前会话，id 前缀 session_002）：只显示事实，不出写动作（设计 §6）。
    render(
      <CockpitInbox
        items={[
          recoveryItem({
            id: "session_002:lc_resume_recovery:rec-obs",
            recoveryId: "rec-obs",
          }),
        ]}
        onStartLcResumeFresh={vi.fn(() => true)}
        actionableSessionId="session_001"
      />,
    );
    const observed = screen.getByTestId("cockpit-inbox-item-lc_resume_recovery");
    expect(observed).toHaveTextContent("旧原生会话已失效");
    expect(within(observed).queryByRole("button")).toBeNull();
  });

  it("anchor_t21_recovery_card_disables_when_disconnected_or_provider_unavailable", () => {
    // 断连守卫：按钮可见但禁用并给原因。
    const disconnected = render(
      <CockpitInbox
        items={[recoveryItem()]}
        onStartLcResumeFresh={vi.fn(() => true)}
        actionableSessionId="session_001"
        resumeFreshConnectionBlocked
      />,
    );
    const disconnectedButton = screen.getByRole("button", {
      name: "启动新会话继续修订",
    });
    expect(disconnectedButton).toBeDisabled();
    expect(screen.getByText(/连接已断开/)).toBeVisible();
    disconnected.unmount();

    // provider 不可用守卫（REQ-PPS-03 同源：已知不可用 fail-closed）。
    useWorkspaceStore.setState({
      providers: { author: "codex", reviewer: "codex" },
    });
    useProviderAvailabilityStore.setState({
      snapshot: {
        schema_version: 1,
        generation: 1,
        checked_at: "2026-10-12T00:00:00Z",
        state_status: "ready",
        state_error: null,
        real_workflow_blocked: false,
        test_provider_enabled: false,
        providers: [
          {
            provider: "codex",
            display_name: "Codex",
            available: false,
            version: null,
            reason_code: "command_missing",
            reason: "CLI 未安装",
            checked_at: "2026-10-12T00:00:00Z",
            install_hint: "",
          },
        ],
      },
    });
    render(
      <CockpitInbox
        items={[recoveryItem()]}
        onStartLcResumeFresh={vi.fn(() => true)}
        actionableSessionId="session_001"
      />,
    );
    const providerBlockedButton = screen.getByRole("button", {
      name: "启动新会话继续修订",
    });
    expect(providerBlockedButton).toBeDisabled();
    expect(screen.getByText(/Codex 当前不可用：CLI 未安装/)).toBeVisible();
  });

  it("anchor_t21_recovery_card_missing_map_copy_does_not_guess_drift", () => {
    render(
      <CockpitInbox
        items={[
          recoveryItem({
            reasonCode: "resume_fingerprint_unavailable",
          }),
        ]}
        onStartLcResumeFresh={vi.fn(() => true)}
        actionableSessionId="session_001"
      />,
    );
    const card = screen.getByTestId("cockpit-inbox-item-lc_resume_recovery");
    expect(within(card).getByText("无法验证旧会话冻结信息")).toBeVisible();
    expect(within(card).queryByText(/运行环境已变化/)).toBeNull();
  });
});
