use super::*;

struct OutlineRevisingDraftMutation {
    original: WorkItemDraftRecord,
    revised: WorkItemDraftRecord,
}

pub(crate) struct OutlineRevisingMutation {
    store: WorkItemPlanStore,
    original_index: WorkItemPlanDraftActiveIndex,
    revised_index: WorkItemPlanDraftActiveIndex,
    drafts: Vec<OutlineRevisingDraftMutation>,
}

impl OutlineRevisingMutation {
    pub(crate) fn persist(self) -> Result<(), String> {
        let mut written_originals = Vec::new();
        for draft in &self.drafts {
            if let Err(error) = self.store.put_draft_record(&draft.revised) {
                let mut rollback_errors = restore_outline_revision_drafts(
                    &self.store,
                    written_originals.into_iter().rev(),
                );
                if let Err(rollback_error) = self.store.save_active_index(&self.original_index) {
                    rollback_errors.push(format!(
                        "restore original work item plan active index failed: {rollback_error}"
                    ));
                }
                return Err(combine_outline_revision_rollback_errors(
                    format!("save superseded outline revision draft failed: {error}"),
                    rollback_errors,
                ));
            }
            written_originals.push(draft.original.clone());
        }

        if let Err(error) = self.store.save_active_index(&self.revised_index) {
            let mut rollback_errors =
                restore_outline_revision_drafts(&self.store, written_originals.into_iter().rev());
            if let Err(rollback_error) = self.store.save_active_index(&self.original_index) {
                rollback_errors.push(format!(
                    "restore original work item plan active index failed: {rollback_error}"
                ));
            }
            return Err(combine_outline_revision_rollback_errors(
                format!("save work item plan active index failed: {error}"),
                rollback_errors,
            ));
        }
        Ok(())
    }
}

fn restore_outline_revision_drafts(
    store: &WorkItemPlanStore,
    originals: impl Iterator<Item = WorkItemDraftRecord>,
) -> Vec<String> {
    originals
        .filter_map(|original| {
            store.put_draft_record(&original).err().map(|error| {
                format!(
                    "restore work item plan draft {} failed: {error}",
                    original.draft_id
                )
            })
        })
        .collect()
}

pub(crate) fn combine_outline_revision_rollback_errors(
    primary_error: String,
    rollback_errors: Vec<String>,
) -> String {
    if rollback_errors.is_empty() {
        primary_error
    } else {
        format!(
            "{primary_error}; rollback outline revision persistence failed: {}",
            rollback_errors.join(" | ")
        )
    }
}

mod authoring;
mod dto;
mod revision;

pub(crate) use dto::*;
pub(crate) use revision::*;

pub(crate) fn work_item_plan_outline_topological_order(
    outline: &WorkItemPlanOutline,
) -> Result<Vec<String>, String> {
    let outline_ids: Vec<String> = outline
        .work_item_outlines
        .iter()
        .map(|item| item.outline_id.clone())
        .collect();
    let known_ids: HashSet<String> = outline_ids.iter().cloned().collect();
    let mut indegree: HashMap<String, usize> = outline_ids
        .iter()
        .map(|outline_id| (outline_id.clone(), 0))
        .collect();
    let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();

    for item in &outline.work_item_outlines {
        for dependency in &item.depends_on {
            if !known_ids.contains(dependency) {
                return Err(format!(
                    "outline `{}` depends_on missing outline_id `{}`",
                    item.outline_id, dependency
                ));
            }
            *indegree.entry(item.outline_id.clone()).or_default() += 1;
            outgoing
                .entry(dependency.clone())
                .or_default()
                .push(item.outline_id.clone());
        }
    }

    for edge in &outline.dependency_graph {
        if !known_ids.contains(&edge.from_outline_id) {
            return Err(format!(
                "dependency edge references missing from_outline_id `{}`",
                edge.from_outline_id
            ));
        }
        if !known_ids.contains(&edge.to_outline_id) {
            return Err(format!(
                "dependency edge references missing to_outline_id `{}`",
                edge.to_outline_id
            ));
        }
    }

    let mut queue: VecDeque<String> = outline_ids
        .iter()
        .filter(|outline_id| indegree.get(*outline_id).copied().unwrap_or_default() == 0)
        .cloned()
        .collect();
    let mut order = Vec::with_capacity(outline_ids.len());

    while let Some(outline_id) = queue.pop_front() {
        order.push(outline_id.clone());
        if let Some(next_ids) = outgoing.get(&outline_id) {
            for next_id in next_ids {
                let Some(count) = indegree.get_mut(next_id) else {
                    continue;
                };
                *count -= 1;
                if *count == 0 {
                    queue.push_back(next_id.clone());
                }
            }
        }
    }

    if order.len() != outline_ids.len() {
        return Err("outline depends_on dependencies contain a cycle".to_string());
    }

    Ok(order)
}

