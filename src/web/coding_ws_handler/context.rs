use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAgentRole, CodingChatEntry, CodingContextNote, CodingEntryType, CodingExecutionAttempt,
    CodingExecutionStage, CodingProviderPermissionMode, CodingProviderRole, CodingStageGateState,
    CodingStageGateStatus,
};
use crate::product::coding_work_item_context::load_coding_work_item_context;
use crate::product::coding_workspace_engine::{
    CodingExecutionContext, CodingWorkspaceEngineError,
    normalize_coding_permission_mode_for_provider,
};
use crate::product::coding_workspace_runner::{
    apply_provider_selection_to_snapshots, coding_provider_role_for_stage,
    parse_coding_provider_role,
};
use crate::product::json_store::ProductStoreError;

use super::active_coding_timeline_node_id;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{ProviderName, WorkItemExecutionPlanStatus};
use crate::product::repository_store::RepositoryStore;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

pub(crate) fn current_work_item_id_for_attempt(attempt: &CodingExecutionAttempt) -> &str {
    attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id)
}

pub(crate) fn coding_execution_context(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<CodingExecutionContext, ProductStoreError> {
    let context = load_coding_work_item_context(app_paths, attempt)?;

    Ok(CodingExecutionContext {
        work_item_markdown: context.markdown,
        verification_commands: context.verification_commands,
    })
}

pub(crate) fn ensure_work_item_execution_plan_confirmed(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<(), CodingWorkspaceEngineError> {
    if is_schema_v2_group_attempt(app_paths, attempt)? {
        return Ok(());
    }
    let current_work_item_id = current_work_item_id_for_attempt(attempt);
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let work_items = lifecycle.list_work_items(&attempt.project_id, &attempt.issue_id)?;
    let Some(work_item) = work_items
        .iter()
        .find(|item| item.id == current_work_item_id)
    else {
        return Ok(());
    };
    if !work_item.require_execution_plan_confirm {
        return Ok(());
    }

    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let plan = coding_store.get_work_item_execution_plan(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    match plan.map(|p| p.status) {
        Some(WorkItemExecutionPlanStatus::Confirmed) => Ok(()),
        _ => Err(CodingWorkspaceEngineError::ExecutionPlanNotConfirmed(
            attempt.id.clone(),
        )),
    }
}

fn is_schema_v2_group_attempt(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<bool, ProductStoreError> {
    let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
        return Ok(false);
    };
    match WorkItemRevisionStore::new(app_paths.clone()).get_plan_lineage(
        &attempt.project_id,
        &attempt.issue_id,
        plan_id,
    ) {
        Ok(_) => Ok(true),
        Err(ProductStoreError::NotFound {
            kind: "work_item_plan_lineage",
            ..
        }) => Ok(false),
        Err(error) => Err(error),
    }
}

pub(crate) fn repository_path_for_attempt(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<PathBuf, CodingWorkspaceEngineError> {
    let repository_store = RepositoryStore::new(app_paths.clone());
    let (_, checkout, _) = if let Some(snapshot) = attempt.target_snapshot.as_ref() {
        repository_store
            .resolve_logical_repository(&attempt.project_id, snapshot.logical_repository_id)?
    } else if is_schema_v2_group_attempt(app_paths, attempt)? {
        let logical_repository_id = issue_selection_logical_repository_id(app_paths, attempt)?;
        repository_store.resolve_logical_repository(&attempt.project_id, logical_repository_id)?
    } else {
        let current_work_item_id = current_work_item_id_for_attempt(attempt);
        let work_item = LifecycleStore::new(app_paths.clone())
            .list_work_items(&attempt.project_id, &attempt.issue_id)?
            .into_iter()
            .find(|work_item| work_item.id == current_work_item_id)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "work_item",
                id: current_work_item_id.to_string(),
            })?;
        match work_item.target_repository_id {
            Some(logical_repository_id) => repository_store
                .resolve_logical_repository(&attempt.project_id, logical_repository_id)?,
            None => repository_store.resolve_legacy_physical_repository_if_dual(
                &attempt.project_id,
                &work_item.repository_id,
            )?,
        }
    };
    Ok(checkout.canonical_path)
}

fn issue_selection_logical_repository_id(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<crate::product::logical_codebase::LogicalRepositoryId, ProductStoreError> {
    #[derive(serde::Deserialize)]
    struct IssueCodebaseSelection {
        focus: Vec<crate::product::logical_codebase::LogicalRepositoryId>,
    }

    let selection: IssueCodebaseSelection = crate::product::json_store::read_json(
        &app_paths
            .issue_root(&attempt.project_id, &attempt.issue_id)
            .join("codebase-selection.json"),
    )?;
    let [logical_repository_id] = selection.focus.as_slice() else {
        return Err(ProductStoreError::Ambiguous {
            kind: "issue_codebase_selection",
            id: attempt.issue_id.clone(),
        });
    };
    Ok(*logical_repository_id)
}

pub(crate) fn update_provider_selection(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role: &str,
    provider: ProviderName,
) -> Result<(CodingExecutionAttempt, CodingProviderRole, ProviderName), ProductStoreError> {
    let mut snapshot = attempt.provider_config_snapshot.clone();
    let mut role_snapshot = coding_store.get_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let changed_provider = provider.clone();
    let changed_role =
        apply_provider_selection_to_snapshots(role, provider, &mut snapshot, &mut role_snapshot)
            .map_err(ProductStoreError::Io)?;
    let role_provider = role_snapshot.provider_for_role(&changed_role).clone();
    let permission_mode = role_snapshot.permission_mode_for_role(&changed_role);
    role_snapshot.set_permission_mode_for_role(
        &changed_role,
        normalize_coding_permission_mode_for_provider(&role_provider, permission_mode),
    );
    let updated = coding_store.update_attempt_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        snapshot,
    )?;
    coding_store.update_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        role_snapshot,
    )?;
    Ok((updated, changed_role, changed_provider))
}

pub(crate) fn update_provider_permission_mode(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role: &str,
    permission_mode: CodingProviderPermissionMode,
) -> Result<(CodingProviderRole, ProviderName), ProductStoreError> {
    let parsed_role = parse_coding_provider_role(role)
        .ok_or_else(|| ProductStoreError::Io(format!("unknown coding role: {role}")))?;
    let mut role_snapshot = coding_store.get_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let provider = role_snapshot.provider_for_role(&parsed_role).clone();
    role_snapshot.set_permission_mode_for_role(
        &parsed_role,
        normalize_coding_permission_mode_for_provider(&provider, permission_mode),
    );
    coding_store.update_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        role_snapshot,
    )?;
    Ok((parsed_role, provider))
}

