use super::*;

impl WorkspaceEngine {
    pub(crate) fn current_work_item_draft_candidate_payload(
        &self,
    ) -> Result<WorkItemDraftCandidatePayload, String> {
        match self.session.artifact.as_ref() {
            Some(ArtifactPayload::WorkItemDraftCandidate { draft_candidate }) => {
                Ok(draft_candidate.as_ref().clone())
            }
            _ => Err("current WorkItemDraft artifact is unavailable".to_string()),
        }
    }

    pub async fn select_work_item_generation_mode(
        &mut self,
        mode: WorkItemGenerationModeDto,
    ) -> Result<(), String> {
        if self.session.stage != WorkspaceStage::AuthorConfirm
            || self.active_node_type() != Some(TimelineNodeType::WorkItemGenerationMode)
        {
            return Err(
                "select_work_item_generation_mode requires active work_item_generation_mode node"
                    .to_string(),
            );
        }

        self.update_work_item_plan_outline_generation_metadata(None, Some(mode.clone()))
            .await?;
        self.pending_revision_context = None;
        match mode {
            WorkItemGenerationModeDto::Serial => {
                self.complete_active_node(Some("已选择逐项生成 Work Item".to_string()))
                    .await;
                self.start_serial_work_item_draft_run().await;
            }
            WorkItemGenerationModeDto::Batch => {
                self.create_current_work_item_batch_record()?;
                self.complete_active_node(Some("已选择自动生成全部 Work Item".to_string()))
                    .await;
                self.transition_stage(WorkspaceStage::Running).await;
                let _ = self
                    .create_timeline_node(TimelineNodeDraft {
                        node_type: TimelineNodeType::WorkItemBatchRun,
                        agent: Some(self.session.author_provider.clone()),
                        stage: WorkspaceStage::Running,
                        round: None,
                        title: "Work Item Batch 生成".to_string(),
                        summary: Some("WP5 占位节点，Batch 实际生成由后续 WP 接入".to_string()),
                        status: TimelineNodeStatus::Active,
                    })
                    .await;
            }
        }
        Ok(())
    }

