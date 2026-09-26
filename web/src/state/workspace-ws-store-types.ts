import type {
  AutomationOwnership,
  ChoiceOption,
  ChoiceQuestion,
  UsageReportPayload,
} from "../api/types/workspace";
import type {
  ProviderPermissionMode,
  WorkspaceProviderName,
  HumanPresentationRevision,
  NodeDetail,
  PlanAmendmentManifest,
  PlanProjectionBundle,
  PlanRepairSessionSnapshot,
  ProjectionValidationReport,
  StructuredOutputDiagnostic,
  WorkItemBatchStatePayload,
  WorkItemDraftCandidatePayload,
  WorkItemPlanArtifactPayload,
  WorkItemPlanArtifactVersion,
  WorkItemPlanCandidateDto,
  WorkItemPlanCompileReportPayload,
  WorkItemPlanContextBlockerPayload,
  WorkItemPlanOutlineCandidatePayload,
  WorkItemProjectionBundle,
  WorkItemRevisionHistoryDto,
} from "../api/types";
import type {
  ChatEntry,
  ChoiceAnswerPayload,
  ChatEntryResolution,
  ChatEntryRole,
} from "./chat-entries";
import type { WorkspaceContentCache } from "./workspace-content-cache";

export type WorkspaceArtifact =
  | string
  | null
  | { markdown: string; diff?: string | null }
  | { candidate: WorkItemPlanCandidateDto }
  | { outline_candidate: WorkItemPlanOutlineCandidatePayload }
  | { context_blocker: WorkItemPlanContextBlockerPayload }
  | { draft_candidate: WorkItemDraftCandidatePayload }
  | { batch_state: WorkItemBatchStatePayload }
  | { compile_report: WorkItemPlanCompileReportPayload }
  | { plan_projection: PlanProjectionBundle }
  | { work_item_projection: WorkItemProjectionBundle }
  | { work_item_revision_history: WorkItemRevisionHistoryDto }
  | { projection_validation: ProjectionValidationReport }
  | { plan_amendment_manifest: PlanAmendmentManifest };

export type WorkItemPlanProjectionArtifacts = {
  planProjection: PlanProjectionBundle | null;
  workItemProjections: WorkItemProjectionBundle[];
  history: WorkItemRevisionHistoryDto | null;
  validation: ProjectionValidationReport | null;
  missingWorkItemProjectionRefs: string[];
};

export type HumanPresentationSaveState = {
  saving: boolean;
  error: string | null;
};

export type WsConnectionStatus = "disconnected" | "connecting" | "connected" | "error";
export type ProviderStatus =
  | "starting"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "aborted";
export type ExecutionEventKind = "provider" | "turn" | "command" | "output" | "artifact" | "usage";
export type ExecutionEventStatus =
  | "started"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "aborted";
export type TimelineNodeType =
  | "prepare_context"
  | "context_note"
  | "start_generation"
  | "author_confirm"
  | "author_run"
  | "reviewer_run"
  | "review_decision"
  | "revision"
  | "human_confirm"
  | "work_item_plan_outline_run"
  | "work_item_plan_outline_confirm"
  | "work_item_plan_outline_review"
  | "work_item_plan_context_blocker"
  | "work_item_generation_mode"
  | "work_item_draft_run"
  | "work_item_draft_confirm"
  | "work_item_draft_review"
  | "work_item_batch_run"
  | "work_item_batch_confirm"
  | "work_item_batch_review"
  | "work_item_plan_compile"
  | "work_item_plan_compile_recovery"
  | "aborted_by_disconnect"
  | "protocol_error"
  | "completed"
  | (string & {});
export type TimelineNodeStatus = "active" | "paused" | "completed" | "failed" | "skipped";
export type ReviewVerdictType = "pass" | "revise" | "needs_human";

export interface PermissionRequest {
  id: string;
  tool_name: string;
  description: string;
  risk_level: "low" | "medium" | "high";
}

export interface ExecutionEvent {
  event_id: string;
  node_id?: string | null;
  agent?: string | null;
  kind: ExecutionEventKind;
  status: ExecutionEventStatus;
  title: string;
  detail?: string | null;
  command?: string | null;
  cwd?: string | null;
  output?: string | null;
  exit_code?: number | null;
}

export interface WsMessage {
  id: string;
  role: string;
  content: string;
  checkpoint_id?: string | null;
  created_at: string;
}