pub(crate) fn current_work_item_batch(
    index: &WorkItemPlanDraftActiveIndex,
) -> Result<&WorkItemBatchRecord, String> {
    index
        .batches
        .iter()
        .rev()
        .find(|batch| {
            batch.generation_round_id == index.current_generation_round_id
                && batch.mode == WorkItemGenerationMode::Batch
                && batch.status == WorkItemBatchStatus::Generating
        })
        .or_else(|| {
            index.batches.iter().rev().find(|batch| {
                batch.generation_round_id == index.current_generation_round_id
                    && batch.mode == WorkItemGenerationMode::Batch
            })
        })
        .ok_or_else(|| "current work item batch record is missing".to_string())
}

pub(crate) fn work_item_plan_findings_summary(
    prefix: &str,
    findings: &[WorkItemSplitFinding],
) -> String {
    let errors = findings
        .iter()
        .filter(|finding| finding.severity == WorkItemSplitFindingSeverity::Error)
        .count();
    let warnings = findings
        .iter()
        .filter(|finding| finding.severity == WorkItemSplitFindingSeverity::Warning)
        .count();
    format!("{prefix}（errors: {errors}, warnings: {warnings}）")
}

pub(crate) fn work_item_plan_outline_terminal_failure_summary(
    findings: &[WorkItemSplitFinding],
) -> String {
    let mut lines = vec![format!(
        "{}，已停止继续生成。",
        work_item_plan_findings_summary("Outline 自动重跑后仍校验失败", findings)
    )];
    let error_findings = findings
        .iter()
        .filter(|finding| finding.severity == WorkItemSplitFindingSeverity::Error)
        .collect::<Vec<_>>();
    if !error_findings.is_empty() {
        lines.push("主要问题：".to_string());
        for finding in error_findings.iter().take(5) {
            lines.push(format!("- {} - {}", finding.code, finding.message));
        }
        if error_findings.len() > 5 {
            lines.push(format!("- 其余 {} 个错误已省略", error_findings.len() - 5));
        }
    }
    lines.push("请终止当前流程并重新创建 Work Item Plan。".to_string());
    lines.join("\n")
}

pub(crate) fn work_item_draft_status_label(status: &WorkItemDraftStatus) -> &'static str {
    match status {
        WorkItemDraftStatus::Draft => "draft",
        WorkItemDraftStatus::Accepted => "accepted",
        WorkItemDraftStatus::Superseded => "superseded",
        WorkItemDraftStatus::ValidationFailed => "validation_failed",
    }
}

