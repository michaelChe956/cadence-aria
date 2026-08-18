use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_repository::{
    SchemaV2GroupAttemptScopePolicy, is_schema_v2_group_attempt, resolve_coding_attempt_repository,
};
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
    if is_schema_v2_group_attempt(
        app_paths,
        attempt,
        SchemaV2GroupAttemptScopePolicy::TreatLineageAsGroup,
    )? {
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

pub(crate) fn repository_path_for_attempt(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<PathBuf, CodingWorkspaceEngineError> {
    let repository = resolve_coding_attempt_repository(
        app_paths,
        attempt,
        SchemaV2GroupAttemptScopePolicy::TreatLineageAsGroup,
    )?;
    Ok(attempt
        .target_snapshot
        .as_ref()
        .map_or(repository.path, |snapshot| snapshot.canonical_path.clone()))
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use uuid::Uuid;

    use super::*;
    use crate::product::coding_models::{
        AttemptTargetSnapshot, CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
    };
    use crate::product::lifecycle_store::CreateWorkItemInput;
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
        IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore,
        LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord,
        RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::models::{ProviderName, RepositoryRecord};
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    #[test]
    fn repository_path_legacy_single_repo_without_selection_uses_physical() {
        // 无 manifest、无 selection 的单仓 attempt → 物理 repository path，不因 selection 缺失报错。
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "legacy coding context".to_string(),
                description: None,
                multi_repo: false,
            })
            .unwrap();
        let repository_path = root.path().join("repository_0001");
        write_repository_projection(&paths, &repository_path, None, None);
        LifecycleStore::new(paths.clone())
            .create_work_item(CreateWorkItemInput {
                id: Some("work_item_0001".to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: "repository_0001".to_string(),
                title: "单仓工作项".to_string(),
                ..Default::default()
            })
            .unwrap();

        let path = repository_path_for_attempt(&paths, &attempt_fixture(None)).unwrap();

        assert!(path.ends_with("repository_0001"));
    }

    #[test]
    fn repository_path_group_attempt_with_manifest_fail_closed_when_snapshot_inconsistent() {
        // B5：有 manifest 的快照 path 与 checkout 不一致时必须 fail-closed，不回退物理仓库。
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "logical coding context".to_string(),
                description: None,
                multi_repo: true,
            })
            .unwrap();
        let (logical_repository_id, checkout_id, canonical_path, git_dir_identity) =
            write_logical_authority_fixture(&paths, root.path());
        let attempt = attempt_fixture(Some(AttemptTargetSnapshot {
            logical_repository_id,
            checkout_id,
            physical_repository_id: "repository_0001".to_string(),
            canonical_path: canonical_path.join("stale"),
            git_dir_identity,
            revision: Some("abcdef".to_string()),
            policy_digest: String::new(),
            membership_revision: 1,
            captured_at: "2026-08-11T00:00:00Z".to_string(),
            capture_source: "test".to_string(),
        }));

        let result = repository_path_for_attempt(&paths, &attempt);

        assert!(result.is_err());
        assert!(
            result
                .expect_err("不一致快照必须被拒绝")
                .to_string()
                .contains("repository_routing_inconsistent")
        );
    }

    fn attempt_fixture(target_snapshot: Option<AttemptTargetSnapshot>) -> CodingExecutionAttempt {
        CodingExecutionAttempt {
            id: "coding_attempt_0001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            attempt_no: 1,
            scope: CodingAttemptScope::WorkItemGroup,
            status: CodingAttemptStatus::Running,
            version: 0,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            stage: CodingExecutionStage::WorktreePrepare,
            base_branch: "main".to_string(),
            branch_name: "aria/attempt".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            rework_count: 0,
            max_auto_rework: 0,
            work_item_group_id: None,
            current_work_item_id: Some("work_item_0001".to_string()),
            active_unit_id: None,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: "2026-08-11T00:00:00Z".to_string(),
            updated_at: "2026-08-11T00:00:00Z".to_string(),
            target_snapshot,
            completed_at: None,
        }
    }

    fn write_logical_authority_fixture(
        paths: &ProductAppPaths,
        root: &Path,
    ) -> (LogicalRepositoryId, RepositoryCheckoutId, PathBuf, String) {
        let logical_repository_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let canonical_path = root.join("repository_0001");
        let source_identity = RepositorySourceIdentity::from_git_parts(
            &canonical_path,
            canonical_path.join(".git"),
            None,
        );
        let authority = LogicalCodebaseStore::new(paths.clone());
        authority
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new(
                    "project_0001",
                    root.join("aggregate-root"),
                    vec![logical_repository_id],
                ),
            )
            .unwrap();
        authority
            .save_member(
                "project_0001",
                &CodebaseMemberRecord {
                    logical_repository_id,
                    physical_repository_id: "repository_0001".to_string(),
                    alias: "repository_0001".to_string(),
                    role: "repository".to_string(),
                    ordinal: 1,
                    source_identity: source_identity.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: "2026-08-11T00:00:00Z".to_string(),
                    updated_at: "2026-08-11T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        authority
            .save_checkout(
                "project_0001",
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id,
                    physical_repository_id: "repository_0001".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical_path.clone(),
                    checkout_path_hash: "sha256:checkout".to_string(),
                    git_dir_identity: source_identity.git_dir_identity(),
                    revision: Some("abcdef".to_string()),
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-08-11T00:00:00Z".to_string(),
                    created_at: "2026-08-11T00:00:00Z".to_string(),
                    updated_at: "2026-08-11T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        IssueCodebaseSelectionStore::new(paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![logical_repository_id],
                Vec::new(),
                vec![logical_repository_id],
                None,
            ))
            .unwrap();
        write_repository_projection(
            paths,
            &canonical_path,
            Some(logical_repository_id),
            Some(checkout_id),
        );
        (
            logical_repository_id,
            checkout_id,
            canonical_path,
            source_identity.git_dir_identity(),
        )
    }

    fn write_repository_projection(
        paths: &ProductAppPaths,
        repository_path: &Path,
        logical_repository_id: Option<LogicalRepositoryId>,
        checkout_id: Option<RepositoryCheckoutId>,
    ) {
        crate::product::json_store::write_json(
            &paths.project_root("project_0001").join("repos.json"),
            &[RepositoryRecord {
                id: "repository_0001".to_string(),
                project_id: "project_0001".to_string(),
                name: "repository_0001".to_string(),
                path: repository_path.to_path_buf(),
                repo_hash: "sha256:repository".to_string(),
                runtime_root: repository_path.join(".aria/runtime"),
                default_policy_preset: "manual-write".to_string(),
                default_provider_mode: "fake".to_string(),
                created_at: "2026-08-11T00:00:00Z".to_string(),
                logical_repository_id,
                primary_checkout_id: checkout_id,
                identity_schema_version: 1,
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            }],
        )
        .unwrap();
    }
}
