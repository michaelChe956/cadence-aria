import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiRequestError } from "./client";
import {
  getActiveAggregateIndex,
  rebuildAggregateIndex,
} from "./aggregate-index";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

describe("aggregate index api client", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("gets the active aggregate index with an encoded project id", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({
        state: "active",
        revision: 7,
        indexed_at: "2026-08-18T00:00:00Z",
        warning: null,
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const index = await getActiveAggregateIndex("project/with space");

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project%2Fwith%20space/logical-codebase/aggregate-indexes/active",
      expect.objectContaining({
        headers: { "content-type": "application/json" },
      }),
    );
    expect(index).toEqual({
      state: "active",
      revision: 7,
      indexed_at: "2026-08-18T00:00:00Z",
      warning: null,
    });
  });

  it("rebuilds the aggregate index and preserves the server state projection", async () => {
    const fetchMock = vi.fn(async () =>
      jsonResponse({
        state: "degraded",
        revision: 6,
        indexed_at: "2026-08-18T00:00:00Z",
        warning: "sync failed",
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const index = await rebuildAggregateIndex("project_0001");

    expect(fetchMock).toHaveBeenCalledWith(
      "/api/projects/project_0001/logical-codebase/aggregate-indexes/rebuild",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({}),
      }),
    );
    expect(index.state).toBe("degraded");
    expect(index.warning).toBe("sync failed");
  });

  it("normalizes failed requests as ApiRequestError", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        jsonResponse(
          { code: "aggregate_index_unavailable", message: "index unavailable" },
          422,
        ),
      ),
    );

    await expect(getActiveAggregateIndex("project_0001")).rejects.toMatchObject(
      {
        name: "ApiRequestError",
        message: "index unavailable",
      } satisfies Partial<ApiRequestError>,
    );
  });
});
