import type {
  ApiError,
  ArtifactContentResponse,
  CodingAttempt,
  CodingAttemptAddress,
  CodingAttemptDiffResponse,
  CodingAttemptSnapshotResponse,
  CreateProductIssueRequest,
  CreateRepositoryRequest,
  GenerateDesignSpecsRequest,
  GenerateDesignSpecsResponse,
  GenerateStorySpecsRequest,
  GenerateStorySpecsResponse,
  GenerateWorkItemsResponse,
  IssueLifecycleResponse,
  LeaseDiagnosticsResponse,
  PrepareWorkItemPlanRequest,
  PrepareWorkItemPlanResponse,
  ProductIssue,
  ProductIssueListResponse,
  Project,
  ProviderHealthResponse,
  Repository,
  RepositoryRegistrationErrorDetails,
  RepositoryDeletionReceipt,
  RepositoryInitializationOperationSnapshot,
  RepositoryBranchListResponse,
  RepositoryListResponse,
  TakeoverResponse,
  WorkspaceSession,
  WorkItemExecutionPlan,
} from "./types";

export class ApiRequestError extends Error implements ApiError {
  code: string;
  details: RepositoryRegistrationErrorDetails;

  constructor(error: ApiError) {
    super(error.message);
    this.name = "ApiRequestError";
    this.code = error.code;
    this.details = error.details;
  }
}

export async function normalizeApiError(response: Response): Promise<ApiError> {
  const body: unknown = await response.json().catch(() => ({}));
  const error = isRecord(body) ? body : {};
  return {
    code: typeof error.code === "string" ? error.code : "web_client_error",
    message:
      typeof error.message === "string" ? error.message : response.statusText,
    details: isRecord(error.details) ? error.details : {},
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
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
  const text = await response.text();
  if (!text.trim()) {
    return undefined as T;
  }
  return JSON.parse(text) as T;
}

export function listProjects(): Promise<{ projects: Project[] }> {
  return requestJson<{ projects: Project[] }>("/api/projects");
}

export function workspaceSessionWebSocketUrl(
  sessionId: string,
  location: Pick<Location, "protocol" | "host"> = window.location,
): string {
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  return `${protocol}//${location.host}/api/workspace-sessions/${encodeURIComponent(sessionId)}/ws`;
}

export function takeoverWorkspaceSession(sessionId: string): Promise<TakeoverResponse> {
  return requestJson<TakeoverResponse>(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/takeover`,
    { method: "POST" },
  );
}

// REQ-DLS-03：STALE 手动错误面引用最近租约转移事件的数据源（只读诊断端点）。
// 失败（网络/非 2xx）抛 ApiRequestError，由调用方静默降级——诊断不可用
// 不阻塞错误面的恢复动作。
export function getWorkspaceSessionLeaseDiagnostics(
  sessionId: string,
): Promise<LeaseDiagnosticsResponse> {
  return requestJson<LeaseDiagnosticsResponse>(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/lease-diagnostics`,
  );
}

// F-20：story/design AuthorConfirm 的 approve 设计通路（WS confirm 帧在该阶段被
// 矩阵拒收，wave2-f18-report §5）；confirmed_by 为审计署名（服务端落 system 消息）。
// F-31 纠偏：评审改为用户可选——withReview=true 附 with_review（服务端接管进入
// ReviewOnly 评审轮，响应非 confirmed）；缺省不带该字段=直接定稿。
export function confirmWorkspaceSession(
  sessionId: string,
  confirmedBy = "user",
  withReview = false,
): Promise<WorkspaceSession> {
  return requestJson<WorkspaceSession>(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/confirm`,
    {
      method: "POST",
      body: JSON.stringify({
        confirmed_by: confirmedBy,
        ...(withReview ? { with_review: true } : {}),
      }),
    },
  );
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
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}`,
    {
      method: "DELETE",
    },
  );
}

export function listRepositories(
  projectId: string,
): Promise<RepositoryListResponse> {
  return requestJson<RepositoryListResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/repositories`,
  );
}

export function createRepository(
  projectId: string,
  payload: CreateRepositoryRequest,
): Promise<RepositoryInitializationOperationSnapshot> {
  return requestJson<RepositoryInitializationOperationSnapshot>(
    `/api/projects/${encodeURIComponent(projectId)}/repositories`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function getRepositoryInitialization(
  projectId: string,
  operationId: string,
): Promise<RepositoryInitializationOperationSnapshot> {
  return requestJson<RepositoryInitializationOperationSnapshot>(
    `/api/projects/${encodeURIComponent(projectId)}/repository-initializations/${encodeURIComponent(operationId)}`,
  );
}

export function getProviderStatus(): Promise<ProviderHealthResponse> {
  return requestJson<ProviderHealthResponse>("/api/providers/status");
}

export function recheckProviders(): Promise<ProviderHealthResponse> {
  return requestJson<ProviderHealthResponse>("/api/providers/recheck", {
    method: "POST",
  });
}

export function deleteRepository(
  projectId: string,
  repositoryId: string,
  operationId: string,
): Promise<RepositoryDeletionReceipt> {
  return requestJson<RepositoryDeletionReceipt>(
    `/api/projects/${encodeURIComponent(projectId)}/repositories/${encodeURIComponent(repositoryId)}`,
    {
      method: "DELETE",
      headers: { "Idempotency-Key": operationId },
    },
  );
}

export function listProductIssues(
  projectId: string,
): Promise<ProductIssueListResponse> {
  return requestJson<ProductIssueListResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/issues`,
  );
}

