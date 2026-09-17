import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import type { CockpitInboxItem } from "../../../state/workspace-cockpit-projection";
import { CockpitInbox } from "./CockpitInbox";

const actions: CockpitActionFacade = {
  confirm: vi.fn(),
  requestChange: vi.fn(),
  feedback: vi.fn(),
  terminate: vi.fn(),
  advance: vi.fn(),
};

const gateItem: CockpitInboxItem = {
  id: "session_001:gate:gate_001",
  kind: "gate",
  severity: 2,
  title: "门禁等待",
  summary: "等待人工确认",
  triage: false,
  source: "gate",
  createdAt: null,
  gate: {
    key: "gate_001",
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
            gate: { ...gateItem.gate!, action_block_reason: "terminal_stage" },
          },
        ]}
        actions={actions}
        actionableSessionId="session_001"
        onBulkConfirm={vi.fn()}
      />,
    );

    const inbox = screen.getByTestId("cockpit-inbox");
    expect(within(inbox).getByText("已离开人工确认门")).toBeVisible();
    expect(within(inbox).queryByLabelText("选择 门禁等待")).toBeNull();
    expect(within(inbox).queryByTestId("gate-feedback-editor")).toBeNull();
    expect(within(inbox).queryByRole("button", { name: "确认" })).toBeNull();
    expect(within(inbox).queryByRole("button", { name: "终止" })).toBeNull();
  });

  it("selects the current session's projected gate and batch confirms it once", async () => {
    const user = userEvent.setup();
    const onBulkConfirm = vi.fn();
    const current: CockpitInboxItem = {
      ...gateItem,
      id: "s1:gate:snapshot:2026-09-15T00:00:00Z:fp",
      gate: {
        ...gateItem.gate!,
        key: "snapshot:2026-09-15T00:00:00Z:fp",
      },
    };
    const observed: CockpitInboxItem = {
      ...gateItem,
      id: "s2:gate:g2",
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

    await user.click(screen.getByLabelText("选择 门禁等待"));
    expect(screen.queryByLabelText("选择 stop")).toBeNull();
    expect(screen.queryByLabelText("选择 error")).toBeNull();
    expect(screen.queryByLabelText("选择 g2")).toBeNull();
    await user.click(screen.getByRole("button", { name: "批量确认 1 项" }));

    expect(onBulkConfirm).toHaveBeenCalledWith([current]);
    expect(screen.queryByRole("button", { name: "批量确认 1 项" })).toBeNull();
  });

  it("renders a custom empty hint when provided", () => {
    render(<CockpitInbox items={[]} actions={actions} emptyHint="会话尚未开始" />);

    expect(screen.getByText("会话尚未开始")).toBeVisible();
  });

  it("keeps the default empty text without a hint", () => {
    render(<CockpitInbox items={[]} actions={actions} />);

    expect(screen.getByText("暂无待处理项")).toBeVisible();
  });
});
