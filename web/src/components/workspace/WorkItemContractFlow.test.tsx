import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { HumanGroupProjection } from "../../api/types";
import { WorkItemContractFlow } from "./WorkItemContractFlow";

describe("WorkItemContractFlow", () => {
  it("shows required, provided, and missing capabilities on the contract edge", () => {
    render(<WorkItemContractFlow projection={contractMismatchFixture()} />);

    expect(screen.getByText("WI-01 → WI-02")).toBeInTheDocument();
    expect(screen.getByText("finalization-contract")).toBeInTheDocument();
    expect(screen.getByText("需要 initialized_state")).toBeInTheDocument();
    expect(screen.getByText("已提供 initialized_state")).toBeInTheDocument();
    expect(screen.getByText("缺少 failure_message")).toBeInTheDocument();
  });
});

function contractMismatchFixture(): HumanGroupProjection {
  return {
    plan_id: "plan-001",
    goal: "仓库初始化实时进度",
    split_reason: "按依赖契约拆分",
    work_items: [],
    contract_flow: [
      {
        from: "WI-01",
        to: "WI-02",
        contract_id: "finalization-contract",
        required_capabilities: ["initialized_state", "failure_message"],
        provided_capabilities: ["initialized_state"],
        missing_capabilities: ["failure_message"],
      },
    ],
    risks: [],
    source_refs: ["design:repository-init-progress"],
    normative: false,
    used_by_provider: false,
  };
}
