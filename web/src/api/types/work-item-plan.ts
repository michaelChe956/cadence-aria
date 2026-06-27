import type {
  ProviderWorkspaceConfigInput,
  WorkItemContextBudget,
  WorkItemKind,
} from "./common";
import type { LifecycleWorkItem } from "./lifecycle";
import type {
  ArtifactVersionSummary,
  WorkspaceSession,
} from "./workspace";

export type RepositoryProfileConfidence = "low" | "medium" | "high";

export type RepositoryProfile = {
  profile_id: string;
  repository_id: string;
  confidence: RepositoryProfileConfidence;
  detected_layers: string[];
  split_recommendation: string;
};

export type VerificationPlan = {
  plan_ref: string;
  work_item_id: string;
  title: string;
  kind: string;
  scope_summary: string;
  required_checks: string[];
};

export type WorkItemSplitFinding = {
  finding_id: string;
  level: string;
  severity?: string;
  code?: string;
  message: string;
  affected_scopes: string[];
  work_item_ids?: string[];
};

export type WorkItemSplitOptions = {
  include_integration_tests: boolean;
  include_e2e_tests: boolean;
  force_frontend_backend_split: boolean;
  require_execution_plan_confirm: boolean;
};

export type IssueWorkItemPlan = {
  plan_id?: string;
  issue_id: string;
  status: string;
  options: WorkItemSplitOptions;
  created_at: string;
  updated_at: string;
};

export type GenerateWorkItemsRequest = ProviderWorkspaceConfigInput & {
  title: string;
  story_spec_ids: string[];
  design_spec_ids: string[];
  include_integration_tests?: boolean;
  include_e2e_tests?: boolean;
  force_frontend_backend_split?: boolean;
  require_execution_plan_confirm?: boolean;
};

export type GenerateWorkItemsResponse = {
  work_items: LifecycleWorkItem[];
  workspace_session: WorkspaceSession;
  workspace_sessions: WorkspaceSession[];
  work_item_plan: IssueWorkItemPlan;
  repository_profile: RepositoryProfile;
  verification_plans: VerificationPlan[];
  validator_findings: WorkItemSplitFinding[];
};

export type WorkItemSplitOptionsDto = WorkItemSplitOptions;

export type WorkItemDependencyEdgeDto = {
  from_work_item_id: string;
  to_work_item_id: string;
  dependency_type: "blocks" | "depends_on" | "related_to";
};

export type WorkItemCandidateMetaDto = {
  summary: string;
  scope_notes?: string[];
  acceptance_criteria?: string[];
};

export type WorkItemCandidateDto = {
  candidate_id: string;
  title: string;
  kind: string;
  exclusive_write_scopes: string[];
  depends_on: string[];
  verification_plan_ref: string | null;
  meta: WorkItemCandidateMetaDto;
  suggested_order?: number | null;
  reverted?: boolean;
  revert_feedback?: string | null;
};

export type ValidatorFindingDto = WorkItemSplitFinding;

export type WorkItemPlanDto = {
  plan_id: string;
  project_id: string;
  issue_id: string;
  title: string;
  source_story_spec_ids: string[];
  source_design_spec_ids: string[];
  options: WorkItemSplitOptionsDto;
  status: string;
  work_item_ids: string[];
  repository_profile_ref: string | null;
  verification_plan_ids: string[];
  dependency_graph: WorkItemDependencyEdgeDto[];
  created_from_provider_run: string | null;
  validator_findings: ValidatorFindingDto[];
  review_summary: string | null;
  created_at: string;
  updated_at: string;
};

export type WorkItemPlanCandidateDto = {
  plan: WorkItemPlanDto;
  work_items: WorkItemCandidateDto[];
  verification_plans: VerificationPlan[];
  repository_profile: RepositoryProfile | null;
  validator_findings: ValidatorFindingDto[];
};

export type WorkItemGenerationMode = "serial" | "batch";

export type WorkItemPlanOutlineItem = {
  outline_id: string;
  title: string;
  kind: WorkItemKind | string;
  goal?: string;
  scope?: string[];
  non_goals?: string[];
  source_story_spec_ids?: string[];
  source_design_spec_ids?: string[];
  depends_on?: string[];
  verification_intent?: string[];
  handoff_notes?: string;
  sequence_hint?: number | null;
  depends_on_outline_ids?: string[];
  exclusive_write_scopes: string[];
  forbidden_write_scopes: string[];
  context_budget?: WorkItemContextBudget;
  required_handoff_from_outline_ids?: string[];
  verification_strategy?: string;
  risk_notes?: string[];
};

export type WorkItemPlanOutline = {
  id: string;
  project_id?: string;
  issue_id?: string;
  plan_id: string;
  source_story_spec_ids?: string[];
  source_design_spec_ids?: string[];
  strategy_summary: string;
  work_items?: WorkItemPlanOutlineItem[];
  work_item_outlines?: WorkItemPlanOutlineItem[];
  dependency_graph: WorkItemDependencyEdgeDto[];
  risks: string[];
  handoff_plan?: string[];
  handoff_strategy?: string;
  status?: string;
  created_at?: string;
  updated_at?: string;
};

export type WorkItemPlanContextBlocker = {
  code: string;
  message: string;
  needed_context: string[];
};

