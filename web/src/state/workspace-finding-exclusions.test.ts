import { afterEach, describe, expect, it, vi } from "vitest";
import { postRevokeFindingExclusionsAction } from "./workspace-finding-exclusions";

afterEach(() => {
  vi.unstubAllGlobals();
});

function stubFetch(status: number, body: unknown) {
  return vi.fn(
    async (_request: RequestInfo | URL, _init?: RequestInit) =>
      new Response(JSON.stringify(body), {
        status,
        headers: { "content-type": "application/json" },
      }),
  );
}

const revokeAction = {
  type: "revoke_finding_exclusions" as const,
  command_id: "cmd-revoke-1",
  finding_keys: ["aaaaaaaaaaaaaaaa"],
};

describe("postRevokeFindingExclusionsAction (REQ-WSC-13/14 k3 P0-2C)", () => {
  it("POSTs the exact wire shape (no expected_gate_id) and returns remaining keys", async () => {
    const fetchMock = stubFetch(200, {
      command_id: "cmd-revoke-1",
      state: "accepted",
      remaining_finding_keys: ["bbbbbbbbbbbbbbbb"],
    });
    vi.stubGlobal("fetch", fetchMock);

    const outcome = await postRevokeFindingExclusionsAction("wsp_0001", revokeAction);

    expect(outcome).toEqual({
      ok: true,
      result: {
        command_id: "cmd-revoke-1",
        state: "accepted",
        remaining_finding_keys: ["bbbbbbbbbbbbbbbb"],
      },
    });
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("/api/workspace-sessions/wsp_0001/human-actions");
    expect(init.method).toBe("POST");
    // 字段名契约：type/command_id/finding_keys，且不带 expected_gate_id。
    expect(JSON.parse(String(init.body))).toEqual({
      type: "revoke_finding_exclusions",
      command_id: "cmd-revoke-1",
      finding_keys: ["aaaaaaaaaaaaaaaa"],
    });
  });

  it("carries the backend code/message verbatim on 422 human_action_rejected", async () => {
    vi.stubGlobal(
      "fetch",
      stubFetch(422, {
        code: "human_action_rejected",
        message: "disposition ledger unavailable",
      }),
    );

    const outcome = await postRevokeFindingExclusionsAction("wsp_0001", revokeAction);

    expect(outcome).toEqual({
      ok: false,
      code: "human_action_rejected",
      message: "disposition ledger unavailable",
    });
  });

  it("reports network failures without throwing (never-throw contract)", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new TypeError("Failed to fetch");
      }),
    );

    const outcome = await postRevokeFindingExclusionsAction("wsp_0001", revokeAction);

    expect(outcome).toEqual({
      ok: false,
      code: "revoke_finding_exclusions_network",
      message: "Failed to fetch",
    });
  });

  it("treats a 200 with an unrecognized body as failure (does not fake success)", async () => {
    vi.stubGlobal("fetch", stubFetch(200, { command_id: "cmd-revoke-1" }));

    const outcome = await postRevokeFindingExclusionsAction("wsp_0001", revokeAction);

    expect(outcome.ok).toBe(false);
    if (!outcome.ok) {
      expect(outcome.code).toBe("revoke_finding_exclusions_http_200");
    }
  });
});
