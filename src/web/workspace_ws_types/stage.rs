use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStage {
    PrepareContext,
    Running,
    AuthorConfirm,
    CrossReview,
    ReviewDecision,
    Revision,
    HumanConfirm,
    Completed,
}
