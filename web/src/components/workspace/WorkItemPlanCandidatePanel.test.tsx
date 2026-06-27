import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkItemPlanCandidateDto } from "../../api/types";
import { WorkItemPlanCandidatePanel } from "./WorkItemPlanCandidatePanel";

function makeCandidate(overrides: Partial<WorkItemPlanCandidateDto> = {}): WorkItemPlanCandidateDto {
  return {
    plan: {
      plan_id: "plan_001",
      project_id: "project_001",
      issue_id: "issue_001",
      title: "Plan 001",
      source_story_spec_ids: [],
      source_design_spec_ids: [],
      options: {
        include_integration_tests: false,
        include_e2e_tests: false,
        force_frontend_backend_split: false,
        require_execution_plan_confirm: false,
      },
      status: "draft",
      work_item_ids: [],
      repository_profile_ref: null,
      verification_plan_ids: [],
      dependency_graph: [
        {
          from_work_item_id: "wi_001",
          to_work_item_id: "wi_002",
          dependency_type: "blocks",
        },
      ],
      created_from_provider_run: null,
      validator_findings: [],
      review_summary: null,
      created_at: "2026-06-17T00:00:00Z",
      updated_at: "2026-06-17T00:00:00Z",
    },
    work_items: [
      {
        candidate_id: "wi_001",
        title: "Frontend Auth",
        kind: "frontend",
        exclusive_write_scopes: ["src/auth"],
        depends_on: [],
        verification_plan_ref: "vp_001",
        meta: { summary: "前端登录" },
        suggested_order: 1,
      },
      {
        candidate_id: "wi_002",
        title: "Backend API",
        kind: "backend",
        exclusive_write_scopes: ["src/api"],
        depends_on: ["wi_001"],
        verification_plan_ref: null,
        meta: { summary: "后端接口" },
        suggested_order: 2,
      },
    ],
    verification_plans: [],
    repository_profile: {
      profile_id: "profile_001",
      repository_id: "repo_001",
      confidence: "high",
      detected_layers: ["frontend", "backend"],
      split_recommendation: "frontend-backend",
    },
    validator_findings: [
      {
        finding_id: "f_001",
        level: "warning",
        code: "SCOPE_OVERLAP",
        message: "范围可能重叠",
        affected_scopes: ["src/auth"],
      },
      {
        finding_id: "f_002",
        level: "error",
        message: "缺少依赖声明",
        affected_scopes: [],
      },
    ],
    ...overrides,
  };
}

