import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { PlanGroupProjectionDto } from "../../api/types";
import { PlanGroupProjectionPanel } from "./PlanGroupProjectionPanel";

const deliveredEntry = {
  target_repository_id: "11111111-1111-1111-1111-111111111111",
  repository_name: "checkout_repo_alpha",
  attempt_id: "coding_attempt_0001",
  attempt_status: "completed",
  stage: "final_confirm",
  branch_name: "aria/issues/issue_0001/checkout_repo_alpha",
  head_commit: "sha1111111111111111111111111111111111111",
  push_status: "pushed",
  review_request_id: "review_request_0001",
  blocked_reason: null,
};

const failedEntry = {
  target_repository_id: "22222222-2222-2222-2222-222222222222",
  repository_name: "checkout_repo_beta",
  attempt_id: "coding_attempt_0002",
  attempt_status: "completed",
  stage: "review_request",
  branch_name: "aria/issues/issue_0001/checkout_repo_beta",
  head_commit: "sha2222222222222222222222222222222222222",
  push_status: "failed",
  review_request_id: "review_request_0002",
  blocked_reason: "push rejected",
};

const allDelivered: PlanGroupProjectionDto = {
  plan_id: "work_item_plan_0001",
  overall: "all_delivered",
  entries: [
    deliveredEntry,
    { ...deliveredEntry, target_repository_id: "33333333-3333-3333-3333-333333333333" },
  ],
};

const partial: PlanGroupProjectionDto = {
  plan_id: "work_item_plan_0001",
  overall: "partial",
  entries: [deliveredEntry, failedEntry],
};

const notStarted: PlanGroupProjectionDto = {
  plan_id: "work_item_plan_0001",
  overall: "not_started",
  entries: [
    {
      target_repository_id: "44444444-4444-4444-4444-444444444444",
      repository_name: "checkout_repo_alpha",
      attempt_id: "coding_attempt_0003",
      attempt_status: "created",
      stage: "prepare_context",
      branch_name: "aria/issues/issue_0001/checkout_repo_alpha",
      head_commit: null,
      push_status: null,
      review_request_id: null,
      blocked_reason: null,
    },
    {
      target_repository_id: "55555555-5555-5555-5555-555555555555",
      repository_name: "checkout_repo_beta",
      attempt_id: "coding_attempt_0004",
      attempt_status: "created",
      stage: "prepare_context",
      branch_name: "aria/issues/issue_0001/checkout_repo_beta",
      head_commit: null,
      push_status: null,
      review_request_id: null,
      blocked_reason: null,
    },
  ],
};

describe("PlanGroupProjectionPanel", () => {
  it("renders all_delivered with green badge and per-target delivered rows", () => {
    render(<PlanGroupProjectionPanel projection={allDelivered} />);

    expect(screen.getByTestId("plan-group-overall-badge")).toHaveAttribute(
      "data-status",
      "all_delivered",
    );
    expect(screen.getByText("已全部交付")).toBeInTheDocument();

    const rows = screen.getAllByTestId("plan-target-entry-row");
    expect(rows).toHaveLength(2);
    for (const row of rows) {
      expect(row).toHaveAttribute("data-status", "delivered");
      expect(row).toHaveTextContent("已交付");
    }
    expect(screen.getAllByText("checkout_repo_alpha")).toHaveLength(2);
    expect(screen.queryAllByTestId("plan-target-unmet-reason")).toHaveLength(0);
  });

  it("renders partial failure explicitly without disguising global success", () => {
    render(<PlanGroupProjectionPanel projection={partial} />);

    expect(screen.getByTestId("plan-group-overall-badge")).toHaveAttribute(
      "data-status",
      "partial",
    );
    expect(screen.getByText("部分交付")).toBeInTheDocument();
    expect(screen.queryByText("已全部交付")).not.toBeInTheDocument();

    const rows = screen.getAllByTestId("plan-target-entry-row");
    expect(rows).toHaveLength(2);
    const failedRow = rows.find((row) => row.getAttribute("data-status") === "failed");
    expect(failedRow).toBeDefined();
    expect(failedRow).toHaveTextContent("checkout_repo_beta");
    expect(failedRow).toHaveTextContent("推送失败");

    // 未满足项与原因显式呈现（blocked_reason 优先）。
    const unmet = screen.getByTestId("plan-target-unmet-reason");
    expect(unmet).toHaveTextContent("push rejected");

    // 已交付 target 不渲染未满足行。
    expect(screen.getAllByTestId("plan-target-unmet-reason")).toHaveLength(1);
  });

  it("renders per-target facts: stage, push state, branch, head commit", () => {
    render(<PlanGroupProjectionPanel projection={partial} />);

    expect(screen.getByText("aria/issues/issue_0001/checkout_repo_beta")).toBeInTheDocument();
    // head commit 取前 7 位。
    expect(screen.getByText("sha2222")).toBeInTheDocument();
    // stage 与推送状态用中文标签呈现。
    expect(screen.getByText(/最终确认/)).toBeInTheDocument();
    expect(screen.getByText(/评审请求/)).toBeInTheDocument();
    expect(screen.getByText("已推送")).toBeInTheDocument();
  });

  it("renders not_started with neutral badge and initial-binary-group entries", () => {
    render(<PlanGroupProjectionPanel projection={notStarted} />);

    expect(screen.getByTestId("plan-group-overall-badge")).toHaveAttribute(
      "data-status",
      "not_started",
    );
    expect(screen.getByText("未启动")).toBeInTheDocument();

    const rows = screen.getAllByTestId("plan-target-entry-row");
    expect(rows).toHaveLength(2);
    for (const row of rows) {
      expect(row).toHaveAttribute("data-status", "pending");
      expect(row).toHaveTextContent("已创建");
      expect(row).toHaveTextContent("准备上下文");
      expect(row).toHaveTextContent("无评审请求");
      expect(row).toHaveTextContent("未交付");
    }
  });

  it("renders empty hint when not_started plan has no target attempts", () => {
    render(
      <PlanGroupProjectionPanel
        projection={{ plan_id: "work_item_plan_0001", overall: "not_started", entries: [] }}
      />,
    );

    expect(screen.getByTestId("plan-group-empty-hint")).toHaveTextContent(
      "尚无 target-attempt",
    );
    expect(screen.queryAllByTestId("plan-target-entry-row")).toHaveLength(0);
  });

  it("carries no action elements in the aggregate view", () => {
    const { container } = render(<PlanGroupProjectionPanel projection={partial} />);

    // 聚合面无任何操作入口（REQ-MTG-04：决策/启动/中止走各 attempt 既有入口）。
    expect(container.querySelectorAll("button")).toHaveLength(0);
    expect(container.querySelectorAll('[role="button"]')).toHaveLength(0);
    expect(container.querySelectorAll("a[href]")).toHaveLength(0);
  });
});
