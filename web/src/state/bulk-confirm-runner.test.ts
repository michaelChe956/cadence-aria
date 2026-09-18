import { afterEach, describe, expect, it, vi } from "vitest";
import {
  BULK_CONFIRM_CONCURRENCY,
  confirmGateOnSession,
  runBulkConfirm,
  type BulkSocket,
  type BulkSocketHandlers,
} from "./bulk-confirm-runner";
import { useOperationAuditStore } from "./operation-audit-store";

type ScriptedSocket = BulkSocket & {
  sent: string[];
  handlers: BulkSocketHandlers;
  closed: boolean;
};

function scriptedSocketFactory(
  onSocket: (socket: ScriptedSocket, sessionId: string) => void,
) {
  const sockets: ScriptedSocket[] = [];
  const factory = vi.fn((sessionId: string, handlers: BulkSocketHandlers): BulkSocket => {
    const socket: ScriptedSocket = {
      sent: [], handlers, closed: false,
      send: function (data: string) { this.sent.push(data); },
      close: function () { this.closed = true; },
    };
    sockets.push(socket);
    onSocket(socket, sessionId);
    return socket;
  });
  return { factory, sockets };
}

const target = { itemId: "s2:gate:g2", sessionId: "s2", gateKey: "g2", title: "方案定稿" };

afterEach(() => { useOperationAuditStore.getState().reset(); });

describe("confirmGateOnSession", () => {
  it("hello 携带 role:driver；session_state 后恰好发送一次 human_confirm；human_gate_closed → confirmed + close + 审计 completed", async () => {
    const { factory, sockets } = scriptedSocketFactory(() => undefined);
    const promise = confirmGateOnSession(target, factory, { timeoutMs: 1_000 });
    await vi.waitFor(() => expect(factory).toHaveBeenCalledWith("s2", expect.anything()));
    const socket = sockets[0]!;
    socket.handlers.onOpen();
    expect(JSON.parse(socket.sent[0])).toEqual(
      { type: "hello", session_id: "s2", last_seen_node_id: null, role: "driver" });
    socket.handlers.onMessage(JSON.stringify({ type: "session_state", session_id: "s2" }));
    expect(socket.sent.filter((d) => JSON.parse(d).type === "human_confirm")).toHaveLength(1);
    expect(JSON.parse(socket.sent[1])).toEqual(
      { type: "human_confirm", decision: "confirm", payload: null });
    const records = useOperationAuditStore.getState().records;
    expect(records).toHaveLength(1);
    expect(records[0]).toMatchObject({ sessionId: "s2", gateId: "g2", operation: "confirm", outcome: "sent", detail: "bulk" });
    socket.handlers.onMessage(JSON.stringify({ type: "human_gate_closed", decision: "confirm" }));
    await expect(promise).resolves.toMatchObject({ status: "confirmed", detail: null });
    expect(socket.closed).toBe(true);
    expect(useOperationAuditStore.getState().records[0].outcome).toBe("completed");
  });

  it("protocol_error → rejected 且 detail=code，审计 markRejected", async () => {
    const { factory, sockets } = scriptedSocketFactory(() => undefined);
    const promise = confirmGateOnSession(target, factory, { timeoutMs: 1_000 });
    await vi.waitFor(() => expect(factory).toHaveBeenCalled());
    const socket = sockets[0]!;
    socket.handlers.onOpen();
    socket.handlers.onMessage(JSON.stringify({ type: "session_state", session_id: "s2" }));
    socket.handlers.onMessage(JSON.stringify({ type: "protocol_error", code: "INVALID_MESSAGE_FOR_STAGE", message: "x" }));
    await expect(promise).resolves.toMatchObject({ status: "rejected", detail: "INVALID_MESSAGE_FOR_STAGE" });
    expect(useOperationAuditStore.getState().records[0]).toMatchObject({ outcome: "rejected", detail: "INVALID_MESSAGE_FOR_STAGE" });
  });

  it("首帧前连接关闭 → failed 且零发送（不产生 confirm，也不产生审计行）", async () => {
    const { factory, sockets } = scriptedSocketFactory(() => undefined);
    const promise = confirmGateOnSession(target, factory, { timeoutMs: 1_000 });
    await vi.waitFor(() => expect(factory).toHaveBeenCalled());
    const socket = sockets[0]!;
    socket.handlers.onClose();
    await expect(promise).resolves.toMatchObject({ status: "failed", detail: "connection_closed_before_session_state" });
    expect(socket.sent).toHaveLength(0);
    expect(useOperationAuditStore.getState().records).toHaveLength(0);
  });

  it("超时 → failed(timeout)，审计行保持 sent（已发送未确认，不虚构结果）", async () => {
    vi.useFakeTimers();
    try {
      const { factory, sockets } = scriptedSocketFactory(() => undefined);
      const promise = confirmGateOnSession(target, factory, { timeoutMs: 20 });
      await vi.waitFor(() => expect(factory).toHaveBeenCalled());
      const socket = sockets[0]!;
      socket.handlers.onOpen();
      socket.handlers.onMessage(JSON.stringify({ type: "session_state", session_id: "s2" }));
      await vi.advanceTimersByTimeAsync(30);
      await expect(promise).resolves.toMatchObject({ status: "failed", detail: "timeout_after_20ms" });
      expect(useOperationAuditStore.getState().records[0].outcome).toBe("sent");
    } finally { vi.useRealTimers(); }
  });
});

