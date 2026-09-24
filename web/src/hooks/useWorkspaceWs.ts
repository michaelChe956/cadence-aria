import { useCallback, useEffect, useRef, useState } from "react";
// L2 退役（T5/REQ-RET-02）：修订路径选择/作者决策/生成模式选择/outline 修订
// 请求/草稿决策/批量决策/review 决策应答的发送器随 wire 消息族删除
// （wp5-attribution-table.md §1）。
import type {
  LinkedWorkspaceAmendmentTarget,
  ProviderConfigSnapshot,
  WorkspaceProviderName,
  WorkItemPlanCompileRecoveryAction,
  WsInMessage,
} from "../api/types";
import { useWorkspaceWsReconnect } from "./useWorkspaceWsReconnect";
import { useLinkedWorkspaceAmendmentStore } from "../state/linked-workspace-amendment-store";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { useOperationAuditStore } from "../state/operation-audit-store";
import { selectGateProjection } from "../state/workspace-cockpit-projection";
import { STALE_DRIVER_LEASE_CODE } from "../state/protocol-error-copy";
import type { ChoiceAnswerPayload } from "../state/chat-entries";
import {
  ACTIVE_PROVIDER_STAGES,
  handleWorkspaceWsMessage,
  providerName,
  type WsServerMessage,
  wsReadyStateName,
} from "./workspace-ws-message-handler";
import {
  aggregatePlanRepairChildMessage,
  type PlanRepairSourceState,
} from "./useWorkspaceWs-plan-repair";

type WorkspaceWsSendMessage =
  | WsInMessage
  | { type: "provider_select"; role: string; provider: string };
const CONNECT_TIMEOUT_MS = 5_000;
const PING_INTERVAL_MS = 25_000;
const SERVER_SILENCE_TIMEOUT_MS = 60_000;
const STALE_SOCKET_CLOSE_CODE = 4000;
const SERVER_SILENCE_CHECK_INTERVAL_MS = 15_000;
const STREAM_FLUSH_INTERVAL_MS = 50;
const DISCONNECTED_PRESENTATION_SAVE_ERROR = "连接已断开，请重连后重试";

// M2：门禁/推进命令 id 唯一生成点。动作发起时生成一次并随动作保存；
// 重试 / 重连重放由调用方把同一 id 传回 helper（不重新生成）。
export function newCommandId(): string {
  return crypto.randomUUID();
}

/** STALE_DRIVER_LEASE 的 context.received（被拒写命令的 message_type）；缺省 null。 */
function staleLeaseReceivedMessageType(context: unknown): string | null {
  if (typeof context !== "object" || context === null) {
    return null;
  }
  const received = (context as Record<string, unknown>).received;
  return typeof received === "string" ? received : null;
}


export type WorkspaceWsApi = ReturnType<typeof useWorkspaceWs>;

