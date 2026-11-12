import { ApiRequestError } from "./client";
import type {
  CreateSessionRequest,
  GenerateImageRequest,
  ImageCreateSession,
  ImageGenerationResponse,
  MaskedSettings,
  SessionRecord,
  SessionSummary,
  SettingsUpdateRequest,
} from "./types/image-create";

const API_ROOT = "/api/image-create";

type ImageCreateErrorBody = {
  code?: unknown;
  message?: unknown;
  details?: unknown;
  error?: unknown;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

async function imageCreateApiError(response: Response): Promise<ApiRequestError> {
  const body: unknown = await response.json().catch(() => ({}));
  const errorBody: ImageCreateErrorBody = isRecord(body) ? body : {};
  const nestedError = isRecord(errorBody.error) ? errorBody.error : null;
  const message =
    (typeof errorBody.error === "string" && errorBody.error) ||
    (nestedError &&
      typeof nestedError.message === "string" &&
      nestedError.message) ||
    (typeof errorBody.message === "string" && errorBody.message) ||
    response.statusText ||
    `图片创作请求失败（HTTP ${response.status}）`;

  return new ApiRequestError({
    code: typeof errorBody.code === "string" ? errorBody.code : "image_create_error",
    message,
    details: isRecord(errorBody.details) ? errorBody.details : {},
  });
}

async function requestJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    credentials: "same-origin",
    headers: {
      "content-type": "application/json",
      ...(init?.headers ?? {}),
    },
  });
  if (!response.ok) {
    throw await imageCreateApiError(response);
  }
  const text = await response.text();
  if (!text.trim()) {
    return undefined as T;
  }
  return JSON.parse(text) as T;
}

function sessionPath(sessionId: string): string {
  return `${API_ROOT}/sessions/${encodeURIComponent(sessionId)}`;
}

export function imageUrl(sessionId: string, imageId: string): string {
  return `${sessionPath(sessionId)}/images/${encodeURIComponent(imageId)}`;
}

export function listImageCreateSessions(): Promise<SessionSummary[]> {
  return requestJson<SessionSummary[]>(`${API_ROOT}/sessions`);
}

export function createImageCreateSession(
  request: CreateSessionRequest,
): Promise<ImageCreateSession> {
  return requestJson<ImageCreateSession>(`${API_ROOT}/sessions`, {
    method: "POST",
    body: JSON.stringify(request),
  });
}

export function getImageCreateSession(sessionId: string): Promise<SessionRecord> {
  return requestJson<SessionRecord>(sessionPath(sessionId));
}

export function deleteImageCreateSession(sessionId: string): Promise<void> {
  return requestJson<void>(sessionPath(sessionId), { method: "DELETE" });
}

export function getImageCreateSettings(): Promise<MaskedSettings> {
  return requestJson<MaskedSettings>(`${API_ROOT}/settings`);
}

export function updateImageCreateSettings(
  request: SettingsUpdateRequest,
): Promise<MaskedSettings> {
  return requestJson<MaskedSettings>(`${API_ROOT}/settings`, {
    method: "PUT",
    body: JSON.stringify(request),
  });
}

export async function generateImage(
  sessionId: string,
  request: GenerateImageRequest,
): Promise<ImageGenerationResponse> {
  // 模型由用户手动选择，必填非空：空值不发出请求。
  if (!request.model.trim()) {
    throw new Error("请先选择图片生成模型");
  }
  const form = new FormData();
  form.append("prompt", request.prompt);
  form.append("size", request.size);
  form.append("quality", request.quality);
  form.append("background", request.background);
  form.append("output_format", request.output_format);
  form.append("model", request.model);
  form.append("use_previous", request.use_previous ? "true" : "false");
  if (request.reference) {
    form.append("reference", request.reference);
  }

  const response = await fetch(`${sessionPath(sessionId)}/generate`, {
    method: "POST",
    credentials: "same-origin",
    body: form,
  });
  if (!response.ok) {
    throw await imageCreateApiError(response);
  }
  return (await response.json()) as ImageGenerationResponse;
}

export function imageCreateChatWebSocketUrl(
  sessionId: string,
  location: Pick<Location, "protocol" | "host"> = window.location,
): string {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${location.host}${sessionPath(sessionId)}/chat`;
}
