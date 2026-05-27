use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{StreamChunk, StreamingProviderAdapter};
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingAttemptStatus, CodingExecutionAttempt,
    CodingExecutionStage, CodingTimelineNode, CodingTimelineNodeStatus, InternalPrReview,
    PushStatus, ReviewFinding, ReviewRequest, ReviewRequestKind, ReviewVerdict,
    TestingOverallStatus, TestingReport,
};
use crate::product::git_workspace_service::{GitWorkspaceError, GitWorkspaceService};
use crate::product::id::next_sequential_id;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{ProviderName, WorkItemStatus};
use crate::product::test_executor::{TestCommandSpec, TestExecutorError, run_all_tests};
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};
use crate::web::coding_ws_handler::CodingWsOutMessage;

#[derive(Debug, Error)]
pub enum CodingWorkspaceEngineError {
    #[error(transparent)]
    Store(#[from] ProductStoreError),
    #[error(transparent)]
    Git(#[from] GitWorkspaceError),
    #[error(transparent)]
    TestExecutor(#[from] TestExecutorError),
    #[error(transparent)]
    ProviderAdapter(#[from] ProviderAdapterError),
    #[error("coding_provider_stream_failed: {0}")]
    ProviderStream(String),
    #[error("coding_rework_limit_exceeded: {0}")]
    ReworkLimitExceeded(String),
    #[error("coding_review_request_missing: {0}")]
    MissingReviewRequest(String),
    #[error("coding_attempt_missing_worktree: {0}")]
    MissingWorktree(String),
    #[error("coding_attempt_not_ready_for_final_confirm: {0}")]
    FinalConfirmNotReady(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodingExecutionContext {
    pub work_item_markdown: Option<String>,
    pub verification_commands: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CodingWorkspaceEngine {
    store: CodingAttemptStore,
    _git_service: GitWorkspaceService,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
}

impl CodingWorkspaceEngine {
    pub fn new(
        store: CodingAttemptStore,
        git_service: GitWorkspaceService,
        event_tx: mpsc::Sender<CodingWsOutMessage>,
    ) -> Self {
        Self {
            store,
            _git_service: git_service,
            event_tx,
        }
    }

    pub async fn start_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Running,
        )?;
        let attempt = self.store.update_attempt_stage(
            project_id,
            issue_id,
            attempt_id,
            CodingExecutionStage::WorktreePrepare,
        )?;
        let node = self.create_running_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingStageChange {
                stage: CodingExecutionStage::WorktreePrepare,
            })
            .await;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node })
            .await;
        Ok(attempt)
    }

    pub async fn execute_worktree_prepare(
        &self,
        attempt: &CodingExecutionAttempt,
        repo_path: &Path,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let worktree_path = worktree_path_for_attempt(repo_path, attempt);
        self._git_service
            .create_branch(repo_path, &attempt.branch_name, &attempt.base_branch)
            .await?;
        self._git_service
            .create_worktree(repo_path, &attempt.branch_name, &worktree_path)
            .await?;
        let updated = self.store.update_attempt_worktree_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            worktree_path,
        )?;
        if let Some(node_id) = self.active_worktree_prepare_node_id(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )? {
            let completed_at = Utc::now().to_rfc3339();
            self.store.update_timeline_node_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &node_id,
                CodingTimelineNodeStatus::Completed,
                Some("worktree 已准备".to_string()),
                Some(completed_at.clone()),
            )?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                    node_id,
                    status: CodingTimelineNodeStatus::Completed,
                    summary: Some("worktree 已准备".to_string()),
                    completed_at: Some(completed_at),
                })
                .await;
        }
        Ok(updated)
    }

    pub async fn execute_coding(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        context: &CodingExecutionContext,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
        )?;
        let node = self.create_coding_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        let input = AdapterInput {
            provider_type: provider_type_for_name(&attempt.provider_config_snapshot.author),
            role: AdapterRole::Executor,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt: build_coding_prompt(&attempt, context),
            context_files: Vec::new(),
            output_schema: "coding_workspace_markdown".to_string(),
            timeout: 2400,
            max_retries: 0,
        };
        let mut stream = provider
            .run_streaming(&input, CancellationToken::new())
            .await?;
        while let Some(chunk) = stream.recv().await {
            match chunk {
                StreamChunk::Text(content) => {
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingStreamChunk {
                            content,
                            node_id: Some(node.id.clone()),
                        })
                        .await;
                }
                StreamChunk::Done { .. } => {
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingMessageComplete {
                            node_id: Some(node.id.clone()),
                        })
                        .await;
                    self.complete_timeline_node(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        &node.id,
                        CodingTimelineNodeStatus::Completed,
                        Some("代码编写完成".to_string()),
                    )
                    .await?;
                    return Ok(attempt);
                }
                StreamChunk::Error(message) => {
                    self.store.update_attempt_status(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        CodingAttemptStatus::Blocked,
                    )?;
                    self.complete_timeline_node(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        &node.id,
                        CodingTimelineNodeStatus::Failed,
                        Some(message.clone()),
                    )
                    .await?;
                    return Err(CodingWorkspaceEngineError::ProviderStream(message));
                }
            }
        }

        self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            CodingTimelineNodeStatus::Failed,
            Some("provider stream ended before completion".to_string()),
        )
        .await?;
        Err(CodingWorkspaceEngineError::ProviderStream(
            "provider stream ended before completion".to_string(),
        ))
    }

    pub async fn execute_testing(
        &self,
        attempt: &CodingExecutionAttempt,
        specs: &[TestCommandSpec],
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Testing,
        )?;
        let node = self.create_testing_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;
        let report = run_all_tests(&attempt.id, worktree_path, specs).await?;
        self.store.save_testing_report(&report)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::TestingReportUpdate {
                report: Box::new(report.clone()),
            })
            .await;

        let (node_status, summary) = match report.overall_status {
            TestingOverallStatus::Passed => (
                CodingTimelineNodeStatus::Completed,
                Some("测试通过".to_string()),
            ),
            TestingOverallStatus::Failed => (
                CodingTimelineNodeStatus::Failed,
                Some("测试失败".to_string()),
            ),
            TestingOverallStatus::SkippedByUserDecision => (
                CodingTimelineNodeStatus::Completed,
                Some("测试由用户决策跳过".to_string()),
            ),
            TestingOverallStatus::Blocked => (
                CodingTimelineNodeStatus::Blocked,
                Some("测试被阻塞".to_string()),
            ),
        };
        if matches!(
            report.overall_status,
            TestingOverallStatus::Failed | TestingOverallStatus::Blocked
        ) {
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )?;
        }
        let completed_at = Utc::now().to_rfc3339();
        self.store.update_timeline_node_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            node_status.clone(),
            summary.clone(),
            Some(completed_at.clone()),
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                node_id: node.id,
                status: node_status,
                summary,
                completed_at: Some(completed_at),
            })
            .await;
        Ok(report)
    }

    pub async fn execute_code_review(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
    ) -> Result<CodeReviewReport, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )?;
        let node = self.create_code_review_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        let reviewer = attempt
            .provider_config_snapshot
            .reviewer
            .as_ref()
            .unwrap_or(&attempt.provider_config_snapshot.author);
        let input = AdapterInput {
            provider_type: provider_type_for_name(reviewer),
            role: AdapterRole::Reviewer,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt: build_code_review_prompt(&attempt),
            context_files: Vec::new(),
            output_schema: "coding_workspace_code_review_json".to_string(),
            timeout: 2400,
            max_retries: 0,
        };
        let mut stream = provider
            .run_streaming(&input, CancellationToken::new())
            .await?;
        while let Some(chunk) = stream.recv().await {
            match chunk {
                StreamChunk::Text(content) => {
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingStreamChunk {
                            content,
                            node_id: Some(node.id.clone()),
                        })
                        .await;
                }
                StreamChunk::Done { full_output } => {
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingMessageComplete {
                            node_id: Some(node.id.clone()),
                        })
                        .await;
                    let report = self.build_code_review_report(&attempt, &full_output)?;
                    self.store.save_code_review_report(&report)?;
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodeReviewComplete {
                            report: Box::new(report.clone()),
                        })
                        .await;
                    let (node_status, summary) = match report.verdict {
                        ReviewVerdict::Approve => (
                            CodingTimelineNodeStatus::Completed,
                            Some("code review 通过".to_string()),
                        ),
                        ReviewVerdict::RequestChanges => (
                            CodingTimelineNodeStatus::Failed,
                            Some("code review 要求修改".to_string()),
                        ),
                        ReviewVerdict::Blocked => {
                            self.store.update_attempt_status(
                                &attempt.project_id,
                                &attempt.issue_id,
                                &attempt.id,
                                CodingAttemptStatus::Blocked,
                            )?;
                            (
                                CodingTimelineNodeStatus::Blocked,
                                Some("code review 被阻塞".to_string()),
                            )
                        }
                    };
                    self.complete_timeline_node(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        &node.id,
                        node_status,
                        summary,
                    )
                    .await?;
                    return Ok(report);
                }
                StreamChunk::Error(message) => {
                    self.store.update_attempt_status(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        CodingAttemptStatus::Blocked,
                    )?;
                    self.complete_timeline_node(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        &node.id,
                        CodingTimelineNodeStatus::Failed,
                        Some(message.clone()),
                    )
                    .await?;
                    return Err(CodingWorkspaceEngineError::ProviderStream(message));
                }
            }
        }

        self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            CodingTimelineNodeStatus::Failed,
            Some("provider stream ended before completion".to_string()),
        )
        .await?;
        Err(CodingWorkspaceEngineError::ProviderStream(
            "provider stream ended before completion".to_string(),
        ))
    }

    pub async fn execute_rework(
        &self,
        attempt: &CodingExecutionAttempt,
        evidence: &str,
        provider: &dyn StreamingProviderAdapter,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let current =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        if current.rework_count >= current.max_auto_rework {
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )?;
            return Err(CodingWorkspaceEngineError::ReworkLimitExceeded(
                attempt.id.clone(),
            ));
        }
        let attempt = self.store.increment_attempt_rework_count(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Rework,
        )?;
        let node = self.create_rework_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        let input = AdapterInput {
            provider_type: provider_type_for_name(&attempt.provider_config_snapshot.author),
            role: AdapterRole::Executor,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt: build_rework_prompt(&attempt, evidence),
            context_files: Vec::new(),
            output_schema: "coding_workspace_rework_markdown".to_string(),
            timeout: 2400,
            max_retries: 0,
        };
        let mut stream = provider
            .run_streaming(&input, CancellationToken::new())
            .await?;
        while let Some(chunk) = stream.recv().await {
            match chunk {
                StreamChunk::Text(content) => {
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingStreamChunk {
                            content,
                            node_id: Some(node.id.clone()),
                        })
                        .await;
                }
                StreamChunk::Done { .. } => {
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingMessageComplete {
                            node_id: Some(node.id.clone()),
                        })
                        .await;
                    self.complete_timeline_node(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        &node.id,
                        CodingTimelineNodeStatus::Completed,
                        Some("返工完成".to_string()),
                    )
                    .await?;
                    let updated = self.store.update_attempt_stage(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        CodingExecutionStage::Testing,
                    )?;
                    return Ok(updated);
                }
                StreamChunk::Error(message) => {
                    self.store.update_attempt_status(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        CodingAttemptStatus::Blocked,
                    )?;
                    self.complete_timeline_node(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        &node.id,
                        CodingTimelineNodeStatus::Failed,
                        Some(message.clone()),
                    )
                    .await?;
                    return Err(CodingWorkspaceEngineError::ProviderStream(message));
                }
            }
        }

        self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            CodingTimelineNodeStatus::Failed,
            Some("provider stream ended before completion".to_string()),
        )
        .await?;
        Err(CodingWorkspaceEngineError::ProviderStream(
            "provider stream ended before completion".to_string(),
        ))
    }

    pub async fn execute_internal_pr_review(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
    ) -> Result<InternalPrReview, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let review_request = self
            .store
            .list_review_requests(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .last()
            .ok_or_else(|| CodingWorkspaceEngineError::MissingReviewRequest(attempt.id.clone()))?;
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::InternalPrReview,
        )?;
        let node = self.create_internal_pr_review_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        let reviewer = attempt
            .provider_config_snapshot
            .reviewer
            .as_ref()
            .unwrap_or(&attempt.provider_config_snapshot.author);
        let input = AdapterInput {
            provider_type: provider_type_for_name(reviewer),
            role: AdapterRole::Reviewer,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt: build_internal_pr_review_prompt(&attempt, &review_request),
            context_files: Vec::new(),
            output_schema: "coding_workspace_internal_pr_review_json".to_string(),
            timeout: 2400,
            max_retries: 0,
        };
        let mut stream = provider
            .run_streaming(&input, CancellationToken::new())
            .await?;
        while let Some(chunk) = stream.recv().await {
            match chunk {
                StreamChunk::Text(content) => {
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingStreamChunk {
                            content,
                            node_id: Some(node.id.clone()),
                        })
                        .await;
                }
                StreamChunk::Done { full_output } => {
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingMessageComplete {
                            node_id: Some(node.id.clone()),
                        })
                        .await;
                    let review =
                        self.build_internal_pr_review(&attempt, &review_request, &full_output)?;
                    self.store.save_internal_pr_review(&review)?;
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::InternalPrReviewComplete {
                            review: Box::new(review.clone()),
                        })
                        .await;
                    let (node_status, summary) = match review.verdict {
                        ReviewVerdict::Approve => (
                            CodingTimelineNodeStatus::Completed,
                            Some("internal PR review 通过".to_string()),
                        ),
                        ReviewVerdict::RequestChanges => (
                            CodingTimelineNodeStatus::Failed,
                            Some("internal PR review 要求修改".to_string()),
                        ),
                        ReviewVerdict::Blocked => {
                            self.store.update_attempt_status(
                                &attempt.project_id,
                                &attempt.issue_id,
                                &attempt.id,
                                CodingAttemptStatus::Blocked,
                            )?;
                            (
                                CodingTimelineNodeStatus::Blocked,
                                Some("internal PR review 被阻塞".to_string()),
                            )
                        }
                    };
                    self.complete_timeline_node(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        &node.id,
                        node_status,
                        summary,
                    )
                    .await?;
                    if review.verdict == ReviewVerdict::Approve {
                        let confirmed_stage = self.store.update_attempt_stage(
                            &attempt.project_id,
                            &attempt.issue_id,
                            &attempt.id,
                            CodingExecutionStage::FinalConfirm,
                        )?;
                        self.store.update_attempt_status(
                            &confirmed_stage.project_id,
                            &confirmed_stage.issue_id,
                            &confirmed_stage.id,
                            CodingAttemptStatus::WaitingForHuman,
                        )?;
                        let final_node =
                            self.create_final_confirm_timeline_node(&confirmed_stage)?;
                        let _ = self
                            .event_tx
                            .send(CodingWsOutMessage::CodingTimelineNodeCreated {
                                node: final_node,
                            })
                            .await;
                    }
                    return Ok(review);
                }
                StreamChunk::Error(message) => {
                    self.store.update_attempt_status(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        CodingAttemptStatus::Blocked,
                    )?;
                    self.complete_timeline_node(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        &node.id,
                        CodingTimelineNodeStatus::Failed,
                        Some(message.clone()),
                    )
                    .await?;
                    return Err(CodingWorkspaceEngineError::ProviderStream(message));
                }
            }
        }

        self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            CodingTimelineNodeStatus::Failed,
            Some("provider stream ended before completion".to_string()),
        )
        .await?;
        Err(CodingWorkspaceEngineError::ProviderStream(
            "provider stream ended before completion".to_string(),
        ))
    }

    pub async fn execute_review_request(
        &self,
        attempt: &CodingExecutionAttempt,
        remote: &str,
        commit_message: &str,
    ) -> Result<ReviewRequest, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )?;
        let node = self.create_review_request_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        self._git_service.git_add_all(worktree_path).await?;
        let commit = self
            ._git_service
            .git_commit(worktree_path, commit_message)
            .await?;
        let push = self
            ._git_service
            .git_push(worktree_path, remote, &attempt.branch_name)
            .await?;
        let remote_kind = self._git_service.detect_remote_kind(worktree_path).await?;
        let existing_requests =
            self.store
                .list_review_requests(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let now = Utc::now().to_rfc3339();
        let request = ReviewRequest {
            id: next_sequential_id("review_request", existing_requests.len()),
            attempt_id: attempt.id.clone(),
            kind: ReviewRequestKind::GitBranchOnly,
            remote_kind,
            remote: remote.to_string(),
            base_branch: attempt.base_branch.clone(),
            branch_name: attempt.branch_name.clone(),
            commit_sha: commit.commit_sha,
            push_status: push.status,
            external_url: None,
            manual_instructions: vec![format!(
                "基于远端 {remote}/{} 发起代码审查",
                attempt.branch_name
            )],
            created_at: now.clone(),
            updated_at: now,
        };
        self.store.save_review_request(&request)?;
        self.store.update_attempt_review_request_state(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            request.commit_sha.clone(),
            remote.to_string(),
            request.id.clone(),
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::ReviewRequestUpdate {
                review_request: Box::new(request.clone()),
            })
            .await;

        let (node_status, summary) = if request.push_status == PushStatus::Pushed {
            (
                CodingTimelineNodeStatus::Completed,
                Some("review request 已创建".to_string()),
            )
        } else {
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )?;
            (
                CodingTimelineNodeStatus::Failed,
                Some("review request 推送失败".to_string()),
            )
        };
        let completed_at = Utc::now().to_rfc3339();
        self.store.update_timeline_node_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            node_status.clone(),
            summary.clone(),
            Some(completed_at.clone()),
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                node_id: node.id,
                status: node_status,
                summary,
                completed_at: Some(completed_at),
            })
            .await;
        Ok(request)
    }

    pub async fn handle_final_confirm(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        if current.status != CodingAttemptStatus::WaitingForHuman
            || current.stage != CodingExecutionStage::FinalConfirm
        {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt_id.to_string(),
            ));
        }

        let updated = self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Completed,
        )?;
        LifecycleStore::new(self.store.paths()).update_work_item_execution_status(
            &updated.project_id,
            &updated.issue_id,
            &updated.work_item_id,
            WorkItemStatus::Completed,
        )?;
        if let Some(node_id) =
            self.active_final_confirm_node_id(project_id, issue_id, attempt_id)?
        {
            let completed_at = Utc::now().to_rfc3339();
            self.store.update_timeline_node_status(
                project_id,
                issue_id,
                attempt_id,
                &node_id,
                CodingTimelineNodeStatus::Completed,
                Some("用户已确认完成".to_string()),
                Some(completed_at.clone()),
            )?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                    node_id,
                    status: CodingTimelineNodeStatus::Completed,
                    summary: Some("用户已确认完成".to_string()),
                    completed_at: Some(completed_at),
                })
                .await;
        }
        Ok(updated)
    }

    pub async fn handle_abort(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let updated = self.store.update_attempt_status(
            project_id,
            issue_id,
            attempt_id,
            CodingAttemptStatus::Aborted,
        )?;
        if let Some(node_id) = self.active_timeline_node_id(project_id, issue_id, attempt_id)? {
            let completed_at = Utc::now().to_rfc3339();
            self.store.update_timeline_node_status(
                project_id,
                issue_id,
                attempt_id,
                &node_id,
                CodingTimelineNodeStatus::Failed,
                Some("用户已中止".to_string()),
                Some(completed_at.clone()),
            )?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                    node_id,
                    status: CodingTimelineNodeStatus::Failed,
                    summary: Some("用户已中止".to_string()),
                    completed_at: Some(completed_at),
                })
                .await;
        }
        Ok(updated)
    }

    fn create_running_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::WorktreePrepare,
            title: "准备 worktree".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Git),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_testing_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Testing,
            title: "执行测试".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Tester),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_coding_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Coding,
            title: "代码编写".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Author),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_review_request_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::ReviewRequest,
            title: "发起 review request".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Git),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_code_review_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::CodeReview,
            title: "代码审查".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Reviewer),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_rework_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Rework,
            title: format!("返工 #{}", attempt.rework_count),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Author),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_internal_pr_review_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::InternalPrReview,
            title: "内部 PR 审查".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Reviewer),
            summary: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn create_final_confirm_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingTimelineNode, ProductStoreError> {
        let existing =
            self.store
                .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::FinalConfirm,
            title: "最终确认".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::System),
            summary: Some("等待用户最终确认".to_string()),
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        self.store.save_timeline_node(node.clone())?;
        Ok(node)
    }

    fn build_code_review_report(
        &self,
        attempt: &CodingExecutionAttempt,
        full_output: &str,
    ) -> Result<CodeReviewReport, ProductStoreError> {
        let existing = self.store.list_code_review_reports(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let payload = parse_code_review_payload(full_output);
        Ok(CodeReviewReport {
            id: next_sequential_id("code_review", existing.len()),
            attempt_id: attempt.id.clone(),
            round: existing.len() as u32 + 1,
            verdict: payload.verdict,
            findings: payload.findings,
            tested_evidence_refs: payload.tested_evidence_refs,
            diff_refs: payload.diff_refs,
            summary: payload.summary,
            created_at: Utc::now().to_rfc3339(),
        })
    }

    fn build_internal_pr_review(
        &self,
        attempt: &CodingExecutionAttempt,
        review_request: &ReviewRequest,
        full_output: &str,
    ) -> Result<InternalPrReview, ProductStoreError> {
        let existing = self.store.list_internal_pr_reviews(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let payload = parse_code_review_payload(full_output);
        Ok(InternalPrReview {
            id: next_sequential_id("internal_review", existing.len()),
            attempt_id: attempt.id.clone(),
            review_request_id: review_request.id.clone(),
            verdict: payload.verdict,
            findings: payload.findings,
            tested_evidence_refs: payload.tested_evidence_refs,
            diff_refs: payload.diff_refs,
            summary: payload.summary,
            created_at: Utc::now().to_rfc3339(),
        })
    }

    fn active_worktree_prepare_node_id(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        Ok(self
            .store
            .get_timeline_nodes(project_id, issue_id, attempt_id)?
            .into_iter()
            .rev()
            .find(|node| {
                node.stage == CodingExecutionStage::WorktreePrepare
                    && node.status == CodingTimelineNodeStatus::Running
            })
            .map(|node| node.id))
    }

    fn active_timeline_node_id(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        Ok(self
            .store
            .get_timeline_nodes(project_id, issue_id, attempt_id)?
            .into_iter()
            .rev()
            .find(|node| {
                matches!(
                    node.status,
                    CodingTimelineNodeStatus::Pending
                        | CodingTimelineNodeStatus::Running
                        | CodingTimelineNodeStatus::Blocked
                )
            })
            .map(|node| node.id))
    }

    fn active_final_confirm_node_id(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        Ok(self
            .store
            .get_timeline_nodes(project_id, issue_id, attempt_id)?
            .into_iter()
            .rev()
            .find(|node| {
                node.stage == CodingExecutionStage::FinalConfirm
                    && matches!(
                        node.status,
                        CodingTimelineNodeStatus::Pending
                            | CodingTimelineNodeStatus::Running
                            | CodingTimelineNodeStatus::Blocked
                    )
            })
            .map(|node| node.id))
    }

    async fn complete_timeline_node(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        node_id: &str,
        status: CodingTimelineNodeStatus,
        summary: Option<String>,
    ) -> Result<(), ProductStoreError> {
        let completed_at = Utc::now().to_rfc3339();
        self.store.update_timeline_node_status(
            project_id,
            issue_id,
            attempt_id,
            node_id,
            status.clone(),
            summary.clone(),
            Some(completed_at.clone()),
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                node_id: node_id.to_string(),
                status,
                summary,
                completed_at: Some(completed_at),
            })
            .await;
        Ok(())
    }
}

