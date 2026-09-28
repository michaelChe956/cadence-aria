import type {
  ApiError,
  ArtifactContentResponse,
  ChoiceReplyStatus,
  ChoiceResponseRequest,
  AutomationEnrollmentPutRequest,
  AutomationEnrollmentRebindRequest,
  AutomationEnrollmentRebindResult,
  AutomationTarget,
  AutomationTargetQuery,
  CodingAttempt,
  IssueAutomationEnrollment,
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
  RetryFailedScRunStatus,
  TakeoverResponse,
  WorkspaceHumanAction,
  WorkspaceHumanActionStatus,
  WorkspaceSession,
  WorkItemExecutionPlan,
  AutomationEnrollmentLeaseTakeoverResult,
  AutomationEnrollmentRetryInitializationResult,
  EnrollmentBindingIdentity,
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

/**
 * P0 1.3（REQ-WIGA-05）Task 10：无 driver 的人工门命令 REST 通道。
 *
 * `accepted`（200）与 `busy`（409，门/轮次占用）都是协议状态回执 → 原样返回；
 * 其余 4xx（`human_action_gate_mismatch` 门身份不匹配、引擎语义拒绝 422）抛
 * `ApiRequestError`——错误细节经 code/message 上抛，调用方负责审计/错误面。
 */
export async function postWorkspaceHumanAction(
  sessionId: string,
  action: WorkspaceHumanAction,
): Promise<WorkspaceHumanActionStatus> {
  const response = await fetch(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/human-actions`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(action),
    },
  );
  const body: unknown = await response.json().catch(() => null);
  if (isHumanActionStatus(body) && (response.ok || response.status === 409)) {
    return body;
  }
  if (!response.ok) {
    const error: unknown = body;
    throw new ApiRequestError({
      code:
        isRecord(error) && typeof error.code === "string"
          ? error.code
          : "web_client_error",
      message:
        isRecord(error) && typeof error.message === "string"
          ? error.message
          : response.statusText,
      details:
        isRecord(error) && isRecord(error.details)
          ? (error.details as ApiError["details"])
          : {},
    });
  }
  throw new ApiRequestError({
    code: "human_action_unexpected_response",
    message: "人工命令端点返回了无法识别的回执",
    details: {},
  });
}

function isHumanActionStatus(value: unknown): value is WorkspaceHumanActionStatus {
  return (
    isRecord(value) &&
    typeof value.command_id === "string" &&
    typeof value.gate_id === "string" &&
    (value.state === "accepted" || value.state === "busy" || value.state === "rejected")
  );
}

/**
 * P2 GAP-E/G（Task 0.1）：失败 SingleCandidate 评审运行的人工显式重驱。
 *
 * command_id 由失败节点稳定派生（`cmd-sc-retry-{failed_node_id}`）——同节点
 * 重复点击命中服务端同键 replayed 幂等，不产生第二次 provider 启动。
 * 200 回执（accepted/replayed/needs_human）原样返回；4xx 抛 `ApiRequestError`。
 */
export async function postRetryFailedScRun(
  sessionId: string,
  failedNodeId: string,
): Promise<RetryFailedScRunStatus> {
  const response = await fetch(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/failed-sc-runs/${encodeURIComponent(failedNodeId)}/retry`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        command_id: `cmd-sc-retry-${failedNodeId}`,
        expected_phase: "failed",
      }),
    },
  );
  const body: unknown = await response.json().catch(() => null);
  if (
    response.ok &&
    isRecord(body) &&
    typeof body.command_id === "string" &&
    typeof body.failed_node_id === "string" &&
    (body.state === "accepted" || body.state === "replayed" || body.state === "needs_human")
  ) {
    return body as RetryFailedScRunStatus;
  }
  if (!response.ok) {
    const error: unknown = body;
    throw new ApiRequestError({
      code:
        isRecord(error) && typeof error.code === "string"
          ? error.code
          : "web_client_error",
      message:
        isRecord(error) && typeof error.message === "string"
          ? error.message
          : response.statusText,
      details:
        isRecord(error) && isRecord(error.details)
          ? (error.details as ApiError["details"])
          : {},
    });
  }
  throw new ApiRequestError({
    code: "sc_retry_unexpected_response",
    message: "失败重驱端点返回了无法识别的回执",
    details: {},
  });
}

/**
 * P0 1.3（REQ-WIGA-05）：workspace choice 应答 REST 通道。200=Delivered、
 * 202=Submitting/Resolving（未承诺送达）；404/409/410 抛 ApiRequestError。
 */
