use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptedRunRecoveryOutcome {
    Review,
    WorkItemDraftGeneration,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InterruptedRunRecoveryError {
    #[error("当前已有运行中的恢复任务")]
    AlreadyActive,
    #[error("中断任务已不可恢复")]
    NotRecoverable,
    #[error("中断任务状态已变化")]
    StateChanged,
}

impl InterruptedRunRecoveryError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::AlreadyActive => "INTERRUPTED_RUN_ALREADY_ACTIVE",
            Self::NotRecoverable => "INTERRUPTED_RUN_NOT_RECOVERABLE",
            Self::StateChanged => "INTERRUPTED_RUN_STATE_CHANGED",
        }
    }
}

impl WorkspaceEngine {
    pub fn recoverable_interrupted_run(&self) -> Option<RecoverableInterruptedRun> {
        if self.session.stage != WorkspaceStage::PrepareContext || self.active_run_id.is_some() {
            return None;
        }

        match self.session.workspace_type {
            WorkspaceType::WorkItemPlan => self.recoverable_work_item_plan_interrupted_run(),
            WorkspaceType::Story | WorkspaceType::Design | WorkspaceType::WorkItem => {
                self.recoverable_shared_review()
            }
        }
    }

    pub async fn retry_interrupted_run(
        &mut self,
        failed_node_id: &str,
    ) -> Result<InterruptedRunRecoveryOutcome, InterruptedRunRecoveryError> {
        if self.session.stage != WorkspaceStage::PrepareContext || self.active_run_id.is_some() {
            return Err(InterruptedRunRecoveryError::AlreadyActive);
        }
        let recovery = self
            .recoverable_interrupted_run()
            .ok_or(InterruptedRunRecoveryError::NotRecoverable)?;
        if recovery.failed_node_id != failed_node_id {
            return Err(InterruptedRunRecoveryError::StateChanged);
        }
        let source_node = self
            .timeline_nodes
            .iter()
            .find(|node| node.node_id == failed_node_id)
            .cloned()
            .ok_or(InterruptedRunRecoveryError::StateChanged)?;
        let retry_attempt = self
            .timeline_nodes
            .iter()
            .filter(|node| {
                node.retry
                    .as_ref()
                    .is_some_and(|retry| retry.retry_of_node_id == failed_node_id)
            })
            .count() as u32
            + 1;
        let retry = TimelineNodeRetry {
            retry_of_node_id: failed_node_id.to_string(),
            retry_attempt,
            retry_reason: "aborted_by_disconnect".to_string(),
            retry_error: TimelineNodeRetryError {
                code: "provider_run_aborted_by_disconnect".to_string(),
                message: "连接断开，运行已中止".to_string(),
            },
        };

        match recovery.operation {
            RecoverableInterruptedOperation::Review => {
                self.transition_stage(WorkspaceStage::CrossReview).await;
                self.create_timeline_node_with_retry(
                    TimelineNodeDraft {
                        node_type: source_node.node_type,
                        agent: Some(
                            self.session
                                .reviewer_provider
                                .clone()
                                .unwrap_or(ProviderName::Codex),
                        ),
                        stage: WorkspaceStage::CrossReview,
                        round: source_node.round,
                        title: source_node.title,
                        summary: Some("重试断线中止的审核".to_string()),
                        status: TimelineNodeStatus::Active,
                    },
                    Some(retry),
                )
                .await;
                Ok(InterruptedRunRecoveryOutcome::Review)
            }
            RecoverableInterruptedOperation::WorkItemDraftGeneration => {
                self.transition_stage(WorkspaceStage::Running).await;
                self.create_timeline_node_with_retry(
                    TimelineNodeDraft {
                        node_type: TimelineNodeType::WorkItemDraftRun,
                        agent: Some(self.session.author_provider.clone()),
                        stage: WorkspaceStage::Running,
                        round: source_node.round,
                        title: source_node.title,
                        summary: source_node.summary,
                        status: TimelineNodeStatus::Active,
                    },
                    Some(retry),
                )
                .await;
                Ok(InterruptedRunRecoveryOutcome::WorkItemDraftGeneration)
            }
        }
    }

