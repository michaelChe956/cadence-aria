import { useCallback, useEffect, useRef } from "react";
import {
  useWorkspaceStore,
  type ExecutionEvent,
  type ProviderStatus,
  type ReviewVerdict,
  type TimelineNode,
  type TimelineNodeStatus,
} from "../state/workspace-ws-store";

interface WsOutMessage {
  type: string;
  [key: string]: unknown;
}

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
        const msg = JSON.parse(event.data) as WsOutMessage;
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

  function handleMessage(msg: WsOutMessage) {
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
    }
  }

  const sendMessage = useCallback(
    (content: string) => {
      const ws = wsRef.current;
      if (ws?.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "user_message", content }));
        const store = useWorkspaceStore.getState();
        store.setError(null);
        store.clearExecutionEvents();
        store.setProviderStatus("running");
        const userMsg = {
          id: `msg_${Date.now()}`,
          role: "user",
          content,
          checkpoint_id: null,
          created_at: new Date().toISOString(),
        };
        useWorkspaceStore.setState((prev) => ({
          messages: [...prev.messages, userMsg],
        }));
      }
    },
    [],
  );

  const startGeneration = useCallback(() => {
    sendMessage("开始生成");
  }, [sendMessage]);

  const rollback = useCallback(
    (checkpointId: string) => {
      const ws = wsRef.current;
      if (ws?.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "rollback", checkpoint_id: checkpointId }));
      }
    },
    [],
  );

  const confirm = useCallback(() => {
    const ws = wsRef.current;
    if (ws?.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "confirm" }));
    }
  }, []);

  const abort = useCallback(() => {
    const ws = wsRef.current;
    if (ws?.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: "abort" }));
    }
  }, []);

  const selectProvider = useCallback(
    (role: string, provider: string) => {
      const ws = wsRef.current;
      if (ws?.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "provider_select", role, provider }));
      }
    },
    [],
  );

  const sendReviewDecision = useCallback((decision: string, extraContext?: string) => {
    const ws = wsRef.current;
    if (ws?.readyState === WebSocket.OPEN) {
      const trimmedContext = extraContext?.trim();
      ws.send(
        JSON.stringify({
          type: "review_decision_response",
          decision,
          extra_context: trimmedContext ? trimmedContext : null,
        }),
      );
    }
  }, []);

  const respondPermission = useCallback(
    (id: string, approved: boolean, reason?: string) => {
      const ws = wsRef.current;
      if (ws?.readyState === WebSocket.OPEN) {
        const trimmedReason = reason?.trim();
        ws.send(
          JSON.stringify({
            type: "permission_response",
            id,
            approved,
            reason: trimmedReason ? trimmedReason : null,
          }),
        );
        useWorkspaceStore.getState().resolvePermissionRequest(id);
      }
    },
    [],
  );

  return {
    sendMessage,
    startGeneration,
    rollback,
    confirm,
    abort,
    selectProvider,
    sendReviewDecision,
    respondPermission,
    connectionStatus,
  };
}
