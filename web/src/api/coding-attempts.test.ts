import { describe, expect, it, vi } from "vitest";
import {
  ApiRequestError,
  abortCodingAttempt,
  confirmWorkItemExecutionPlan,
  createCodingAttempt,
  createGroupCodingAttempt,
  deleteCodingAttempt,
  deletePlanCodingWorkspaces,
  getCodingAttemptArtifact,
  getCodingAttemptDiff,
  getCodingAttemptSnapshot,
  getLegacyCodingAttemptSnapshot,
  getPlanCodingProgress,
  requestWorkItemExecutionPlanChange,
  startPlanCoding,
} from "./client";

const CODING_ATTEMPT_ADDRESS = {
  projectId: "project_0001",
  issueId: "issue_0001",
  attemptId: "coding_attempt_0001",
} as const;

describe("coding attempts api client", () => {
  it("calls coding attempt endpoints with encoded ids and expected payloads", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return new Response(JSON.stringify(codingAttemptResponse()), { status: 200 });
      }),
    );

    await createCodingAttempt("project/with space", "issue/with space", "work item/1");
    const address = {
      projectId: "project/with space",
      issueId: "issue/with space",
      attemptId: "coding attempt/1",
    };
    await getCodingAttemptSnapshot(address);
    await getCodingAttemptDiff(address);
    await abortCodingAttempt(address);
    await deleteCodingAttempt(address);
    await getCodingAttemptArtifact(address, "unit.stdout.log");
    await confirmWorkItemExecutionPlan(address);
    await requestWorkItemExecutionPlanChange(address, { note: "补充边界测试" });

    const basePath =
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/coding-attempts/coding%20attempt%2F1";

    expect(calls.map((call) => call.input)).toEqual([
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/work-items/work%20item%2F1/coding-attempts",
      basePath,
      `${basePath}/diff`,
      `${basePath}/abort`,
      basePath,
      `${basePath}/artifacts/unit.stdout.log`,
      `${basePath}/execution-plan/confirm`,
      `${basePath}/execution-plan/change-request`,
    ]);
    expect(calls[0].init?.method).toBe("POST");
    expect(calls[0].init?.body).toBe(JSON.stringify({}));
    expect(calls[1].init?.method).toBeUndefined();
    expect(calls[2].init?.method).toBeUndefined();
    expect(calls[3].init?.method).toBe("POST");
    expect(calls[4].init?.method).toBe("DELETE");
    expect(calls[6].init?.method).toBe("POST");
    expect(calls[7].init?.body).toBe(JSON.stringify({ note: "补充边界测试" }));
  });

  it("maps coding attempt snapshot and artifact response fields", async () => {
    const responses = [
      codingAttemptSnapshotResponse(),
      codingAttemptDiffResponse(),
      artifactContentResponse(),
    ];
    let index = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        const body = responses[index];
        index += 1;
        return new Response(JSON.stringify(body), { status: 200 });
      }),
    );

    const snapshot = await getCodingAttemptSnapshot(CODING_ATTEMPT_ADDRESS);
    const diff = await getCodingAttemptDiff(CODING_ATTEMPT_ADDRESS);
    const artifact = await getCodingAttemptArtifact(
      CODING_ATTEMPT_ADDRESS,
      "unit.stdout.log",
    );

    expect(snapshot.attempt.stage).toBe("prepare_context");
    expect(snapshot.provider_config_snapshot.author).toBe("fake");
    expect(snapshot.timeline_nodes[0].title).toBe("准备上下文");
    expect(snapshot.pending_gates[0].available_actions[0].action_type).toBe("abort");
    expect(diff).toMatchObject({
      attempt_id: "coding_attempt_0001",
      base_branch: "main",
      worktree_path: "/tmp/worktree",
      diff: "diff --git a/climbing_stairs.py b/climbing_stairs.py\n+def climb_stairs(n):\n",
    });
    expect(artifact).toMatchObject({
      artifact_ref: "unit.stdout.log",
      artifact_kind: "coding_attempt_artifact",
      content_type: "text/plain",
      content: "cargo test ok",
    });
  });

  it("keeps the legacy snapshot lookup isolated from canonical api calls", async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify(codingAttemptSnapshotResponse()), {
        status: 200,
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await getLegacyCodingAttemptSnapshot("coding attempt/1");

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/coding-attempts/coding%20attempt%2F1",
      expect.any(Object),
    );
  });

  it("passes through normalized api errors from coding attempt endpoints", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response(
          JSON.stringify({
            code: "coding_attempt_active",
            message: "active coding attempt already exists",
            details: { attempt_id: "coding_attempt_0001" },
          }),
          { status: 409 },
        ),
      ),
    );

    await expect(
      createCodingAttempt("project_0001", "issue_0001", "work_item_0001"),
    ).rejects.toMatchObject({
      name: "ApiRequestError",
      code: "coding_attempt_active",
      message: "active coding attempt already exists",
      details: { attempt_id: "coding_attempt_0001" },
    });
  });

  it("creates group coding attempts from work item plan route", async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify(codingAttemptResponse()), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await createGroupCodingAttempt(
      "project/with space",
      "issue/with space",
      "plan/1",
    );

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/work-item-plans/plan%2F1/coding-attempts",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("starts plan coding with exact body via explicit coding-start route", async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify(planCodingStartResultResponse()), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const result = await startPlanCoding(
      "project/with space",
      "issue/with space",
      "plan/1",
      {
        command_id: "plan_start_abc123",
        expected_plan_revision_id: "plan_revision_0002",
      },
    );

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/work-item-plans/plan%2F1/coding-start",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          command_id: "plan_start_abc123",
          expected_plan_revision_id: "plan_revision_0002",
        }),
      },
    );
    expect(result.operation_state).toBe("accepted");
    expect(result.selected_attempt_id).toBe("coding_attempt_busi_0001");
    expect(result.start_outcome).toEqual({
      kind: "started",
      attempt_id: "coding_attempt_busi_0001",
    });
  });

  it("gets plan coding progress from the same route without a payload", async () => {
    const fetchMock = vi.fn(
      async (_input: RequestInfo | URL, _init?: RequestInit) =>
        new Response(JSON.stringify(planCodingProgressResponse()), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const progress = await getPlanCodingProgress(
      "project/with space",
      "issue/with space",
      "plan/1",
    );

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0]?.[0]).toBe(
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/work-item-plans/plan%2F1/coding-start",
    );
    const init = fetchMock.mock.calls[0]?.[1] as RequestInit | undefined;
    expect(init?.method).toBeUndefined();
    expect(init?.body).toBeUndefined();
    expect(progress.plan_id).toBe("issue_plan_0001");
    expect(progress.bound_plan_revision_id).toBe("plan_revision_0002");
    expect(progress.plan_complete).toBe(false);
    expect(progress.units).toHaveLength(5);
  });

  it("keeps the legacy group coding attempt post a single empty object", async () => {
    const fetchMock = vi.fn(async () =>
      new Response(JSON.stringify(codingAttemptResponse()), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    await createGroupCodingAttempt("project_0001", "issue_0001", "issue_plan_0001");

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-attempts",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({}),
      },
    );
  });
});

