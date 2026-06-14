import { useCallback, useEffect, useRef } from "react";
import type {
  CodingProviderPermissionMode,
  CodingProviderRole,
  CodingProviderSelectRole,
  CodingWsInMessage,
  CodingWsOutMessage,
  WorkspaceProviderName,
} from "../api/types";
import type { ChoiceResponsePayload } from "../state/chat-entries";
import { codingChatEntryToChatEntry } from "../state/coding-chat-entry-mapping";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";

interface CodingWsServerMessage {
  type: string;
  [key: string]: unknown;
}

const STREAM_CHUNK_FLUSH_MS = 50;

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

  const sendPermissionModeSelect = useCallback(
    (role: CodingProviderRole, permissionMode: CodingProviderPermissionMode) => {
      sendJson({ type: "permission_mode_select", role, permission_mode: permissionMode });
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
      if (!sendJson({ type: "permission_response", id, approved, reason: reason ?? null })) {
        return;
      }
      resolveCodingPermissionRequest(id, approved);
    },
    [sendJson],
  );

  const respondChoice = useCallback(
    (id: string, selectedOptionIds: string[], freeText?: string | null) => {
      const trimmedText = freeText?.trim();
      if (
        !sendJson({
          type: "choice_response",
          id,
          selected_option_ids: selectedOptionIds,
          free_text: trimmedText ? trimmedText : null,
        })
      ) {
        return;
      }
    },
    [sendJson],
  );

  const respondGate = useCallback(
    (gateId: string, actionId: string, extraContext?: string | null) => {
      const trimmedExtraContext = extraContext?.trim() ?? "";
      if (gateActionRequiresContext(actionId) && !trimmedExtraContext) {
        useCodingWorkspaceStore
          .getState()
          .setGateError(gateId, "coding_gate_extra_context_required");
        return;
      }
      if (!sendJson({
        type: "gate_response",
        gate_id: gateId,
        action_id: actionId,
        extra_context: trimmedExtraContext ? trimmedExtraContext : null,
      })) {
        return;
      }
      useCodingWorkspaceStore.getState().markGateSubmitting(gateId);
    },
    [sendJson],
  );

  const continueRework = useCallback(
    (extraContext?: string | null) => {
      const trimmedExtraContext = extraContext?.trim() ?? "";
      sendJson({
        type: "continue_rework",
        extra_context: trimmedExtraContext ? trimmedExtraContext : null,
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
    const streamBatcher = createCodingStreamBatcher();

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
        streamBatcher.flushAll();
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
        streamBatcher.flushAll();
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
          handleCodingWsMessage(JSON.parse(event.data) as CodingWsServerMessage, streamBatcher);
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
      streamBatcher.clear();
      const ws = wsRef.current;
      wsRef.current = null;
      ws?.close(1000);
    };
  }, [attemptId]);

  return {
    startCoding,
    sendContextNote,
    sendProviderSelect,
    sendPermissionModeSelect,
    confirmStageGate,
    respondPermission,
    respondChoice,
    respondGate,
    continueRework,
    finalConfirm,
    abortAttempt,
    requestManualPause,
    sendHello,
    sendPing,
  };
}

type CodingStreamBatcher = ReturnType<typeof createCodingStreamBatcher>;

function createCodingStreamBatcher() {
  let timer: ReturnType<typeof window.setTimeout> | null = null;
  const pending = new Map<string, { nodeId: string | null; content: string }>();

  function keyFor(nodeId?: string | null) {
    return nodeId ?? "__global__";
  }

  function clearTimer() {
    if (timer !== null) {
      window.clearTimeout(timer);
      timer = null;
    }
  }

  function flushKey(key: string) {
    const item = pending.get(key);
    if (!item) return;
    pending.delete(key);
    if (item.content && useCodingWorkspaceStore.getState().status !== "aborted") {
      useCodingWorkspaceStore.getState().appendStreamChunk(item.content, item.nodeId);
    }
  }

  function flushAll() {
    clearTimer();
    for (const key of Array.from(pending.keys())) {
      flushKey(key);
    }
  }

  function schedule() {
    if (timer !== null) return;
    timer = window.setTimeout(() => {
      timer = null;
      flushAll();
    }, STREAM_CHUNK_FLUSH_MS);
  }

  return {
    append(content: string, nodeId?: string | null) {
      const key = keyFor(nodeId);
      const existing = pending.get(key);
      pending.set(key, {
        nodeId: nodeId ?? null,
        content: `${existing?.content ?? ""}${content}`,
      });
      schedule();
    },
    flush(nodeId?: string | null) {
      flushKey(keyFor(nodeId));
      if (pending.size === 0) {
        clearTimer();
      }
    },
    flushAll,
    clear() {
      clearTimer();
      pending.clear();
    },
  };
}