export interface WsCheckpoint {
  id: string;
  message_index: number;
  stage: string;
  created_at: string;
}

export interface WsProviderConfig {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
  permission_modes?: {
    author: ProviderPermissionMode;
    reviewer: ProviderPermissionMode;
  };
}

export interface ProviderConfigSnapshot {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
  review_rounds: number;
  permission_modes?: {
    author: ProviderPermissionMode;
    reviewer: ProviderPermissionMode;
  };
}

export interface TimelineNodeRetryError {
  code: string;
  message: string;
}

export interface TimelineNodeRetry {
  retry_of_node_id: string;
  retry_attempt: number;
  retry_reason: string;
  retry_error: TimelineNodeRetryError;
}

export interface TimelineNode {
  node_id: string;
  node_type: TimelineNodeType;
  agent?: WorkspaceProviderName | null;
  stage: string;
  round?: number | null;
  status: TimelineNodeStatus;
  title: string;
  summary?: string | null;
  started_at: string;
  // 最近一次引擎事件时间（客户端维护，见 store 的 addTimelineNode / updateTimelineNode /
  // appendStreamChunk 刷新点）；REQ-UI37-18 的「长时间无事件」静默判据取本字段，
  // 老数据或重建快照缺省时回退 `started_at`。
  last_event_at?: string | null;
  completed_at?: string | null;
  duration_ms?: number | null;
  artifact_ref?: string | null;
  provider_config_snapshot: ProviderConfigSnapshot;
  retry?: TimelineNodeRetry | null;
}

export interface ReviewVerdict {
  verdict: ReviewVerdictType;
  comments: string;
  summary: string;
  findings?: ReviewFinding[];
  review_gate?: ReviewGate;
  structured_output_diagnostic?: StructuredOutputDiagnostic | null;
}

export type ReviewFindingSeverity = "blocking" | "must_fix" | "suggestion";

export type ReviewGate =
  | "requires_revision"
  | "user_confirm_allowed"
  | "user_triage_required";

export interface ReviewFinding {
  severity: ReviewFindingSeverity;
  message: string;
  evidence: string;
  required_action: string;
}

export interface ArtifactVersionSummary {
  version: number;
  markdown?: string;
  generated_by: WorkspaceProviderName;
  reviewed_by?: WorkspaceProviderName | null;
  review_verdict?: ReviewVerdictType | null;
  confirmed_by?: string | null;
  is_current?: boolean;
  created_at: string;
  source_node_id: string;
}

export interface ArtifactVersion extends ArtifactVersionSummary {
  markdown?: string;
  plan_projection?: PlanProjectionBundle;
  work_item_projection?: WorkItemProjectionBundle;
  work_item_revision_history?: WorkItemRevisionHistoryDto;
  projection_validation?: ProjectionValidationReport;
}

/**
 * F-47 REQ-NDR-01/03：`hydration_pending` 是前端本地水合态标记（非 WS/REST 载荷
 * 字段）。`emptyNodeDetail` 造的占位壳带 `hydration_pending: true`，表示该节点的
 * durable detail 尚未经 REST 水合或快照内联填充；一旦有本地内容写入（流式分片 /
 * 执行事件 / 消息 / 结论，见 `ensureNodeDetail`）或 REST/内联 detail 落库，标记即消失。
 * 用途：①快照 merge 只保留「已物化」条目，纯占位壳按当前节点重建（避免沿用陈旧
 * status/ended_at）；②token 位据此区分「尚未读取」与「确认无 usage」。
 */
export type TimelineNodeDetail = NodeDetail & { hydration_pending?: boolean };

export interface NodeDetailSummary {
  node_id: string;
  node_type: string;
  status: string;
  agent_role?: string | null;
  provider_name?: string | null;
  prompt_size: number;
  prompt_preview?: string | null;
  stream_size: number;
  stream_preview?: string | null;
  execution_event_count: number;
  has_large_outputs: boolean;
  artifact_ref?: string | null;
  started_at: string;
  ended_at?: string | null;
}

export interface ReviewDecisionRequired {
  node_id: string;
  round: number;
  options: string[];
}

export interface ProtocolErrorState {
  code: string;
  message: string;
}

export interface RecoverableInterruptedRun {
  failed_node_id: string;
  operation: "review" | "work_item_draft_generation";
  label: string;
}

