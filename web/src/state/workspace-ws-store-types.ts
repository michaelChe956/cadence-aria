import type {
  NodeDetail,
  WorkItemBatchStatePayload,
  WorkItemDraftCandidatePayload,
  WorkItemPlanArtifactPayload,
  WorkItemPlanArtifactVersion,
  WorkItemPlanCandidateDto,
  WorkItemPlanCompileReportPayload,
  WorkItemPlanContextBlockerPayload,
  WorkItemPlanOutlineCandidatePayload,
  WorkspaceProviderName,
} from "../api/types";
import type {
  ChatEntry,
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
  | { compile_report: WorkItemPlanCompileReportPayload };

export type WsConnectionStatus = "disconnected" | "connecting" | "connected" | "error";
export type ProviderStatus =
  | "starting"
  | "running"
  | "waiting_approval"
  | "completed"
  | "failed"
  | "aborted";
export type ExecutionEventKind = "provider" | "turn" | "command" | "output" | "artifact";
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
}

export interface ProviderConfigSnapshot {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
  review_rounds: number;
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
}

export type ReviewFindingSeverity =
  | "blocking"
  | "must_fix"
  | "strong_recommend_fix"
  | "suggestion"
  | "minor"
  | "optional";

export type ReviewGate =
  | "requires_revision"
  | "user_confirm_allowed"
  | "user_triage_required";

export interface ReviewFinding {
  severity: ReviewFindingSeverity;
  message: string;
  evidence: string;
  impact: string;
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
  markdown: string;
}

export type TimelineNodeDetail = NodeDetail;

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

export interface WorkspaceWsState {
  sessionId: string | null;
  workspaceType: string | null;
  stage: string;
  superpowersEnabled: boolean;
  openSpecEnabled: boolean;
  visitedStages: string[];
  messages: WsMessage[];
  checkpoints: WsCheckpoint[];
  chatEntries: ChatEntry[];
  artifact: string | null;
  workItemPlanCandidate: WorkItemPlanCandidateDto | null;
  workItemPlanArtifact: WorkItemPlanArtifactPayload | null;
  workItemPlanArtifactVersions: WorkItemPlanArtifactVersion[];
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
  protocolError: ProtocolErrorState | null;
  providerLocked: boolean;
  providerSnapshot: ProviderConfigSnapshot | null;
  providerLockedAt: string | null;
  acknowledgedAbortedNodes: string[];
  reviewerEnabled: boolean;
  reviewRounds: number;
  pendingReviewDecision: { verdict: string; summary: string } | null;
  pendingReviewerSummary: { verdict: string; points: string[] } | null;
}

export interface WorkspaceWsActions {
  setSessionState: (state: {
    session_id: string;
    workspace_type: string;
    stage: string;
    superpowers_enabled?: boolean;
    openspec_enabled?: boolean;
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
  }) => void;
  appendStreamChunk: (content: string, nodeId?: string | null) => void;
  appendBufferedStreamChunk: (content: string, nodeId: string, role: ChatEntryRole) => void;
  flushBufferedStream: (nodeId: string) => void;
  completeBufferedStream: (nodeId: string, messageId: string, checkpointId: string) => void;
  clearBufferedStream: (nodeId: string) => void;
  clearAllStreamBuffers: () => void;
  completeMessage: (messageId: string, checkpointId: string, nodeId?: string | null) => void;
  appendChatEntry: (entry: ChatEntry) => void;
  resolveGateEntry: (resolution: ChatEntryResolution) => void;
  updateStreamingEntry: (entryId: string, content: string) => void;
  finalizeStreamingEntry: (entryId: string) => void;
  rebuildChatEntries: () => void;
  setStage: (stage: string) => void;
  setArtifact: (markdown: string, version?: number) => void;
  setWorkItemPlanCandidate: (candidate: WorkItemPlanCandidateDto | null) => void;
  setWorkItemPlanArtifact: (artifact: WorkItemPlanArtifactPayload | null, version?: number) => void;
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
  setAcknowledgedAbortedNodes: (nodeIds: string[]) => void;
  reset: () => void;
}

