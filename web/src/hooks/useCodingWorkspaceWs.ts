import { useCallback, useEffect, useRef } from "react";
import type {
  CodingProviderSelectRole,
  CodingWsInMessage,
  CodingWsOutMessage,
  WorkspaceProviderName,
} from "../api/types";
import { codingChatEntryToChatEntry } from "../state/coding-chat-entry-mapping";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";

interface CodingWsServerMessage {
  type: string;
  [key: string]: unknown;
}

export function useCodingWorkspaceWs(attemptId: string | null) {
  const wsRef = useRef<WebSocket | null>(null);
  const heartbeatTimerRef = useRef<ReturnType<typeof window.setInterval> | null>(null);
  const reconnectTimerRef = useRef<ReturnType<typeof window.setTimeout> | null>(null);

  const sendJson = useCallback((message: CodingWsInMessage) => {
    const ws = wsRef.current;
    if (!ws || ws.readyState !== WebSocket.OPEN) return false;
    ws.send(JSON.stringify(message));
    return true;
  }, []);

  const sendHello = useCallback(
    (lastSeenNodeId?: string | null) => {
      if (!attemptId) return;
      sendJson({
        type: "coding_hello",
        attempt_id: attemptId,
        last_seen_node_id: lastSeenNodeId ?? null,
      });
    },
    [attemptId, sendJson],
  );

  const startCoding = useCallback(() => {
    sendJson({ type: "start_coding" });
  }, [sendJson]);

  const sendContextNote = useCallback(
    (content: string) => {
      if (!sendJson({ type: "context_note", content })) return;
      useCodingWorkspaceStore.getState().appendChatEntry({
        id: pendingContextNoteId(),
        type: "context_note",
        role: "user",
        content,
        timestamp: new Date().toISOString(),
        metadata: { pending: true },
      });
    },
    [sendJson],
  );

  const sendProviderSelect = useCallback(
    (role: CodingProviderSelectRole, provider: WorkspaceProviderName) => {
      sendJson({ type: "provider_select", role, provider });
    },
    [sendJson],
  );

  const confirmStageGate = useCallback(
    (stage: Extract<CodingWsInMessage, { type: "stage_gate_confirm" }>["stage"]) => {
      sendJson({ type: "stage_gate_confirm", stage });
    },
    [sendJson],
  );

  const respondPermission = useCallback(
    (id: string, approved: boolean, reason?: string | null) => {
      sendJson({ type: "permission_response", id, approved, reason: reason ?? null });
    },
    [sendJson],
  );

  const respondGate = useCallback(
    (gateId: string, actionId: string, extraContext?: string | null) => {
      sendJson({
        type: "gate_response",
        gate_id: gateId,
        action_id: actionId,
        extra_context: extraContext ?? null,
      });
    },
    [sendJson],
  );

  const finalConfirm = useCallback(() => {
    sendJson({ type: "final_confirm" });
  }, [sendJson]);

  const abortAttempt = useCallback(() => {
    sendJson({ type: "abort_attempt" });
  }, [sendJson]);

  const requestManualPause = useCallback(() => {
    sendJson({ type: "request_manual_pause" });
  }, [sendJson]);

  const sendPing = useCallback(() => {
    sendJson({ type: "coding_ping" });
  }, [sendJson]);

  useEffect(() => {
    if (!attemptId) {
      useCodingWorkspaceStore.getState().reset();
      return;
    }

    let disposed = false;
    let reconnectAttempt = 0;

    function clearHeartbeat() {
      if (heartbeatTimerRef.current !== null) {
        window.clearInterval(heartbeatTimerRef.current);
        heartbeatTimerRef.current = null;
      }
    }

    function clearReconnect() {
      if (reconnectTimerRef.current !== null) {
        window.clearTimeout(reconnectTimerRef.current);
        reconnectTimerRef.current = null;
      }
    }

    function sendOpeningHello(ws: WebSocket) {
      const store = useCodingWorkspaceStore.getState();
      ws.send(
        JSON.stringify({
          type: "coding_hello",
          attempt_id: attemptId,
          last_seen_node_id: store.activeNodeId ?? store.timelineNodes.at(-1)?.id ?? null,
        }),
      );
    }

    function startHeartbeat(ws: WebSocket) {
      clearHeartbeat();
      heartbeatTimerRef.current = window.setInterval(() => {
        if (wsRef.current === ws && ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: "coding_ping" }));
        }
      }, 25_000);
    }

    function scheduleReconnect() {
      if (disposed) return;
      clearHeartbeat();
      clearReconnect();
      reconnectAttempt += 1;
      const delayMs = Math.min(1_000 * 2 ** (reconnectAttempt - 1), 10_000);
      useCodingWorkspaceStore.getState().setConnectionStatus("reconnecting");
      reconnectTimerRef.current = window.setTimeout(() => connect(true), delayMs);
    }

    function connect(reconnecting: boolean) {
      if (disposed) return;
      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      const ws = new WebSocket(
        `${protocol}//${window.location.host}/ws/coding-attempts/${attemptId}`,
      );
      wsRef.current = ws;
      useCodingWorkspaceStore
        .getState()
        .setConnectionStatus(reconnecting ? "reconnecting" : "connecting");

      ws.onopen = () => {
        if (wsRef.current !== ws) return;
        reconnectAttempt = 0;
        clearReconnect();
        const store = useCodingWorkspaceStore.getState();
        store.setConnectionStatus("connected");
        store.setProtocolError(null);
        sendOpeningHello(ws);
        startHeartbeat(ws);
      };

      ws.onclose = (event) => {
        if (wsRef.current !== ws) return;
        clearHeartbeat();
        wsRef.current = null;
        if (disposed || event.code === 1000) {
          useCodingWorkspaceStore.getState().setConnectionStatus("disconnected");
          return;
        }
        scheduleReconnect();
      };

      ws.onerror = () => {
        if (wsRef.current !== ws) return;
        clearHeartbeat();
        wsRef.current = null;
        const store = useCodingWorkspaceStore.getState();
        store.setProtocolError({
          code: "coding_ws_connection_failed",
          message: "Coding WebSocket connection failed",
        });
        scheduleReconnect();
      };

      ws.onmessage = (event) => {
        try {
          handleCodingWsMessage(JSON.parse(event.data) as CodingWsServerMessage);
        } catch {
          // Ignore malformed websocket messages; backend protocol errors are handled explicitly.
        }
      };
    }

    connect(false);

    return () => {
      disposed = true;
      clearHeartbeat();
      clearReconnect();
      const ws = wsRef.current;
      wsRef.current = null;
      ws?.close(1000);
    };
  }, [attemptId]);

  return {
    startCoding,
    sendContextNote,
    sendProviderSelect,
    confirmStageGate,
    respondPermission,
    respondGate,
    finalConfirm,
    abortAttempt,
    requestManualPause,
    sendHello,
    sendPing,
  };
}

