import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ChatEntry } from "../../../state/chat-entries";
import type { CockpitActionFacade } from "../../../state/cockpit-action-routing";
import { GatePromptEntry } from "./GatePromptEntry";

function gateEntry(actionBlockReason: "terminal_stage" | "phase_mismatch" | null): ChatEntry {
  return {
    id: "gate:entry",
    type: "gate_prompt",
    role: "system",
    content: "等待人工确认",
    timestamp: "2026-09-17T00:00:00Z",
    metadata: {
      action_facade: "typed",
      command_id: "cmd_1",
      action_block_reason: actionBlockReason,
    },
  };
}

function actions(): CockpitActionFacade {
  return {
    confirm: vi.fn(() => true),
    requestChange: vi.fn(() => true),
    feedback: vi.fn(() => true),
    terminate: vi.fn(() => true),
    advance: vi.fn(() => true),
  };
}

describe("GatePromptEntry actionability", () => {
  it("replaces every gate action with its block reason when the projection is stale", () => {
    const gateActions = actions();

    render(<GatePromptEntry entry={gateEntry("terminal_stage")} actions={gateActions} />);

    expect(screen.getByText("已离开人工确认门")).toBeVisible();
    expect(screen.queryByTestId("gate-feedback-editor")).toBeNull();
    expect(screen.queryByRole("button", { name: "确认产物" })).toBeNull();
    expect(screen.queryByRole("button", { name: "终止" })).toBeNull();
    expect(gateActions.confirm).not.toHaveBeenCalled();
    expect(gateActions.feedback).not.toHaveBeenCalled();
    expect(gateActions.terminate).not.toHaveBeenCalled();
  });

  it("keeps typed gate controls wired to the supplied facade when no block reason exists", async () => {
    const gateActions = actions();
    const user = userEvent.setup();

    render(<GatePromptEntry entry={gateEntry(null)} actions={gateActions} />);

    await user.type(screen.getByLabelText("门禁反馈"), "请补齐边界");
    await user.click(screen.getByRole("button", { name: "提交反馈" }));
    fireEvent.click(screen.getByRole("button", { name: "确认产物" }));
    fireEvent.click(screen.getByRole("button", { name: "终止" }));
    fireEvent.click(screen.getByRole("button", { name: "确认终止" }));

    expect(gateActions.feedback).toHaveBeenCalledWith("请补齐边界");
    expect(gateActions.confirm).toHaveBeenCalledOnce();
    expect(gateActions.terminate).toHaveBeenCalledOnce();
  });
});
