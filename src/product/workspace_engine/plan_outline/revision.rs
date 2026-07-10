use super::*;
use crate::product::workspace_engine::review::format_review_feedback;

const WORK_ITEM_PLAN_IMPACT_CLOSURE_CONTRACT: &str = "[impact_closure_contract]
当 finding 涉及 API 契约、共享状态或测试迁移时：
1. 不得只修改 reviewer 点名的文件。
2. 必须重新检索 src/**、tests/it_web/**、tests/it_core/**、tests/it_product/**、web/src/**。
3. 必须为每个 matched file 声明 owner，或明确无需修改的原因。
4. 返修摘要必须包含 searched_scopes、matched_files、owner_mapping。";

/// 从当前 session artifact 与 lifecycle 构建 WorkItemPlan revision 的输入三元组。
///
/// - `retained`: candidate 中未标记 revert 的项，从 lifecycle 取完整记录。
/// - `redo_specs`: candidate 中标记 revert 的项（old_id + 反馈）。
/// - `request`: 从当前 Draft plan 与 session 配置组装，并注入 `feedback` 作为
///   `revision_feedback`。
pub(crate) fn build_work_item_plan_revision_input(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
    feedback: Option<&str>,
) -> Result<
    (
        Vec<LifecycleWorkItemRecord>,
        Vec<RedoSpec>,
        GenerateWorkItemsRequest,
    ),
    String,
> {
    let session = engine.session();
    let plan = lifecycle
        .get_issue_work_item_plan(&session.project_id, &session.issue_id, &session.entity_id)
        .map_err(|e| format!("load plan failed: {e}"))?;
    let candidate = match &session.artifact {
        Some(ArtifactPayload::WorkItemPlanCandidate { candidate }) => candidate,
        _ => return Err("current artifact is not a WorkItemPlanCandidate".to_string()),
    };

    let all_work_items = lifecycle
        .list_work_items(&session.project_id, &session.issue_id)
        .map_err(|e| format!("list work items failed: {e}"))?;
    let by_id: HashMap<String, LifecycleWorkItemRecord> = all_work_items
        .into_iter()
        .map(|wi| (wi.id.clone(), wi))
        .collect();

    let mut retained = Vec::new();
    let mut redo_specs = Vec::new();
    for wi in &candidate.work_items {
        if wi.meta.reverted {
            let item_feedback = match (&wi.meta.revert_feedback, feedback) {
                (Some(rev), Some(overall)) => format!("{}\n\n整体反馈: {}", rev, overall),
                (Some(rev), None) => rev.clone(),
                (None, Some(overall)) => overall.to_string(),
                (None, None) => "请重做".to_string(),
            };
            redo_specs.push(RedoSpec {
                old_id: wi.id.clone(),
                feedback: item_feedback,
            });
        } else {
            let record = by_id
                .get(&wi.id)
                .ok_or_else(|| format!("retained work item {} not found", wi.id))?;
            retained.push(record.clone());
        }
    }

    let provider_name_string = |name: &ProviderName| -> Result<String, String> {
        serde_json::to_value(name)
            .map_err(|e| format!("serialize provider name failed: {e}"))
            .and_then(|v| {
                v.as_str()
                    .map(ToString::to_string)
                    .ok_or_else(|| format!("provider name is not a string: {v}"))
            })
    };

    let request = GenerateWorkItemsRequest {
        title: plan.id.clone(),
        story_spec_ids: plan.source_story_spec_ids.clone(),
        design_spec_ids: plan.source_design_spec_ids.clone(),
        include_integration_tests: Some(plan.options.include_integration_tests),
        include_e2e_tests: Some(plan.options.include_e2e_tests),
        force_frontend_backend_split: Some(plan.options.force_frontend_backend_split),
        require_execution_plan_confirm: Some(plan.options.require_execution_plan_confirm),
        author_provider: Some(provider_name_string(&session.author_provider)?),
        reviewer_provider: session
            .reviewer_provider
            .as_ref()
            .map(provider_name_string)
            .transpose()?,
        review_rounds: Some(session.review_rounds),
        superpowers_enabled: Some(session.superpowers_enabled),
        openspec_enabled: Some(session.openspec_enabled),
        revision_feedback: feedback.map(ToString::to_string),
    };

    Ok((retained, redo_specs, request))
}

impl WorkspaceEngine {
    pub(crate) fn review_decision_restarts_work_item_plan_outline(&self) -> bool {
        self.session.workspace_type == WorkspaceType::WorkItemPlan
            && self
                .latest_review_verdict
                .as_ref()
                .and_then(|verdict| verdict.work_item_plan_review.as_ref())
                .is_some_and(|review| {
                    review.review_action == WorkItemPlanReviewAction::ReviseOutline
                        || review.verdict == WorkItemPlanReviewVerdict::PlanReopenRequired
                        || review
                            .gates
                            .contains(&WorkItemPlanReviewGate::RequiresPlanReopen)
                })
    }

    pub(crate) fn human_confirm_should_revise_work_item_plan_outline(&self) -> bool {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || !self.current_artifact_is_work_item_plan_outline_candidate()
        {
            return false;
        }

        self.timeline_nodes
            .iter()
            .rev()
            .find(|node| {
                node.status == TimelineNodeStatus::Completed
                    && matches!(
                        &node.node_type,
                        TimelineNodeType::WorkItemPlanOutlineReview
                            | TimelineNodeType::WorkItemDraftReview
                            | TimelineNodeType::WorkItemBatchReview
                    )
            })
            .is_some_and(|node| node.node_type == TimelineNodeType::WorkItemPlanOutlineReview)
    }

    pub(crate) async fn prepare_work_item_plan_outline_revision(
        &mut self,
        feedback: Option<String>,
        source: WorkItemPlanOutlineRevisionSource,
    ) -> Result<Option<String>, String> {
        let outline_feedback = self.work_item_plan_outline_revision_feedback(feedback.as_deref());
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let original_status = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| {
                format!("load workspace session status before outline revision failed: {error}")
            })?
            .status;
        lifecycle
            .update_workspace_session_status(&self.session.session_id, WorkspaceSessionStatus::Open)
            .map_err(|error| {
                format!("update workspace session status before outline revision failed: {error}")
            })?;
        if let Err(mark_error) = self.mark_work_item_plan_outline_revising_if_present() {
            return match lifecycle
                .update_workspace_session_status(&self.session.session_id, original_status.clone())
            {
                Ok(_) => Err(mark_error),
                Err(rollback_error) => Err(format!(
                    "{mark_error}; rollback workspace session status to {original_status:?} failed: {rollback_error}"
                )),
            };
        }

        self.pending_revision_context = feedback;
        self.mark_latest_artifact_rejected();
        let summary = match source {
            WorkItemPlanOutlineRevisionSource::AuthorConfirm => {
                "Author Confirm 已请求返修 WorkItemPlan Outline"
            }
            WorkItemPlanOutlineRevisionSource::ReviewDecision => {
                "Review Decision 已请求返修 WorkItemPlan Outline"
            }
            WorkItemPlanOutlineRevisionSource::HumanConfirm => {
                "Human Confirm 已请求返修 WorkItemPlan Outline"
            }
        };
        self.complete_active_node(Some(summary.to_string())).await;
        self.transition_stage(WorkspaceStage::Running).await;
        self.work_item_plan_author_retry_count = 0;
        self.work_item_plan_revision_retry_count = 0;
        Ok(outline_feedback)
    }

    /// WorkItemPlan Revision 完成：validate → replace Draft candidate → 组装 DTO →
    /// `update_artifact(WorkItemPlanCandidate)`（新 version）→ 回 AuthorConfirm。
    ///
    /// 校验逻辑与 `complete_work_item_plan_author` 保持一致：出现 errors 时进入
    /// AutoRevision/HumanConfirm，避免非法候选直接暴露给用户。
    pub async fn complete_work_item_plan_revision(
        &mut self,
        output: WorkItemSplitProviderOutput,
    ) -> Result<WorkItemPlanAuthorOutcome, String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or("lifecycle_store unavailable")?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();

        let report = WorkItemSplitValidator::validate(
            &output.plan,
            &output.work_items,
            Some(&output.repository_profile),
            &output.verification_plans,
        );
        let findings = report.findings.clone();

        if report.has_errors() {
            self.work_item_plan_revision_retry_count += 1;
            if self.work_item_plan_revision_retry_count >= 3 {
                if let Err(error) = lifecycle.replace_issue_work_item_plan_candidate(
                    &project_id,
                    &issue_id,
                    &plan_id,
                    &output,
                    findings.clone(),
                ) {
                    tracing::warn!(%error, "persist final validate findings before HumanConfirm failed");
                }
                self.complete_active_node(Some(work_item_plan_findings_summary(
                    "WorkItemPlan 返修校验失败，转人工确认",
                    &findings,
                )))
                .await;
                self.enter_human_confirm_for_work_item_plan_author_failure(&findings)
                    .await;
                return Ok(WorkItemPlanAuthorOutcome::HumanConfirm {
                    reason: "revision validate 连续 3 次失败".to_string(),
                });
            }

            lifecycle
                .replace_issue_work_item_plan_candidate(
                    &project_id,
                    &issue_id,
                    &plan_id,
                    &output,
                    findings.clone(),
                )
                .map_err(|e| format!("replace candidate failed: {e}"))?;
            self.complete_active_node(Some(work_item_plan_findings_summary(
                "WorkItemPlan 返修校验失败，准备自动返修",
                &findings,
            )))
            .await;
            return Ok(WorkItemPlanAuthorOutcome::AutoRevision { findings });
        }

        lifecycle
            .replace_issue_work_item_plan_candidate(
                &project_id,
                &issue_id,
                &plan_id,
                &output,
                findings.clone(),
            )
            .map_err(|e| format!("replace candidate failed: {e}"))?;

        let candidate =
            build_work_item_plan_candidate_dto(&lifecycle, &project_id, &issue_id, &plan_id)
                .map_err(|e| format!("build candidate dto failed: {e}"))?;
        self.update_artifact(ArtifactPayload::WorkItemPlanCandidate {
            candidate: Box::new(candidate),
        })
        .await;

        self.complete_active_node(Some("WorkItemPlan 返修 provider 输出完成".to_string()))
            .await;
        self.enter_author_confirm(Some("WorkItemPlan 候选已重做，等待确认".to_string()))
            .await;
        self.work_item_plan_revision_retry_count = 0;
        Ok(WorkItemPlanAuthorOutcome::AuthorConfirm)
    }

    /// AuthorConfirm 阶段用户主动请求 revision：进入 Revision 阶段并记录反馈。
    pub async fn request_work_item_plan_revision(
        &mut self,
        feedback: Option<String>,
    ) -> Result<ReviewDecisionOutcome, String> {
        if self.session.stage != WorkspaceStage::AuthorConfirm {
            return Err(
                "request_revision is only available during author_confirm stage".to_string(),
            );
        }
        if self.active_node_type() == Some(TimelineNodeType::WorkItemPlanOutlineConfirm)
            || self.current_artifact_is_work_item_plan_outline_candidate()
        {
            let outline_feedback = self
                .prepare_work_item_plan_outline_revision(
                    feedback,
                    WorkItemPlanOutlineRevisionSource::AuthorConfirm,
                )
                .await?;
            return Ok(ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision {
                feedback: outline_feedback,
            });
        }
        self.pending_revision_context = feedback;
        self.work_item_plan_revision_retry_count = 0;
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
        self.work_item_plan_author_retry_count = 0;
        Ok(ReviewDecisionOutcome::StartRevision)
    }

    /// 组装 review / AutoRevision 触发 WorkItemPlan revision 时使用的整体反馈文本。
    pub fn work_item_plan_revision_feedback(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(verdict) = &self.latest_review_verdict {
            if !verdict.comments.is_empty() {
                parts.push(format!("Reviewer 审核意见:\n{}", verdict.comments));
            }
            if !verdict.summary.is_empty() {
                parts.push(format!("摘要: {}", verdict.summary));
            }
            for finding in &verdict.findings {
                parts.push(format!(
                    "[{}] {}",
                    serialized_string(&finding.severity),
                    finding.message
                ));
            }
            let detailed_feedback = format_review_feedback(verdict);
            if !verdict.findings.is_empty() && !detailed_feedback.is_empty() {
                parts.push(detailed_feedback);
            }
        }
        if let Some(context) = &self.pending_revision_context {
            parts.push(format!("用户补充信息:\n{}", context));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        }
    }

    /// 组装 outline 阶段增量返修使用的反馈文本。
    ///
    /// 与 `work_item_plan_revision_feedback` 区别：该反馈会注入到同一会话
    /// 的增量 prompt 中，不再重复完整 issue/story/design 上下文。
    pub fn work_item_plan_outline_revision_feedback(
        &self,
        context: Option<&str>,
    ) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(verdict) = &self.latest_review_verdict {
            if !verdict.summary.is_empty() {
                parts.push(format!("Reviewer 摘要: {}", verdict.summary));
            }
            if !verdict.comments.is_empty() {
                parts.push(format!("Reviewer 审核意见:\n{}", verdict.comments));
            }
            for finding in &verdict.findings {
                parts.push(format!(
                    "[{}] {}",
                    serialized_string(&finding.severity),
                    finding.message
                ));
            }
            let detailed_feedback = format_review_feedback(verdict);
            if !verdict.findings.is_empty() && !detailed_feedback.is_empty() {
                parts.push(detailed_feedback);
            }
        }
        if let Some(context) = context.map(str::trim).filter(|c| !c.is_empty()) {
            parts.push(format!("用户补充信息:\n{}", context));
        }
        let requires_impact_closure = self.latest_review_verdict.as_ref().is_some_and(|verdict| {
            verdict.findings.iter().any(|finding| {
                matches!(
                    &finding.severity,
                    ReviewFindingSeverity::Blocking
                        | ReviewFindingSeverity::MustFix
                        | ReviewFindingSeverity::StrongRecommendFix
                )
            }) || verdict
                .structured_output_diagnostic
                .as_ref()
                .is_some_and(|diagnostic| !diagnostic.repair_succeeded)
        });
        if requires_impact_closure {
            parts.push(WORK_ITEM_PLAN_IMPACT_CLOSURE_CONTRACT.to_string());
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        }
    }

    pub async fn begin_work_item_plan_outline_auto_retry_run(
        &mut self,
        retry_of_node_id: String,
        retry_attempt: u32,
        retry_reason: String,
        retry_error: TimelineNodeRetryError,
    ) -> String {
        self.create_timeline_node_with_retry(
            TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemPlanOutlineRun,
                agent: Some(self.session.author_provider.clone()),
                stage: WorkspaceStage::Running,
                round: None,
                title: "WorkItemPlan Outline 生成".to_string(),
                summary: Some(format!("自动重跑 #{retry_attempt}")),
                status: TimelineNodeStatus::Active,
            },
            Some(TimelineNodeRetry {
                retry_of_node_id,
                retry_attempt,
                retry_reason,
                retry_error,
            }),
        )
        .await
    }
}
