export const IMAGE_SIZE_OPTIONS = [
  "1024x1024",
  "1536x1024",
  "1024x1536",
  "auto",
] as const;
export type ImageSize = (typeof IMAGE_SIZE_OPTIONS)[number];

export const IMAGE_QUALITY_OPTIONS = ["low", "medium", "high", "auto"] as const;
export type ImageQuality = (typeof IMAGE_QUALITY_OPTIONS)[number];

export const IMAGE_BACKGROUND_OPTIONS = [
  "transparent",
  "opaque",
  "auto",
] as const;
export type ImageBackground = (typeof IMAGE_BACKGROUND_OPTIONS)[number];

export const IMAGE_OUTPUT_FORMAT_OPTIONS = ["png", "jpeg", "webp"] as const;
export type ImageOutputFormat = (typeof IMAGE_OUTPUT_FORMAT_OPTIONS)[number];

/** 图片供应方。与对话执行器的 ImageCreateProvider 无关。 */
export const IMAGE_VENDORS = ["gpt", "minimax"] as const;
export type ImageVendor = (typeof IMAGE_VENDORS)[number];

export const IMAGE_VENDOR_LABELS: Record<ImageVendor, string> = {
  gpt: "GPT",
  minimax: "MiniMax",
};

export type ImageReferenceSource = "none" | "uploaded" | "previous";

export type ImageCreateProviderOption = {
  value: ImageCreateProvider;
  label: string;
  disabled: boolean;
  reason: string | null;
};

export const IMAGE_CREATE_PROVIDER_OPTIONS = [
  "claude_code",
  "codex",
  "pi",
  "kimi_code",
] as const;
export type ImageCreateProvider =
  (typeof IMAGE_CREATE_PROVIDER_OPTIONS)[number];

export const IMAGE_CREATE_PRESET_OPTIONS = [
  "ppt_business_illustration",
  "business_flow_diagram",
  "web_page_ui",
] as const;
export type ImageCreatePreset =
  (typeof IMAGE_CREATE_PRESET_OPTIONS)[number];

export type ImageCreateSessionStatus = "active" | "deleting";

export type ImageCreateTemplateChoice = {
  preset?: ImageCreatePreset | null;
  custom?: string | null;
};

export type DefaultImageParams = {
  size: ImageSize;
  quality: ImageQuality;
  background: ImageBackground;
  output_format: ImageOutputFormat;
};

export const DEFAULT_IMAGE_PARAMS: DefaultImageParams = {
  size: "auto",
  quality: "auto",
  background: "auto",
  output_format: "png",
};

/** 参数面板状态。input_fidelity 已移除。 */
export type ImageCreateParams = DefaultImageParams & {
  prompt: string;
  /** 模型 id：用户从已配置且可用的供应方清单中手动选择，必填非空。 */
  model: string;
  use_previous: boolean;
};

export type ImageCreateSession = {
  id: string;
  provider_name: ImageCreateProvider;
  template: ImageCreateTemplateChoice;
  last_provider_session_id: string | null;
  current_prompt: string | null;
  status: ImageCreateSessionStatus;
  created_at: string;
};

export type SessionSummary = {
  id: string;
  provider_name: ImageCreateProvider;
  template: ImageCreateTemplateChoice;
  status: ImageCreateSessionStatus;
  created_at: string;
  updated_at: string;
};

export type ImageCreateChatMessage = {
  role: string;
  content: string;
  ts: string;
};

export type ImageCreatePromptBlock = {
  content: string;
  version: number;
  /** 提示词块产生时间（ISO）。旧会话记录没有该字段，读出为 undefined。 */
  ts?: string | null;
};

/** 会话记录中的生成结果：旧记录没有后 4 个字段。 */
export type ImageGenerationResult = {
  prompt: string;
  params: DefaultImageParams;
  media_type: string;
  image_id: string | null;
  legacy_pending: boolean;
  vendor?: ImageVendor;
  model?: string;
  reference_source?: ImageReferenceSource;
  ts: string;
};

export type ImageCreateSessionEvent = {
  kind: string;
  message: string;
  ts: string;
};

export type SessionRecord = {
  session: ImageCreateSession;
  messages: ImageCreateChatMessage[];
  prompt_blocks: ImageCreatePromptBlock[];
  generation_results: ImageGenerationResult[];
  events: ImageCreateSessionEvent[];
  generation: number;
};

export type CreateSessionRequest = {
  template: ImageCreateTemplateChoice;
  provider_name: ImageCreateProvider;
};

export type IterationEvent = {
  kind: "text" | "prompt" | "done" | "error" | string;
  text: string | null;
  suggested_prompt: string | null;
  provider_session_id: string | null;
  error: string | null;
};

/** 生成成功响应。 */
export type ImageGenerationResponse = {
  media_type: string;
  image_id: string;
  vendor: ImageVendor;
  model: string;
  reference_source: ImageReferenceSource;
};

/** 不变：GenerateImageRequest = ImageCreateParams & { reference?: File | null } */
export type GenerateImageRequest = ImageCreateParams & {
  reference?: File | null;
};

/** 设置接口中的单家视图（已脱敏）。 */
export type MaskedVendorSettings = {
  base_url: string;
  api_key_masked: string;
  models: string[];
  recommended_models: string[];
  available: boolean;
};

export type MaskedSettings = {
  vendors: Record<ImageVendor, MaskedVendorSettings>;
  defaults: DefaultImageParams;
};

/** 不变：ApiKeyAction = "retain" | "replace" | "clear" */
export type ApiKeyAction = "retain" | "replace" | "clear";

/** 单家更新。 */
export type VendorSettingsUpdate = {
  base_url?: string | null;
  api_key_action: ApiKeyAction;
  api_key?: string | null;
  models?: string[] | null;
};

/** 某家缺省 = 该家全部保留。 */
export type SettingsUpdateRequest = {
  vendors?: Partial<Record<ImageVendor, VendorSettingsUpdate>> | null;
  defaults?: DefaultImageParams | null;
};

/** 连通性探测的单步状态。skipped = 该步不适用，已跳到下一步。 */
export type ConnectivityStepStatus = "ok" | "skipped" | "failed";

/** 连通性探测的单步结果。detail 为中文说明，不含 API Key 明文。 */
export type ConnectivityStep = {
  key: string;
  label: string;
  status: ConnectivityStepStatus;
  detail: string;
};

/**
 * 连通性探测请求。
 * base_url 只填主机根路径（如 https://api.openai.com），系统会自动拼 /v1/images/generations。
 * api_key 为空串表示沿用该家已保存的 key。
 */
export type ConnectivityProbeRequest = {
  vendor: ImageVendor;
  base_url: string;
  api_key: string;
  models: string[];
};

/** 连通性探测结果：两步都通过时 ok 为 true。 */
export type ConnectivityProbeResult = {
  vendor: ImageVendor;
  ok: boolean;
  message: string;
  steps: ConnectivityStep[];
  /** 配置了但网关模型列表里没有的模型 id。 */
  missing_models: string[];
  /** 网关模型列表：/v1/models 步骤成功时返回的全部模型 id（去重、保序）。 */
  available_models: string[];
  duration_ms: number;
};
