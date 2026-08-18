import { act, render } from "@testing-library/react";
import { createElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useGroupChatWs, type UseGroupChatWsResult } from "./useGroupChatWs";

class MockWebSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: MockWebSocket[] = [];
  readonly sent: string[] = [];
  readonly url: string;
  readyState = MockWebSocket.CONNECTING;
  onopen: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent<string>) => void) | null = null;

  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }

  send(data: string) {
    this.sent.push(data);
  }

  open() {
    this.readyState = MockWebSocket.OPEN;
    this.onopen?.(new Event("open"));
  }

  receive(data: unknown) {
    this.onmessage?.(new MessageEvent("message", { data: JSON.stringify(data) }));
  }

  close(code = 1000) {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.(new CloseEvent("close", { code }));
  }
}

describe("useGroupChatWs", () => {
  let hook: UseGroupChatWsResult | undefined;
  function Harness({ onError }: { onError?: UseGroupChatWsResult["send"] }) {
    hook = useGroupChatWs("session/1", {
      reconnectDelayMs: 10,
      maxReconnectDelayMs: 10,
      onError: onError
        ? (message) => {
            onError({ type: "ping" });
            void message;
          }
        : undefined,
    });
    return null;
  }

  beforeEach(() => {
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
    vi.useFakeTimers();
    hook = undefined;
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("连接后收发消息并把 RoomEvent 写入时间线", () => {
    const view = render(createElement(Harness));
    const socket = MockWebSocket.instances[0];
    act(() => socket.open());
    expect(hook?.connectionStatus).toBe("connected");

    act(() => {
      hook?.sendMessage("请分析", ["role-1"], "story/slot");
    });
    expect(socket.sent).toEqual([
      JSON.stringify({
        type: "send_message",
        text: "请分析",
        mentions: ["role-1"],
        draft_slot: "story/slot",
      }),
    ]);

    act(() => {
      socket.receive({
        type: "room_event",
        seq: 3,
        event: { type: "user_message", text: "请分析", mentions: ["role-1"] },
      });
      socket.receive({ type: "turn_started", role_instance_id: "role-1" });
      socket.receive({ type: "turn_delta", role_instance_id: "role-1", delta: "答复" });
    });
    expect(hook?.lastSeq).toBe(3);
    expect(hook?.timeline).toHaveLength(1);
    expect(hook?.turns["role-1"]).toEqual({ text: "答复", status: "started" });
    view.unmount();
  });

  it("断开后按 last_seq 自动重连，并在 URL 中携带 after_seq", () => {
    render(createElement(Harness));
    const first = MockWebSocket.instances[0];
    act(() => {
      first.open();
      first.receive({
        type: "room_event",
        seq: 8,
        event: { type: "system_notice", text: "之前的事件" },
      });
      first.close(1006);
    });
    expect(hook?.connectionStatus).toBe("disconnected");

    act(() => {
      vi.advanceTimersByTime(10);
    });
    expect(MockWebSocket.instances).toHaveLength(2);
    expect(MockWebSocket.instances[1].url).toBe(
      "ws://localhost:3000/ws/group-chat/session%2F1?after_seq=8",
    );
  });

  it("处理服务端错误帧并调用错误回调", () => {
    const onError = vi.fn();
    function ErrorHarness() {
      hook = useGroupChatWs("session/1", {
        onError,
        reconnectDelayMs: 10,
      });
      return null;
    }
    render(createElement(ErrorHarness));
    const socket = MockWebSocket.instances[0];
    act(() => {
      socket.open();
      socket.receive({ type: "error", code: "invalid_message", message: "消息无效" });
    });
    expect(hook?.connectionStatus).toBe("error");
    expect(hook?.error).toBe("invalid_message: 消息无效");
    expect(hook?.messages).toEqual([
      { type: "error", code: "invalid_message", message: "消息无效" },
    ]);
    expect(onError).toHaveBeenCalledWith({
      type: "error",
      code: "invalid_message",
      message: "消息无效",
    });
  });
});
