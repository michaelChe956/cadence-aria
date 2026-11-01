import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiRequestError } from "./client";
import {
  listProjectRepositoryInitializationWaitingItems,
  postRepositoryInitializationResume,
} from "./repository-initialization";

// C5 Task 6/7：project 级 repository 初始化等待项/resume API——沿
// logical-codebase-bootstrap.test.ts 先例断言真实 URL/method/body 与
// 错误稳定码透传（ApiRequestError 不降级为普通 Error）。

function waitingItemResponse() {
  return [
    {
      id: "c1:project:project_0001:repository_init:op_init_0001",
      kind: "repository_initialization_failed",
      reason:
        "repository initialization failed at pre_check (provider_unavailable); awaiting gateway recovery",
      completed_steps: ["cadence_skills"],
      target: null,
      plan_id: null,
      session_id: null,
      attempt_id: null,
      gate_id: null,
      possible_side_effect: null,
      actions: ["resume_repository_initialization"],
      next_phase: "repository_registered",
      action_context: [],
      operation_id: "op_init_0001",
      diagnostics: {
        failed_step: "pre_check",
        reason_code: "provider_unavailable",
        provider: "claude_code",
        stderr_summary: null,
        changed_paths: [],
        retryable: true,
      },
      project_id: "project_0001",
    },
  ];
}

function operationSnapshotResponse() {
  return {
    operation_id: "op_init_0002",
    status: "created",
    steps: [{ step_id: "cadence_skills", status: "pending" }],
    current_step: null,
    failed_step: null,
    result: null,
    error: null,
    created_at: "2026-09-30T00:00:00Z",
    updated_at: "2026-09-30T00:00:00Z",
    completed_at: null,
  };
}

describe("repository initialization waiting items api client", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("GET 返回裸 JSON 数组并透传 snake_case 等待项", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(waitingItemResponse()), { status: 200 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const items = await listProjectRepositoryInitializationWaitingItems(
      "project 0001",
    );

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project%200001/repository-initializations/waiting-items",
      expect.anything(),
    );
    expect(items).toHaveLength(1);
    expect(items[0].kind).toBe("repository_initialization_failed");
    expect(items[0].operation_id).toBe("op_init_0001");
    expect(items[0].diagnostics?.reason_code).toBe("provider_unavailable");
    expect(items[0].project_id).toBe("project_0001");
  });

  it("POST resume 以 command_id 提交并返回 operation snapshot", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(operationSnapshotResponse()), { status: 202 }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const snapshot = await postRepositoryInitializationResume(
      "project_0001",
      "op_init_0001",
      "cmd-repo-init-resume-op_init_0001",
    );

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/repository-initializations/op_init_0001/resume",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          command_id: "cmd-repo-init-resume-op_init_0001",
        }),
      },
    );
    expect(snapshot.operation_id).toBe("op_init_0002");
    expect(snapshot.status).toBe("created");
  });

  it("409 in-progress 冲突透传稳定码供 UI 呈现", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          code: "repository_initialization_in_progress",
          message: "successor operation already running",
          details: {},
        }),
        { status: 409 },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);

    const error = await postRepositoryInitializationResume(
      "project_0001",
      "op_init_0001",
      "cmd-repo-init-resume-op_init_0001",
    ).catch((reason: unknown) => reason);

    expect(error).toBeInstanceOf(ApiRequestError);
    expect((error as ApiRequestError).code).toBe(
      "repository_initialization_in_progress",
    );
  });
});