describe("WorkItemPlanCandidatePanel", () => {
  const onRevert = vi.fn();
  const onRequestRevision = vi.fn();
  const onAccept = vi.fn();

  beforeEach(() => {
    onRevert.mockClear();
    onRequestRevision.mockClear();
    onAccept.mockClear();
  });

  it("renders title, dependency DAG, work items, repository profile and validator findings", () => {
    render(
      <WorkItemPlanCandidatePanel
        candidate={makeCandidate()}
        stage="author_confirm"
        onRevert={onRevert}
        onRequestRevision={onRequestRevision}
        onAccept={onAccept}
      />,
    );

    expect(screen.getByText("Work Item Plan 候选")).toBeInTheDocument();
    expect(screen.getByTestId("candidate-dependency-dag")).toHaveTextContent("wi_001 → wi_002");
    expect(screen.getByTestId("candidate-work-items")).toHaveTextContent("Frontend Auth");
    expect(screen.getByTestId("candidate-work-items")).toHaveTextContent("Backend API");
    expect(screen.getByTestId("candidate-repository-profile")).toHaveTextContent("high");
    expect(screen.getByTestId("candidate-repository-profile")).toHaveTextContent("frontend");
    expect(screen.getByTestId("candidate-validator-findings")).toHaveTextContent("范围可能重叠");
    expect(screen.getByTestId("candidate-validator-findings")).toHaveTextContent("缺少依赖声明");
  });

  it("shows revert buttons only in author_confirm stage", () => {
    const { rerender } = render(
      <WorkItemPlanCandidatePanel
        candidate={makeCandidate()}
        stage="author_confirm"
        onRevert={onRevert}
        onRequestRevision={onRequestRevision}
        onAccept={onAccept}
      />,
    );

    expect(screen.getByTestId("start-revert-wi_001")).toBeInTheDocument();

    rerender(
      <WorkItemPlanCandidatePanel
        candidate={makeCandidate()}
        stage="running"
        onRevert={onRevert}
        onRequestRevision={onRequestRevision}
        onAccept={onAccept}
      />,
    );

    expect(screen.queryByTestId("start-revert-wi_001")).not.toBeInTheDocument();
  });

  it("submits revert with feedback", async () => {
    render(
      <WorkItemPlanCandidatePanel
        candidate={makeCandidate()}
        stage="author_confirm"
        onRevert={onRevert}
        onRequestRevision={onRequestRevision}
        onAccept={onAccept}
      />,
    );

    await userEvent.click(screen.getByTestId("start-revert-wi_001"));
    await userEvent.type(screen.getByTestId("revert-feedback-input-wi_001"), "范围过大");
    await userEvent.click(screen.getByTestId("submit-revert-wi_001"));

    expect(onRevert).toHaveBeenCalledWith("wi_001", "范围过大", false);
  });

  it("cancels revert input", async () => {
    render(
      <WorkItemPlanCandidatePanel
        candidate={makeCandidate()}
        stage="author_confirm"
        onRevert={onRevert}
        onRequestRevision={onRequestRevision}
        onAccept={onAccept}
      />,
    );

    await userEvent.click(screen.getByTestId("start-revert-wi_001"));
    expect(screen.getByTestId("revert-feedback-input-wi_001")).toBeInTheDocument();

    await userEvent.click(screen.getByTestId("cancel-revert-wi_001"));

    expect(screen.queryByTestId("revert-feedback-input-wi_001")).not.toBeInTheDocument();
    expect(onRevert).not.toHaveBeenCalled();
  });

  it("shows clear revert button for reverted items", async () => {
    render(
      <WorkItemPlanCandidatePanel
        candidate={makeCandidate({
          work_items: [
            {
              candidate_id: "wi_001",
              title: "Frontend Auth",
              kind: "frontend",
              exclusive_write_scopes: ["src/auth"],
              depends_on: [],
              verification_plan_ref: null,
              meta: { summary: "前端登录" },
              reverted: true,
              revert_feedback: "范围过大",
            },
          ],
        })}
        stage="author_confirm"
        onRevert={onRevert}
        onRequestRevision={onRequestRevision}
        onAccept={onAccept}
      />,
    );

    expect(screen.getByText("已标记撤销：范围过大")).toBeInTheDocument();
    await userEvent.click(screen.getByTestId("clear-revert-wi_001"));

    expect(onRevert).toHaveBeenCalledWith("wi_001", "", true);
  });

  it("disables request revision button when no item is reverted", () => {
    render(
      <WorkItemPlanCandidatePanel
        candidate={makeCandidate()}
        stage="author_confirm"
        onRevert={onRevert}
        onRequestRevision={onRequestRevision}
        onAccept={onAccept}
      />,
    );

    expect(screen.getByTestId("request-revision-button")).toBeDisabled();
  });

  it("enables request revision button and triggers callback when items are reverted", async () => {
    render(
      <WorkItemPlanCandidatePanel
        candidate={makeCandidate({
          work_items: [
            {
              candidate_id: "wi_001",
              title: "Frontend Auth",
              kind: "frontend",
              exclusive_write_scopes: ["src/auth"],
              depends_on: [],
              verification_plan_ref: null,
              meta: { summary: "前端登录" },
              reverted: true,
            },
          ],
        })}
        stage="author_confirm"
        onRevert={onRevert}
        onRequestRevision={onRequestRevision}
        onAccept={onAccept}
      />,
    );

    const button = screen.getByTestId("request-revision-button");
    expect(button).not.toBeDisabled();
    expect(button).toHaveTextContent("重新生成被标记的 1 项");

    await userEvent.click(button);

    expect(onRequestRevision).toHaveBeenCalled();
  });

  it("triggers accept callback", async () => {
    render(
      <WorkItemPlanCandidatePanel
        candidate={makeCandidate()}
        stage="author_confirm"
        onRevert={onRevert}
        onRequestRevision={onRequestRevision}
        onAccept={onAccept}
      />,
    );

    await userEvent.click(screen.getByTestId("accept-plan-button"));

    expect(onAccept).toHaveBeenCalled();
  });
});
