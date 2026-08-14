import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createPointerPublication,
  getPointerPublication,
  listPointerPublications,
  retryPointerPublicationRepo,
  revokePointerPublication,
} from "./pointer-publication";

const PROJECT_ID = "project_0001";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function publicationResponse() {
  return {
    id: "pub-0001",
    project_id: PROJECT_ID,
    logical_codebase_id: "logical-0001",
    batch_kind: "full",
    entries: [],
    status: "completed_all",
    created_at: "2026-08-14T00:00:00Z",
    updated_at: "2026-08-14T00:00:01Z",
  };
}

describe("pointer publication api client", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("lists pointer publications for a project", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse([publicationResponse()]);
      }),
    );

    const result = await listPointerPublications("project/with space");

    expect(calls[0].input).toBe(
      "/api/projects/project%2Fwith%20space/logical-codebase/pointer-publications",
    );
    expect(calls[0].init?.method).toBeUndefined();
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("pub-0001");
  });

  it("gets a single pointer publication with encoded ids", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse(publicationResponse());
      }),
    );

    await getPointerPublication(PROJECT_ID, "pub/1");

    expect(calls[0].input).toBe(
      "/api/projects/project_0001/logical-codebase/pointer-publications/pub%2F1",
    );
    expect(calls[0].init?.method).toBeUndefined();
  });

  it("creates a full pointer publication", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse(publicationResponse());
      }),
    );

    await createPointerPublication(PROJECT_ID, "full");

    expect(calls[0].input).toBe(
      "/api/projects/project_0001/logical-codebase/pointer-publications",
    );
    expect(calls[0].init?.method).toBe("POST");
    expect(calls[0].init?.body).toBe(JSON.stringify({ batch_kind: "full" }));
  });

  it("retries a member repo inside a publication", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse(publicationResponse());
      }),
    );

    await retryPointerPublicationRepo(PROJECT_ID, "pub-0001", "repo/1");

    expect(calls[0].input).toBe(
      "/api/projects/project_0001/logical-codebase/pointer-publications/pub-0001/retry-repo",
    );
    expect(calls[0].init?.method).toBe("POST");
    expect(calls[0].init?.body).toBe(JSON.stringify({ member_repo_id: "repo/1" }));
  });

  it("revokes a pointer publication", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse({ ...publicationResponse(), status: "revoked" });
      }),
    );

    const result = await revokePointerPublication(PROJECT_ID, "pub-0001");

    expect(calls[0].input).toBe(
      "/api/projects/project_0001/logical-codebase/pointer-publications/pub-0001/revoke",
    );
    expect(calls[0].init?.method).toBe("POST");
    expect(calls[0].init?.body).toBe(JSON.stringify({}));
    expect(result.status).toBe("revoked");
  });
});
