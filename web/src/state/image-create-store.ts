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
  testImageVendorConnectivity,
  updateImageCreateSettings,
} from "../api/image-create";
import {
  DEFAULT_IMAGE_PARAMS,
  IMAGE_VENDORS,
  type ConnectivityProbeResult,
  type GenerateImageRequest,
  type ImageCreateParams,
  type ImageCreateProvider,
  type ImageCreateTemplateChoice,
  type IterationEvent,
  type MaskedSettings,
  type MaskedVendorSettings,
  type SessionRecord,
  type SessionSummary,
  type SettingsUpdateRequest,
  type ImageVendor,
} from "../api/types/image-create";
import type { ImageChatEntry } from "./image-create-entries";
import {
  initialVendorModel,
  mergeCandidatePool,
  normalizeModels,
  orderByPool,
  refreshCandidatePool,
  type ImageVendorModelPool,
  type ModelCandidatesRefresh,
} from "./image-create-model-pool";

export type ImageCreateConnectionStatus =
  | "disconnected"
  | "connecting"
  | "connected"
  | "error";

/** 连通性探测阶段：idle 未探测、testing 探测中、settled 已有结论。 */
export type ImageVendorConnectivityPhase = "idle" | "testing" | "settled";

/** 单家供应方的连通性探测状态。error 与 result 内均不含 API Key 明文。 */
export type ImageVendorConnectivityState = {
  phase: ImageVendorConnectivityPhase;
  result: ConnectivityProbeResult | null;
  error: string | null;
};

/** 单家供应方的模型池：网关返回的模型与可勾选的候选池。 */
export type { ImageVendorModelPool, ModelCandidatesRefresh } from "./image-create-model-pool";

export type ImageCreateState = {
  sessions: SessionSummary[];
  currentSession: SessionRecord | null;
  entries: ImageChatEntry[];
  params: ImageCreateParams;
  referenceImage: File | null;
  settings: MaskedSettings | null;
  /** 各家供应方的连通性探测结果，按家存放。 */
  connectivity: Record<ImageVendor, ImageVendorConnectivityState>;
  /** 各家供应方的模型池（网关列表与候选池），按家存放。 */
  modelPools: Record<ImageVendor, ImageVendorModelPool>;
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
  /**
   * 探测某家供应方的连通性：只发探测请求，不生成图片、不计费。
   * 入参是弹窗里的草稿值（base_url / key / 模型清单），与已保存设置无关。
   */
  testVendorConnectivity: (
    vendor: ImageVendor,
    input: { baseUrl: string; apiKey: string; models: string[] },
  ) => Promise<void>;
  /**
   * 覆盖该家的候选池（载入已保存的模型清单时用），返回去重后的清单。
   */
  setVendorModels: (vendor: ImageVendor, models: readonly string[]) => string[];
  /**
   * 用最近一次探测到的网关模型刷新该家的候选池：
   * - 候选池 = 网关模型（网关顺序在前）∪ 草稿清单里已有的；
   * - 勾选集合 = 草稿清单原样（探测不新增勾选，也不取消勾选）；
   *   草稿清单为空且网关返回非空时取网关全量，超过单家上限只保留前 N 个；
   * - 已被另一家勾选的模型 id 不进入候选池。
   */
  refreshModelCandidates: (
    vendor: ImageVendor,
    current: readonly string[],
    otherSelected: readonly string[],
  ) => ModelCandidatesRefresh;
  /**
   * 把额外的模型 id 并入该家的候选池并勾选（恢复推荐清单、自定义模型），
   * 返回按候选池顺序排列的勾选集合。
   */
  mergeVendorModels: (
    vendor: ImageVendor,
    current: readonly string[],
    incoming: readonly string[],
  ) => string[];
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
    // 未加载设置前没有任何可用供应方，模型为空串。
    model: "",
    use_previous: true,
  },
  referenceImage: null,
  settings: null,
  connectivity: {
    gpt: { phase: "idle", result: null, error: null },
    minimax: { phase: "idle", result: null, error: null },
  },
  modelPools: {
    gpt: { gateway: [], candidates: [] },
    minimax: { gateway: [], candidates: [] },
  },
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

/** 抹掉文本里的 API Key 明文，避免凭据被带进界面或状态。 */
function redactApiKey(text: string, apiKey: string): string {
  const key = apiKey.trim();
  if (!key) {
    return text;
  }
  return text.split(key).join("***");
}

/** 抹掉探测结果文本里的 API Key 明文。 */
function redactProbeResult(
  result: ConnectivityProbeResult,
  apiKey: string,
): ConnectivityProbeResult {
  return {
    ...result,
    message: redactApiKey(result.message, apiKey),
    steps: result.steps.map((step) => ({
      ...step,
      detail: redactApiKey(step.detail, apiKey),
    })),
  };
}

/**
 * 该家是否已配置凭据：Base URL 与 API Key 均非空。
 * 注意：形参是该家的 MaskedVendorSettings 视图（掩码后的单家设置），不是供应方 id。
 */
export function isImageVendorConfigured(
  vendorSettings: MaskedVendorSettings | undefined,
): boolean {
  if (!vendorSettings) {
    return false;
  }
  return vendorSettings.base_url.trim() !== "" && vendorSettings.api_key_masked.trim() !== "";
}

/** 已配置凭据的供应方列表，供模型下拉过滤使用。 */
export function configuredImageVendors(
  settings: MaskedSettings | null,
): ImageVendor[] {
  if (!settings) {
    return [];
  }
  return IMAGE_VENDORS.filter((vendor) =>
    isImageVendorConfigured(settings.vendors[vendor]),
  );
}

