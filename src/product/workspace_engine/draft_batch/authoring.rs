use super::*;

impl WorkspaceEngine {
    pub async fn complete_work_item_draft_author(
        &mut self,
        candidate: WorkItemDraftCandidate,
    ) -> Result<(), String> {
        if self.active_node_type() != Some(TimelineNodeType::WorkItemDraftRun) {
            return Err("work item draft author completion requires active draft run".to_string());
        }

        let generated_from_node_id = self
            .active_node_id
            .clone()
            .ok_or_else(|| "active draft run node missing".to_string())?;
        let store = self.work_item_plan_store()?;
        let mut index = store
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
        if candidate.outline_id != active_outline_id {
            return Err(format!(
                "draft outline_id {} does not match active outline {}",
                candidate.outline_id, active_outline_id
            ));
        }
        let previous_draft_record = match index
            .outline_to_current_draft_id
            .get(&active_outline_id)
            .cloned()
        {
            Some(previous_draft_id) => Some(
                store
                    .get_draft_record(
                        &index.project_id,
                        &index.issue_id,
                        &index.plan_id,
                        &index.current_generation_round_id,
                        &previous_draft_id,
                    )
                    .map_err(|error| format!("load previous draft record failed: {error}"))?,
            ),
            None => None,
        };

        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let current_outline = outline_candidate
            .outline
            .work_item_outlines
            .iter()
            .find(|item| item.outline_id == active_outline_id)
            .cloned()
            .ok_or_else(|| format!("active outline {active_outline_id} not found"))?;
        let accepted_drafts = self.accepted_work_item_plan_draft_records(&store, &index)?;
        let accepted_candidates: Vec<WorkItemDraftCandidate> = accepted_drafts
            .iter()
            .map(|record| record.candidate.clone())
            .collect();
        let report = WorkItemDraftLocalValidator::validate(
            &candidate,
            &accepted_candidates,
            &current_outline,
        );
        let status = if report.has_errors() {
            WorkItemDraftStatus::ValidationFailed
        } else {
            WorkItemDraftStatus::Draft
        };
        let draft_id = next_draft_id(&index);
        let now = chrono::Utc::now().to_rfc3339();
        let record = WorkItemDraftRecord {
            project_id: self.session.project_id.clone(),
            issue_id: self.session.issue_id.clone(),
            plan_id: self.session.entity_id.clone(),
            draft_id: draft_id.clone(),
            outline_id: active_outline_id.clone(),
            generation_round_id: index.current_generation_round_id.clone(),
            batch_id: None,
            attempt_index: previous_draft_record
                .as_ref()
                .map(|record| record.attempt_index + 1)
                .unwrap_or(1),
            outline_version_ref: outline_candidate.outline.id.clone(),
            generation_mode: WorkItemGenerationMode::Serial,
            candidate,
            status: status.clone(),
            active: true,
            superseded_by_draft_id: None,
            supersede_reason: None,
            copied_from_draft_id: None,
            review_node_id: None,
            review_verdict_ref: None,
            generated_from_node_id,
            accepted_at: None,
            superseded_at: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        store
            .put_draft_record(&record)
            .map_err(|error| format!("save work item draft record failed: {error}"))?;
        if let Some(mut previous_record) = previous_draft_record
            && previous_record.draft_id != draft_id
        {
            mark_draft_record_superseded(
                &mut previous_record,
                Some(draft_id.clone()),
                WorkItemDraftSupersedeReason::DirectRewrite,
                &now,
            );
            store
                .put_draft_record(&previous_record)
                .map_err(|error| format!("save superseded draft record failed: {error}"))?;
        }
        mark_draft_active(&mut index, &active_outline_id, &draft_id, status.clone());
        index.active_outline_id = Some(active_outline_id);
        index.updated_at = now;
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;

        let validator_findings = work_item_split_findings_to_dto(&report.findings);
        let can_accept = !report.has_errors();
        let completion_summary = format!(
            "{} · {} · {}",
            record.outline_id,
            record.draft_id,
            work_item_draft_status_label(&record.status)
        );
        self.update_artifact(ArtifactPayload::WorkItemDraftCandidate {
            draft_candidate: Box::new(WorkItemDraftCandidatePayload {
                draft_record: record,
                validator_findings,
                can_accept,
            }),
        })
        .await;
        self.complete_active_node(Some(completion_summary)).await;
        self.enter_work_item_draft_confirm(Some("请确认当前 Work Item Draft".to_string()))
            .await;
        Ok(())
    }

    pub async fn complete_work_item_batch_draft_author(
        &mut self,
        candidate: WorkItemDraftCandidate,
    ) -> Result<(), String> {
        if self.active_node_type() != Some(TimelineNodeType::WorkItemBatchRun) {
            return Err("batch draft author completion requires active batch run".to_string());
        }

        let generated_from_node_id = self
            .active_node_id
            .clone()
            .ok_or_else(|| "active batch run node missing".to_string())?;
        let store = self.work_item_plan_store()?;
        let mut index = store
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
        if candidate.outline_id != active_outline_id {
            return Err(format!(
                "draft outline_id {} does not match active outline {}",
                candidate.outline_id, active_outline_id
            ));
        }
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let current_outline = outline_candidate
            .outline
            .work_item_outlines
            .iter()
            .find(|item| item.outline_id == active_outline_id)
            .cloned()
            .ok_or_else(|| format!("active outline {active_outline_id} not found"))?;
        let batch_id = current_work_item_batch(&index)?.batch_id.clone();
        let batch_drafts = self.batch_work_item_plan_draft_records(&store, &index, &batch_id)?;
        let batch_candidates: Vec<WorkItemDraftCandidate> = batch_drafts
            .iter()
            .map(|record| record.candidate.clone())
            .collect();
        let report =
            WorkItemDraftLocalValidator::validate(&candidate, &batch_candidates, &current_outline);
        if report.has_errors() {
            let retry_count = self
                .work_item_batch_retry_counts
                .entry(active_outline_id.clone())
                .or_default();
            if *retry_count == 0 {
                *retry_count += 1;
                return Ok(());
            }
        } else {
            self.work_item_batch_retry_counts.remove(&active_outline_id);
        }
        let status = if report.has_errors() {
            WorkItemDraftStatus::ValidationFailed
        } else {
            WorkItemDraftStatus::Draft
        };
        let draft_id = next_draft_id(&index);
        let now = chrono::Utc::now().to_rfc3339();
        let record = WorkItemDraftRecord {
            project_id: self.session.project_id.clone(),
            issue_id: self.session.issue_id.clone(),
            plan_id: self.session.entity_id.clone(),
            draft_id: draft_id.clone(),
            outline_id: active_outline_id.clone(),
            generation_round_id: index.current_generation_round_id.clone(),
            batch_id: Some(batch_id.clone()),
            attempt_index: 1,
            outline_version_ref: outline_candidate.outline.id.clone(),
            generation_mode: WorkItemGenerationMode::Batch,
            candidate,
            status: status.clone(),
            active: true,
            superseded_by_draft_id: None,
            supersede_reason: None,
            copied_from_draft_id: None,
            review_node_id: None,
            review_verdict_ref: None,
            generated_from_node_id,
            accepted_at: None,
            superseded_at: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        store
            .put_draft_record(&record)
            .map_err(|error| format!("save batch work item draft record failed: {error}"))?;
        mark_draft_active(&mut index, &active_outline_id, &draft_id, status.clone());
        let outline_order = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let current_pos = outline_order
            .iter()
            .position(|id| id == &active_outline_id)
            .ok_or_else(|| format!("outline {active_outline_id} not found in order"))?;
        let next_outline_id = outline_order.get(current_pos + 1).cloned();
        {
            let batch = index
                .batches
                .iter_mut()
                .find(|batch| batch.batch_id == batch_id)
                .ok_or_else(|| format!("batch `{batch_id}` not found"))?;
            match status {
                WorkItemDraftStatus::ValidationFailed => {
                    batch.validation_failed_ids.push(draft_id.clone());
                }
                _ => {
                    batch.item_draft_ids.push(draft_id.clone());
                }
            }
            if next_outline_id.is_none() {
                batch.status = WorkItemBatchStatus::Completed;
            }
        }
        index.active_outline_id = next_outline_id;
        index.updated_at = now;
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;

        let validator_findings = work_item_split_findings_to_dto(&report.findings);
        self.update_artifact(ArtifactPayload::WorkItemDraftCandidate {
            draft_candidate: Box::new(WorkItemDraftCandidatePayload {
                draft_record: record,
                validator_findings,
                can_accept: !report.has_errors(),
            }),
        })
        .await;
        if index.active_outline_id.is_none() {
            let batch_state =
                self.current_work_item_batch_state_payload(&store, &index, &batch_id)?;
            self.update_artifact(ArtifactPayload::WorkItemBatchState {
                batch_state: Box::new(batch_state),
            })
            .await;
            self.complete_active_node(Some("Work Item Batch 生成完成，等待整组确认".to_string()))
                .await;
            self.enter_work_item_batch_confirm(Some("请确认整组 Work Item Draft".to_string()))
                .await;
        }
        Ok(())
    }
}
