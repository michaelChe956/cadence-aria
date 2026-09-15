// web/src/components/chat-workspace/cockpit/ContractChecklistView.test.tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ContractChecklist } from "../../../state/plan-approval-projection";
import { ContractChecklistView } from "./ContractChecklistView";

const checklist: ContractChecklist = {
  hasData: true,
  gapCount: 1,
  rows: [
    {
      key: "contract_metrics::cap_export_csv",
      contractId: "contract_metrics",
      from: "wi_backend",
      to: "wi_frontend",
      capability: "cap_export_csv",
      satisfied: true,
      matchedFindings: [],
    },
    {
      key: "contract_metrics::cap_export_json",
      contractId: "contract_metrics",
      from: "wi_backend",
      to: "wi_frontend",
      capability: "cap_export_json",
      satisfied: false,
      matchedFindings: [
        {
          code: "CONTRACT_CAPABILITY_MISSING",
          severity: "Error",
          contractRef: "contract_metrics",
          capabilityRef: "cap_export_json",
          message: "cap_export_json 未由上游提供",
        },
      ],
    },
  ],
};

function renderView(
  targets: Record<string, string | null> = { contract_metrics: "gate_new" },
) {
  const onJumpToFinding = vi.fn();
  render(
    <ContractChecklistView
      checklist={checklist}
      findingTargetFor={(row) => targets[row.contractId] ?? null}
      onJumpToFinding={onJumpToFinding}
    />,
  );
  return { onJumpToFinding };
}

describe("ContractChecklistView", () => {
  it("renders one row per capability with satisfied and gap badges", () => {
    renderView();
    const view = screen.getByTestId("contract-checklist-view");
    expect(view).toHaveTextContent("contract_metrics");
    expect(view).toHaveTextContent("wi_backend → wi_frontend");
    expect(view).toHaveTextContent("cap_export_csv");
    expect(view).toHaveTextContent("cap_export_json");
    expect(screen.getByTestId("checklist-row-satisfied")).toHaveTextContent(
      "满足",
    );
    expect(screen.getByTestId("checklist-row-gap")).toHaveTextContent("缺口");
    expect(screen.getByTestId("contract-checklist-gap-count")).toHaveTextContent(
      "缺口 1 条 / 共 2 条",
    );
  });

  it("shows required vs provided columns per row", () => {
    renderView();
    expect(screen.getAllByText("Required").length).toBe(2);
    expect(screen.getAllByText("Provided").length).toBe(2);
    expect(screen.getByText("已由 wi_backend 提供"));
    expect(screen.getByText("上游 wi_backend 未提供"));
  });

  it("jumps to the finding entry when the target resolves", async () => {
    const user = userEvent.setup();
    const { onJumpToFinding } = renderView();
    await user.click(screen.getByRole("button", { name: "查看 finding" }));
    expect(onJumpToFinding).toHaveBeenCalledWith("gate_new");
  });

  it("explains when no finding matches a gap instead of offering a dead jump", () => {
    renderView({ contract_metrics: null });
    expect(screen.getByTestId("contract-checklist-view")).toHaveTextContent(
      "无关联 finding（以门禁 finding 为准）",
    );
    expect(screen.queryByRole("button", { name: "查看 finding" })).toBeNull();
  });

  it("uses the gate-finding fallback copy when no target exists", () => {
    renderView({ contract_metrics: null });
    expect(screen.getByTestId("contract-checklist-view")).toHaveTextContent(
      "无关联 finding（以门禁 finding 为准）",
    );
  });

  it("renders the empty state when the plan has no contract entries", () => {
    render(
      <ContractChecklistView
        checklist={{ rows: [], gapCount: 0, hasData: false }}
        findingTargetFor={() => null}
        onJumpToFinding={vi.fn()}
      />,
    );
    expect(screen.getByTestId("contract-checklist-view")).toHaveTextContent(
      "当前计划没有 capability / 契约条目",
    );
  });
});