/**
 * 已配置凭据且清单非空的供应方列表：它们的模型才会出现在下拉里。
 */
export function availableImageVendors(
  settings: MaskedSettings | null,
): ImageVendor[] {
  if (!settings) {
    return [];
  }
  return IMAGE_VENDORS.filter(
    (vendor) =>
      isImageVendorConfigured(settings.vendors[vendor]) &&
      settings.vendors[vendor].models.length > 0,
  );
}

/**
 * 初始选中项：第一个可用供应方的清单首个 id；没有可用供应方时为空串。
 */
function initialImageModel(settings: MaskedSettings | null): string {
  if (!settings) {
    return "";
  }
  for (const vendor of availableImageVendors(settings)) {
    const model = initialVendorModel(settings.vendors[vendor]);
    if (model !== "") {
      return model;
    }
  }
  return "";
}

/**
 * 所选模型若不再属于任何可用供应方的清单，则重置为初始选中项。
 * 设置变化后调用，避免留下已下线的模型。
 */
function resolveSelectedModel(
  settings: MaskedSettings | null,
  model: string,
): string {
  if (!settings || !model) {
    return initialImageModel(settings);
  }
  for (const vendor of availableImageVendors(settings)) {
    if (settings.vendors[vendor].models.includes(model)) {
      return model;
    }
  }
  return initialImageModel(settings);
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
          // 旧会话记录没有这三个字段，读出为 undefined。
          vendor: result.vendor,
          model: result.model,
          referenceSource: result.reference_source,
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
    // 模型由用户手动选择：未选择时不发请求，直接给出可读错误。
    if (!params.model.trim()) {
      set({ error: "请先选择图片生成模型" });
      throw new Error("请先选择图片生成模型");
    }
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
        vendor: result.vendor,
        model: result.model,
        referenceSource: result.reference_source,
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
        params: {
          ...state.params,
          ...settings.defaults,
          model: resolveSelectedModel(settings, state.params.model),
        },
      }));
    } catch (error) {
      set({ error: errorMessage(error) });
      throw error;
    }
  },

  saveSettings: async (request) => {
    // 逐家校验本次提交的 Base URL；两家都未出现时不做前置校验。
    for (const vendor of IMAGE_VENDORS) {
      const baseUrl = request.vendors?.[vendor]?.base_url;
      if (baseUrl != null && baseUrl.trim() !== "") {
        validateImageCreateBaseUrl(baseUrl);
      }
    }
    set({ error: null });
    try {
      const settings = await updateImageCreateSettings(request);
      set((state) => ({
        settings,
        params: {
          ...state.params,
          ...settings.defaults,
          model: resolveSelectedModel(settings, state.params.model),
        },
      }));
    } catch (error) {
      set({ error: errorMessage(error) });
      throw error;
    }
  },

  testVendorConnectivity: async (vendor, input) => {
    const baseUrl = input.baseUrl.trim();
    const apiKey = input.apiKey.trim();
    const models = input.models
      .map((model) => model.trim())
      .filter((model) => model !== "");

    const fail = (message: string) => {
      set((state) => ({
        connectivity: {
          ...state.connectivity,
          [vendor]: { phase: "settled", result: null, error: message },
        },
      }));
    };

    if (baseUrl === "") {
      fail("请先填写 Base URL 再测试连通");
      return;
    }
    try {
      validateImageCreateBaseUrl(baseUrl);
    } catch (reason) {
      fail(redactApiKey(errorMessage(reason), apiKey));
      return;
    }

    set((state) => ({
      connectivity: {
        ...state.connectivity,
        [vendor]: { phase: "testing", result: null, error: null },
      },
    }));
    try {
      const result = await testImageVendorConnectivity({
        vendor,
        base_url: baseUrl,
        api_key: apiKey,
        models,
      });
      // 网关模型列表按家存放：/v1/models 步骤成功时非空。
      const availableModels = result.available_models ?? [];
      set((state) => ({
        connectivity: {
          ...state.connectivity,
          [vendor]: {
            phase: "settled",
            result: redactProbeResult(result, apiKey),
            error: null,
          },
        },
        modelPools: {
          ...state.modelPools,
          [vendor]: { ...state.modelPools[vendor], gateway: availableModels },
        },
      }));
    } catch (reason) {
      fail(redactApiKey(errorMessage(reason), apiKey));
    }
  },

  setVendorModels: (vendor, models) => {
    const candidates = normalizeModels(models);
    set((state) => ({
      modelPools: {
        ...state.modelPools,
        [vendor]: { ...state.modelPools[vendor], candidates },
      },
    }));
    return candidates;
  },

  refreshModelCandidates: (vendor, current, otherSelected) => {
    const { gateway, candidates } = get().modelPools[vendor];
    if (gateway.length === 0) {
      // 网关没有模型列表（未探测或该步失败）：候选池与勾选都不变。
      return {
        candidates,
        selected: orderByPool(candidates, normalizeModels(current)),
        skipped: [],
        truncated: null,
      };
    }
    const refresh = refreshCandidatePool(gateway, current, otherSelected);
    set((state) => ({
      modelPools: {
        ...state.modelPools,
        [vendor]: { ...state.modelPools[vendor], candidates: refresh.candidates },
      },
    }));
    return refresh;
  },

  mergeVendorModels: (vendor, current, incoming) => {
    const pool = get().modelPools[vendor].candidates;
    const merged = mergeCandidatePool(pool, current, incoming);
    set((state) => ({
      modelPools: {
        ...state.modelPools,
        [vendor]: { ...state.modelPools[vendor], candidates: merged.candidates },
      },
    }));
    return merged.selected;
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
