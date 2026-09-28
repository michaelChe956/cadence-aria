import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiRequestError } from "./client";
import {
  getLogicalCodebaseBootstrap,
  postLogicalCodebaseBootstrapAction,
} from "./logical-codebase-bootstrap";

function projectionResponse() {
  return {
    project_id: "project_0001",
    logical_codebase_id: "logical_codebase_0001",
    authority_root: "/worktrees/aggregate-root",
    membership_revision: 2,
    policy: {
      policy_id: "policy/project_0001/1",
      policy_revision: 1,
      policy_digest: "sha256:policy",
      artifact_root: "/worktrees/.aria",
    },
    steps: [
      { step: "identity", status: "completed", object_id: "identity-1", checkpoint: null, failure: null, allowed_actions: [] },
      { step: "manifest_checkout", status: "completed", object_id: "manifest-1", checkpoint: null, failure: null, allowed_actions: [] },
      { step: "rules_policy", status: "completed", object_id: "policy-1", checkpoint: null, failure: null, allowed_actions: [] },
      { step: "member_index", status: "completed", object_id: "operation-1", checkpoint: null, failure: null, allowed_actions: [] },
      {
        step: "aggregate_index_active",
        status: "waiting_for_human",
        object_id: "aggregate-index-1",
        checkpoint: null,
        failure: null,
        allowed_actions: ["retry", "revalidate"],
      },
    ],
    planning_ready: false,
    notices: [
      {
        key: "bootstrap:aggregate_index_active:aggregate-index-1:bootstrap_waiting_for_human",
        step: "aggregate_index_active",
        object_id: "aggregate-index-1",
        reason_code: "bootstrap_waiting_for_human",
        summary: "step aggregate_index_active is waiting for an explicit action",
        external_side_effect: "none",
        allowed_actions: ["retry", "revalidate"],
        next_step: null,
        created_at: "",
      },
    ],
  };
}

describe("logical codebase bootstrap api client", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("GET 读取纯投影并透传 snake_case 字段", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(projectionResponse()), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const projection = await getLogicalCodebaseBootstrap(
      "project_0001",
      "logical_codebase_0001",
    );

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/logical-codebases/logical_codebase_0001/bootstrap",
      expect.anything(),
    );
    expect(projection.planning_ready).toBe(false);
    expect(projection.steps).toHaveLength(5);
    expect(projection.notices[0]?.reason_code).toBe(
      "bootstrap_waiting_for_human",
    );
  });

  it("POST 统一动作以 command_id/expected revision 提交并返回结果投影", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          command_id: "cmd-1",
          outcome: "accepted",
          projection: projectionResponse(),
        }),
        { status: 200 },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);

    const result = await postLogicalCodebaseBootstrapAction(
      "project_0001",
      "logical_codebase_0001",
      {
        command_id: "cmd-1",
        step: "member_index",
        action: "continue",
        expected_revision: 2,
        expected_object_id: "operation-1",
      },
    );

    const [, init] = fetchMock.mock.calls[0] as unknown as [
      string,
      RequestInit,
    ];
    expect(fetchMock.mock.calls[0][0]).toBe(
      "/api/projects/project_0001/logical-codebases/logical_codebase_0001/bootstrap/actions",
    );
    expect(JSON.parse(String(init.body))).toEqual({
      command_id: "cmd-1",
      step: "member_index",
      action: "continue",
      expected_revision: 2,
      expected_object_id: "operation-1",
    });
    expect(result.outcome).toBe("accepted");
    expect(result.command_id).toBe("cmd-1");
  });

  it("409 冲突透传稳定码供 UI 呈现", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            code: "bootstrap_stale_revision",
            message: "logical codebase bootstrap action was rejected",
            details: {},
          }),
          { status: 409 },
        ),
      ),
    );

    await expect(
      postLogicalCodebaseBootstrapAction("project_0001", "logical_codebase_0001", {
        command_id: "cmd-stale",
        step: "member_index",
        action: "continue",
        expected_revision: 99,
        expected_object_id: "operation-1",
      }),
    ).rejects.toBeInstanceOf(ApiRequestError);
  });
});
