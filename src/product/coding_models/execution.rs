use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::product::models::ProviderConversationRef;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingExecutionStage {
    PrepareContext,
    WorktreePrepare,
    Coding,
    Testing,
    CodeReview,
    Rework,
    ReviewRequest,
    InternalPrReview,
    FinalConfirm,
}

impl CodingExecutionStage {
    pub fn order(&self) -> u8 {
        match self {
            Self::PrepareContext => 0,
            Self::WorktreePrepare => 1,
            Self::Coding => 2,
            Self::Testing => 3,
            Self::CodeReview => 4,
            Self::Rework => 5,
            Self::ReviewRequest => 6,
            Self::InternalPrReview => 7,
            Self::FinalConfirm => 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingAttemptStatus {
    Created,
    Running,
    WaitingForHuman,
    Blocked,
    Completed,
    Failed,
    Aborted,
}

impl CodingAttemptStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Created | Self::Running | Self::WaitingForHuman | Self::Blocked
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingProviderRole {
    Coder,
    Tester,
    Analyst,
    CodeReviewer,
    InternalReviewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingAgentRole {
    Author,
    Tester,
    Reviewer,
    Git,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingExecutionAttempt {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub attempt_no: u32,
    pub status: CodingAttemptStatus,
    pub stage: CodingExecutionStage,
    pub base_branch: String,
    pub branch_name: String,
    pub worktree_path: Option<PathBuf>,
    pub provider_config_snapshot: ProviderConfigSnapshot,
    pub rework_count: u32,
    pub max_auto_rework: u32,
    pub head_commit: Option<String>,
    pub pushed_remote: Option<String>,
    pub review_request_id: Option<String>,
    #[serde(default)]
    pub provider_conversations: Vec<ProviderConversationRef>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}
