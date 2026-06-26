use super::*;

impl WorkspaceEngine {
    pub async fn begin_work_item_plan_author_run(&mut self) -> String {
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::AuthorRun,
            agent: Some(self.session.author_provider.clone()),
            stage: WorkspaceStage::Running,
            round: None,
            title: "Work Item Plan 生成".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await
    }

    pub async fn begin_work_item_plan_outline_run(&mut self) -> String {
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemPlanOutlineRun,
            agent: Some(self.session.author_provider.clone()),
            stage: WorkspaceStage::Running,
            round: None,
            title: "WorkItemPlan Outline 生成".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await
    }

    pub async fn begin_work_item_plan_outline_review_run(&mut self) -> String {
        self.transition_stage(WorkspaceStage::CrossReview).await;
        let round = self.next_review_round();
        let reviewer = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemPlanOutlineReview,
            agent: Some(reviewer),
            stage: WorkspaceStage::CrossReview,
            round: Some(round),
            title: format!("WorkItemPlan Outline Review Round {round}"),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await
    }

    pub async fn request_work_item_plan_outline_revision(
        &mut self,
        feedback: Option<String>,
    ) -> Result<(), String> {
        let active_node_type = self.active_node_type();
        let is_allowed_node = matches!(
            active_node_type,
            Some(
                TimelineNodeType::WorkItemPlanOutlineConfirm
                    | TimelineNodeType::WorkItemGenerationMode
            )
        );
        if self.session.stage != WorkspaceStage::AuthorConfirm || !is_allowed_node {
            return Err(
                "request_outline_revision requires active work_item_plan_outline_confirm or work_item_generation_mode node"
                    .to_string(),
            );
        }
        self.pending_revision_context = feedback;
        self.mark_work_item_plan_outline_revising()?;
        self.complete_active_node(Some("已返回 WorkItemPlan Outline 返修".to_string()))
            .await;
        self.transition_stage(WorkspaceStage::Running).await;
        self.begin_work_item_plan_outline_run().await;
        Ok(())
    }