    pub(crate) async fn begin_work_item_draft_review_run(&mut self, outline_id: &str) -> String {
        self.transition_stage(WorkspaceStage::CrossReview).await;
        let round = self.next_review_round();
        let reviewer = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemDraftReview,
            agent: Some(reviewer),
            stage: WorkspaceStage::CrossReview,
            round: Some(round),
            title: format!("Work Item Draft Review Round {round}"),
            summary: Some(format!("审核 outline `{outline_id}` 的 Work Item Draft")),
            status: TimelineNodeStatus::Active,
        })
        .await
    }

    pub(crate) async fn begin_work_item_batch_review_run(&mut self) -> String {
        self.transition_stage(WorkspaceStage::CrossReview).await;
        let round = self.next_review_round();
        let reviewer = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemBatchReview,
            agent: Some(reviewer),
            stage: WorkspaceStage::CrossReview,
            round: Some(round),
            title: format!("Work Item Batch Review Round {round}"),
            summary: Some("审核整组 Work Item Draft".to_string()),
            status: TimelineNodeStatus::Active,
        })
        .await
    }

    pub(crate) fn create_current_work_item_batch_record(
        &self,
    ) -> Result<WorkItemBatchRecord, String> {
        let store = self.work_item_plan_store()?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let mut index = store
            .load_active_index(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index is missing".to_string())?;
        let now = chrono::Utc::now().to_rfc3339();
        let batch = WorkItemBatchRecord {
            batch_id: next_batch_id(&index, &now),
            generation_round_id: index.current_generation_round_id.clone(),
            mode: WorkItemGenerationMode::Batch,
            item_draft_ids: Vec::new(),
            status: WorkItemBatchStatus::Generating,
            validation_failed_ids: Vec::new(),
            created_at: now.clone(),
        };
        let outline_candidate = self.current_work_item_plan_outline_candidate()?;
        let first_outline_id =
            work_item_plan_outline_topological_order(&outline_candidate.outline)?
                .into_iter()
                .next()
                .ok_or_else(|| "WorkItemPlan Outline has no work item outlines".to_string())?;
        index.active_outline_id = Some(first_outline_id);
        index.batches.push(batch.clone());
        index.updated_at = now;
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;
        Ok(batch)
    }

    pub(crate) async fn start_serial_work_item_draft_run(&mut self) {
        let first_outline_id =
            match self
                .current_work_item_plan_outline_candidate()
                .and_then(|outline_candidate| {
                    work_item_plan_outline_topological_order(&outline_candidate.outline).and_then(
                        |order| {
                            order.into_iter().next().ok_or_else(|| {
                                "WorkItemPlan Outline has no work item outlines".to_string()
                            })
                        },
                    )
                }) {
                Ok(outline_id) => outline_id,
                Err(message) => {
                    self.enter_human_confirm(Some(format!(
                        "无法开始逐项生成 Work Item：{message}"
                    )))
                    .await;
                    return;
                }
            };

        if let Err(message) = self.set_active_work_item_plan_outline(&first_outline_id) {
            let _ = self.event_tx.send(EngineEvent::Error { message }).await;
            self.enter_human_confirm(Some("保存当前 Work Item 游标失败".to_string()))
                .await;
            return;
        }

        self.create_serial_work_item_draft_run_node(&first_outline_id)
            .await;
    }

    pub(crate) async fn start_serial_work_item_draft_run_for(
        &mut self,
        outline_id: &str,
    ) -> Result<(), String> {
        self.set_active_work_item_plan_outline(outline_id)?;
        self.create_serial_work_item_draft_run_node(outline_id)
            .await;
        Ok(())
    }

    pub(crate) async fn create_serial_work_item_draft_run_node(&mut self, outline_id: &str) {
        self.transition_stage(WorkspaceStage::Running).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemDraftRun,
                agent: Some(self.session.author_provider.clone()),
                stage: WorkspaceStage::Running,
                round: None,
                title: "Work Item Draft 生成".to_string(),
                summary: Some(format!(
                    "准备生成 outline `{outline_id}` 的 Work Item Draft"
                )),
                status: TimelineNodeStatus::Active,
            })
            .await;
    }

    pub fn build_current_work_item_draft_streaming_input(
        &mut self,
        feedback: Option<&str>,
    ) -> Result<StreamingProviderInput, String> {
        let effective_feedback = match feedback {
            Some(value) => {
                self.pending_revision_context = None;
                Some(value.to_string())
            }
            None => self.pending_revision_context.take(),
        };
        self.build_current_work_item_draft_streaming_input_with_feedback(
            effective_feedback.as_deref(),
        )
    }

    pub(crate) fn build_current_work_item_draft_streaming_input_with_feedback(
        &self,
        feedback: Option<&str>,
    ) -> Result<StreamingProviderInput, String> {
        let store = self.work_item_plan_store()?;
        let index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let active_outline_id = index
            .active_outline_id
            .clone()
            .ok_or_else(|| "active work item outline missing".to_string())?;
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let accepted_drafts = self.accepted_work_item_plan_draft_records(&store, &index)?;
        let invocation = build_work_item_draft_invocation(
            &outline_candidate.outline,
            &active_outline_id,
            WorkItemGenerationMode::Serial,
            &accepted_drafts,
            feedback,
        )
        .map_err(|error| error.message)?;
        let working_dir = self
            .session
            .repository_path
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| "working directory unavailable".to_string())?;
        Ok(self.build_work_item_plan_streaming_input(
            provider_type_for_name(&self.session.author_provider),
            invocation.prompt,
            working_dir.to_string_lossy().to_string(),
            self.session.author_provider.clone(),
        ))
    }

    pub fn build_current_work_item_batch_draft_streaming_input(
        &self,
    ) -> Result<StreamingProviderInput, String> {
        if self.active_node_type() != Some(TimelineNodeType::WorkItemBatchRun) {
            return Err("batch draft input requires active work_item_batch_run node".to_string());
        }
        let store = self.work_item_plan_store()?;
        let index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let active_outline_id = index
            .active_outline_id
            .clone()
            .ok_or_else(|| "active batch work item outline missing".to_string())?;
        let batch = current_work_item_batch(&index)?;
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let batch_drafts =
            self.batch_work_item_plan_draft_records(&store, &index, &batch.batch_id)?;
        let invocation = build_work_item_draft_invocation(
            &outline_candidate.outline,
            &active_outline_id,
            WorkItemGenerationMode::Batch,
            &batch_drafts,
            None,
        )
        .map_err(|error| error.message)?;
        let working_dir = self
            .session
            .repository_path
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| "working directory unavailable".to_string())?;
        Ok(self.build_work_item_plan_streaming_input(
            provider_type_for_name(&self.session.author_provider),
            invocation.prompt,
            working_dir.to_string_lossy().to_string(),
            self.session.author_provider.clone(),
        ))
    }

    pub(crate) fn accepted_work_item_plan_draft_records(
        &self,
        store: &WorkItemPlanStore,
        index: &WorkItemPlanDraftActiveIndex,
    ) -> Result<Vec<WorkItemDraftRecord>, String> {
        let mut records = Vec::new();
        for draft_id in index.outline_to_current_draft_id.values() {
            if index.draft_statuses.get(draft_id) != Some(&WorkItemDraftStatus::Accepted) {
                continue;
            }
            let record = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    draft_id,
                )
                .map_err(|error| format!("load accepted draft record failed: {error}"))?;
            records.push(record);
        }
        Ok(records)
    }

    pub(crate) fn batch_work_item_plan_draft_records(
        &self,
        store: &WorkItemPlanStore,
        index: &WorkItemPlanDraftActiveIndex,
        batch_id: &str,
    ) -> Result<Vec<WorkItemDraftRecord>, String> {
        let batch = index
            .batches
            .iter()
            .find(|batch| batch.batch_id == batch_id)
            .ok_or_else(|| format!("batch `{batch_id}` not found"))?;
        let mut records = Vec::new();
        for draft_id in &batch.item_draft_ids {
            let record = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    draft_id,
                )
                .map_err(|error| format!("load batch draft record failed: {error}"))?;
            records.push(record);
        }
        Ok(records)
    }

    pub(crate) fn current_work_item_batch_state_payload(
        &self,
        store: &WorkItemPlanStore,
        index: &WorkItemPlanDraftActiveIndex,
        batch_id: &str,
    ) -> Result<WorkItemBatchStatePayload, String> {
        let batch = index
            .batches
            .iter()
            .find(|batch| batch.batch_id == batch_id)
            .ok_or_else(|| format!("batch `{batch_id}` not found"))?;
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let queue = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let mut draft_records = Vec::new();
        for draft_id in batch
            .item_draft_ids
            .iter()
            .chain(batch.validation_failed_ids.iter())
        {
            draft_records.push(
                store
                    .get_draft_record(
                        &index.project_id,
                        &index.issue_id,
                        &index.plan_id,
                        &index.current_generation_round_id,
                        draft_id,
                    )
                    .map_err(|error| format!("load batch state draft failed: {error}"))?,
            );
        }
        let failure_summary = draft_records
            .iter()
            .filter(|record| record.status == WorkItemDraftStatus::ValidationFailed)
            .map(|record| WorkItemBatchFailureSummaryDto {
                draft_id: record.draft_id.clone(),
                outline_id: record.outline_id.clone(),
                status: work_item_draft_status_label(&record.status).to_string(),
            })
            .collect();

        Ok(WorkItemBatchStatePayload {
            batch_id: batch.batch_id.clone(),
            generation_round_id: batch.generation_round_id.clone(),
            queue,
            draft_records,
            batch_status: batch.status.clone(),
            failure_summary,
        })
    }
}
