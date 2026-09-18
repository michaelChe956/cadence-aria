import { workspaceSessionWebSocketUrl } from "../api/client";
import { useOperationAuditStore } from "./operation-audit-store";

/** 并发上限：同时存活的短命 driver 连接至多 3 条（REQ-CFC-06 批量面如实标注值）。 */
export const BULK_CONFIRM_CONCURRENCY = 3;
/** 单目标全协议超时（hello→confirm→结果帧）。超时后审计行保持 sent（已发送未确认）。 */
export const BULK_CONFIRM_TIMEOUT_MS = 8_000;

export interface BulkConfirmTarget {
  itemId: string;
  sessionId: string;
  gateKey: string;
  title: string;
}

export type BulkConfirmItemStatus = "pending" | "confirmed" | "rejected" | "failed";

export interface BulkConfirmItemOutcome {
  itemId: string;
  sessionId: string;
  gateKey: string;
  title: string;
  status: BulkConfirmItemStatus;
  detail: string | null;
  atIso: string;
}

export interface BulkSocket {
  send(data: string): void;
  close(): void;
}

export interface BulkSocketHandlers {
  onOpen(): void;
  onMessage(data: string): void;
  onClose(): void;
  onError(): void;
}

export type BulkSocketFactory = (sessionId: string, handlers: BulkSocketHandlers) => BulkSocket;

export function createBulkSocketFactory(
  urlFor: (sessionId: string) => string = workspaceSessionWebSocketUrl,
): BulkSocketFactory {
  return (sessionId, handlers) => {
    const socket = new WebSocket(urlFor(sessionId));
    socket.onopen = () => handlers.onOpen();
    socket.onmessage = (event) => handlers.onMessage(String(event.data));
    socket.onclose = () => handlers.onClose();
    socket.onerror = () => handlers.onError();
    return {
      send: (data) => socket.send(data),
      close: () => socket.close(),
    };
  };
}

interface ParsedFrame {
  type?: unknown;
  code?: unknown;
}

/**
 * 单目标会话确认协议（每会话恰好一条连接、恰好一次发送、零重试）：
 * 1. open 后发 hello（role:"driver"→服务端 bind_role 显式接管会话 lease，epoch+1）；
 * 2. 等首帧 session_state（attach 就绪）；等不到即 failed 且零发送；
 * 3. 发 human_confirm{decision:"confirm"} 并记审计 sent 行（detail:"bulk"）；
 * 4. human_gate_closed→confirmed+markCompleted（前置：auditRecordId !== null，即本连接
 *    confirm 已发出；早到关门帧=他方并发关门，按 failed(gate_closed_before_confirm) 如实呈现）；
 *    protocol_error|error→rejected+markRejected(code)；关闭/错误/超时→failed（detail 如实）；
 * 5. 恒 close()（服务端 revoke_if_holder 撤 lease，run 不受影响——P2 语义）。
 */
export function confirmGateOnSession(
  target: BulkConfirmTarget,
  socketFactory: BulkSocketFactory,
  options: { timeoutMs?: number } = {},
): Promise<BulkConfirmItemOutcome> {
  const timeoutMs = options.timeoutMs ?? BULK_CONFIRM_TIMEOUT_MS;
  const { promise, resolve } = Promise.withResolvers<BulkConfirmItemOutcome>();
  let settled = false;
  let sawSessionState = false;
  let auditRecordId: string | null = null;
  let socket: BulkSocket | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const base = {
    itemId: target.itemId,
    sessionId: target.sessionId,
    gateKey: target.gateKey,
    title: target.title,
    atIso: new Date().toISOString(),
  };
  const finish = (status: BulkConfirmItemStatus, detail: string | null) => {
    if (settled) {
      return;
    }
    settled = true;
    if (timer !== null) {
      clearTimeout(timer);
    }
    try {
      socket?.close();
    } catch {
      // 已关闭
    }
    resolve({ ...base, status, detail });
  };
  socket = socketFactory(target.sessionId, {
    onOpen: () => {
      socket?.send(
        JSON.stringify({
          type: "hello",
          session_id: target.sessionId,
          last_seen_node_id: null,
          role: "driver",
        }),
      );
    },
    onMessage: (data) => {
      let frame: ParsedFrame;
      try {
        frame = JSON.parse(data) as ParsedFrame;
      } catch {
        return; // 畸形帧不影响协议（observer 同款纪律）
      }
      if (frame.type === "session_state" && !sawSessionState) {
        sawSessionState = true;
        socket?.send(JSON.stringify({ type: "human_confirm", decision: "confirm", payload: null }));
        auditRecordId = useOperationAuditStore.getState().record({
          sessionId: target.sessionId,
          gateId: target.gateKey,
          operation: "confirm",
          source: "chat",
          outcome: "sent",
          detail: "bulk",
        });
        return;
      }
      if (frame.type === "human_gate_closed") {
        if (auditRecordId === null) {
          // 他方并发关门窗口：confirm 尚未发出——不误报 confirmed、不虚构审计行
          finish("failed", "gate_closed_before_confirm");
          return;
        }
        useOperationAuditStore.getState().markCompleted(auditRecordId);
        finish("confirmed", null);
        return;
      }
      if (frame.type === "protocol_error" || frame.type === "error") {
        const code = frame.type === "protocol_error" && typeof frame.code === "string"
          ? frame.code
          : "server_error";
        if (auditRecordId !== null) {
          useOperationAuditStore.getState().markRejected(auditRecordId, code);
        }
        finish("rejected", code);
        return;
      }
    },
    onClose: () => {
      finish(
        "failed",
        sawSessionState ? "connection_closed_before_result" : "connection_closed_before_session_state",
      );
    },
    onError: () => {
      finish("failed", "socket_error");
    },
  });
  timer = setTimeout(() => {
    finish("failed", `timeout_after_${timeoutMs}ms`);
  }, timeoutMs);
  return promise;
}

/**
 * 批量入口：按 sessionId 去重（恰一次的防御面）后以有界并发执行；
 * onSettled 对每条结果即时回调（供 store/UI 增量呈现），返回全部结果。
 */
export async function runBulkConfirm(
  targets: readonly BulkConfirmTarget[],
  socketFactory: BulkSocketFactory,
  onSettled: (outcome: BulkConfirmItemOutcome) => void = () => undefined,
  options: { concurrency?: number } = {},
): Promise<BulkConfirmItemOutcome[]> {
  const concurrency = options.concurrency ?? BULK_CONFIRM_CONCURRENCY;
  const seen = new Set<string>();
  const queue = targets.filter((target) => {
    if (seen.has(target.sessionId)) {
      return false;
    }
    seen.add(target.sessionId);
    return true;
  });
  const results: BulkConfirmItemOutcome[] = [];
  const workers = Array.from({ length: Math.min(Math.max(concurrency, 1), queue.length) }, async () => {
    for (let target = queue.shift(); target !== undefined; target = queue.shift()) {
      const outcome = await confirmGateOnSession(target, socketFactory);
      results.push(outcome);
      onSettled(outcome);
    }
  });
  await Promise.all(workers);
  return results;
}
