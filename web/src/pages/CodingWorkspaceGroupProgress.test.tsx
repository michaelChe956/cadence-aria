import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { CodingExecutionUnit, PlanCodingProgressDto } from "../api/types";
import { CodingWorkspaceGroupProgress } from "./CodingWorkspaceGroupProgress";
import {
  planCodingGroupDeleteEntry,
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

describe("h5: plan workspace lifecycle projection and group delete entry copy", () => {
  it("projects workspace_lifecycle facts verbatim", () => {
    const view = toPlanCodingProgressView(
      planCodingProgressDto({
        workspace_lifecycle: {
          generation: 3,
          existing_member_count: 2,
          missing_member_count: 1,
          state: "incomplete",
        },
      }),
    );
    expect(view.workspaceLifecycle).toEqual({
      generation: 3,
      existingMemberCount: 2,
      missingMemberCount: 1,
      state: "incomplete",
    });
  });

  it("derives full-plan copy when no members are missing", () => {
    const entry = planCodingGroupDeleteEntry(
      toPlanCodingProgressView(
        planCodingProgressDto({
          workspace_lifecycle: {
            generation: 1,
            existing_member_count: 4,
            missing_member_count: 0,
            state: "clean",
          },
        }),
      ),
    );
    expect(entry.label).toBe("删除本计划全部 Coding 工作区(4)");
    expect(entry.invalidationSummary).toBeNull();
  });

  it("derives remaining copy and invalidation summary after partial deletion", () => {
    const entry = planCodingGroupDeleteEntry(
      toPlanCodingProgressView(
        planCodingProgressDto({
          workspace_lifecycle: {
            generation: 2,
            existing_member_count: 2,
            missing_member_count: 1,
            state: "incomplete",
          },
        }),
      ),
    );
    expect(entry.label).toBe("删除剩余 Coding 工作区(2)");
    expect(entry.invalidationSummary).toBe("本轮授权失效:已删 1/3");
  });

  it("falls back to target counts when workspace_lifecycle is absent (pre-H4 server)", () => {
    const legacy = planCodingGroupDeleteEntry(
      toPlanCodingProgressView(planCodingProgressDto()),
    );
    expect(legacy.label).toBe("删除本计划全部 Coding 工作区(4)");
    expect(legacy.invalidationSummary).toBeNull();

    const empty = planCodingGroupDeleteEntry(
      toPlanCodingProgressView(planCodingProgressDto({ target_attempts: [] })),
    );
    expect(empty.label).toBe("删除本计划全部 Coding 工作区");
  });
});

describe("h5: plan-scoped group delete entry reachability", () => {
  it("renders the entry with full-plan count in the aggregate header and fires one request", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify(
              planCodingProgressDto({
                workspace_lifecycle: {
                  generation: 1,
                  existing_member_count: 4,
                  missing_member_count: 0,
                  state: "clean",
                },
              }),
            ),
            { status: 200 },
          ),
      ),
    );
    const onRequest = vi.fn();

    render(
      <CodingWorkspaceGroupProgress
        planId="issue_plan_0001"
        currentWorkItemId="work_item_0001"
        units={LOCAL_UNITS}
        projectId="project_0001"
        issueId="issue_0001"
        onDeleteAllCodingWorkspaces={onRequest}
      />,
    );

    // 聚合分支就绪标记先等 fetch 落地（按钮在局部/slim 分支也立即可见）。
    await screen.findByTestId("plan-coding-progress");
    const button = screen.getByTestId("plan-coding-delete-all");
    expect(button).toHaveTextContent("删除本计划全部 Coding 工作区(4)");
    expect(screen.queryByTestId("plan-coding-invalidation-summary")).toBeNull();
    fireEvent.click(button);
    expect(onRequest).toHaveBeenCalledTimes(1);

    vi.unstubAllGlobals();
  });

  it("switches to 删除剩余 copy with invalidation summary after partial deletion", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify(
              planCodingProgressDto({
                workspace_lifecycle: {
                  generation: 2,
                  existing_member_count: 2,
                  missing_member_count: 1,
                  state: "incomplete",
                },
              }),
            ),
            { status: 200 },
          ),
      ),
    );

    render(
      <CodingWorkspaceGroupProgress
        planId="issue_plan_0001"
        currentWorkItemId="work_item_0001"
        units={LOCAL_UNITS}
        projectId="project_0001"
        issueId="issue_0001"
        onDeleteAllCodingWorkspaces={vi.fn()}
      />,
    );

    await screen.findByTestId("plan-coding-progress");
    const button = screen.getByTestId("plan-coding-delete-all");
    expect(button).toHaveTextContent("删除剩余 Coding 工作区(2)");
    expect(screen.getByTestId("plan-coding-invalidation-summary")).toHaveTextContent(
      "本轮授权失效:已删 1/3",
    );

    vi.unstubAllGlobals();
  });

  it("keeps the entry reachable in a slim row when local units are empty and the aggregate fails", async () => {
    const fetchMock = vi.fn(
      async () =>
        new Response(JSON.stringify({ code: "not_found", message: "no run" }), {
          status: 404,
        }),
    );
    vi.stubGlobal("fetch", fetchMock);
    const onRequest = vi.fn();

    render(
      <CodingWorkspaceGroupProgress
        planId="issue_plan_0001"
        currentWorkItemId={null}
        units={[]}
        projectId="project_0001"
        issueId="issue_0001"
        onDeleteAllCodingWorkspaces={onRequest}
      />,
    );

    const button = await screen.findByTestId("plan-coding-delete-all");
    expect(button).toHaveTextContent("删除本计划全部 Coding 工作区");
    fireEvent.click(button);
    expect(onRequest).toHaveBeenCalledTimes(1);

    vi.unstubAllGlobals();
  });

  it("renders nothing when units are empty, the aggregate is absent and no entry handler is wired", async () => {
    const fetchMock = vi.fn(
      async () =>
        new Response(JSON.stringify({ code: "not_found", message: "no run" }), {
          status: 404,
        }),
    );
    vi.stubGlobal("fetch", fetchMock);

    render(
      <CodingWorkspaceGroupProgress
        planId="issue_plan_0001"
        currentWorkItemId={null}
        units={[]}
        projectId="project_0001"
        issueId="issue_0001"
      />,
    );

    await waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(screen.queryByTestId("plan-coding-delete-all")).toBeNull();
    expect(screen.queryByTestId("onboarding-anchor-coding-progress")).toBeNull();

    vi.unstubAllGlobals();
  });

  it("keeps the entry in the legacy local projection branch without counts", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    const onRequest = vi.fn();

    render(
      <CodingWorkspaceGroupProgress
        planId="issue_plan_0001"
        currentWorkItemId="work_item_0001"
        units={LOCAL_UNITS}
        onDeleteAllCodingWorkspaces={onRequest}
      />,
    );

    expect(await screen.findByText("WorkItemGroup")).toBeInTheDocument();
    const button = screen.getByTestId("plan-coding-delete-all");
    expect(button).toHaveTextContent("删除本计划全部 Coding 工作区");
    fireEvent.click(button);
    expect(onRequest).toHaveBeenCalledTimes(1);
    expect(fetchMock).not.toHaveBeenCalled();

    vi.unstubAllGlobals();
  });

  it("disables the entry while a group delete request is in flight", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify(planCodingProgressDto()), { status: 200 })),
    );

    render(
      <CodingWorkspaceGroupProgress
        planId="issue_plan_0001"
        currentWorkItemId="work_item_0001"
        units={LOCAL_UNITS}
        projectId="project_0001"
        issueId="issue_0001"
        onDeleteAllCodingWorkspaces={vi.fn()}
        groupDeleteBusy
      />,
    );

    expect(await screen.findByTestId("plan-coding-delete-all")).toBeDisabled();

    vi.unstubAllGlobals();
  });
});
