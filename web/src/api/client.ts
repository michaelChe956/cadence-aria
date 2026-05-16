import type {
  ApiError,
  ArtifactContentResponse,
  CreateIssueRequest,
  CreateProductIssueRequest,
  CreateWorkspaceRequest,
  CreateRepositoryRequest,
  CreateTaskRequest,
  CreateTaskResponse,
  FileContentResponse,
  FileDiffResponse,
  IssueLifecycleResponse,
  Issue,
  IssueListResponse,
  ProductIssue,
  ProductIssueListResponse,
  Project,
  Repository,
  RepositoryListResponse,
  RollbackPreviewResponse,
  StartIssueRequest,
  StartIssueResponse,
  StartProductIssueRequest,
  StartProductIssueResponse,
  StopTaskResponse,
  StorySpec,
  TaskListResponse,
  WebWorkspaceProjection,
  Workspace,
  WorkspaceListResponse,
  WorkspaceSession,
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

export function deleteWorkspace(workspaceId: string): Promise<{ status: string }> {
  return requestJson<{ status: string }>(`/api/workspaces/${encodeURIComponent(workspaceId)}`, {
    method: "DELETE",
  });
}

export function listProjects(): Promise<{ projects: Project[] }> {
  return requestJson<{ projects: Project[] }>("/api/projects");
}

export function createProject(payload: {
  name: string;
  description?: string | null;
}): Promise<Project> {
  return requestJson<Project>("/api/projects", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function deleteProject(projectId: string): Promise<{ status: string }> {
  return requestJson<{ status: string }>(`/api/projects/${encodeURIComponent(projectId)}`, {
    method: "DELETE",
  });
}

export function listRepositories(projectId: string): Promise<RepositoryListResponse> {
  return requestJson<RepositoryListResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/repositories`,
  );
}

export function createRepository(
  projectId: string,
  payload: CreateRepositoryRequest,
): Promise<Repository> {
  return requestJson<Repository>(`/api/projects/${encodeURIComponent(projectId)}/repositories`, {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function deleteRepository(
  projectId: string,
  repositoryId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/repositories/${encodeURIComponent(repositoryId)}`,
    {
      method: "DELETE",
    },
  );
}

export function listProductIssues(projectId: string): Promise<ProductIssueListResponse> {
  return requestJson<ProductIssueListResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/issues`,
  );
}

export function createProductIssue(
  projectId: string,
  payload: CreateProductIssueRequest,
): Promise<ProductIssue> {
  return requestJson<ProductIssue>(`/api/projects/${encodeURIComponent(projectId)}/issues`, {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function deleteProductIssue(
  projectId: string,
  issueId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}`,
    {
      method: "DELETE",
    },
  );
}

export function startProductIssue(
  projectId: string,
  issueId: string,
  payload: StartProductIssueRequest,
): Promise<StartProductIssueResponse> {
  return requestJson<StartProductIssueResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/start`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function getIssueLifecycle(
  issueId: string,
  projectId: string,
): Promise<IssueLifecycleResponse> {
  return requestJson<IssueLifecycleResponse>(
    `/api/issues/${encodeURIComponent(issueId)}/lifecycle?project_id=${encodeURIComponent(projectId)}`,
  );
}

export function generateStorySpecs(
  projectId: string,
  issueId: string,
  payload: { title: string },
): Promise<{ story_specs: StorySpec[]; workspace_session: WorkspaceSession }> {
  return requestJson<{ story_specs: StorySpec[]; workspace_session: WorkspaceSession }>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/story-specs:generate`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function sendWorkspaceSessionMessage(
  sessionId: string,
  payload: { role: string; content: string },
): Promise<WorkspaceSession> {
  return requestJson<WorkspaceSession>(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/message`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function runWorkspaceSessionNext(sessionId: string): Promise<WorkspaceSession> {
  return requestJson<WorkspaceSession>(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/run-next`,
    {
      method: "POST",
    },
  );
}

export function confirmWorkspaceSession(
  sessionId: string,
  payload: { confirmed_by: string },
): Promise<WorkspaceSession> {
  return requestJson<WorkspaceSession>(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/confirm`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
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

export function deleteIssue(issueId: string): Promise<{ status: string }> {
  return requestJson<{ status: string }>(`/api/issues/${encodeURIComponent(issueId)}`, {
    method: "DELETE",
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
  payload: {
    checkpoint_id: string;
    prompt: string;
    policy_override?: string | null;
    provider_type?: string | null;
  },
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
