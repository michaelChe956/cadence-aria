use super::*;

impl WorkspaceEngine {
    pub(crate) async fn complete_review(&mut self, output: String) {
        let node_id = self
            .active_node_id
            .clone()
            .unwrap_or_else(|| "review_unknown".to_string());
        let round = self.active_review_round().unwrap_or(1);
        let active_node_type = self.active_node_type();
        let verdict = self.parse_review_verdict_for_active_node(&output);
        self.record_review_message(output);
        self.latest_review_verdict = Some(verdict.clone());
        let reviewer = self
            .active_node_agent()
            .or_else(|| self.session.reviewer_provider.clone());
        let _ = self
            .persist_review_verdict(
                &node_id,
                serde_json::json!({
                    "verdict": verdict.verdict.clone(),
                    "comments": verdict.comments.clone(),
                    "summary": verdict.summary.clone(),
                    "findings": verdict.findings.clone(),
                    "review_gate": verdict.review_gate.clone(),
                    "work_item_plan_review": verdict.work_item_plan_review.clone(),
                }),
            )
            .await;
        let _ = self
            .event_tx
            .send(review_complete_event_from_verdict(
                node_id.clone(),
                round,
                &verdict,
            ))
            .await;
        self.update_timeline_node(
            &node_id,
            TimelineNodeStatus::Completed,
            Some(verdict.summary.clone()),
        )
        .await;
        let artifact_verdict = match &verdict.review_gate {
            ReviewGate::RequiresRevision => ReviewVerdictType::Revise,
            ReviewGate::UserConfirmAllowed => match &verdict.verdict {
                ReviewVerdictType::Pass => ReviewVerdictType::Pass,
                ReviewVerdictType::Revise | ReviewVerdictType::NeedsHuman => {
                    ReviewVerdictType::NeedsHuman
                }
            },
            ReviewGate::UserTriageRequired => ReviewVerdictType::NeedsHuman,
        };
        self.mark_latest_artifact_reviewed(reviewer, Some(artifact_verdict));

        if active_node_type == Some(TimelineNodeType::WorkItemPlanOutlineReview) {
            self.route_work_item_plan_outline_review(verdict).await;
            return;
        }

        if active_node_type == Some(TimelineNodeType::WorkItemDraftReview) {
            self.route_work_item_draft_review(verdict).await;
            return;
        }

        if active_node_type == Some(TimelineNodeType::WorkItemBatchReview) {
            self.route_work_item_batch_review(verdict).await;
            return;
        }

        match &verdict.review_gate {
            ReviewGate::UserConfirmAllowed | ReviewGate::UserTriageRequired => {
                self.enter_human_confirm(Some(verdict.summary)).await;
            }
            ReviewGate::RequiresRevision => {
                self.enter_review_decision(round, verdict.summary).await;
            }
        }
    }

