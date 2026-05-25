import { describe, expect, it, vi } from "vitest";
import {
  createProductIssue,
  createProject,
  createRepository,
  deleteDesignSpec,
  deleteProductIssue,
  deleteProject,
  deleteRepository,
  deleteStorySpec,
  deleteWorkItem,
  generateDesignSpecs,
  generateStorySpecs,
  generateWorkItems,
  getIssueLifecycle,
  listProductIssues,
  listProjects,
  listRepositories,
  normalizeApiError,
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
      design_kind: "frontend",
    });
    await generateWorkItems("project/with space", "issue/with space", {
      title: "Work",
      story_spec_ids: ["story_0001"],
      design_spec_ids: ["design_0001"],
    });

    expect(calls.map((call) => call.input)).toEqual([
      "/api/issues/issue%2Fwith%20space/lifecycle?project_id=project%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/story-specs:generate",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/design-specs:generate",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/work-items:generate",
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

    expect(calls.map((call) => call.input)).toEqual([
      "/api/projects/project%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/repositories/repository%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/story-specs/story%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/design-specs/design%2Fwith%20space",
      "/api/projects/project%2Fwith%20space/issues/issue%2Fwith%20space/work-items/work%2Fwith%20space",
    ]);
    expect(calls.every((call) => call.init?.method === "DELETE")).toBe(true);
  });
});
