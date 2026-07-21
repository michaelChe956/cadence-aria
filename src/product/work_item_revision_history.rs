use std::collections::BTreeSet;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{WorkspaceSessionRelation, WorkspaceType};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::workspace_ws_types::{
    ArtifactPayload, WorkItemHistoryEntryDto, WorkItemHistoryEntryKind, WorkItemRevisionHistoryDto,
};

pub(crate) struct AuthoritativeCodingRevisionHistory {
    pub(crate) history: WorkItemRevisionHistoryDto,
    pub(crate) plan_id: String,
    pub(crate) plan_revision_id: String,
    pub(crate) plan_session_ids: Vec<String>,
}

pub(crate) fn build_authoritative_coding_revision_history(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    current_child_session_id: Option<&str>,
) -> Result<AuthoritativeCodingRevisionHistory, ProductStoreError> {
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let binding = coding_store.get_plan_binding(attempt)?;
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    let lineage = revision_store.get_plan_lineage(
        &attempt.project_id,
        &attempt.issue_id,
        &binding.plan_id,
    )?;
    let units =
        coding_store.list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let mut runtime_entries = Vec::new();
    for unit in &units {
        for run in coding_store.list_coding_unit_runs(attempt, &unit.id)? {
            runtime_entries.push(WorkItemHistoryEntryDto {
                kind: WorkItemHistoryEntryKind::UnitRun,
                id: run.id,
                logical_work_item_id: unit.logical_work_item_id.clone(),
                related_revision_id: Some(run.work_item_revision_id),
                summary: format!("UnitRun #{} ({:?})", run.execution_no, run.status),
                created_at: run.created_at,
            });
        }
        for handoff in
            revision_store.list_handoff_revisions(&lineage, &unit.logical_work_item_id)?
        {
            runtime_entries.push(WorkItemHistoryEntryDto {
                kind: WorkItemHistoryEntryKind::HandoffRevision,
                id: handoff.id,
                logical_work_item_id: unit.logical_work_item_id.clone(),
                related_revision_id: Some(handoff.work_item_revision_id),
                summary: format!("Handoff at commit {}", handoff.commit_sha),
                created_at: handoff.created_at,
            });
        }
    }
    runtime_entries.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });

    let lifecycle = LifecycleStore::new(app_paths.clone());
    let mut repair_child_session_ids = lifecycle
        .list_session_links(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .filter(|link| link.relation == WorkspaceSessionRelation::PlanRepair)
        .map(|link| link.child_session_id)
        .collect::<BTreeSet<_>>();
    if let Some(session_id) = current_child_session_id {
        repair_child_session_ids.insert(session_id.to_string());
    }
    let mut sessions = lifecycle
        .list_workspace_sessions(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .filter(|session| {
            session.workspace_type == WorkspaceType::WorkItemPlan
                && session.entity_id == binding.plan_id
                && !repair_child_session_ids.contains(&session.id)
        })
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    let base_session = sessions.last().ok_or_else(|| ProductStoreError::NotFound {
        kind: "coding_revision_history_plan_session",
        id: binding.plan_id.clone(),
    })?;
    let base_history = lifecycle
        .list_artifact_versions(&base_session.id)?
        .into_iter()
        .rev()
        .find_map(|version| match version.payload {
            ArtifactPayload::WorkItemRevisionHistory { history } => Some(*history),
            _ => None,
        })
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "coding_revision_history_artifact",
            id: base_session.id.clone(),
        })?;

    Ok(AuthoritativeCodingRevisionHistory {
        history: base_history.merge_runtime_entries(runtime_entries),
        plan_id: binding.plan_id,
        plan_revision_id: binding.bound_plan_revision_id,
        plan_session_ids: vec![base_session.id.clone()],
    })
}