export type WorkspaceSessionStatus =
  | "open"
  | "running"
  | "waiting_for_human"
  | "confirmed"
  | "change_requested"
  | "blocked_provider_unavailable"
  | "terminated"
  | "stopped_needs_human"
  | "failed";
export type WorkItemPlanFlowKind = "legacy" | "single_candidate";
export type WorkItemPlanRunPolicy = "interactive" | "auto_if_valid";
export type WorkItemPlanSingleCandidatePhase =
  | "prepare"
  | "generate"
  | "evaluate"
  | "approval"
  | "completed"
  | "failed";
export interface WorkItemPlanRunHistory {
  seen_fingerprints: string[];
  repairs_used: number;
  manual_repairs_used: number;
  transitions_used: number;
  initial_review_count: number;
  verification_review_count: number;
  review_cycles?: Record<
    string,
    { repairs_used: number; initial_count: number; verification_count: number }
  >;
}

export interface WorkItemPlanHumanGateSnapshot {
  findings: Array<{
    class: "mechanical_error" | "repairable" | "human_required" | "advisory";
    fingerprint: string;
    category: string | null;
    severity: string;
    message: string;
    evidence: string | null;
    required_action: string | null;
    contract_field: string | null;
    /** C1（REQ-TOP-04 场景 4）：无稳定 ID 措辞域身份——true 时跨轮对比禁推断。 */
    identity_unstable?: boolean;
  }>;
  repeated_fingerprints: string[];
  attempts_used: number;
  manual_repairs_remaining: number;
  /** C2（REQ-HGC-01）：gate-local 已接受反馈轮次；旧会话缺席=预算历史不可用。 */
  accepted_feedback_turns?: number | null;
  trigger:
    | "native_human_required"
    | "repeated_fingerprint"
    | "verification_new_findings"
    | "repair_budget_exhausted";
  resumable: boolean;
  opened_at?: string;
}

export type HumanGateTurnStatus = "open" | "awaiting_confirm" | "failed" | "busy";
export type GateClosureDecision = "confirm" | "terminate";
export type AdvanceCommandStatus = "pending" | "completed" | "rejected";

export interface HumanGateTurnState {
  turn_id: string;
  command_id: string | null;
  remaining_budget: number;
  status: HumanGateTurnStatus;
  artifact_ref: string | null;
  failure_class: string | null;
  failure_message: string | null;
  opened_at: string;
  inlineError: InlineProtocolError | null;
  /**
   * F-49 A8：本 turn 开出时的门快照身份（`snapshotGateIdentity`）。门以新快照重建后
   * （修订成功经 Evaluate 重建、预算重置为默认值）残留 turn 的预算即过期——投影以
   * 本字段与当前快照身份的差异判定「快照已接管预算」。缺省（undefined）表示开出时
   * 无快照凭据，此时维持 turn 值（fail-closed）。
   */
  opened_snapshot_identity?: string | null;
}

export interface InlineProtocolError {
  code: string;
  message: string;
}

export interface HumanGateClosure {
  decision: GateClosureDecision;
  stage: string;
}


export interface AdvanceCommandState {
  command_id: string;
  status: AdvanceCommandStatus;
  code: string | null;
  reason: string | null;
  attempt_id: string | null;
  workspace_entry: string | null;
  inlineError: InlineProtocolError | null;
}


export interface ProtocolDiagnostic {
  code: string;
  message: string;
  at: string;
  type: string;
}
export interface ConnectionCloseDiagnostic {
  connectionId: string | null;
  closeCode: number;
  closeReason: string;
  wasClean: boolean;
  visibilityState: DocumentVisibilityState;
  lastPongOrServerMessageAt: string | null;
  lastPingAt: string | null;
  at: string;
}


export interface WorkItemPlanRepairReservation {
  token: string;
  owner_session_id: string;
  owner_run_id: string;
  provider_start_idempotency_key: string;
  state: "reserved" | "provider_started" | "committed" | "released";
  commit_id: string | null;
}

export interface WorkItemPlanPolicyDiagnostic {
  code: string;
  message: string;
  field: string | null;
}

export interface WorkItemPlanProviderStartLedgerEntry {
  provider_start_idempotency_key: string;
  started: boolean;
}

