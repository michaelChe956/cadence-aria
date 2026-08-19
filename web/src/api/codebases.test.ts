import { afterEach, describe, expect, it, vi } from "vitest";
import { createLogicalCodebase, listCodebases } from "./codebases";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

describe("codebases api", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("lists the mixed codebases via GET /codebases", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse({
          codebases: [
            {
              id: "repository_0001",
              name: "aria",
              kind: "single_repo",
              repository_id: "repository_0001",
              logical_codebase_id: null,
              member_count: null,
            },
            {
              id: "lc_0001",
              name: "monorepo",
              kind: "logical",
              repository_id: null,
              logical_codebase_id: "lc_0001",
              member_count: 3,
            },
          ],
        });
      }),
    );

    const response = await listCodebases("project/with space");

    expect(calls.map(({ input }) => input)).toEqual([
      "/api/projects/project%2Fwith%20space/codebases",
    ]);
    expect(response.codebases).toHaveLength(2);
    expect(response.codebases[1]).toMatchObject({
      kind: "logical",
      member_count: 3,
    });
  });

  it("creates a logical codebase via POST /logical-codebases", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse({
          id: "lc_0001",
          name: "monorepo",
          aggregate_root: "/repos/monorepo",
          created_at: "2026-08-19T00:00:00Z",
        });
      }),
    );

    const codebase = await createLogicalCodebase("project_0001", {
      name: "monorepo",
      aggregate_root: "/repos/monorepo",
    });

    expect(calls.map(({ input }) => input)).toEqual([
      "/api/projects/project_0001/logical-codebases",
    ]);
    expect(calls[0].init?.method).toBe("POST");
    expect(JSON.parse(String(calls[0].init?.body))).toEqual({
      name: "monorepo",
      aggregate_root: "/repos/monorepo",
    });
    expect(codebase.id).toBe("lc_0001");
  });

  it("throws ApiRequestError on failure", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        jsonResponse(
          { code: "aggregate_root_required", message: "aggregate_root must not be empty", details: {} },
          422,
        ),
      ),
    );

    await expect(
      createLogicalCodebase("project_0001", {
        name: "x",
        aggregate_root: " ",
      }),
    ).rejects.toMatchObject({ code: "aggregate_root_required" });
  });
});
