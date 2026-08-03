import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  ImageCreateSession,
  SessionRecord,
  SessionSummary,
} from "../api/types/image-create";

const api = vi.hoisted(() => ({
  listImageCreateSessions: vi.fn(),
  getImageCreateSession: vi.fn(),
  createImageCreateSession: vi.fn(),
  deleteImageCreateSession: vi.fn(),
  generateImage: vi.fn(),
  getImageCreateSettings: vi.fn(),
  updateImageCreateSettings: vi.fn(),
  imageCreateChatWebSocketUrl: vi.fn(
    (sessionId: string) =>
      `ws://localhost:3000/api/image-create/sessions/${sessionId}/chat`,
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
    this.onclose?.(new CloseEvent("close"));
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

describe("image create store", () => {
  beforeEach(async () => {
    vi.resetModules();
    vi.clearAllMocks();
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
    Object.defineProperty(window, "location", {
      configurable: true,
      value: { protocol: "http:", host: "localhost:3000" },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("creates a session with the required provider_name", async () => {
    const session = imageSession();
    api.createImageCreateSession.mockResolvedValue(session);
    api.listImageCreateSessions.mockResolvedValue([summary(session)]);
    api.getImageCreateSession.mockResolvedValue(record(session));
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().createSession(
      { preset: "business_flow_diagram" },
      "codex",
    );

    expect(api.createImageCreateSession).toHaveBeenCalledWith({
      template: { preset: "business_flow_diagram" },
      provider_name: "codex",
    });
    expect(useImageCreateStore.getState().currentSession?.session.id).toBe(
      "session-1",
    );
  });

  it("maps iteration events to independent entries and keeps the previous prompt when parsing has no suggestion", async () => {
    const session = imageSession({ current_prompt: "previous prompt" });
    api.getImageCreateSession.mockResolvedValue(record(session));
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().openSession(session.id);
    const socket = MockWebSocket.instances[0];
    socket.open();
    useImageCreateStore.getState().sendMessage("make it more vivid");

    expect(socket.sent).toEqual(["make it more vivid"]);
    expect(useImageCreateStore.getState().isBusy).toBe(true);
    expect(useImageCreateStore.getState().entries.at(-1)).toMatchObject({
      type: "user_message",
      content: "make it more vivid",
    });

    socket.receive({
      kind: "text",
      text: "I improved the lighting.",
      suggested_prompt: null,
      provider_session_id: null,
      error: null,
    });
    socket.receive({
      kind: "done",
      text: null,
      suggested_prompt: null,
      provider_session_id: "provider-session-1",
      error: null,
    });

    expect(useImageCreateStore.getState().params.prompt).toBe("previous prompt");
    expect(useImageCreateStore.getState().entries).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          type: "provider_text",
          content: "I improved the lighting.",
        }),
        expect.objectContaining({ type: "prompt_block", content: "previous prompt" }),
        expect.objectContaining({
          type: "system_notice",
          content: "本轮未产出新的建议 prompt，已保留上一版",
        }),
      ]),
    );
    expect(useImageCreateStore.getState().isBusy).toBe(false);
  });

  it("keeps the latest A to B session open when A resolves last", async () => {
    const first = imageSession({ id: "session-a", current_prompt: "prompt a" });
    const second = imageSession({ id: "session-b", current_prompt: "prompt b" });
    const requests = new Map<
      string,
      (value: SessionRecord) => void
    >();
    api.getImageCreateSession.mockImplementation(
      (sessionId: string) =>
        new Promise<SessionRecord>((resolve) => requests.set(sessionId, resolve)),
    );
    const { useImageCreateStore } = await import("./image-create-store");

    const openingFirst = useImageCreateStore.getState().openSession(first.id);
    const openingSecond = useImageCreateStore.getState().openSession(second.id);
    requests.get(second.id)!(record(second));
    await openingSecond;
    requests.get(first.id)!(record(first));
    await openingFirst;

    expect(useImageCreateStore.getState().currentSession?.session.id).toBe(second.id);
    expect(useImageCreateStore.getState().params.prompt).toBe("prompt b");
    expect(MockWebSocket.instances).toHaveLength(1);
    expect(MockWebSocket.instances[0].url).toContain("/session-b/chat");
  });

  it("keeps the latest A target in an A to B to A out-of-order sequence", async () => {
    const firstOld = imageSession({ id: "session-a", current_prompt: "old a" });
    const second = imageSession({ id: "session-b", current_prompt: "prompt b" });
    const firstLatest = imageSession({ id: "session-a", current_prompt: "latest a" });
    const pending: Array<{
      sessionId: string;
      resolve: (value: SessionRecord) => void;
    }> = [];
    api.getImageCreateSession.mockImplementation(
      (sessionId: string) =>
        new Promise<SessionRecord>((resolve) => pending.push({ sessionId, resolve })),
    );
    const { useImageCreateStore } = await import("./image-create-store");

    const openingFirstOld = useImageCreateStore.getState().openSession(firstOld.id);
    const openingSecond = useImageCreateStore.getState().openSession(second.id);
    const openingFirstLatest = useImageCreateStore.getState().openSession(firstLatest.id);
    pending[1].resolve(record(second));
    await openingSecond;
    pending[0].resolve(record(firstOld));
    await openingFirstOld;
    pending[2].resolve(record(firstLatest));
    await openingFirstLatest;

    expect(useImageCreateStore.getState().currentSession?.session.id).toBe(firstLatest.id);
    expect(useImageCreateStore.getState().params.prompt).toBe("latest a");
    expect(MockWebSocket.instances).toHaveLength(1);
    expect(MockWebSocket.instances[0].url).toContain("/session-a/chat");
    expect(pending.map(({ sessionId }) => sessionId)).toEqual([
      "session-a",
      "session-b",
      "session-a",
    ]);
  });

  it("does not append an async generation result after switching sessions", async () => {
    const first = imageSession({ id: "session-a", current_prompt: "prompt a" });
    const second = imageSession({ id: "session-b", current_prompt: "prompt b" });
    api.getImageCreateSession.mockImplementation(async (sessionId: string) =>
      record(sessionId === first.id ? first : second),
    );
    let resolveGeneration!: (result: {
      media_type: string;
      b64: string;
    }) => void;
    api.generateImage.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveGeneration = resolve;
        }),
    );
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().openSession(first.id);
    const generation = useImageCreateStore.getState().generate();
    await useImageCreateStore.getState().openSession(second.id);
    resolveGeneration({ media_type: "image/png", b64: "old-session-image" });
    await generation;

    expect(useImageCreateStore.getState().currentSession?.session.id).toBe(second.id);
    expect(useImageCreateStore.getState().entries).not.toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          type: "generation_image",
          base64: "old-session-image",
        }),
      ]),
    );
    expect(useImageCreateStore.getState().isBusy).toBe(false);
  });

  it.each([
    "http://127.example.com",
    "http://127.0.0.1.example.com",
  ])("rejects non-loopback base URL %s", async (baseUrl) => {
    const { validateImageCreateBaseUrl } = await import("./image-create-store");

    expect(() => validateImageCreateBaseUrl(baseUrl)).toThrow(
      "base_url 必须使用 HTTPS，或使用 localhost/loopback IP",
    );
  });

  it("treats malformed numeric hosts as invalid URLs", async () => {
    const { validateImageCreateBaseUrl } = await import("./image-create-store");

    expect(() => validateImageCreateBaseUrl("http://127.0.0.999")).toThrow(
      "base_url 必须是有效 URL",
    );
  });

  it.each([
    "http://localhost:8080",
    "http://127.0.0.2:8080",
    "http://[::1]:8080",
  ])("accepts loopback base URL %s", async (baseUrl) => {
    const { validateImageCreateBaseUrl } = await import("./image-create-store");

    expect(() => validateImageCreateBaseUrl(baseUrl)).not.toThrow();
  });

  it("does not show a no-prompt notice after receiving a valid prompt", async () => {
    const session = imageSession({ current_prompt: "previous prompt" });
    api.getImageCreateSession.mockResolvedValue(record(session));
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().openSession(session.id);
    const socket = MockWebSocket.instances[0];
    socket.open();
    useImageCreateStore.getState().sendMessage("revise");
    socket.receive({
      kind: "prompt",
      text: null,
      suggested_prompt: "new prompt",
      provider_session_id: null,
      error: null,
    });
    socket.receive({
      kind: "done",
      text: null,
      suggested_prompt: null,
      provider_session_id: "provider-session-1",
      error: null,
    });

    expect(useImageCreateStore.getState().params.prompt).toBe("new prompt");
    expect(useImageCreateStore.getState().lastIterationHadPrompt).toBe(false);
    expect(useImageCreateStore.getState().entries).not.toEqual(
      expect.arrayContaining([
        expect.objectContaining({ type: "system_notice" }),
      ]),
    );
  });

  it("updates the prompt when a prompt event arrives and clears busy on errors", async () => {
    const session = imageSession({ current_prompt: "previous prompt" });
    api.getImageCreateSession.mockResolvedValue(record(session));
    const { useImageCreateStore } = await import("./image-create-store");

    await useImageCreateStore.getState().openSession(session.id);
    const socket = MockWebSocket.instances[0];
    socket.open();
    useImageCreateStore.getState().sendMessage("revise");
    socket.receive({
      kind: "prompt",
      text: null,
      suggested_prompt: "new prompt",
      provider_session_id: null,
      error: null,
    });
    socket.receive({
      kind: "error",
      text: null,
      suggested_prompt: null,
      provider_session_id: null,
      error: "provider failed",
    });

    expect(useImageCreateStore.getState().params.prompt).toBe("new prompt");
    expect(useImageCreateStore.getState().entries.at(-1)).toMatchObject({
      type: "generation_error",
      content: "provider failed",
    });
    expect(useImageCreateStore.getState().isBusy).toBe(false);
  });
});

function imageSession(
  overrides: Partial<ImageCreateSession> = {},
): ImageCreateSession {
  return {
    id: "session-1",
    provider_name: "codex",
    template: { preset: "business_flow_diagram", custom: null },
    last_provider_session_id: null,
    current_prompt: null,
    status: "active",
    created_at: "2026-08-03T00:00:00Z",
    ...overrides,
  };
}

function summary(session: ImageCreateSession): SessionSummary {
  return {
    id: session.id,
    provider_name: session.provider_name,
    template: session.template,
    status: session.status,
    created_at: session.created_at,
    updated_at: session.created_at,
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
