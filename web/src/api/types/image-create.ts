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

export const IMAGE_INPUT_FIDELITY_OPTIONS = ["low", "high"] as const;
export type ImageInputFidelity =
  (typeof IMAGE_INPUT_FIDELITY_OPTIONS)[number];

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

export type ImageCreateParams = DefaultImageParams & {
  prompt: string;
  input_fidelity: ImageInputFidelity | null;
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
};

export type ImageGenerationResult = {
  prompt: string;
  params: DefaultImageParams;
  media_type: string;
  image_id: string | null;
  legacy_pending: boolean;
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

export type ImageGenerationResponse = {
  media_type: string;
  image_id: string;
};

export type GenerateImageRequest = ImageCreateParams & {
  reference?: File | null;
};

export type MaskedSettings = {
  base_url: string;
  api_key_masked: string;
  defaults: DefaultImageParams;
};

export type ApiKeyAction = "retain" | "replace" | "clear";

export type SettingsUpdateRequest = {
  base_url?: string | null;
  api_key_action: ApiKeyAction;
  api_key?: string | null;
  defaults?: DefaultImageParams | null;
};
