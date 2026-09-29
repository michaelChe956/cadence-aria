import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import {
  c1WaitingItem,
  type CockpitInboxItem,
} from "../../../state/workspace-cockpit-projection";
import { logicalCodebaseBootstrapItem } from "../../../state/workspace-cockpit-projection";
import type {
  C1WaitingItem,
  LogicalCodebaseBootstrapNoticeDto,
} from "../../../api/types";
import { CockpitInbox } from "./CockpitInbox";

const actions = mockActions();

const gateItem: CockpitInboxItem = {
  id: "session_001:gate:gate_001",
  kind: "gate",
  severity: 2,
  title: "需要人工确认",
  summary: "等待人工确认",
  triage: false,
  source: "gate",
  createdAt: null,
  gate: {
    key: "gate_001",
    kind: "human_gate",
    turn_id: "turn_001",
    stage: "human_confirm",
    flow_kind: "single_candidate",
    status: "open",
    trigger: null,
    remaining_budget: null,
    findings: [],
    resumable: false,
    triage: false,
    closed: null,
    closure_stage: null,
    opened_at: "2026-09-15T00:00:00.000Z",
    turn: null,
    action_block_reason: null,
    terminate_block_reason: null,
  },
  inlineError: null,
  choice: null,
};

const artifactVersions = [
  {
    version: 3,
    markdown: "# 发布方案 v3\n\n- 修复 Issue 索引\n- 补齐流式渲染",
    generated_by: "claude_code" as const,
    reviewed_by: "codex" as const,
    review_verdict: "pass" as const,
    confirmed_by: null,
    is_current: true,
    created_at: "2026-09-17T10:00:00Z",
    source_node_id: "node-artifact",
  },
] as const;

// F-31（v37 复验 #2）：story/design author 门（产物确认阶段）在待处理抽屉内的
// 动作面与主区门卡对齐——reviewer 启用三动作（确认定稿/确认并评审/终止），
// 未启用两动作（确认定稿/终止）。
const authorConfirmGate: NonNullable<CockpitInboxItem["gate"]> = {
  key: "stage:author_confirm",
  kind: "human_gate",
  turn_id: null,
  stage: "author_confirm",
  flow_kind: "legacy",
  status: "open",
  trigger: null,
  remaining_budget: null,
  findings: [],
  resumable: false,
  triage: false,
  closed: null,
  closure_stage: null,
  opened_at: "",
  turn: null,
  action_block_reason: null,
  terminate_block_reason: null,
};

const authorConfirmItem = (reviewAvailable: boolean): CockpitInboxItem => ({
  id: "session_001:gate:stage:author_confirm",
  kind: "gate",
  severity: 1,
  title: "需要人工确认",
  summary: "等待人工确认",
  triage: false,
  source: "gate",
  createdAt: null,
  gate: { ...authorConfirmGate, review_available: reviewAvailable },
  inlineError: null,
  choice: null,
});

// REQ-PCG-01：整组 Draft 确认门——key=`node:${node_id}`，动作面 [确认整组][终止]。
const batchConfirmItem: CockpitInboxItem = {
  id: "session_001:gate:node:node_batch",
  kind: "gate",
  severity: 1,
  title: "确认整组 Work Item Draft",
  summary: "等待整组 Work Item Draft 确认",
  triage: false,
  source: "gate",
  createdAt: "2026-09-22T00:00:00.000Z",
  gate: {
    ...gateItem.gate!,
    key: "node:node_batch",
    kind: "batch_confirm",
    turn_id: null,
    stage: "author_confirm",
    opened_at: "2026-09-22T00:00:00.000Z",
  },
  inlineError: null,
  choice: null,
};

// REQ-PCG-02：compile recovery 门——动作面 [继续][放弃并回滚][转人工]。
const recoveryItem: CockpitInboxItem = {
  id: "session_001:gate:node:node_recovery",
  kind: "gate",
  severity: 1,
  title: "Final Compile 恢复",
  summary: "Final Compile 中断，等待恢复动作 · Final Compile 需要恢复：provider timeout",
  triage: false,
  source: "gate",
  createdAt: "2026-09-22T00:00:00.000Z",
  gate: {
    ...gateItem.gate!,
    key: "node:node_recovery",
    kind: "compile_recovery",
    turn_id: null,
    opened_at: "2026-09-22T00:00:00.000Z",
  },
  inlineError: null,
  choice: null,
};

