import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type {
  PlanProjectionBundle,
  WorkItemProjectionBundle,
  WorkItemRevisionHistoryDto,
} from "../../api/types";
import { WorkItemProjectionTabs } from "./WorkItemProjectionTabs";

describe("WorkItemProjectionTabs", () => {
  it("renders Human Overview by default and mounts Coder and Reviewer only after explicit selection", async () => {
    const user = userEvent.setup();
    renderProjectionTabs();

    const tablist = screen.getByRole("tablist", {
      name: "Work Item Plan projections",
    });
    expect(tablist).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Human Overview" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    const controlledPanelId = screen
      .getByRole("tab", { name: "Human Overview" })
      .getAttribute("aria-controls");
    expect(controlledPanelId).not.toBeNull();
    expect(document.getElementById(controlledPanelId ?? "")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Coder" })).not.toHaveAttribute(
      "aria-controls",
    );
    expect(screen.getByText("仓库初始化实时进度")).toBeInTheDocument();
    expect(screen.queryByText("Coder 执行协议")).not.toBeInTheDocument();
    expect(screen.queryByText("Reviewer 验证矩阵")).not.toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Coder" }));

    expect(screen.getByText("Coder 执行协议")).toBeInTheDocument();
    expect(screen.getByText("初始化状态模型并提交契约")).toBeInTheDocument();
    expect(screen.getByText("Provider rendering")).toBeInTheDocument();
    expect(screen.getAllByText("等待运行时 Envelope（P5）")).toHaveLength(3);
    expect(screen.getByText("Codex")).toBeInTheDocument();
    expect(screen.getByText("Claude Code")).toBeInTheDocument();
    expect(screen.getByText("Fake")).toBeInTheDocument();
    expect(screen.queryByText(/unit_run/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/synthetic/i)).not.toBeInTheDocument();
    expect(screen.queryByText("Reviewer 验证矩阵")).not.toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Reviewer" }));

    expect(screen.getByText("Reviewer 验证矩阵")).toBeInTheDocument();
    expect(screen.getByText("AC-01")).toBeInTheDocument();
    expect(screen.getByText("coder_rework")).toBeInTheDocument();
    expect(screen.getByText("Provider rendering")).toBeInTheDocument();
    expect(screen.getAllByText("等待运行时 Envelope（P5）")).toHaveLength(3);
    expect(screen.queryByText("Coder 执行协议")).not.toBeInTheDocument();
  });

  it("supports arrow-key tab navigation and renders real history artifact refs", async () => {
    const user = userEvent.setup();
    renderProjectionTabs();

    const overviewTab = screen.getByRole("tab", { name: "Human Overview" });
    overviewTab.focus();
    await user.keyboard("{ArrowRight}");

    expect(screen.getByRole("tab", { name: "Contract Flow" })).toHaveFocus();

    await user.click(screen.getByRole("tab", { name: "History" }));

    expect(screen.getByText("draft-revision-001")).toBeInTheDocument();
    expect(screen.getByText("plan-review-001")).toBeInTheDocument();
    expect(screen.getByText("contract-delta-001")).toBeInTheDocument();
  });
});

function renderProjectionTabs() {
  return render(
    <WorkItemProjectionTabs
      planProjection={planProjectionFixture()}
      workItemProjections={[workItemProjectionFixture()]}
      history={historyFixture()}
      validation={{ findings: [] }}
    />,
  );
}

function planProjectionFixture(): PlanProjectionBundle {
  return {
    id: "plan-projection-001",
    plan_revision_id: "plan-revision-001",
    dependency_graph_revision_id: "dependency-graph-001",
    work_item_projection_bundle_refs: ["work-item-projection-001"],
    human_group_projection: {
      plan_id: "plan-001",
      goal: "仓库初始化实时进度",
      split_reason: "按契约边拆分",
      work_items: [
        {
          logical_work_item_id: "WI-01",
          title: "初始化领域模型",
          goal: "建立初始化状态模型",
          depends_on: [],
          provides: ["finalization-contract"],
          scope_summary: {
            owned_scopes: ["src/product/repository_init.rs"],
            forbidden_scopes: ["web/src"],
          },
        },
      ],
      contract_flow: [],
      risks: [],
      source_refs: ["story:repository-init"],
      normative: false,
      used_by_provider: false,
    },
    coder_group_context: {
      plan_id: "plan-001",
      ordered_logical_work_item_ids: ["WI-01"],
      dependency_edges: [],
      group_write_scopes: {
        "WI-01": {
          exclusive_scopes: ["src/product/repository_init.rs"],
          forbidden_scopes: ["web/src"],
        },
      },
    },
    reviewer_group_matrix: {
      plan_id: "plan-001",
      work_items: [
        {
          logical_work_item_id: "WI-01",
          criterion_refs: ["AC-01"],
          input_contract_refs: [],
          output_contract_refs: ["finalization-contract"],
        },
      ],
      dependency_edges: [],
      design_traceability_refs: [],
    },
    human_group_projection_hash: "human-hash",
    coder_group_context_hash: "coder-hash",
    reviewer_group_matrix_hash: "reviewer-hash",
    compiler_version: "projection-compiler-v1",
    created_at: "2026-07-18T10:00:00Z",
  };
}

