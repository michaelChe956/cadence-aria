import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { PlanRepairTimelineGroup } from "./PlanRepairTimelineGroup";
import { repairAwaitingConfirmationFixture } from "./plan-repair-test-fixtures";

describe("PlanRepairTimelineGroup", () => {
  it("groups only visible child session projections and preserves node selection", async () => {
    const state = repairAwaitingConfirmationFixture();
    const onSelectNode = vi.fn();
    render(
      <PlanRepairTimelineGroup
        nodes={state.timelineNodes}
        activeNodeId="plan_repair_node_confirm_0001"
        selectedNodeId={null}
        onSelectNode={onSelectNode}
      />,
    );

    expect(screen.getByRole("group", { name: "Plan Repair Timeline" })).toHaveTextContent(
      "修订 Work Item Contract",
    );
    expect(screen.getByText("新增 failure_message")).toBeInTheDocument();
    expect(screen.queryByText("provider stream chunk")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /确认 Plan 修订/ }));
    expect(onSelectNode).toHaveBeenCalledWith("plan_repair_node_confirm_0001");
  });
});