fn worktree_path_for_attempt(repo_path: &Path, attempt: &CodingExecutionAttempt) -> PathBuf {
    repo_path
        .join(".worktrees")
        .join("aria-work-items")
        .join(&attempt.work_item_id)
        .join(format!("attempt-{}", attempt.attempt_no))
}

fn provider_type_for_name(provider: &ProviderName) -> ProviderType {
    match provider {
        ProviderName::ClaudeCode => ProviderType::ClaudeCode,
        ProviderName::Codex => ProviderType::Codex,
        ProviderName::Fake => ProviderType::Fake,
    }
}

fn build_coding_prompt(
    attempt: &CodingExecutionAttempt,
    context: &CodingExecutionContext,
) -> String {
    let mut prompt = format!(
        "Coding Workspace\n\
         你是 Coding Workspace author。请在指定 worktree 中完成真实代码修改和测试，不要只输出计划或 Story/Design/Work Item 文档。\n\
         Project: {}\n\
         Issue: {}\n\
         Work Item: {}\n\
         Attempt: {}\n\
         Branch: {}\n",
        attempt.project_id, attempt.issue_id, attempt.work_item_id, attempt.id, attempt.branch_name
    );
    if let Some(worktree_path) = attempt.worktree_path.as_ref() {
        prompt.push_str(&format!("Worktree Path: {}\n", worktree_path.display()));
    }
    if !context.verification_commands.is_empty() {
        prompt.push_str("\n验证命令:\n");
        for command in &context.verification_commands {
            prompt.push_str("- ");
            prompt.push_str(command);
            prompt.push('\n');
        }
    }
    if let Some(markdown) = context.work_item_markdown.as_deref() {
        prompt.push_str("\n已确认 Work Item:\n````markdown\n");
        prompt.push_str(markdown.trim());
        prompt.push_str("\n````\n");
    }
    prompt.push_str(
        "\n执行要求:\n\
         - 遵循仓库规则和 TDD 流程。\n\
         - 优先按已确认 Work Item 的文件落点、范围和验证命令执行。\n\
         - 完成后报告修改文件、测试命令和结果。\n",
    );
    prompt
}

