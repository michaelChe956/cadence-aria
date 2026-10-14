import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { CodingExecutionUnit, PlanCodingProgressDto } from "../api/types";
import { CodingWorkspaceGroupProgress } from "./CodingWorkspaceGroupProgress";
import {
  planCodingWaitingLabel,
  toPlanCodingProgressView,
} from "./plan-coding-progress";

// N6（REQ-CS-06）：N3 冻结聚合 DTO 夹具——四 target / 五 WI，镜像 N3 后端测试拓扑
// （busi=wi_core、web=wi_registration（跨桶依赖 wi_core）、gateway=wi_unrelated、
// frontend={wi_service, wi_panel}，wi_panel 直接依赖 {wi_service, wi_unrelated}）。
// primary 已 Completed 但 plan_complete=false：聚合不冒充整 plan 完成。
function planCodingProgressDto(
  overrides: Partial<PlanCodingProgressDto> = {},
): PlanCodingProgressDto {
  return {
    plan_id: "issue_plan_0001",
    bound_plan_revision_id: "plan_revision_0002",
    command_id: "plan_start_abc123",
    mode: "first_ready_unit",
    phase: "paused",
    primary_attempt_id: "coding_attempt_busi_0001",
    selected_unit_id: "coding_unit_0001",
    selected_attempt_id: "coding_attempt_busi_0001",
    target_attempts: [
      {
        target_repository_id: "repo_busi",
        attempt_id: "coding_attempt_busi_0001",
        attempt_status: "blocked",
        worktree_path: "/tmp/wt/busi",
        lease_state: "held",
      },
      {
        target_repository_id: "repo_web",
        attempt_id: "coding_attempt_web_0001",
        attempt_status: "created",
        worktree_path: null,
        lease_state: "absent",
      },
      {
        target_repository_id: "repo_gateway",
        attempt_id: "coding_attempt_gateway_0001",
        attempt_status: "created",
        worktree_path: null,
        lease_state: "absent",
      },
      {
        target_repository_id: "repo_frontend",
        attempt_id: "coding_attempt_frontend_0001",
        attempt_status: "created",
        worktree_path: null,
        lease_state: "absent",
      },
    ],
    units: [
      {
        unit_id: "coding_unit_0001",
        logical_work_item_id: "wi_core",
        target_repository_id: "repo_busi",
        attempt_id: "coding_attempt_busi_0001",
        unit_status: "completed",
        execution_state: "completed",
        unsatisfied_dependency_ids: [],
        waiting_reason: null,
        dependency_logical_work_item_ids: [],
        order_index: 0,
        summary: null,
      },
      {
        unit_id: "coding_unit_0002",
        logical_work_item_id: "wi_registration",
        target_repository_id: "repo_web",
        attempt_id: "coding_attempt_web_0001",
        unit_status: "pending",
        execution_state: "waiting_dependency",
        unsatisfied_dependency_ids: ["wi_core"],
        waiting_reason: "dependency_not_ready",
        dependency_logical_work_item_ids: ["wi_core"],
        order_index: 1,
        summary: null,
      },
      {
        unit_id: "coding_unit_0003",
        logical_work_item_id: "wi_unrelated",
        target_repository_id: "repo_gateway",
        attempt_id: "coding_attempt_gateway_0001",
        unit_status: "pending",
        execution_state: "waiting_slot",
        unsatisfied_dependency_ids: [],
        waiting_reason: "execution_slot_occupied",
        dependency_logical_work_item_ids: [],
        order_index: 2,
        summary: null,
      },
      {
        unit_id: "coding_unit_0004",
        logical_work_item_id: "wi_service",
        target_repository_id: "repo_frontend",
        attempt_id: "coding_attempt_frontend_0001",
        unit_status: "pending",
        execution_state: "initialized",
        unsatisfied_dependency_ids: ["wi_unrelated"],
        waiting_reason: "dependency_not_ready",
        dependency_logical_work_item_ids: ["wi_unrelated"],
        order_index: 3,
        summary: null,
      },
      {
        unit_id: "coding_unit_0005",
        logical_work_item_id: "wi_panel",
        target_repository_id: "repo_frontend",
        attempt_id: "coding_attempt_frontend_0001",
        unit_status: "pending",
        execution_state: "paused",
        unsatisfied_dependency_ids: ["wi_service", "wi_unrelated"],
        waiting_reason: "plan_coding_continuation_not_enabled",
        dependency_logical_work_item_ids: ["wi_unrelated", "wi_service"],
        order_index: 4,
        summary: null,
      },
    ],
    plan_complete: false,
    ...overrides,
  };
}

