use serde::{Deserialize, Serialize};

use crate::product::models::{
    WorkItemBatchStatus, WorkItemDraftRecord, WorkItemPlanCommitState, WorkItemPlanCompileStatus,
    WorkItemPlanOutline,
};

use super::plan_candidate::{ValidatorFindingDto, WorkItemPlanCandidateDto};

/// Artifact payload union for `WsOutMessage::ArtifactUpdate`.
///
/// Defined in WP1; WP2a will mount this payload into `ArtifactUpdate` and
/// `SessionState`, while WP2b/WP7 will produce the `WorkItemPlanCandidate`
/// variant from the author/reviewer generation flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ArtifactPayload {
    Markdown {
        markdown: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
    },
    WorkItemPlanCandidate {
        candidate: Box<WorkItemPlanCandidateDto>,
    },
    WorkItemPlanOutlineCandidate {
        outline_candidate: Box<WorkItemPlanOutlineCandidateDto>,
    },
    WorkItemPlanContextBlocker {
        context_blocker: Box<WorkItemPlanContextBlockerPayload>,
    },
    WorkItemDraftCandidate {
        draft_candidate: Box<WorkItemDraftCandidatePayload>,
    },
    WorkItemBatchState {
        batch_state: Box<WorkItemBatchStatePayload>,
    },
    WorkItemPlanCompileReport {
        compile_report: Box<WorkItemPlanCompileReportPayload>,
    },
}

impl ArtifactPayload {
    pub fn markdown(&self) -> Option<&str> {
        match self {
            Self::Markdown { markdown, .. } => Some(markdown.as_str()),
            Self::WorkItemPlanCandidate { .. } => None,
            Self::WorkItemPlanOutlineCandidate { .. } => None,
            Self::WorkItemPlanContextBlocker { .. } => None,
            Self::WorkItemDraftCandidate { .. } => None,
            Self::WorkItemBatchState { .. } => None,
            Self::WorkItemPlanCompileReport { .. } => None,
        }
    }

    pub fn markdown_or_empty(&self) -> &str {
        self.markdown().unwrap_or("")
    }

    pub fn into_markdown(self) -> Option<String> {
        match self {
            Self::Markdown { markdown, .. } => Some(markdown),
            Self::WorkItemPlanCandidate { .. } => None,
            Self::WorkItemPlanOutlineCandidate { .. } => None,
            Self::WorkItemPlanContextBlocker { .. } => None,
            Self::WorkItemDraftCandidate { .. } => None,
            Self::WorkItemBatchState { .. } => None,
            Self::WorkItemPlanCompileReport { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanOutlineCandidateDto {
    pub outline: WorkItemPlanOutline,
    pub design_context_gaps: Vec<String>,
    pub validator_findings: Vec<ValidatorFindingDto>,
    pub context_blockers: Vec<WorkItemPlanContextBlockerDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_generation_round_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_generation_mode: Option<super::in_::WorkItemGenerationModeDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanContextBlockerPayload {
    pub context_blockers: Vec<WorkItemPlanContextBlockerDto>,
    pub design_context_gaps: Vec<String>,
    pub exploration_summary: String,
    pub allowed_actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemDraftCandidatePayload {
    pub draft_record: WorkItemDraftRecord,
    pub validator_findings: Vec<ValidatorFindingDto>,
    pub can_accept: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemBatchStatePayload {
    pub batch_id: String,
    pub generation_round_id: String,
    pub queue: Vec<String>,
    pub draft_records: Vec<WorkItemDraftRecord>,
    pub batch_status: WorkItemBatchStatus,
    pub failure_summary: Vec<WorkItemBatchFailureSummaryDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanCompileReportPayload {
    pub compile_id: String,
    pub generation_round_id: String,
    pub status: WorkItemPlanCompileStatus,
    pub plan_commit_state: WorkItemPlanCommitState,
    pub work_item_ids: Vec<String>,
    pub verification_plan_ids: Vec<String>,
    pub child_session_ids: Vec<String>,
    pub validator_findings: Vec<ValidatorFindingDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemBatchFailureSummaryDto {
    pub draft_id: String,
    pub outline_id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanContextBlockerDto {
    pub code: String,
    pub message: String,
    pub needed_context: Vec<String>,
}