describe("h5: plan coding workspaces group delete client", () => {
  it("encodes command_id with optional generation/confirmed query per frozen contract", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return new Response(JSON.stringify(planCodingWorkspaceDeletionResponse()), {
          status: 200,
        });
      }),
    );

    await deletePlanCodingWorkspaces("project_0001", "issue_0001", "issue_plan_0001", {
      command_id: "plan_delete_ui_1",
      expected_generation: 3,
    });
    await deletePlanCodingWorkspaces("project_0001", "issue_0001", "issue_plan_0001", {
      command_id: "plan_delete_ui_1",
    });
    await deletePlanCodingWorkspaces("project/with space", "issue_0001", "plan/with space", {
      command_id: "plan_delete_ui_2",
      confirmed: true,
    });

    expect(calls.map((call) => call.input)).toEqual([
      "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-workspaces?command_id=plan_delete_ui_1&expected_generation=3",
      "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_plan_0001/coding-workspaces?command_id=plan_delete_ui_1",
      "/api/projects/project%2Fwith%20space/issues/issue_0001/work-item-plans/plan%2Fwith%20space/coding-workspaces?command_id=plan_delete_ui_2&confirmed=true",
    ]);
    expect(calls.every((call) => call.init?.method === "DELETE")).toBe(true);
  });

  it("surfaces typed 409 (clearing/resource busy) as ApiRequestError without retry", async () => {
    let callCount = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        callCount += 1;
        return new Response(
          JSON.stringify({
            code: "plan_coding_run_resource_busy",
            message: "目标仓资源正被使用，本次未完成。请稍后重试。",
            details: { plan_id: "issue_plan_0001" },
          }),
          { status: 409 },
        );
      }),
    );

    const rejection: unknown = await deletePlanCodingWorkspaces(
      "project_0001",
      "issue_0001",
      "issue_plan_0001",
      { command_id: "plan_delete_ui_1" },
    ).then(
      () => new Error("expected resource busy rejection"),
      (reason: unknown) => reason,
    );

    expect(rejection).toBeInstanceOf(ApiRequestError);
    expect((rejection as ApiRequestError).code).toBe("plan_coding_run_resource_busy");
    expect(callCount).toBe(1);
  });
});