// F-50 §4.1-1/6/7/8（第一批布局减负）：抽屉只保留顶栏「待处理」，收件箱内部
// 改语义分组；滚动归抽屉 body；协议错误压成紧凑 alert 条（原文进折叠详情）；
// 无安全重放命令的重试不再以灰置占位（advance 来源才渲染）。
// F-50 裁决 6：投影后协议错误条目的 title 是中文主显 lead，code 走
// protocolErrorCode（mono 副行），原文进 summary（折叠详情）。
const staleLeaseErrorItem: CockpitInboxItem = {
  id: "session_001:hard_error:protocol:STALE_DRIVER_LEASE",
  kind: "hard_error",
  severity: 3,
  title: "连接租约已失效",
  summary: "driver connection no longer holds the lease for write message advance",
  triage: false,
  source: "protocol_error",
  createdAt: null,
  gate: null,
  inlineError: null,
      choice: null,
  protocolErrorCode: "STALE_DRIVER_LEASE",
};

const advanceErrorItem: CockpitInboxItem = {
  id: "session_001:hard_error:advance:cmd_1",
  kind: "hard_error",
  severity: 3,
  title: "推进被拒",
  summary: "ADVANCE_STAGE_INVALID · stage mismatch",
  triage: false,
  source: "advance",
  createdAt: null,
  gate: null,
  inlineError: null,
  choice: null,
};
describe("CockpitInbox F-50 layout", () => {
  it("不再内嵌「待处理」标题，按连接问题/需要人工处理分组", () => {
    render(
      <CockpitInbox
        items={[staleLeaseErrorItem, gateItem]}
        actions={mockActions()}
        actionableSessionId="session_001"
      />,
    );

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).queryByRole("heading", { name: "待处理" })).toBeNull();
    expect(within(inbox).getByRole("heading", { name: "连接问题" })).toBeVisible();
    expect(within(inbox).getByRole("heading", { name: "需要人工处理" })).toBeVisible();
    // 连接问题分组在需要人工处理之前（错误优先）。
    const connection = within(inbox).getByRole("heading", { name: "连接问题" });
    const human = within(inbox).getByRole("heading", { name: "需要人工处理" });
    expect(
      (connection.compareDocumentPosition(human) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0,
    ).toBe(true);
  });

  it("收件箱不再自滚：滚动容器归抽屉 body", () => {
    render(<CockpitInbox items={[]} actions={mockActions()} />);

    expect(screen.getByTestId("cockpit-inbox").className).not.toContain("overflow-auto");
  });

  it("协议错误为紧凑 alert 条：role=alert、原文只在折叠详情内", () => {
    render(
      <CockpitInbox
        items={[staleLeaseErrorItem]}
        actions={mockActions()}
        actionableSessionId="session_001"
        onRetakeLease={vi.fn()}
      />,
    );

    const row = screen.getByTestId("cockpit-inbox-item-hard_error");
    expect(row).toHaveAttribute("role", "alert");
    const details = within(row).getByTestId("cockpit-inbox-error-details");
    expect(details.tagName).toBe("DETAILS");
    expect(
      within(details).getByText(/driver connection no longer holds the lease/),
    ).toBeInTheDocument();
  });

  it("无安全重放命令的重试不渲染；advance 来源保留「重试推进」", () => {
    render(
      <CockpitInbox
        items={[staleLeaseErrorItem, advanceErrorItem]}
        actions={mockActions()}
        actionableSessionId="session_001"
        onRetry={vi.fn()}
      />,
    );

    const retryButtons = screen.getAllByRole("button", { name: "重试推进" });
    expect(retryButtons).toHaveLength(1);
    expect(retryButtons[0].closest('[data-testid="cockpit-inbox-item-hard_error"]')).toHaveTextContent(
      "推进被拒",
    );
    expect(screen.queryByRole("button", { name: "重试" })).toBeNull();
  });

  it("协议错误中文主显 + mono 错误码副行 + 原文/不可重试说明折叠（裁决 5/6）", () => {
    render(
      <CockpitInbox
        items={[staleLeaseErrorItem]}
        actions={mockActions()}
        actionableSessionId="session_001"
        onRetakeLease={vi.fn()}
      />,
    );

    const row = screen.getByTestId("cockpit-inbox-item-hard_error");
    expect(within(row).getByText("连接租约已失效")).toBeVisible();
    const code = within(row).getByTestId("cockpit-inbox-error-code");
    expect(code).toHaveTextContent("STALE_DRIVER_LEASE");
    expect(code.className).toContain("aria-mono");
    expect(within(row).getByText("本连接已失去写入租约，当前操作未提交。")).toBeVisible();
    const details = within(row).getByTestId("cockpit-inbox-error-details");
    expect(within(details).getByText(/driver connection no longer holds/)).toBeInTheDocument();
    expect(within(details).getByText("该错误不支持安全重试")).toBeInTheDocument();
  });

  it("REQ-CFC-06 规则改短句 + 详情折叠（裁决 8）", () => {
    render(<CockpitInbox items={[]} actions={mockActions()} />);

    const rule = screen.getByTestId("inbox-bulk-rule");
    expect(rule).toHaveTextContent("跨会话批量确认 · 每个会话一次");
    const details = screen.getByTestId("inbox-bulk-rule-details");
    expect(details.tagName).toBe("DETAILS");
    // 默认收起：完整规则与 REQ 编号只在展开后可见。
    expect(details).not.toHaveAttribute("open");
    expect(within(details).getByText(/每会话仅一个开态门/)).toBeInTheDocument();
    expect(within(details).getByText(/REQ-CFC-06/)).toBeInTheDocument();
  });

  it("空 summary 不渲染摘要行，仅保留标题（F-50 fix1）", () => {
    render(<CockpitInbox items={[{ ...gateItem, summary: "" }]} />);

    const row = screen.getByTestId("cockpit-inbox-item-gate");
    expect(within(row).getByText("需要人工确认")).toBeVisible();
    expect(row.querySelectorAll("p")).toHaveLength(1);
  });
});

