import { beforeEach, describe, expect, it, vi } from "vitest";
import { useOperationAuditStore } from "./operation-audit-store";

describe("operation audit store", () => {
  beforeEach(() => {
    useOperationAuditStore.getState().reset();
  });

  it("records a successful local command as local browser with client ISO time", () => {
    vi.spyOn(Date, "now").mockReturnValue(1_726_000_000_000);

    useOperationAuditStore.getState().record({
      sessionId: "s1",
      gateId: "turn-1",
      operation: "confirm",
      source: "chat",
      outcome: "sent",
      detail: null,
    });

    expect(useOperationAuditStore.getState().records).toEqual([
      expect.objectContaining({
        operator: "本地浏览器",
        atMs: 1_726_000_000_000,
        atIso: new Date(1_726_000_000_000).toISOString(),
      }),
    ]);
  });

  it("keeps only the latest one hundred local records", () => {
    vi.spyOn(Date, "now").mockReturnValue(1_726_000_000_000);
    const store = useOperationAuditStore.getState();

    for (let index = 0; index < 101; index += 1) {
      store.record({
        sessionId: "s1",
        gateId: "turn-1",
        operation: "confirm",
        source: "chat",
        outcome: "sent",
        detail: String(index),
      });
    }

    expect(useOperationAuditStore.getState().records).toHaveLength(100);
    expect(useOperationAuditStore.getState().records[0]?.detail).toBe("1");
  });

  it("marks only the specified local command as rejected", () => {
    const firstId = useOperationAuditStore.getState().record({
      sessionId: "s1",
      gateId: "turn-1",
      operation: "confirm",
      source: "chat",
      outcome: "sent",
      detail: null,
    });
    const secondId = useOperationAuditStore.getState().record({
      sessionId: "s1",
      gateId: "turn-2",
      operation: "advance",
      source: "chat",
      outcome: "sent",
      detail: null,
    });

    useOperationAuditStore.getState().markRejected(firstId, "HUMAN_GATE_NOT_READY");

    expect(useOperationAuditStore.getState().records).toEqual([
      expect.objectContaining({ id: firstId, outcome: "rejected", detail: "HUMAN_GATE_NOT_READY" }),
      expect.objectContaining({ id: secondId, outcome: "sent", detail: null }),
    ]);
  });
});
