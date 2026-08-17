import { afterEach, describe, expect, it, vi } from "vitest";
import { listLogicalCodebaseMembers } from "./logicalCodebaseMembers";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

describe("logical codebase members api client", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("lists projected members with an encoded project id", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({
        members: [
          {
            logical_repository_id: "repo-0001",
            alias: "api",
            status: "active",
          },
        ],
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const response = await listLogicalCodebaseMembers("project/with space");

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project%2Fwith%20space/logical-codebase/members",
      expect.objectContaining({ headers: { "content-type": "application/json" } }),
    );
    expect(response.members).toEqual([
      {
        logical_repository_id: "repo-0001",
        alias: "api",
        status: "active",
      },
    ]);
  });

  it("preserves an empty response for projects without a manifest", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => jsonResponse({ members: [] })));

    await expect(listLogicalCodebaseMembers("project_0001")).resolves.toEqual({
      members: [],
    });
  });
});
