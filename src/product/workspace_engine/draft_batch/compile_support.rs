use super::*;
use crate::product::logical_codebase::{
    IssueCodebaseSelectionStore, LogicalCodebaseFeature, LogicalCodebaseStore, LogicalRepositoryId,
};
use crate::product::repository_store::RepositoryStore;

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

    pub(crate) fn logical_work_item_plan_repository_targets(
        &self,
        lifecycle: &LifecycleStore,
        plan: &IssueWorkItemPlan,
    ) -> Result<Option<std::collections::BTreeMap<LogicalRepositoryId, String>>, String> {
        let paths = lifecycle.app_paths();
        let logical_store = LogicalCodebaseStore::new(paths.clone());
        let selection_store = IssueCodebaseSelectionStore::new(paths.clone());
        let manifest = logical_store
            .load_manifest(&plan.project_id)
            .map_err(|error| format!("load logical codebase manifest failed: {error}"))?;
        let selection = selection_store
            .load(&plan.project_id, &plan.issue_id)
            .map_err(|error| format!("load issue codebase selection failed: {error}"))?;
        let (manifest, _selection) = match (manifest, selection) {
            (None, None) => return Ok(None),
            (Some(manifest), Some(selection)) => (manifest, selection),
            _ => {
                return Err(
                    "work_item_target_missing: logical codebase manifest and issue selection must both exist"
                        .to_string(),
                );
            }
        };
        let resolution = selection_store
            .resolve_effective_members(&plan.project_id, &plan.issue_id, &manifest.member_ids)
            .map_err(|error| format!("resolve issue codebase selection failed: {error}"))?;
        if !resolution.invalid_member_ids.is_empty() {
            return Err(format!(
                "work_item_target_missing: issue codebase selection has invalid members: {:?}",
                resolution.invalid_member_ids
            ));
        }

        let repository_store = RepositoryStore::with_logical_codebase_feature(
            paths,
            LogicalCodebaseFeature::enabled(),
        );
        resolution
            .effective_member_ids
            .into_iter()
            .map(|target_repository_id| {
                repository_store
                    .resolve_logical_repository(&plan.project_id, target_repository_id)
                    .map(|(_, _, repository)| (target_repository_id, repository.id))
                    .map_err(|error| {
                        format!(
                            "work_item_target_missing: cannot resolve target repository `{target_repository_id:?}`: {error}"
                        )
                    })
            })
            .collect::<Result<_, _>>()
            .map(Some)
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
        let logical_targets = context.logical_targets;
        let now = context.now;
        let draft_by_outline: HashMap<&str, &WorkItemDraftRecord> = draft_records
            .iter()
            .map(|record| (record.outline_id.as_str(), record))
            .collect();
        let mut logical_to_outline_id = HashMap::new();
        for record in draft_records {
            if logical_to_outline_id
                .insert(
                    record.candidate.logical_work_item_id.as_str(),
                    record.outline_id.as_str(),
                )
                .is_some()
            {
                return Err(format!(
                    "duplicate logical work item identity `{}` during compile",
                    record.candidate.logical_work_item_id
                ));
            }
        }
        let mut work_items = Vec::with_capacity(outline_order.len());
        let mut verification_plans = Vec::with_capacity(outline_order.len());
        if let Some(logical_targets) = logical_targets {
            for outline_id in outline_order {
                let record = draft_by_outline
                    .get(outline_id.as_str())
                    .ok_or_else(|| format!("accepted draft for outline `{outline_id}` missing"))?;
                let target_repository_id = record.candidate.target_repository_id.ok_or_else(|| {
                    format!(
                        "work_item_target_missing: target_repository_id_missing for outline `{outline_id}`"
                    )
                })?;
                if !logical_targets.contains_key(&target_repository_id) {
                    return Err(format!(
                        "work_item_target_missing: target_repository_id_not_effective for outline `{outline_id}`"
                    ));
                }
            }
        }
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
            let (repository_id, target_repository_id) = match logical_targets {
                Some(logical_targets) => {
                    let target_repository_id = candidate.target_repository_id.expect(
                        "logical target prevalidation must ensure target_repository_id is present",
                    );
                    let repository_id = logical_targets.get(&target_repository_id).expect(
                        "logical target prevalidation must ensure target_repository_id is effective",
                    );
                    (repository_id.as_str(), Some(target_repository_id))
                }
                None => (context.repository_id, None),
            };
            let depends_on = candidate
                .canonical_contract_candidate
                .input_contracts
                .iter()
                .map(|input| {
                    let dependency_outline_id = logical_to_outline_id
                        .get(input.provider_logical_work_item_id.as_str())
                        .ok_or_else(|| {
                            format!(
                                "provider logical identity `{}` for `{outline_id}` missing",
                                input.provider_logical_work_item_id
                            )
                        })?;
                    outline_to_work_item_id
                        .get(*dependency_outline_id)
                        .cloned()
                        .ok_or_else(|| {
                            format!(
                                "dependency outline `{dependency_outline_id}` for `{outline_id}` missing"
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            work_items.push(LifecycleWorkItemRecord {
                id: work_item_id.clone(),
                project_id: previous_plan.project_id.clone(),
                issue_id: previous_plan.issue_id.clone(),
                repository_id: repository_id.to_string(),
                target_repository_id,
                story_spec_ids: previous_plan.source_story_spec_ids.clone(),
                design_spec_ids: previous_plan.source_design_spec_ids.clone(),
                title: candidate
                    .canonical_contract_candidate
                    .identity
                    .title
                    .clone(),
                plan_status: WorkItemPlanStatus::Confirmed,
                execution_status: crate::product::models::WorkItemStatus::Pending,
                worktree_path: None,
                work_item_set_id: Some(previous_plan.id.clone()),
                source_work_item_plan_id: Some(previous_plan.id.clone()),
                source_outline_id: Some(record.outline_id.clone()),
                source_draft_id: Some(record.draft_id.clone()),
                planned_implementation_context: None,
                kind: crate::product::work_item_split_engine::types::parse_work_item_kind(
                    &candidate.canonical_contract_candidate.identity.kind,
                ),
                sequence_hint: Some((index + 1) as u32),
                depends_on,
                exclusive_write_scopes: candidate
                    .canonical_contract_candidate
                    .write_policy
                    .exclusive_scopes
                    .clone(),
                forbidden_write_scopes: candidate
                    .canonical_contract_candidate
                    .write_policy
                    .forbidden_scopes
                    .clone(),
                context_budget: crate::product::models::WorkItemContextBudget::default(),
                verification_plan_ref: Some(verification_plan_id.clone()),
                require_execution_plan_confirm: previous_plan
                    .options
                    .require_execution_plan_confirm,
                execution_plan_status:
                    crate::product::models::WorkItemExecutionPlanStatus::NotStarted,
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
        let dependency_graph = work_items
            .iter()
            .flat_map(|work_item| {
                work_item
                    .depends_on
                    .iter()
                    .map(|dependency_id| IssueWorkItemDependencyEdge {
                        from_work_item_id: dependency_id.clone(),
                        to_work_item_id: work_item.id.clone(),
                    })
            })
            .collect();
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
