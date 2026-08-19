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
    resolve_issue_logical_codebase_id,
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
/// v1.3：按 issue 所属 lc_id 寻址（R9 编码/交付链切换点）；单仓/无 LC issue
/// 回退 legacy project 级路径，行为不变。
pub fn resolve_coding_attempt_repository(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    group_scope_policy: SchemaV2GroupAttemptScopePolicy,
) -> Result<RepositoryRecord, ProductStoreError> {
    let project = ProjectStore::new(app_paths.clone()).get(&attempt.project_id)?;
    let routing =
        RepositoryRouting::load_for_issue(app_paths, &attempt.project_id, &attempt.issue_id)?;
    let lc_id =
        resolve_issue_logical_codebase_id(app_paths, &attempt.project_id, &attempt.issue_id)?;
    // Legacy 回退 store：仅单仓/无 LC issue 使用（R9 残留清理后唯一 for_project 用途）。
    let legacy_repository_store = RepositoryStore::for_project(app_paths.clone(), &project);

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
            lc_id.as_deref(),
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
        validate_snapshot_fields(app_paths, attempt, lc_id.as_deref()).map_err(|code| {
            routing_error(
                code,
                "target snapshot does not match logical codebase authority",
            )
        })?;
        return resolve_logical_repository(
            app_paths,
            &attempt.project_id,
            lc_id.as_deref(),
            snapshot.logical_repository_id,
        );
    }

    match routing {
        RepositoryRouting::Legacy { .. } => {
            legacy_repository_for_attempt(&legacy_repository_store, app_paths, attempt)
        }
        RepositoryRouting::Logical {
            manifest,
            selection,
        } => {
            let selected_ids = validate_logical_selection(
                app_paths,
                lc_id.as_deref(),
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
                app_paths,
                &attempt.project_id,
                lc_id.as_deref(),
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
    app_paths: &ProductAppPaths,
    project_id: &str,
    lc_id: Option<&str>,
    logical_repository_id: LogicalRepositoryId,
) -> Result<RepositoryRecord, ProductStoreError> {
    let repository_store = match lc_id {
        Some(_) => RepositoryStore::new(app_paths.clone()),
        None => {
            let project = ProjectStore::new(app_paths.clone()).get(project_id)?;
            RepositoryStore::for_project(app_paths.clone(), &project)
        }
    };
    repository_store
        .resolve_logical_repository_for_issue_codebase(project_id, lc_id, logical_repository_id)
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
    lc_id: Option<&str>,
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
    let authority = match lc_id {
        Some(lc_id) => LogicalCodebaseStore::for_lc(app_paths.clone(), lc_id),
        None => LogicalCodebaseStore::new(app_paths.clone()),
    };
    let active_members: BTreeSet<LogicalRepositoryId> = authority
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

    #[test]
    fn repository_resolver_resolves_new_lc_attempt_from_lc_subtree() {
        // R9：非 legacy 新 LC——权威记录/selection 只落在 logical-codebases/{lc_id}/
        // 子树（repos.json 无投影），attempt 携带 target snapshot，resolver 必须按
        // issue 所属 lc_id 寻址并从权威记录合成物理 RepositoryRecord。
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let project = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "new lc resolver".to_string(),
                description: None,
            })
            .unwrap();
        let record = LogicalCodebaseStore::new(paths.clone())
            .create(
                &project.id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "new-lc".to_string(),
                    aggregate_root: root.path().join("aggregate-root"),
                },
            )
            .unwrap();
        let lc_id = record.id;

        let canonical_path = root.path().join("api");
        std::fs::create_dir_all(&canonical_path).unwrap();
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&canonical_path)
                .status()
                .unwrap();
            assert!(status.success());
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "resolver@example.test"]);
        git(&["config", "user.name", "Resolver"]);
        std::fs::write(canonical_path.join("README.md"), "# api\n").unwrap();
        git(&["add", "README.md"]);
        git(&["commit", "-qm", "initial"]);

        let authority = LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone());
        let logical_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_id =
            crate::product::logical_codebase::RepositoryCheckoutId(uuid::Uuid::new_v4());
        let physical_repository_id = format!("repository_{}", uuid::Uuid::new_v4().simple());
        let manifest = LogicalCodebaseManifest::new(
            &project.id,
            root.path().join("aggregate-root"),
            vec![logical_id],
        );
        authority.save_manifest(&project.id, &manifest).unwrap();
        let now = "2026-08-18T00:00:00Z".to_string();
        let source_identity =
            crate::product::logical_codebase::RepositorySourceIdentity::from_git_parts(
                &canonical_path,
                canonical_path.join(".git"),
                None,
            );
        authority
            .save_member(
                &project.id,
                &crate::product::logical_codebase::CodebaseMemberRecord {
                    logical_repository_id: logical_id,
                    physical_repository_id: physical_repository_id.clone(),
                    alias: "api".to_string(),
                    role: "repository".to_string(),
                    ordinal: 0,
                    source_identity: source_identity.clone(),
                    repo_type: crate::product::logical_codebase::RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();
        authority
            .save_checkout(
                &project.id,
                &crate::product::logical_codebase::RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: logical_id,
                    physical_repository_id: physical_repository_id.clone(),
                    kind: crate::product::logical_codebase::CheckoutKind::Main,
                    canonical_path: canonical_path.clone(),
                    checkout_path_hash: "sha256:checkout".to_string(),
                    git_dir_identity: source_identity.git_dir_identity().to_string(),
                    revision: None,
                    availability: crate::product::logical_codebase::CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now,
                },
            )
            .unwrap();
        crate::product::logical_codebase::IdentityRegistryStore::new(paths.clone())
            .upsert_active(
                &project.id,
                crate::product::logical_codebase::IdentityRegistryEntry::active(
                    source_identity,
                    logical_id,
                    physical_repository_id.clone(),
                    checkout_id,
                    "resolver-new-lc-fixture".to_string(),
                ),
            )
            .unwrap();
        let policy = crate::product::logical_codebase::AggregatePolicyArtifactStore::for_lc(
            paths.clone(),
            lc_id.clone(),
        )
        .ensure_bootstrap(&manifest)
        .unwrap();

        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project.id.clone(),
                repo_id: Some(physical_repository_id.clone()),
                logical_codebase_id: Some(lc_id.clone()),
                title: "new lc issue".to_string(),
                description: None,
                change_id: None,
            })
            .unwrap();
        crate::product::logical_codebase::IssueCodebaseSelectionStore::for_lc(
            paths.clone(),
            lc_id.clone(),
        )
        .save(
            &IssueCodebaseSelection::all_members(&project.id, &issue.id, None)
                .for_logical_codebase(lc_id.clone()),
        )
        .unwrap();

        let snapshot = crate::product::coding_models::AttemptTargetSnapshot {
            logical_repository_id: logical_id,
            checkout_id,
            physical_repository_id: physical_repository_id.clone(),
            canonical_path: canonical_path.clone(),
            git_dir_identity: policy.digest.clone(),
            revision: None,
            policy_digest: policy.digest,
            membership_revision: manifest.membership_revision,
            captured_at: "2026-08-18T00:00:00Z".to_string(),
            capture_source: "test".to_string(),
        };
        // git_dir_identity 必须与 checkout 权威一致：重新以权威记录为准。
        let mut snapshot = snapshot;
        snapshot.git_dir_identity = authority
            .load_checkout(&project.id, checkout_id)
            .unwrap()
            .unwrap()
            .git_dir_identity;
        let mut attempt = attempt_fixture();
        attempt.project_id = project.id.clone();
        attempt.issue_id = issue.id.clone();
        attempt.target_snapshot = Some(snapshot);

        let repository = resolve_coding_attempt_repository(
            &paths,
            &attempt,
            SchemaV2GroupAttemptScopePolicy::RequireWorkItemGroupScope,
        )
        .expect("resolver must address the lc subtree");

        assert_eq!(repository.id, physical_repository_id);
        assert_eq!(repository.path, canonical_path);
        assert_eq!(repository.project_id, project.id);
        assert_eq!(repository.logical_repository_id, Some(logical_id));
        assert_eq!(repository.primary_checkout_id, Some(checkout_id));
    }
}
