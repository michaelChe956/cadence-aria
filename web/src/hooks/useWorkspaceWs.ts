import { useCallback, useEffect, useRef, useState } from "react";
import type {
  AuthorDecision,
  HumanConfirmDecision,
  ProviderConfigSnapshot,
  RevisionPath,
  WorkspaceProviderName,
  WorkItemBatchDecision,
  WorkItemDraftDecision,
  WorkItemGenerationMode,
  WorkItemPlanCompileRecoveryAction,
  WsInMessage,
} from "../api/types";
import { useWorkspaceWsReconnect } from "./useWorkspaceWsReconnect";
import {
  useWorkspaceStore,
} from "../state/workspace-ws-store";
import type { ChoiceAnswerPayload } from "../state/chat-entries";
import {
  ACTIVE_PROVIDER_STAGES,
  handleWorkspaceWsMessage,
  providerName,
  type WsServerMessage,
  wsReadyStateName,
} from "./workspace-ws-message-handler";


type WorkspaceWsSendMessage =
  | WsInMessage
  | { type: "provider_select"; role: string; provider: string };

const PING_INTERVAL_MS = 25_000;
const SERVER_SILENCE_TIMEOUT_MS = 60_000;
const STALE_SOCKET_CLOSE_CODE = 4000;
const SERVER_SILENCE_CHECK_INTERVAL_MS = 15_000;
const CONNECT_TIMEOUT_MS = 5_000;
const STREAM_FLUSH_INTERVAL_MS = 80;

