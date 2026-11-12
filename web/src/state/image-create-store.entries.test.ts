import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ImageCreateSession,
  IterationEvent,
  SessionRecord,
} from "../api/types/image-create";
import { useImageCreateStore } from "./image-create-store";
import type { ProviderText } from "./image-create-entries";

const api = vi.hoisted(() => ({
  listImageCreateSessions: vi.fn(),
  getImageCreateSession: vi.fn(),
  createImageCreateSession: vi.fn(),
  deleteImageCreateSession: vi.fn(),
  generateImage: vi.fn(),
  getImageCreateSettings: vi.fn(),
  updateImageCreateSettings: vi.fn(),
  testImageVendorConnectivity: vi.fn(),
  imageCreateChatWebSocketUrl: vi.fn(
    (sessionId: string) =>
      `ws://localhost:3000/api/image-create/sessions/${sessionId}/chat`,
  ),
  imageUrl: vi.fn(
    (sessionId: string, imageId: string) =>
      `/api/image-create/sessions/${encodeURIComponent(sessionId)}/images/${encodeURIComponent(imageId)}`,
  ),
}));

vi.mock("../api/image-create", () => api);

class MockWebSocket {
  static instances: MockWebSocket[] = [];
  static OPEN = 1;
  readonly url: string;
  readyState = MockWebSocket.OPEN;
  sent: string[] = [];
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent<string>) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;

  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }

  send(message: string) {
    this.sent.push(message);
  }

  close() {
    this.readyState = 3;
  }

  open() {
    this.onopen?.(new Event("open"));
  }

  receive(payload: unknown) {
    this.onmessage?.(
      new MessageEvent("message", { data: JSON.stringify(payload) }),
    );
  }
}

function imageSession(
  overrides: Partial<ImageCreateSession> = {},
): ImageCreateSession {
  return {
    id: "session-1",
    provider_name: "codex",
    template: { preset: "business_flow_diagram", custom: null },
    last_provider_session_id: null,
    current_prompt: "上一版 prompt",
    status: "active",
    created_at: "2026-08-03T00:00:00Z",
    ...overrides,
  };
}

function record(session: ImageCreateSession): SessionRecord {
  return {
    session,
    messages: [],
    prompt_blocks: [],
    generation_results: [],
    events: [],
    generation: 0,
  };
}

function textEvent(text: string, promptDeclined?: boolean): IterationEvent {
  return {
    kind: "text",
    text,
    suggested_prompt: null,
    provider_session_id: null,
    error: null,
    ...(promptDeclined === undefined ? {} : { prompt_declined: promptDeclined }),
  };
}

function doneEvent(promptDeclined?: boolean): IterationEvent {
  return {
    kind: "done",
    text: null,
    suggested_prompt: null,
    provider_session_id: "provider-session-1",
    error: null,
    ...(promptDeclined === undefined ? {} : { prompt_declined: promptDeclined }),
  };
}

/** 打开会话并进入可收发消息的状态，返回已连接的 socket。 */
async function openSession(
  sessionOverrides: Partial<ImageCreateSession> = {},
) {
  const session = imageSession(sessionOverrides);
  api.getImageCreateSession.mockResolvedValue(record(session));
  await useImageCreateStore.getState().openSession(session.id);
  const socket = MockWebSocket.instances.at(-1)!;
  socket.open();
  useImageCreateStore.getState().sendMessage("改一下这张图");
  return socket;
}

/** 当前时间线里的模型说明文本条目。 */
function providerTexts(): ProviderText[] {
  return useImageCreateStore
    .getState()
    .entries.filter((item): item is ProviderText => item.type === "provider_text");
}

