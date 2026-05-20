import { useCallback, useEffect, useRef } from "react";
import type {
  HumanConfirmDecision,
  ProviderConfigSnapshot,
  RevisionPath,
  WsInMessage,
} from "../api/types";
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

export function useWorkspaceWs(sessionId: string | null) {
  const wsRef = useRef<WebSocket | null>(null);
  const connectionStatus = useWorkspaceStore((state) => state.connectionStatus);

  useEffect(() => {
    if (!sessionId) {
      useWorkspaceStore.getState().reset();
      return;
    }

    useWorkspaceStore.getState().setConnectionStatus("connecting");

    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const url = `${protocol}//${window.location.host}/api/workspace-sessions/${sessionId}/ws`;
    const ws = new WebSocket(url);
    wsRef.current = ws;

    ws.onopen = () => {
      const store = useWorkspaceStore.getState();
      store.setConnectionStatus("connected");
      store.setError(null);
    };

    ws.onclose = () => {
      useWorkspaceStore.getState().setConnectionStatus("disconnected");
    };

    ws.onerror = () => {
      const store = useWorkspaceStore.getState();
      store.setConnectionStatus("error");
      store.setError("WebSocket 连接失败");
    };

    ws.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data) as WsServerMessage;
        handleMessage(msg);
      } catch {
        // ignore malformed messages
      }
    };

    return () => {
      ws.close();
      wsRef.current = null;
    };
  }, [sessionId]);

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
        store.setArtifact(msg.markdown as string);
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
  };
}