export function useWorkspaceWs(sessionId: string | null) {
  const wsRef = useRef<WebSocket | null>(null);
  const connectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const streamFlushTimeoutsRef = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const invalidatedPreStageNodeIdsRef = useRef<Set<string>>(new Set());
  const lastMessageAtRef = useRef(Date.now());
  const [closeCode, setCloseCode] = useState<number | undefined>();
  const connectionStatus = useWorkspaceStore((state) => state.connectionStatus);

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
      (current.readyState === WebSocket.CONNECTING || current.readyState === WebSocket.OPEN)
    ) {
      return;
    }

    setCloseCode(undefined);
    useWorkspaceStore.getState().setConnectionStatus("connecting");

    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const url = `${protocol}//${window.location.host}/api/workspace-sessions/${sessionId}/ws`;
    const ws = new WebSocket(url);
    wsRef.current = ws;
    clearConnectTimeout();
    connectTimeoutRef.current = setTimeout(() => {
      if (wsRef.current !== ws) return;
      wsRef.current = null;
      setCloseCode(1006);
      const store = useWorkspaceStore.getState();
      store.setConnectionStatus("disconnected");
      store.setError("WebSocket 连接超时");
      ws.close();
    }, CONNECT_TIMEOUT_MS);

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
        }),
      );
    };

    ws.onclose = (event) => {
      if (wsRef.current !== ws) return;
      clearConnectTimeout();
      clearPendingStreams();
      wsRef.current = null;
      setCloseCode(event.code);
      useWorkspaceStore.getState().setConnectionStatus("disconnected");
    };

    ws.onerror = () => {
      if (wsRef.current !== ws) return;
      clearConnectTimeout();
      clearPendingStreams();
      const store = useWorkspaceStore.getState();
      wsRef.current = null;
      setCloseCode(1006);
      store.setConnectionStatus("disconnected");
      store.setError("WebSocket 连接失败");
    };

    ws.onmessage = (event) => {
      lastMessageAtRef.current = Date.now();
      try {
        const msg = JSON.parse(event.data) as WsServerMessage;
        handleMessage(msg);
      } catch {
        // ignore malformed messages
      }
    };
  }, [clearConnectTimeout, clearPendingStreams, sessionId]);

  const {
    isReconnecting,
    attemptCount: reconnectAttemptCount,
    retryNow,
    reset: resetReconnect,
  } = useWorkspaceWsReconnect({
    enabled:
      Boolean(sessionId) &&
      connectionStatus === "disconnected" &&
      closeCode !== undefined &&
      closeCode !== 1000,
    closeCode,
    onReconnect: connect,
  });

  useEffect(() => {
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
      ws?.close(1000);
    };
  }, [clearConnectTimeout, clearPendingStreams, connect, sessionId]);

  useEffect(() => {
    if (connectionStatus === "connected") {
      resetReconnect();
    }
  }, [connectionStatus, resetReconnect]);

  function handleMessage(msg: WsServerMessage) {
    handleWorkspaceWsMessage(msg, {
      invalidatedPreStageNodeIds: invalidatedPreStageNodeIdsRef.current,
      scheduleFlush,
      streamFlushTimeouts: streamFlushTimeoutsRef.current,
    });
  }

  const sendJson = useCallback((message: WorkspaceWsSendMessage) => {
    const ws = wsRef.current;
    if (ws?.readyState !== WebSocket.OPEN) {
      return false;
    }
    ws.send(JSON.stringify(message));
    return true;
  }, []);

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

  const sendHello = useCallback(
    (targetSessionId: string, lastSeenNodeId?: string | null) => {
      sendJson({
        type: "hello",
        session_id: targetSessionId,
        last_seen_node_id: lastSeenNodeId ?? null,
      });
    },
    [sendJson],
  );

  const sendPing = useCallback(() => {
    sendJson({ type: "ping" });
  }, [sendJson]);

  useEffect(() => {
    if (connectionStatus !== "connected") return;

    const interval = window.setInterval(() => {
      sendPing();
    }, PING_INTERVAL_MS);

    return () => window.clearInterval(interval);
  }, [connectionStatus, sendPing]);

  useEffect(() => {
    if (connectionStatus !== "connected") return;

    const interval = window.setInterval(() => {
      const ws = wsRef.current;
      const stage = useWorkspaceStore.getState().stage;
      if (
        ws?.readyState === WebSocket.OPEN &&
        !ACTIVE_PROVIDER_STAGES.has(stage) &&
        Date.now() - lastMessageAtRef.current >= SERVER_SILENCE_TIMEOUT_MS
      ) {
        ws.close(STALE_SOCKET_CLOSE_CODE);
      }
    }, SERVER_SILENCE_CHECK_INTERVAL_MS);

    return () => window.clearInterval(interval);
  }, [connectionStatus]);

  const sendSelectRevisionPath = useCallback(
    (path: RevisionPath, extraContext?: string) => {
      const trimmedContext = extraContext?.trim();
      sendJson({
        type: "select_revision_path",
        path,
        extra_context: trimmedContext ? trimmedContext : null,
      });
    },
    [sendJson],
  );

  const sendHumanConfirm = useCallback(
    (decision: HumanConfirmDecision, payload?: unknown) => {
      if (sendJson({ type: "human_confirm", decision, payload: payload ?? null })) {
        useWorkspaceStore.getState().resolveGateEntry(decision);
      }
    },
    [sendJson],
  );

  const sendAuthorDecision = useCallback(
    (decision: AuthorDecision) => {
      sendJson({ type: "author_decision", decision });
    },
    [sendJson],
  );

  const sendRequestRevision = useCallback(
    (feedback?: string) => {
      const trimmedFeedback = feedback?.trim();
      sendJson({
        type: "request_revision",
        feedback: {
          feedback_types: ["revision"],
          description: trimmedFeedback ?? "",
        },
      });
    },
    [sendJson],
  );

  const sendRevertWorkItem = useCallback(
    (workItemId: string, feedback?: string, clear = false) => {
      sendJson({
        type: "revert_work_item",
        work_item_id: workItemId,
        feedback: feedback?.trim() ?? null,
        clear,
      });
    },
    [sendJson],
  );

  const sendSelectWorkItemGenerationMode = useCallback(
    (mode: WorkItemGenerationMode) => {
      sendJson({ type: "select_work_item_generation_mode", mode });
    },
    [sendJson],
  );

  const sendRequestOutlineRevision = useCallback(
    (feedback?: string) => {
      const trimmedFeedback = feedback?.trim();
      sendJson({
        type: "request_outline_revision",
        feedback: trimmedFeedback ? trimmedFeedback : null,
      });
    },
    [sendJson],
  );

  const sendWorkItemDraftDecision = useCallback(
    (outlineId: string, decision: WorkItemDraftDecision, feedback?: string) => {
      const trimmedFeedback = feedback?.trim();
      sendJson({
        type: "work_item_draft_decision",
        outline_id: outlineId,
        decision,
        feedback: trimmedFeedback ? trimmedFeedback : null,
      });
    },
    [sendJson],
  );

  const sendWorkItemBatchDecision = useCallback(
    (
      decision: WorkItemBatchDecision,
      feedback?: string,
      firstAffectedOutlineId?: string,
    ) => {
      const trimmedFeedback = feedback?.trim();
      const trimmedOutlineId = firstAffectedOutlineId?.trim();
      sendJson({
        type: "work_item_batch_decision",
        decision,
        feedback: trimmedFeedback ? trimmedFeedback : null,
        first_affected_outline_id: trimmedOutlineId ? trimmedOutlineId : null,
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

  const sendReviewDecision = useCallback((decision: string, extraContext?: string) => {
    const trimmedContext = extraContext?.trim();
    sendJson({
      type: "review_decision_response",
      decision,
      extra_context: trimmedContext ? trimmedContext : null,
    });
  }, [sendJson]);

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
    sendSelectRevisionPath,
    sendAuthorDecision,
    sendRequestRevision,
    sendRevertWorkItem,
    sendSelectWorkItemGenerationMode,
    sendRequestOutlineRevision,
    sendWorkItemDraftDecision,
    sendWorkItemBatchDecision,
    sendWorkItemPlanCompileRecoveryAction,
    sendHumanConfirm,
    sendHello,
    sendPing,
    startGeneration,
    rollback,
    confirm,
    abort,
    selectProvider,
    sendProviderSelect,
    sendReviewDecision,
    respondPermission,
    sendPermissionResponse,
    sendChoiceResponse,
    connectionStatus,
    isReconnecting,
    reconnectAttemptCount,
    retryNow,
  };
}
