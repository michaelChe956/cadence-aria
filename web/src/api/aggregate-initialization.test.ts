import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiRequestError } from "./client";
import {
  cancelAggregateInitialization,
  getAggregateInitialization,
  startAggregateInitialization,
} from "./aggregate-initialization";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status });
}

function operationResponse() {
  return {
    operation_id: "aggregate_initialization_0001",
    project_id: "project_0001",
    status: "running",
    profile: null,
    steps: [
      { step_id: "machine_skills", status: "completed" },
      { step_id: "aggregate_preflight", status: "completed" },
      { step_id: "pre_check", status: "running" },
      { step_id: "rule_and_mcp_config", status: "pending" },
      { step_id: "openspec_and_examples", status: "pending" },
    ],
    current_step: "pre_check",
    failed_step: null,
    member_projections: [],
    cancellation: null,
    error: null,
    created_at: "2026-08-18T00:00:00Z",
    updated_at: "2026-08-18T00:00:01Z",
    completed_at: null,
  };
}

describe("aggregate initialization api client", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("starts aggregate initialization with an encoded project id and idempotency key", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse(operationResponse());
      }),
    );

    await startAggregateInitialization(
      "project/with space",
      "lc-0001",
      "key-1",
    );

    expect(calls[0].input).toBe(
      "/api/projects/project%2Fwith%20space/logical-codebases/lc-0001/initializations",
    );
    expect(calls[0].init?.method).toBe("POST");
    expect(calls[0].init?.body).toBe(JSON.stringify({ idempotency_key: "key-1" }));
  });

  it("gets an aggregate initialization with encoded ids", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse({ ...operationResponse(), status: "completed" });
      }),
    );

    const operation = await getAggregateInitialization(
      "project_0001",
      "lc-0001",
      "aggregate_initialization/1",
    );

    expect(calls[0].input).toBe(
      "/api/projects/project_0001/logical-codebases/lc-0001/initializations/aggregate_initialization%2F1",
    );
    expect(calls[0].init?.method).toBeUndefined();
    expect(operation.status).toBe("completed");
  });

  it("cancels an aggregate initialization with reason and detail", async () => {
    const calls: Array<{ input: string; init?: RequestInit }> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        calls.push({ input: String(input), init });
        return jsonResponse({
          ...operationResponse(),
          status: "cancelled",
          cancellation: {
            reason_code: "user_cancelled",
            cancelled_at: "2026-08-18T00:01:00Z",
            detail: "operator requested stop",
          },
        });
      }),
    );

    const operation = await cancelAggregateInitialization(
      "project_0001",
      "lc-0001",
      "aggregate_initialization_0001",
      { reason: "user_cancelled", detail: "operator requested stop" },
    );

    expect(calls[0].input).toBe(
      "/api/projects/project_0001/logical-codebases/lc-0001/initializations/aggregate_initialization_0001/cancel",
    );
    expect(calls[0].init?.method).toBe("POST");
    expect(calls[0].init?.body).toBe(
      JSON.stringify({
        reason: "user_cancelled",
        detail: "operator requested stop",
      }),
    );
    expect(operation.status).toBe("cancelled");
  });

  it("normalizes failed requests as ApiRequestError", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        jsonResponse(
          {
            code: "aggregate_initialization_in_progress",
            message: "aggregate initialization is already in progress",
          },
          409,
        ),
      ),
    );

    await expect(
      startAggregateInitialization("project_0001", "lc-0001", "key-1"),
    ).rejects.toMatchObject({
      name: "ApiRequestError",
      message: "aggregate initialization is already in progress",
    } satisfies Partial<ApiRequestError>);
  });
});
