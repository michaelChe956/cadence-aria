// web/src/components/chat-workspace/cockpit/PlanRepairReadOnlyPanel.test.tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { PlanRepairPanelModel } from "../../../state/plan-approval-projection";
import { PlanRepairReadOnlyPanel } from "./PlanRepairReadOnlyPanel";

const model: PlanRepairPanelModel = {
  visible: true,
  stageLabel: "等待最终确认",
  requestStatus: "awaiting_confirmation",
  defectClass: "upstream_contract_invalid",
  reasonCode: "CONTRACT_CAPABILITY_MISSING",
  triggerFindingId: "finding_0001",
  amendment: {
    id: "amend_0001",
    previousPlanRevisionId: "plan_rev_0001",
    newPlanRevisionId: "plan_rev_0002",
    revisedWorkItemCount: 1,
    contractDeltaCount: 2,
    supersededCount: 1,
    dependencyGraphChangeCount: 0,
    resumeTarget: "wi_backend · revalidate",
  },
  rounds: [
    {
      nodeId: "timeline_node_001",
      title: "修订编写",
      summary: "已产出修订",
      status: "completed",
      startedAt: "2026-09-14T08:10:00Z",
      completedAt: "2026-09-14T08:20:00Z",
    },
    {
      nodeId: "timeline_node_002",
      title: "Plan Review",
      summary: null,
      status: "active",
      startedAt: "2026-09-14T08:20:00Z",
      completedAt: null,
    },
  ],
  feedbackTurn: {
    turnId: "turn_0001",
    commandId: "cmd_0001",
    status: "open",
    remainingBudget: 1,
    openedAt: "2026-09-14T08:30:00Z",
  },
  error: null,
};

describe("PlanRepairReadOnlyPanel", () => {
  it("renders request metadata, stage, rounds, feedback turn and amendment summary", () => {
    render(<PlanRepairReadOnlyPanel model={model} />);
    const view = screen.getByTestId("plan-repair-read-only-panel");
    expect(view).toHaveTextContent("等待最终确认");
    expect(view).toHaveTextContent("upstream_contract_invalid");
    expect(view).toHaveTextContent("CONTRACT_CAPABILITY_MISSING");
    expect(view).toHaveTextContent("finding_0001");
    expect(view).toHaveTextContent("修订编写");
    expect(view).toHaveTextContent("Plan Review");
    expect(view).toHaveTextContent("turn_0001");
    expect(view).toHaveTextContent("剩余修复轮次 1");
    expect(view).toHaveTextContent("amend_0001");
    expect(view).toHaveTextContent("plan_rev_0001 → plan_rev_0002");
    expect(view).toHaveTextContent("修订工作项 1");
    expect(view).toHaveTextContent("契约变更 2");
    expect(view).toHaveTextContent("被替代修订 1");
    expect(view).toHaveTextContent("恢复目标");
    expect(view).toHaveTextContent("wi_backend · revalidate");
  });

  it("renders a dash for an absent resume target", () => {
    render(<PlanRepairReadOnlyPanel model={{
      ...model,
      amendment: { ...model.amendment!, resumeTarget: null },
    }} />);
    expect(screen.getByText("恢复目标").nextElementSibling).toHaveTextContent("—");
  });

  it("exposes no write affordances of any kind (DEF-3 只读边界)", () => {
    const { container } = render(<PlanRepairReadOnlyPanel model={model} />);
    expect(
      container.querySelectorAll("button, input, textarea, select, form, a[href]"),
    ).toHaveLength(0);
  });

  it("omits absent sections instead of rendering placeholders", () => {
    render(
      <PlanRepairReadOnlyPanel
        model={{
          ...model,
          amendment: null,
          feedbackTurn: null,
          rounds: [],
        }}
      />,
    );
    const view = screen.getByTestId("plan-repair-read-only-panel");
    expect(view).not.toHaveTextContent("amend_");
    expect(view).not.toHaveTextContent("turn_");
    expect(screen.getByTestId("plan-repair-rounds-empty")).toHaveTextContent(
      "本连接尚未观测到子会话轮次",
    );
  });

  it("surfaces the snapshot error verbatim", () => {
    render(<PlanRepairReadOnlyPanel model={{ ...model, error: "resume 冲突" }} />);
    expect(screen.getByRole("alert")).toHaveTextContent("resume 冲突");
  });
});