function mockActions(): CockpitActionFacade {
  return {
    confirm: vi.fn(),
    confirmReview: vi.fn(),
    feedback: vi.fn(),
    terminate: vi.fn(),
    advance: vi.fn(),
    adoptReview: vi.fn(),
    confirmBatch: vi.fn(async () => undefined),
    recoverCompile: vi.fn(async () => undefined),
    recoverCandidate: vi.fn(),
    retryInitialization: vi.fn(async () => undefined),
    confirmTakeover: vi.fn(async () => undefined),
    rebind: vi.fn(),
    resumeRepositoryInitialization: vi.fn(async () => undefined),
    sendBootstrapAction: vi.fn(async () => undefined),
  };
}

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

describe("CockpitInbox", () => {
  it("uses the shared dangerous confirmation and feedback editor for all actionable cards", () => {
    render(
      <CockpitInbox
        items={[
          gateItem,
          {
            id: "session_001:stopped:session_001",
            kind: "stopped",
            severity: 2,
            title: "会话停在停点",
            summary: "等待人工接管后继续",
            triage: false,
            source: "session_status",
            createdAt: null,
            gate: null,
            inlineError: null,
            choice: null,
          },
          {
            id: "session_001:hard_error:error",
            kind: "hard_error",
            severity: 3,
            title: "引擎错误",
            summary: "错误",
            triage: false,
            source: "engine_error",
            createdAt: null,
            gate: null,
            inlineError: null,
            choice: null,
          },
        ]}
        actions={actions}
        onTakeover={vi.fn(async () => undefined)}
        actionableSessionId="session_001"
      />,
    );

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByTestId("gate-feedback-editor")).toBeVisible();
    expect(within(inbox).getAllByTestId("confirm-twice-button")).toHaveLength(3);
  });

  it("explains the current plan, exposes bulk selection text, and makes confirmation primary", () => {
    render(
      <CockpitInbox
        items={[gateItem]}
        actions={actions}
        actionableSessionId="session_001"
        onBulkConfirm={vi.fn()}
        artifactVersions={artifactVersions}
        latestReviewSummary="审核通过，允许人工确认"
      />,
    );

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByText("等待确认的内容")).toBeVisible();
    expect(within(inbox).getByText("发布方案 v3")).toBeVisible();
    expect(within(inbox).getByText("版本 3 · 审核通过")).toBeVisible();
    expect(within(inbox).getByText("修复 Issue 索引；补齐流式渲染")).toBeVisible();
    expect(within(inbox).getByText("审核通过，允许人工确认")).toBeVisible();
    expect(within(inbox).getByText("选择此门以批量确认")).toBeVisible();
    expect(within(inbox).getByRole("button", { name: "确认" })).toHaveClass("bg-emerald-600");
    expect(within(inbox).queryByText("未同步门命令，将以新命令提交")).toBeNull();
  });

  it("does not render optional gate details when no optional props are supplied", () => {
    render(
      <CockpitInbox
        items={[gateItem]}
        actions={actions}
        actionableSessionId="session_001"
      />,
    );

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).queryByText("等待确认的内容")).toBeNull();
    expect(within(inbox).queryByText("未同步门命令，将以新命令提交")).toBeNull();
  });

  it("author 门 reviewer 启用：抽屉内三动作，定稿/评审/终止各自发对应命令（F-31）", async () => {
    const user = userEvent.setup();
    const gateActions = mockActions();
    render(
      <CockpitInbox
        items={[authorConfirmItem(true)]}
        actions={gateActions}
        actionableSessionId="session_001"
      />,
    );

    const inbox = screen.getByTestId("cockpit-inbox");
    await user.click(within(inbox).getByRole("button", { name: "确认定稿" }));
    expect(gateActions.confirm).toHaveBeenCalledOnce();

    await user.click(within(inbox).getByRole("button", { name: "确认并评审" }));
    expect(gateActions.confirmReview).toHaveBeenCalledOnce();

    await user.click(within(inbox).getByRole("button", { name: "终止此门" }));
    expect(gateActions.terminate).not.toHaveBeenCalled();
    await user.click(within(inbox).getByRole("button", { name: "确认终止此门" }));
    expect(gateActions.terminate).toHaveBeenCalledOnce();
  });

  it("author 门 reviewer 未启用：两动作（定稿/终止），不露「确认并评审」（F-31）", () => {
    render(
      <CockpitInbox
        items={[authorConfirmItem(false)]}
        actions={actions}
        actionableSessionId="session_001"
      />,
    );

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByRole("button", { name: "确认定稿" })).toBeVisible();
    expect(within(inbox).queryByRole("button", { name: "确认并评审" })).toBeNull();
    expect(within(inbox).getByRole("button", { name: "终止此门" })).toBeVisible();
    // author 门是 HTTP confirm 通路：无 typed 反馈编辑器（维持既有纪律）。
    expect(within(inbox).queryByTestId("gate-feedback-editor")).toBeNull();
  });

  // v40 复验 #3：review 已完成（存在未被修订取代的最新 review 报告）时，作者门
  // 在待处理抽屉内补齐第四动作「采纳 Review 意见」——与主区/产物审核面板同款
  // 行为，经门面 adoptReview 派发（预填修订反馈+切回对话视图，纯客户端）。
  it("author 门 review 已完成：抽屉内四动作，采纳 Review 意见经门面派发（v40 #3）", async () => {
    const user = userEvent.setup();
    const gateActions = mockActions();
    render(
      <CockpitInbox
        items={[authorConfirmItem(true)]}
        actions={gateActions}
        actionableSessionId="session_001"
        latestReviewSummary="[review_findings]\n1. severity: major\n   message: 遗漏边界场景"
      />,
    );

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByRole("button", { name: "确认定稿" })).toBeVisible();
    expect(within(inbox).getByRole("button", { name: "确认并评审" })).toBeVisible();
    expect(within(inbox).getByRole("button", { name: "采纳 Review 意见" })).toBeVisible();
    expect(within(inbox).getByRole("button", { name: "终止此门" })).toBeVisible();

    await user.click(within(inbox).getByRole("button", { name: "采纳 Review 意见" }));
    expect(gateActions.adoptReview).toHaveBeenCalledOnce();
    // 采纳是纯客户端预填：不得误触任何 WS/HTTP 决策命令。
    expect(gateActions.confirm).not.toHaveBeenCalled();
    expect(gateActions.confirmReview).not.toHaveBeenCalled();
    expect(gateActions.terminate).not.toHaveBeenCalled();
  });

  it("author 门无 review 结果：不露「采纳 Review 意见」，维持三动作（v40 #3）", () => {
    render(
      <CockpitInbox
        items={[authorConfirmItem(true)]}
        actions={actions}
        actionableSessionId="session_001"
      />,
    );

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).queryByRole("button", { name: "采纳 Review 意见" })).toBeNull();
    expect(within(inbox).getByRole("button", { name: "确认定稿" })).toBeVisible();
    expect(within(inbox).getByRole("button", { name: "确认并评审" })).toBeVisible();
    expect(within(inbox).getByRole("button", { name: "终止此门" })).toBeVisible();
  });

  it("only warns about a new feedback command while a typed gate has a repair reservation", () => {
    render(
      <CockpitInbox
        items={[gateItem]}
        actions={actions}
        actionableSessionId="session_001"
        repairReservation={{
          token: "reservation-1",
          owner_session_id: "session_001",
          owner_run_id: "run-1",
          provider_start_idempotency_key: "start-1",
          state: "reserved",
          commit_id: null,
        }}
      />,
    );

    expect(screen.getByText("未同步门命令，将以新命令提交")).toBeVisible();
  });

  it("renders the block reason instead of any gate controls for a stale projection", () => {
    render(
      <CockpitInbox
        items={[
          {
            ...gateItem,
            gate: {
              ...gateItem.gate!,
              action_block_reason: "terminal_stage",
              terminate_block_reason: "terminal_stage",
            },
          },
        ]}
        actions={actions}
        actionableSessionId="session_001"
        onBulkConfirm={vi.fn()}
      />,
    );

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByText("已离开人工确认门")).toBeVisible();
    expect(within(inbox).queryByLabelText("选择 需要人工确认")).toBeNull();
    expect(within(inbox).queryByTestId("gate-feedback-editor")).toBeNull();
  });

  // F-21（v28 监控 0449）：plan 会话停在 human_confirm 的 context blocker/
  // author 失败门（phase_mismatch）——终止专属放行（terminate_block_reason
  // null）时门条渲染终止（二次确认），确认与反馈编辑器维持相位纪律不露出。
  it("renders a terminate-only gate row for a phase-mismatched plan gate (F-21)", async () => {
    const user = userEvent.setup();
    const gateActions = mockActions();
    render(
      <CockpitInbox
        items={[
          {
            ...gateItem,
            gate: {
              ...gateItem.gate!,
              action_block_reason: "phase_mismatch",
              terminate_block_reason: null,
            },
          },
        ]}
        actions={gateActions}
        actionableSessionId="session_001"
        onBulkConfirm={vi.fn()}
      />,
    );

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByRole("button", { name: "终止此门" })).toBeVisible();
    expect(within(inbox).queryByRole("button", { name: "确认" })).toBeNull();
    expect(within(inbox).queryByTestId("gate-feedback-editor")).toBeNull();
    // phase_mismatch 语义对 confirm 仍成立：批量勾选同样不提供。
    expect(within(inbox).queryByLabelText("选择 需要人工确认")).toBeNull();

    await user.click(within(inbox).getByRole("button", { name: "终止此门" }));
    expect(gateActions.terminate).not.toHaveBeenCalled();
    await user.click(within(inbox).getByRole("button", { name: "确认终止此门" }));
    expect(gateActions.terminate).toHaveBeenCalledOnce();
  });

  it("跨会话多选：观察会话的开态门可选（REQ-CFC-06 解锁 3.7 同会话限定）；当前会话门亦可选", async () => {
    const user = userEvent.setup();
    const onBulkConfirm = vi.fn();
    const current: CockpitInboxItem = {
      ...gateItem,
      id: "s1:gate:snapshot:2026-09-15T00:00:00Z:fp",
      title: "方案定稿 gate",
      gate: {
        ...gateItem.gate!,
        key: "snapshot:2026-09-15T00:00:00Z:fp",
      },
    };
    const observed: CockpitInboxItem = {
      ...gateItem,
      id: "s2:gate:g2",
      title: "方案定稿 gate",
      gate: { ...gateItem.gate!, key: "g2" },
    };
    const stopped: CockpitInboxItem = {
      ...gateItem,
      id: "s1:stop",
      kind: "stopped",
      title: "stop",
      gate: null,
    };
    const hardError: CockpitInboxItem = {
      ...gateItem,
      id: "s1:error",
      kind: "hard_error",
      title: "error",
      gate: null,
    };

    render(
      <CockpitInbox
        items={[current, observed, stopped, hardError]}
        actions={actions}
        actionableSessionId="s1"
        onBulkConfirm={onBulkConfirm}
      />,
    );

    const gateChoices = screen.getAllByRole("checkbox", { name: "选择 方案定稿 gate" });
    expect(gateChoices).toHaveLength(2);
    await user.click(gateChoices[0]);
    await user.click(gateChoices[1]);
    expect(screen.queryByLabelText("选择 stop")).toBeNull();
    expect(screen.queryByLabelText("选择 error")).toBeNull();
    const button = screen.getByRole("button", { name: /批量确认/ });
    expect(button).toHaveTextContent("2");
    await user.click(button);

    expect(onBulkConfirm).toHaveBeenCalledWith(
      expect.arrayContaining([
        expect.objectContaining({ id: "s1:gate:snapshot:2026-09-15T00:00:00Z:fp" }),
        expect.objectContaining({ id: "s2:gate:g2" }),
      ]),
    );
    expect(screen.queryByRole("button", { name: /批量确认 2/ })).toBeNull();
  });

  it("renders a custom empty hint when provided", () => {
    render(<CockpitInbox items={[]} actions={actions} emptyHint="会话尚未开始" />);

    expect(screen.getByText("会话尚未开始")).toBeVisible();
  });

  it("keeps the default empty text without a hint", () => {
    render(<CockpitInbox items={[]} actions={actions} />);

    expect(screen.getByText("暂无待处理项")).toBeVisible();
  });

  const staleLeaseItem: CockpitInboxItem = {
    id: "session_001:hard_error:protocol:STALE_DRIVER_LEASE",
    kind: "hard_error",
    severity: 3,
    title: "协议错误 STALE_DRIVER_LEASE",
    summary: "driver connection no longer holds the lease for write message advance",
    triage: false,
    source: "protocol_error",
    createdAt: null,
    gate: null,
    inlineError: null,
      choice: null,
    protocolErrorCode: "STALE_DRIVER_LEASE",
  };

  it("offers a confirmed lease retake on the stale driver lease error (F-11)", async () => {
    const user = userEvent.setup();
    const onRetakeLease = vi.fn();
    render(
      <CockpitInbox
        items={[staleLeaseItem]}
        actions={actions}
        actionableSessionId="session_001"
        onRetakeLease={onRetakeLease}
      />,
    );

    await user.click(screen.getByRole("button", { name: "重新接管" }));
    await user.click(screen.getByRole("button", { name: "确认重新接管" }));

    expect(onRetakeLease).toHaveBeenCalledTimes(1);
  });

  it("does not offer a lease retake for other hard errors or without a handler", () => {
    const { rerender } = render(
      <CockpitInbox
        items={[{ ...staleLeaseItem, protocolErrorCode: "OTHER_CODE" }]}
        actions={actions}
        actionableSessionId="session_001"
        onRetakeLease={vi.fn()}
      />,
    );
    expect(screen.queryByRole("button", { name: "重新接管" })).toBeNull();

    rerender(
      <CockpitInbox
        items={[staleLeaseItem]}
        actions={actions}
        actionableSessionId="session_001"
      />,
    );
    expect(screen.queryByRole("button", { name: "重新接管" })).toBeNull();
  });

  // REQ-DLS-03 规范句：STALE 手动错误面引用最近租约转移事件——抽屉错误条
  // （完整动作面归属地）渲染折叠摘要；非 STALE 硬错误或诊断缺省不渲染。
  it("renders the folded lease transfer summary on the stale driver lease error row (REQ-DLS-03)", () => {
    const leaseEvents = [
      {
        recorded_at: "2026-09-24T03:06:07.123456+00:00",
        event: "hold",
        connection_id: "conn-thief",
      },
      {
        recorded_at: "2026-09-24T03:07:08.123456+00:00",
        event: "write_rejected_stale",
        connection_id: "conn-self",
      },
    ];
    const { rerender } = render(
      <CockpitInbox
        items={[staleLeaseItem]}
        actions={actions}
        actionableSessionId="session_001"
        onRetakeLease={vi.fn()}
        leaseEvents={leaseEvents}
      />,
    );

    const row = screen.getByTestId("cockpit-inbox-item-hard_error");
    const summary = within(row).getByTestId("lease-diagnostics-summary");
    expect(summary).toHaveTextContent("最近租约转移");
    expect(within(summary).getAllByTestId("lease-diagnostics-event")).toHaveLength(2);
    expect(summary).toHaveTextContent("持有 · conn-thief");

    rerender(
      <CockpitInbox
        items={[{ ...staleLeaseItem, protocolErrorCode: "OTHER_CODE" }]}
        actions={actions}
        actionableSessionId="session_001"
        onRetakeLease={vi.fn()}
        leaseEvents={leaseEvents}
      />,
    );
    expect(screen.queryByTestId("lease-diagnostics-summary")).toBeNull();
  });

  // REQ-PCG-01：整组 Draft 确认门——[确认整组] 走 HTTP confirm 通路（门面
  // confirmBatch），[终止] 复用既有 WS abandon 二次确认；整组确认不是 WS confirm
  // 门，故不提供批量勾选、不出现 typed 反馈编辑器。
  it("routes the batch gate row to confirmBatch and the shared terminate (REQ-PCG-01)", async () => {
    const user = userEvent.setup();
    const gateActions = mockActions();
    render(
      <CockpitInbox
        items={[batchConfirmItem]}
        actions={gateActions}
        actionableSessionId="session_001"
        onBulkConfirm={vi.fn()}
      />,
    );

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByText("确认整组 Work Item Draft")).toBeVisible();
    expect(within(inbox).getByRole("button", { name: "确认整组" })).toBeVisible();
    expect(within(inbox).queryByTestId("gate-feedback-editor")).toBeNull();
    expect(within(inbox).queryByRole("checkbox")).toBeNull();

    await user.click(within(inbox).getByRole("button", { name: "确认整组" }));
    expect(gateActions.confirmBatch).toHaveBeenCalledOnce();
    expect(gateActions.confirm).not.toHaveBeenCalled();

    await user.click(within(inbox).getByRole("button", { name: "终止此门" }));
    expect(gateActions.terminate).not.toHaveBeenCalled();
    await user.click(within(inbox).getByRole("button", { name: "确认终止此门" }));
    expect(gateActions.terminate).toHaveBeenCalledOnce();
  });

  // REQ-PCG-02：recovery 门三动作按既有 recovery action 枚举原样派发；human_triage
  // 可携带原因（wire 可选 reason），不得映射成 confirm/feedback/abandon。
  it("routes the recovery gate row to the three existing recovery actions (REQ-PCG-02)", async () => {
    const user = userEvent.setup();
    const gateActions = mockActions();
    render(
      <CockpitInbox
        items={[recoveryItem]}
        actions={gateActions}
        actionableSessionId="session_001"
        onBulkConfirm={vi.fn()}
      />,
    );

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByText("Final Compile 恢复")).toBeVisible();
    expect(within(inbox).getByText(/provider timeout/)).toBeVisible();

    await user.click(within(inbox).getByRole("button", { name: "继续" }));
    expect(gateActions.recoverCompile).toHaveBeenLastCalledWith("continue");

    await user.click(within(inbox).getByRole("button", { name: "放弃并回滚" }));
    expect(gateActions.recoverCompile).toHaveBeenLastCalledWith("abort_and_rollback");

    await user.type(within(inbox).getByLabelText("转人工原因（可选）"), "  需要人工判断  ");
    await user.click(within(inbox).getByRole("button", { name: "转人工" }));
    expect(gateActions.recoverCompile).toHaveBeenLastCalledWith(
      "human_triage",
      "需要人工判断",
    );

    expect(gateActions.confirm).not.toHaveBeenCalled();
    expect(gateActions.confirmBatch).not.toHaveBeenCalled();
    expect(gateActions.feedback).not.toHaveBeenCalled();
    expect(gateActions.terminate).not.toHaveBeenCalled();
    // recovery 门不参与批量确认（批量 runner 只发 WS confirm 帧）。
    expect(within(inbox).queryByRole("checkbox")).toBeNull();
  });

  // REQ-PCG-02/F-30：门已关闭或会话终态（action_block_reason/terminate_block_reason
  // 同源）时只呈现诊断原因，两新门的写动作一律不露出。
  it.each([batchConfirmItem, recoveryItem])(
    "shows the block reason instead of write actions for a collapsed node gate (F-30)",
    (item) => {
      render(
        <CockpitInbox
          items={[
            {
              ...item,
              gate: {
                ...item.gate!,
                action_block_reason: "terminal_stage",
                terminate_block_reason: "terminal_stage",
              },
            },
          ]}
          actions={mockActions()}
          actionableSessionId="session_001"
        />,
      );

      expect(screen.getByText("已离开人工确认门")).toBeVisible();
      expect(screen.queryByRole("button", { name: "确认整组" })).toBeNull();
      expect(screen.queryByRole("button", { name: "继续" })).toBeNull();
      expect(screen.queryByRole("button", { name: "放弃并回滚" })).toBeNull();
      expect(screen.queryByRole("button", { name: "转人工" })).toBeNull();
      expect(screen.queryByRole("button", { name: "终止此门" })).toBeNull();
    },
  );

  // REQ-PCG-02：当前连接不是该会话的 driver（actionable=false）时，两新门的写动作
  // 一律不可用——恢复动作只由持锁连接提交。
  it("hides batch/recovery write actions for a non-driver connection (REQ-PCG-02)", () => {
    render(
      <CockpitInbox
        items={[batchConfirmItem, recoveryItem]}
        actions={mockActions()}
        actionableSessionId="session_other"
      />,
    );

    expect(screen.queryByRole("button", { name: "确认整组" })).toBeNull();
    expect(screen.queryByRole("button", { name: "继续" })).toBeNull();
    expect(screen.queryByRole("button", { name: "放弃并回滚" })).toBeNull();
    expect(screen.queryByRole("button", { name: "转人工" })).toBeNull();
  });
});

