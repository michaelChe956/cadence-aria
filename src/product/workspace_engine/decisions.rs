use super::review::format_review_feedback;
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

fn human_confirm_payload_source(payload: Option<&serde_json::Value>) -> Option<&str> {
    payload?
        .get("source")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|source| !source.is_empty())
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
                // 兼容路由：provisional 快照落盘后按创建时意图分流（spec-design-dialog-revision T3）。
                match self.session.reviewer_enabled_at_start {
                    Some(false) => {
                        self.finalize_current_artifact("人工确认定稿").await?;
                        Ok(AuthorDecisionOutcome::Finalized)
                    }
                    Some(true) => {
                        self.ensure_reviewer_available_for_review_request()?;
                        self.complete_active_node(Some("已确认，进入 Review".to_string()))
                            .await;
                        Ok(self.start_review_and_outcome().await)
                    }
                    None => {
                        // 旧记录（未落盘 provisional）：保持既有有效态判定。
                        let review_enabled = self.session.review_rounds > 0
                            && self.session.reviewer_provider.is_some();
                        if review_enabled {
                            self.complete_active_node(Some("已进入 Review".to_string()))
                                .await;
                            Ok(self.start_review_and_outcome().await)
                        } else if matches!(
                            self.session.workspace_type,
                            WorkspaceType::Story | WorkspaceType::Design
                        ) {
                            // Minor-4（T6）：HumanConfirm 已从 Story/Design 退役，旧记录（None）
                            // 无 reviewer 时按 Some(false) 同语义直接定稿，避免运行中重新落入退役阶段。
                            self.finalize_current_artifact("人工确认定稿").await?;
                            Ok(AuthorDecisionOutcome::Finalized)
                        } else {
                            self.complete_active_node(Some("已进入 Review".to_string()))
                                .await;
                            self.enter_human_confirm(Some(
                                "未启用交叉审核，等待人工确认".to_string(),
                            ))
                            .await;
                            Ok(AuthorDecisionOutcome::HumanConfirm)
                        }
                    }
                }
            }
            AuthorDecision::Reject => {
                // Story/Design 不再推倒重来：返回引导错误，改用反馈修订表达重写意图。
                // WorkItemPlan outline 的 Reject 分流在 active_node_type 检查前已返回。
                Err(
                    "已移除推倒重来：请通过反馈修订表达重写意图（spec-design-dialog-revision）"
                        .to_string(),
                )
            }
            AuthorDecision::Revise { feedback } => {
                let trimmed = feedback.trim().to_string();
                if trimmed.is_empty() {
                    return Err("revise feedback must not be empty".to_string());
                }
                self.complete_active_node(Some("用户提交反馈，进入修订".to_string()))
                    .await;
                self.pending_revision_context = Some(trimmed.clone());
                // I-1（spec-design-dialog-revision T5）：post-review 新反馈提交时清空 review verdict，
                // 使 T4 分流谓词（pending.is_some() && verdict.is_none()）成立，否则会错走 reviewer 返修 prompt。
                self.latest_review_verdict = None;
                self.transition_stage(WorkspaceStage::Revision).await;
                let revision_node_id = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::Revision,
                        agent: Some(self.session.author_provider.clone()),
                        stage: WorkspaceStage::Revision,
                        round: None,
                        title: "反馈修订".to_string(),
                        summary: Some(trimmed.clone()),
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
                // T7 fix1（Finding-A）：反馈全文落盘到 Revision 节点 detail（既有
                // NodeDetail.revision_feedback 字段）。重连后 new_persistent 重建 engine 会
                // 丢失内存态 pending_revision_context（该字段不在 WorkspaceSessionRecord），
                // retry 臂从该字段重建，retried run 才能走 author 反馈 prompt 分支。
                // （节点 summary 不可用：断线时 append_aborted_by_disconnect 会覆写为
                // "连接断开，运行已中止"。）
                let _ = self
                    .update_node_detail(&revision_node_id, |detail| {
                        detail.revision_feedback = Some(trimmed.clone());
                    })
                    .await;
                Ok(AuthorDecisionOutcome::StartRevision { feedback: trimmed })
            }
            AuthorDecision::AcceptWithReview => {
                self.ensure_reviewer_available_for_review_request()?;
                self.complete_active_node(Some("已确认，进入 Review".to_string()))
                    .await;
                Ok(self.start_review_and_outcome().await)
            }
            AuthorDecision::AcceptFinalize => {
                self.finalize_current_artifact("人工确认定稿").await?;
                Ok(AuthorDecisionOutcome::Finalized)
            }
        }
    }

    /// spec-design-dialog-revision T5/M-1：author 反馈修订分流谓词（pending 存在且无 review verdict）。
    /// prompts/revision.rs 与 provider_drive.rs 两处共用，避免重复实现漂移。
    pub(crate) fn is_author_feedback_revision(&self) -> bool {
        self.pending_revision_context.is_some() && self.latest_review_verdict.is_none()
    }

    /// AcceptWithReview 的 reviewer 就绪检查：判定依据是落盘的 reviewer_enabled_at_start，
    /// 不可用 reviewer_provider.is_none()（from_record 恒 Some + fallback author，重连后失真）。
    fn ensure_reviewer_available_for_review_request(&mut self) -> Result<(), String> {
        let review_disabled_at_start = self.session.reviewer_enabled_at_start == Some(false);
        let review_active =
            self.session.review_rounds > 0 && self.session.reviewer_provider.is_some();
        if review_active {
            return Ok(());
        }
        if review_disabled_at_start {
            if let Some(provisional) = self.session.provisional_reviewer_provider.clone() {
                self.session.reviewer_provider = Some(provisional);
                self.session.review_rounds = 1;
                return Ok(());
            }
            return Err(
                "创建时未启用 review 且未保留 reviewer 选择：请确认定稿，或重新开始并启用 review"
                    .to_string(),
            );
        }
        Err("当前会话无可用 reviewer：请确认定稿，或重新开始并启用 review".to_string())
    }

    /// 兼容旧测试的评审启动：未启用 review 时进入人工确认；已启用时复用当前
    /// `start_review` 的 CrossReview/Fake 快速路径。
    #[cfg(test)]
    pub(crate) async fn start_review_or_skip(&mut self) {
        if self.session.review_rounds == 0 || self.session.reviewer_provider.is_none() {
            self.enter_human_confirm(Some("未启用交叉审核，等待人工确认".to_string()))
                .await;
            return;
        }
        self.start_review().await;
    }

    /// 只进 CrossReview 不再跳 HumanConfirm 的评审启动（跳过路径已由 AcceptFinalize 显式覆盖）。
    /// Fake provider 快速路径保留：标记 Skipped 并进入 HumanConfirm。
    pub(crate) async fn start_review(&mut self) {
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

    /// start_review 后按实际 stage 判定 outcome：Fake 快速路径会直接进入 HumanConfirm，
    /// 此时必须返回 HumanConfirm（避免 handler 向已处 HumanConfirm 的会话 spawn ReviewOnly run）。
    /// Accept Some(true)/Accept None 有效态/AcceptWithReview 三处共用，保持单点判定。
    async fn start_review_and_outcome(&mut self) -> AuthorDecisionOutcome {
        self.start_review().await;
        if self.session.stage == WorkspaceStage::CrossReview {
            AuthorDecisionOutcome::StartReview
        } else {
            AuthorDecisionOutcome::HumanConfirm
        }
    }

    /// 定稿当前产物：标记人工确认、落库 Confirmed、进入 Completed 阶段并建 Completed 节点。
    /// 由 HumanConfirm::Confirm 分支（handle_confirm）与 AuthorDecision::AcceptFinalize 共用。
    pub(crate) async fn finalize_current_artifact(&mut self, summary: &str) -> Result<(), String> {
        self.validate_confirm_aggregate_spec_gate()?;
        self.complete_active_node(Some(summary.to_string())).await;
        self.mark_latest_artifact_confirmed(Some("human".to_string()));
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::Confirmed,
            );
            let _ = match self.session.workspace_type {
                WorkspaceType::Story | WorkspaceType::Design => store
                    .update_spec_confirmation_status(
                        &self.session.project_id,
                        &self.session.issue_id,
                        &self.session.entity_id,
                        LifecycleConfirmationStatus::Confirmed,
                    )
                    .map(|_| ()),
                WorkspaceType::WorkItem => store
                    .update_work_item_plan_status(
                        &self.session.project_id,
                        &self.session.issue_id,
                        &self.session.entity_id,
                        WorkItemPlanStatus::Confirmed,
                    )
                    .map(|_| ()),
                WorkspaceType::WorkItemPlan => Ok(()),
            };
        }
        self.transition_stage(WorkspaceStage::Completed).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::Completed,
                agent: None,
                stage: WorkspaceStage::Completed,
                round: None,
                title: "流程完成".to_string(),
                summary: Some(summary.to_string()),
                status: TimelineNodeStatus::Completed,
            })
            .await;
        Ok(())
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
                let feedback = self
                    .prepare_work_item_plan_outline_revision(
                        None,
                        WorkItemPlanOutlineRevisionSource::AuthorConfirm,
                        OutlineRevisionPersistencePolicy::AllowMissingInitialRound,
                    )
                    .await?;
                Ok(AuthorDecisionOutcome::StartWorkItemPlanOutlineRevision { feedback })
            }
            _ => Err("WorkItemPlan 不支持该 author decision 变体".to_string()),
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
                                .map(format_review_feedback);
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
                                .map(format_review_feedback);
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
                        .prepare_work_item_plan_outline_revision(
                            normalized_context,
                            WorkItemPlanOutlineRevisionSource::ReviewDecision,
                            self.review_decision_outline_revision_persistence_policy(),
                        )
                        .await?;
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
                let payload_source =
                    human_confirm_payload_source(payload.as_ref()).map(str::to_owned);
                let context = human_confirm_payload_description(payload);
                let review_has_no_trusted_findings =
                    self.latest_review_verdict.as_ref().is_some_and(|verdict| {
                        verdict.review_gate == ReviewGate::UserTriageRequired
                            && verdict.findings.is_empty()
                    });
                if review_has_no_trusted_findings
                    && (context.is_none() || payload_source.as_deref() != Some("human"))
                {
                    return Err(
                        "无可信 reviewer 修改目标时，请提供 source=human 的非空修改说明"
                            .to_string(),
                    );
                }
                if self.human_confirm_should_revise_work_item_plan_outline() {
                    let feedback = self
                        .prepare_work_item_plan_outline_revision(
                            context,
                            WorkItemPlanOutlineRevisionSource::HumanConfirm,
                            OutlineRevisionPersistencePolicy::AllowMissingInitialRound,
                        )
                        .await?;
                    return Ok(ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision {
                        feedback,
                    });
                }
                if self.latest_review_verdict.is_none() {
                    self.latest_review_verdict = Some(ReviewVerdict {
                        verdict: ReviewVerdictType::Revise,
                        comments: context.clone().unwrap_or_else(|| "人工请求修改".into()),
                        summary: "人工请求修改".to_string(),
                        findings: Vec::new(),
                        review_gate: ReviewGate::RequiresRevision,
                        work_item_plan_review: None,
                        structured_output_diagnostic: None,
                    });
                }
                self.pending_revision_context = context.clone();
                self.complete_active_node(Some("已请求修改".to_string()))
                    .await;
                self.transition_stage(WorkspaceStage::Revision).await;
                let round = (self
                    .timeline_nodes
                    .iter()
                    .filter(|node| node.node_type == TimelineNodeType::ReviewerRun)
                    .count() as u32)
                    .max(1);
                let revision_node_id = self
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
                // T7 fix1（Finding-A）：人工反馈返修同样把反馈全文落盘到节点 detail——
                // 该路径的 verdict 可能是内存合成值（不落盘、重连后丢失），重连 retry 时
                // 若不重建 pending_revision_context，build_revision_input 同样会因
                // verdict 缺失而失败。
                if let Some(feedback) = context {
                    let _ = self
                        .update_node_detail(&revision_node_id, |detail| {
                            detail.revision_feedback = Some(feedback);
                        })
                        .await;
                }
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
