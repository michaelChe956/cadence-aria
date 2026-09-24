import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import type { CockpitInboxItem } from "../../../state/workspace-cockpit-projection";
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
  };
}

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
    expect(within(inbox).getByRole("button", { name: "确认" })).toHaveClass("btn-primary");
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
