use super::*;

const OUTLINE_REVISION_JOURNAL_FILE: &str = "work_item_plan_outline_revision.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OutlineRevisionJournalPhase {
    Prepared,
    Committed,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct OutlineRevisionTransactionJournal {
    phase: OutlineRevisionJournalPhase,
    pub(super) session_id: String,
    pub(super) project_id: String,
    pub(super) issue_id: String,
    pub(super) plan_id: String,
    pub(super) target_run_node_id: String,
    pub(super) revision_feedback: Option<String>,
    pub(super) original_status: WorkspaceSessionStatus,
    pub(super) original_artifact_versions: Vec<ArtifactVersion>,
    pub(super) original_timeline_nodes: Vec<TimelineNode>,
    pub(super) source_node_id: Option<String>,
    pub(super) original_source_detail: Option<NodeDetail>,
    pub(super) original_active_index: Option<WorkItemPlanDraftActiveIndex>,
    pub(super) original_drafts: Vec<WorkItemDraftRecord>,
}

impl OutlineRevisionTransactionJournal {
    pub(super) fn prepared(
        engine: &WorkspaceEngine,
        snapshot: &OutlineRevisionPersistenceSnapshot,
        plan_mutation: Option<&OutlineRevisingMutation>,
        revision_feedback: Option<String>,
    ) -> Self {
        Self {
            phase: OutlineRevisionJournalPhase::Prepared,
            session_id: engine.session.session_id.clone(),
            project_id: engine.session.project_id.clone(),
            issue_id: engine.session.issue_id.clone(),
            plan_id: engine.session.entity_id.clone(),
            target_run_node_id: snapshot.run_node.node_id.clone(),
            revision_feedback,
            original_status: snapshot.original_status.clone(),
            original_artifact_versions: snapshot.original_artifact_versions.clone(),
            original_timeline_nodes: snapshot.original_timeline_nodes.clone(),
            source_node_id: snapshot
                .source_node
                .as_ref()
                .map(|node| node.node_id.clone()),
            original_source_detail: snapshot
                .source_node
                .as_ref()
                .and_then(|node| node.original_detail.clone()),
            original_active_index: plan_mutation.map(|mutation| mutation.original_index.clone()),
            original_drafts: plan_mutation
                .map(|mutation| {
                    mutation
                        .drafts
                        .iter()
                        .map(|draft| draft.original.clone())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    pub(super) fn mark_committed(&mut self) {
        self.phase = OutlineRevisionJournalPhase::Committed;
    }
}

pub(super) enum OutlineRevisionPersistenceFailure {
    Error(String),
    SimulatedCrash(OutlineRevisionCrashPoint),
}

fn outline_revision_journal_path(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    session_id: &str,
) -> PathBuf {
    lifecycle
        .app_paths()
        .issue_lifecycle_root(project_id, issue_id)
        .join("workspace-transactions")
        .join(session_id)
        .join(OUTLINE_REVISION_JOURNAL_FILE)
}

pub(super) fn save_outline_revision_journal(
    lifecycle: &LifecycleStore,
    journal: &OutlineRevisionTransactionJournal,
) -> Result<(), String> {
    crate::product::json_store::write_json(
        &outline_revision_journal_path(
            lifecycle,
            &journal.project_id,
            &journal.issue_id,
            &journal.session_id,
        ),
        journal,
    )
    .map_err(|error| format!("save outline revision transaction journal failed: {error}"))
}

pub(super) fn delete_outline_revision_journal(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    session_id: &str,
) -> Result<(), String> {
    let path = outline_revision_journal_path(lifecycle, project_id, issue_id, session_id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "delete outline revision transaction journal {} failed: {error}",
            path.display()
        )),
    }
}

pub(super) fn rollback_outline_revision_journal(
    lifecycle: &LifecycleStore,
    journal: &OutlineRevisionTransactionJournal,
) -> Vec<String> {
    let mut rollback_errors = Vec::new();
    let store = WorkItemPlanStore::new(lifecycle.app_paths());
    for draft in &journal.original_drafts {
        if let Err(error) = store.put_draft_record(draft) {
            rollback_errors.push(format!(
                "restore work item plan draft {} failed: {error}",
                draft.draft_id
            ));
        }
    }
    let active_index_result = match &journal.original_active_index {
        Some(index) => store.save_active_index(index),
        None => store.delete_active_index(&journal.project_id, &journal.issue_id, &journal.plan_id),
    };
    if let Err(error) = active_index_result {
        rollback_errors.push(format!(
            "restore original work item plan active index failed: {error}"
        ));
    }
    if let Err(error) =
        lifecycle.delete_node_detail(&journal.session_id, &journal.target_run_node_id)
    {
        rollback_errors.push(format!(
            "delete outline revision run node detail failed: {error}"
        ));
    }
    if let Some(source_node_id) = &journal.source_node_id {
        let result = match &journal.original_source_detail {
            Some(detail) => lifecycle.save_node_detail(&journal.session_id, source_node_id, detail),
            None => lifecycle.delete_node_detail(&journal.session_id, source_node_id),
        };
        if let Err(error) = result {
            rollback_errors.push(format!(
                "restore outline revision node detail failed: {error}"
            ));
        }
    }
    if let Err(error) =
        lifecycle.save_timeline_nodes(&journal.session_id, &journal.original_timeline_nodes)
    {
        rollback_errors.push(format!("restore outline revision timeline failed: {error}"));
    }
    if let Err(error) =
        lifecycle.save_artifact_versions(&journal.session_id, &journal.original_artifact_versions)
    {
        rollback_errors.push(format!(
            "restore outline revision artifact versions failed: {error}"
        ));
    }
    if let Err(error) = lifecycle
        .update_workspace_session_status(&journal.session_id, journal.original_status.clone())
    {
        rollback_errors.push(format!(
            "restore outline revision workspace session status failed: {error}"
        ));
    }
    rollback_errors
}

pub(crate) fn recover_work_item_plan_outline_revision_transaction(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    session_id: &str,
) -> Result<(), String> {
    let path = outline_revision_journal_path(lifecycle, project_id, issue_id, session_id);
    if !path
        .try_exists()
        .map_err(|error| format!("inspect outline revision transaction journal failed: {error}"))?
    {
        return Ok(());
    }
    let journal: OutlineRevisionTransactionJournal = crate::product::json_store::read_json(&path)
        .map_err(|error| {
        format!("load outline revision transaction journal failed: {error}")
    })?;
    if journal.project_id != project_id
        || journal.issue_id != issue_id
        || journal.plan_id != plan_id
        || journal.session_id != session_id
    {
        return Err("outline revision transaction journal identity mismatch".to_string());
    }
    if journal.phase == OutlineRevisionJournalPhase::Prepared {
        let rollback_errors = rollback_outline_revision_journal(lifecycle, &journal);
        if !rollback_errors.is_empty() {
            return Err(combine_outline_revision_rollback_errors(
                "recover prepared outline revision transaction failed".to_string(),
                rollback_errors,
            ));
        }
    }
    delete_outline_revision_journal(lifecycle, project_id, issue_id, session_id)
}

pub(super) fn maybe_simulate_outline_revision_crash(
    configured: Option<OutlineRevisionCrashPoint>,
    current: OutlineRevisionCrashPoint,
) -> Result<(), OutlineRevisionPersistenceFailure> {
    if configured == Some(current) {
        Err(OutlineRevisionPersistenceFailure::SimulatedCrash(current))
    } else {
        Ok(())
    }
}

pub(super) fn persist_outline_revision_plan_mutation(
    mutation: OutlineRevisingMutation,
    crash_after: Option<OutlineRevisionCrashPoint>,
) -> Result<(), OutlineRevisionPersistenceFailure> {
    for (index, draft) in mutation.drafts.iter().enumerate() {
        mutation
            .store
            .put_draft_record(&draft.revised)
            .map_err(|error| {
                OutlineRevisionPersistenceFailure::Error(format!(
                    "save superseded outline revision draft failed: {error}"
                ))
            })?;
        if index == 0 {
            maybe_simulate_outline_revision_crash(
                crash_after,
                OutlineRevisionCrashPoint::PlanDrafts,
            )?;
        }
    }
    mutation
        .store
        .save_active_index(&mutation.revised_index)
        .map_err(|error| {
            OutlineRevisionPersistenceFailure::Error(format!(
                "save work item plan active index failed: {error}"
            ))
        })?;
    maybe_simulate_outline_revision_crash(crash_after, OutlineRevisionCrashPoint::ActiveIndex)
}
