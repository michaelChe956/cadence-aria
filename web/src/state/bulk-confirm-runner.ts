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
  message?: unknown;
  context?: unknown;
}

/** T5.1/5.2（REQ-REG-15/D4）：唯一透传 message/context 的 waiting code——
 * 其它 protocol error 维持 detail=code。 */
const TRUST_WAITING_CODE = "workspace_confirmation_trust_waiting";

/** 确认 trust 等待的 detail 组装：同源 message 优先（后端 message 已内嵌
 * 原因/人工处置/门未关闭/重新确认短语）；message 未含的 retry_action/
 * manual_guidance 从 context 补齐并去重；无任何可读内容时回落稳定 code。
 * 超时/断连不经过本函数（结果未知，不伪标 waiting）。 */
function trustWaitingDetail(frame: ParsedFrame): string | null {
  if (frame.code !== TRUST_WAITING_CODE) {
    return null;
  }
  const parts: string[] = [];
  if (typeof frame.message === "string" && frame.message !== "") {
    parts.push(frame.message);
  }
  const context =
    typeof frame.context === "object" && frame.context !== null
      ? (frame.context as Record<string, unknown>)
      : null;
  for (const key of ["retry_action", "manual_guidance"]) {
    const value = context?.[key];
    if (typeof value !== "string" || value === "") {
      continue;
    }
    if (!parts.some((part) => part.includes(value))) {
      parts.push(value);
    }
  }
  return parts.length > 0 ? parts.join(" ") : TRUST_WAITING_CODE;
}

/**
 * 单目标会话确认协议（每会话恰好一条连接、恰好一次发送、零重试）：
 * 1. open 后发 hello（role:"driver"→服务端 bind_role 显式接管会话 lease，epoch+1）；
 * 2. 等首帧 session_state（attach 就绪）；等不到即 failed 且零发送；
 * 3. 发 typed `{type:"confirm"}` 并记审计 sent 行（detail:"bulk"）；
 * 4. human_gate_closed→confirmed+markCompleted（前置：auditRecordId !== null，即本连接
 *    confirm 已发出；早到关门帧=他方并发关门，按 failed(gate_closed_before_confirm) 如实呈现）；
 *    protocol_error|error→rejected+markRejected(code)——T5.2 仅 trust waiting code
 *    （workspace_confirmation_trust_waiting）以 message/context 组装 detail 展示指导，
 *    其它 detail=code；关闭/错误/超时→failed（detail 如实，超时=结果未知不伪标 waiting）；
 * 5. 恒 close()（服务端 revoke_if_holder 撤 lease，run 不受影响——P2 语义）。
 */
export function confirmGateOnSession(
  target: BulkConfirmTarget,
  socketFactory: BulkSocketFactory,
  options: { timeoutMs?: number } = {},
): Promise<BulkConfirmItemOutcome> {
  const timeoutMs = options.timeoutMs ?? BULK_CONFIRM_TIMEOUT_MS;
  return new Promise((resolve) => {
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
          socket?.send(JSON.stringify({ type: "confirm" }));
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
          // T5.2：仅 trust waiting code 透传 message/context 组装 detail；
          // 审计仍存稳定 code（操作语义不随展示变化）。
          const waitingDetail = trustWaitingDetail(frame);
          if (auditRecordId !== null) {
            useOperationAuditStore.getState().markRejected(auditRecordId, code);
          }
          finish("rejected", waitingDetail ?? code);
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
  });
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