pub(crate) fn provider_selection_targets_current_running_stage(
    attempt: &CodingExecutionAttempt,
    role: &str,
) -> bool {
    if attempt.status != crate::product::coding_models::CodingAttemptStatus::Running {
        return false;
    }
    let Some(current_role) = coding_provider_role_for_stage(&attempt.stage) else {
        return false;
    };
    parse_coding_provider_role(role).as_ref() == Some(&current_role)
}

pub(crate) fn confirm_open_stage_gate(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    stage: &CodingExecutionStage,
) -> Result<Option<CodingStageGateState>, ProductStoreError> {
    let Some(gate) = coding_store
        .list_open_stage_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .find(|gate| gate.stage == *stage)
    else {
        return Ok(None);
    };
    coding_store
        .update_stage_gate_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            CodingStageGateStatus::Confirmed,
        )
        .map(Some)
}

pub(crate) fn context_note_chat_entry(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    note: CodingContextNote,
) -> Result<CodingChatEntry, ProductStoreError> {
    let timeline_nodes =
        coding_store.get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    Ok(CodingChatEntry {
        id: chat_entry_id_for_context_note(&note.id),
        attempt_id: attempt.id.clone(),
        node_id: active_coding_timeline_node_id(&timeline_nodes),
        role: CodingAgentRole::Author,
        entry_type: CodingEntryType::UserMessage,
        content: Some(note.content),
        metadata: Some(serde_json::json!({
            "context_note_id": note.id,
        })),
        created_at: note.created_at,
    })
}

fn chat_entry_id_for_context_note(note_id: &str) -> String {
    note_id.replacen("coding_context_note", "coding_chat_entry", 1)
}
