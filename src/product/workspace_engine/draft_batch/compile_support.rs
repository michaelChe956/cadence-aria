use super::*;

impl WorkspaceEngine {
    pub(crate) fn is_current_work_item_plan_batch_mode(&self) -> bool {
        let Ok(store) = self.work_item_plan_store() else {
            return false;
        };
        store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .ok()
            .flatten()
            .map(|index| {
                index.batches.iter().any(|batch| {
                    batch.generation_round_id == index.current_generation_round_id
                        && batch.mode == WorkItemGenerationMode::Batch
                })
            })
            .unwrap_or(false)
    }

    pub(crate) fn work_item_plan_repository_id(
        &self,
        lifecycle: &LifecycleStore,
        plan: &IssueWorkItemPlan,
    ) -> Result<String, String> {
        let story_specs = lifecycle
            .list_story_specs(&plan.project_id, &plan.issue_id)
            .map_err(|error| format!("list story specs failed: {error}"))?;
        for story_id in &plan.source_story_spec_ids {
            if let Some(story) = story_specs.iter().find(|story| &story.id == story_id) {
                return Ok(story.repository_id.clone());
            }
        }
        Err("cannot resolve repository_id for WorkItemPlan compile".to_string())
    }

    pub(crate) fn project_work_item_plan_drafts_for_compile(
        &self,
        previous_plan: &IssueWorkItemPlan,
        draft_records: &[WorkItemDraftRecord],
        context: WorkItemPlanCompileProjectionContext<'_>,
    ) -> Result<
        (
            IssueWorkItemPlan,
            Vec<LifecycleWorkItemRecord>,
            Vec<VerificationPlan>,
        ),
        String,
    > {
        let outline_order = context.outline_order;
        let outline_to_work_item_id = context.outline_to_work_item_id;
        let outline_to_verification_plan_id = context.outline_to_verification_plan_id;
        let repository_id = context.repository_id;
        let now = context.now;
        let draft_by_outline: HashMap<&str, &WorkItemDraftRecord> = draft_records
            .iter()
            .map(|record| (record.outline_id.as_str(), record))
            .collect();
        let mut work_items = Vec::with_capacity(outline_order.len());
        let mut verification_plans = Vec::with_capacity(outline_order.len());
        for (index, outline_id) in outline_order.iter().enumerate() {
            let record = draft_by_outline
                .get(outline_id.as_str())
                .ok_or_else(|| format!("accepted draft for outline `{outline_id}` missing"))?;
            let candidate = &record.candidate;
            let work_item_id = outline_to_work_item_id
                .get(outline_id)
                .cloned()
                .ok_or_else(|| format!("work item id for outline `{outline_id}` missing"))?;
            let verification_plan_id = outline_to_verification_plan_id
                .get(outline_id)
                .cloned()
                .ok_or_else(|| {
                    format!("verification plan id for outline `{outline_id}` missing")
                })?;
            let depends_on = candidate
                .depends_on_outline_ids
                .iter()
                .map(|dependency_outline_id| {
                    outline_to_work_item_id
                        .get(dependency_outline_id)
                        .cloned()
                        .ok_or_else(|| {
                            format!(
                                "dependency outline `{dependency_outline_id}` for `{outline_id}` missing"
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let required_handoff_from = candidate
                .required_handoff_from_outline_ids
                .iter()
                .map(|dependency_outline_id| {
                    outline_to_work_item_id
                        .get(dependency_outline_id)
                        .cloned()
                        .ok_or_else(|| {
                            format!(
                                "handoff outline `{dependency_outline_id}` for `{outline_id}` missing"
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            work_items.push(LifecycleWorkItemRecord {
                id: work_item_id.clone(),
                project_id: previous_plan.project_id.clone(),
                issue_id: previous_plan.issue_id.clone(),
                repository_id: repository_id.to_string(),
                story_spec_ids: previous_plan.source_story_spec_ids.clone(),
                design_spec_ids: previous_plan.source_design_spec_ids.clone(),
                title: candidate.title.clone(),
                plan_status: WorkItemPlanStatus::Confirmed,
                execution_status: crate::product::models::WorkItemStatus::Pending,
                worktree_path: None,
                work_item_set_id: Some(previous_plan.id.clone()),
                kind: candidate.kind.clone(),
                sequence_hint: Some((index + 1) as u32),
                depends_on,
                exclusive_write_scopes: candidate.exclusive_write_scopes.clone(),
                forbidden_write_scopes: candidate.forbidden_write_scopes.clone(),
                context_budget: crate::product::models::WorkItemContextBudget::default(),
                required_handoff_from,
                verification_plan_ref: Some(verification_plan_id.clone()),
                require_execution_plan_confirm: previous_plan
                    .options
                    .require_execution_plan_confirm,
                execution_plan_status:
                    crate::product::models::WorkItemExecutionPlanStatus::NotStarted,
                handoff_summary_ref: None,
                completion_commit: None,
                completion_diff_summary_ref: None,
                created_at: now.to_string(),
                updated_at: now.to_string(),
            });
            verification_plans.push(parse_compile_verification_plan(
                &candidate.verification_plan,
                verification_plan_id,
                previous_plan.project_id.clone(),
                previous_plan.issue_id.clone(),
                work_item_id,
                now.to_string(),
            ));
        }
        let work_item_ids: Vec<String> = outline_order
            .iter()
            .filter_map(|outline_id| outline_to_work_item_id.get(outline_id).cloned())
            .collect();
        let verification_plan_ids: Vec<String> = outline_order
            .iter()
            .filter_map(|outline_id| outline_to_verification_plan_id.get(outline_id).cloned())
            .collect();
        let dependency_graph = self
            .latest_work_item_plan_outline_candidate()?
            .outline
            .dependency_graph
            .iter()
            .map(|edge| {
                let from_work_item_id = outline_to_work_item_id
                    .get(&edge.from_outline_id)
                    .cloned()
                    .ok_or_else(|| {
                        format!("dependency from outline `{}` missing", edge.from_outline_id)
                    })?;
                let to_work_item_id = outline_to_work_item_id
                    .get(&edge.to_outline_id)
                    .cloned()
                    .ok_or_else(|| {
                        format!("dependency to outline `{}` missing", edge.to_outline_id)
                    })?;
                Ok(IssueWorkItemDependencyEdge {
                    from_work_item_id,
                    to_work_item_id,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut compiled_plan = previous_plan.clone();
        compiled_plan.status = crate::product::models::IssueWorkItemPlanStatus::Confirmed;
        compiled_plan.work_item_ids = work_item_ids;
        compiled_plan.verification_plan_ids = verification_plan_ids;
        compiled_plan.repository_profile_ref = None;
        compiled_plan.dependency_graph = dependency_graph;
        compiled_plan.validator_findings = Vec::new();
        compiled_plan.updated_at = now.to_string();
        Ok((compiled_plan, work_items, verification_plans))
    }
}
