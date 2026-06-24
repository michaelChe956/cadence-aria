use super::*;

impl WorkspaceEngine {
    pub async fn handle_work_item_batch_decision(
        &mut self,
        decision: WorkItemBatchDecisionDto,
        feedback: Option<String>,
        first_affected_outline_id: Option<String>,
    ) -> Result<WorkItemBatchDecisionOutcome, String> {
        if self.session.stage != WorkspaceStage::AuthorConfirm
            || self.active_node_type() != Some(TimelineNodeType::WorkItemBatchConfirm)
        {
            return Err(
                "work_item_batch_decision requires active work_item_batch_confirm node".to_string(),
            );
        }

        match decision {
            WorkItemBatchDecisionDto::AcceptAll => self.accept_current_work_item_batch().await,
            WorkItemBatchDecisionDto::Pause => {
                self.complete_active_node(Some("Work Item Batch 已暂停".to_string()))
                    .await;
                self.enter_human_confirm(Some("Work Item Batch 已暂停，等待人工处理".to_string()))
                    .await;
                Ok(WorkItemBatchDecisionOutcome::HumanConfirm)
            }
            WorkItemBatchDecisionDto::RewriteBatch => self.rewrite_current_work_item_batch().await,
            WorkItemBatchDecisionDto::DowngradeToSerial => {
                self.downgrade_current_work_item_batch_to_serial(
                    first_affected_outline_id,
                    feedback,
                )
                .await
            }
        }
    }

    pub(crate) async fn accept_current_work_item_batch(
        &mut self,
    ) -> Result<WorkItemBatchDecisionOutcome, String> {
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
        let batch_pos = index
            .batches
            .iter()
            .position(|batch| batch.batch_id == batch_id)
            .ok_or_else(|| format!("batch `{batch_id}` not found"))?;
        if !index.batches[batch_pos].validation_failed_ids.is_empty() {
            return Err("accept_all requires no validation_failed drafts".to_string());
        }

        let now = chrono::Utc::now().to_rfc3339();
        let draft_ids = index.batches[batch_pos].item_draft_ids.clone();
        for draft_id in &draft_ids {
            let mut record = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    draft_id,
                )
                .map_err(|error| format!("load batch draft record failed: {error}"))?;
            if record.status == WorkItemDraftStatus::ValidationFailed {
                return Err(format!(
                    "draft `{}` has validation errors and cannot be accepted",
                    record.draft_id
                ));
            }
            record.status = WorkItemDraftStatus::Accepted;
            record.accepted_at = Some(now.clone());
            record.updated_at = now.clone();
            store
                .put_draft_record(&record)
                .map_err(|error| format!("save accepted batch draft failed: {error}"))?;
            index
                .draft_statuses
                .insert(draft_id.clone(), WorkItemDraftStatus::Accepted);
        }
        let review_enabled =
            self.session.review_rounds > 0 && self.session.reviewer_provider.is_some();
        index.batches[batch_pos].status = if review_enabled {
            WorkItemBatchStatus::ReviewPending
        } else {
            WorkItemBatchStatus::ReviewDone
        };
        index.updated_at = now;
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;

