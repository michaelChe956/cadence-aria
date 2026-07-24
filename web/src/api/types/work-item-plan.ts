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

export type WorkItemOutlineSessionFit =
  | "fits_single_agent_session"
  | "too_large_must_split";

export type WorkItemPlanOutlineItem = {
  outline_id: string;
  title: string;
  kind: WorkItemKind | string;
  goal?: string;
  scope?: string[];
  non_goals?: string[];
  estimated_context_tokens?: number;
  session_fit?: WorkItemOutlineSessionFit;
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

export type WorkItemDraftGenerationDiagnostics = {
  auto_repair_attempted: boolean;
  initial_validation_findings: ValidatorFindingDto[];
  final_validation_findings: ValidatorFindingDto[];
};

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
  generation_diagnostics?: WorkItemDraftGenerationDiagnostics | null;
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
  | { type: "compile_report"; payload: WorkItemPlanCompileReportPayload }
  | { type: "plan_projection"; payload: PlanProjectionBundle }
  | { type: "work_item_projection"; payload: WorkItemProjectionBundle }
  | { type: "work_item_revision_history"; payload: WorkItemRevisionHistoryDto }
  | { type: "projection_validation"; payload: ProjectionValidationReport };

export type WorkItemPlanArtifactVersion = ArtifactVersionSummary & {
  artifact?: WorkItemPlanArtifactPayload | null;
};

export type WorkItemProjectionTab =
  | "overview"
  | "contract"
  | "coder"
  | "reviewer"
  | "history";

export type ContractCompatibilityPolicy = "require_all" | "require_any";

export type RequiredDependencyContract = {
  contract_id: string;
  required_capabilities: string[];
  compatibility_policy: ContractCompatibilityPolicy;
};

export type DependencyContractEdge = {
  from: string;
  to: string;
  required_contracts: RequiredDependencyContract[];
};

export type WorkItemWritePolicy = {
  exclusive_scopes: string[];
  forbidden_scopes: string[];
};

export type HumanScopeSummary = {
  owned_scopes: string[];
  forbidden_scopes: string[];
};

export type HumanGroupProjection = {
  plan_id: string;
  goal: string;
  split_reason: string;
  work_items: Array<{
    logical_work_item_id: string;
    title: string;
    goal: string;
    depends_on: string[];
    provides: string[];
    scope_summary: HumanScopeSummary;
  }>;
  contract_flow: Array<{
    from: string;
    to: string;
    contract_id: string;
    required_capabilities: string[];
    provided_capabilities: string[];
    missing_capabilities: string[];
  }>;
  risks: string[];
  source_refs: string[];
  normative: false;
  used_by_provider: false;
};

export type CoderGroupContext = {
  plan_id: string;
  ordered_logical_work_item_ids: string[];
  dependency_edges: DependencyContractEdge[];
  group_write_scopes: Record<string, WorkItemWritePolicy>;
};

export type ReviewerGroupMatrix = {
  plan_id: string;
  work_items: Array<{
    logical_work_item_id: string;
    criterion_refs: string[];
    input_contract_refs: string[];
    output_contract_refs: string[];
  }>;
  dependency_edges: DependencyContractEdge[];
  design_traceability_refs: Array<{
    source_type: string;
    source_id: string;
    requirement_id: string;
  }>;
};

export type RequiredInputContract = {
  contract_id: string;
  provider_logical_work_item_id: string;
  required_capabilities: string[];
  compatibility_policy: ContractCompatibilityPolicy;
};

export type PromisedOutputContract = {
  contract_id: string;
  capabilities: string[];
};

export type WorkItemTask = {
  task_id: string;
  statement: string;
  requirement_refs: string[];
  done_when_refs: string[];
};

export type EvidenceKind =
  | "source_diff"
  | "non_zero_test_execution"
  | "manual_check"
  | "handoff_field";

export type AcceptanceCriterion = {
  criterion_id: string;
  statement: string;
  required_evidence: EvidenceKind[];
};

export type VerificationCheck = {
  check_id: string;
  command: string | null;
  manual_instruction: string | null;
  required: boolean;
  non_zero_test_execution_required: boolean;
};

export type BlockerRoute =
  | "coder_rework"
  | "verification_retry"
  | "plan_repair_current"
  | "plan_repair_upstream"
  | "subgraph_replan"
  | "story_amendment"
  | "design_amendment"
  | "operational_gate";

export type BlockerRule = {
  reason_code: string;
  route: BlockerRoute;
  target_contract_refs: string[];
};

export type HandoffContract = {
  required_fields: string[];
  provided_contract_refs: string[];
  reviewer_check_refs: string[];
};

export type HumanWorkItemProjection = {
  logical_work_item_id: string;
  title: string;
  goal: string;
  non_goals: string[];
  inputs: Array<{
    contract_id: string;
    capabilities: string[];
    source_refs: string[];
  }>;
  outputs: Array<{
    contract_id: string;
    capabilities: string[];
    source_refs: string[];
  }>;
  dependencies: string[];
  scope_summary: HumanScopeSummary;
  completion_summary: string[];
  source_refs: string[];
  normative: false;
  used_by_provider: false;
};

export type CoderWorkItemProjection = {
  work_item_revision_id: string;
  objective: string;
  required_input_contracts: RequiredInputContract[];
  task_refs: string[];
  tasks: WorkItemTask[];
  write_policy: WorkItemWritePolicy;
  acceptance_criteria: AcceptanceCriterion[];
  verification_checks: VerificationCheck[];
  blocker_rules: BlockerRule[];
  handoff_contract: HandoffContract;
};

export type ReviewerWorkItemProjection = {
  work_item_revision_id: string;
  criterion_refs: string[];
  requirement_matrix: Array<{
    criterion_id: string;
    requirement_refs: string[];
    required_evidence: EvidenceKind[];
    failure_route: BlockerRoute;
  }>;
  scope_policy: WorkItemWritePolicy;
  input_contract_checks: RequiredInputContract[];
  output_contract_checks: PromisedOutputContract[];
  verification_evidence_rules: VerificationCheck[];
  blocker_routing: BlockerRule[];
};

export type WorkItemProjectionBundle = {
  id: string;
  work_item_revision_id: string;
  canonical_contract_hash: string;
  projection_schema_version: number;
  compiler_version: string;
  human_projection: HumanWorkItemProjection;
  coder_projection: CoderWorkItemProjection;
  reviewer_projection: ReviewerWorkItemProjection;
  human_projection_hash: string;
  coder_projection_hash: string;
  reviewer_projection_hash: string;
  created_at: string;
};

export type WorkItemHistoryEntry = {
  kind:
    | "draft_revision"
    | "work_item_revision"
    | "plan_review"
    | "contract_delta"
    | "unit_run"
    | "handoff_revision";
  id: string;
  logical_work_item_id: string;
  related_revision_id: string | null;
  summary: string;
  created_at: string;
};

export type WorkItemRevisionHistoryDto = {
  entries: WorkItemHistoryEntry[];
};

export type PlanProjectionBundle = {
  id: string;
  plan_revision_id: string;
  dependency_graph_revision_id: string;
  work_item_projection_bundle_refs: string[];
  human_group_projection: HumanGroupProjection;
  coder_group_context: CoderGroupContext;
  reviewer_group_matrix: ReviewerGroupMatrix;
  human_group_projection_hash: string;
  coder_group_context_hash: string;
  reviewer_group_matrix_hash: string;
  compiler_version: string;
  created_at: string;
};

export type ProjectionValidationFinding = {
  code: string;
  projection: string;
  contract_ref: string | null;
  message: string;
};

export type ProjectionValidationReport = {
  findings: ProjectionValidationFinding[];
};

export type HumanPresentationRevision = {
  id: string;
  source_plan_projection_bundle_id: string | null;
  source_work_item_projection_bundle_id: string | null;
  supersedes: string | null;
  human_summary: string;
  why_split: string | null;
  dependency_explanation: string[];
  risk_explanation: string[];
  source_refs: string[];
  normative: false;
  used_by_provider: false;
  created_at: string;
};

export type HumanPresentationScope = "plan" | "work_item";

export type SaveHumanPresentationRevisionMessage = {
  type: "save_human_presentation_revision";
  source_projection_bundle_id: string;
  scope: HumanPresentationScope;
  supersedes: string | null;
  human_summary: string;
  why_split: string | null;
  dependency_explanation: string[];
  risk_explanation: string[];
  source_refs: string[];
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
