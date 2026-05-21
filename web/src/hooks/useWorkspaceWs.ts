import { useCallback, useEffect, useRef, useState } from "react";
import type {
  HumanConfirmDecision,
  ProviderConfigSnapshot,
  RevisionPath,
  WsInMessage,
} from "../api/types";
import { useWorkspaceWsReconnect } from "./useWorkspaceWsReconnect";
import {
  useWorkspaceStore,
  type ExecutionEvent,
  type ProviderStatus,
  type ReviewVerdict,
  type TimelineNode,
  type TimelineNodeStatus,
} from "../state/workspace-ws-store";

interface WsServerMessage {
  type: string;
  [key: string]: unknown;
}

type WorkspaceWsSendMessage =
  | WsInMessage
  | { type: "provider_select"; role: string; provider: string };

const PING_INTERVAL_MS = 25_000;
const SERVER_SILENCE_TIMEOUT_MS = 60_000;
const STALE_SOCKET_CLOSE_CODE = 4000;
const SERVER_SILENCE_CHECK_INTERVAL_MS = 15_000;
const CONNECT_TIMEOUT_MS = 5_000;

export function useWorkspaceWs(sessionId: string | null) {
  const wsRef = useRef<WebSocket | null>(null);
  const connectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastMessageAtRef = useRef(Date.now());
  const [closeCode, setCloseCode] = useState<number | undefined>();
  const connectionStatus = useWorkspaceStore((state) => state.connectionStatus);

  const clearConnectTimeout = useCallback(() => {
    if (connectTimeoutRef.current) {
      clearTimeout(connectTimeoutRef.current);
      connectTimeoutRef.current = null;
    }
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
      wsRef.current = null;
      setCloseCode(event.code);
      useWorkspaceStore.getState().setConnectionStatus("disconnected");
    };

    ws.onerror = () => {
      if (wsRef.current !== ws) return;
      clearConnectTimeout();
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
  }, [clearConnectTimeout, sessionId]);

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
      useWorkspaceStore.getState().reset();
      return;
    }

    connect();

    return () => {
      clearConnectTimeout();
      const ws = wsRef.current;
      wsRef.current = null;
      ws?.close(1000);
    };
  }, [clearConnectTimeout, connect, sessionId]);

  useEffect(() => {
    if (connectionStatus === "connected") {
      resetReconnect();
    }
  }, [connectionStatus, resetReconnect]);

  function handleMessage(msg: WsServerMessage) {
    const store = useWorkspaceStore.getState();
    switch (msg.type) {
      case "session_state":
        store.setSessionState(msg as never);
        break;
      case "stream_chunk":
        store.appendStreamChunk(msg.content as string, msg.node_id as string | null | undefined);
        break;
      case "message_complete":
        store.completeMessage(
          msg.message_id as string,
          msg.checkpoint_id as string,
          msg.node_id as string | null | undefined,
        );
        break;
      case "stage_change":
        store.setStage(msg.stage as string);
        break;
      case "artifact_update":
        store.setArtifact(msg.markdown as string, msg.version as number | undefined);
        break;
      case "permission_request":
        store.addPermissionRequest({
          id: msg.id as string,
          tool_name: msg.tool_name as string,
          description: msg.description as string,
          risk_level: msg.risk_level as "low" | "medium" | "high",
        });
        break;
      case "provider_status":
        store.setProviderStatus(msg.status as ProviderStatus);
        break;
      case "execution_event":
        store.upsertExecutionEvent(msg.event as ExecutionEvent);
        break;
      case "timeline_node_created":
        store.addTimelineNode(msg.node as TimelineNode);
        break;
      case "timeline_node_updated":
        store.updateTimelineNode(
          msg.node_id as string,
          msg.status as TimelineNodeStatus,
          msg.summary as string | null | undefined,
          msg.completed_at as string | null | undefined,
        );
        break;
      case "review_complete":
        store.setNodeVerdict(msg.node_id as string, {
          verdict: msg.verdict,
          comments: msg.comments,
          summary: msg.summary,
        } as ReviewVerdict);
        break;
      case "review_decision_required":
        store.setPendingDecision({
          node_id: msg.node_id as string,
          round: msg.round as number,
          options: msg.options as string[],
        });
        break;
      case "error":
        store.setError(msg.message as string);
        break;
      case "protocol_error":
        store.setProtocolError({
          code: msg.code as string,
          message: msg.message as string,
        });
        break;
      case "provider_locked":
        store.setProviderLocked({
          snapshot: msg.snapshot as ProviderConfigSnapshot,
          locked_at: msg.locked_at as string,
        });
        break;
      case "pong":
        break;
    }
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
      if (
        ws?.readyState === WebSocket.OPEN &&
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
      sendJson({ type: "human_confirm", decision, payload: payload ?? null });
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
      sendJson({ type: "provider_select", role, provider });
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
        useWorkspaceStore.getState().resolvePermissionRequest(id);
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
    connectionStatus,
    isReconnecting,
    reconnectAttemptCount,
    retryNow,
  };
}