function codingAttemptResponse() {
  return {
    project_id: "project_0001",
    issue_id: "issue_0001",
    attempt_id: "coding_attempt_0001",
    work_item_id: "work_item_0001",
    attempt_no: 1,
    status: "created",
    stage: "prepare_context",
    branch_name: "aria/work-items/work_item_0001/attempt-1",
    base_branch: "main",
    worktree_path: null,
    rework_count: 0,
    head_commit: null,
    push_status: null,
    review_request_url: null,
    created_at: "2026-05-23T00:00:00Z",
    updated_at: "2026-05-23T00:00:00Z",
  };
}

function codingAttemptSnapshotResponse() {
  return {
    attempt: codingAttemptResponse(),
    provider_config_snapshot: {
      author: "fake",
      reviewer: "fake",
      review_rounds: 1,
    },
    timeline_nodes: [
      {
        id: "coding_node_0001",
        attempt_id: "coding_attempt_0001",
        stage: "prepare_context",
        title: "准备上下文",
        status: "running",
        agent_role: "system",
        summary: null,
        started_at: "2026-05-23T00:00:00Z",
        completed_at: null,
        artifact_refs: [],
      },
    ],
    active_node_id: "coding_node_0001",
    code_review_reports: [],
    review_request: null,
    internal_pr_review: null,
    pending_gates: [
      {
        gate_id: "gate_0001",
        kind: "blocked",
        title: "需要人工处理",
        description: "测试失败次数达到上限",
        available_actions: [
          {
            action_id: "abort",
            label: "中止",
            action_type: "abort",
          },
        ],
      },
    ],
    pending_choices: [],
    work_item_execution_plan: null,
    require_execution_plan_confirm: false,
  };
}

function codingAttemptDiffResponse() {
  return {
    attempt_id: "coding_attempt_0001",
    base_branch: "main",
    worktree_path: "/tmp/worktree",
    diff: "diff --git a/climbing_stairs.py b/climbing_stairs.py\n+def climb_stairs(n):\n",
  };
}

function artifactContentResponse() {
  return {
    artifact_ref: "unit.stdout.log",
    artifact_kind: "coding_attempt_artifact",
    producer_node: null,
    path: "/tmp/worktree/.aria/coding-artifacts/test-output/unit.stdout.log",
    content_type: "text/plain",
    content: "cargo test ok",
  };
}