function handleCodingWsMessage(message: CodingWsServerMessage) {
  const store = useCodingWorkspaceStore.getState();
  switch (message.type) {
    case "coding_session_state":
      store.setSessionState(message as Extract<CodingWsOutMessage, { type: "coding_session_state" }>);
      break;
    case "coding_stage_change":
      store.updateStage(message.stage as never);
      break;
    case "coding_timeline_node_created":
      store.addTimelineNode(
        (message as Extract<CodingWsOutMessage, { type: "coding_timeline_node_created" }>).node,
      );
      break;
    case "coding_timeline_node_updated":
      store.updateTimelineNode(
        message.node_id as string,
        message.status as never,
        (message.summary as string | null | undefined) ?? null,
        (message.completed_at as string | null | undefined) ?? null,
      );
      break;
    case "coding_stream_chunk":
      store.appendStreamChunk(
        message.content as string,
        (message.node_id as string | null | undefined) ?? null,
      );
      break;
    case "coding_message_complete":
      store.completeStream((message.node_id as string | null | undefined) ?? null);
      break;
    case "coding_execution_event":
      store.addExecutionEvent(
        (message as Extract<CodingWsOutMessage, { type: "coding_execution_event" }>).event,
      );
      break;
    case "testing_report_update":
      store.setTestingReport(
        (message as Extract<CodingWsOutMessage, { type: "testing_report_update" }>).report,
      );
      break;
    case "code_review_complete":
      store.addCodeReviewReport(
        (message as Extract<CodingWsOutMessage, { type: "code_review_complete" }>).report,
      );
      break;
    case "review_request_update":
      store.setReviewRequest(
        (message as Extract<CodingWsOutMessage, { type: "review_request_update" }>).review_request,
      );
      break;
    case "internal_pr_review_complete":
      store.setInternalPrReview(
        (message as Extract<CodingWsOutMessage, { type: "internal_pr_review_complete" }>).review,
      );
      break;
    case "coding_gate_required":
      store.addPendingGate(
        (message as Extract<CodingWsOutMessage, { type: "coding_gate_required" }>).gate,
      );
      break;
    case "coding_chat_entry_created":
      store.replacePendingEntry(
        codingChatEntryToChatEntry(
          (message as Extract<CodingWsOutMessage, { type: "coding_chat_entry_created" }>).entry,
        ),
      );
      break;
    case "coding_provider_config_updated":
      store.updateProviderConfig(
        (message as Extract<CodingWsOutMessage, { type: "coding_provider_config_updated" }>).role,
        (message as Extract<CodingWsOutMessage, { type: "coding_provider_config_updated" }>).provider,
      );
      break;
    case "coding_protocol_error":
      store.setProtocolError({
        code: message.code as string,
        message: message.message as string,
      });
      break;
    case "coding_pong":
      break;
  }
}

function pendingContextNoteId() {
  return `pending_context_note_${Date.now()}_${Math.random().toString(36).slice(2)}`;
}
