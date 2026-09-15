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
  id: "session_001:gate",
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
  },
  inlineError: null,
};

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
});
