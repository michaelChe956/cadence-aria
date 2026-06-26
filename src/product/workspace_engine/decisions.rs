use super::*;

pub(crate) fn human_confirm_payload_description(
    payload: Option<serde_json::Value>,
) -> Option<String> {
    let payload = payload?;
    let description = payload.as_str().map(ToString::to_string).or_else(|| {
        payload
            .get("description")
            .and_then(|value| value.as_str())
            .map(ToString::to_string)
    })?;
    let trimmed = description.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

pub(crate) fn empty_design_context_capabilities() -> DesignContextCapabilities {
    DesignContextCapabilities {
        has_architecture: false,
        has_module_breakdown: false,
        has_tech_stack: false,
        has_test_strategy: false,
        has_key_paths: false,
    }
}

pub(crate) fn estimate_context_resolution_tokens(value: &str) -> u32 {
    ((value.chars().count() as u32).saturating_add(3) / 4).max(1)
}

pub(crate) fn format_context_blocker_resolution_markdown(resolution: &str) -> String {
    format!(
        "# WorkItemPlan 上下文补充\n\n## 用户补充\n\n{resolution}\n",
        resolution = resolution.trim()
    )
}

impl WorkspaceEngine {
    pub async fn handle_author_decision(
        &mut self,
        decision: AuthorDecision,
    ) -> Result<AuthorDecisionOutcome, String> {
        if self.session.stage != WorkspaceStage::AuthorConfirm {
            return Err(
                "author decision is only available during author_confirm stage".to_string(),
            );
        }

        if self.session.workspace_type == WorkspaceType::WorkItemPlan {
            match self.active_node_type() {
                Some(TimelineNodeType::WorkItemPlanOutlineConfirm) => {
                    return self.handle_work_item_plan_outline_decision(decision).await;
                }
                Some(TimelineNodeType::WorkItemGenerationMode) => {
                    return Err(
                        "author_decision is not valid on work_item_generation_mode node"
                            .to_string(),
                    );
                }
                Some(TimelineNodeType::WorkItemDraftConfirm) => {
                    return Err(
                        "author_decision is not valid on work_item_draft_confirm node; use work_item_draft_decision"
                            .to_string(),
                    );
                }
                Some(TimelineNodeType::WorkItemBatchConfirm) => {
                    return Err(
                        "author_decision is not valid on work_item_batch_confirm node; use work_item_batch_decision"
                            .to_string(),
                    );
                }
                Some(TimelineNodeType::WorkItemPlanCompileRecovery) => {
                    return Err(
                        "author_decision is not valid on work_item_plan_compile_recovery node; use work_item_plan_compile_recovery_action"
                            .to_string(),
                    );
                }
                _ => {}
            }
        }

        match decision {
            AuthorDecision::Accept => {
                let review_enabled =
                    self.session.review_rounds > 0 && self.session.reviewer_provider.is_some();
                self.complete_active_node(Some("已进入 Review".to_string()))
                    .await;
                self.start_review_or_skip().await;
                if review_enabled && self.session.stage == WorkspaceStage::CrossReview {
                    Ok(AuthorDecisionOutcome::StartReview)
                } else {
                    Ok(AuthorDecisionOutcome::HumanConfirm)
                }
            }
            AuthorDecision::Reject => {
                self.complete_active_node(Some("用户要求重新编写".to_string()))
                    .await;
                self.session.artifact = None;
                self.mark_latest_artifact_rejected();
                if let Some(store) = &self.lifecycle_store {
                    let _ = store.update_workspace_session_status(
                        &self.session.session_id,
                        WorkspaceSessionStatus::Open,
                    );
                }
                self.transition_stage(WorkspaceStage::PrepareContext).await;
                let _ = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::PrepareContext,
                        agent: None,
                        stage: WorkspaceStage::PrepareContext,
                        round: None,
                        title: "准备上下文".to_string(),
                        summary: Some("等待重新补充上下文".to_string()),
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
                Ok(AuthorDecisionOutcome::PrepareContext)
            }
        }
    }

    pub(crate) async fn handle_work_item_plan_outline_decision(
        &mut self,
        decision: AuthorDecision,
    ) -> Result<AuthorDecisionOutcome, String> {
        match decision {
            AuthorDecision::Accept => {
                let generation_round_id = self.save_confirmed_work_item_plan_outline_index()?;
                self.update_work_item_plan_outline_generation_metadata(
                    Some(generation_round_id.clone()),
                    None,
                )
                .await?;
                self.mark_latest_artifact_confirmed(Some("human".to_string()));
                let review_enabled =
                    self.session.review_rounds > 0 && self.session.reviewer_provider.is_some();
                let summary = format!(
                    "WorkItemPlan Outline 已确认，generation_round_id={generation_round_id}"
                );
                self.complete_active_node(Some(summary)).await;
                if review_enabled {
                    self.begin_work_item_plan_outline_review_run().await;
                    Ok(AuthorDecisionOutcome::StartReview)
                } else {
                    self.enter_work_item_generation_mode(Some(
                        "请选择 Work Item 生成模式".to_string(),
                    ))
                    .await;
                    Ok(AuthorDecisionOutcome::HumanConfirm)
                }
            }
            AuthorDecision::Reject => {
                self.mark_latest_artifact_rejected();
                self.complete_active_node(Some("用户要求重写 WorkItemPlan Outline".to_string()))
                    .await;
                self.mark_work_item_plan_outline_revising()?;
                self.transition_stage(WorkspaceStage::Running).await;
                self.begin_work_item_plan_outline_run().await;
                Ok(AuthorDecisionOutcome::HumanConfirm)
            }
        }
    }

    pub async fn handle_review_decision(
        &mut self,
        decision: String,
        extra_context: Option<String>,
    ) -> Result<ReviewDecisionOutcome, String> {
        if self.session.stage != WorkspaceStage::ReviewDecision {
            return Err(
                "review decision is only available during review_decision stage".to_string(),
            );
        }

        let round = self.active_review_round().unwrap_or(1);
        match decision.as_str() {
            "skip_optional_findings" => self.skip_work_item_plan_optional_findings().await,
            "continue" | "continue_with_context" | "apply_optional_findings" => {
                if decision == "apply_optional_findings"
                    && self.latest_work_item_plan_optional_pass_review().is_none()
                {
                    return Err(
                        "apply_optional_findings is only available for optional work item plan findings"
                            .to_string(),
                    );
                }
                if decision == "apply_optional_findings" {
                    let review = self
                        .latest_work_item_plan_optional_pass_review()
                        .cloned()
                        .ok_or_else(|| {
                            "apply_optional_findings is only available for optional work item plan findings"
                                .to_string()
                        })?;
                    match review.review_scope {
                        WorkItemPlanReviewScope::Item => {
                            let target_outline_id = review
                                .target_outline_id
                                .clone()
                                .or_else(|| {
                                    self.current_work_item_draft_candidate_payload()
                                        .ok()
                                        .map(|payload| payload.draft_record.outline_id)
                                })
                                .ok_or_else(|| {
                                    "optional item review target outline is missing".to_string()
                                })?;
                            let feedback = self
                                .latest_review_verdict
                                .as_ref()
                                .map(|verdict| verdict.comments.clone());
                            self.pending_revision_context = feedback;
                            self.complete_active_node(Some(
                                "已选择修复当前 Work Item Draft 的可选建议".to_string(),
                            ))
                            .await;
                            self.start_serial_work_item_draft_run_for(&target_outline_id)
                                .await?;
                            return Ok(ReviewDecisionOutcome::StartWorkItemDraft {
                                feedback: None,
                            });
                        }
                        WorkItemPlanReviewScope::Batch => {
                            self.pending_revision_context = self
                                .latest_review_verdict
                                .as_ref()
                                .map(|verdict| verdict.comments.clone());
                            let outcome = self.rewrite_current_work_item_batch().await?;
                            return match outcome {
                                WorkItemBatchDecisionOutcome::StartBatchRun => {
                                    Ok(ReviewDecisionOutcome::StartWorkItemBatch)
                                }
                                WorkItemBatchDecisionOutcome::StartDraftRun => {
                                    Ok(ReviewDecisionOutcome::StartWorkItemDraft { feedback: None })
                                }
                                WorkItemBatchDecisionOutcome::HumanConfirm
                                | WorkItemBatchDecisionOutcome::StartReview => {
                                    Ok(ReviewDecisionOutcome::HumanConfirm)
                                }
                            };
                        }
                        WorkItemPlanReviewScope::Outline => {}
                    }
                }
                let normalized_context = if decision == "continue_with_context" {
                    extra_context.and_then(|context| {
                        let trimmed = context.trim().to_string();
                        if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed)
                        }
                    })
                } else {
                    None
                };
                if decision == "continue_with_context" && normalized_context.is_none() {
                    return Err(
                        "continue_with_context requires non-empty extra_context".to_string()
                    );
                }
                if self.review_decision_restarts_work_item_plan_outline()
                    || self.current_artifact_is_work_item_plan_outline_candidate()
                {
                    let outline_feedback = self
                        .work_item_plan_outline_revision_feedback(normalized_context.as_deref());
                    self.pending_revision_context = normalized_context;
                    self.complete_active_node(Some("已选择返修 WorkItemPlan Outline".to_string()))
                        .await;
                    self.mark_work_item_plan_outline_revising()?;
                    self.transition_stage(WorkspaceStage::Running).await;
                    self.work_item_plan_author_retry_count = 0;
                    self.work_item_plan_revision_retry_count = 0;
                    return Ok(ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision {
                        feedback: outline_feedback,
                    });
                }
                self.pending_revision_context = normalized_context;
                self.complete_active_node(Some("已选择返修".to_string()))
                    .await;
                self.transition_stage(WorkspaceStage::Revision).await;
                let _ = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::Revision,
                        agent: Some(self.session.author_provider.clone()),
                        stage: WorkspaceStage::Revision,
                        round: Some(round),
                        title: format!("返修 Round {round}"),
                        summary: Some("根据 review 意见返修".to_string()),
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
                Ok(ReviewDecisionOutcome::StartRevision)
            }
            "human_intervene" => {
                self.complete_active_node(Some("转人工介入".to_string()))
                    .await;
                let summary = self
                    .latest_review_verdict
                    .as_ref()
                    .map(|verdict| verdict.summary.clone())
                    .or_else(|| Some("等待人工介入".to_string()));
                self.enter_human_confirm(summary).await;
                Ok(ReviewDecisionOutcome::HumanConfirm)
            }
            _ => Err(format!("unknown review decision: {decision}")),
        }
    }

    pub(crate) async fn skip_work_item_plan_optional_findings(
        &mut self,
    ) -> Result<ReviewDecisionOutcome, String> {
        let review = self
            .latest_work_item_plan_optional_pass_review()
            .cloned()
            .ok_or_else(|| {
                "skip_optional_findings is only available for optional work item plan findings"
                    .to_string()
            })?;
        match review.review_scope {
            WorkItemPlanReviewScope::Outline => {
                self.complete_active_node(Some("已选择不修复可选建议".to_string()))
                    .await;
                self.enter_work_item_generation_mode(Some(
                    "已跳过可选建议，请选择 Work Item 生成模式".to_string(),
                ))
                .await;
                Ok(ReviewDecisionOutcome::HumanConfirm)
            }
            WorkItemPlanReviewScope::Item => {
                let target_outline_id = review
                    .target_outline_id
                    .clone()
                    .or_else(|| {
                        self.current_work_item_draft_candidate_payload()
                            .ok()
                            .map(|payload| payload.draft_record.outline_id)
                    })
                    .ok_or_else(|| "optional item review target outline is missing".to_string())?;
                self.complete_active_node(Some("已选择不修复当前 Draft 可选建议".to_string()))
                    .await;
                self.continue_after_work_item_draft_review_pass(&target_outline_id)
                    .await?;
                if self.active_node_type() == Some(TimelineNodeType::WorkItemDraftRun) {
                    Ok(ReviewDecisionOutcome::StartWorkItemDraft { feedback: None })
                } else {
                    Ok(ReviewDecisionOutcome::HumanConfirm)
                }
            }
            WorkItemPlanReviewScope::Batch => {
                self.complete_active_node(Some("已选择不修复 Batch 可选建议".to_string()))
                    .await;
                self.mark_current_work_item_batch_review_done()?;
                self.enter_work_item_plan_compile().await;
                Ok(ReviewDecisionOutcome::HumanConfirm)
            }
        }
    }

    pub(crate) fn review_decision_restarts_work_item_plan_outline(&self) -> bool {
        self.session.workspace_type == WorkspaceType::WorkItemPlan
            && self
                .latest_review_verdict
                .as_ref()
                .and_then(|verdict| verdict.work_item_plan_review.as_ref())
                .is_some_and(|review| {
                    review.review_scope == WorkItemPlanReviewScope::Outline
                        && review.review_action == WorkItemPlanReviewAction::ReviseOutline
                })
    }

    pub async fn handle_human_confirm(
        &mut self,
        decision: HumanConfirmDecision,
        payload: Option<serde_json::Value>,
    ) -> Result<ReviewDecisionOutcome, String> {
        if self.session.stage != WorkspaceStage::HumanConfirm {
            return Err("human confirm is only available during human_confirm stage".to_string());
        }

        if self.session.workspace_type == WorkspaceType::WorkItemPlan
            && self.active_node_type() == Some(TimelineNodeType::WorkItemPlanContextBlocker)
        {
            return self
                .handle_work_item_plan_context_blocker_decision(decision, payload)
                .await;
        }

        match decision {
            HumanConfirmDecision::Confirm => match self.handle_confirm().await? {
                WorkspaceConfirmOutcome::None => Ok(ReviewDecisionOutcome::HumanConfirm),
                WorkspaceConfirmOutcome::WorkItemPlan { child_sessions } => {
                    Ok(ReviewDecisionOutcome::ConfirmedWithChildSessions { child_sessions })
                }
            },
            HumanConfirmDecision::RequestChange => {
                let context = human_confirm_payload_description(payload);
                if self.latest_review_verdict.is_none() {
                    self.latest_review_verdict = Some(ReviewVerdict {
                        verdict: ReviewVerdictType::Revise,
                        comments: context
                            .clone()
                            .unwrap_or_else(|| "人工请求修改".to_string()),
                        summary: "人工请求修改".to_string(),
                        findings: Vec::new(),
                        review_gate: ReviewGate::RequiresRevision,
                        work_item_plan_review: None,
                    });
                }
                self.pending_revision_context = context;
                self.complete_active_node(Some("已请求修改".to_string()))
                    .await;
                self.transition_stage(WorkspaceStage::Revision).await;
                let round = (self
                    .timeline_nodes
                    .iter()
                    .filter(|node| node.node_type == TimelineNodeType::ReviewerRun)
                    .count() as u32)
                    .max(1);
                let _ = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::Revision,
                        agent: Some(self.session.author_provider.clone()),
                        stage: WorkspaceStage::Revision,
                        round: Some(round),
                        title: format!("返修 Round {round}"),
                        summary: Some("根据人工反馈返修".to_string()),
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
                Ok(ReviewDecisionOutcome::StartRevision)
            }
            HumanConfirmDecision::Terminate => {
                self.complete_active_node(Some("已终止".to_string())).await;
                if let Some(store) = &self.lifecycle_store {
                    let _ = store.update_workspace_session_status(
                        &self.session.session_id,
                        WorkspaceSessionStatus::Terminated,
                    );
                }
                self.transition_stage(WorkspaceStage::Completed).await;
                let _ = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::Completed,
                        agent: None,
                        stage: WorkspaceStage::Completed,
                        round: None,
                        title: "流程终止".to_string(),
                        summary: Some("已终止".to_string()),
                        status: TimelineNodeStatus::Completed,
                    })
                    .await;
                Ok(ReviewDecisionOutcome::HumanConfirm)
            }
        }
    }

    pub(crate) async fn handle_work_item_plan_context_blocker_decision(
        &mut self,
        decision: HumanConfirmDecision,
        payload: Option<serde_json::Value>,
    ) -> Result<ReviewDecisionOutcome, String> {
        match decision {
            HumanConfirmDecision::Confirm => Err(
                "work item plan context blocker cannot be confirmed; provide context or terminate"
                    .to_string(),
            ),
            HumanConfirmDecision::RequestChange => {
                let resolution = human_confirm_payload_description(payload).ok_or_else(|| {
                    "work item plan context blocker requires non-empty context".to_string()
                })?;
                self.append_work_item_plan_context_blocker_resolution(resolution)
                    .await?;
                if let Some(store) = &self.lifecycle_store {
                    let _ = store.update_workspace_session_status(
                        &self.session.session_id,
                        WorkspaceSessionStatus::Open,
                    );
                }
                self.transition_stage(WorkspaceStage::Running).await;
                Ok(ReviewDecisionOutcome::StartWorkItemPlanOutline)
            }
            HumanConfirmDecision::Terminate => {
                self.complete_active_node(Some("已终止 WorkItemPlan Outline 生成".to_string()))
                    .await;
                if let Some(store) = &self.lifecycle_store {
                    let _ = store.update_workspace_session_status(
                        &self.session.session_id,
                        WorkspaceSessionStatus::Terminated,
                    );
                }
                self.transition_stage(WorkspaceStage::Completed).await;
                let _ = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::Completed,
                        agent: None,
                        stage: WorkspaceStage::Completed,
                        round: None,
                        title: "WorkItemPlan Outline 生成已终止".to_string(),
                        summary: Some("用户终止上下文补充流程".to_string()),
                        status: TimelineNodeStatus::Completed,
                    })
                    .await;
                Ok(ReviewDecisionOutcome::HumanConfirm)
            }
        }
    }

    pub(crate) async fn append_work_item_plan_context_blocker_resolution(
        &mut self,
        resolution: String,
    ) -> Result<(), String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let blocker_node_id = self
            .active_node_id
            .clone()
            .ok_or_else(|| "context blocker node unavailable".to_string())?;
        let now = chrono::Utc::now().to_rfc3339();

        self.complete_active_node(Some("已记录上下文补充".to_string()))
            .await;
        let resolution_node = self
            .append_completed_timeline_event(
                TimelineNodeType::ContextNote,
                WorkspaceStage::HumanConfirm,
                "WorkItemPlan 上下文补充".to_string(),
                Some(resolution.clone()),
                TimelineNodeStatus::Completed,
                true,
            )
            .await;
        let resolution_node_id = resolution_node.node_id.clone();
        let artifact_ref = self
            .update_artifact(ArtifactPayload::Markdown {
                markdown: format_context_blocker_resolution_markdown(&resolution),
                diff: None,
            })
            .await;

        let store = WorkItemPlanStore::new(lifecycle.app_paths());
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let mut index = store
            .load_outline_context_index(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load outline context index failed: {error}"))?
            .unwrap_or_else(|| OutlineContextIndex {
                project_id: project_id.clone(),
                issue_id: issue_id.clone(),
                plan_id: plan_id.clone(),
                generation_round_id: "outline_stage".to_string(),
                blocker_resolutions: Vec::new(),
                design_context_gaps: Vec::new(),
                design_context_capabilities: empty_design_context_capabilities(),
                updated_at: now.clone(),
            });

        index
            .blocker_resolutions
            .push(OutlineContextBlockerResolution {
                blocker_node_id: blocker_node_id.clone(),
                resolution_node_id: resolution_node_id.clone(),
                resolution_artifact_ref: format!(
                    "{}/v{}",
                    artifact_ref.artifact_id, artifact_ref.version
                ),
                estimated_tokens: estimate_context_resolution_tokens(&resolution),
                created_at: now.clone(),
                summary: Some(resolution.clone()),
                merged_count: None,
            });
        index.updated_at = now;
        store
            .save_outline_context_index(&index)
            .map_err(|error| format!("save outline context index failed: {error}"))?;
        Ok(())
    }

    pub(crate) async fn enter_human_confirm_for_work_item_plan_author_failure(
        &mut self,
        _findings: &[WorkItemSplitFinding],
    ) {
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::HumanConfirm,
                agent: None,
                stage: WorkspaceStage::HumanConfirm,
                round: None,
                title: "WorkItemPlan validate 连续失败".to_string(),
                summary: Some("author 多次重生仍 validate 失败，需人工介入".to_string()),
                status: TimelineNodeStatus::Active,
            })
            .await;
    }

    pub(crate) async fn enter_work_item_plan_context_blocker(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemPlanContextBlocker,
                agent: None,
                stage: WorkspaceStage::HumanConfirm,
                round: None,
                title: "WorkItemPlan 上下文补充".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
    }

    pub(crate) async fn start_review_or_skip(&mut self) {
        if self.session.review_rounds == 0 || self.session.reviewer_provider.is_none() {
            self.enter_human_confirm(Some("未启用交叉审核，等待人工确认".to_string()))
                .await;
            return;
        }

        self.transition_stage(WorkspaceStage::CrossReview).await;
        let round = self.next_review_round();
        let reviewer = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        let review_node_id = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::ReviewerRun,
                agent: Some(reviewer.clone()),
                stage: WorkspaceStage::CrossReview,
                round: Some(round),
                title: format!("Review Round {round}"),
                summary: None,
                status: TimelineNodeStatus::Active,
            })
            .await;

        if reviewer == ProviderName::Fake {
            self.update_timeline_node(
                &review_node_id,
                TimelineNodeStatus::Skipped,
                Some("未执行真实 review（Fake 快速路径）".to_string()),
            )
            .await;
            self.mark_latest_artifact_reviewed(Some(ProviderName::Fake), None);
            self.enter_human_confirm(Some("等待人工确认".to_string()))
                .await;
        }
    }

    pub(crate) async fn enter_author_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::AuthorConfirm,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "Author 结果确认".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
    }

    pub(crate) async fn enter_work_item_plan_outline_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemPlanOutlineConfirm,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "WorkItemPlan Outline 确认".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
    }

    pub(crate) async fn enter_work_item_generation_mode(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemGenerationMode,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "Work Item 生成模式选择".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
    }

    pub(crate) async fn enter_work_item_draft_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemDraftConfirm,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "Work Item Draft 确认".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
    }

    pub(crate) async fn enter_work_item_batch_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::AuthorConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemBatchConfirm,
                agent: None,
                stage: WorkspaceStage::AuthorConfirm,
                round: None,
                title: "Work Item Batch 确认".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
    }

    pub(crate) async fn enter_human_confirm(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::HumanConfirm,
                agent: None,
                stage: WorkspaceStage::HumanConfirm,
                round: None,
                title: "人工确认".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
    }
}