/**
 * F-59：session_state `pending_choice_requests` 投影条目——等待提示条的
 * 数据源。`created_at_ms` 是 provider pending 登记时刻（epoch ms，跨刷新
 * 不失真的等待锚点；TextFallback/旧载荷缺省 null，回退 first_seen_at_ms）；
 * `role` 标注发问方（author/reviewer）。
 * P0 1.3（REQ-WIGA-05）Task 11：同时是驾驶舱就地作答的数据源——完整
 * options/questions/source/allow_* 与服务端 `WsPendingChoiceRequest` 同形
 *（questions 严格校验：question id/选项数组畸形即置空，禁止喂 REST 提交面）。
 */
export interface PendingChoiceRequestProjection {
  id: string;
  prompt: string;
  role: string;
  created_at_ms: number | null;
  first_seen_at_ms: number | null;
  /** P0 1.3：应答须绑定的 run 化身；null=旧载荷/无活跃 run（不猜 run）。 */
  expected_run_id: string | null;
  options: ChoiceOption[];
  allow_multiple: boolean;
  allow_free_text: boolean;
  questions: ChoiceQuestion[];
  source: string;
}

export interface WorkspaceWsState {
  sessionId: string | null;
  workspaceType: string | null;
  stage: string;
  superpowersEnabled: boolean;
  openSpecEnabled: boolean;
  sessionStatus: WorkspaceSessionStatus | null;
  flowKind: WorkItemPlanFlowKind | null;
  runPolicy: WorkItemPlanRunPolicy | null;
  runHistory: WorkItemPlanRunHistory | null;
  reviewInvocationScope: unknown | null;
  humanGateSnapshot: WorkItemPlanHumanGateSnapshot | null;
  /**
   * C2（REQ-HGC-02 场景 3）：同一 logical gate 内上一轮 durable 快照的
   * findings（复评重建时由 setSessionState 保留）——跨轮 delta 的前轮事实；
   * 刷新/跨会话/关门后为 null（历史不全显式 unknown，不猜）。
   */
  previousGateFindings: WorkItemPlanHumanGateSnapshot["findings"] | null;
  repairReservation: WorkItemPlanRepairReservation | null;
  policyDiagnostics: WorkItemPlanPolicyDiagnostic[];
  providerStartLedger: WorkItemPlanProviderStartLedgerEntry[];
  singleCandidatePhase: WorkItemPlanSingleCandidatePhase | null;
  workItemPlanSourceRevisionRef: string | null;
  planCandidateIrRef: string | null;
  mechanicalReportRef: string | null;
  publicationProvenanceRef: string | null;
  visitedStages: string[];
  messages: WsMessage[];
  chatEntries: ChatEntry[];
  checkpoints: WsCheckpoint[];
  /** F-59：session_state pending_choice_requests 归一投影（等待提示条数据源）。 */
  pendingChoiceRequests: PendingChoiceRequestProjection[];
  artifact: string | null;
  workItemPlanCandidate: WorkItemPlanCandidateDto | null;
  workItemPlanArtifact: WorkItemPlanArtifactPayload | null;
  workItemPlanArtifactVersions: WorkItemPlanArtifactVersion[];
  workItemPlanProjectionArtifacts: WorkItemPlanProjectionArtifacts;
  humanPresentationRevisions: Record<string, HumanPresentationRevision>;
  humanPresentationSaveStates: Record<string, HumanPresentationSaveState>;
  providers: WsProviderConfig | null;
  connectionStatus: WsConnectionStatus;
  streamingContent: string;
  streamBuffers: Record<string, { chunks: string[]; visibleText: string; role: ChatEntryRole }>;
  activeStreamEntryId: string | null;
  pendingPermissions: PermissionRequest[];
  providerStatus: ProviderStatus;
  executionEvents: ExecutionEvent[];
  timelineNodes: TimelineNode[];
  activeNodeId: string | null;
  selectedNodeId: string | null;
  nodeDetails: Record<string, TimelineNodeDetail>;
  nodeSummaries: Record<string, NodeDetailSummary>;
  contentCache: WorkspaceContentCache;
  artifactContentCache: WorkspaceContentCache;
  artifactVersions: ArtifactVersionSummary[];
  pendingDecision: ReviewDecisionRequired | null;
  error: string | null;
  activeRunId: string | null;
  recoverableInterruptedRun: RecoverableInterruptedRun | null;
  protocolError: ProtocolErrorState | null;
  providerLocked: boolean;
  providerSnapshot: ProviderConfigSnapshot | null;
  providerLockedAt: string | null;
  acknowledgedAbortedNodes: string[];
  reviewerEnabled: boolean;
  reviewRounds: number;
  permissionModes: {
    author: ProviderPermissionMode;
    reviewer: ProviderPermissionMode;
  };
  pendingReviewDecision: { verdict: string; summary: string } | null;
  pendingReviewerSummary: { verdict: string; points: string[] } | null;
  humanGateTurn: HumanGateTurnState | null;
  planRepair: PlanRepairSessionSnapshot | null;
  /**
   * P0 1.2（REQ-WIGA-08）：durable automation 归属（与 REST summary 同形）。
   * null=未知（首帧未到/服务端读取失败省略），不与 client 混淆——D6 以
   * owner!=="client" 退位发令。
   */
  automation: AutomationOwnership | null;

