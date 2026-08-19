use std::collections::BTreeSet;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{CodingAttemptScope, CodingExecutionAttempt};
use crate::product::issue_store::IssueStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::snapshot_validator::validate_snapshot_fields;
use crate::product::logical_codebase::{
    IssueCodebaseSelection, LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId,
    MemberStatus, RepositoryRouting, RepositoryRoutingErrorCode, SelectionPolicy,
};
use crate::product::models::RepositoryRecord;
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

/// 指定 schema-v2 lineage 与 attempt scope 不一致时的入口兼容策略。
///
/// HTTP 删除/repair 在 fix base 中仅将 `WorkItemGroup` scope 视为 group；workspace
/// context 则将存在的 lineage 直接视为 group。共享 resolver 接收该策略以原样保留
/// 两个入口的既有语义，而不把该状态转换成新的 routing 错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaV2GroupAttemptScopePolicy {
    RequireWorkItemGroupScope,
    TreatLineageAsGroup,
}

/// 以逻辑代码库权威状态解析 coding attempt 所属 repository。
///
/// 所有 coding 入口必须通过该函数执行三态 routing、snapshot 优先校验、
/// group target 唯一性和 legacy 物理回退，避免不同入口产生 fail-closed 语义漂移。
pub fn resolve_coding_attempt_repository(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    group_scope_policy: SchemaV2GroupAttemptScopePolicy,
) -> Result<RepositoryRecord, ProductStoreError> {
    let project = ProjectStore::new(app_paths.clone()).get(&attempt.project_id)?;
    let routing =
        RepositoryRouting::load_for_issue(app_paths, &attempt.project_id, &attempt.issue_id)?;
    let repository_store = RepositoryStore::for_project(app_paths.clone(), &project);

    if let Some(snapshot) = attempt.target_snapshot.as_ref() {
        let (manifest, selection) = match routing {
            RepositoryRouting::Logical {
                manifest,
                selection,
            } => (manifest, selection),
            RepositoryRouting::Legacy { .. } => {
                return Err(routing_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    "target snapshot has no logical codebase authority",
                ));
            }
            RepositoryRouting::FailClosed { code, reason } => {
                return Err(routing_error(code, reason));
            }
        };
        let selected_ids = validate_logical_selection(
            app_paths,
            &attempt.project_id,
            &attempt.issue_id,
            &manifest,
            &selection,
        )?;
        if !selected_ids.contains(&snapshot.logical_repository_id) {
            return Err(routing_error(
                RepositoryRoutingErrorCode::TargetUnknown,
                "target snapshot repository is not in the effective selection",
            ));
        }
        validate_snapshot_fields(app_paths, attempt).map_err(|code| {
            routing_error(
                code,
                "target snapshot does not match logical codebase authority",
            )
        })?;
        return resolve_logical_repository(
            &repository_store,
            &attempt.project_id,
            snapshot.logical_repository_id,
        );
    }

    match routing {
        RepositoryRouting::Legacy { .. } => {
            legacy_repository_for_attempt(&repository_store, app_paths, attempt)
        }
        RepositoryRouting::Logical {
            manifest,
            selection,
        } => {
            let selected_ids = validate_logical_selection(
                app_paths,
                &attempt.project_id,
                &attempt.issue_id,
                &manifest,
                &selection,
            )?;
            let logical_repository_id =
                if is_schema_v2_group_attempt(app_paths, attempt, group_scope_policy)? {
                    logical_repository_for_group_attempt(app_paths, attempt, &selection)?
                } else {
                    let current_work_item_id = current_work_item_id_for_attempt(attempt);
                    LifecycleStore::new(app_paths.clone())
                        .list_work_items(&attempt.project_id, &attempt.issue_id)?
                        .into_iter()
                        .find(|work_item| work_item.id == current_work_item_id)
                        .and_then(|work_item| work_item.target_repository_id)
                        .ok_or_else(|| {
                            routing_error(
                                RepositoryRoutingErrorCode::TargetMissing,
                                format!(
                                    "work item {current_work_item_id} has no target repository"
                                ),
                            )
                        })?
                };
            if !selected_ids.contains(&logical_repository_id) {
                return Err(routing_error(
                    RepositoryRoutingErrorCode::TargetUnknown,
                    "logical repository target is not in the effective selection",
                ));
            }
            resolve_logical_repository(
                &repository_store,
                &attempt.project_id,
                logical_repository_id,
            )
        }
        RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
    }
}

