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
  PlanCodingProgressDto,
  PlanCodingFinalConfirmResultDto,
  PlanCodingStartRequest,
  PlanCodingStartResultDto,
  PlanCodingWorkspaceDeletionResultDto,
  WorkspaceHumanAction,
  WorkspaceHumanActionStatus,
  WorkspaceSession,
  WorkItemExecutionPlan,
  AutomationEnrollmentLeaseTakeoverResult,
  AutomationEnrollmentRetryInitializationResult,
  EnrollmentBindingIdentity,
  VerificationCommandEvidence,
  VerificationTriageConclusion,
  VerificationTriageRecord,
} from "./types";

/**
 * 批2 T1：任一 API 请求返回 401 时派发（AuthGate 监听后回到认证态）。
 * 副作用请求绝不自动重发。
 */
export const AUTH_REQUIRED_EVENT = "aria:auth-required";

/** 401 时广播认证失效事件（非浏览器环境忽略）。 */
export function notifyAuthRequired(): void {
  if (typeof window !== "undefined") {
    window.dispatchEvent(new CustomEvent(AUTH_REQUIRED_EVENT));
  }
}
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
    // 批2 T1：aria_session cookie（HttpOnly）随同源请求自动携带。
    credentials: "same-origin",
    headers: {
      "content-type": "application/json",
      ...(init?.headers ?? {}),
    },
  });
  if (!response.ok) {
    // 批2 T1：401 回认证态（AuthGate 监听事件），绝不自动重发副作用请求。
    if (response.status === 401) {
      notifyAuthRequired();
    }
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
      credentials: "same-origin",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(action),
    },
  );
  if (response.status === 401) {
    notifyAuthRequired();
  }
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
      credentials: "same-origin",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        command_id: `cmd-sc-retry-${failedNodeId}`,
        expected_phase: "failed",
      }),
    },
  );
  if (response.status === 401) {
    notifyAuthRequired();
  }
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

export type DeleteLifecycleEntityOptions = { confirmed?: boolean };

// 级联删除 D2.3（plan §0.2）：确认 query 只在弹窗同意后显式携带；默认 URL
// 无 query，服务端三分类才是权威（query 布尔不是安全授权）。
function deleteLifecycleEntityQueryString(
  options?: DeleteLifecycleEntityOptions,
): string {
  if (options?.confirmed !== true) {
    return "";
  }
  const params = new URLSearchParams();
  params.set("confirmed", "true");
  return `?${params.toString()}`;
}

export function deleteStorySpec(
  projectId: string,
  issueId: string,
  storySpecId: string,
  options?: DeleteLifecycleEntityOptions,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/story-specs/${encodeURIComponent(storySpecId)}${deleteLifecycleEntityQueryString(options)}`,
    {
      method: "DELETE",
    },
  );
}

export function deleteDesignSpec(
  projectId: string,
  issueId: string,
  designSpecId: string,
  options?: DeleteLifecycleEntityOptions,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/design-specs/${encodeURIComponent(designSpecId)}${deleteLifecycleEntityQueryString(options)}`,
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
  options?: DeleteLifecycleEntityOptions,
): Promise<{ status: string }> {
  return requestJson<{ status: string }>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-item-plans/${encodeURIComponent(planId)}${deleteLifecycleEntityQueryString(options)}`,
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

// 批 2 P5（REQ-WIGA-10）：plan 阶段失败继续的只读 control 投影（P4 落地
// 的 GET automation-enrollment/control）。DTO 类型留在 client.ts——wire
// 类型文件（api/types/lifecycle.ts）不在本包独占清单内；expected_binding
// 复用 EnrollmentBindingIdentity（与 Rust EnrollmentBindingIdentity 同形）。
export type AutomationControlStatus = {
  managed: boolean;
  binding?: EnrollmentBindingIdentity | null;
  plan_id?: string | null;
  session_id?: string | null;
  terminated_at?: string | null;
  stop_key?: string | null;
  stage: string;
  allowed_actions: string[];
  context_required: boolean;
};

export type AutomationContinuePlanRequest = {
  command_id: string;
  expected_binding: EnrollmentBindingIdentity;
  expected_stop_key: string;
  /** 对话窗口补充说明：provider 失败继续可空；评审超限继续必须非空。 */
  context?: string;
};

export type AutomationContinuePlanStatus = {
  command_id: string;
  state: "accepted" | "replayed" | "needs_human" | string;
  session_id: string;
  reason_code?: string | null;
};

/** GET 只读管理投影：真实 stop_key/stage/allowed_actions/context_required。 */
export function getAutomationEnrollmentControl(
  projectId: string,
  issueId: string,
): Promise<AutomationControlStatus> {
  return requestJson<AutomationControlStatus>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/automation-enrollment/control`,
  );
}

