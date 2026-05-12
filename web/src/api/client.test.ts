import { describe, expect, it, vi } from "vitest";
import {
  confirmTask,
  createTask,
  getArtifactContent,
  getFileContent,
  getFileDiff,
  listTasks,
  normalizeApiError,
  stopTask,
} from "./client";

describe("api client", () => {
  it("posts create task payload and returns task response", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              task_id: "task_0001",
              session_id: "sess_task_0001",
              change_id: "aria-fibonacci-square",
              phase: "intake",
            }),
            { status: 200 },
          ),
      ),
    );

    const result = await createTask({
      request_text: "实现 Fibonacci square sum",
      change_id: "aria-fibonacci-square",
      policy_preset: "manual-write",
      provider_mode: "fake",
      timeout_secs: 2400,
    });

    expect(result.task_id).toBe("task_0001");
  });

  it("normalizes standard api error", async () => {
    const error = await normalizeApiError(
      new Response(
        JSON.stringify({
          code: "checkpoint_unsafe_dirty_worktree",
          message: "worktree has uncommitted changes",
          details: {},
        }),
        { status: 409 },
      ),
    );
    expect(error.code).toBe("checkpoint_unsafe_dirty_worktree");
  });

  it("throws standard Error with api message for failed confirm requests", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(
        async () =>
          new Response(
            JSON.stringify({
              code: "provider_execution_failed",
              message: "provider command timed out",
              details: { node_id: "N16" },
            }),
            { status: 500 },
          ),
      ),
    );

    let thrown: unknown;
    try {
      await confirmTask("task_0001", {
        checkpoint_id: "ckpt_0001",
        prompt: "confirm",
        policy_override: null,
      });
    } catch (error) {
      thrown = error;
    }

    expect(thrown).toBeInstanceOf(Error);
    expect((thrown as Error).message).toBe("provider command timed out");
  });

  it("calls task resource and stop endpoints with encoded parameters", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return new Response(JSON.stringify({ ok: true, tasks: [] }), { status: 200 });
      }),
    );

    await listTasks();
    await getArtifactContent("artifact/with space");
    await getFileContent("src/file with space.ts");
    await getFileDiff("ckpt_0001", "src/file with space.ts");
    await stopTask("task_0001");

    expect(calls.map((call) => call.input)).toEqual([
      "/api/tasks",
      "/api/artifacts/artifact%2Fwith%20space",
      "/api/files/content?path=src%2Ffile+with+space.ts",
      "/api/files/diff?base_checkpoint=ckpt_0001&path=src%2Ffile+with+space.ts",
      "/api/tasks/task_0001/stop",
    ]);
    expect(calls.at(-1)?.init?.method).toBe("POST");
  });
});
