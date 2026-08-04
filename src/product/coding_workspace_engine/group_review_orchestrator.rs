#![allow(dead_code)]

#[cfg(test)]
use std::collections::VecDeque;
#[cfg(test)]
use std::sync::Mutex;

use super::group_review_types::PromptBudgetBreakdown;
use crate::product::json_store::ProductStoreError;

#[async_trait::async_trait]
pub(crate) trait GroupReviewExecutor: Send + Sync {
    async fn execute(
        &self,
        prompt: &str,
    ) -> Result<GroupReviewExecutionResult, GroupReviewExecutionError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GroupReviewExecutionResult {
    pub full_output: String,
    pub provider_session_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GroupReviewExecutionError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("user_cancelled")]
    UserCancelled,
    #[error("internal: {0}")]
    Internal(String),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GroupReviewOrchestrationError {
    #[error("capacity_exceeded")]
    CapacityExceeded,
    #[error("material_overflow")]
    MaterialOverflow { breakdown: PromptBudgetBreakdown },
    #[error("shard_output_invalid: {shard_id}")]
    ShardOutputInvalid { shard_id: String, raw_ref: String },
    #[error("store: {0}")]
    Store(#[from] ProductStoreError),
    #[error("executor: {0}")]
    Executor(#[from] GroupReviewExecutionError),
}

#[cfg(test)]
pub(crate) struct FakeGroupReviewExecutor {
    results: Mutex<VecDeque<Result<GroupReviewExecutionResult, GroupReviewExecutionError>>>,
    prompts: Mutex<Vec<String>>,
}

#[cfg(test)]
impl FakeGroupReviewExecutor {
    pub(crate) fn new(
        results: Vec<Result<GroupReviewExecutionResult, GroupReviewExecutionError>>,
    ) -> Self {
        Self {
            results: Mutex::new(results.into()),
            prompts: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn push_result(
        &self,
        result: Result<GroupReviewExecutionResult, GroupReviewExecutionError>,
    ) {
        self.results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(result);
    }

    pub(crate) fn prompts(&self) -> Vec<String> {
        self.prompts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl GroupReviewExecutor for FakeGroupReviewExecutor {
    async fn execute(
        &self,
        prompt: &str,
    ) -> Result<GroupReviewExecutionResult, GroupReviewExecutionError> {
        self.prompts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(prompt.to_string());
        self.results
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
            .unwrap_or_else(|| {
                Err(GroupReviewExecutionError::Internal(
                    "fake_group_review_executor_exhausted".to_string(),
                ))
            })
    }
}