export function createProductIssue(
  projectId: string,
  payload: CreateProductIssueRequest,
): Promise<ProductIssue> {
  return requestJson<ProductIssue>(
    `/api/projects/${encodeURIComponent(projectId)}/issues`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

/// REQ-PIB-01：仓库本地分支列表（服务端默认链；UI 不自行推导默认值）。
export function listRepositoryBranches(
  projectId: string,
  repositoryId: string,
): Promise<RepositoryBranchListResponse> {
  return requestJson<RepositoryBranchListResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/repositories/${encodeURIComponent(repositoryId)}/branches`,
  );
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

export function deleteStorySpec(
  projectId: string,
  issueId: string,
  storySpecId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/story-specs/${encodeURIComponent(storySpecId)}`,
    {
      method: "DELETE",
    },
  );
}

export function deleteDesignSpec(
  projectId: string,
  issueId: string,
  designSpecId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/design-specs/${encodeURIComponent(designSpecId)}`,
    {
      method: "DELETE",
    },
  );
}

export function deleteWorkItem(
  projectId: string,
  issueId: string,
  workItemId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-items/${encodeURIComponent(workItemId)}`,
    {
      method: "DELETE",
    },
  );
}

export function deleteWorkItemPlan(
  projectId: string,
  issueId: string,
  planId: string,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-item-plans/${encodeURIComponent(planId)}`,
    {
      method: "DELETE",
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
  payload: GenerateStorySpecsRequest,
): Promise<GenerateStorySpecsResponse> {
  return requestJson<GenerateStorySpecsResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/story-specs:generate`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function generateDesignSpecs(
  projectId: string,
  issueId: string,
  payload: GenerateDesignSpecsRequest,
): Promise<GenerateDesignSpecsResponse> {
  return requestJson<GenerateDesignSpecsResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/design-specs:generate`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function prepareWorkItemPlan(
  projectId: string,
  issueId: string,
  payload: PrepareWorkItemPlanRequest,
): Promise<PrepareWorkItemPlanResponse> {
  return requestJson<PrepareWorkItemPlanResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-item-plans:prepare`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function createCodingAttempt(
  projectId: string,
  issueId: string,
  workItemId: string,
): Promise<CodingAttempt> {
  return requestJson<CodingAttempt>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-items/${encodeURIComponent(workItemId)}/coding-attempts`,
    {
      method: "POST",
      body: JSON.stringify({}),
    },
  );
}

export function createGroupCodingAttempt(
  projectId: string,
  issueId: string,
  planId: string,
): Promise<CodingAttempt> {
  return requestJson<CodingAttempt>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-item-plans/${encodeURIComponent(planId)}/coding-attempts`,
    {
      method: "POST",
      body: JSON.stringify({}),
    },
  );
}

function codingAttemptApiPath(address: CodingAttemptAddress): string {
  return `/api/projects/${encodeURIComponent(address.projectId)}/issues/${encodeURIComponent(address.issueId)}/coding-attempts/${encodeURIComponent(address.attemptId)}`;
}

export function getCodingAttemptSnapshot(
  address: CodingAttemptAddress,
): Promise<CodingAttemptSnapshotResponse> {
  return requestJson<CodingAttemptSnapshotResponse>(
    codingAttemptApiPath(address),
  );
}

export function getLegacyCodingAttemptSnapshot(
  attemptId: string,
): Promise<CodingAttemptSnapshotResponse> {
  return requestJson<CodingAttemptSnapshotResponse>(
    `/api/coding-attempts/${encodeURIComponent(attemptId)}`,
  );
}

export function getCodingAttemptDiff(
  address: CodingAttemptAddress,
): Promise<CodingAttemptDiffResponse> {
  return requestJson<CodingAttemptDiffResponse>(
    `${codingAttemptApiPath(address)}/diff`,
  );
}

export function deleteCodingAttempt(
  address: CodingAttemptAddress,
): Promise<void> {
  return requestJson<void>(
    codingAttemptApiPath(address),
    {
      method: "DELETE",
    },
  );
}

export function abortCodingAttempt(
  address: CodingAttemptAddress,
): Promise<CodingAttempt> {
  return requestJson<CodingAttempt>(
    `${codingAttemptApiPath(address)}/abort`,
    {
      method: "POST",
      body: JSON.stringify({}),
    },
  );
}

export function getCodingAttemptArtifact(
  address: CodingAttemptAddress,
  artifactId: string,
): Promise<ArtifactContentResponse> {
  return requestJson<ArtifactContentResponse>(
    `${codingAttemptApiPath(address)}/artifacts/${encodeURIComponent(artifactId)}`,
  );
}

export function confirmWorkItemExecutionPlan(
  address: CodingAttemptAddress,
): Promise<WorkItemExecutionPlan> {
  return requestJson<WorkItemExecutionPlan>(
    `${codingAttemptApiPath(address)}/execution-plan/confirm`,
    {
      method: "POST",
      body: JSON.stringify({}),
    },
  );
}

export function requestWorkItemExecutionPlanChange(
  address: CodingAttemptAddress,
  payload: { note: string },
): Promise<WorkItemExecutionPlan> {
  return requestJson<WorkItemExecutionPlan>(
    `${codingAttemptApiPath(address)}/execution-plan/change-request`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}
