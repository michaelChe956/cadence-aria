import { create } from "zustand";
import {
  createImageCreateSession,
  deleteImageCreateSession,
  generateImage as requestImageGeneration,
  getImageCreateSession,
  getImageCreateSettings,
  imageCreateChatWebSocketUrl,
  imageUrl,
  listImageCreateSessions,
  updateImageCreateSettings,
} from "../api/image-create";
import {
  DEFAULT_IMAGE_PARAMS,
  type GenerateImageRequest,
  type ImageCreateParams,
  type ImageCreateProvider,
  type ImageCreateTemplateChoice,
  type IterationEvent,
  type MaskedSettings,
  type SessionRecord,
  type SessionSummary,
  type SettingsUpdateRequest,
} from "../api/types/image-create";
import type { ImageChatEntry } from "./image-create-entries";

export type ImageCreateConnectionStatus =
  | "disconnected"
  | "connecting"
  | "connected"
  | "error";

export type ImageCreateState = {
  sessions: SessionSummary[];
  currentSession: SessionRecord | null;
  entries: ImageChatEntry[];
  params: ImageCreateParams;
  referenceImage: File | null;
  settings: MaskedSettings | null;
  connectionStatus: ImageCreateConnectionStatus;
  isBusy: boolean;
  lastIterationHadPrompt: boolean;
  error: string | null;
};

export type ImageCreateActions = {
  loadSessions: () => Promise<void>;
  openSession: (sessionId: string) => Promise<void>;
  createSession: (
    template: ImageCreateTemplateChoice,
    providerName: ImageCreateProvider,
  ) => Promise<SessionRecord>;
  deleteSession: (sessionId: string) => Promise<void>;
  sendMessage: (message: string) => void;
  editPromptBlock: (content: string) => void;
  setParams: (params: Partial<ImageCreateParams>) => void;
  setReferenceImage: (file: File | null) => void;
  generate: () => Promise<void>;
  loadSettings: () => Promise<void>;
  saveSettings: (request: SettingsUpdateRequest) => Promise<void>;
  disconnect: () => void;
  reset: () => void;
};

const initialState: ImageCreateState = {
  sessions: [],
  currentSession: null,
  entries: [],
  params: {
    prompt: "",
    ...DEFAULT_IMAGE_PARAMS,
    input_fidelity: null,
  },
  referenceImage: null,
  settings: null,
  connectionStatus: "disconnected",
  isBusy: false,
  lastIterationHadPrompt: false,
  error: null,
};

let activeSocket: WebSocket | null = null;
let entrySequence = 0;
let generationRequestSequence = 0;
let openSessionRequestSequence = 0;