/**
 * 批 2 P5（REQ-WIGA-10）：POST continue-plan。200 回执
 * （accepted/replayed/needs_human）原样返回；4xx（含
 * automation_continue_context_required 空说明、stop_key 漂移、terminated
 * 拒绝等）抛 ApiRequestError——code/message 上抛，由调用方呈现。
 */
export async function postAutomationContinuePlan(
  projectId: string,
  issueId: string,
  payload: AutomationContinuePlanRequest,
): Promise<AutomationContinuePlanStatus> {
  const response = await fetch(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/automation-enrollment/continue-plan`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      credentials: "same-origin",
      body: JSON.stringify(payload),
    },
  );
  if (response.status === 401) {
    notifyAuthRequired();
  }
  const body: unknown = await response.json().catch(() => null);
  if (
    response.ok &&
    isRecord(body) &&
    typeof body.command_id === "string" &&
    typeof body.state === "string" &&
    typeof body.session_id === "string"
  ) {
    return body as AutomationContinuePlanStatus;
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
    code: "automation_continue_unexpected_response",
    message: "继续自动化端点返回了无法识别的回执",
    details: {},
  });
}

// ── 批 2 T4（REQ-WIGA-09）：统一「终止自动化」REST 与 428 身份解析 ──

/** T3 wire 契约：REST 428 details 与 WS CodingProtocolError.message（JSON
 * 字符串）同形——confirm_required＋plan/enrollment/binding 身份。 */
export const AUTOMATION_PLAN_TERMINATION_REQUIRED = "automation_plan_termination_required";

export type AutomationTerminationIdentityDetails = {
  confirm_required: boolean;
  plan_id: string | null;
  session_id: string | null;
  enrollment_id: string;
  policy_revision: number;
  binding_version: number | null;
  terminated_at: string | null;
};

/**
 * 解析统一终止确认身份：接受 REST `ApiRequestError.details` 对象与 WS
 * `CodingProtocolError.message` JSON 字符串（两者同形）。缺 enrollment_id/
 * policy_revision 或载荷损坏返回 null（调用方保持未知，不猜身份）。
 */
export function parseAutomationTerminationIdentity(
  raw: unknown,
): AutomationTerminationIdentityDetails | null {
  let source: unknown = raw;
  if (typeof raw === "string") {
    try {
      source = JSON.parse(raw);
    } catch {
      return null;
    }
  }
  if (!isRecord(source)) {
    return null;
  }
  if (
    typeof source.enrollment_id !== "string" ||
    typeof source.policy_revision !== "number"
  ) {
    return null;
  }
  return {
    confirm_required: source.confirm_required === true,
    plan_id: typeof source.plan_id === "string" ? source.plan_id : null,
    session_id: typeof source.session_id === "string" ? source.session_id : null,
    enrollment_id: source.enrollment_id,
    policy_revision: source.policy_revision,
    binding_version: typeof source.binding_version === "number" ? source.binding_version : null,
    terminated_at: typeof source.terminated_at === "string" ? source.terminated_at : null,
  };
}

export type AutomationEnrollmentTerminateRequest = {
  command_id: string;
  expected_enrollment_id: string;
  expected_policy_revision: number;
  /** 提示性绑定互证（可选；与 enrollment 当前代全等，不符 409 零副作用）。 */
  expected_binding?: EnrollmentBindingIdentity;
  /** true＝统一二次确认已接受；缺省零副作用（服务端 428 要求确认）。 */
  confirmed: boolean;
  reason?: string;
};

export type AutomationEnrollmentTerminateResponse = {
  command_id: string;
  state: string;
  enrollment_id: string;
  plan_id?: string | null;
  terminated_at: string;
  /** terminated | cleanup_required（cleanup_required 仍不可继续，同命令重放）。 */
  outcome: string;
  results: Array<{
    attempt_id: string;
    status: string;
    reason_code?: string | null;
    message?: string | null;
  }>;
};

/**
 * 批 2 T4：POST 统一终止 REST（`confirmed:true` 才有终止副作用）。未确认
 * 探测／服务端要求确认时抛 `ApiRequestError`（code
 * `automation_plan_termination_required`，details 携带身份）——调用方只打开
 * 同一个二次确认面，不渲染为错误弹窗。
 */
export function postAutomationEnrollmentTerminate(
  projectId: string,
  issueId: string,
  payload: AutomationEnrollmentTerminateRequest,
): Promise<AutomationEnrollmentTerminateResponse> {
  return requestJson<AutomationEnrollmentTerminateResponse>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/automation-enrollment/terminate`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

/**
 * 批 2 P5：workspace session → (project_id, issue_id) 只读目录解析——
 * control/continue 均以 issue 为作用域，会话页只有 sessionId；按 observers
 * 同源目录（projects → issues → lifecycle workspace_sessions）扫描定位，
 * 找不到返回 null（调用方保持静默，不出恢复面）。
 */
export async function findWorkspaceSessionIssueAddress(
  sessionId: string,
): Promise<{ projectId: string; issueId: string } | null> {
  try {
    const { projects } = await listProjects();
    for (const project of projects) {
      const listed = await listProductIssues(project.project_id);
      for (const issue of listed.issues) {
        const lifecycle = await getIssueLifecycle(issue.issue_id, project.project_id);
        if (
          lifecycle.workspace_sessions.some(
            (session) => session.workspace_session_id === sessionId,
          )
        ) {
          return { projectId: project.project_id, issueId: issue.issue_id };
        }
      }
    }
  } catch {
    return null;
  }
  return null;
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

// N6（REQ-CS-01）：多仓 confirmed plan 的一键显式启动——command_id 为稳定幂等键，
// expected_plan_revision_id 与 GET 聚合返回的 bound revision 互证（漂移即拒）。
export function startPlanCoding(
  projectId: string,
  issueId: string,
  planId: string,
  request: PlanCodingStartRequest,
): Promise<PlanCodingStartResultDto> {
  return requestJson<PlanCodingStartResultDto>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-item-plans/${encodeURIComponent(planId)}/coding-start`,
    {
      method: "POST",
      body: JSON.stringify(request),
    },
  );
}

// N6（REQ-CS-06）：plan 聚合进度 GET（与 POST 同地址）——未建 run 时只读投影，
// bound_plan_revision_id 供一键启动互证。
export function getPlanCodingProgress(
  projectId: string,
  issueId: string,
  planId: string,
): Promise<PlanCodingProgressDto> {
  return requestJson<PlanCodingProgressDto>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-item-plans/${encodeURIComponent(planId)}/coding-start`,
  );
}

// P1-b（REQ-MTG-11 / 契约 A2）：plan 级聚合最终确认——无 body POST，按 target
// 顺序逐仓调用既有确认（首失败即停），回执逐仓结果（失败仓 error_code/message
// 可见）；不做回执/防重放/跨仓事务（owner 已裁定剔除，已知风险 H-1~H-6）。
export function confirmPlanCoding(
  projectId: string,
  issueId: string,
  planId: string,
): Promise<PlanCodingFinalConfirmResultDto> {
  return requestJson<PlanCodingFinalConfirmResultDto>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-item-plans/${encodeURIComponent(planId)}/coding-final-confirm`,
    { method: "POST" },
  );
}

// H5（设计 C 节冻结契约）：删除 plan 下全部 Coding 工作区——plan-scoped，不收
// 前端 attempt ID 列表（服务端是范围权威）。query：稳定 command_id（同动作
// 重试复用）、expected_generation（GET workspace_lifecycle.generation 互证，
// 未知时省略）、confirmed=true（仅确认弹窗同意后显式携带；query 不是安全
// 授权，服务端分类才是权威）。typed 409（required/busy/clearing）抛
// ApiRequestError，由调用方分类消费，不自动重试。
export function deletePlanCodingWorkspaces(
  projectId: string,
  issueId: string,
  planId: string,
  query: {
    command_id: string;
    expected_generation?: number;
    confirmed?: boolean;
  },
): Promise<PlanCodingWorkspaceDeletionResultDto> {
  const params = new URLSearchParams();
  params.set("command_id", query.command_id);
  if (query.expected_generation !== undefined) {
    params.set("expected_generation", String(query.expected_generation));
  }
  if (query.confirmed === true) {
    params.set("confirmed", "true");
  }
  return requestJson<PlanCodingWorkspaceDeletionResultDto>(
    `/api/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/work-item-plans/${encodeURIComponent(planId)}/coding-workspaces?${params.toString()}`,
    { method: "DELETE" },
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

// C2 Task 3/12（REQ-CRO-03/06）：终态 coding attempt 显式 restart——
// Accepted／Replayed→200，NeedsHuman（停等）→202，Rejected（版本／身份
// 不符"请刷新"）→409；同 command 同 payload 幂等重放。
export type CodingAttemptOperationState =
  | "accepted"
  | "replayed"
  | "needs_human"
  | "rejected";

export type RestartCodingAttemptResponse = {
  command_id: string;
  state: CodingAttemptOperationState;
  attempt_id: string;
  reason?: string | null;
};

export function restartCodingAttempt(
  address: CodingAttemptAddress,
  payload: {
    command_id: string;
    attempt_id: string;
    expected_attempt_version: number;
  },
): Promise<RestartCodingAttemptResponse> {
  return requestJson<RestartCodingAttemptResponse>(
    `${codingAttemptApiPath(address)}/restart`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

// C2 Task 12（REQ-CRO-06）：驾驶舱 gate response REST——与 coding WS
// `GateResponse` 同一应用服务（无 coding socket 也能作答）；携带稳定
// command_id＋expected 版本（旧版本 fail-closed"请刷新"）。
export type CodingGateResponseResult = {
  command_id: string;
  state: CodingAttemptOperationState;
  attempt_id: string;
  gate_id: string;
  action_id: string;
  reason?: string | null;
};

export function postCodingGateResponse(
  address: CodingAttemptAddress,
  payload: {
    command_id: string;
    gate_id: string;
    action_id: string;
    extra_context?: string | null;
    expected_version: number;
  },
): Promise<CodingGateResponseResult> {
  return requestJson<CodingGateResponseResult>(
    `${codingAttemptApiPath(address)}/gate-responses`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

// C2 oracle C-2a/b/c：验证处理／并列证据／重跑／受限政策读取 REST——
// 与后端 verification_surface.rs 同一契约（复用既有引擎服务与命令账本）。
export type VerificationTriageRecordsResponse = {
  attempt_id: string;
  attempt_version: number;
  records: VerificationTriageRecord[];
};

export function listVerificationTriageRecords(
  address: CodingAttemptAddress,
): Promise<VerificationTriageRecordsResponse> {
  return requestJson<VerificationTriageRecordsResponse>(
    `${codingAttemptApiPath(address)}/verification-triage`,
  );
}

export function enterVerificationTriage(
  address: CodingAttemptAddress,
  payload: {
    finding_id: string;
    check_id: string;
    original_command?: string | null;
    alternative_command?: string | null;
    cwd?: string | null;
    outcome?: string | null;
    test_execution_count?: number | null;
    environment?: string | null;
  },
): Promise<VerificationTriageRecord> {
  return requestJson<VerificationTriageRecord>(
    `${codingAttemptApiPath(address)}/verification-triage`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export function decideVerificationTriage(
  address: CodingAttemptAddress,
  triageId: string,
  payload: {
    conclusion: VerificationTriageConclusion;
    decided_by: string;
    reason: string;
    exemption_scope?: string[];
  },
): Promise<VerificationTriageRecord> {
  return requestJson<VerificationTriageRecord>(
    `${codingAttemptApiPath(address)}/verification-triage/${encodeURIComponent(triageId)}/decision`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export type VerificationCommandEvidenceListResponse = {
  attempt_id: string;
  rework_count: number;
  evidence: VerificationCommandEvidence[];
};

export function getVerificationCommandEvidence(
  address: CodingAttemptAddress,
): Promise<VerificationCommandEvidenceListResponse> {
  return requestJson<VerificationCommandEvidenceListResponse>(
    `${codingAttemptApiPath(address)}/verification-command-evidence`,
  );
}

export type RerunPlannedCommandResponse = {
  command_id: string;
  attempt_id: string;
  instruction_id: string;
  replayed: boolean;
};

export function rerunPlannedCommand(
  address: CodingAttemptAddress,
  payload: {
    command_id: string;
    gate_id: string;
    check_id: string;
    expected_version: number;
  },
): Promise<RerunPlannedCommandResponse> {
  return requestJson<RerunPlannedCommandResponse>(
    `${codingAttemptApiPath(address)}/rerun-planned-command`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
}

export type CodingPolicyTextResponse = {
  attempt_id: string;
  attempt_version: number;
  policy: {
    policy_id: string;
    policy_revision: number;
    policy_digest: string;
    text: string;
  };
};

export function readCodingPolicyText(
  address: CodingAttemptAddress,
): Promise<CodingPolicyTextResponse> {
  return requestJson<CodingPolicyTextResponse>(
    `${codingAttemptApiPath(address)}/policy-text`,
  );
}

export function reauthorizeCodingPolicy(
  address: CodingAttemptAddress,
  payload: {
    command_id: string;
    attempt_id: string;
    role: "coder" | "reviewer";
    policy_digest: string;
    expected_version: number;
  },
): Promise<CodingGateResponseResult> {
  return requestJson<CodingGateResponseResult>(
    `${codingAttemptApiPath(address)}/policy-reauthorization`,
    {
      method: "POST",
      body: JSON.stringify(payload),
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
