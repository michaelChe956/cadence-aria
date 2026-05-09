import type {
  ApiError,
  ArtifactContentResponse,
  CreateTaskRequest,
  CreateTaskResponse,
  FileContentResponse,
  FileDiffResponse,
  StopTaskResponse,
  TaskListResponse,
  WebWorkspaceProjection,
} from "./types";

export async function normalizeApiError(response: Response): Promise<ApiError> {
  const body = await response.json().catch(() => ({}));
  return {
    code: typeof body.code === "string" ? body.code : "web_client_error",
    message: typeof body.message === "string" ? body.message : response.statusText,
    details: typeof body.details === "object" && body.details !== null ? body.details : {},
  };
}

async function requestJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: {
      "content-type": "application/json",
      ...(init?.headers ?? {}),
    },
  });
  if (!response.ok) {
    throw await normalizeApiError(response);
  }
  return response.json() as Promise<T>;
}

export function createTask(payload: CreateTaskRequest): Promise<CreateTaskResponse> {
  return requestJson<CreateTaskResponse>("/api/tasks", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function getProjection(taskId?: string, nodeId?: string): Promise<WebWorkspaceProjection> {
  const params = new URLSearchParams();
  if (taskId) params.set("task_id", taskId);
  if (nodeId) params.set("node_id", nodeId);
  const query = params.toString();
  return requestJson<WebWorkspaceProjection>(`/api/projection${query ? `?${query}` : ""}`);
}

export function listTasks(): Promise<TaskListResponse> {
  return requestJson<TaskListResponse>("/api/tasks");
}

export function getArtifactContent(artifactRef: string): Promise<ArtifactContentResponse> {
  return requestJson<ArtifactContentResponse>(`/api/artifacts/${encodeURIComponent(artifactRef)}`);
}

export function getFileContent(path: string): Promise<FileContentResponse> {
  const params = new URLSearchParams({ path });
  return requestJson<FileContentResponse>(`/api/files/content?${params.toString()}`);
}

export function getFileDiff(baseCheckpoint: string, path: string): Promise<FileDiffResponse> {
  const params = new URLSearchParams({ base_checkpoint: baseCheckpoint, path });
  return requestJson<FileDiffResponse>(`/api/files/diff?${params.toString()}`);
}

export function stopTask(taskId: string): Promise<StopTaskResponse> {
  return requestJson<StopTaskResponse>(`/api/tasks/${encodeURIComponent(taskId)}/stop`, {
    method: "POST",
    body: JSON.stringify({}),
  });
}