    pub(crate) async fn enter_review_decision(&mut self, round: u32, summary: String) {
        self.transition_stage(WorkspaceStage::ReviewDecision).await;
        let decision_node_id = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::ReviewDecision,
                agent: None,
                stage: WorkspaceStage::ReviewDecision,
                round: Some(round),
                title: format!("Review Decision Round {round}"),
                summary: Some(summary),
                status: TimelineNodeStatus::Paused,
            })
            .await;
        let _ = self
            .event_tx
            .send(EngineEvent::ReviewDecisionRequired {
                node_id: decision_node_id,
                round,
                options: vec![
                    "continue".to_string(),
                    "continue_with_context".to_string(),
                    "human_intervene".to_string(),
                ],
            })
            .await;
    }

    pub(crate) async fn route_work_item_plan_outline_review(&mut self, verdict: ReviewVerdict) {
        let outline_verdict = verdict
            .work_item_plan_review
            .as_ref()
            .map(|review| review.verdict.clone());
        match outline_verdict.unwrap_or(match verdict.verdict {
            ReviewVerdictType::Pass => WorkItemPlanReviewVerdict::Pass,
            ReviewVerdictType::Revise => WorkItemPlanReviewVerdict::Revise,
            ReviewVerdictType::NeedsHuman => WorkItemPlanReviewVerdict::NeedsHuman,
        }) {
            WorkItemPlanReviewVerdict::Pass => {
                self.enter_work_item_generation_mode(Some(
                    "Outline review 通过，请选择 Work Item 生成模式".to_string(),
                ))
                .await;
            }
            WorkItemPlanReviewVerdict::Revise | WorkItemPlanReviewVerdict::PlanReopenRequired => {
                let round = self.active_review_round().unwrap_or(1);
                self.enter_review_decision(round, verdict.summary).await;
            }
            WorkItemPlanReviewVerdict::NeedsHuman | WorkItemPlanReviewVerdict::ReviseBatch => {
                self.enter_human_confirm(Some(verdict.summary)).await;
            }
        }
    }

    pub(crate) async fn route_work_item_draft_review(&mut self, verdict: ReviewVerdict) {
        let draft_payload = match self.current_work_item_draft_candidate_payload() {
            Ok(payload) => payload,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.enter_human_confirm(Some("Work Item Draft artifact 缺失".to_string()))
                    .await;
                return;
            }
        };
        let current_outline_id = draft_payload.draft_record.outline_id.clone();
        let review = verdict.work_item_plan_review.clone();
        let item_verdict = review
            .as_ref()
            .map(|review| review.verdict.clone())
            .unwrap_or(match verdict.verdict {
                ReviewVerdictType::Pass => WorkItemPlanReviewVerdict::Pass,
                ReviewVerdictType::Revise => WorkItemPlanReviewVerdict::Revise,
                ReviewVerdictType::NeedsHuman => WorkItemPlanReviewVerdict::NeedsHuman,
            });
        let target_outline_id = review
            .as_ref()
            .and_then(|review| review.target_outline_id.clone())
            .unwrap_or_else(|| current_outline_id.clone());

        match item_verdict {
            WorkItemPlanReviewVerdict::Pass => {
                if let Err(message) = self
                    .continue_after_work_item_draft_review_pass(&current_outline_id)
                    .await
                {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some(
                        "继续生成下一个 Work Item Draft 失败".to_string(),
                    ))
                    .await;
                }
            }
            WorkItemPlanReviewVerdict::Revise => {
                if target_outline_id != current_outline_id {
                    self.enter_human_confirm(Some(
                        "Reviewer 要求修改非当前 Work Item，已转人工确认".to_string(),
                    ))
                    .await;
                    return;
                }
                self.pending_revision_context = Some(verdict.comments);
                if let Err(message) = self
                    .start_serial_work_item_draft_run_for(&current_outline_id)
                    .await
                {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some("重写当前 Work Item Draft 失败".to_string()))
                        .await;
                }
            }
            WorkItemPlanReviewVerdict::PlanReopenRequired => {
                if let Err(message) = self.mark_work_item_plan_outline_revising() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some("Outline 返修状态保存失败".to_string()))
                        .await;
                    return;
                }
                self.enter_human_confirm(Some(
                    "Reviewer 要求重开 Outline，已暂停逐项生成".to_string(),
                ))
                .await;
            }
            WorkItemPlanReviewVerdict::NeedsHuman | WorkItemPlanReviewVerdict::ReviseBatch => {
                self.enter_human_confirm(Some(verdict.summary)).await;
            }
        }
    }

    pub(crate) async fn route_work_item_batch_review(&mut self, verdict: ReviewVerdict) {
        let review = verdict.work_item_plan_review.clone();
        let batch_verdict = review
            .as_ref()
            .map(|review| review.verdict.clone())
            .unwrap_or(match verdict.verdict {
                ReviewVerdictType::Pass => WorkItemPlanReviewVerdict::Pass,
                ReviewVerdictType::Revise => WorkItemPlanReviewVerdict::ReviseBatch,
                ReviewVerdictType::NeedsHuman => WorkItemPlanReviewVerdict::NeedsHuman,
            });

        match batch_verdict {
            WorkItemPlanReviewVerdict::Pass => {
                if let Err(message) = self.mark_current_work_item_batch_review_done() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some("Batch review 状态保存失败".to_string()))
                        .await;
                    return;
                }
                self.enter_work_item_plan_compile().await;
            }
            WorkItemPlanReviewVerdict::ReviseBatch => {
                self.enter_work_item_batch_confirm(Some(verdict.summary))
                    .await;
            }
            WorkItemPlanReviewVerdict::PlanReopenRequired => {
                if let Err(message) = self.mark_work_item_plan_outline_revising() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some("Outline 返修状态保存失败".to_string()))
                        .await;
                    return;
                }
                self.enter_human_confirm(Some(
                    "Reviewer 要求重开 Outline，已暂停自动生成流程".to_string(),
                ))
                .await;
            }
            WorkItemPlanReviewVerdict::NeedsHuman | WorkItemPlanReviewVerdict::Revise => {
                self.enter_human_confirm(Some(verdict.summary)).await;
            }
        }
    }

    pub(crate) fn mark_current_work_item_batch_review_done(&self) -> Result<(), String> {
        let store = self.work_item_plan_store()?;
        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let batch_id = current_work_item_batch(&index)?.batch_id.clone();
        let batch = index
            .batches
            .iter_mut()
            .find(|batch| batch.batch_id == batch_id)
            .ok_or_else(|| format!("batch `{batch_id}` not found"))?;
        batch.status = WorkItemBatchStatus::ReviewDone;
        index.updated_at = chrono::Utc::now().to_rfc3339();
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;
        Ok(())
    }

    pub(crate) async fn continue_after_work_item_draft_review_pass(
        &mut self,
        outline_id: &str,
    ) -> Result<(), String> {
        let store = self.work_item_plan_store()?;
        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let outline_order = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let current_pos = outline_order
            .iter()
            .position(|id| id == outline_id)
            .ok_or_else(|| format!("outline {outline_id} not found in order"))?;
        let now = chrono::Utc::now().to_rfc3339();

        if let Some(next_outline_id) = outline_order.get(current_pos + 1).cloned() {
            index.active_outline_id = Some(next_outline_id.clone());
            index.updated_at = now;
            store
                .save_active_index(&index)
                .map_err(|error| format!("save work item plan active index failed: {error}"))?;
            self.create_serial_work_item_draft_run_node(&next_outline_id)
                .await;
        } else {
            index.active_outline_id = None;
            index.updated_at = now;
            store
                .save_active_index(&index)
                .map_err(|error| format!("save work item plan active index failed: {error}"))?;
            self.enter_work_item_plan_compile().await;
        }
        Ok(())
    }

    pub(crate) fn parse_review_verdict(output: &str) -> ReviewVerdict {
        Self::parse_review_verdict_for_workspace(output, &WorkspaceType::Story)
    }

    pub(crate) fn parse_review_verdict_for_active_node(&self, output: &str) -> ReviewVerdict {
        if self.session.workspace_type == WorkspaceType::WorkItemPlan
            && self.active_node_type() == Some(TimelineNodeType::WorkItemPlanOutlineReview)
        {
            let valid_outline_ids = self.current_work_item_plan_outline_ids();
            let trimmed = output.trim();
            let parsed = extract_structured_json(trimmed).and_then(|(comments, json)| {
                parse_work_item_plan_review_json(
                    &json,
                    &comments,
                    &valid_outline_ids,
                    WorkItemPlanReviewScope::Outline,
                )
                .or_else(|| parse_review_json(&json, &comments))
            });
            return parsed.unwrap_or_else(|| ReviewVerdict {
                verdict: ReviewVerdictType::NeedsHuman,
                comments: output.to_string(),
                summary: "需要人工确认".to_string(),
                findings: Vec::new(),
                review_gate: ReviewGate::UserTriageRequired,
                work_item_plan_review: None,
            });
        }

        if self.session.workspace_type == WorkspaceType::WorkItemPlan
            && self.active_node_type() == Some(TimelineNodeType::WorkItemDraftReview)
        {
            let valid_outline_ids = self.current_work_item_plan_outline_ids();
            let trimmed = output.trim();
            let parsed = extract_structured_json(trimmed).and_then(|(comments, json)| {
                parse_work_item_plan_review_json(
                    &json,
                    &comments,
                    &valid_outline_ids,
                    WorkItemPlanReviewScope::Item,
                )
                .or_else(|| parse_review_json(&json, &comments))
            });
            return parsed.unwrap_or_else(|| ReviewVerdict {
                verdict: ReviewVerdictType::NeedsHuman,
                comments: output.to_string(),
                summary: "需要人工确认".to_string(),
                findings: Vec::new(),
                review_gate: ReviewGate::UserTriageRequired,
                work_item_plan_review: None,
            });
        }

        if self.session.workspace_type == WorkspaceType::WorkItemPlan
            && self.active_node_type() == Some(TimelineNodeType::WorkItemBatchReview)
        {
            let valid_outline_ids = self.current_work_item_plan_outline_ids();
            let trimmed = output.trim();
            let parsed = extract_structured_json(trimmed).and_then(|(comments, json)| {
                parse_work_item_plan_review_json(
                    &json,
                    &comments,
                    &valid_outline_ids,
                    WorkItemPlanReviewScope::Batch,
                )
                .or_else(|| parse_review_json(&json, &comments))
            });
            return parsed.unwrap_or_else(|| ReviewVerdict {
                verdict: ReviewVerdictType::NeedsHuman,
                comments: output.to_string(),
                summary: "需要人工确认".to_string(),
                findings: Vec::new(),
                review_gate: ReviewGate::UserTriageRequired,
                work_item_plan_review: None,
            });
        }

        Self::parse_review_verdict_for_workspace(output, &self.session.workspace_type)
    }

    pub(crate) fn parse_review_verdict_for_workspace(
        output: &str,
        workspace_type: &WorkspaceType,
    ) -> ReviewVerdict {
        let trimmed = output.trim();
        let parsed = extract_structured_json(trimmed).and_then(|(comments, json)| {
            if *workspace_type == WorkspaceType::WorkItemPlan {
                parse_work_item_plan_review_json(
                    &json,
                    &comments,
                    &[],
                    WorkItemPlanReviewScope::Batch,
                )
                .or_else(|| parse_review_json(&json, &comments))
            } else {
                parse_review_json(&json, &comments)
            }
        });

        parsed.unwrap_or_else(|| ReviewVerdict {
            verdict: ReviewVerdictType::NeedsHuman,
            comments: output.to_string(),
            summary: "需要人工确认".to_string(),
            findings: Vec::new(),
            review_gate: ReviewGate::UserTriageRequired,
            work_item_plan_review: None,
        })
    }
}