describe("runBulkConfirm", () => {
  it("两会话各恰一次发送（REQ-CFC-06 场景 1 单发面）；sessionId 去重", async () => {
    const { factory, sockets } = scriptedSocketFactory((socket) => {
      // 自动应答脚本：open→session_state→(confirm 到达后)human_gate_closed
      const send = socket.send.bind(socket);
      socket.send = (data) => {
        send(data);
        const frame = JSON.parse(data) as { type: string };
        if (frame.type === "hello") {
          queueMicrotask(() => socket.handlers.onMessage(JSON.stringify({ type: "session_state" })));
        }
        if (frame.type === "human_confirm") {
          queueMicrotask(() => socket.handlers.onMessage(JSON.stringify({ type: "human_gate_closed", decision: "confirm" })));
        }
      };
      queueMicrotask(() => socket.handlers.onOpen());
    });
    const targets = [
      target,
      { ...target, itemId: "s3:gate:g3", sessionId: "s3", gateKey: "g3", title: "T3" },
      { ...target, itemId: "s3:gate:g3-dup", sessionId: "s3", gateKey: "g3-dup", title: "dup" }, // 同会话防御性去重
    ];
    const outcomes = await runBulkConfirm(targets, factory, undefined, { concurrency: 2 });
    expect(outcomes).toHaveLength(2);
    for (const socket of sockets) {
      socket.handlers.onOpen();
      socket.handlers.onMessage(JSON.stringify({ type: "session_state" }));
      socket.handlers.onMessage(JSON.stringify({ type: "human_gate_closed", decision: "confirm" }));
    }
    // 注：脚本驱动的时序见完整实施——核心断言：每个 sessionId 的 human_confirm 恰一次、
    // s3 第二目标被去重不建第二条连接。
    expect(factory).toHaveBeenCalledTimes(2);
    for (const socket of sockets) {
      expect(socket.sent.filter((data) => JSON.parse(data).type === "human_confirm")).toHaveLength(1);
    }
  });

  it("REQ-CFC-06 场景1（自动化面）：两个会话各恰好收到一次 human_confirm，无重复动作", async () => {
    const confirmsBySession = new Map<string, number>();
    const { factory } = scriptedSocketFactory((socket, sessionId) => {
      const send = socket.send.bind(socket);
      socket.send = (data) => {
        send(data);
        const frame = JSON.parse(data) as { type: string };
        if (frame.type === "hello") {
          queueMicrotask(() => socket.handlers.onMessage(
            JSON.stringify({ type: "session_state", session_id: sessionId }),
          ));
        }
        if (frame.type === "human_confirm") {
          confirmsBySession.set(sessionId, (confirmsBySession.get(sessionId) ?? 0) + 1);
          queueMicrotask(() => socket.handlers.onMessage(
            JSON.stringify({ type: "human_gate_closed", decision: "confirm" }),
          ));
        }
      };
      queueMicrotask(() => socket.handlers.onOpen());
    });
    const s3 = { ...target, itemId: "s3:gate:g3", sessionId: "s3", gateKey: "g3", title: "T3" };

    const outcomes = await runBulkConfirm([target, s3, { ...s3, itemId: "s3:gate:g3-dup" }], factory, undefined, { concurrency: 2 });

    expect(outcomes).toHaveLength(2);
    expect(confirmsBySession.get("s2")).toBe(1);
    expect(confirmsBySession.get("s3")).toBe(1);
  });

  it("并发上限：5 目标 concurrency=3 时同时打开的连接至多 3 条", async () => {
    let openCount = 0; let maxOpen = 0;
    const pending: Array<{ sessionId: string; handlers: BulkSocketHandlers }> = [];
    const factory = vi.fn((_sessionId: string, handlers: BulkSocketHandlers): BulkSocket => {
      openCount += 1; maxOpen = Math.max(maxOpen, openCount);
      pending.push({ sessionId: _sessionId, handlers });
      return {
        send: () => undefined,
        close: () => { openCount -= 1; },
        // 不主动回调——挂住连接，仅观察并发峰值
        ...({} as Record<string, never>),
        // handlers 保留在闭包供测试驱动（完整实施里持引用）
      } satisfies BulkSocket as BulkSocket;
    });
    const targets = Array.from({ length: 5 }, (_, i) =>
      ({ itemId: `s${i}:gate:g`, sessionId: `s${i}`, gateKey: "g", title: `t${i}` }));
    const promise = runBulkConfirm(targets, factory, undefined, { concurrency: 3 });
    // 并发峰值断言（runBulkConfirm 未完成也无妨：峰值在调度时即成）
    await vi.waitFor(() => expect(factory).toHaveBeenCalledTimes(3));
    expect(factory.mock.calls.length).toBeLessThanOrEqual(BULK_CONFIRM_CONCURRENCY + 2); // 5 目标 3 并发的管道上限

    for (const entry of pending.slice(0, 3)) {
      entry.handlers.onOpen();
      entry.handlers.onMessage(JSON.stringify({ type: "session_state", session_id: entry.sessionId }));
      entry.handlers.onMessage(JSON.stringify({ type: "human_gate_closed", decision: "confirm" }));
    }
    await vi.waitFor(() => expect(factory).toHaveBeenCalledTimes(5));
    for (const entry of pending.slice(3)) {
      entry.handlers.onOpen();
      entry.handlers.onMessage(JSON.stringify({ type: "session_state", session_id: entry.sessionId }));
      entry.handlers.onMessage(JSON.stringify({ type: "human_gate_closed", decision: "confirm" }));
    }
    await expect(promise).resolves.toHaveLength(5);
    expect(maxOpen).toBeLessThanOrEqual(BULK_CONFIRM_CONCURRENCY);
  });
});