/// 检查 attempt 是否确实引用 schema-v2 group lineage。
///
/// `group_scope_policy` 由调用入口传入，以保持 fix base 的 scope 兼容行为。
pub fn is_schema_v2_group_attempt(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    group_scope_policy: SchemaV2GroupAttemptScopePolicy,
) -> Result<bool, ProductStoreError> {
    if group_scope_policy == SchemaV2GroupAttemptScopePolicy::RequireWorkItemGroupScope
        && attempt.scope != CodingAttemptScope::WorkItemGroup
    {
        return Ok(false);
    }
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

fn resolve_logical_repository(
    repository_store: &RepositoryStore,
    project_id: &str,
    logical_repository_id: LogicalRepositoryId,
) -> Result<RepositoryRecord, ProductStoreError> {
    repository_store
        .resolve_logical_repository_strict(project_id, logical_repository_id)
        .map(|(_, _, repository)| repository)
        .map_err(|_| {
            routing_error(
                RepositoryRoutingErrorCode::Inconsistent,
                "logical repository target cannot be resolved from authority",
            )
        })
}

fn legacy_repository_for_attempt(
    repository_store: &RepositoryStore,
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<RepositoryRecord, ProductStoreError> {
    let current_work_item_id = current_work_item_id_for_attempt(attempt);
    let physical_repository_id = LifecycleStore::new(app_paths.clone())
        .list_work_items(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .find(|work_item| work_item.id == current_work_item_id)
        .map(|work_item| work_item.repository_id)
        .or_else(|| {
            IssueStore::new(app_paths.clone())
                .get(&attempt.project_id, &attempt.issue_id)
                .ok()
                .and_then(|issue| issue.repo_id)
        })
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "work_item",
            id: current_work_item_id.to_string(),
        })?;
    legacy_physical_repository(
        repository_store,
        &attempt.project_id,
        &physical_repository_id,
    )
}

fn current_work_item_id_for_attempt(attempt: &CodingExecutionAttempt) -> &str {
    attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id)
}

fn logical_repository_for_group_attempt(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    selection: &IssueCodebaseSelection,
) -> Result<LogicalRepositoryId, ProductStoreError> {
    let plan_id = attempt.work_item_group_id.as_deref().ok_or_else(|| {
        routing_error(
            RepositoryRoutingErrorCode::Inconsistent,
            "schema-v2 group attempt has no plan ID",
        )
    })?;
    let authoritative = CodingAttemptStore::new(app_paths.clone())
        .resolve_authoritative_group_plan_binding(&attempt.project_id, &attempt.issue_id, plan_id)
        .map_err(|_| {
            routing_error(
                RepositoryRoutingErrorCode::Inconsistent,
                "schema-v2 group target cannot be resolved from the authoritative plan",
            )
        })?;
    if let Some(reason) = authoritative
        .units
        .iter()
        .find_map(|unit| unit.source_draft_error.as_deref())
    {
        return Err(routing_error(
            RepositoryRoutingErrorCode::Inconsistent,
            reason,
        ));
    }
    let target_ids: BTreeSet<LogicalRepositoryId> = authoritative
        .units
        .iter()
        .filter_map(|unit| unit.target_repository_id)
        .collect();
    match target_ids.len() {
        1 => Ok(*target_ids.first().expect("one group target exists")),
        0 => {
            let [focus_repository_id] = selection.focus_repository_ids.as_slice() else {
                return Err(routing_error(
                    RepositoryRoutingErrorCode::TargetMissing,
                    "group has no unique target repository and selection focus is not unique",
                ));
            };
            Ok(*focus_repository_id)
        }
        _ => Err(routing_error(
            RepositoryRoutingErrorCode::TargetAmbiguous,
            "group has multiple target repositories",
        )),
    }
}

