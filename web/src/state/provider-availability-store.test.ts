import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ProviderHealthEntry,
  ProviderHealthResponse,
} from "../api/types";
import { getProviderStatus, recheckProviders } from "../api/client";
import { useProviderAvailabilityStore } from "./provider-availability-store";

vi.mock("../api/client", () => ({
  getProviderStatus: vi.fn(),
  recheckProviders: vi.fn(),
}));

const getProviderStatusMock = vi.mocked(getProviderStatus);
const recheckProvidersMock = vi.mocked(recheckProviders);

function entry(
  provider: "claude_code" | "codex",
  available: boolean,
): ProviderHealthEntry {
  return {
    provider,
    display_name: provider === "claude_code" ? "Claude Code" : "Codex",
    available,
    version: available ? "1.0.0" : null,
    reason_code: available ? null : "command_missing",
    reason: available ? null : "not found",
    checked_at: "2026-07-14T00:00:00Z",
    install_hint: `Install ${provider}`,
  };
}

function snapshot(
  generation: number,
  overrides: Partial<ProviderHealthResponse> = {},
): ProviderHealthResponse {
  return {
    schema_version: 1,
    generation,
    checked_at: `2026-07-14T00:00:0${generation}Z`,
    state_status: "ready",
    state_error: null,
    real_workflow_blocked: false,
    test_provider_enabled: false,
    providers: [entry("claude_code", true), entry("codex", false)],
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("provider availability store", () => {
  beforeEach(() => {
    useProviderAvailabilityStore.getState().reset();
    vi.resetAllMocks();
  });

  it("starts unknown and fail-closed", () => {
    expect(useProviderAvailabilityStore.getState()).toMatchObject({
      snapshot: null,
      loadStatus: "idle",
      recheckStatus: "idle",
      error: null,
      generation: null,
      stateStatus: null,
      stateError: null,
      realWorkflowBlocked: true,
      testProviderEnabled: false,
      providers: {},
    });
  });

  it("loads one available real provider into the indexed state", async () => {
    getProviderStatusMock.mockResolvedValue(snapshot(1));

    await useProviderAvailabilityStore.getState().load();

    expect(getProviderStatusMock).toHaveBeenCalledTimes(1);
    expect(useProviderAvailabilityStore.getState()).toMatchObject({
      loadStatus: "loaded",
      error: null,
      generation: 1,
      stateStatus: "ready",
      realWorkflowBlocked: false,
      testProviderEnabled: false,
    });
    expect(useProviderAvailabilityStore.getState().providers.claude_code?.available).toBe(
      true,
    );
    expect(useProviderAvailabilityStore.getState().providers.codex?.available).toBe(false);
  });

  it("treats all unavailable and degraded HTTP 200 responses as snapshots, not errors", async () => {
    getProviderStatusMock
      .mockResolvedValueOnce(
        snapshot(1, {
          providers: [entry("claude_code", false), entry("codex", false)],
          real_workflow_blocked: true,
        }),
      )
      .mockResolvedValueOnce(
        snapshot(2, {
          state_status: "degraded",
          state_error: "failed to persist provider health snapshot",
          real_workflow_blocked: true,
          providers: [entry("claude_code", true), entry("codex", true)],
        }),
      );

    await useProviderAvailabilityStore.getState().load();
    expect(useProviderAvailabilityStore.getState()).toMatchObject({
      generation: 1,
      error: null,
      realWorkflowBlocked: true,
    });

    await useProviderAvailabilityStore.getState().load();
    expect(useProviderAvailabilityStore.getState()).toMatchObject({
      generation: 2,
      stateStatus: "degraded",
      stateError: "failed to persist provider health snapshot",
      error: null,
      realWorkflowBlocked: true,
    });
  });

  it("keeps the initial state fail-closed when the first load fails", async () => {
    getProviderStatusMock.mockRejectedValue(new Error("network offline"));

    await useProviderAvailabilityStore.getState().load();

    expect(useProviderAvailabilityStore.getState()).toMatchObject({
      snapshot: null,
      loadStatus: "error",
      error: "network offline",
      generation: null,
      realWorkflowBlocked: true,
    });
  });

  it("does not let a slow GET overwrite a newer recheck generation", async () => {
    const slowLoad = deferred<ProviderHealthResponse>();
    getProviderStatusMock.mockReturnValue(slowLoad.promise);
    recheckProvidersMock.mockResolvedValue(
      snapshot(2, {
        providers: [entry("claude_code", false), entry("codex", true)],
      }),
    );

    const loadPromise = useProviderAvailabilityStore.getState().load();
    await useProviderAvailabilityStore.getState().recheck();
    slowLoad.resolve(snapshot(1));
    await loadPromise;

    expect(useProviderAvailabilityStore.getState().generation).toBe(2);
    expect(useProviderAvailabilityStore.getState().providers.codex?.available).toBe(true);
    expect(useProviderAvailabilityStore.getState().snapshot?.checked_at).toBe(
      "2026-07-14T00:00:02Z",
    );
  });

  it("ignores an older recheck response without rolling back any snapshot fields", async () => {
    getProviderStatusMock.mockResolvedValue(
      snapshot(5, {
        state_status: "degraded",
        state_error: "newer state",
        real_workflow_blocked: true,
        test_provider_enabled: true,
      }),
    );
    recheckProvidersMock.mockResolvedValue(snapshot(4));
    await useProviderAvailabilityStore.getState().load();

    await useProviderAvailabilityStore.getState().recheck();

    expect(useProviderAvailabilityStore.getState()).toMatchObject({
      generation: 5,
      stateStatus: "degraded",
      stateError: "newer state",
      realWorkflowBlocked: true,
      testProviderEnabled: true,
      recheckStatus: "idle",
      error: null,
    });
  });

  it("reuses one in-flight recheck promise and sends one POST", async () => {
    const pending = deferred<ProviderHealthResponse>();
    recheckProvidersMock.mockReturnValue(pending.promise);

    const first = useProviderAvailabilityStore.getState().recheck();
    const second = useProviderAvailabilityStore.getState().recheck();

    expect(second).toBe(first);
    expect(recheckProvidersMock).toHaveBeenCalledTimes(1);
    expect(useProviderAvailabilityStore.getState().recheckStatus).toBe("rechecking");

    pending.resolve(snapshot(1));
    await first;
    expect(useProviderAvailabilityStore.getState().recheckStatus).toBe("idle");
  });

  it("preserves the last snapshot on recheck failure and permits a later retry", async () => {
    getProviderStatusMock.mockResolvedValue(snapshot(1));
    recheckProvidersMock
      .mockRejectedValueOnce(new Error("temporary failure"))
      .mockResolvedValueOnce(snapshot(2));
    await useProviderAvailabilityStore.getState().load();

    await useProviderAvailabilityStore.getState().recheck();
    expect(useProviderAvailabilityStore.getState()).toMatchObject({
      generation: 1,
      recheckStatus: "error",
      error: "temporary failure",
    });

    await useProviderAvailabilityStore.getState().recheck();
    expect(recheckProvidersMock).toHaveBeenCalledTimes(2);
    expect(useProviderAvailabilityStore.getState()).toMatchObject({
      generation: 2,
      recheckStatus: "idle",
      error: null,
    });
  });

  it("reset clears state and the module-level recheck reference", async () => {
    const firstRequest = deferred<ProviderHealthResponse>();
    recheckProvidersMock
      .mockReturnValueOnce(firstRequest.promise)
      .mockResolvedValueOnce(snapshot(2));

    const abandoned = useProviderAvailabilityStore.getState().recheck();
    useProviderAvailabilityStore.getState().reset();
    await useProviderAvailabilityStore.getState().recheck();

    expect(recheckProvidersMock).toHaveBeenCalledTimes(2);
    expect(useProviderAvailabilityStore.getState().generation).toBe(2);

    firstRequest.resolve(snapshot(1));
    await abandoned;
    expect(useProviderAvailabilityStore.getState().generation).toBe(2);
  });
});