function workItemProjectionFixture(): WorkItemProjectionBundle {
  return {
    id: "work-item-projection-001",
    work_item_revision_id: "work-item-revision-001",
    canonical_contract_hash: "contract-hash",
    projection_schema_version: 1,
    compiler_version: "projection-compiler-v1",
    human_projection: {
      logical_work_item_id: "WI-01",
      title: "初始化领域模型",
      goal: "建立初始化状态模型",
      non_goals: [],
      inputs: [],
      outputs: [
        {
          contract_id: "finalization-contract",
          capabilities: ["initialized_state"],
          source_refs: ["design:repository-init-progress"],
        },
      ],
      dependencies: [],
      scope_summary: {
        owned_scopes: ["src/product/repository_init.rs"],
        forbidden_scopes: ["web/src"],
      },
      completion_summary: ["初始化状态可被最终提交阶段消费"],
      source_refs: ["story:repository-init"],
      normative: false,
      used_by_provider: false,
    },
    coder_projection: {
      work_item_revision_id: "work-item-revision-001",
      objective: "初始化状态模型并提交契约",
      required_input_contracts: [],
      task_refs: ["TASK-01"],
      tasks: [
        {
          task_id: "TASK-01",
          statement: "实现初始化状态模型",
          requirement_refs: ["REQ-01"],
          done_when_refs: ["AC-01"],
        },
      ],
      write_policy: {
        exclusive_scopes: ["src/product/repository_init.rs"],
        forbidden_scopes: ["web/src"],
      },
      acceptance_criteria: [
        {
          criterion_id: "AC-01",
          statement: "初始化状态包含成功与失败信息",
          required_evidence: ["source_diff", "non_zero_test_execution"],
        },
      ],
      verification_checks: [],
      blocker_rules: [],
      handoff_contract: {
        required_fields: ["commit_sha", "tests"],
        provided_contract_refs: ["finalization-contract"],
        reviewer_check_refs: ["AC-01"],
      },
    },
    reviewer_projection: {
      work_item_revision_id: "work-item-revision-001",
      criterion_refs: ["AC-01"],
      requirement_matrix: [
        {
          criterion_id: "AC-01",
          requirement_refs: ["REQ-01"],
          required_evidence: ["source_diff", "non_zero_test_execution"],
          failure_route: "coder_rework",
        },
      ],
      scope_policy: {
        exclusive_scopes: ["src/product/repository_init.rs"],
        forbidden_scopes: ["web/src"],
      },
      input_contract_checks: [],
      output_contract_checks: [
        {
          contract_id: "finalization-contract",
          capabilities: ["initialized_state"],
        },
      ],
      verification_evidence_rules: [],
      blocker_routing: [],
    },
    human_projection_hash: "human-work-item-hash",
    coder_projection_hash: "coder-work-item-hash",
    reviewer_projection_hash: "reviewer-work-item-hash",
    created_at: "2026-07-18T10:00:00Z",
  };
}

function historyFixture(): WorkItemRevisionHistoryDto {
  return {
    entries: [
      {
        kind: "draft_revision",
        id: "draft-revision-001",
        logical_work_item_id: "WI-01",
        related_revision_id: null,
        summary: "建立初始 canonical contract",
        created_at: "2026-07-18T09:00:00Z",
      },
      {
        kind: "plan_review",
        id: "plan-review-001",
        logical_work_item_id: "WI-01",
        related_revision_id: "draft-revision-001",
        summary: "Reviewer 接受契约边",
        created_at: "2026-07-18T09:05:00Z",
      },
      {
        kind: "contract_delta",
        id: "contract-delta-001",
        logical_work_item_id: "WI-01",
        related_revision_id: "work-item-revision-001",
        summary: "新增 failure_message capability",
        created_at: "2026-07-18T09:10:00Z",
      },
    ],
  };
}