function entryId(prefix: string): string {
  entrySequence += 1;
  return `${prefix}-${Date.now()}-${entrySequence}`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function buildEntries(record: SessionRecord): ImageChatEntry[] {
  const entries: ImageChatEntry[] = record.messages.map<ImageChatEntry>(
    (message) =>
      message.role === "user"
        ? {
            id: entryId("message"),
            type: "user_message",
            role: "user",
            content: message.content,
            timestamp: message.ts,
          }
        : {
            id: entryId("message"),
            type: "provider_text",
            role: "provider",
            content: message.content,
            timestamp: message.ts,
          },
  );
  entries.push(
    ...record.prompt_blocks.map<ImageChatEntry>((block) => ({
      id: entryId("prompt"),
      type: "prompt_block",
      role: "provider",
      content: block.content,
      version: block.version,
      timestamp: record.session.created_at,
    })),
    ...record.generation_results.map<ImageChatEntry>((result) => {
      if (result.image_id) {
        return {
          id: entryId("image"),
          type: "generation_image",
          role: "provider",
          content: result.prompt,
          prompt: result.prompt,
          mediaType: result.media_type,
          imageUrl: imageUrl(record.session.id, result.image_id),
          timestamp: result.ts,
        };
      }
      if (result.legacy_pending) {
        return {
          id: entryId("event"),
          type: "system_notice",
          role: "system",
          content: "历史图片正在迁移，请稍后刷新",
          timestamp: result.ts,
        };
      }
      return {
        id: entryId("error"),
        type: "generation_error",
        role: "system",
        content: "图片引用缺失",
        timestamp: result.ts,
      };
    }),
    ...record.events.map<ImageChatEntry>((event) => ({
      id: entryId("event"),
      type:
        event.kind === "generation_error"
          ? "generation_error"
          : "system_notice",
      role: "system",
      content: event.message,
      timestamp: event.ts,
    })),
  );
  if (
    record.session.current_prompt &&
    !record.prompt_blocks.some(
      (block) => block.content === record.session.current_prompt,
    )
  ) {
    entries.push({
      id: entryId("prompt"),
      type: "prompt_block",
      role: "provider",
      content: record.session.current_prompt,
      timestamp: record.session.created_at,
    });
  }
  return entries;
}

function closeActiveSocket() {
  if (!activeSocket) {
    return;
  }
  activeSocket.onopen = null;
  activeSocket.onmessage = null;
  activeSocket.onerror = null;
  activeSocket.onclose = null;
  activeSocket.close();
  activeSocket = null;
}

function appendEntry(entry: ImageChatEntry) {
  useImageCreateStore.setState((state) => ({
    entries: [...state.entries, entry],
  }));
}

function handleIterationEvent(event: IterationEvent) {
  const timestamp = new Date().toISOString();
  if (event.kind === "text" && event.text) {
    appendEntry({
      id: entryId("provider"),
      type: "provider_text",
      role: "provider",
      content: event.text,
      timestamp,
    });
    return;
  }
  if (event.kind === "prompt" && event.suggested_prompt?.trim()) {
    const prompt = event.suggested_prompt.trim();
    useImageCreateStore.setState((state) => ({
      lastIterationHadPrompt: true,
      params: { ...state.params, prompt },
      currentSession: state.currentSession
        ? {
            ...state.currentSession,
            session: { ...state.currentSession.session, current_prompt: prompt },
          }
        : null,
      entries: [
        ...state.entries,
        {
          id: entryId("prompt"),
          type: "prompt_block",
          role: "provider",
          content: prompt,
          timestamp,
        },
      ],
    }));
    return;
  }
  if (event.kind === "done") {
    useImageCreateStore.setState((state) => {
      const retainedPrompt = state.params.prompt.trim();
      const noPromptNotice: ImageChatEntry[] =
        !state.lastIterationHadPrompt && retainedPrompt
          ? [
              {
                id: entryId("notice"),
                type: "system_notice",
                role: "system",
                content: "本轮未产出新的建议 prompt，已保留上一版",
                timestamp,
              },
            ]
          : [];
      return {
        isBusy: false,
        lastIterationHadPrompt: false,
        entries: [...state.entries, ...noPromptNotice],
        currentSession: state.currentSession
          ? {
              ...state.currentSession,
              session: {
                ...state.currentSession.session,
                last_provider_session_id:
                  event.provider_session_id ??
                  state.currentSession.session.last_provider_session_id,
              },
            }
          : null,
      };
    });
    return;
  }
  if (event.kind === "error") {
    const message = event.error || "图片创作迭代失败";
    useImageCreateStore.setState((state) => ({
      isBusy: false,
      lastIterationHadPrompt: false,
      error: message,
      entries: [
        ...state.entries,
        {
          id: entryId("error"),
          type: "generation_error",
          role: "system",
          content: message,
          timestamp,
        },
      ],
    }));
  }
}

function connectSession(sessionId: string) {
  closeActiveSocket();
  useImageCreateStore.setState({ connectionStatus: "connecting" });
  const socket = new WebSocket(imageCreateChatWebSocketUrl(sessionId));
  activeSocket = socket;
  socket.onopen = () => {
    if (activeSocket === socket) {
      useImageCreateStore.setState({ connectionStatus: "connected", error: null });
    }
  };
  socket.onmessage = (message) => {
    try {
      handleIterationEvent(JSON.parse(message.data) as IterationEvent);
    } catch {
      useImageCreateStore.setState({
        error: "收到无法解析的图片创作事件",
      });
    }
  };
  socket.onerror = () => {
    if (activeSocket === socket) {
      useImageCreateStore.setState({
        connectionStatus: "error",
        isBusy: false,
        error: "图片创作 WebSocket 连接失败",
      });
    }
  };
  socket.onclose = () => {
    if (activeSocket === socket) {
      activeSocket = null;
      useImageCreateStore.setState({
        connectionStatus: "disconnected",
        isBusy: false,
      });
    }
  };
}

export function validateImageCreateBaseUrl(baseUrl: string): void {
  let parsed: URL;
  try {
    parsed = new URL(baseUrl);
  } catch {
    throw new Error("base_url 必须是有效 URL");
  }
  if (parsed.protocol === "https:") {
    return;
  }
  const host = parsed.hostname.toLowerCase();
  const ipv4Parts = host.split(".");
  const isIpv4Loopback =
    ipv4Parts.length === 4 &&
    ipv4Parts.every((part) => /^\d+$/.test(part) && Number(part) <= 255) &&
    Number(ipv4Parts[0]) === 127;
  const loopback =
    parsed.protocol === "http:" &&
    (host === "localhost" ||
      isIpv4Loopback ||
      host === "[::1]" ||
      host === "::1");
  if (!loopback) {
    throw new Error("base_url 必须使用 HTTPS，或使用 localhost/loopback IP");
  }
}

export const useImageCreateStore = create<
  ImageCreateState & ImageCreateActions
>((set, get) => ({
  ...initialState,

  loadSessions: async () => {
    set({ error: null });
    try {
      set({ sessions: await listImageCreateSessions() });
    } catch (error) {
      set({ error: errorMessage(error) });
      throw error;
    }
  },

  openSession: async (sessionId) => {
    generationRequestSequence += 1;
    const requestToken = ++openSessionRequestSequence;
    set({ error: null, isBusy: false, lastIterationHadPrompt: false });
    try {
      const record = await getImageCreateSession(sessionId);
      if (openSessionRequestSequence !== requestToken) {
        return;
      }
      set({
        currentSession: record,
        entries: buildEntries(record),
        params: {
          ...get().params,
          prompt: record.session.current_prompt ?? "",
        },
        referenceImage: null,
      });
      connectSession(sessionId);
    } catch (error) {
      if (openSessionRequestSequence !== requestToken) {
        return;
      }
      set({ error: errorMessage(error) });
      throw error;
    }
  },

  createSession: async (template, providerName) => {
    set({ error: null });
    try {
      const session = await createImageCreateSession({
        template,
        provider_name: providerName,
      });
      const createdSummary: SessionSummary = {
        id: session.id,
        provider_name: session.provider_name,
        template: session.template,
        status: session.status,
        created_at: session.created_at,
        updated_at: session.created_at,
      };
      set((state) => ({ sessions: [createdSummary, ...state.sessions] }));
      await get().openSession(session.id);
      return get().currentSession!;
    } catch (error) {
      set({ error: errorMessage(error) });
      throw error;
    }
  },

  deleteSession: async (sessionId) => {
    set({ error: null });
    try {
      await deleteImageCreateSession(sessionId);
      if (get().currentSession?.session.id === sessionId) {
        closeActiveSocket();
        set({
          currentSession: null,
          entries: [],
          connectionStatus: "disconnected",
          isBusy: false,
        });
      }
      set((state) => ({
        sessions: state.sessions.filter((session) => session.id !== sessionId),
      }));
    } catch (error) {
      set({ error: errorMessage(error) });
      throw error;
    }
  },

  sendMessage: (rawMessage) => {
    const message = rawMessage.trim();
    if (!message || get().isBusy) {
      return;
    }
    if (!activeSocket || activeSocket.readyState !== WebSocket.OPEN) {
      set({ error: "图片创作会话尚未连接" });
      return;
    }
    activeSocket.send(message);
    set((state) => ({
      isBusy: true,
      lastIterationHadPrompt: false,
      error: null,
      entries: [
        ...state.entries,
        {
          id: entryId("user"),
          type: "user_message",
          role: "user",
          content: message,
          timestamp: new Date().toISOString(),
        },
      ],
    }));
  },

  editPromptBlock: (content) => {
    set((state) => ({
      params: { ...state.params, prompt: content },
      currentSession: state.currentSession
        ? {
            ...state.currentSession,
            session: { ...state.currentSession.session, current_prompt: content },
          }
        : null,
    }));
  },

  setParams: (params) => {
    set((state) => ({ params: { ...state.params, ...params } }));
  },

  setReferenceImage: (referenceImage) => set({ referenceImage }),

  generate: async () => {
    const { currentSession, params, referenceImage } = get();
    if (!currentSession || get().isBusy) {
      return;
    }
    const sessionId = currentSession.session.id;
    const requestToken = ++generationRequestSequence;
    set({ isBusy: true, error: null });
    try {
      const request: GenerateImageRequest = {
        ...params,
        reference: referenceImage,
      };
      const result = await requestImageGeneration(sessionId, request);
      if (
        get().currentSession?.session.id !== sessionId ||
        generationRequestSequence !== requestToken
      ) {
        return;
      }
      appendEntry({
        id: entryId("image"),
        type: "generation_image",
        role: "provider",
        content: params.prompt,
        prompt: params.prompt,
        mediaType: result.media_type,
        imageUrl: imageUrl(sessionId, result.image_id),
        timestamp: new Date().toISOString(),
      });
      set({ isBusy: false });
    } catch (error) {
      if (
        get().currentSession?.session.id !== sessionId ||
        generationRequestSequence !== requestToken
      ) {
        return;
      }
      const message = errorMessage(error);
      appendEntry({
        id: entryId("error"),
        type: "generation_error",
        role: "system",
        content: message,
        timestamp: new Date().toISOString(),
      });
      set({ isBusy: false, error: message });
      throw error;
    }
  },

  loadSettings: async () => {
    set({ error: null });
    try {
      const settings = await getImageCreateSettings();
      set((state) => ({
        settings,
        params: { ...state.params, ...settings.defaults },
      }));
    } catch (error) {
      set({ error: errorMessage(error) });
      throw error;
    }
  },

  saveSettings: async (request) => {
    if (request.base_url != null) {
      validateImageCreateBaseUrl(request.base_url);
    }
    set({ error: null });
    try {
      const settings = await updateImageCreateSettings(request);
      set((state) => ({
        settings,
        params: { ...state.params, ...settings.defaults },
      }));
    } catch (error) {
      set({ error: errorMessage(error) });
      throw error;
    }
  },

  disconnect: () => {
    closeActiveSocket();
    set({ connectionStatus: "disconnected", isBusy: false });
  },

  reset: () => {
    closeActiveSocket();
    entrySequence = 0;
    generationRequestSequence += 1;
    set({ ...initialState, params: { ...initialState.params } });
  },
}));