    fn recoverable_work_item_plan_interrupted_run(&self) -> Option<RecoverableInterruptedRun> {
        let index = self
            .work_item_plan_store()
            .ok()?
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .ok()??;
        if index.outline_state != "confirmed" {
            return None;
        }

        if let Some(ArtifactPayload::WorkItemDraftCandidate { draft_candidate }) =
            self.session.artifact.as_ref()
            && draft_candidate.draft_record.status == WorkItemDraftStatus::Accepted
        {
            let outline_id = &draft_candidate.draft_record.outline_id;
            let draft_id = &draft_candidate.draft_record.draft_id;
            if index.active_outline_id.as_deref() == Some(outline_id)
                && index
                    .outline_to_current_draft_id
                    .get(outline_id)
                    .map(String::as_str)
                    == Some(draft_id)
                && index.draft_statuses.get(draft_id) == Some(&WorkItemDraftStatus::Accepted)
            {
                return self.recoverable_failed_review(TimelineNodeType::WorkItemDraftReview);
            }
        }

        let active_outline_id = index.active_outline_id.as_deref()?;
        if index
            .outline_to_current_draft_id
            .get(active_outline_id)
            .and_then(|draft_id| index.draft_statuses.get(draft_id))
            .is_some_and(|status| {
                matches!(
                    status,
                    WorkItemDraftStatus::Draft
                        | WorkItemDraftStatus::Accepted
                        | WorkItemDraftStatus::ValidationFailed
                )
            })
        {
            return None;
        }
        let failed_draft_index =
            self.failed_disconnect_node_index(TimelineNodeType::WorkItemDraftRun)?;
        if self
            .current_artifact_source_index()
            .is_some_and(|source_index| source_index > failed_draft_index)
        {
            return None;
        }

        Some(RecoverableInterruptedRun {
            failed_node_id: self.timeline_nodes[failed_draft_index].node_id.clone(),
            operation: RecoverableInterruptedOperation::WorkItemDraftGeneration,
            label: "重新生成中断的 Work Item Draft".to_string(),
        })
    }

    fn recoverable_shared_review(&self) -> Option<RecoverableInterruptedRun> {
        self.session.artifact.as_ref()?;
        self.recoverable_failed_review(TimelineNodeType::ReviewerRun)
    }

    fn recoverable_failed_review(
        &self,
        review_node_type: TimelineNodeType,
    ) -> Option<RecoverableInterruptedRun> {
        let failed_review_index = self.failed_disconnect_node_index(review_node_type.clone())?;
        let source_index = self.current_artifact_source_index()?;
        if source_index > failed_review_index
            || self.timeline_nodes[failed_review_index + 1..]
                .iter()
                .any(|node| {
                    node.node_type == review_node_type
                        && node.status == TimelineNodeStatus::Completed
                })
        {
            return None;
        }

        Some(RecoverableInterruptedRun {
            failed_node_id: self.timeline_nodes[failed_review_index].node_id.clone(),
            operation: RecoverableInterruptedOperation::Review,
            label: "重试中断审核".to_string(),
        })
    }

    fn failed_disconnect_node_index(&self, node_type: TimelineNodeType) -> Option<usize> {
        self.timeline_nodes
            .iter()
            .enumerate()
            .rev()
            .find(|(index, node)| {
                node.node_type == node_type
                    && node.status == TimelineNodeStatus::Failed
                    && node
                        .summary
                        .as_deref()
                        .is_some_and(|summary| summary.contains("连接断开"))
                    && self.timeline_nodes.get(*index + 1).is_some_and(|later| {
                        later.node_type == TimelineNodeType::AbortedByDisconnect
                    })
            })
            .map(|(index, _)| index)
    }

    fn current_artifact_source_index(&self) -> Option<usize> {
        let source_node_id = &self
            .artifact_versions
            .iter()
            .find(|version| version.is_current)?
            .source_node_id;
        self.timeline_nodes
            .iter()
            .position(|node| &node.node_id == source_node_id)
    }
}
