import { useCallback, useEffect, useRef, useState } from "react";
import type {
  AddGroupChatRoleRequest,
  DraftSlotKey,
  FinalizeGroupChatRequest,
  GroupChatWsInMessage,
  GroupChatWsOutMessage,
  TimelineEvent,
} from "../api/groupChat";

export type GroupChatConnectionStatus =
  | "connecting"
  | "connected"
  | "disconnected"
  | "error";

export type GroupChatTurnState = {
  text: string;
  status: "started" | "held";
};

export type UseGroupChatWsOptions = {
  reconnectDelayMs?: number;
  maxReconnectDelayMs?: number;
  onMessage?: (message: GroupChatWsOutMessage) => void;
  onError?: (message: Extract<GroupChatWsOutMessage, { type: "error" }>) => void;
};

export type UseGroupChatWsResult = {
  connectionStatus: GroupChatConnectionStatus;
  error: string | null;
  timeline: TimelineEvent[];
  /** 收到的瞬态帧，按接收顺序保留；RoomEvent 同时会进入 timeline。 */
  messages: GroupChatWsOutMessage[];
  turns: Record<string, GroupChatTurnState>;
  lastSeq: number;
  send: (message: GroupChatWsInMessage) => boolean;
  sendMessage: (
    text: string,
    mentions?: string[],
    draftSlot?: DraftSlotKey | null,
  ) => boolean;
  addRole: (payload: AddGroupChatRoleRequest) => boolean;
  finalize: (payload: FinalizeGroupChatRequest) => boolean;
  sendPing: () => boolean;
  reconnect: () => void;
};

const DEFAULT_RECONNECT_DELAY_MS = 1_000;
const DEFAULT_MAX_RECONNECT_DELAY_MS = 30_000;