function handleCodingWsMessage(message: CodingWsServerMessage, streamBatcher: CodingStreamBatcher) {
  const store = useCodingWorkspaceStore.getState();
  switch (message.type) {
    case "coding_session_state":
      streamBatcher.clear();
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
      if (store.status === "aborted") {
        break;
      }
      streamBatcher.append(
        message.content as string,
        (message.node_id as string | null | undefined) ?? null,
      );
      break;
    case "coding_message_complete":
      streamBatcher.flush((message.node_id as string | null | undefined) ?? null);
      store.completeStream((message.node_id as string | null | undefined) ?? null);
      break;
    case "coding_execution_event":
      if (store.status === "aborted") {
        break;
      }
      streamBatcher.flushAll();
      store.addExecutionEvent(
        (message as Extract<CodingWsOutMessage, { type: "coding_execution_event" }>).event,
      );
      break;
    case "coding_permission_request": {
      const request = message as Extract<
        CodingWsOutMessage,
        { type: "coding_permission_request" }
      >;
      store.appendChatEntry({
        id: permissionRequestEntryId(request.id),
        type: "permission_request",
        role: "system",
        content: permissionRequestContent(request.tool_name, request.description),
        timestamp: new Date().toISOString(),
        metadata: {
          request_id: request.id,
          tool_name: request.tool_name,
          description: request.description,
          risk_level: request.risk_level,
          request,
        },
      });
      break;
    }
    case "coding_choice_request": {
      const request = message as Extract<CodingWsOutMessage, { type: "coding_choice_request" }>;
      store.appendChatEntry({
        id: choiceRequestEntryId(request.id),
        type: "choice_request",
        role: "system",
        content: request.prompt,
        timestamp: new Date().toISOString(),
        metadata: {
          request_id: request.id,
          prompt: request.prompt,
          source: request.source,
          options: request.options,
          allow_multiple: request.allow_multiple,
          allow_free_text: request.allow_free_text,
        },
      });
      break;
    }
    case "coding_choice_response_ack": {
      const response = message as Extract<
        CodingWsOutMessage,
        { type: "coding_choice_response_ack" }
      >;
      resolveCodingChoiceRequest(response.id, {
        selected_option_ids: response.selected_option_ids,
        free_text: response.free_text ?? null,
      });
      break;
    }
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
      markSubmittingGateError(message.code as string);
      rejectCodingChoiceRequestFromError(message.message as string);
      break;
    case "coding_pong":
      break;
  }
}

function pendingContextNoteId() {
  return `pending_context_note_${Date.now()}_${Math.random().toString(36).slice(2)}`;
}

function permissionRequestEntryId(id: string) {
  return `permission_request:${id}`;
}

function choiceRequestEntryId(id: string) {
  return `choice_request:${id}`;
}

function gateActionRequiresContext(actionId: string) {
  return actionId === "manual_continue" || actionId === "accept_risk";
}

function markSubmittingGateError(errorCode: string) {
  const store = useCodingWorkspaceStore.getState();
  const submittingGate = store.pendingGates.find((gate) => gate.submitting);
  if (!submittingGate) return;
  store.setGateError(submittingGate.gate_id, errorCode);
}

function permissionRequestContent(toolName: string, description: string) {
  return description ? `${toolName} · ${description}` : toolName;
}

function resolveCodingPermissionRequest(id: string, approved: boolean) {
  const store = useCodingWorkspaceStore.getState();
  const entry = store.chatEntries.find(
    (candidate) =>
      candidate.type === "permission_request" && candidate.metadata?.request_id === id,
  );
  if (!entry) return;
  store.appendChatEntry({
    ...entry,
    resolved: true,
    metadata: {
      ...entry.metadata,
      approved,
      response: { approved },
    },
  });
}

function resolveCodingChoiceRequest(id: string, response: ChoiceResponsePayload) {
  const store = useCodingWorkspaceStore.getState();
  const entry = store.chatEntries.find(
    (candidate) => candidate.type === "choice_request" && candidate.metadata?.request_id === id,
  );
  if (!entry) return;
  store.appendChatEntry({
    ...entry,
    resolved: true,
    metadata: {
      ...entry.metadata,
      response,
    },
  });
}

function rejectCodingChoiceRequestFromError(message: string) {
  const id = choiceIdFromProtocolError(message);
  if (!id) return;
  const store = useCodingWorkspaceStore.getState();
  const entry = store.chatEntries.find(
    (candidate) => candidate.type === "choice_request" && candidate.metadata?.request_id === id,
  );
  if (!entry) return;
  store.appendChatEntry({
    ...entry,
    resolved: true,
    metadata: {
      ...entry.metadata,
      rejected: true,
      rejection_reason: message,
    },
  });
}

function choiceIdFromProtocolError(message: string) {
  return message.match(/^ChoiceResponse id=([^ ]+)/)?.[1] ?? null;
}
