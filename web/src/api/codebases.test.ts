import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createLogicalCodebase,
  deleteLogicalCodebase,
  listCodebases,
  listLogicalCodebases,
  purgeLogicalCodebase,
  restoreLogicalCodebase,
} from "./codebases";

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

describe("deleteLogicalCodebase", () => {
  it("soft-deletes via DELETE /logical-codebases/{lc_id} with encoded ids", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse({ status: "deleted" });
      }),
    );

    await deleteLogicalCodebase("project/with space", "lc 0001");

    expect(calls).toEqual([
      {
        input: "/api/projects/project%2Fwith%20space/logical-codebases/lc%200001",
        init: expect.objectContaining({ method: "DELETE" }),
      },
    ]);
  });
});

describe("logical codebase lifecycle api（2026-10-29）", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("lists logical codebases with 墓碑 via include_deleted=true", async () => {
    const calls: string[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        calls.push(String(input));
        return jsonResponse({
          logical_codebases: [
            {
              id: "lc_0001",
              name: "monorepo",
              aggregate_root: "/repos/monorepo",
              created_at: "2026-08-19T00:00:00Z",
              deleted: true,
              deleted_at: "2026-10-28T08:00:00Z",
            },
          ],
        });
      }),
    );

    const response = await listLogicalCodebases("project/with space", {
      includeDeleted: true,
    });

    expect(calls).toEqual([
      "/api/projects/project%2Fwith%20space/logical-codebases?include_deleted=true",
    ]);
    expect(response.logical_codebases[0]).toMatchObject({
      deleted: true,
      deleted_at: "2026-10-28T08:00:00Z",
    });

    // 不传参：只取存活 LC（无查询串）。
    calls.length = 0;
    await listLogicalCodebases("project_0001");
    expect(calls).toEqual([
      "/api/projects/project_0001/logical-codebases",
    ]);
  });

  it("restores via POST /restore and purges via DELETE /purge", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse(
          String(input).endsWith("/purge")
            ? { status: "purged" }
            : {
                status: "restored",
                codebase: {
                  id: "lc 0001",
                  name: "monorepo",
                  aggregate_root: "/repos/monorepo",
                  created_at: "2026-08-19T00:00:00Z",
                },
              },
        );
      }),
    );

    const restored = await restoreLogicalCodebase("project_0001", "lc 0001");
    expect(restored.status).toBe("restored");
    expect(restored.codebase.id).toBe("lc 0001");

    const purged = await purgeLogicalCodebase("project_0001", "lc 0001");
    expect(purged.status).toBe("purged");

    expect(calls).toEqual([
      {
        input: "/api/projects/project_0001/logical-codebases/lc%200001/restore",
        init: expect.objectContaining({ method: "POST" }),
      },
      {
        input: "/api/projects/project_0001/logical-codebases/lc%200001/purge",
        init: expect.objectContaining({ method: "DELETE" }),
      },
    ]);
  });
});
