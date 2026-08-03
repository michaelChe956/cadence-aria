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
      ]),
    );
    expect(useImageCreateStore.getState().isBusy).toBe(false);
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