    pub async fn begin_work_item_plan_auto_revision_run(&mut self, round: u32) -> String {
        self.transition_stage(WorkspaceStage::Revision).await;
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::Revision,
            agent: Some(self.session.author_provider.clone()),
            stage: WorkspaceStage::Revision,
            round: Some(round),
            title: format!("Work Item Plan 自动返修 Round {round}"),
            summary: Some("根据 Work Item Plan 校验结果自动返修".to_string()),
            status: TimelineNodeStatus::Active,
        })
        .await
    }

    pub(crate) fn current_artifact_is_work_item_plan_outline_candidate(&self) -> bool {
        matches!(
            self.session.artifact,
            Some(ArtifactPayload::WorkItemPlanOutlineCandidate { .. })
        )
    }

    pub async fn complete_work_item_plan_author(
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
            self.work_item_plan_author_retry_count += 1;
            if self.work_item_plan_author_retry_count >= 3 {
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
                    "WorkItemPlan 校验失败，转人工确认",
                    &findings,
                )))
                .await;
                self.enter_human_confirm_for_work_item_plan_author_failure(&findings)
                    .await;
                return Ok(WorkItemPlanAuthorOutcome::HumanConfirm {
                    reason: "validate 连续 3 次失败".to_string(),
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
                "WorkItemPlan 校验失败，准备自动返修",
                &findings,
            )))
            .await;
            return Ok(WorkItemPlanAuthorOutcome::AutoRevision { findings });
        }

        // 所有同步落盘与 DTO 组装完成后，再进入异步事件发送；保持锁内同步操作连续，
        // 避免在 await 点之间穿插同步 IO。
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

        self.complete_active_node(Some("WorkItemPlan provider 输出完成".to_string()))
            .await;
        self.enter_author_confirm(Some("WorkItemPlan 候选已生成，等待确认".to_string()))
            .await;

        self.work_item_plan_author_retry_count = 0;
        Ok(WorkItemPlanAuthorOutcome::AuthorConfirm)
    }

    pub async fn complete_work_item_plan_outline_author(
        &mut self,
        output: OutlineAuthorOutput,
    ) -> Result<WorkItemPlanAuthorOutcome, String> {
        let design_context_gaps = self.current_work_item_plan_design_context_gaps();
        if !output.context_blockers.is_empty() {
            let payload = ArtifactPayload::WorkItemPlanContextBlocker {
                context_blocker: Box::new(WorkItemPlanContextBlockerPayload {
                    context_blockers: work_item_plan_context_blockers_to_dto(
                        &output.context_blockers,
                    ),
                    design_context_gaps: design_context_gaps.clone(),
                    exploration_summary: "Outline author 需要补充上下文后才能继续".to_string(),
                    allowed_actions: vec!["provide_context".to_string(), "abort".to_string()],
                }),
            };
            self.update_artifact(payload).await;
            self.complete_active_node(Some(
                "WorkItemPlan Outline author 请求补充上下文".to_string(),
            ))
            .await;
            self.enter_work_item_plan_context_blocker(Some(
                "请补充 WorkItemPlan Outline 所需上下文".to_string(),
            ))
            .await;
            return Ok(WorkItemPlanAuthorOutcome::HumanConfirm {
                reason: "context_blockers".to_string(),
            });
        }

        let outline = output
            .outline
            .ok_or_else(|| "WorkItemPlan Outline output missing outline".to_string())?;
        let report = WorkItemPlanOutlineValidator::validate(&outline);
        let findings = report.findings.clone();

        if report.has_errors() {
            self.work_item_plan_author_retry_count += 1;
            self.complete_active_node(Some(work_item_plan_findings_summary(
                "WorkItemPlan Outline 校验失败",
                &findings,
            )))
            .await;

            if self.work_item_plan_author_retry_count >= 2 {
                let exploration_summary =
                    work_item_plan_outline_terminal_failure_summary(&findings);
                let payload = ArtifactPayload::WorkItemPlanContextBlocker {
                    context_blocker: Box::new(WorkItemPlanContextBlockerPayload {
                        context_blockers: Vec::new(),
                        design_context_gaps: design_context_gaps.clone(),
                        exploration_summary,
                        allowed_actions: vec!["provide_context".to_string(), "abort".to_string()],
                    }),
                };
                self.update_artifact(payload).await;
                self.enter_work_item_plan_context_blocker(Some(
                    "Outline 校验失败，请终止后重新创建 Work Item Plan".to_string(),
                ))
                .await;
                return Ok(WorkItemPlanAuthorOutcome::HumanConfirm {
                    reason: "outline_validation_failed".to_string(),
                });
            }

            return Ok(WorkItemPlanAuthorOutcome::AutoRevision { findings });
        }

        self.update_artifact(ArtifactPayload::WorkItemPlanOutlineCandidate {
            outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
                outline,
                design_context_gaps,
                validator_findings: work_item_split_findings_to_dto(&findings),
                context_blockers: Vec::new(),
                current_generation_round_id: None,
                selected_generation_mode: None,
            }),
        })
        .await;
        self.complete_active_node(Some("WorkItemPlan Outline provider 输出完成".to_string()))
            .await;
        self.enter_work_item_plan_outline_confirm(Some(
            "WorkItemPlan Outline 已生成，等待确认".to_string(),
        ))
        .await;
        self.work_item_plan_author_retry_count = 0;
        Ok(WorkItemPlanAuthorOutcome::AuthorConfirm)
    }

    pub async fn complete_work_item_plan_outline_author_output_error(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<WorkItemPlanAuthorOutcome, String> {
        let code = code.into();
        let message = message.into();
        let findings = vec![WorkItemSplitFinding {
            severity: WorkItemSplitFindingSeverity::Error,
            code,
            message: format!(
                "Provider did not return a valid WorkItemPlan Outline structured output: {message}"
            ),
            work_item_ids: Vec::new(),
        }];
        let design_context_gaps = self.current_work_item_plan_design_context_gaps();

        self.work_item_plan_author_retry_count += 1;
        self.complete_active_node(Some(work_item_plan_findings_summary(
            "WorkItemPlan Outline 结构化输出解析失败",
            &findings,
        )))
        .await;

        if self.work_item_plan_author_retry_count >= 2 {
            let payload = ArtifactPayload::WorkItemPlanContextBlocker {
                context_blocker: Box::new(WorkItemPlanContextBlockerPayload {
                    context_blockers: Vec::new(),
                    design_context_gaps,
                    exploration_summary: work_item_plan_findings_summary(
                        "Outline 自动重跑后仍无法解析 provider 输出",
                        &findings,
                    ),
                    allowed_actions: vec!["provide_context".to_string(), "abort".to_string()],
                }),
            };
            self.update_artifact(payload).await;
            self.enter_work_item_plan_context_blocker(Some(
                "Outline 结构化输出解析失败，请补充上下文或终止".to_string(),
            ))
            .await;
            return Ok(WorkItemPlanAuthorOutcome::HumanConfirm {
                reason: "outline_output_parse_failed".to_string(),
            });
        }

        Ok(WorkItemPlanAuthorOutcome::AutoRevision { findings })
    }

    pub(crate) fn current_work_item_plan_design_context_gaps(&self) -> Vec<String> {
        let Some(lifecycle) = &self.lifecycle_store else {
            return Vec::new();
        };
        let store = WorkItemPlanStore::new(lifecycle.app_paths());
        store
            .load_outline_context_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .ok()
            .flatten()
            .map(|index| index.design_context_gaps)
            .unwrap_or_default()
    }
}