fn validate_logical_selection(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    manifest: &LogicalCodebaseManifest,
    selection: &IssueCodebaseSelection,
) -> Result<BTreeSet<LogicalRepositoryId>, ProductStoreError> {
    if selection.project_id != project_id || selection.issue_id != issue_id {
        return Err(routing_error(
            RepositoryRoutingErrorCode::Inconsistent,
            "issue codebase selection identity does not match the attempt",
        ));
    }
    if selection.invalidation.is_some() {
        return Err(routing_error(
            RepositoryRoutingErrorCode::SelectionInvalidated,
            "issue codebase selection has been invalidated",
        ));
    }
    let active_members: BTreeSet<LogicalRepositoryId> =
        LogicalCodebaseStore::new(app_paths.clone())
            .list_members(project_id)?
            .into_iter()
            .filter(|member| member.status == MemberStatus::Active)
            .map(|member| member.logical_repository_id)
            .collect();
    if manifest
        .member_ids
        .iter()
        .any(|id| !active_members.contains(id))
    {
        return Err(routing_error(
            RepositoryRoutingErrorCode::MemberRemoved,
            "logical codebase manifest references a missing or inactive member",
        ));
    }
    match selection.selection_policy {
        SelectionPolicy::AllMembers => {
            if selection
                .focus_repository_ids
                .iter()
                .any(|id| !manifest.member_ids.contains(id))
            {
                return Err(routing_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    "issue codebase selection focus is outside the manifest",
                ));
            }
            Ok(manifest.member_ids.iter().copied().collect())
        }
        SelectionPolicy::Explicit => {
            selection.validate_focus_subset().map_err(|error| {
                routing_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    format!("invalid issue codebase selection: {error}"),
                )
            })?;
            let selected_ids: BTreeSet<LogicalRepositoryId> =
                selection.resolve_effective_members().into_iter().collect();
            if selected_ids
                .iter()
                .any(|id| !manifest.member_ids.contains(id))
            {
                return Err(routing_error(
                    RepositoryRoutingErrorCode::Inconsistent,
                    "issue codebase selection references a member absent from the manifest",
                ));
            }
            Ok(selected_ids)
        }
    }
}

fn legacy_physical_repository(
    repository_store: &RepositoryStore,
    project_id: &str,
    physical_repository_id: &str,
) -> Result<RepositoryRecord, ProductStoreError> {
    repository_store
        .list(project_id)?
        .into_iter()
        .find(|repository| repository.id == physical_repository_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "repository",
            id: physical_repository_id.to_string(),
        })
}

fn routing_error(code: RepositoryRoutingErrorCode, reason: impl Into<String>) -> ProductStoreError {
    let stable_code = code.stable_code();
    ProductStoreError::InvalidRecord {
        kind: "repository_routing",
        reason: format!("{stable_code}: {}", reason.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_models::{
        CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    };
    use crate::product::lifecycle_store::{CreateWorkItemInput, LifecycleStore};
    use crate::product::models::{ProviderName, RepositoryRecord, WorkItemPlanLineage};
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::product::work_item_revision_store::WorkItemRevisionStore;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    #[test]
    fn repository_resolver_legacy_single_repo_without_selection_returns_physical_repository() {
        // 无 manifest、无 selection 的单仓 attempt 必须从 work item 物理仓库解析。
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "legacy repository resolver".to_string(),
                description: None,
            })
            .unwrap();
        let repository_path = root.path().join("repository_0001");
        write_repository_projection(&paths, &repository_path);
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

        let repository = resolve_coding_attempt_repository(
            &paths,
            &attempt_fixture(),
            SchemaV2GroupAttemptScopePolicy::RequireWorkItemGroupScope,
        )
        .unwrap();

        assert_eq!(repository.id, "repository_0001");
        assert_eq!(repository.path, repository_path);
    }

    #[test]
    fn schema_v2_group_detection_preserves_entrypoint_scope_semantics() {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        WorkItemRevisionStore::new(paths.clone())
            .put_plan_lineage(&WorkItemPlanLineage {
                id: "work_item_plan_0001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                story_spec_refs: Vec::new(),
                design_spec_refs: Vec::new(),
                active_revision_id: None,
                active_amendment_id: None,
                created_at: "2026-08-11T00:00:00Z".to_string(),
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            })
            .unwrap();
        let mut attempt = attempt_fixture();
        attempt.scope = CodingAttemptScope::WorkItem;
        attempt.work_item_group_id = Some("work_item_plan_0001".to_string());

        assert!(
            !is_schema_v2_group_attempt(
                &paths,
                &attempt,
                SchemaV2GroupAttemptScopePolicy::RequireWorkItemGroupScope,
            )
            .unwrap()
        );
        assert!(
            is_schema_v2_group_attempt(
                &paths,
                &attempt,
                SchemaV2GroupAttemptScopePolicy::TreatLineageAsGroup,
            )
            .unwrap()
        );
    }

    fn attempt_fixture() -> CodingExecutionAttempt {
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
            target_snapshot: None,
            completed_at: None,
        }
    }

    fn write_repository_projection(paths: &ProductAppPaths, repository_path: &Path) {
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
                logical_repository_id: None,
                primary_checkout_id: None,
                identity_schema_version: 1,
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            }],
        )
        .unwrap();
    }
}
