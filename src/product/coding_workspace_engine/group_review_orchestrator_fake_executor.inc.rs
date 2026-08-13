// 从 `group_review_orchestrator.rs` 拆出的测试专用 Fake executor
// （large_file_guard 1200 行红线，T11 fix round 2）。共享父模块作用域内的
// `Mutex`/`VecDeque` 等 `#[cfg(test)]` 导入。

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
