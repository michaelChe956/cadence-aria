import type {
  ApiError,
  ArtifactContentResponse,
  CreateIssueRequest,
  CreateWorkspaceRequest,
  CreateTaskRequest,
  CreateTaskResponse,
  FileContentResponse,
  FileDiffResponse,
  Issue,
  IssueListResponse,
  RollbackPreviewResponse,
  StartIssueRequest,
  StartIssueResponse,
  StopTaskResponse,
  TaskListResponse,
  WebWorkspaceProjection,
  Workspace,
  WorkspaceListResponse,
} from "./types";

export class ApiRequestError extends Error implements ApiError {
  code: string;
  details: Record<string, unknown>;

  constructor(error: ApiError) {
    super(error.message);
    this.name = "ApiRequestError";
    this.code = error.code;
    this.details = error.details;
  }
}

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
    throw new ApiRequestError(await normalizeApiError(response));
  }
  return response.json() as Promise<T>;
}

export function createTask(payload: CreateTaskRequest): Promise<CreateTaskResponse> {
  return requestJson<CreateTaskResponse>("/api/tasks", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function listWorkspaces(): Promise<WorkspaceListResponse> {
  return requestJson<WorkspaceListResponse>("/api/workspaces");
}

export function createWorkspace(payload: CreateWorkspaceRequest): Promise<Workspace> {
  return requestJson<Workspace>("/api/workspaces", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function listIssues(): Promise<IssueListResponse> {
  return requestJson<IssueListResponse>("/api/issues");
}

export function createIssue(payload: CreateIssueRequest): Promise<Issue> {
  return requestJson<Issue>("/api/issues", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function startIssue(
  issueId: string,
  payload: StartIssueRequest,
): Promise<StartIssueResponse> {
  return requestJson<StartIssueResponse>(`/api/issues/${encodeURIComponent(issueId)}/start`, {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function getProjection(
  taskId?: string,
  nodeId?: string,
  workspaceId?: string,
): Promise<WebWorkspaceProjection> {
  const params = new URLSearchParams();
  if (taskId) params.set("task_id", taskId);
  if (nodeId) params.set("node_id", nodeId);
  if (workspaceId) params.set("workspace_id", workspaceId);
  const query = params.toString();
  return requestJson<WebWorkspaceProjection>(`/api/projection${query ? `?${query}` : ""}`);
}

export function listTasks(): Promise<TaskListResponse> {
  return requestJson<TaskListResponse>("/api/tasks");
}

export function getArtifactContent(
  artifactRef: string,
  workspaceId?: string,
): Promise<ArtifactContentResponse> {
  const params = new URLSearchParams();
  if (workspaceId) params.set("workspace_id", workspaceId);
  const query = params.toString();
  return requestJson<ArtifactContentResponse>(
    `/api/artifacts/${encodeURIComponent(artifactRef)}${query ? `?${query}` : ""}`,
  );
}

export function getFileContent(path: string, workspaceId?: string): Promise<FileContentResponse> {
  const params = new URLSearchParams({ path });
  if (workspaceId) params.set("workspace_id", workspaceId);
  return requestJson<FileContentResponse>(`/api/files/content?${params.toString()}`);
}

export function getFileDiff(
  baseCheckpoint: string,
  path: string,
  workspaceId?: string,
): Promise<FileDiffResponse> {
  const params = new URLSearchParams({ base_checkpoint: baseCheckpoint, path });
  if (workspaceId) params.set("workspace_id", workspaceId);
  return requestJson<FileDiffResponse>(`/api/files/diff?${params.toString()}`);
}

export function stopTask(taskId: string, workspaceId?: string): Promise<StopTaskResponse> {
  return requestJson<StopTaskResponse>(taskActionPath(taskId, "stop", workspaceId), {
    method: "POST",
    body: JSON.stringify({}),
  });
}

export function confirmTask(
  taskId: string,
  payload: { checkpoint_id: string; prompt: string; policy_override?: string | null },
  workspaceId?: string,
) {
  return requestJson<{ status: string; node_id: string; turn_id: string }>(
    taskActionPath(taskId, "confirm", workspaceId),
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function advanceTask(taskId: string, workspaceId?: string) {
  return requestJson<unknown>(taskActionPath(taskId, "advance", workspaceId), {
    method: "POST",
    body: JSON.stringify({}),
  });
}

export function rollbackPreview(taskId: string, checkpointId: string, workspaceId?: string) {
  return requestJson<RollbackPreviewResponse>(
    taskActionPath(taskId, "rollback/preview", workspaceId),
    {
      method: "POST",
      body: JSON.stringify({ checkpoint_id: checkpointId }),
    },
  );
}

export function rollbackTask(
  taskId: string,
  payload: { checkpoint_id: string; force_when_dirty: boolean },
  workspaceId?: string,
) {
  return requestJson<{ status: string; checkpoint_id: string }>(
    taskActionPath(taskId, "rollback", workspaceId),
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

function taskActionPath(taskId: string, action: string, workspaceId?: string) {
  const params = new URLSearchParams();
  if (workspaceId) params.set("workspace_id", workspaceId);
  const query = params.toString();
  return `/api/tasks/${encodeURIComponent(taskId)}/${action}${query ? `?${query}` : ""}`;
}
