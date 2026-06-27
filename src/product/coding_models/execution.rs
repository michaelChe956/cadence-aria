use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CodingAttemptScope {
    #[default]
    WorkItem,
    WorkItemGroup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodingExecutionAttempt {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub attempt_no: u32,
    #[serde(default)]
    pub scope: CodingAttemptScope,
    pub status: CodingAttemptStatus,
    pub stage: CodingExecutionStage,
    pub base_branch: String,
    pub branch_name: String,
    pub worktree_path: Option<PathBuf>,
    pub provider_config_snapshot: ProviderConfigSnapshot,
    pub rework_count: u32,
    pub max_auto_rework: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item_group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_work_item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_unit_id: Option<String>,
    pub head_commit: Option<String>,
    pub pushed_remote: Option<String>,
    pub review_request_id: Option<String>,
    #[serde(default)]
    pub provider_conversations: Vec<ProviderConversationRef>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Deserialize)]
struct CodingExecutionAttemptSerde {
    id: String,
    project_id: String,
    issue_id: String,
    work_item_id: String,
    attempt_no: u32,
    #[serde(default)]
    scope: CodingAttemptScope,
    status: CodingAttemptStatus,
    stage: CodingExecutionStage,
    base_branch: String,
    branch_name: String,
    worktree_path: Option<PathBuf>,
    provider_config_snapshot: ProviderConfigSnapshot,
    rework_count: u32,
    max_auto_rework: u32,
    #[serde(default)]
    work_item_group_id: Option<String>,
    #[serde(default)]
    current_work_item_id: Option<String>,
    #[serde(default)]
    active_unit_id: Option<String>,
    head_commit: Option<String>,
    pushed_remote: Option<String>,
    review_request_id: Option<String>,
    #[serde(default)]
    provider_conversations: Vec<ProviderConversationRef>,
    created_at: String,
    updated_at: String,
    completed_at: Option<String>,
}

impl<'de> Deserialize<'de> for CodingExecutionAttempt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = CodingExecutionAttemptSerde::deserialize(deserializer)?;
        let current_work_item_id = raw.current_work_item_id.or_else(|| {
            if raw.scope == CodingAttemptScope::WorkItem {
                Some(raw.work_item_id.clone())
            } else {
                None
            }
        });
        Ok(Self {
            id: raw.id,
            project_id: raw.project_id,
            issue_id: raw.issue_id,
            work_item_id: raw.work_item_id,
            attempt_no: raw.attempt_no,
            scope: raw.scope,
            status: raw.status,
            stage: raw.stage,
            base_branch: raw.base_branch,
            branch_name: raw.branch_name,
            worktree_path: raw.worktree_path,
            provider_config_snapshot: raw.provider_config_snapshot,
            rework_count: raw.rework_count,
            max_auto_rework: raw.max_auto_rework,
            work_item_group_id: raw.work_item_group_id,
            current_work_item_id,
            active_unit_id: raw.active_unit_id,
            head_commit: raw.head_commit,
            pushed_remote: raw.pushed_remote,
            review_request_id: raw.review_request_id,
            provider_conversations: raw.provider_conversations,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
            completed_at: raw.completed_at,
        })
    }
}