describe("图片创作：模型未产出时的提示", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
    Object.defineProperty(window, "location", {
      configurable: true,
      value: { protocol: "http:", host: "localhost:3000" },
    });
    useImageCreateStore.getState().reset();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    useImageCreateStore.getState().reset();
  });

  it("把模型的 fallback 文本标记为可折叠", async () => {
    const socket = await openSession();

    socket.receive(textEvent("模型给出的长篇说明", true));

    const entry = useImageCreateStore
      .getState()
      .entries.find((item) => item.type === "provider_text");
    expect(entry).toMatchObject({
      type: "provider_text",
      content: "模型给出的长篇说明",
      collapsed: true,
    });
  });

  it("模型明确未产出时使用拒答文案", async () => {
    const socket = await openSession();

    socket.receive(textEvent("我不愿意给出这张图的 prompt", true));
    socket.receive(doneEvent(true));

    const notices = useImageCreateStore
      .getState()
      .entries.filter((item) => item.type === "system_notice");
    expect(notices).toHaveLength(1);
    expect(notices[0]?.content).toBe(
      "本轮模型未产出可用 prompt（下方是它给出的说明），已保留上一版",
    );
  });

  it("模型未按结构化契约输出时沿用原提示文案", async () => {
    const socket = await openSession();

    socket.receive(textEvent("格式不符的输出", false));
    socket.receive(doneEvent(false));

    const notices = useImageCreateStore
      .getState()
      .entries.filter((item) => item.type === "system_notice");
    expect(notices).toHaveLength(1);
    expect(notices[0]?.content).toBe(
      "本轮未产出新的建议 prompt，已保留上一版",
    );
  });

  it("后端未下发 prompt_declined 时按未拒答处理", async () => {
    const socket = await openSession();

    socket.receive(textEvent("旧版后端的输出"));
    socket.receive(doneEvent());

    const notices = useImageCreateStore
      .getState()
      .entries.filter((item) => item.type === "system_notice");
    expect(notices[0]?.content).toBe(
      "本轮未产出新的建议 prompt，已保留上一版",
    );
  });

  it("done 未带标记时回落到 fallback 文本携带的拒答标记", async () => {
    const socket = await openSession();

    socket.receive(textEvent("我不愿意给出这张图的 prompt", true));
    socket.receive(doneEvent());

    const notices = useImageCreateStore
      .getState()
      .entries.filter((item) => item.type === "system_notice");
    expect(notices[0]?.content).toBe(
      "本轮模型未产出可用 prompt（下方是它给出的说明），已保留上一版",
    );
  });

  it("首轮没有上一版时同样给出提示", async () => {
    const socket = await openSession({ current_prompt: null });

    socket.receive(textEvent("我不愿意给出这张图的 prompt", true));
    socket.receive(doneEvent(true));

    const notices = useImageCreateStore
      .getState()
      .entries.filter((item) => item.type === "system_notice");
    expect(notices).toHaveLength(1);
    expect(notices[0]?.content).toBe(
      "本轮模型未产出可用 prompt（下方是它给出的说明）",
    );
  });

  it("没有上一版时文案不带已保留上一版", async () => {
    const socket = await openSession({ current_prompt: null });

    socket.receive(textEvent("格式不符的输出", false));
    socket.receive(doneEvent(false));

    const notices = useImageCreateStore
      .getState()
      .entries.filter((item) => item.type === "system_notice");
    expect(notices[0]?.content).toBe("本轮未产出新的建议 prompt");
  });

  it("模型只给结构化块没有说明文本时文案不带下方说明", async () => {
    const socket = await openSession();

    socket.receive(doneEvent(true));

    const notices = useImageCreateStore
      .getState()
      .entries.filter((item) => item.type === "system_notice");
    expect(notices[0]?.content).toBe(
      "本轮模型未产出可用 prompt，已保留上一版",
    );
  });

  it("模型未按契约输出时的说明文本不折叠", async () => {
    const socket = await openSession();

    socket.receive(textEvent("格式不符的输出", false));

    expect(providerTexts()[0]?.collapsed).toBe(false);
  });

  it("模型明确未产出时的说明文本折叠", async () => {
    const socket = await openSession();

    socket.receive(textEvent("我不愿意给出这张图的 prompt", true));

    expect(providerTexts()[0]?.collapsed).toBe(true);
  });

  it("模型未产出且既无说明文本也无上一版时文案最短", async () => {
    const socket = await openSession({ current_prompt: null });

    socket.receive(doneEvent(true));

    const notices = useImageCreateStore
      .getState()
      .entries.filter((item) => item.type === "system_notice");
    expect(notices).toHaveLength(1);
    expect(notices[0]?.content).toBe("本轮模型未产出可用 prompt");
  });

  it("说明文本为空白时不生成折叠块", async () => {
    const socket = await openSession();

    socket.receive(textEvent("   \n  ", true));
    socket.receive(doneEvent(true));

    expect(providerTexts()).toHaveLength(0);
  });

  it("提示排在 fallback 文本之前，不会被长文本淹没", async () => {
    const socket = await openSession();

    socket.receive(textEvent("模型给出的长篇说明", true));
    socket.receive(doneEvent(true));

    const types = useImageCreateStore
      .getState()
      .entries.map((entry) => entry.type);
    // 历史里保留的上一版提示词在最前面；本轮的提示必须排在模型说明之前。
    expect(types.slice(-2)).toEqual(["system_notice", "provider_text"]);
    expect(types.indexOf("system_notice")).toBeLessThan(
      types.indexOf("provider_text"),
    );
  });
});
