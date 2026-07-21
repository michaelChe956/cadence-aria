import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { HumanGroupProjection } from "../../api/types";
import { WorkItemPlanOverview } from "./WorkItemPlanOverview";

describe("WorkItemPlanOverview", () => {
  it("shows human projection without exposing coder prompt by default", () => {
    render(
      <WorkItemPlanOverview
        projection={planProjectionFixture()}
        presentation={null}
      />,
    );

    expect(screen.getByText("仓库初始化实时进度")).toBeInTheDocument();
    expect(screen.getByText("WI-01 初始化领域模型")).toBeInTheDocument();
    expect(screen.getByText("提供 finalization contract")).toBeInTheDocument();
    expect(screen.queryByText("Coder 执行协议")).not.toBeInTheDocument();
  });
});

function planProjectionFixture(): HumanGroupProjection {
  return {
    plan_id: "plan-001",
    goal: "仓库初始化实时进度",
    split_reason: "将领域模型与最终提交拆分，避免写入范围重叠。",
    work_items: [
      {
        logical_work_item_id: "WI-01",
        title: "初始化领域模型",
        goal: "建立仓库初始化状态模型",
        depends_on: [],
        provides: ["提供 finalization contract"],
        scope_summary: {
          owned_scopes: ["src/product/repository_init.rs"],
          forbidden_scopes: ["web/src"],
        },
      },
    ],
    contract_flow: [],
    risks: ["初始化失败时必须保留可诊断信息"],
    source_refs: ["story:repository-init", "design:repository-init-progress"],
    normative: false,
    used_by_provider: false,
  };
}
