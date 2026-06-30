import type { CodingAttempt } from "./coding";
import type {
  LifecycleConfirmationStatus,
  ProductIssue,
  ProviderWorkspaceConfigInput,
  WorkItemContextBudget,
  WorkItemExecutionPlanStatus,
  WorkItemKind,
} from "./common";
import type {
  IssueWorkItemPlanDetailDto,
  WorkItemSplitFinding,
} from "./work-item-plan";
import type { ArtifactVersion, WorkspaceSession } from "./workspace";

export type StorySpec = {
  story_spec_id: string;
  issue_id: string;
  repository_id: string;
  title: string;
  current_version: number | null;
  current_markdown_preview: string | null;
  confirmation_status: LifecycleConfirmationStatus;
  artifact_versions: ArtifactVersion[];
};

export type DesignSpec = {
  design_spec_id: string;
  issue_id: string;
  story_spec_ids: string[];
  title: string;
  current_version: number | null;
  current_markdown_preview: string | null;
  confirmation_status: LifecycleConfirmationStatus;
  artifact_versions: ArtifactVersion[];
};

export type LifecycleWorkItem = {
  work_item_id: string;
  issue_id: string;
  repository_id: string;
  story_spec_ids: string[];
  design_spec_ids: string[];
  title: string;
  plan_status: "not_started" | "draft" | "confirmed" | "change_requested";
  execution_status: "pending" | "planning" | "coding" | "completed" | "blocked";
  latest_attempt: CodingAttempt | null;
  artifact_versions: ArtifactVersion[];
  work_item_set_id: string | null;
  source_work_item_plan_id?: string | null;
  source_outline_id?: string | null;
  source_draft_id?: string | null;
  planned_implementation_context?: string | null;
  planned_handoff_summary?: string | null;
  kind: WorkItemKind;
  sequence_hint: number | null;
  depends_on: string[];
  exclusive_write_scopes: string[];
  forbidden_write_scopes: string[];
  context_budget: WorkItemContextBudget;
  required_handoff_from: string[];
  verification_plan_ref: string | null;
  require_execution_plan_confirm: boolean;
  execution_plan_status: WorkItemExecutionPlanStatus;
  handoff_summary_ref: string | null;
  completion_commit: string | null;
  completion_diff_summary_ref: string | null;
  validator_findings?: WorkItemSplitFinding[];
};

export type IssueLifecycleResponse = {
  issue: ProductIssue;
  story_specs: StorySpec[];
  design_specs: DesignSpec[];
  work_item_plans: IssueWorkItemPlanDetailDto[];
  work_items: LifecycleWorkItem[];
  workspace_sessions: WorkspaceSession[];
  coding_attempts: CodingAttempt[];
};

export type GenerateStorySpecsRequest = ProviderWorkspaceConfigInput & {
  title: string;
};

export type GenerateStorySpecsResponse = {
  story_specs: StorySpec[];
  workspace_session: WorkspaceSession;
};

export type GenerateDesignSpecsRequest = ProviderWorkspaceConfigInput & {
  title: string;
  story_spec_ids: string[];
};

export type GenerateDesignSpecsResponse = {
  design_specs: DesignSpec[];
  workspace_session: WorkspaceSession;
};
