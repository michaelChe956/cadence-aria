import { describe, expect, it, vi } from "vitest";
import {
  advanceTask,
  confirmTask,
  createIssue,
  createProject,
  createProductIssue,
  createRepository,
  createTask,
  createWorkspace,
  deleteProductIssue,
  deleteProject,
  deleteRepository,
  deleteWorkspace,
  getArtifactContent,
  getFileContent,
  getFileDiff,
  getIssueLifecycle,
  getProjection,
  generateStorySpecs,
  listProductIssues,
  listProjects,
  listRepositories,
  listTasks,
  normalizeApiError,
  startIssue,
  startProductIssue,
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

  it("creates a workspace through the api", async () => {
    const fetchMock = vi.fn(
      async () =>
        new Response(
          JSON.stringify({
            workspace_id: "workspace_0001",
            name: "Repo",
            path: "/tmp/repo",
            default_policy_preset: "manual-write",
            default_provider_mode: "fake",
            created_at: "2026-05-14T00:00:00Z",
            updated_at: "2026-05-14T00:00:00Z",
          }),
          { status: 200 },
        ),
    );
    vi.stubGlobal("fetch", fetchMock);

    await createWorkspace({ name: "Repo", path: "/tmp/repo" });

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/workspaces",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ name: "Repo", path: "/tmp/repo" }),
      }),
    );
  });

  it("lists and creates product projects through the api", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    const responses = [
      { projects: [] },
      {
        project_id: "project_0001",
        name: "Aria",
        description: null,
        created_at: "2026-05-14T00:00:00Z",
        updated_at: "2026-05-14T00:00:00Z",
        last_opened_at: null,
      },
    ];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return new Response(JSON.stringify(responses[calls.length - 1]), { status: 200 });
      }),
    );

    await listProjects();
    await createProject({ name: "Aria", description: null });

    expect(calls.map((call) => call.input)).toEqual(["/api/projects", "/api/projects"]);
    expect(calls[1].init?.method).toBe("POST");
    expect(calls[1].init?.body).toBe(JSON.stringify({ name: "Aria", description: null }));
  });

  it("calls product repository and issue endpoints with project scoped payloads", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return new Response(JSON.stringify({ ok: true, repositories: [], issues: [] }), {
          status: 200,
        });
      }),
    );

    await listRepositories("project/with space");
    await createRepository("project/with space", {
      name: "Aria Core",
      path: "/tmp/aria-core",
      default_policy_preset: null,
      default_provider_mode: null,
    });
    await listProductIssues("project/with space");
    await createProductIssue("project/with space", {
      title: "新增计费设置",
      description: "需要先确认 story spec",
      change_id: null,
      repository_id: "repository_0001",
    });
    await startProductIssue("project/with space", "issue/with space", {
      repository_id: "repository_0001",
    });

    expect(calls.map((call) => call.input)).toEqual([
      "/api/projects/project%2Fwith%20space/repositories",
      "/api/projects/project%2Fwith%20space/repositories",
      "/api/projects/project%2Fwith%20space/issues",
      "/api/projects/project%2Fwith%20space/issues",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/start",
    ]);
    expect(calls[1].init?.method).toBe("POST");
    expect(calls[1].init?.body).toBe(
      JSON.stringify({
        name: "Aria Core",
        path: "/tmp/aria-core",
        default_policy_preset: null,
        default_provider_mode: null,
      }),
    );
    expect(calls[3].init?.method).toBe("POST");
    expect(calls[3].init?.body).toBe(
      JSON.stringify({
        title: "新增计费设置",
        description: "需要先确认 story spec",
        change_id: null,
        repository_id: "repository_0001",
      }),
    );
    expect(calls[4].init?.method).toBe("POST");
    expect(calls[4].init?.body).toBe(JSON.stringify({ repository_id: "repository_0001" }));
  });

  it("calls lifecycle generation endpoints with encoded ids", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return new Response(JSON.stringify({ ok: true, story_specs: [], messages: [] }), {
          status: 200,
        });
      }),
    );

    await getIssueLifecycle("issue/with space", "project/with space");
    await generateStorySpecs("project/with space", "issue/with space", { title: "Story" });

    expect(calls.map((call) => call.input)).toEqual([
      "/api/issues/issue%2Fwith%20space/lifecycle?project_id=project%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/story-specs:generate",
    ]);
    expect(calls[1].init?.method).toBe("POST");
    expect(calls[1].init?.body).toBe(JSON.stringify({ title: "Story" }));
  });

  it("calls delete endpoints with encoded resource ids", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return new Response(JSON.stringify({ status: "deleted" }), { status: 200 });
      }),
    );

    await deleteWorkspace("workspace/with space");
    await deleteProject("project/with space");
    await deleteRepository("project/with space", "repository/with space");
    await deleteProductIssue("project/with space", "issue/with space");

    expect(calls.map((call) => call.input)).toEqual([
      "/api/workspaces/workspace%2Fwith%20space",
      "/api/projects/project%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/repositories/repository%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space",
    ]);
    expect(calls.every((call) => call.init?.method === "DELETE")).toBe(true);
  });

  it("creates and starts an issue through the api", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return new Response(
          JSON.stringify({
            issue_id: "issue_0001",
            workspace_id: "workspace_0001",
            task_id: "task_0001",
            session_id: "sess_task_0001",
            status: "started",
          }),
          { status: 200 },
        );
      }),
    );

    await createIssue({ title: "Implement picker", description: "Select repo" });
    await startIssue("issue_0001", { workspace_id: "workspace_0001" });

    expect(calls.map((call) => call.input)).toEqual([
      "/api/issues",
      "/api/issues/issue_0001/start",
    ]);
    expect(calls[0].init?.method).toBe("POST");
    expect(calls[1].init?.body).toBe(JSON.stringify({ workspace_id: "workspace_0001" }));
  });

  it("adds workspace id to execution API query strings", async () => {
    const calls: Array<string> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        calls.push(String(input));
        return new Response(JSON.stringify({ ok: true }), { status: 200 });
      }),
    );

    await getProjection("task_0001", "N16", "workspace_0001");
    await getArtifactContent("artifact", "workspace_0001");
    await getFileContent("src/main.rs", "workspace_0001");
    await getFileDiff("ckpt_0001", "src/main.rs", "workspace_0001");
    await advanceTask("task_0001", "workspace_0001");
    await stopTask("task_0001", "workspace_0001");

    expect(calls).toEqual([
      "/api/projection?task_id=task_0001&node_id=N16&workspace_id=workspace_0001",
      "/api/artifacts/artifact?workspace_id=workspace_0001",
      "/api/files/content?path=src%2Fmain.rs&workspace_id=workspace_0001",
      "/api/files/diff?base_checkpoint=ckpt_0001&path=src%2Fmain.rs&workspace_id=workspace_0001",
      "/api/tasks/task_0001/advance?workspace_id=workspace_0001",
      "/api/tasks/task_0001/stop?workspace_id=workspace_0001",
    ]);
  });
});