export function useWorkspaceWs(sessionId: string | null) {
  const wsRef = useRef<WebSocket | null>(null);
  const socketSessionIdRef = useRef<string | null>(null);
  const connectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const streamFlushTimeoutsRef = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const invalidatedPreStageNodeIdsRef = useRef<Set<string>>(new Set());
  const planRepairSourceRef = useRef<PlanRepairSourceState>({
    hasSnapshot: false,
    source: null,
  });
  const lastMessageAtRef = useRef(Date.now());
  const currentConnectionIdRef = useRef<string | null>(null);
  const lastServerMessageAtRef = useRef<string | null>(null);
  const lastPongOrServerMessageAtRef = useRef<string | null>(null);
  const lastPingAtRef = useRef<string | null>(null);
  const lastEventSeqRef = useRef<number | null>(null);
  // REQ-DLS-02（driver-lease-self-healing）：写命令 STALE 单次自动重试状态。
  // 镜像服务端 is_write_message（hello/ping 之外皆写面）记录最近一条已出站
  // 写命令；STALE_DRIVER_LEASE 到达且该命令未重放过时，先重发 driver hello
  // 夺回租约再原样重放一次（恰一次）。重放仍 STALE 则回落既有手动接管面。
  const leaseAutoRetryRef = useRef<{
    message: WorkspaceWsSendMessage;
    retried: boolean;
  } | null>(null);

  const [closeCode, setCloseCode] = useState<number | undefined>();
  const [sessionSnapshot, setSessionSnapshot] = useState({
    sessionId: null as string | null,
    generation: 0,
  });
  const workspaceConnectionStatus = useWorkspaceStore((state) => state.connectionStatus);
  const workspaceSessionId = useWorkspaceStore((state) => state.sessionId);
  const connectionStatus =
    sessionId &&
    socketSessionIdRef.current === sessionId &&
    workspaceSessionId === sessionId &&
    workspaceConnectionStatus === "connected"
      ? "connected"
      : workspaceConnectionStatus === "disconnected" ||
          workspaceConnectionStatus === "error"
        ? workspaceConnectionStatus
        : "connecting";

  const clearConnectTimeout = useCallback(() => {
    if (connectTimeoutRef.current) {
      clearTimeout(connectTimeoutRef.current);
      connectTimeoutRef.current = null;
    }
  }, []);

  const clearStreamFlushTimeouts = useCallback(() => {
    for (const timeout of Object.values(streamFlushTimeoutsRef.current)) {
      clearTimeout(timeout);
    }
    streamFlushTimeoutsRef.current = {};
  }, []);

  const clearPendingStreams = useCallback(() => {
    clearStreamFlushTimeouts();
    useWorkspaceStore.getState().clearAllStreamBuffers();
  }, [clearStreamFlushTimeouts]);

  const failPendingPresentationSaves = useCallback(() => {
    useWorkspaceStore
      .getState()
      .failPendingHumanPresentationSaves(DISCONNECTED_PRESENTATION_SAVE_ERROR);
  }, []);

  const scheduleFlush = useCallback((nodeId: string) => {
    if (streamFlushTimeoutsRef.current[nodeId]) {
      return;
    }
    streamFlushTimeoutsRef.current[nodeId] = setTimeout(() => {
      delete streamFlushTimeoutsRef.current[nodeId];
      useWorkspaceStore.getState().flushBufferedStream(nodeId);
    }, STREAM_FLUSH_INTERVAL_MS);
  }, []);
  const connect = useCallback(() => {
    if (!sessionId) return;

    const current = wsRef.current;
    if (
      current &&
      socketSessionIdRef.current === sessionId &&
      (current.readyState === WebSocket.CONNECTING || current.readyState === WebSocket.OPEN)
    ) {
      return;
    }
    if (current) {
      wsRef.current = null;
      socketSessionIdRef.current = null;
      current.close(1000);
    }

    setCloseCode(undefined);
    useWorkspaceStore.getState().setConnectionStatus("connecting");

    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const url = `${protocol}//${window.location.host}/api/workspace-sessions/${sessionId}/ws`;
    const ws = new WebSocket(url);
    wsRef.current = ws;
    socketSessionIdRef.current = sessionId;
    clearConnectTimeout();
    connectTimeoutRef.current = setTimeout(() => {
      if (wsRef.current !== ws) return;
      wsRef.current = null;
      socketSessionIdRef.current = null;
      setCloseCode(1006);
      failPendingPresentationSaves();
      const store = useWorkspaceStore.getState();
      store.setConnectionStatus("disconnected");
      store.setError("WebSocket 连接超时");
      ws.close();
    }, CONNECT_TIMEOUT_MS);

    currentConnectionIdRef.current = null;
    lastServerMessageAtRef.current = null;
    lastPongOrServerMessageAtRef.current = null;
    lastPingAtRef.current = null;
    ws.onopen = () => {
      if (wsRef.current !== ws) return;
      clearConnectTimeout();
      const store = useWorkspaceStore.getState();
      lastMessageAtRef.current = Date.now();
      store.setConnectionStatus("connected");
      store.setError(null);
      setCloseCode(undefined);
      ws.send(
        JSON.stringify({
          type: "hello",
          session_id: sessionId,
          last_seen_node_id: store.activeNodeId ?? store.timelineNodes.at(-1)?.node_id ?? null,
          role: "driver",
          ...(lastEventSeqRef.current !== null
            ? { after_event_seq: lastEventSeqRef.current }
            : {}),
        }),
      );
    };

    ws.onclose = (event) => {
      if (wsRef.current !== ws) return;
      clearConnectTimeout();
      clearPendingStreams();
      failPendingPresentationSaves();
      wsRef.current = null;
      socketSessionIdRef.current = null;
      setCloseCode(event.code);
      useWorkspaceStore.getState().recordConnectionCloseDiagnostic({
        connectionId: currentConnectionIdRef.current,
        closeCode: event.code,
        closeReason: event.reason,
        wasClean: event.wasClean,
        visibilityState: document.visibilityState,
        lastPongOrServerMessageAt: lastPongOrServerMessageAtRef.current,
        lastPingAt: lastPingAtRef.current,
        at: new Date().toISOString(),
      });
      useWorkspaceStore.getState().setConnectionStatus("disconnected");
    };

    ws.onerror = () => {
      if (wsRef.current !== ws) return;
      clearConnectTimeout();
      clearPendingStreams();
      failPendingPresentationSaves();
      const store = useWorkspaceStore.getState();
      wsRef.current = null;
      socketSessionIdRef.current = null;
      setCloseCode(1006);
      store.setConnectionStatus("disconnected");
      store.setError("WebSocket 连接失败");
    };

    ws.onmessage = (event) => {
      if (wsRef.current !== ws || socketSessionIdRef.current !== sessionId) return;
      lastMessageAtRef.current = Date.now();
      const now = new Date().toISOString();
      lastServerMessageAtRef.current = now;
      lastPongOrServerMessageAtRef.current = now;
      try {
        const msg = JSON.parse(event.data) as WsServerMessage;
        if (msg.type === "resync_required") {
          ws.close(STALE_SOCKET_CLOSE_CODE);
          return;
        }
        if (typeof msg.event_seq === "number") {
          if (msg.type === "session_state") {
            lastEventSeqRef.current = msg.event_seq;
          } else if (
            lastEventSeqRef.current !== null &&
            msg.event_seq <= lastEventSeqRef.current
          ) {
            return;
          } else {
            lastEventSeqRef.current = msg.event_seq;
          }
        }
        if (msg.type === "pong") {
          lastPongOrServerMessageAtRef.current = now;
        }
        if (msg.type === "session_state") {
          if (msg.session_id !== sessionId) return;
          currentConnectionIdRef.current = msg.connection_id ?? null;
          setSessionSnapshot((current) =>
            current.sessionId === sessionId
              ? { sessionId, generation: current.generation + 1 }
              : { sessionId, generation: 1 },
          );
        }
        handleMessage(msg);
      } catch {
        // ignore malformed messages
      }
    };
  }, [
    clearConnectTimeout,
    clearPendingStreams,
    failPendingPresentationSaves,
    sessionId,
  ]);

  const {
    isReconnecting,
    attemptCount: reconnectAttemptCount,
    retryNow,
    reset: resetReconnect,
  } = useWorkspaceWsReconnect({
    enabled:
      Boolean(sessionId) &&
      workspaceConnectionStatus === "disconnected" &&
      closeCode !== undefined &&
      closeCode !== 1000,
    closeCode,
    onReconnect: connect,
  });

  useEffect(() => {
    planRepairSourceRef.current = { hasSnapshot: false, source: null };
    useLinkedWorkspaceAmendmentStore.getState().reset(sessionId);
    const sessionChanged = useWorkspaceStore.getState().sessionId !== sessionId;
    if (sessionChanged) {
      lastEventSeqRef.current = null;
      useWorkspaceStore.setState({ connectionCloseDiagnostics: [] });
    }
    if (!sessionId) {
      clearPendingStreams();
      useWorkspaceStore.getState().reset();
      return;
    }

    connect();

    return () => {
      clearConnectTimeout();
      clearPendingStreams();
      const ws = wsRef.current;
      wsRef.current = null;
      socketSessionIdRef.current = null;
      ws?.close(1000);
    };
  }, [clearConnectTimeout, clearPendingStreams, connect, sessionId]);

  useEffect(() => {
    if (workspaceConnectionStatus === "connected") {
      resetReconnect();
    }
  }, [workspaceConnectionStatus, resetReconnect]);

  function handleMessage(msg: WsServerMessage) {
    // REQ-DLS-02：写命令被 STALE_DRIVER_LEASE 拒收时单次自动重试——重发
    // driver hello 夺回租约（服务端 bind_role 对既有连接同样执行
    // lease.acquire）后原样重放一次，成功即无感（不落 protocolError 错误
    // 面）；重放后的再次 STALE 消费完恰一次守卫，回落下方既有手动接管面。
    if (
      sessionId &&
      msg.type === "protocol_error" &&
      msg.code === STALE_DRIVER_LEASE_CODE
    ) {
      const lastWrite = leaseAutoRetryRef.current;
      const receivedType = staleLeaseReceivedMessageType(msg.context);
      if (
        lastWrite !== null &&
        !lastWrite.retried &&
        (receivedType === null || receivedType === lastWrite.message.type)
      ) {
        lastWrite.retried = true;
        const store = useWorkspaceStore.getState();
        sendHello(
          sessionId,
          store.activeNodeId ?? store.timelineNodes.at(-1)?.node_id ?? null,
        );
        sendWire(lastWrite.message);
        return;
      }
    }
    handleWorkspaceWsMessage(msg, {
      invalidatedPreStageNodeIds: invalidatedPreStageNodeIdsRef.current,
      scheduleFlush,
      streamFlushTimeouts: streamFlushTimeoutsRef.current,
    });
    aggregatePlanRepairChildMessage(msg, sessionId, planRepairSourceRef.current);
  }

  const sendWire = useCallback(
    (message: WorkspaceWsSendMessage) => {
      const ws = wsRef.current;
      if (
        !sessionId ||
        socketSessionIdRef.current !== sessionId ||
        ws?.readyState !== WebSocket.OPEN
      ) {
        return false;
      }
      ws.send(JSON.stringify(message));
      return true;
    },
    [sessionId],
  );

  const sendJson = useCallback(
    (message: WorkspaceWsSendMessage) => {
      const sent = sendWire(message);
      if (sent && message.type !== "hello" && message.type !== "ping") {
        leaseAutoRetryRef.current = { message, retried: false };
      }
      return sent;
    },
    [sendWire],
  );

  const sendContextNote = useCallback(
    (content: string) => {
      if (sendJson({ type: "context_note", content })) {
        useWorkspaceStore.getState().setError(null);
      }
    },
    [sendJson],
  );

  const sendStartGeneration = useCallback(
    (providerConfig: ProviderConfigSnapshot, reviewerEnabled: boolean) => {
      if (
        sendJson({
          type: "start_generation",
          provider_config: providerConfig,
          reviewer_enabled: reviewerEnabled,
        })
      ) {
        const store = useWorkspaceStore.getState();
        store.setError(null);
        store.clearExecutionEvents();
        store.setProviderStatus("running");
      }
    },
    [sendJson],
  );

  const retryInterruptedRun = useCallback(
    (failedNodeId: string) => {
      const sent = sendJson({
        type: "retry_interrupted_run",
        failed_node_id: failedNodeId,
      });
      if (sent) {
        const store = useWorkspaceStore.getState();
        store.setError(null);
        store.clearExecutionEvents();
        store.setProviderStatus("running");
      }
      return sent;
    },
    [sendJson],
  );

  const sendHello = useCallback(
    (targetSessionId: string, lastSeenNodeId?: string | null) => {
      sendJson({
        type: "hello",
        session_id: targetSessionId,
        last_seen_node_id: lastSeenNodeId ?? null,
        role: "driver",
        ...(lastEventSeqRef.current !== null
          ? { after_event_seq: lastEventSeqRef.current }
          : {}),
      });
    },
    [sendJson],
  );

  const sendPing = useCallback(() => {
    if (sendJson({ type: "ping" })) {
      lastPingAtRef.current = new Date().toISOString();
    }
  }, [sendJson]);

  useEffect(() => {
    if (workspaceConnectionStatus !== "connected") return;

    const interval = window.setInterval(() => {
      sendPing();
    }, PING_INTERVAL_MS);

    return () => window.clearInterval(interval);
  }, [workspaceConnectionStatus, sendPing]);

  useEffect(() => {
    function handleVisibilityChange() {
      sendPing();
      if (!document.hidden) {
        connect();
      }
    }

    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => document.removeEventListener("visibilitychange", handleVisibilityChange);
  }, [connect, sendPing]);

  useEffect(() => {
    if (workspaceConnectionStatus !== "connected") return;

    const interval = window.setInterval(() => {
      const ws = wsRef.current;
      const state = useWorkspaceStore.getState();
      const hasInFlightHumanConfirm =
        state.stage === "human_confirm" &&
        state.activeNodeId !== null &&
        state.timelineNodes.some(
          (node) => node.node_id === state.activeNodeId && node.status === "active",
        );
      if (
        ws?.readyState === WebSocket.OPEN &&
        !ACTIVE_PROVIDER_STAGES.has(state.stage) &&
        !hasInFlightHumanConfirm &&
        Date.now() - lastMessageAtRef.current >= SERVER_SILENCE_TIMEOUT_MS
      ) {
        ws.close(STALE_SOCKET_CLOSE_CODE);
      }
    }, SERVER_SILENCE_CHECK_INTERVAL_MS);

    return () => window.clearInterval(interval);
  }, [workspaceConnectionStatus]);


  const recordSentOperation = useCallback(
    (
      operation:
        | "confirm"
        | "abandon_gate"
        | "feedback"
        | "advance"
        | "confirm_plan_amendment",
      detail: string | null,
    ) => {
      if (!sessionId) {
        return;
      }
      const state = useWorkspaceStore.getState();
      useOperationAuditStore.getState().record({
        sessionId,
        gateId: selectGateProjection(state)?.key ?? null,
        operation,
        source: "chat",
        outcome: "sent",
        detail,
      });
    },
    [sessionId],
  );

  // L1 typed 重承载（REQ-RET-02）：SC 门 approve=既有 `confirm` unit 变体（REQ-CG-02
  // 接受面不动）；abandon=显式 `abandon_human_gate` 命令（T2 落地）。legacy
  // `human_confirm` 决策帧（request-change/terminate）不再由前端发送。
  const sendConfirmGate = useCallback(() => {
    const sent = sendJson({ type: "confirm" });
    if (sent) {
      recordSentOperation("confirm", "confirm");
    }
    return sent;
  }, [recordSentOperation, sendJson]);

  const sendAbandonGate = useCallback(
    (commandId: string) => {
      const sent = sendJson({ type: "abandon_human_gate", command_id: commandId });
      if (sent) {
        recordSentOperation("abandon_gate", commandId);
        // 乐观收敛门卡呈现；权威关门以服务端 human_gate_closed 事件为准。
        useWorkspaceStore.getState().resolveGateEntry("terminate");
      }
      return sent;
    },
    [recordSentOperation, sendJson],
  );
  const sendHumanGateFeedback = useCallback(
    (feedback: string, commandId?: string) => {
      const trimmed = feedback.trim();
      if (!trimmed) {
        return false;
      }
      const sent = sendJson({
        type: "human_gate_feedback",
        command_id: commandId ?? newCommandId(),
        feedback: trimmed,
      });
      if (sent) {
        recordSentOperation("feedback", trimmed);
      }
      return sent;
    },
    [recordSentOperation, sendJson],
  );

  const sendAdvance = useCallback(
    (commandId?: string) => {
      const id = commandId ?? newCommandId();
      const sent = sendJson({ type: "advance", command_id: id });
      if (sent) {
        recordSentOperation("advance", id);
      }
      return sent;
    },
    [recordSentOperation, sendJson],
  );

  const confirmPlanAmendment = useCallback(
    (amendmentId: string) => {
      const id = amendmentId.trim();
      if (!id) {
        return false;
      }
      const sent = sendJson({ type: "confirm_plan_amendment", amendment_id: id });
      if (sent) {
        recordSentOperation("confirm_plan_amendment", id);
      }
      return sent;
    },
    [recordSentOperation, sendJson],
  );

  const cancelPlanAmendment = useCallback(
    (amendmentId: string, reason?: string | null) => {
      const id = amendmentId.trim();
      const trimmedReason = reason?.trim() ?? "";
      return id
        ? sendJson({
            type: "cancel_plan_amendment",
            amendment_id: id,
            reason: trimmedReason || null,
          })
        : false;
    },
    [sendJson],
  );

  const startLinkedWorkspaceAmendment = useCallback(
    (target: LinkedWorkspaceAmendmentTarget) => {
      const entityId = target.entity_id.trim();
      const validPair =
        (target.workspace_type === "story" &&
          target.relation === "story_amendment") ||
        (target.workspace_type === "design" &&
          target.relation === "design_amendment");
      if (!entityId || !validPair) {
        return false;
      }
      const normalizedTarget = { ...target, entity_id: entityId };
      const sent = sendJson({
        type: "start_linked_workspace_amendment",
        target: normalizedTarget,
      });
      const store = useLinkedWorkspaceAmendmentStore.getState();
      if (sent) {
        const workspaceStore = useWorkspaceStore.getState();
        if (
          workspaceStore.protocolError?.code ===
          "LINKED_WORKSPACE_AMENDMENT_INVALID"
        ) {
          workspaceStore.setProtocolError(null);
        }
        store.begin(normalizedTarget);
      } else {
        store.fail("关联修订请求发送失败，请检查 Child Workspace 连接。");
      }
      return sent;
    },
    [sendJson],
  );


  const sendRequestRevision = useCallback(
    (feedback?: string): boolean => {
      const trimmedFeedback = feedback?.trim();
      return sendJson({
        type: "request_revision",
        feedback: {
          feedback_types: ["revision"],
          description: trimmedFeedback ?? "",
        },
      });
    },
    [sendJson],
  );


  const sendWorkItemPlanCompileRecoveryAction = useCallback(
    (action: WorkItemPlanCompileRecoveryAction, reason?: string) => {
      const trimmedReason = reason?.trim();
      sendJson({
        type: "work_item_plan_compile_recovery_action",
        action,
        reason: trimmedReason ? trimmedReason : null,
      });
    },
    [sendJson],
  );



  const sendMessage = useCallback(
    (content: string) => {
      console.warn("sendMessage is deprecated, use sendContextNote or sendStartGeneration");
      sendContextNote(content);
    },
    [sendContextNote],
  );

  const startGeneration = useCallback(() => {
    console.warn("startGeneration() without args is deprecated");
  }, []);

  const rollback = useCallback(
    (checkpointId: string) => {
      sendJson({ type: "rollback", checkpoint_id: checkpointId });
    },
    [sendJson],
  );

  const confirm = useCallback(() => {
    sendJson({ type: "confirm" });
  }, [sendJson]);

  const abort = useCallback(() => {
    sendJson({ type: "abort" });
  }, [sendJson]);

  const selectProvider = useCallback(
    (role: string, provider: string) => {
      if (sendJson({ type: "provider_select", role, provider })) {
        const validRole = role === "author" || role === "reviewer";
        const validProvider = providerName(provider);
        if (validRole && validProvider) {
          useWorkspaceStore.getState().setProviderSelection(role, validProvider);
        }
      }
    },
    [sendJson],
  );


  const respondPermission = useCallback(
    (id: string, approved: boolean, reason?: string) => {
      const trimmedReason = reason?.trim();
      if (
        sendJson({
          type: "permission_response",
          id,
          approved,
          reason: trimmedReason ? trimmedReason : null,
        })
      ) {
        console.info("[permission] sending response", { id, approved });
        useWorkspaceStore.getState().resolvePermissionRequest(id, approved);
      }
    },
    [sendJson],
  );

  const sendChoiceResponse = useCallback(
    (
      id: string,
      selectedOptionIds: string[],
      freeText?: string | null,
      answers?: ChoiceAnswerPayload[],
    ) => {
      const trimmedText = freeText?.trim();
      const store = useWorkspaceStore.getState();
      const choiceEntry = store.chatEntries.find(
        (entry) => entry.type === "choice_request" && entry.metadata?.request_id === id,
      );
      console.info("[aria-choice-diag] frontend choice_response send attempt", {
        id,
        selected_option_ids: selectedOptionIds,
        free_text_present: Boolean(trimmedText),
        source: choiceEntry?.metadata?.source ?? null,
        node_id: choiceEntry?.node_id ?? null,
        connection_status: store.connectionStatus,
        ws_ready_state: wsReadyStateName(wsRef.current),
      });
      const sent = sendJson({
        type: "choice_response",
        id,
        selected_option_ids: selectedOptionIds,
        free_text: trimmedText ? trimmedText : null,
        answers: answers && answers.length > 0 ? answers : undefined,
      });
      console.info("[aria-choice-diag] frontend choice_response send result", {
        id,
        sent,
      });
      if (sent) {
        useWorkspaceStore
          .getState()
          .resolveChoiceRequest(id, selectedOptionIds, trimmedText ? trimmedText : null, answers);
      }
    },
    [sendJson],
  );

  const sendProviderSelect = selectProvider;
  const sendPermissionResponse = respondPermission;

  return {
    sendMessage,
    sendContextNote,
    sendStartGeneration,
    retryInterruptedRun,
    sendRequestRevision,
    sendWorkItemPlanCompileRecoveryAction,
    sendConfirmGate,
    sendAbandonGate,
    sendHumanGateFeedback,
    sendAdvance,
    confirmPlanAmendment,
    cancelPlanAmendment,
    startLinkedWorkspaceAmendment,
    sendHello,
    sendPing,
    startGeneration,
    rollback,
    confirm,
    abort,
    selectProvider,
    sendProviderSelect,
    respondPermission,
    sendPermissionResponse,
    sendChoiceResponse,
    connectionStatus,
    isReconnecting,
    reconnectAttemptCount,
    retryNow,
    sessionSnapshotGeneration:
      sessionSnapshot.sessionId === sessionId ? sessionSnapshot.generation : 0,
  };
}
