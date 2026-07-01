import type {
  AuthorDecision,
  HumanConfirmDecision,
  ProviderConfigSnapshot,
  ProviderWorkspaceConfigInput,
  ReviewGate,
  ReviewVerdictType,
  RevisionPath,
  StructuredFeedback,
  WorkspaceProviderName,
  WorkspaceReviewFindingSeverity,
} from "./common";
import type {
  WorkItemBatchStatePayload,
  WorkItemDraftCandidatePayload,
  WorkItemGenerationMode,
  WorkItemPlanArtifactPayload,
  WorkItemPlanCandidateDto,
  WorkItemPlanCompileReportPayload,
  WorkItemPlanContextBlockerPayload,
  WorkItemPlanOutlineCandidatePayload,
} from "./work-item-plan";

export type WorkspaceMessage = {
  role: string;
  content: string;
  created_at: string;
};

export type WorkspaceSession = {
  workspace_session_id: string;
  issue_id: string;
  entity_id: string;
  workspace_type: "story" | "design" | "work_item" | "work_item_plan";
  status:
    | "open"
    | "running"
    | "waiting_for_human"
    | "confirmed"
    | "change_requested"
    | "blocked_provider_unavailable"
    | "terminated";
  author_provider: WorkspaceProviderName;
  reviewer_provider: WorkspaceProviderName;
  review_rounds: number;
  superpowers_enabled: boolean;
  openspec_enabled: boolean;
  messages: WorkspaceMessage[];
};

export type ArtifactUpdateMessage =
  | { type: "artifact_update"; version: number; markdown: string; diff?: string | null }
  | { type: "artifact_update"; version: number; candidate: WorkItemPlanCandidateDto }
  | {
      type: "artifact_update";
      version: number;
      outline_candidate: WorkItemPlanOutlineCandidatePayload;
    }
  | { type: "artifact_update"; version: number; context_blocker: WorkItemPlanContextBlockerPayload }
  | { type: "artifact_update"; version: number; draft_candidate: WorkItemDraftCandidatePayload }
  | { type: "artifact_update"; version: number; batch_state: WorkItemBatchStatePayload }
  | { type: "artifact_update"; version: number; compile_report: WorkItemPlanCompileReportPayload };

export type RevertWorkItemMessage = {
  type: "revert_work_item";
  work_item_id: string;
  feedback?: string | null;
  clear: boolean;
};

export type WorkItemDraftDecision = "accept" | "rewrite" | "pause";
export type WorkItemBatchDecision =
  | "accept_all"
  | "rewrite_batch"
  | "pause"
  | "downgrade_to_serial";
export type WorkItemPlanCompileRecoveryAction =
  | "continue"
  | "abort_and_rollback"
  | "human_triage";

export type WsInMessage =
  | { type: "user_message"; content: string }
  | { type: "context_note"; content: string }
  | {
      type: "start_generation";
      provider_config: ProviderConfigSnapshot;
      reviewer_enabled: boolean;
    }
  | { type: "rollback"; checkpoint_id: string }
  | { type: "confirm" }
  | { type: "provider_select"; role: string; provider: WorkspaceProviderName }
  | { type: "permission_response"; id: string; approved: boolean; reason?: string | null }
  | {
      type: "choice_response";
      id: string;
      selected_option_ids: string[];
      free_text?: string | null;
      answers?: ChoiceAnswer[];
    }
  | { type: "review_decision_response"; decision: string; extra_context?: string | null }
  | { type: "author_decision"; decision: AuthorDecision }
  | { type: "select_revision_path"; path: RevisionPath; extra_context?: string | null }
  | { type: "request_revision"; feedback: StructuredFeedback }
  | RevertWorkItemMessage
  | { type: "select_work_item_generation_mode"; mode: WorkItemGenerationMode }
  | { type: "request_outline_revision"; feedback?: string | null }
  | {
      type: "work_item_draft_decision";
      outline_id: string;
      decision: WorkItemDraftDecision;
      feedback?: string | null;
    }
  | {
      type: "work_item_batch_decision";
      decision: WorkItemBatchDecision;
      feedback?: string | null;
      first_affected_outline_id?: string | null;
    }
  | {
      type: "work_item_plan_compile_recovery_action";
      action: WorkItemPlanCompileRecoveryAction;
      reason?: string | null;
    }
  | { type: "human_confirm"; decision: HumanConfirmDecision; payload?: unknown }
  | { type: "abort" }
  | { type: "hello"; session_id: string; last_seen_node_id?: string | null }
  | { type: "ping" };

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

export type WorkspaceReviewFinding = {
  severity: WorkspaceReviewFindingSeverity;
  message: string;
  evidence: string;
  impact: string;
  required_action: string;
};

export type WsMessage = {
  id: string;
  role: string;
  content: string;
  checkpoint_id?: string | null;
  created_at: string;
};

export type WsCheckpoint = {
  id: string;
  message_index: number;
  stage: string;
  created_at: string;
};

export type WsProviderConfig = {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
};

export type ProviderDefaults = {
  author: WorkspaceProviderName;
  reviewer?: WorkspaceProviderName | null;
  review_rounds: number;
};

export type TimelineNodeRetryError = {
  code: string;
  message: string;
};

export type TimelineNodeRetry = {
  retry_of_node_id: string;
  retry_attempt: number;
  retry_reason: string;
  retry_error: TimelineNodeRetryError;
};

export type TimelineNode = {
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
};

export type ReviewVerdict = {
  verdict: ReviewVerdictType;
  comments: string;
  summary: string;
  findings?: WorkspaceReviewFinding[];
  review_gate?: ReviewGate;
};

