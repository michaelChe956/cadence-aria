import { describe, expect, it, vi } from "vitest";
import {
  ApiRequestError,
  createProductIssue,
  createProject,
  createRepository,
  deleteDesignSpec,
  deleteProductIssue,
  deleteProject,
  deleteRepository,
  deleteStorySpec,
  deleteWorkItem,
  deleteWorkItemPlan,
  deleteCodingAttempt,
  generateDesignSpecs,
  generateStorySpecs,
  getProviderStatus,
  getIssueLifecycle,
  prepareWorkItemPlan,
  listProductIssues,
  listProjects,
  listRepositories,
  normalizeApiError,
  recheckProviders,
} from "./client";

describe("api client", () => {
  it("normalizes standard api error", async () => {
    const error = await normalizeApiError(
      new Response(
        JSON.stringify({
          code: "provider_execution_failed",
          message: "provider command timed out",
          details: {},
        }),
        { status: 500 },
      ),
    );
    expect(error.code).toBe("provider_execution_failed");
    expect(error.message).toBe("provider command timed out");
  });

  it("falls back for non-object error details", async () => {
    const error = await normalizeApiError(
      new Response(
        JSON.stringify({
          code: "invalid_details",
          message: "details is not an object",
          details: "do not coerce this value",
        }),
        { status: 500 },
      ),
    );

    expect(error.details).toEqual({});
  });

  it("gets provider status and rechecks with the accepted health envelope", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    const degraded = {
      schema_version: 1,
      generation: 7,
      checked_at: "2026-07-14T00:00:00Z",
      state_status: "degraded",
      state_error: "provider health state is degraded",
      real_workflow_blocked: true,
      test_provider_enabled: false,
      providers: [
        {
          provider: "claude_code",
          display_name: "Claude Code",
          available: false,
          version: null,
          reason_code: "command_missing",
          reason: "not found",
          checked_at: "2026-07-14T00:00:00Z",
          install_hint: "Install Claude Code CLI.",
        },
        {
          provider: "codex",
          display_name: "Codex",
          available: false,
          version: null,
          reason_code: "command_missing",
          reason: "not found",
          checked_at: "2026-07-14T00:00:00Z",
          install_hint: "Install Codex CLI.",
        },
      ],
    } as const;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return new Response(JSON.stringify(degraded), { status: 200 });
      }),
    );

    await expect(getProviderStatus()).resolves.toEqual(degraded);
    await expect(recheckProviders()).resolves.toEqual(degraded);

    expect(calls).toHaveLength(2);
    expect(calls[0]).toMatchObject({ input: "/api/providers/status" });
    expect(calls[0].init?.method).toBeUndefined();
    expect(calls[1]).toMatchObject({ input: "/api/providers/recheck" });
    expect(calls[1].init?.method).toBe("POST");
  });

  it("returns the complete repository initialization envelope from HTTP 201", async () => {
    const response = {
      repository: {
        repository_id: "repository_0001",
        project_id: "project_0001",
        name: "Aria",
        path: "/work/aria",
        repo_hash: "repo-hash",
        runtime_root: "/work/aria/.aria",
        default_policy_preset: "balanced",
        default_provider_mode: "claude_code",
        created_at: "2026-07-14T00:00:00Z",
        updated_at: "2026-07-14T00:00:00Z",
      },
      initialization: {
        source: "offline",
        commands: [
          { index: 1, command: "/pre-check", status: "completed" },
          { index: 2, command: "/rule-config", status: "completed" },
          { index: 3, command: "/mcp-configuration", status: "completed" },
          {
            index: 4,
            command: "/project-rules-examples",
            status: "completed",
          },
        ],
        warnings: ["cadence_skills_conflict:<path>"],
        changed_paths: [".claude/rules/project.md"],
        completed_at: "2026-07-14T00:01:00Z",
      },
    } as const;
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response(JSON.stringify(response), { status: 201 }),
      ),
    );

    await expect(
      createRepository("project_0001", {
        name: "Aria",
        path: "/work/aria",
        default_policy_preset: "balanced",
        default_provider_mode: "claude_code",
      }),
    ).resolves.toEqual(response);
  });

  it("preserves structured repository registration error details", async () => {
    const details = {
      stage: "repository_init_command",
      provider: "claude_code",
      command: "/rule-config",
      reason_code: "repository_init_command_failed",
      stderr_summary: null,
      changed_paths: [".claude/rules/project.md"],
      retryable: true,
      action: "Fix the problem, inspect changed_paths, then retry.",
    } as const;
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response(
          JSON.stringify({
            code: "repository_init_command_failed",
            message: "repository registration failed",
            details,
          }),
          { status: 500 },
        ),
      ),
    );

    try {
      await createRepository("project_0001", {
        name: "Aria",
        path: "/work/aria",
      });
      throw new Error("expected createRepository to reject");
    } catch (error) {
      expect(error).toBeInstanceOf(ApiRequestError);
      expect((error as ApiRequestError).details).toEqual(details);
      expect((error as ApiRequestError).details.changed_paths).toEqual([
        ".claude/rules/project.md",
      ]);
      expect((error as ApiRequestError).details.retryable).toBe(true);
      expect((error as ApiRequestError).details.stderr_summary).toBeNull();
    }
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
        return new Response(JSON.stringify(responses[calls.length - 1]), {
          status: 200,
        });
      }),
    );

    await listProjects();
    await createProject({ name: "Aria", description: null });

    expect(calls.map((call) => call.input)).toEqual([
      "/api/projects",
      "/api/projects",
    ]);
    expect(calls[1].init?.method).toBe("POST");
    expect(calls[1].init?.body).toBe(
      JSON.stringify({ name: "Aria", description: null }),
    );
  });

  it("calls product repository and issue endpoints with project scoped payloads", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return new Response(
          JSON.stringify({ ok: true, repositories: [], issues: [] }),
          {
            status: 200,
          },
        );
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

    expect(calls.map((call) => call.input)).toEqual([
      "/api/projects/project%2Fwith%20space/repositories",
      "/api/projects/project%2Fwith%20space/repositories",
      "/api/projects/project%2Fwith%20space/issues",
      "/api/projects/project%2Fwith%20space/issues",
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
  });

  it("calls lifecycle generation endpoints with encoded ids", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return new Response(JSON.stringify({ ok: true }), { status: 200 });
      }),
    );

    await getIssueLifecycle("issue/with space", "project/with space");
    await generateStorySpecs("project/with space", "issue/with space", {
      title: "Story",
    });
    await generateDesignSpecs("project/with space", "issue/with space", {
      title: "Design",
      story_spec_ids: ["story_0001"],
    });
    await prepareWorkItemPlan("project/with space", "issue/with space", {
      title: "Work",
      story_spec_ids: ["story_0001"],
      design_spec_ids: ["design_0001"],
    });

    expect(calls.map((call) => call.input)).toEqual([
      "/api/issues/issue%2Fwith%20space/lifecycle?project_id=project%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/story-specs:generate",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/design-specs:generate",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/work-item-plans:prepare",
    ]);
    expect(calls.slice(1).every((call) => call.init?.method === "POST")).toBe(
      true,
    );
  });

  it("calls delete endpoints with encoded resource ids", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return new Response(JSON.stringify({ status: "deleted" }), {
          status: 200,
        });
      }),
    );

    await deleteProject("project/with space");
    await deleteRepository("project/with space", "repository/with space");
    await deleteProductIssue("project/with space", "issue/with space");
    await deleteStorySpec(
      "project/with space",
      "issue/with space",
      "story/with space",
    );
    await deleteDesignSpec(
      "project/with space",
      "issue/with space",
      "design/with space",
    );
    await deleteWorkItem(
      "project/with space",
      "issue/with space",
      "work/with space",
    );
    await deleteWorkItemPlan(
      "project/with space",
      "issue/with space",
      "plan/with space",
    );
    await deleteCodingAttempt("attempt/with space");

    expect(calls.map((call) => call.input)).toEqual([
      "/api/projects/project%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/repositories/repository%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/story-specs/story%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/design-specs/design%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/work-items/work%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/work-item-plans/plan%2Fwith%20space",
      "/api/coding-attempts/attempt%2Fwith%20space",
    ]);
    expect(calls.every((call) => call.init?.method === "DELETE")).toBe(true);
  });

  it("handles delete coding attempt 204 response without parsing json", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(null, { status: 204 })),
    );

    await expect(
      deleteCodingAttempt("coding_attempt_0001"),
    ).resolves.toBeUndefined();
  });
});
