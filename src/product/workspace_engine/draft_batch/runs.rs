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

    // 退役留档（T5/REQ-RET-02）：select_work_item_generation_mode（SelectWorkItem
    // GenerationMode 消息族唯一引擎入口）随消息族删除（wp5-attribution-table.md
    // §1）；mode 值类型迁至 artifact.rs（历史 artifact 载荷字段+SC 内部诊断）。

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
        let outline_title = self.work_item_outline_title_for(outline_id);
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemDraftRun,
                agent: Some(self.session.author_provider.clone()),
                stage: WorkspaceStage::Running,
                round: None,
                title: format!("Draft · {outline_title}"),
                summary: Some(format!("{outline_id} · pending")),
                status: TimelineNodeStatus::Active,
            })
            .await;
    }

    fn work_item_outline_title_for(&self, outline_id: &str) -> String {
        self.latest_work_item_plan_outline_candidate()
            .ok()
            .and_then(|candidate| {
                candidate
                    .outline
                    .work_item_outlines
                    .into_iter()
                    .find(|item| item.outline_id == outline_id)
                    .map(|item| item.title)
            })
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| outline_id.to_string())
    }

    pub(crate) fn build_current_work_item_draft_streaming_input(
        &mut self,
        feedback: Option<&str>,
        context: &crate::product::cadence_skills::routing_reference::RoutingReferenceContext,
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
            context,
        )
    }

    pub(crate) fn build_current_work_item_draft_streaming_input_with_feedback(
        &self,
        feedback: Option<&str>,
        context: &crate::product::cadence_skills::routing_reference::RoutingReferenceContext,
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
            context,
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

    pub(crate) fn build_current_work_item_batch_draft_streaming_input(
        &mut self,
        feedback: Option<&str>,
        context: &crate::product::cadence_skills::routing_reference::RoutingReferenceContext,
    ) -> Result<StreamingProviderInput, String> {
        if self.active_node_type() != Some(TimelineNodeType::WorkItemBatchRun) {
            return Err("batch draft input requires active work_item_batch_run node".to_string());
        }
        let effective_feedback = match feedback {
            Some(value) => {
                self.pending_revision_context = None;
                Some(value.to_string())
            }
            None => self.pending_revision_context.take(),
        };
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
            effective_feedback.as_deref(),
            context,
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

    // L2 退役（T5/REQ-RET-02）：wire 入口已删；保留供 policy routing 测试构造
    // batch review 场景（cfg(test)）。
    #[cfg(test)]
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
}