describe("toPlanCodingProgressView pure projection", () => {
  it("projects owning target, attempt and full direct dependency ids per unit", () => {
    const view = toPlanCodingProgressView(planCodingProgressDto());

    expect(view.planId).toBe("issue_plan_0001");
    expect(view.boundPlanRevisionId).toBe("plan_revision_0002");
    expect(view.phase).toBe("paused");
    expect(view.selectedAttemptId).toBe("coding_attempt_busi_0001");
    expect(view.targets).toHaveLength(4);
    expect(view.units).toHaveLength(5);

    const panel = view.units.find((unit) => unit.logicalWorkItemId === "wi_panel");
    expect(panel).toMatchObject({
      targetRepositoryId: "repo_frontend",
      attemptId: "coding_attempt_frontend_0001",
      unsatisfiedDependencyIds: ["wi_service", "wi_unrelated"],
    });
    const registration = view.units.find(
      (unit) => unit.logicalWorkItemId === "wi_registration",
    );
    expect(registration?.unsatisfiedDependencyIds).toEqual(["wi_core"]);
  });

  it("maps the three waiting reasons to REQ-CS-06 copy", () => {
    const view = toPlanCodingProgressView(planCodingProgressDto());

    expect(
      view.units.find((unit) => unit.logicalWorkItemId === "wi_registration")
        ?.waitingLabel,
    ).toBe("等待依赖完成");
    expect(
      view.units.find((unit) => unit.logicalWorkItemId === "wi_unrelated")
        ?.waitingLabel,
    ).toBe("等待执行槽位");
    expect(
      view.units.find((unit) => unit.logicalWorkItemId === "wi_panel")
        ?.waitingLabel,
    ).toBe("下一项编排尚未启用");
  });

  it("does not mistake a completed primary or initialized running for plan completion or provider running", () => {
    const view = toPlanCodingProgressView(planCodingProgressDto());

    // primary Completed 不误 plan_completed：只透传 DTO 事实，不重算。
    expect(view.units[0]?.executionState).toBe("completed");
    expect(view.planComplete).toBe(false);
    expect(view.completedUnitCount).toBe(1);

    // 初始化桶首 unit Running（物化事实）不是 provider 运行。
    const service = view.units.find(
      (unit) => unit.logicalWorkItemId === "wi_service",
    );
    expect(service?.unitStatus).toBe("pending");
    expect(service?.executionState).toBe("initialized");
    expect(service?.executionStateLabel).not.toContain("provider");
    expect(
      toPlanCodingProgressView(
        planCodingProgressDto({
          units: [
            {
              unit_id: "coding_unit_0009",
              logical_work_item_id: "wi_init",
              target_repository_id: "repo_busi",
              attempt_id: "coding_attempt_busi_0001",
              unit_status: "running",
              execution_state: "initialized",
              unsatisfied_dependency_ids: [],
              waiting_reason: null,
              dependency_logical_work_item_ids: [],
              order_index: 0,
              summary: null,
            },
          ],
        }),
      ).units[0]?.executionStateLabel,
    ).not.toContain("provider");
    expect(
      toPlanCodingProgressView(
        planCodingProgressDto({
          units: [
            {
              unit_id: "coding_unit_0010",
              logical_work_item_id: "wi_run",
              target_repository_id: "repo_busi",
              attempt_id: "coding_attempt_busi_0001",
              unit_status: "running",
              execution_state: "running",
              unsatisfied_dependency_ids: [],
              waiting_reason: null,
              dependency_logical_work_item_ids: [],
              order_index: 0,
              summary: null,
            },
          ],
        }),
      ).units[0]?.executionStateLabel,
    ).toContain("provider");
  });

  it("falls back to execution state copy when waiting reason is absent", () => {
    expect(
      planCodingWaitingLabel({
        unit_id: "u1",
        logical_work_item_id: "wi",
        target_repository_id: "repo",
        attempt_id: "a",
        unit_status: "pending",
        execution_state: "waiting_dependency",
        unsatisfied_dependency_ids: [],
        waiting_reason: null,
        dependency_logical_work_item_ids: [],
        order_index: 0,
        summary: null,
      }),
    ).toBe("等待依赖完成");
    expect(
      planCodingWaitingLabel({
        unit_id: "u2",
        logical_work_item_id: "wi",
        target_repository_id: "repo",
        attempt_id: "a",
        unit_status: "pending",
        execution_state: "running",
        unsatisfied_dependency_ids: [],
        waiting_reason: null,
        dependency_logical_work_item_ids: [],
        order_index: 0,
        summary: null,
      }),
    ).toBeNull();
  });

  it("maps invalidated plan-run authorization to its copy instead of the raw token", () => {
    const view = toPlanCodingProgressView(
      planCodingProgressDto({
        phase: "invalidated",
        units: [
          {
            unit_id: "coding_unit_0011",
            logical_work_item_id: "wi_revoked",
            target_repository_id: "repo_busi",
            attempt_id: "coding_attempt_busi_0001",
            unit_status: "pending",
            execution_state: "invalidated",
            unsatisfied_dependency_ids: [],
            waiting_reason: null,
            dependency_logical_work_item_ids: [],
            order_index: 0,
            summary: null,
          },
        ],
      }),
    );

    expect(view.units[0]?.executionState).toBe("invalidated");
    expect(view.units[0]?.executionStateLabel).toBe("授权已失效");
    expect(view.units[0]?.waitingLabel).toBeNull();
  });
});

