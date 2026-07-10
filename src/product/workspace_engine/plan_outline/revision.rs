use super::*;
use crate::product::workspace_engine::review::format_review_feedback;

const WORK_ITEM_PLAN_IMPACT_CLOSURE_CONTRACT: &str = "[impact_closure_contract]
当 finding 涉及 API 契约、共享状态或测试迁移时：
1. 不得只修改 reviewer 点名的文件。
2. 必须重新检索 src/**、tests/it_web/**、tests/it_core/**、tests/it_product/**、web/src/**。
3. 必须为每个 matched file 声明 owner，或明确无需修改的原因。
4. 返修摘要必须包含 searched_scopes、matched_files、owner_mapping。";

struct OutlineRevisionNodeMutation {
    node_id: String,
    summary: String,
    completed_at: String,
    original_detail: Option<NodeDetail>,
    revised_detail: NodeDetail,
}

struct OutlineRevisionPersistenceSnapshot {
    original_status: WorkspaceSessionStatus,
    original_artifact_versions: Vec<ArtifactVersion>,
    revised_artifact_versions: Vec<ArtifactVersion>,
    artifact_versions_changed: bool,
    original_timeline_nodes: Vec<TimelineNode>,
    revised_timeline_nodes: Vec<TimelineNode>,
    node: Option<OutlineRevisionNodeMutation>,
}

#[derive(Default)]
struct OutlineRevisionPersistedSteps {
    status: bool,
    artifact_versions: bool,
    timeline: bool,
    node_detail: bool,
}

impl OutlineRevisionPersistenceSnapshot {
    fn rollback(
        &self,
        lifecycle: &LifecycleStore,
        session_id: &str,
        steps: &OutlineRevisionPersistedSteps,
        primary_error: String,
    ) -> String {
        let mut rollback_errors = Vec::new();
        if steps.node_detail
            && let Some(node) = &self.node
        {
            let result = match &node.original_detail {
                Some(detail) => lifecycle.save_node_detail(session_id, &node.node_id, detail),
                None => lifecycle.delete_node_detail(session_id, &node.node_id),
            };
            if let Err(error) = result {
                rollback_errors.push(format!(
                    "restore outline revision node detail failed: {error}"
                ));
            }
        }
        if steps.timeline
            && let Err(error) =
                lifecycle.save_timeline_nodes(session_id, &self.original_timeline_nodes)
        {
            rollback_errors.push(format!("restore outline revision timeline failed: {error}"));
        }
        if steps.artifact_versions
            && let Err(error) =
                lifecycle.save_artifact_versions(session_id, &self.original_artifact_versions)
        {
            rollback_errors.push(format!(
                "restore outline revision artifact versions failed: {error}"
            ));
        }
        if steps.status
            && let Err(error) =
                lifecycle.update_workspace_session_status(session_id, self.original_status.clone())
        {
            rollback_errors.push(format!(
                "restore outline revision workspace session status failed: {error}"
            ));
        }
        combine_outline_revision_rollback_errors(primary_error, rollback_errors)
    }
}

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

    pub(crate) fn review_decision_outline_revision_persistence_policy(
        &self,
    ) -> OutlineRevisionPersistencePolicy {
        match self
            .latest_review_verdict
            .as_ref()
            .and_then(|verdict| verdict.work_item_plan_review.as_ref())
            .map(|review| &review.review_scope)
        {
            Some(WorkItemPlanReviewScope::Item | WorkItemPlanReviewScope::Batch) => {
                OutlineRevisionPersistencePolicy::RequireActiveRound
            }
            Some(WorkItemPlanReviewScope::Outline) | None => {
                OutlineRevisionPersistencePolicy::AllowMissingInitialRound
            }
        }
    }

    fn outline_revision_persistence_snapshot(
        &self,
        lifecycle: &LifecycleStore,
        source: WorkItemPlanOutlineRevisionSource,
    ) -> Result<OutlineRevisionPersistenceSnapshot, String> {
        let original_status = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| {
                format!("load workspace session status before outline revision failed: {error}")
            })?
            .status;
        let original_artifact_versions = self.artifact_versions.clone();
        let mut revised_artifact_versions = original_artifact_versions.clone();
        let artifact_versions_changed = revised_artifact_versions
            .last_mut()
            .map(|version| {
                version.is_current = false;
            })
            .is_some();
        let original_timeline_nodes = self.timeline_nodes.clone();
        let mut revised_timeline_nodes = original_timeline_nodes.clone();
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
        }
        .to_string();
        let node = if let Some(node_id) = self.active_node_id.clone() {
            let completed_at = chrono::Utc::now().to_rfc3339();
            let original_node = original_timeline_nodes
                .iter()
                .find(|node| node.node_id == node_id)
                .cloned()
                .ok_or_else(|| format!("timeline node not found: {node_id}"))?;
            let revised_node = revised_timeline_nodes
                .iter_mut()
                .find(|node| node.node_id == node_id)
                .ok_or_else(|| format!("timeline node not found: {node_id}"))?;
            revised_node.status = TimelineNodeStatus::Completed;
            revised_node.summary = Some(summary.clone());
            revised_node.completed_at = Some(completed_at.clone());
            let original_detail =
                match lifecycle.load_node_detail(&self.session.session_id, &node_id) {
                    Ok(detail) => Some(detail),
                    Err(ProductStoreError::NotFound { .. }) => None,
                    Err(error) => {
                        return Err(format!("load outline revision node detail failed: {error}"));
                    }
                };
            let mut revised_detail = original_detail
                .clone()
                .unwrap_or_else(|| self.empty_node_detail_for(&original_node));
            revised_detail.status = TimelineNodeStatus::Completed;
            revised_detail.ended_at = Some(completed_at.clone());
            Some(OutlineRevisionNodeMutation {
                node_id,
                summary,
                completed_at,
                original_detail,
                revised_detail,
            })
        } else {
            None
        };

        Ok(OutlineRevisionPersistenceSnapshot {
            original_status,
            original_artifact_versions,
            revised_artifact_versions,
            artifact_versions_changed,
            original_timeline_nodes,
            revised_timeline_nodes,
            node,
        })
    }

    pub(crate) async fn prepare_work_item_plan_outline_revision(
        &mut self,
        feedback: Option<String>,
        source: WorkItemPlanOutlineRevisionSource,
        policy: OutlineRevisionPersistencePolicy,
    ) -> Result<Option<String>, String> {
        let outline_feedback = self.work_item_plan_outline_revision_feedback(feedback.as_deref());
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let snapshot = self.outline_revision_persistence_snapshot(&lifecycle, source)?;
        let mut plan_mutation = self.prepare_work_item_plan_outline_revising(policy)?;
        let mut steps = OutlineRevisionPersistedSteps::default();
        let persistence_result = (|| -> Result<(), String> {
            lifecycle
                .update_workspace_session_status(
                    &self.session.session_id,
                    WorkspaceSessionStatus::Open,
                )
                .map_err(|error| {
                    format!(
                        "update workspace session status before outline revision failed: {error}"
                    )
                })?;
            steps.status = true;
            if snapshot.artifact_versions_changed {
                lifecycle
                    .save_artifact_versions(
                        &self.session.session_id,
                        &snapshot.revised_artifact_versions,
                    )
                    .map_err(|error| {
                        format!("save outline revision artifact versions failed: {error}")
                    })?;
                steps.artifact_versions = true;
            }
            if let Some(node) = &snapshot.node {
                lifecycle
                    .save_timeline_nodes(&self.session.session_id, &snapshot.revised_timeline_nodes)
                    .map_err(|error| format!("save outline revision timeline failed: {error}"))?;
                steps.timeline = true;
                lifecycle
                    .save_node_detail(
                        &self.session.session_id,
                        &node.node_id,
                        &node.revised_detail,
                    )
                    .map_err(|error| {
                        format!("save outline revision node detail failed: {error}")
                    })?;
                steps.node_detail = true;
            }
            if let Some(mutation) = plan_mutation.take() {
                mutation.persist()?;
            }
            Ok(())
        })();
        if let Err(error) = persistence_result {
            return Err(snapshot.rollback(&lifecycle, &self.session.session_id, &steps, error));
        }

        self.pending_revision_context = feedback;
        self.artifact_versions = snapshot.revised_artifact_versions;
        self.timeline_nodes = snapshot.revised_timeline_nodes;
        self.session.stage = WorkspaceStage::Running;
        self.work_item_plan_author_retry_count = 0;
        self.work_item_plan_revision_retry_count = 0;
        if let Some(node) = snapshot.node {
            let _ = self
                .event_tx
                .send(EngineEvent::TimelineNodeUpdated {
                    node_id: node.node_id,
                    status: TimelineNodeStatus::Completed,
                    summary: Some(node.summary),
                    completed_at: Some(node.completed_at),
                })
                .await;
        }
        let _ = self
            .event_tx
            .send(EngineEvent::StageChange {
                stage: WorkspaceStage::Running.as_str().to_string(),
            })
            .await;
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
        let is_initial_outline_confirm =
            self.active_node_type() == Some(TimelineNodeType::WorkItemPlanOutlineConfirm);
        if is_initial_outline_confirm || self.current_artifact_is_work_item_plan_outline_candidate()
        {
            let policy = if is_initial_outline_confirm {
                OutlineRevisionPersistencePolicy::AllowMissingInitialRound
            } else {
                OutlineRevisionPersistencePolicy::RequireActiveRound
            };
            let outline_feedback = self
                .prepare_work_item_plan_outline_revision(
                    feedback,
                    WorkItemPlanOutlineRevisionSource::AuthorConfirm,
                    policy,
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