// F-50 视觉 v2（f50-ui-visual-spec-v2 §3）：抽屉门禁条目与门卡同一视觉常量、
// 错误条红只上图标+mono 码、分组标题弱化、组间距 space-y-4、终止 ghost。
describe("CockpitInbox 视觉 v2", () => {
  it("门禁条目复用门卡视觉：中性底+琥珀左线（不再整卡琥珀底）", () => {
    render(
      <CockpitInbox items={[gateItem]} actions={mockActions()} />,
    );

    const row = screen.getByTestId("cockpit-inbox-item-gate");
    expect(row.className).toContain("bg-white");
    expect(row.className).toContain("border-l-4");
    expect(row.className).toContain("border-l-amber-500/60");
    expect(row.className).not.toContain("bg-[var(--aria-gate-open-bg)]");
  });

  it("错误条：中性底+红色只上图标与 mono 码，不再红底整条", () => {
    render(
      <CockpitInbox
        items={[staleLeaseErrorItem]}
        actions={mockActions()}
        actionableSessionId="session_001"
      />,
    );

    const row = screen.getByTestId("cockpit-inbox-item-hard_error");
    expect(row.className).toContain("bg-white");
    expect(row.className).not.toContain("bg-[var(--aria-danger-soft)]");
    const code = within(row).getByTestId("cockpit-inbox-error-code");
    expect(code.className).toContain("text-red-600");
    const glyph = row.querySelector("svg.lucide-triangle-alert");
    expect(glyph?.getAttribute("class")).toContain("text-red-600");
  });

  it("分组标题弱化为 uppercase tracking-wider，两组之间 space-y-4", () => {
    render(
      <CockpitInbox
        items={[staleLeaseErrorItem, gateItem]}
        actions={mockActions()}
      />,
    );

    const connection = screen.getByRole("heading", { name: "连接问题" });
    expect(connection.className).toContain("tracking-wider");
    expect(connection.className).toContain("font-medium");
    const human = screen.getByRole("heading", { name: "需要人工处理" });
    expect(human.className).toContain("tracking-wider");
    // 两组共享同一个 space-y-4 容器（组间距 16px）。
    const connectionGroup = connection.closest("div.space-y-4");
    const humanGroup = human.closest("div.space-y-4");
    expect(connectionGroup).not.toBeNull();
    expect(connectionGroup).toBe(humanGroup);
  });

  it("终止走 ghost 形态：红字无底，不与主操作抢权重", () => {
    render(
      <CockpitInbox
        items={[gateItem]}
        actions={mockActions()}
        actionableSessionId="session_001"
      />,
    );

    const terminate = screen.getByRole("button", { name: "终止此门" });
    expect(terminate.className).toContain("text-red-600");
    expect(terminate.className).not.toContain("bg-white");
  });

  it("错误条终止钮同样走 ghost 形态：红字透明底，不与重试抢权重（F-50 fix2）", () => {
    render(
      <CockpitInbox
        items={[staleLeaseErrorItem]}
        actions={mockActions()}
        actionableSessionId="session_001"
      />,
    );

    const row = screen.getByTestId("cockpit-inbox-item-hard_error");
    const terminate = within(row).getByRole("button", { name: "终止此门" });
    expect(terminate.className).toContain("bg-transparent");
    expect(terminate.className).toContain("text-red-600");
    expect(terminate.className).not.toContain("bg-white");
  });

  it("批量勾选行为独立 text-sm 行", () => {
    render(
      <CockpitInbox
        items={[gateItem]}
        actions={mockActions()}
        onBulkConfirm={vi.fn()}
      />,
    );

    const label = screen.getByText("选择此门以批量确认").closest("label");
    expect(label?.className).toContain("text-sm");
  });
});