impl WorkspaceEngine {
    pub(crate) fn current_work_item_plan_outline_candidate(
        &self,
    ) -> Result<&WorkItemPlanOutlineCandidateDto, String> {
        match self.session.artifact.as_ref() {
            Some(ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate }) => {
                Ok(outline_candidate)
            }
            _ => Err("current WorkItemPlan Outline artifact is unavailable".to_string()),
        }
    }

    pub(crate) fn latest_work_item_plan_outline_candidate(
        &self,
    ) -> Result<WorkItemPlanOutlineCandidateDto, String> {
        if let Some(ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate }) =
            self.session.artifact.as_ref()
        {
            return Ok(outline_candidate.as_ref().clone());
        }

        self.artifact_versions
            .iter()
            .rev()
            .find_map(|version| match &version.payload {
                ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate } => {
                    Some(outline_candidate.as_ref().clone())
                }
                _ => None,
            })
            .ok_or_else(|| "latest WorkItemPlan Outline artifact is unavailable".to_string())
    }

    pub(crate) fn current_work_item_plan_outline_ids(&self) -> Vec<String> {
        self.latest_work_item_plan_outline_candidate()
            .map(|candidate| {
                candidate
                    .outline
                    .work_item_outlines
                    .iter()
                    .map(|outline| outline.outline_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn work_item_plan_store(&self) -> Result<WorkItemPlanStore, String> {
        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        Ok(WorkItemPlanStore::new(lifecycle.app_paths()))
    }

    pub(crate) fn save_confirmed_work_item_plan_outline_index(&self) -> Result<String, String> {
        self.current_work_item_plan_outline_candidate()?;
        let store = self.work_item_plan_store()?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let current = store
            .load_active_index(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load work item plan active index failed: {error}"))?;
        let generation_round_id = current
            .as_ref()
            .map(next_generation_round_id)
            .unwrap_or_else(|| "round_001".to_string());
        let now = chrono::Utc::now().to_rfc3339();
        let index = WorkItemPlanDraftActiveIndex {
            project_id,
            issue_id,
            plan_id,
            current_generation_round_id: generation_round_id.clone(),
            outline_state: "confirmed".to_string(),
            active_outline_id: None,
            outline_to_current_draft_id: BTreeMap::new(),
            draft_statuses: BTreeMap::new(),
            batches: Vec::new(),
            updated_at: now,
        };
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;
        Ok(generation_round_id)
    }

    pub(crate) fn mark_work_item_plan_outline_revising(&self) -> Result<(), String> {
        self.persist_work_item_plan_outline_revising(
            OutlineRevisionPersistencePolicy::RequireActiveRound,
        )
    }

    fn persist_work_item_plan_outline_revising(
        &self,
        policy: OutlineRevisionPersistencePolicy,
    ) -> Result<(), String> {
        if let Some(mutation) = self.prepare_work_item_plan_outline_revising(policy)? {
            mutation.persist()?;
        }
        Ok(())
    }

    pub(crate) fn prepare_work_item_plan_outline_revising(
        &self,
        policy: OutlineRevisionPersistencePolicy,
    ) -> Result<Option<OutlineRevisingMutation>, String> {
        let store = self.work_item_plan_store()?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let index = store
            .load_active_index(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load work item plan active index failed: {error}"))?;
        let mut index = match index {
            Some(index) => index,
            None if policy == OutlineRevisionPersistencePolicy::AllowMissingInitialRound => {
                return Ok(None);
            }
            None => return Err("work item plan active index missing".to_string()),
        };
        let original_index = index.clone();
        let now = chrono::Utc::now().to_rfc3339();
        let drafts = self.prepare_outline_revision_draft_mutations(&store, &mut index, &now)?;
        index.outline_state = "revising".to_string();
        index.active_outline_id = None;
        index.updated_at = now;
        Ok(Some(OutlineRevisingMutation {
            store,
            original_index,
            revised_index: index,
            drafts,
        }))
    }

    fn prepare_outline_revision_draft_mutations(
        &self,
        store: &WorkItemPlanStore,
        index: &mut WorkItemPlanDraftActiveIndex,
        now: &str,
    ) -> Result<Vec<OutlineRevisingDraftMutation>, String> {
        let draft_ids: Vec<String> = index
            .draft_statuses
            .iter()
            .filter_map(|(draft_id, status)| {
                if status == &WorkItemDraftStatus::Superseded {
                    None
                } else {
                    Some(draft_id.clone())
                }
            })
            .collect();

        let mut drafts = Vec::with_capacity(draft_ids.len());
        for draft_id in draft_ids {
            let original = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    &draft_id,
                )
                .map_err(|error| format!("load draft for outline revision failed: {error}"))?;
            let mut revised = original.clone();
            mark_draft_record_superseded(
                &mut revised,
                None,
                WorkItemDraftSupersedeReason::OutlineRevised,
                now,
            );
            index
                .draft_statuses
                .insert(draft_id.clone(), WorkItemDraftStatus::Superseded);
            drafts.push(OutlineRevisingDraftMutation { original, revised });
        }

        index.outline_to_current_draft_id.clear();
        Ok(drafts)
    }

    pub(crate) fn set_active_work_item_plan_outline(&self, outline_id: &str) -> Result<(), String> {
        let store = self.work_item_plan_store()?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let mut index = store
            .load_active_index(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        index.active_outline_id = Some(outline_id.to_string());
        index.updated_at = chrono::Utc::now().to_rfc3339();
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))
    }

    pub(crate) async fn update_work_item_plan_outline_generation_metadata(
        &mut self,
        generation_round_id: Option<String>,
        selected_mode: Option<WorkItemGenerationModeDto>,
    ) -> Result<(), String> {
        let Some(ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate }) =
            self.session.artifact.clone()
        else {
            return Err("current WorkItemPlan Outline artifact is unavailable".to_string());
        };
        let mut outline_candidate = *outline_candidate;
        if generation_round_id.is_some() {
            outline_candidate.current_generation_round_id = generation_round_id;
        }
        outline_candidate.selected_generation_mode = selected_mode;
        self.replace_current_artifact_payload(ArtifactPayload::WorkItemPlanOutlineCandidate {
            outline_candidate: Box::new(outline_candidate),
        })
        .await?;
        Ok(())
    }
}
