import { describe, expect, it, vi } from "vitest";
import {
  abortCodingAttempt,
  createCodingAttempt,
  createGroupCodingAttempt,
  getCodingAttemptArtifact,
  getCodingAttemptDiff,
  getCodingAttemptSnapshot,
} from "./client";

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
    await getCodingAttemptSnapshot("coding attempt/1");
    await getCodingAttemptDiff("coding attempt/1");
    await abortCodingAttempt("coding attempt/1");
    await getCodingAttemptArtifact("coding attempt/1", "unit.stdout.log");

    expect(calls.map((call) => call.input)).toEqual([
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/work-items/work%20item%2F1/coding-attempts",
      "/api/coding-attempts/coding%20attempt%2F1",
      "/api/coding-attempts/coding%20attempt%2F1/diff",
      "/api/coding-attempts/coding%20attempt%2F1/abort",
      "/api/coding-attempts/coding%20attempt%2F1/artifacts/unit.stdout.log",
    ]);
    expect(calls[0].init?.method).toBe("POST");
    expect(calls[0].init?.body).toBe(JSON.stringify({}));
    expect(calls[1].init?.method).toBeUndefined();
    expect(calls[2].init?.method).toBeUndefined();
    expect(calls[3].init?.method).toBe("POST");
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

    const snapshot = await getCodingAttemptSnapshot("coding_attempt_0001");
    const diff = await getCodingAttemptDiff("coding_attempt_0001");
    const artifact = await getCodingAttemptArtifact("coding_attempt_0001", "unit.stdout.log");

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
});

function codingAttemptResponse() {
  return {
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
    testing_report: null,
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
    work_item_handoff: null,
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