export function useGroupChatWs(
  sessionId: string | null,
  options: UseGroupChatWsOptions = {},
): UseGroupChatWsResult {
  const wsRef = useRef<WebSocket | null>(null);
  const sessionIdRef = useRef<string | null>(null);
  const lastSeqRef = useRef(0);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const reconnectAttemptRef = useRef(0);
  const manuallyClosedRef = useRef(false);
  const optionsRef = useRef(options);
  const [connectionStatus, setConnectionStatus] =
    useState<GroupChatConnectionStatus>(sessionId ? "connecting" : "disconnected");
  const [error, setError] = useState<string | null>(null);
  const [timeline, setTimeline] = useState<TimelineEvent[]>([]);
  const [messages, setMessages] = useState<GroupChatWsOutMessage[]>([]);
  const [turns, setTurns] = useState<Record<string, GroupChatTurnState>>({});
  const [lastSeq, setLastSeq] = useState(0);

  optionsRef.current = options;

  const clearReconnectTimer = useCallback(() => {
    if (reconnectTimerRef.current !== null) {
      clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
  }, []);

  const closeSocket = useCallback(() => {
    const socket = wsRef.current;
    wsRef.current = null;
    sessionIdRef.current = null;
    if (socket) {
      socket.onopen = null;
      socket.onclose = null;
      socket.onerror = null;
      socket.onmessage = null;
      if (
        socket.readyState === WebSocket.CONNECTING ||
        socket.readyState === WebSocket.OPEN
      ) {
        socket.close(1000);
      }
    }
  }, []);

  const scheduleReconnect = useCallback(() => {
    if (manuallyClosedRef.current || !sessionId || reconnectTimerRef.current !== null) {
      return;
    }
    const baseDelay = optionsRef.current.reconnectDelayMs ?? DEFAULT_RECONNECT_DELAY_MS;
    const maxDelay =
      optionsRef.current.maxReconnectDelayMs ?? DEFAULT_MAX_RECONNECT_DELAY_MS;
    const delay = Math.min(
      maxDelay,
      baseDelay * 2 ** Math.min(reconnectAttemptRef.current, 5),
    );
    reconnectAttemptRef.current += 1;
    reconnectTimerRef.current = setTimeout(() => {
      reconnectTimerRef.current = null;
      connect();
    }, delay);
  }, [sessionId]);

  const handleMessage = useCallback((socket: WebSocket, event: MessageEvent) => {
    if (wsRef.current !== socket) return;
    let message: GroupChatWsOutMessage;
    try {
      const parsed: unknown = JSON.parse(String(event.data));
      if (!isGroupChatWsOutMessage(parsed)) {
        throw new Error("群聊 WebSocket 消息格式无效");
      }
      message = parsed;
    } catch (cause) {
      const messageText = cause instanceof Error ? cause.message : "群聊 WebSocket 消息格式无效";
      setError(messageText);
      setConnectionStatus("error");
      return;
    }

    optionsRef.current.onMessage?.(message);
    setMessages((current) => [...current, message]);
    if (message.type === "error") {
      setError(`${message.code}: ${message.message}`);
      setConnectionStatus("error");
      optionsRef.current.onError?.(message);
      return;
    }
    if (message.type === "room_event") {
      lastSeqRef.current = Math.max(lastSeqRef.current, message.seq);
      setLastSeq(lastSeqRef.current);
      setTimeline((current) => {
        if (current.some((entry) => entry.seq === message.seq)) return current;
        return [...current, { seq: message.seq, event: message.event }].sort(
          (left, right) => left.seq - right.seq,
        );
      });
    } else if (message.type === "turn_started") {
      setTurns((current) => ({
        ...current,
        [message.role_instance_id]: { text: "", status: "started" },
      }));
    } else if (message.type === "turn_delta") {
      setTurns((current) => ({
        ...current,
        [message.role_instance_id]: {
          text: `${current[message.role_instance_id]?.text ?? ""}${message.delta}`,
          status: current[message.role_instance_id]?.status ?? "started",
        },
      }));
    } else if (message.type === "turn_held") {
      setTurns((current) => ({
        ...current,
        [message.role_instance_id]: {
          text: current[message.role_instance_id]?.text ?? "",
          status: "held",
        },
      }));
    }
  }, []);

  const connect = useCallback(() => {
    if (!sessionId || manuallyClosedRef.current) return;
    const current = wsRef.current;
    if (
      current &&
      sessionIdRef.current === sessionId &&
      (current.readyState === WebSocket.CONNECTING || current.readyState === WebSocket.OPEN)
    ) {
      return;
    }
    closeSocket();
    clearReconnectTimer();
    setConnectionStatus("connecting");
    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const query = lastSeqRef.current > 0 ? `?after_seq=${lastSeqRef.current}` : "";
    const url = `${protocol}//${window.location.host}/ws/group-chat/${encodeURIComponent(sessionId)}${query}`;
    const socket = new WebSocket(url);
    wsRef.current = socket;
    sessionIdRef.current = sessionId;

    socket.onopen = () => {
      if (wsRef.current !== socket) return;
      reconnectAttemptRef.current = 0;
      setConnectionStatus("connected");
      setError(null);
    };
    socket.onmessage = (event) => handleMessage(socket, event);
    socket.onerror = () => {
      if (wsRef.current !== socket) return;
      setError("群聊 WebSocket 连接失败");
      setConnectionStatus("error");
    };
    socket.onclose = () => {
      if (wsRef.current !== socket) return;
      wsRef.current = null;
      sessionIdRef.current = null;
      if (!manuallyClosedRef.current) {
        setConnectionStatus("disconnected");
        scheduleReconnect();
      }
    };
  }, [clearReconnectTimer, closeSocket, handleMessage, scheduleReconnect, sessionId]);

  useEffect(() => {
    manuallyClosedRef.current = false;
    clearReconnectTimer();
    closeSocket();
    reconnectAttemptRef.current = 0;
    lastSeqRef.current = 0;
    setLastSeq(0);
    setTimeline([]);
    setMessages([]);
    setTurns({});
    setError(null);
    setConnectionStatus(sessionId ? "connecting" : "disconnected");
    if (sessionId) connect();

    return () => {
      manuallyClosedRef.current = true;
      clearReconnectTimer();
      closeSocket();
    };
  }, [clearReconnectTimer, closeSocket, connect, sessionId]);

  const send = useCallback((message: GroupChatWsInMessage): boolean => {
    const socket = wsRef.current;
    if (!socket || socket.readyState !== WebSocket.OPEN) return false;
    try {
      socket.send(JSON.stringify(message));
      return true;
    } catch {
      setError("群聊消息发送失败");
      return false;
    }
  }, []);

  const sendMessage = useCallback(
    (text: string, mentions: string[] = [], draftSlot?: DraftSlotKey | null) =>
      send({
        type: "send_message",
        text,
        mentions,
        ...(draftSlot === undefined ? {} : { draft_slot: draftSlot }),
      }),
    [send],
  );
  const addRole = useCallback((payload: AddGroupChatRoleRequest) => send({ type: "add_role", ...payload }), [send]);
  const finalize = useCallback((payload: FinalizeGroupChatRequest) => {
    const message: GroupChatWsInMessage = {
      type: "finalize",
      line_kind: payload.line_kind,
      ...(payload.included_slots_override === undefined
        ? {}
        : { included_slots: payload.included_slots_override }),
      ...(payload.confirmed_by === undefined ? {} : { confirmed_by: payload.confirmed_by }),
    };
    return send(message);
  }, [send]);
  const sendPing = useCallback(() => send({ type: "ping" }), [send]);
  const reconnect = useCallback(() => {
    manuallyClosedRef.current = false;
    clearReconnectTimer();
    closeSocket();
    connect();
  }, [clearReconnectTimer, closeSocket, connect]);

  return {
    connectionStatus,
    error,
    timeline,
    messages,
    turns,
    lastSeq,
    send,
    sendMessage,
    addRole,
    finalize,
    sendPing,
    reconnect,
  };
}

function isGroupChatWsOutMessage(value: unknown): value is GroupChatWsOutMessage {
  if (!isRecord(value) || typeof value.type !== "string") return false;
  switch (value.type) {
    case "room_event":
      return typeof value.seq === "number" && isRecord(value.event) && typeof value.event.type === "string";
    case "turn_started":
      return typeof value.role_instance_id === "string";
    case "turn_delta":
      return typeof value.role_instance_id === "string" && typeof value.delta === "string";
    case "turn_held":
      return typeof value.role_instance_id === "string" && typeof value.reason === "string";
    case "error":
      return typeof value.code === "string" && typeof value.message === "string";
    case "pong":
      return true;
    default:
      return false;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