export type WorkItemPlanOutlineCandidatePayload = {
  outline: WorkItemPlanOutline;
  design_context_gaps: string[];
  validator_findings: ValidatorFindingDto[];
  context_blockers: WorkItemPlanContextBlocker[];
  current_generation_round_id?: string | null;
  selected_generation_mode?: WorkItemGenerationMode | null;
};

export type WorkItemPlanContextBlockerPayload = {
  context_blockers: WorkItemPlanContextBlocker[];
  design_context_gaps: string[];
  exploration_summary: string;
  allowed_actions: string[];
};

export type WorkItemDraftVerificationCommand = {
  id?: string;
  label?: string;
  command?: string;
  description?: string;
  cwd?: string;
  purpose?: string;
  required?: boolean;
  timeout_seconds?: number;
  safety?: string;
  expected_exit_code?: number;
};

export type WorkItemDraftVerificationManualCheck = {
  label?: string;
  instructions?: string;
  required?: boolean;
};

export type WorkItemDraftVerificationPlan = {
  commands: WorkItemDraftVerificationCommand[];
  manual_checks: WorkItemDraftVerificationManualCheck[];
  required_gates: Array<
    | string
    | {
        gate_id?: string;
        name?: string;
        description?: string;
        depends_on?: string[];
      }
  >;
  risk_notes: string[];
};

export type WorkItemDraftCandidate = {
  outline_id: string;
  title: string;
  kind: WorkItemKind | string;
  goal?: string;
  implementation_context: string;
  exclusive_write_scopes: string[];
  forbidden_write_scopes: string[];
  depends_on_outline_ids: string[];
  required_handoff_from_outline_ids: string[];
  verification_plan: WorkItemDraftVerificationPlan;
  handoff_summary: string;
};

export type WorkItemDraftStatus =
  | "draft"
  | "accepted"
  | "superseded"
  | "validation_failed"
  | "copied";

export type WorkItemDraftRecord = {
  project_id?: string;
  issue_id?: string;
  draft_id: string;
  plan_id: string;
  generation_round_id: string;
  outline_id: string;
  batch_id?: string | null;
  attempt_index?: number;
  outline_version_ref?: string;
  generation_mode?: WorkItemGenerationMode | string;
  candidate: WorkItemDraftCandidate;
  status: WorkItemDraftStatus | string;
  active: boolean;
  superseded?: boolean;
  superseded_by_draft_id?: string | null;
  supersede_reason?: string | null;
  copied_from_draft_id?: string | null;
  generated_from_node_id: string;
  accepted_by_node_id?: string | null;
  created_at: string;
  updated_at: string;
};

export type WorkItemDraftCandidatePayload = {
  draft_record: WorkItemDraftRecord;
  validator_findings: ValidatorFindingDto[];
  can_accept: boolean;
};

export type WorkItemBatchFailureSummary = {
  draft_id: string;
  outline_id: string;
  status: string;
};

export type WorkItemBatchStatePayload = {
  batch_id: string;
  generation_round_id: string;
  queue: string[];
  draft_records: WorkItemDraftRecord[];
  batch_status: "generating" | "completed" | "review_pending" | "review_done" | string;
  failure_summary: WorkItemBatchFailureSummary[];
};

export type WorkItemPlanCompileReportPayload = {
  compile_id: string;
  generation_round_id: string;
  status: "preparing" | "committing" | "committed" | "failed" | "recovery_required" | string;
  plan_commit_state: "not_started" | "committed" | "rolled_back" | string;
  work_item_ids: string[];
  verification_plan_ids: string[];
  child_session_ids: string[];
  validator_findings: ValidatorFindingDto[];
};

export type WorkItemPlanArtifactPayload =
  | { type: "outline_candidate"; payload: WorkItemPlanOutlineCandidatePayload }
  | { type: "context_blocker"; payload: WorkItemPlanContextBlockerPayload }
  | { type: "draft_candidate"; payload: WorkItemDraftCandidatePayload }
  | { type: "batch_state"; payload: WorkItemBatchStatePayload }
  | { type: "compile_report"; payload: WorkItemPlanCompileReportPayload };

export type WorkItemPlanArtifactVersion = ArtifactVersionSummary & {
  artifact?: WorkItemPlanArtifactPayload | null;
};

export type IssueWorkItemPlanDependencyEdgeDto = {
  from_work_item_id: string;
  to_work_item_id: string;
};

export type IssueWorkItemPlanDetailDto = {
  id: string;
  issue_id: string;
  project_id: string;
  status: string;
  source_story_spec_ids: string[];
  source_design_spec_ids: string[];
  work_item_ids: string[];
  verification_plan_ids: string[];
  dependency_graph: IssueWorkItemPlanDependencyEdgeDto[];
  repository_profile_ref: string | null;
  options: WorkItemSplitOptions;
  validator_findings: WorkItemSplitFinding[];
  created_at: string;
  updated_at: string;
};

export type PrepareWorkItemPlanRequest = ProviderWorkspaceConfigInput & {
  title: string;
  story_spec_ids?: string[] | null;
  design_spec_ids?: string[] | null;
  include_integration_tests?: boolean | null;
  include_e2e_tests?: boolean | null;
  force_frontend_backend_split?: boolean | null;
  require_execution_plan_confirm?: boolean | null;
};

export type PrepareWorkItemPlanResponse = {
  work_item_plan: IssueWorkItemPlanDetailDto;
  workspace_session: WorkspaceSession;
};