fn build_code_review_prompt(attempt: &CodingExecutionAttempt) -> String {
    format!(
        "Coding Workspace Code Review\nProject: {}\nIssue: {}\nWork Item: {}\nAttempt: {}\nBranch: {}\nReturn JSON with verdict, summary, findings.\n",
        attempt.project_id, attempt.issue_id, attempt.work_item_id, attempt.id, attempt.branch_name
    )
}

fn build_rework_prompt(attempt: &CodingExecutionAttempt, evidence: &str) -> String {
    format!(
        "Coding Workspace Rework\nProject: {}\nIssue: {}\nWork Item: {}\nAttempt: {}\nBranch: {}\nRework Count: {}\nEvidence:\n{}\n",
        attempt.project_id,
        attempt.issue_id,
        attempt.work_item_id,
        attempt.id,
        attempt.branch_name,
        attempt.rework_count,
        evidence
    )
}

fn build_internal_pr_review_prompt(
    attempt: &CodingExecutionAttempt,
    review_request: &ReviewRequest,
) -> String {
    format!(
        "Coding Workspace Internal PR Review\nProject: {}\nIssue: {}\nWork Item: {}\nAttempt: {}\nBranch: {}\nReview Request: {}\nCommit: {}\nReturn JSON with verdict, summary, findings.\n",
        attempt.project_id,
        attempt.issue_id,
        attempt.work_item_id,
        attempt.id,
        attempt.branch_name,
        review_request.id,
        review_request.commit_sha
    )
}

#[derive(Debug, Deserialize)]
struct CodeReviewProviderPayload {
    verdict: ReviewVerdict,
    summary: String,
    #[serde(default)]
    findings: Vec<ReviewFinding>,
    #[serde(default)]
    tested_evidence_refs: Vec<String>,
    #[serde(default)]
    diff_refs: Vec<String>,
}

fn parse_code_review_payload(full_output: &str) -> CodeReviewProviderPayload {
    serde_json::from_str(full_output).unwrap_or_else(|_| CodeReviewProviderPayload {
        verdict: ReviewVerdict::Approve,
        summary: non_empty_trimmed(full_output).unwrap_or_else(|| "code review 通过".to_string()),
        findings: Vec::new(),
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
    })
}

fn non_empty_trimmed(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