  snapshotGateIdentity: string | null;
  snapshotGateOpenedAt: string | null;
  humanGateClosure: HumanGateClosure | null;
  advanceCommands: Record<string, AdvanceCommandState>;
  protocolDiagnostics: ProtocolDiagnostic[];
  connectionCloseDiagnostics: ConnectionCloseDiagnostic[];
}

export interface WorkspaceSessionStatePayload {
  session_id: string;
  connection_id?: string;
  workspace_type: string;
  stage: string;
  superpowers_enabled?: boolean;
  openspec_enabled?: boolean;
  session_status: WorkspaceSessionStatus;
  flow_kind: WorkItemPlanFlowKind;
  run_policy: WorkItemPlanRunPolicy;
  run_history: WorkItemPlanRunHistory;
  review_invocation_scope?: unknown | null;
  human_gate_snapshot?: WorkItemPlanHumanGateSnapshot | null;
  repair_reservation?: WorkItemPlanRepairReservation | null;
  policy_diagnostics?: WorkItemPlanPolicyDiagnostic[];
  provider_start_ledger?: WorkItemPlanProviderStartLedgerEntry[];
  single_candidate_phase?: WorkItemPlanSingleCandidatePhase | null;
  work_item_plan_source_revision_ref?: string | null;
  plan_candidate_ir_ref?: string | null;
  mechanical_report_ref?: string | null;
  publication_provenance_ref?: string | null;
  messages: WsMessage[];
  checkpoints: WsCheckpoint[];
  artifact: WorkspaceArtifact;
  providers: WsProviderConfig;
  timeline_nodes?: TimelineNode[];
  active_node_id?: string | null;
  artifact_versions?: ArtifactVersion[];
  artifact_version_summaries?: ArtifactVersionSummary[];
  timeline_node_details?: Record<string, TimelineNodeDetail>;
  timeline_node_summaries?: Record<string, NodeDetailSummary>;
  active_run_id?: string | null;
  human_presentation_revisions?: HumanPresentationRevision[];
  reviewer_enabled_at_start?: boolean | null;
  recoverable_interrupted_run?: RecoverableInterruptedRun | null;
  plan_repair?: PlanRepairSessionSnapshot | null;
  /** F-59：挂起 choice 全量投影（含 role/created_at_ms）。 */
  pending_choice_requests?: unknown;
  /** P0 1.2：durable automation 归属；缺省=未知（store 保持 null）。 */
  automation?: AutomationOwnership | null;
}