// N6（REQ-CS-01/06）：N3 冻结 DTO 的镜像夹具——四 target/五 WI（busi=wi_core、
// web=wi_registration、gateway=wi_unrelated、frontend={wi_service, wi_panel}），
// wi_panel 直接依赖 {wi_service, wi_unrelated}。
function planCodingUnitRecord(overrides: Record<string, unknown> = {}) {
  return {
    unit_id: "coding_unit_0001",
    logical_work_item_id: "wi_core",
    target_repository_id: "repo_busi",
    attempt_id: "coding_attempt_busi_0001",
    unit_status: "running",
    execution_state: "running",
    unsatisfied_dependency_ids: [],
    waiting_reason: null,
    ...overrides,
  };
}

function planCodingProgressResponse() {
  return {
    plan_id: "issue_plan_0001",
    bound_plan_revision_id: "plan_revision_0002",
    command_id: null,
    mode: null,
    phase: null,
    primary_attempt_id: null,
    selected_unit_id: null,
    selected_attempt_id: null,
    target_attempts: [],
    units: [
      planCodingUnitRecord(),
      planCodingUnitRecord({
        unit_id: "coding_unit_0002",
        logical_work_item_id: "wi_registration",
        target_repository_id: "repo_web",
        attempt_id: "coding_attempt_web_0001",
        unit_status: "pending",
        execution_state: "waiting_dependency",
        unsatisfied_dependency_ids: ["wi_core"],
        waiting_reason: "dependency_not_ready",
      }),
      planCodingUnitRecord({
        unit_id: "coding_unit_0003",
        logical_work_item_id: "wi_unrelated",
        target_repository_id: "repo_gateway",
        attempt_id: "coding_attempt_gateway_0001",
        unit_status: "pending",
        execution_state: "waiting_slot",
        unsatisfied_dependency_ids: [],
        waiting_reason: "execution_slot_occupied",
      }),
      planCodingUnitRecord({
        unit_id: "coding_unit_0004",
        logical_work_item_id: "wi_service",
        target_repository_id: "repo_frontend",
        attempt_id: "coding_attempt_frontend_0001",
        unit_status: "pending",
        execution_state: "waiting_dependency",
        unsatisfied_dependency_ids: ["wi_unrelated"],
        waiting_reason: "dependency_not_ready",
      }),
      planCodingUnitRecord({
        unit_id: "coding_unit_0005",
        logical_work_item_id: "wi_panel",
        target_repository_id: "repo_frontend",
        attempt_id: "coding_attempt_frontend_0001",
        unit_status: "pending",
        execution_state: "waiting_dependency",
        unsatisfied_dependency_ids: ["wi_service", "wi_unrelated"],
        waiting_reason: "dependency_not_ready",
      }),
    ],
    plan_complete: false,
  };
}

function planCodingStartResultResponse() {
  const progress = planCodingProgressResponse();
  return {
    plan_id: progress.plan_id,
    bound_plan_revision_id: progress.bound_plan_revision_id,
    command_id: "plan_start_abc123",
    operation_state: "accepted",
    primary_attempt_id: "coding_attempt_busi_0001",
    target_attempts: [],
    selected_unit_id: "coding_unit_0001",
    selected_attempt_id: "coding_attempt_busi_0001",
    start_outcome: {
      kind: "started",
      attempt_id: "coding_attempt_busi_0001",
    },
    waiting_units: [],
    progress,
  };
}

function planCodingWorkspaceDeletionResponse() {
  return {
    plan_id: "issue_plan_0001",
    command_id: "plan_delete_ui_1",
    operation_state: "accepted",
    generation: 3,
    deleted_attempt_ids: ["coding_attempt_busi_0001", "coding_attempt_web_0001"],
    remaining_attempt_ids: [],
  };
}