// P0 1.3（REQ-WIGA-05）Task 11：驾驶舱 choice 就地卡片——driver/coding socket
// 缺席时待答选择不再死等 WS；REST 作答按 command 幂等，202 保卡复查。
describe("CockpitInbox choice card (P0 1.3)", () => {
  const choiceItem = (
    overrides: Partial<NonNullable<CockpitInboxItem["choice"]>> = {},
  ): CockpitInboxItem => ({
    id: "session_001:choice:choice-1",
    kind: "choice",
    severity: 2,
    title: "选择请求待作答",
    summary: "拆分方案确认",
    triage: false,
    source: "choice",
    createdAt: null,
    gate: null,
    inlineError: null,
    choice: {
      sessionId: "session_001",
      choiceId: "choice-1",
      prompt: "拆分方案确认",
      expectedRunId: "run-1",
      questions: [
        {
          id: "q-1",
          prompt: "是否包含集成测试",
          options: [
            { id: "yes", label: "包含" },
            { id: "no", label: "不包含" },
          ],
          allow_multiple: false,
          allow_free_text: false,
        },
        {
          id: "q-2",
          prompt: "评审轮数",
          options: [
            { id: "one", label: "一轮" },
            { id: "two", label: "两轮" },
          ],
          allow_multiple: false,
          allow_free_text: false,
        },
      ],
      allowMultiple: false,
      allowFreeText: false,
      source: "workspace",
      status: "open",
      ...overrides,
    },
  });

  it("提交选择把两题各自独立答案交回答写通道（一个 command）", async () => {
    const onChoiceRespond = vi.fn();
    render(
      <CockpitInbox
        items={[choiceItem()]}
        actions={mockActions()}
        actionableSessionId="session_001"
        onChoiceRespond={onChoiceRespond}
      />,
    );

    await userEvent.click(screen.getByRole("radio", { name: "包含" }));
    await userEvent.click(screen.getByRole("radio", { name: "两轮" }));
    await userEvent.click(screen.getByRole("button", { name: "提交选择" }));

    expect(onChoiceRespond).toHaveBeenCalledTimes(1);
    const [item, payload] = onChoiceRespond.mock.calls[0];
    expect(item.choice.choiceId).toBe("choice-1");
    expect(payload.answers).toEqual([
      { question_id: "q-1", selected_option_ids: ["yes"], free_text: null },
      { question_id: "q-2", selected_option_ids: ["two"], free_text: null },
    ]);
  });

  it("202 后保卡显示处理中并禁用重复分配答案", () => {
    render(
      <CockpitInbox
        items={[choiceItem({ status: "resolving" })]}
        actions={mockActions()}
        actionableSessionId="session_001"
      />,
    );

    expect(screen.getByText("处理中")).toBeVisible();
    expect(screen.getByRole("radio", { name: "包含" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "提交选择" })).toBeDisabled();
  });

  it("旧投影无 run 信息时只提示刷新后作答，不猜 run", () => {
    render(
      <CockpitInbox
        items={[choiceItem({ expectedRunId: null })]}
        actions={mockActions()}
        actionableSessionId="session_001"
      />,
    );

    expect(screen.getByText(/刷新后作答/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "提交选择" })).toBeNull();
  });

  it("已失效（410）不再提供提交，不路由到新 run", () => {
    render(
      <CockpitInbox
        items={[choiceItem({ status: "expired" })]}
        actions={mockActions()}
        actionableSessionId="session_001"
      />,
    );

    expect(screen.getByText("已失效")).toBeVisible();
    expect(screen.queryByRole("button", { name: "提交选择" })).toBeNull();
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
    const call = facade.sendBootstrapAction.mock.calls[0][0];
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