export interface WorkspaceWsActions {
  setSessionState: (state: WorkspaceSessionStatePayload) => void;
  appendStreamChunk: (content: string, nodeId?: string | null) => void;
  appendBufferedStreamChunk: (content: string, nodeId: string, role: ChatEntryRole) => void;
  flushBufferedStream: (nodeId: string) => void;
  completeBufferedStream: (nodeId: string, messageId: string, checkpointId: string) => void;
  clearBufferedStream: (nodeId: string) => void;
  clearAllStreamBuffers: () => void;
  completeMessage: (messageId: string, checkpointId: string, nodeId?: string | null) => void;
  appendChatEntry: (entry: ChatEntry) => void;
  /** F-49 A1：门卡 upsert——门内轮次切换时把上一轮未决门卡留档（只读）。 */
  upsertGatePromptEntry: (entry: ChatEntry) => void;
  /** F-49 A4/B5：记录某张门卡本轮的反馈提交文本。 */
  recordGateFeedbackSubmission: (entryId: string, feedback: string) => void;
  resolveGateEntry: (resolution: ChatEntryResolution) => void;
  updateStreamingEntry: (entryId: string, content: string) => void;
  setEntryUsage: (entryId: string, usage: UsageReportPayload) => void;
  finalizeStreamingEntry: (entryId: string) => void;
  rebuildChatEntries: () => void;
  setStage: (stage: string) => void;
  setArtifact: (markdown: string, version?: number) => void;
  setWorkItemPlanCandidate: (candidate: WorkItemPlanCandidateDto | null) => void;
  setWorkItemPlanArtifact: (artifact: WorkItemPlanArtifactPayload | null, version?: number) => void;
  beginHumanPresentationSave: (sourceProjectionBundleId: string) => void;
  completeHumanPresentationSave: (revision: HumanPresentationRevision) => void;
  failHumanPresentationSave: (sourceProjectionBundleId: string, message: string) => void;
  failPendingHumanPresentationSaves: (message: string) => void;
  addTimelineNode: (node: TimelineNode) => void;
  updateTimelineNode: (
    nodeId: string,
    status: TimelineNodeStatus,
    summary?: string | null,
    completedAt?: string | null,
  ) => void;
  setSelectedNode: (nodeId: string | null) => void;
  setNodeDetail: (detail: TimelineNodeDetail) => void;
  setNodeVerdict: (nodeId: string, verdict: ReviewVerdict) => void;
  setContentCacheEntry: (key: string, value: string, now?: number) => void;
  touchContentCacheEntry: (key: string, now?: number) => void;
  setArtifactContentCacheEntry: (version: number, value: string, now?: number) => void;
  touchArtifactContentCacheEntry: (version: number, now?: number) => void;
  setPendingDecision: (decision: ReviewDecisionRequired | null) => void;
  setConnectionStatus: (status: WsConnectionStatus) => void;
  addPermissionRequest: (request: PermissionRequest) => void;
  resolvePermissionRequest: (id: string, approved?: boolean) => void;
  resolveChoiceRequest: (
    id: string,
    selectedOptionIds: string[],
    freeText: string | null,
    answers?: ChoiceAnswerPayload[],
  ) => void;
  rejectChoiceRequest: (id: string, reason: string) => void;
  setProviderStatus: (status: ProviderStatus) => void;
  upsertExecutionEvent: (event: ExecutionEvent) => void;
  clearExecutionEvents: () => void;
  setError: (error: string | null) => void;
  clearStreaming: () => void;
  selectNodeDetail: (nodeId: string | null | undefined) => TimelineNodeDetail | null;
  setProtocolError: (error: ProtocolErrorState | null) => void;
  setProviderLocked: (
    payload: { snapshot: ProviderConfigSnapshot; locked_at: string } | null,
  ) => void;
  setProviderSelection: (role: "author" | "reviewer", provider: WorkspaceProviderName) => void;
  setPermissionMode: (
    role: "author" | "reviewer",
    mode: ProviderPermissionMode,
  ) => void;
  setAcknowledgedAbortedNodes: (nodeIds: string[]) => void;
  applyHumanGateTurnOpen: (
    turnId: string,
    commandId: string,
    remainingBudget: number,
  ) => void;
  applyHumanGateTurnCompleted: (turnId: string, artifactRef: string) => void;
  applyHumanGateTurnFailed: (
    turnId: string,
    failureClass: string,
    message: string,
  ) => void;
  applyHumanGateBusy: (turnId: string) => void;
  applyHumanGateClosed: (decision: GateClosureDecision, stage: string) => void;
  applyAdvanceCompleted: (
    commandId: string,
    attemptId: string,
    workspaceEntry: string,
  ) => void;
  applyAdvanceRejected: (commandId: string, code: string, reason: string) => void;
  recordProtocolDiagnostic: (diagnostic: ProtocolDiagnostic) => void;
  recordConnectionCloseDiagnostic: (diagnostic: ConnectionCloseDiagnostic) => void;

  applyGateProtocolError: (turnId: string, error: InlineProtocolError) => void;
  applyAdvanceProtocolError: (commandId: string, error: InlineProtocolError) => void;
  setTimelineNodesForTest: (nodes: TimelineNode[]) => void;
  setActiveNodeId: (nodeId: string | null) => void;
  setSessionStatus: (status: WorkspaceSessionStatus) => void;
  setHumanGateSnapshot: (snapshot: WorkItemPlanHumanGateSnapshot | null) => void;
  setSessionIdForTest: (sessionId: string) => void;
  reset: () => void;
}
