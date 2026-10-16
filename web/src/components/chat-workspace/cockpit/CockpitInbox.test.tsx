import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { CockpitInboxItem } from "../../../state/workspace-cockpit-projection";
import { CockpitInbox } from "./CockpitInbox";
import { mockActions } from "./CockpitInbox.test-utils";

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

  // 批 2 P5（REQ-WIGA-08 C1 前端半）：enabled 精确绑定当前 enrollment 的
  // stopped 卡不渲染接管按钮（服务端 P4 已拒，隐藏仅同步呈现），托管卡给
  // 对话窗口继续指引；非托管 stopped 卡接管按钮原样保留。
  it("hides takeover on managed stopped cards and keeps it for unmanaged ones", () => {
    const stoppedBase: CockpitInboxItem = {
      id: "session_001:stopped:session_001",
      kind: "stopped",
      severity: 2,
      title: "会话停在停点",
      summary: "停点原因：stopped_needs_human · 等待人工接管后继续",
      triage: false,
      source: "session_status",
      createdAt: null,
      gate: null,
      inlineError: null,
      choice: null,
    };

    render(
      <CockpitInbox
        items={[
          { ...stoppedBase, automationManaged: true, summary: "停点原因：stopped_needs_human · 自动化托管中——在对话窗口补说明继续" },
          { ...stoppedBase, id: "session_002:stopped:session_002" },
        ]}
        actions={actions}
        onTakeover={vi.fn(async () => undefined)}
        actionableSessionId="session_001"
      />,
    );

    const rows = screen.getAllByTestId("cockpit-inbox-item-stopped");
    expect(rows).toHaveLength(2);
    // 托管卡：无接管按钮，只给托管指引。
    expect(within(rows[0]!).queryByRole("button", { name: "接管" })).toBeNull();
    expect(within(rows[0]!).getByTestId("stopped-managed-hint")).toBeVisible();
    // 非托管卡：接管按钮原样保留（回归）。
    expect(within(rows[1]!).getByRole("button", { name: "接管" })).toBeVisible();
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


// 「登记并重试」按钮（owner 2026-10-12 直批）：trust 缺失错误卡按稳定码
// 识别（投影已解析 provider）——按钮显式授权登记；成功回执后切
// 「已登记，请显式重新生成」态，不再渲染按钮（不自动重新生成）。
describe("CockpitInbox trust register button", () => {
  const trustMissingItem: CockpitInboxItem = {
    id: "session_001:hard_error:error",
    kind: "hard_error",
    severity: 3,
    title: "引擎错误",
    summary:
      "provider_trust_missing: trust_entry_missing: canonical root 下未找到 codex 的用户级 workspace trust 条目",
    triage: false,
    source: "engine_error",
    createdAt: null,
    gate: null,
    inlineError: null,
    choice: null,
    trustMissing: { provider: "codex" },
    trustRegistered: false,
  };

  it("trust 缺失错误卡渲染「登记并重试」，点击经 facade 以 provider 出站", () => {
    const actions = mockActions();
    render(
      <CockpitInbox
        items={[trustMissingItem]}
        actions={actions}
        actionableSessionId="session_001"
      />,
    );

    const button = screen.getByTestId("cockpit-trust-register-button");
    expect(screen.queryByTestId("cockpit-trust-registered-hint")).toBeNull();
    fireEvent.click(button);
    expect(actions.registerTrustEntry).toHaveBeenCalledWith("codex");
  });

  it("成功回执后切「已登记，请显式重新生成」，不再渲染按钮", () => {
    render(
      <CockpitInbox
        items={[{ ...trustMissingItem, trustRegistered: true }]}
        actions={mockActions()}
        actionableSessionId="session_001"
      />,
    );

    expect(screen.getByTestId("cockpit-trust-registered-hint")).toHaveTextContent(
      "已登记，请显式重新生成",
    );
    expect(screen.queryByTestId("cockpit-trust-register-button")).toBeNull();
  });

  it("非 trust 缺失错误卡不渲染登记面（fail-closed 不猜 provider）", () => {
    render(
      <CockpitInbox
        items={[advanceErrorItem]}
        actions={mockActions()}
        actionableSessionId="session_001"
      />,
    );

    expect(screen.queryByTestId("cockpit-trust-register-button")).toBeNull();
    expect(screen.queryByTestId("cockpit-trust-registered-hint")).toBeNull();
  });
});

// 指引卡简化（owner 2026-10-12 直批冻结设计）：trust 未信任指引卡从
// 长文本卡改为「两行简洁说明＋可复制命令窗」——第一行直白告知该目录
// 尚未被哪些 provider 信任（单复数按实际）；第二行怎么做；命令窗带
// 复制按钮；「登记并重试」保留；冗长诊断收进「查看诊断详情」折叠。
describe("CockpitInbox trust guidance card simplification", () => {
  const trustCommand =
    "mkdir -p '/home/u/.kimi-code/workspace-trust' && printf '{\"root\":\"/agg/root\",\"trustedAt\":%s}' \"$(date +%s%3N)\" > '/home/u/.kimi-code/workspace-trust/wd_root_000000000000' # GNU/Linux（date +%s%3N 需 GNU date）";

  const structuredTrustItem: CockpitInboxItem = {
    id: "session_001:hard_error:error",
    kind: "hard_error",
    severity: 3,
    title: "引擎错误",
    summary:
      "provider_trust_missing: trust_entry_missing: canonical root /agg/root 下未找到 kimi_code 的用户级 workspace trust 条目（entry absent）；原拒绝：spawn denied；人工登记指导：……（长文本）",
    triage: false,
    source: "engine_error",
    createdAt: null,
    gate: null,
    inlineError: null,
    choice: null,
    trustMissing: {
      provider: "kimi_code",
      providers: ["kimi_code"],
      canonicalRoot: "/agg/root",
      commandMarkdown: trustCommand,
      retryAction: "人工登记后显式重新生成",
    },
    trustRegistered: false,
  };

  it("两行说明＋命令窗＋复制按钮＋登记按钮齐备；诊断进折叠", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    const actions = mockActions();
    render(
      <CockpitInbox
        items={[structuredTrustItem]}
        actions={actions}
        actionableSessionId="session_001"
      />,
    );

    // 第一行：直白告知（provider 显示名 Kimi，单数）。
    expect(screen.getByTestId("cockpit-trust-missing-headline")).toHaveTextContent(
      "该目录（/agg/root）尚未被 Kimi 信任",
    );
    // 第二行：怎么做（按钮一键登记 / 复制命令自行登记）。
    const actionLine = screen.getByTestId("cockpit-trust-missing-action");
    expect(actionLine).toHaveTextContent("点击下方按钮一键登记");
    expect(actionLine).toHaveTextContent("复制命令自行登记后重新生成");

    // 命令窗：渲染后端同源命令，带复制按钮；复制写入剪贴板并反馈。
    const window = screen.getByTestId("cockpit-trust-command-window");
    expect(window.textContent).toContain(trustCommand);
    fireEvent.click(screen.getByTestId("cockpit-trust-command-copy"));
    await vi.waitFor(() => expect(writeText).toHaveBeenCalledWith(trustCommand));
    await vi.waitFor(() =>
      expect(screen.getByTestId("cockpit-trust-command-copy")).toHaveTextContent("已复制"),
    );

    // 「登记并重试」保留：点击仍经 facade 以 provider 出站。
    fireEvent.click(screen.getByTestId("cockpit-trust-register-button"));
    expect(actions.registerTrustEntry).toHaveBeenCalledWith("kimi_code");

    // 冗长细节（原拒绝串/长指导）只出现在「查看诊断详情」折叠内。
    const fold = screen.getByTestId("cockpit-inbox-error-details");
    expect(fold).toHaveTextContent("查看诊断详情");
    expect(fold.textContent).toContain("原拒绝：spawn denied");
    // 主卡面不再平铺长诊断。
    expect(screen.getByTestId("cockpit-trust-missing-headline").textContent).not.toContain(
      "人工登记指导",
    );
  });

  it("多 provider 时标题按实际并列（Kimi 与 Codex）", () => {
    render(
      <CockpitInbox
        items={[
          {
            ...structuredTrustItem,
            trustMissing: {
              provider: "kimi_code",
              providers: ["kimi_code", "codex"],
              canonicalRoot: "/agg/root",
              commandMarkdown: trustCommand,
              retryAction: "人工登记后显式重新生成",
            },
          },
        ]}
        actions={mockActions()}
        actionableSessionId="session_001"
      />,
    );
    expect(screen.getByTestId("cockpit-trust-missing-headline")).toHaveTextContent(
      "该目录（/agg/root）尚未被 Kimi 与 Codex 信任",
    );
  });

  it("无结构化命令（旧链文本识别/冲突分型）不渲染命令窗，说明与登记按钮仍在", () => {
    render(
      <CockpitInbox
        items={[
          {
            ...structuredTrustItem,
            trustMissing: {
              provider: "codex",
              providers: ["codex"],
              canonicalRoot: null,
              commandMarkdown: null,
              retryAction: null,
            },
          },
        ]}
        actions={mockActions()}
        actionableSessionId="session_001"
      />,
    );
    expect(screen.getByTestId("cockpit-trust-missing-headline")).toHaveTextContent(
      "该目录尚未被 Codex 信任",
    );
    expect(screen.getByTestId("cockpit-trust-missing-action")).toBeVisible();
    expect(screen.queryByTestId("cockpit-trust-command-window")).toBeNull();
    expect(screen.getByTestId("cockpit-trust-register-button")).toBeVisible();
  });
});

// ── P4-e（REQ-UI37-19）：coding_action 卡「去处理」下钻 ──
// coding 卡与页面内卡同源（codingAttentionItem 事实投影驱动），收件箱只
// 导航，不在卡上重复渲染表单/确认/终止动作（通知只导航同此口径）。
// 导航经注入的 onOpenAttentionTarget（生产由 router 挂载点提供统一
// opener）；测试以 mock 断言统一导航入参（契约 A1 target/focus）。
import { codingAttentionItem } from "../../../state/workspace-cockpit-projection";
import type { CodingAttentionItemDto } from "../../../api/types/lifecycle";
import type { CodingAttentionLocationItem } from "../../../router";

function attentionDto(overrides: Partial<CodingAttentionItemDto> = {}): CodingAttentionItemDto {
  return {
    key: "gate:attempt_a:gate_7",
    kind: "stalled",
    project_id: "project_0002",
    issue_id: "issue_0002",
    plan_id: null,
    attempt_id: "coding_attempt_0001",
    logical_work_item_id: null,
    repository_name: "repo-b",
    reason_code: "coder_provider_stalled",
    title: "AI 挂起了",
    summary: "Coder 长时间没有输出",
    occurred_at: "2026-10-16T00:00:00Z",
    target: { type: "coding_attempt", attempt_id: "coding_attempt_0001" },
    focus: { gate_id: "gate_7" },
    ...overrides,
  };
}

describe("CockpitInbox coding_action 卡「去处理」（P4-e/REQ-UI37-19）", () => {
  it("coding_action 卡渲染标题/摘要并经「去处理」以统一 target/focus 导航（gateId）", async () => {
    const item = codingAttentionItem(attentionDto());
    const onOpenAttentionTarget = vi.fn(() => true);
    render(
      <CockpitInbox items={[item]} onOpenAttentionTarget={onOpenAttentionTarget} />,
    );

    const row = screen.getByTestId("cockpit-inbox-item-coding_action");
    expect(row).toHaveTextContent("AI 挂起了");
    expect(row).toHaveTextContent("Coder 长时间没有输出 · repo-b");

    await userEvent.click(within(row).getByRole("button", { name: "去处理" }));
    expect(onOpenAttentionTarget).toHaveBeenCalledWith({
      kind: "stalled",
      project_id: "project_0002",
      issue_id: "issue_0002",
      plan_id: undefined,
      attempt_id: "coding_attempt_0001",
      logical_work_item_id: undefined,
      target: { type: "coding_attempt", attempt_id: "coding_attempt_0001" },
      focus: { gate_id: "gate_7" },
    } satisfies CodingAttentionLocationItem);
  });

  it("final_confirm 事项导航带 focus=final_confirm 语义（kind 透传）；plan/workspace 目标各到对应路由", async () => {
    const finalConfirmItem = codingAttentionItem(
      attentionDto({
        key: "final:attempt_a:node_1",
        kind: "final_confirm",
        reason_code: null,
        focus: null,
      }),
    );
    const firstOpener = vi.fn(() => true);
    render(
      <CockpitInbox
        items={[finalConfirmItem]}
        onOpenAttentionTarget={firstOpener}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "去处理" }));
    expect(firstOpener).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "final_confirm", focus: undefined }),
    );
    cleanup();
    const planItem = codingAttentionItem(
      attentionDto({
        key: "plan-final:plan_9:run_1",
        kind: "plan_final_confirm",
        plan_id: "plan_9",
        attempt_id: null,
        target: { type: "coding_plan", plan_id: "plan_9" },
        focus: null,
      }),
    );
    const planOpener = vi.fn(() => true);
    const second = render(
      <CockpitInbox items={[planItem]} onOpenAttentionTarget={planOpener} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "去处理" }));
    expect(planOpener).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "plan_final_confirm",
        plan_id: "plan_9",
        target: { type: "coding_plan", plan_id: "plan_9" },
      }),
    );
    second.unmount();

    const sessionItem = codingAttentionItem(
      attentionDto({
        key: "recovery:attempt_a:run_2",
        kind: "manual_recovery",
        attempt_id: null,
        target: { type: "workspace_session", session_id: "ws_7" },
        focus: null,
      }),
    );
    const sessionOpener = vi.fn(() => true);
    render(
      <CockpitInbox items={[sessionItem]} onOpenAttentionTarget={sessionOpener} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "去处理" }));
    expect(sessionOpener).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "manual_recovery",
        target: { type: "workspace_session", session_id: "ws_7" },
      }),
    );
  });

  it("coding_action 卡不重复渲染表单：与页面内卡同源，只有「去处理」一个按钮", () => {
    const item = codingAttentionItem(attentionDto());
    render(
      <CockpitInbox
        items={[item]}
        actions={actions}
        actionableSessionId="session_001"
        onOpenAttentionTarget={vi.fn(() => true)}
      />,
    );

    const row = screen.getByTestId("cockpit-inbox-item-coding_action");
    expect(within(row).getAllByRole("button")).toHaveLength(1);
    expect(within(row).getByRole("button", { name: "去处理" })).toBeInTheDocument();
    expect(within(row).queryByRole("checkbox")).toBeNull();
  });

  it("导航未接线（无 shell context/显式 prop）时不渲染「去处理」，平台消息卡仍在", () => {
    const item = codingAttentionItem(attentionDto());
    render(<CockpitInbox items={[item]} />);

    const row = screen.getByTestId("cockpit-inbox-item-coding_action");
    expect(row).toHaveTextContent("AI 挂起了");
    expect(within(row).queryByRole("button", { name: "去处理" })).toBeNull();
  });
});
