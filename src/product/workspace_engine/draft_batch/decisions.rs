use super::*;

impl WorkspaceEngine {
    // 退役留档（T5/REQ-RET-02）：handle_work_item_batch_decision（WorkItemBatch
    // Decision 消息族唯一引擎入口）随消息族删除（wp5-attribution-table.md §1）；
    // accept/rewrite/downgrade 内部函数保留（review routing legacy 臂仍调用，
    // 服务在途会话读侧状态机）。

    // 退役留档（T5/REQ-RET-02）：handle_work_item_draft_decision（WorkItemDraft
    // Decision 消息族唯一引擎入口）随消息族删除（wp5-attribution-table.md §1）。

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
}
