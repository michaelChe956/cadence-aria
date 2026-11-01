import { afterEach, describe, expect, it, vi } from "vitest";
import {
  cancelLogicalCodebaseRegistration,
  getLogicalCodebaseRegistration,
  preflightLogicalCodebaseRegistration,
  resumeLogicalCodebaseRegistration,
  submitLogicalCodebaseRegistration,
} from "./logical-codebase-registration";
import { ApiRequestError } from "./client";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

const projectId = "project/with space";
const batch = { batch_id: "batch/0001", status: "completed", items: [] };

describe("logical codebase registration api", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("uses the registration DTOs and URL-encodes project and batch ids", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse(
          calls.length === 1
            ? {
                preflight_id: "preflight_0001",
                created_at: "2026-08-18T00:00:00Z",
                items: [],
              }
            : batch,
        );
      }),
    );

    const lcId = "lc/0001";
    await preflightLogicalCodebaseRegistration(projectId, lcId, {
      aggregate_root: "/root",
      candidate_paths: ["/root/api"],
    });
    await submitLogicalCodebaseRegistration(projectId, lcId, {
      aggregate_root: "/root",
      preflight_id: "preflight/0001",
      confirmed_paths: ["/root/api"],
    });
    await getLogicalCodebaseRegistration(projectId, lcId, "batch/0001");
    await resumeLogicalCodebaseRegistration(projectId, lcId, "batch/0001");
    await cancelLogicalCodebaseRegistration(projectId, lcId, "batch/0001");

    expect(calls.map(({ input }) => input)).toEqual([
      "/api/projects/project%2Fwith%20space/logical-codebases/lc%2F0001/registrations/preflight",
      "/api/projects/project%2Fwith%20space/logical-codebases/lc%2F0001/registrations",
      "/api/projects/project%2Fwith%20space/logical-codebases/lc%2F0001/registrations/batch%2F0001",
      "/api/projects/project%2Fwith%20space/logical-codebases/lc%2F0001/registrations/batch%2F0001/resume",
      "/api/projects/project%2Fwith%20space/logical-codebases/lc%2F0001/registrations/batch%2F0001/cancel",
    ]);
    expect(calls[0].init?.method).toBe("POST");
    expect(calls[0].init?.body).toBe(
      JSON.stringify({ aggregate_root: "/root", candidate_paths: ["/root/api"] }),
    );
    expect(calls[1].init?.body).toBe(
      JSON.stringify({
        aggregate_root: "/root",
        preflight_id: "preflight/0001",
        confirmed_paths: ["/root/api"],
      }),
    );
    expect(calls[3].init?.body).toBe(JSON.stringify({}));
    expect(calls[4].init?.body).toBe(JSON.stringify({}));
  });

  it("normalizes API errors using ApiRequestError", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        jsonResponse({ code: "registration_preflight_not_found", message: "预检已过期" }, 404),
      ),
    );

    await expect(
      preflightLogicalCodebaseRegistration("project_0001", "lc_0001", {
        aggregate_root: "/root",
        candidate_paths: [],
      }),
    ).rejects.toMatchObject({
      code: "registration_preflight_not_found",
      message: "预检已过期",
    } satisfies Partial<ApiRequestError>);
  });
});