export type ExecutionEvent = {
  event_id: string;
  node_id?: string | null;
  agent?: WorkspaceProviderName | string | null;
  kind: ExecutionEventKind;
  status: ExecutionEventStatus;
  title: string;
  detail?: string | null;
  command?: string | null;
  cwd?: string | null;
  output?: string | null;
  exit_code?: number | null;
};

export type ChoiceOption = {
  id: string;
  label: string;
  description?: string | null;
};

export type ChoiceQuestion = {
  id: string;
  prompt: string;
  options: ChoiceOption[];
  allow_multiple: boolean;
  allow_free_text: boolean;
};

export type ChoiceAnswer = {
  question_id: string;
  selected_option_ids: string[];
  free_text?: string | null;
};

export type WorkspaceChoiceRequestSource =
  | "ask_user_question"
  | "request_user_input"
  | "text_fallback"
  | "provider_choice";

export type ArtifactVersion = {
  version: number;
  markdown: string;
  generated_by: WorkspaceProviderName;
  reviewed_by?: WorkspaceProviderName | null;
  review_verdict?: ReviewVerdictType | null;
  confirmed_by?: string | null;
  is_current?: boolean;
  created_at: string;
  source_node_id: string;
};

export type ArtifactVersionSummary = Omit<ArtifactVersion, "markdown"> & { markdown?: string };

export type ProviderSnapshot = {
  name: string;
  model: string;
};

export type ArtifactRef = {
  artifact_id: string;
  version: number;
};

export type PermissionEvent = {
  request_id: string;
  request: unknown;
  response: unknown | null;
  ts: string;
};

export type NodeDetail = {
  node_id: string;
  session_id: string;
  node_type: TimelineNodeType;
  status: TimelineNodeStatus;
  agent_role: "author" | "reviewer" | null;
  provider: ProviderSnapshot | null;
  prompt?: string | null;
  messages: WsMessage[];
  streaming_content: string;
  execution_events: ExecutionEvent[];
  permission_events: PermissionEvent[];
  verdict: ReviewVerdict | null;
  artifact_ref: ArtifactRef | null;
  is_revision: boolean;
  base_artifact_ref: ArtifactRef | null;
  started_at: string;
  ended_at: string | null;
};

export type WorkspaceNodeDetailResponse = NodeDetail;

export type WorkspacePromptResponse = {
  node_id: string;
  prompt: string;
};

export type WorkspaceEventOutputResponse = {
  node_id: string;
  event_id: string;
  output: string;
};

export type WorkspaceArtifactVersionResponse = {
  version: number;
  markdown: string;
  artifact?:
    | { markdown: string; diff?: string | null }
    | { candidate: WorkItemPlanCandidateDto }
    | { outline_candidate: WorkItemPlanOutlineCandidatePayload }
    | { context_blocker: WorkItemPlanContextBlockerPayload }
    | { draft_candidate: WorkItemDraftCandidatePayload }
    | { batch_state: WorkItemBatchStatePayload }
    | { compile_report: WorkItemPlanCompileReportPayload }
    | WorkItemPlanArtifactPayload
    | null;
  generated_by?: WorkspaceProviderName;
  reviewed_by?: WorkspaceProviderName | null;
  review_verdict?: ReviewVerdictType | null;
  confirmed_by?: string | null;
  is_current?: boolean;
  created_at?: string;
  source_node_id?: string;
};

export type WsOutMessage =
  | { type: "stream_chunk"; role: string; content: string; node_id?: string | null }
  | {
      type: "message_complete";
      message_id: string;
      checkpoint_id: string;
      node_id?: string | null;
    }
  | { type: "stage_change"; stage: string }
  | ArtifactUpdateMessage
  | { type: "provider_select_request"; stage: string; defaults: ProviderDefaults }
  | {
      type: "permission_request";
      id: string;
      tool_name: string;
      description: string;
      risk_level: "low" | "medium" | "high";
    }
  | {
      type: "choice_request";
      id: string;
      prompt: string;
      options: ChoiceOption[];
      allow_multiple: boolean;
      allow_free_text: boolean;
      questions?: ChoiceQuestion[];
      source: WorkspaceChoiceRequestSource;
    }
  | { type: "provider_status"; status: ProviderStatus }
  | { type: "execution_event"; event: ExecutionEvent }
  | { type: "timeline_node_created"; node: TimelineNode }
  | {
      type: "timeline_node_updated";
      node_id: string;
      status: TimelineNodeStatus;
      summary?: string | null;
      completed_at?: string | null;
    }
  | {
      type: "review_complete";
      node_id: string;
      round: number;
      verdict: ReviewVerdictType;
      comments: string;
      summary: string;
      findings?: WorkspaceReviewFinding[];
      review_gate?: ReviewGate;
    }
  | { type: "review_decision_required"; node_id: string; round: number; options: string[] }
  | {
      type: "session_state";
      session_id: string;
      workspace_type: string;
      stage: string;
      superpowers_enabled: boolean;
      openspec_enabled: boolean;
      messages: WsMessage[];
      checkpoints: WsCheckpoint[];
      artifact:
        | string
        | null
        | { markdown: string; diff?: string | null }
        | { candidate: WorkItemPlanCandidateDto };
      providers: WsProviderConfig;
      timeline_nodes: TimelineNode[];
      active_node_id: string | null;
      artifact_versions: ArtifactVersion[];
      artifact_version_summaries?: ArtifactVersionSummary[];
      timeline_node_details: Record<string, NodeDetail>;
      active_run_id: string | null;
    }
  | { type: "error"; message: string }
  | { type: "protocol_error"; code: string; message: string; context?: unknown }
  | { type: "provider_locked"; snapshot: ProviderConfigSnapshot; locked_at: string }
  | { type: "pong" };
