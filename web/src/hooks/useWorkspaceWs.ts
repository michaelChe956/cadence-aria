import { useCallback, useEffect, useRef } from "react";
import { useWorkspaceStore } from "../state/workspace-ws-store";

interface WsOutMessage {
  type: string;
  [key: string]: unknown;
}

export function useWorkspaceWs(sessionId: string | null) {
  const wsRef = useRef<WebSocket | null>(null);
  const store = useWorkspaceStore();

  useEffect(() => {
    if (!sessionId) {
      store.reset();
      return;
    }

    store.setConnectionStatus("connecting");

    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const url = `${protocol}//${window.location.host}/api/workspace-sessions/${sessionId}/ws`;
    const ws = new WebSocket(url);
    wsRef.current = ws;

    ws.onopen = () => {
      store.setConnectionStatus("connected");
      store.setError(null);
    };

    ws.onclose = () => {
      store.setConnectionStatus("disconnected");
    };

    ws.onerror = () => {
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
    switch (msg.type) {
      case "session_state":
        store.setSessionState(msg as never);
        break;
      case "stream_chunk":
        store.appendStreamChunk(msg.content as string);
        break;
      case "message_complete":
        store.completeMessage(msg.message_id as string, msg.checkpoint_id as string);
        break;
      case "stage_change":
        store.setStage(msg.stage as string);
        break;
      case "artifact_update":
        store.setArtifact(msg.markdown as string);
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
        store.setError(null);
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

  return {
    sendMessage,
    rollback,
    confirm,
    abort,
    selectProvider,
    connectionStatus: store.connectionStatus,
  };
}