export function postWorkspaceChoiceResponse(
  sessionId: string,
  choiceId: string,
  request: ChoiceResponseRequest,
): Promise<ChoiceReplyStatus> {
  return requestJson<ChoiceReplyStatus>(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/choices/${encodeURIComponent(choiceId)}/response`,
    { method: "POST", body: JSON.stringify(request) },
  );
}

/** 同 command 复查（202 后保卡轮询；同一 command_id 查询不启动新 command）。 */
export function getWorkspaceChoiceResponseStatus(
  sessionId: string,
  choiceId: string,
  commandId: string,
): Promise<ChoiceReplyStatus> {
  return requestJson<ChoiceReplyStatus>(
    `/api/workspace-sessions/${encodeURIComponent(sessionId)}/choices/${encodeURIComponent(choiceId)}/responses/${encodeURIComponent(commandId)}`,
  );
}

/** P0 1.3：coding choice 应答 REST 通道（同一 claim 语义，per attempt 作用域）。 */
export function postCodingChoiceResponse(
  address: CodingAttemptAddress,
  choiceId: string,
  request: ChoiceResponseRequest,
): Promise<ChoiceReplyStatus> {
  return requestJson<ChoiceReplyStatus>(
    `${codingAttemptApiPath(address)}/choices/${encodeURIComponent(choiceId)}/response`,
    { method: "POST", body: JSON.stringify(request) },
  );
}

export function getCodingChoiceResponseStatus(
  address: CodingAttemptAddress,
  choiceId: string,
  commandId: string,
): Promise<ChoiceReplyStatus> {
  return requestJson<ChoiceReplyStatus>(
    `${codingAttemptApiPath(address)}/choices/${encodeURIComponent(choiceId)}/responses/${encodeURIComponent(commandId)}`,
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

// P3（REQ-WIGA-07）：不传 options 完全保持旧两参 URL；只有观察者显式携带
// recent_since/recent_limit 才追加有界近期完成目录查询键。
export function getIssueLifecycle(
  issueId: string,
  projectId: string,
  options?: { recentSince: string; recentLimit?: number },
): Promise<IssueLifecycleResponse> {
  let query = `project_id=${encodeURIComponent(projectId)}`;
  if (options) {
    const params = new URLSearchParams();
    params.set("recent_since", options.recentSince);
    if (options.recentLimit !== undefined) {
      params.set("recent_limit", String(options.recentLimit));
    }
    query += `&${params.toString()}`;
  }
  return requestJson<IssueLifecycleResponse>(
    `/api/issues/${encodeURIComponent(issueId)}/lifecycle?${query}`,
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

// P1 WIGA Task 2：Design 后自动化 enrollment 面（GET/PUT 复用 P0 REST，
// target 是 Task 1 只读投影）。前端不从 repo_id 推断 UUID、不猜默认 provider。
export function getAutomationTarget(
  projectId: string,
  issueId: string,
  query: AutomationTargetQuery,
): Promise<AutomationTarget> {
  const params = new URLSearchParams();
  if (query.author_provider) {
    params.set("author_provider", query.author_provider);
  }
  if (query.reviewer_provider) {
    params.set("reviewer_provider", query.reviewer_provider);
  }
  if (typeof query.review_rounds === "number") {
    params.set("review_rounds", String(query.review_rounds));
  }
  for (const key of [
    "superpowers_enabled",
    "openspec_enabled",
    "include_integration_tests",
    "include_e2e_tests",
    "force_frontend_backend_split",
    "require_execution_plan_confirm",
  ] as const) {
    const value = query[key];
    if (typeof value === "boolean") {
      params.set(key, String(value));
    }
  }
  const suffix = params.size > 0 ? `?${params.toString()}` : "";
  return requestJson<AutomationTarget>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/automation-target${suffix}`,
  );
}

export function getAutomationEnrollment(
  projectId: string,
  issueId: string,
): Promise<IssueAutomationEnrollment | null> {
  return requestJson<IssueAutomationEnrollment | null>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/automation-enrollment`,
  );
}

export function putAutomationEnrollment(
  projectId: string,
  issueId: string,
  payload: AutomationEnrollmentPutRequest,
): Promise<IssueAutomationEnrollment> {
  return requestJson<IssueAutomationEnrollment>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/automation-enrollment`,
    {
      method: "PUT",
      body: JSON.stringify(payload),
    },
  );
}

// C1 Task 6（REQ-WIGA-03）：死亡租约确认接管（durable CAS；expected
// binding/lease/attempt 由调用方从 durable 投影携带，前端不猜）。
export function confirmLeaseTakeover(
  projectId: string,
  issueId: string,
  payload: {
    command_id: string;
    expected_binding: EnrollmentBindingIdentity;
    expected_lease_id: string;
    expected_attempt_id: string;
  },
): Promise<AutomationEnrollmentLeaseTakeoverResult> {
  return requestJson<AutomationEnrollmentLeaseTakeoverResult>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/automation-enrollment/lease/takeover`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

// C1 Task 7（REQ-ADV-C1-RETRY）：Failed advance 显式 retry-initialization。
// 响应固定为 RetryInitializationResult；NeedsHuman 由 state 承载。
export function retryAdvanceInitialization(
  projectId: string,
  issueId: string,
  planId: string,
  payload: {
    command_id: string;
    expected_binding: EnrollmentBindingIdentity;
    expected_attempt_id: string;
    expected_checkpoint: string;
    confirm_unknown_side_effect: boolean;
  },
): Promise<AutomationEnrollmentRetryInitializationResult> {
  return requestJson<AutomationEnrollmentRetryInitializationResult>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-item-plans/${encodeURIComponent(planId)}/advance/retry-initialization`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function rebindAutomationEnrollment(
  projectId: string,
  issueId: string,
  payload: AutomationEnrollmentRebindRequest,
): Promise<AutomationEnrollmentRebindResult> {
  return requestJson<AutomationEnrollmentRebindResult>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/automation-enrollment/rebind`,
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