const LOCAL_UNITS: CodingExecutionUnit[] = [
  {
    unit_id: "coding_unit_0001",
    logical_work_item_id: "work_item_0001",
    work_item_revision_id: "work_item_revision_0001",
    dependency_logical_work_item_ids: [],
    order_index: 0,
    status: "running",
    summary: null,
    latest_handoff_revision_id: null,
    completion_commit: null,
  },
  {
    unit_id: "coding_unit_0002",
    logical_work_item_id: "work_item_0002",
    work_item_revision_id: "work_item_revision_0002",
    dependency_logical_work_item_ids: ["work_item_0001"],
    order_index: 1,
    status: "pending",
    summary: null,
    latest_handoff_revision_id: null,
    completion_commit: null,
  },
];

describe("CodingWorkspaceGroupProgress aggregated plan progress", () => {
  it("fetches the plan aggregate itself and renders owning repos, dependency ids and waiting copy", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response(JSON.stringify(planCodingProgressDto()), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    render(
      <CodingWorkspaceGroupProgress
        planId="issue_plan_0001"
        currentWorkItemId="work_item_0001"
        units={LOCAL_UNITS}
        projectId="project_0001"
        issueId="issue_0001"
        selectedAttemptId="coding_attempt_busi_0001"
      />,
    );

    const section = await screen.findByTestId("plan-coding-progress");
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-start",
    );
    const init = fetchMock.mock.calls[0]?.[1] as RequestInit | undefined;
    expect(init?.method).toBeUndefined();
    expect(init?.body).toBeUndefined();
    expect(section).toHaveTextContent("repo_busi");
    expect(section).toHaveTextContent("repo_web");
    expect(section).toHaveTextContent("repo_gateway");
    expect(section).toHaveTextContent("repo_frontend");
    expect(section).toHaveTextContent("wi_panel");
    expect(section).toHaveTextContent("wi_service");
    expect(section).toHaveTextContent("wi_unrelated");
    expect(section).toHaveTextContent("等待依赖完成");
    expect(section).toHaveTextContent("等待执行槽位");
    expect(section).toHaveTextContent("下一项编排尚未启用");
    // selected 工作台入口可见，plan 未完成如实呈现。
    expect(section).toHaveTextContent("coding_attempt_busi_0001");
    expect(section).toHaveTextContent("plan 未完成");
    expect(section).not.toHaveTextContent("plan 完成");

    vi.unstubAllGlobals();
  });

  it("keeps the legacy local projection when no address props are provided", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    render(
      <CodingWorkspaceGroupProgress
        planId="issue_plan_0001"
        currentWorkItemId="work_item_0001"
        units={LOCAL_UNITS}
      />,
    );

    expect(await screen.findByText("WorkItemGroup")).toBeInTheDocument();
    expect(screen.getByText("1 / 2")).toBeInTheDocument();
    expect(screen.queryByTestId("plan-coding-progress")).toBeNull();
    expect(fetchMock).not.toHaveBeenCalled();

    vi.unstubAllGlobals();
  });

  it("degrades to the local projection when the aggregate request fails", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response(JSON.stringify({ code: "not_found", message: "no run" }), {
          status: 404,
        }),
      ),
    );

    render(
      <CodingWorkspaceGroupProgress
        planId="issue_plan_0001"
        currentWorkItemId="work_item_0001"
        units={LOCAL_UNITS}
        projectId="project_0001"
        issueId="issue_0001"
      />,
    );

    expect(await screen.findByText("1 / 2")).toBeInTheDocument();
    expect(screen.queryByTestId("plan-coding-progress")).toBeNull();

    vi.unstubAllGlobals();
  });

  it("re-pulls the aggregate when the refresh key changes", async () => {
    let calls = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        calls += 1;
        return new Response(JSON.stringify(planCodingProgressDto()), {
          status: 200,
        });
      }),
    );

    const { rerender } = render(
      <CodingWorkspaceGroupProgress
        planId="issue_plan_0001"
        currentWorkItemId="work_item_0001"
        units={LOCAL_UNITS}
        projectId="project_0001"
        issueId="issue_0001"
        selectedAttemptId="coding_attempt_busi_0001"
        refreshKey="a"
      />,
    );
    await screen.findByTestId("plan-coding-progress");
    expect(calls).toBe(1);

    rerender(
      <CodingWorkspaceGroupProgress
        planId="issue_plan_0001"
        currentWorkItemId="work_item_0002"
        units={LOCAL_UNITS}
        projectId="project_0001"
        issueId="issue_0001"
        selectedAttemptId="coding_attempt_busi_0001"
        refreshKey="b"
      />,
    );

    await waitFor(() => expect(calls).toBe(2));

    vi.unstubAllGlobals();
  });
});