        self.complete_active_node(Some("Work Item Batch 已接受".to_string()))
            .await;
        if review_enabled {
            self.begin_work_item_batch_review_run().await;
            Ok(WorkItemBatchDecisionOutcome::StartReview)
        } else {
            self.enter_work_item_plan_compile().await;
            Ok(WorkItemBatchDecisionOutcome::HumanConfirm)
        }
    }

    pub async fn handle_work_item_draft_decision(
        &mut self,
        outline_id: String,
        decision: WorkItemDraftDecisionDto,
        feedback: Option<String>,
    ) -> Result<WorkItemDraftDecisionOutcome, String> {
        if self.session.stage != WorkspaceStage::AuthorConfirm
            || self.active_node_type() != Some(TimelineNodeType::WorkItemDraftConfirm)
        {
            return Err(
                "work_item_draft_decision requires active work_item_draft_confirm node".to_string(),
            );
        }

        match decision {
            WorkItemDraftDecisionDto::Accept => {
                self.accept_current_work_item_draft(outline_id).await
            }
            WorkItemDraftDecisionDto::Rewrite => {
                self.pending_revision_context = feedback;
                self.complete_active_node(Some("用户要求重写当前 Work Item Draft".to_string()))
                    .await;
                self.start_serial_work_item_draft_run_for(&outline_id)
                    .await?;
                Ok(WorkItemDraftDecisionOutcome::StartDraftRun)
            }
            WorkItemDraftDecisionDto::Pause => {
                self.complete_active_node(Some("用户暂停逐项 Work Item 生成".to_string()))
                    .await;
                self.enter_human_confirm(Some("逐项 Work Item 生成已暂停".to_string()))
                    .await;
                Ok(WorkItemDraftDecisionOutcome::HumanConfirm)
            }
        }
    }

    pub(crate) async fn accept_current_work_item_draft(
        &mut self,
        outline_id: String,
    ) -> Result<WorkItemDraftDecisionOutcome, String> {
        let Some(ArtifactPayload::WorkItemDraftCandidate { draft_candidate }) =
            self.session.artifact.clone()
        else {
            return Err("current artifact is not a WorkItemDraftCandidate".to_string());
        };
        if draft_candidate.draft_record.outline_id != outline_id {
            return Err(format!(
                "draft decision outline_id {} does not match current draft {}",
                outline_id, draft_candidate.draft_record.outline_id
            ));
        }
        if !draft_candidate.can_accept
            || draft_candidate.draft_record.status == WorkItemDraftStatus::ValidationFailed
        {
            return Err("current work item draft has local validation errors".to_string());
        }

        let store = self.work_item_plan_store()?;
        let mut record = draft_candidate.draft_record.clone();
        let now = chrono::Utc::now().to_rfc3339();
        record.status = WorkItemDraftStatus::Accepted;
        record.accepted_at = Some(now.clone());
        record.updated_at = now.clone();
        store
            .put_draft_record(&record)
            .map_err(|error| format!("save accepted work item draft failed: {error}"))?;

        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        index
            .draft_statuses
            .insert(record.draft_id.clone(), WorkItemDraftStatus::Accepted);
        let review_enabled =
            self.session.review_rounds > 0 && self.session.reviewer_provider.is_some();

        self.update_artifact(ArtifactPayload::WorkItemDraftCandidate {
            draft_candidate: Box::new(WorkItemDraftCandidatePayload {
                draft_record: record.clone(),
                validator_findings: draft_candidate.validator_findings.clone(),
                can_accept: true,
            }),
        })
        .await;
        self.complete_active_node(Some("Work Item Draft 已接受".to_string()))
            .await;

        if review_enabled {
            index.active_outline_id = Some(outline_id.clone());
            index.updated_at = now;
            store
                .save_active_index(&index)
                .map_err(|error| format!("save work item plan active index failed: {error}"))?;
            self.begin_work_item_draft_review_run(&outline_id).await;
            return Ok(WorkItemDraftDecisionOutcome::StartReview);
        }

        let outline_order = {
            let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
            work_item_plan_outline_topological_order(&outline_candidate.outline)?
        };
        let current_pos = outline_order
            .iter()
            .position(|id| id == &outline_id)
            .ok_or_else(|| format!("outline {outline_id} not found in order"))?;
        let next_outline_id = outline_order.get(current_pos + 1).cloned();

        if let Some(next_outline_id) = next_outline_id {
            index.active_outline_id = Some(next_outline_id.clone());
            index.updated_at = now;
            store
                .save_active_index(&index)
                .map_err(|error| format!("save work item plan active index failed: {error}"))?;
            self.create_serial_work_item_draft_run_node(&next_outline_id)
                .await;
            Ok(WorkItemDraftDecisionOutcome::StartDraftRun)
        } else {
            index.active_outline_id = None;
            index.updated_at = now;
            store
                .save_active_index(&index)
                .map_err(|error| format!("save work item plan active index failed: {error}"))?;
            self.enter_work_item_plan_compile().await;
            Ok(WorkItemDraftDecisionOutcome::HumanConfirm)
        }
    }

    pub(crate) async fn rewrite_current_work_item_batch(
        &mut self,
    ) -> Result<WorkItemBatchDecisionOutcome, String> {
        let store = self.work_item_plan_store()?;
        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let current_batch = current_work_item_batch(&index)?.clone();
        let now = chrono::Utc::now().to_rfc3339();
        let old_draft_ids: Vec<String> = current_batch
            .item_draft_ids
            .iter()
            .chain(current_batch.validation_failed_ids.iter())
            .cloned()
            .collect();
        for draft_id in &old_draft_ids {
            let mut record = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    draft_id,
                )
                .map_err(|error| format!("load batch draft record failed: {error}"))?;
            mark_draft_record_superseded(
                &mut record,
                None,
                WorkItemDraftSupersedeReason::DirectRewrite,
                &now,
            );
            store
                .put_draft_record(&record)
                .map_err(|error| format!("save superseded batch draft failed: {error}"))?;
            index
                .draft_statuses
                .insert(draft_id.clone(), WorkItemDraftStatus::Superseded);
            if index
                .outline_to_current_draft_id
                .get(&record.outline_id)
                .is_some_and(|current_draft_id| current_draft_id == draft_id)
            {
                index.outline_to_current_draft_id.remove(&record.outline_id);
            }
        }

        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let first_outline_id =
            work_item_plan_outline_topological_order(&outline_candidate.outline)?
                .into_iter()
                .next()
                .ok_or_else(|| "WorkItemPlan Outline has no work item outlines".to_string())?;
        let new_batch = WorkItemBatchRecord {
            batch_id: next_batch_id(&index, &now),
            generation_round_id: index.current_generation_round_id.clone(),
            mode: WorkItemGenerationMode::Batch,
            item_draft_ids: Vec::new(),
            status: WorkItemBatchStatus::Generating,
            validation_failed_ids: Vec::new(),
            created_at: now.clone(),
        };
        index.active_outline_id = Some(first_outline_id);
        index.batches.push(new_batch);
        index.updated_at = now;
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;

        self.complete_active_node(Some("Work Item Batch 已请求整组重写".to_string()))
            .await;
        self.transition_stage(WorkspaceStage::Running).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemBatchRun,
                agent: Some(self.session.author_provider.clone()),
                stage: WorkspaceStage::Running,
                round: None,
                title: "Work Item Batch 生成".to_string(),
                summary: Some("正在整组重写 Work Item Draft".to_string()),
                status: TimelineNodeStatus::Active,
            })
            .await;
        Ok(WorkItemBatchDecisionOutcome::StartBatchRun)
    }

    pub(crate) async fn downgrade_current_work_item_batch_to_serial(
        &mut self,
        first_affected_outline_id: Option<String>,
        feedback: Option<String>,
    ) -> Result<WorkItemBatchDecisionOutcome, String> {
        let store = self.work_item_plan_store()?;
        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        if !self.current_round_has_failed_compile(&store, &index)? {
            return Err(
                "downgrade_to_serial is not available before strict validation".to_string(),
            );
        }
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let outline_order = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let target_outline_id = first_affected_outline_id
            .or_else(|| outline_order.first().cloned())
            .ok_or_else(|| "WorkItemPlan Outline has no work item outlines".to_string())?;
        if !outline_order
            .iter()
            .any(|outline_id| outline_id == &target_outline_id)
        {
            return Err(format!(
                "first_affected_outline_id `{target_outline_id}` is not in current outline"
            ));
        }
        let target_pos = outline_order
            .iter()
            .position(|outline_id| outline_id == &target_outline_id)
            .ok_or_else(|| format!("outline {target_outline_id} not found in order"))?;
        let generated_from_node_id = self
            .active_node_id
            .clone()
            .unwrap_or_else(|| "work_item_batch_downgrade".to_string());
        let now = chrono::Utc::now().to_rfc3339();
        let mut accepted_copied_candidates = Vec::new();

        for outline_id in outline_order.iter().take(target_pos) {
            let source_draft_id = index
                .outline_to_current_draft_id
                .get(outline_id)
                .cloned()
                .ok_or_else(|| {
                    format!("cannot downgrade before `{target_outline_id}`: outline `{outline_id}` has no current draft")
                })?;
            let mut source = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    &source_draft_id,
                )
                .map_err(|error| format!("load draft for downgrade failed: {error}"))?;
            let current_outline = outline_candidate
                .outline
                .work_item_outlines
                .iter()
                .find(|item| item.outline_id == *outline_id)
                .cloned()
                .ok_or_else(|| format!("outline `{outline_id}` not found"))?;
            let report = WorkItemDraftLocalValidator::validate(
                &source.candidate,
                &accepted_copied_candidates,
                &current_outline,
            );
            let mut copied =
                copy_draft_for_current_round(&index, &source, &generated_from_node_id, &now);
            copied.status = if report.has_errors() {
                WorkItemDraftStatus::ValidationFailed
            } else {
                WorkItemDraftStatus::Accepted
            };
            copied.accepted_at = if copied.status == WorkItemDraftStatus::Accepted {
                Some(now.clone())
            } else {
                None
            };

            store
                .put_draft_record(&copied)
                .map_err(|error| format!("save copied serial draft failed: {error}"))?;
            mark_draft_record_superseded(
                &mut source,
                Some(copied.draft_id.clone()),
                WorkItemDraftSupersedeReason::DirectRewrite,
                &now,
            );
            store
                .put_draft_record(&source)
                .map_err(|error| format!("save superseded batch draft failed: {error}"))?;
            mark_draft_active(
                &mut index,
                outline_id,
                &copied.draft_id,
                copied.status.clone(),
            );
            if copied.status == WorkItemDraftStatus::Accepted {
                accepted_copied_candidates.push(copied.candidate.clone());
            } else {
                return Err(format!(
                    "copied draft for outline `{outline_id}` failed local validation during downgrade"
                ));
            }
        }

        index.active_outline_id = Some(target_outline_id.clone());
        index.updated_at = now;
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;

        self.complete_active_node(Some(
            feedback
                .map(|feedback| format!("已降级为逐项生成：{feedback}"))
                .unwrap_or_else(|| "已降级为逐项生成".to_string()),
        ))
        .await;
        self.start_serial_work_item_draft_run_for(&target_outline_id)
            .await?;
        Ok(WorkItemBatchDecisionOutcome::StartDraftRun)
    }

    pub(crate) fn current_round_has_failed_compile(
        &self,
        store: &WorkItemPlanStore,
        index: &WorkItemPlanDraftActiveIndex,
    ) -> Result<bool, String> {
        let transactions = store
            .list_compile_transactions(&index.project_id, &index.issue_id, &index.plan_id)
            .map_err(|error| format!("list compile transactions failed: {error}"))?;
        Ok(transactions.iter().any(|tx| {
            tx.generation_round_id == index.current_generation_round_id
                && tx.status == WorkItemPlanCompileStatus::Failed
                && tx.plan_commit_state == WorkItemPlanCommitState::NotStarted
        }))
    }
}
