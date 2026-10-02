use std::collections::BTreeSet;

use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_store::IssueStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::{
    LogicalRepositoryId, RepositoryRouting, RepositoryRoutingErrorCode, SelectionPolicy,
};
use crate::product::models::{RepositoryRecord, WorkspaceSessionRecord, WorkspaceType};
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;
use crate::product::work_item_runtime_reader::WorkItemRuntimeReader;
use crate::product::workspace_engine::draft_batch::compile_support::resolve_logical_work_item_plan_repository_targets;

pub fn workspace_repository_for_session(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<RepositoryRecord, ProductStoreError> {
    workspace_repository(app_paths, lifecycle, session)
}

fn workspace_repository(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<RepositoryRecord, ProductStoreError> {
    let routing =
        RepositoryRouting::load_for_issue(app_paths, &session.project_id, &session.issue_id)?;
    match session.workspace_type {
        WorkspaceType::Story => {
            let story = lifecycle
                .list_story_specs(&session.project_id, &session.issue_id)?
                .into_iter()
                .find(|story| story.id == session.entity_id)
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "story_spec",
                    id: session.entity_id.clone(),
                })?;
            match routing {
                RepositoryRouting::Legacy { .. } => resolve_legacy_physical_repository(
                    app_paths,
                    &session.project_id,
                    &story.repository_id,
                ),
                RepositoryRouting::Logical {
                    manifest,
                    selection,
                } => {
                    match story.focus_repository_id {
                        Some(logical_id) => resolve_selected_logical_repository(
                            app_paths,
                            &session.project_id,
                            logical_id,
                            &manifest,
                            &selection,
                        ),
                        // 方案X草稿态（缺陷 #2 修法 A，controller 裁决 2026-10-02）：
                        // focus=None ∧ involved 空 = AI 自决 involved 之前，目标即
                        // LC 聚合根本身（ENV-10/11 cwd≠target 合法形态；author_root_launch
                        // 的 PolicyTarget::aggregate_root 锚同口径）。回写 focus 后
                        // 恢复成员解析；involved 非空仍 fail-closed（下方 None 臂）。
                        None if story.involved_repository_ids.is_empty() => {
                            Ok(aggregate_root_view(&manifest))
                        }
                        None => Err(routing_error(
                            RepositoryRoutingErrorCode::TargetMissing,
                            format!("story {} has no focus repository", story.id),
                        )),
                    }
                }
                RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
            }
        }
        WorkspaceType::Design => {
            let design = lifecycle
                .list_design_specs(&session.project_id, &session.issue_id)?
                .into_iter()
                .find(|design| design.id == session.entity_id)
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "design_spec",
                    id: session.entity_id.clone(),
                })?;
            match routing {
                RepositoryRouting::Legacy { .. } => resolve_issue_repository(app_paths, session),
                RepositoryRouting::Logical {
                    manifest,
                    selection,
                } => {
                    let target_ids = unique_ids(design.involved_repository_ids);
                    let logical_id = unique_target(target_ids, &design.id)?;
                    resolve_selected_logical_repository(
                        app_paths,
                        &session.project_id,
                        logical_id,
                        &manifest,
                        &selection,
                    )
                }
                RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
            }
        }
        WorkspaceType::WorkItemPlan => {
            let plan = lifecycle.get_issue_work_item_plan(
                &session.project_id,
                &session.issue_id,
                &session.entity_id,
            )?;
            match routing {
                RepositoryRouting::Legacy { .. } => resolve_issue_repository(app_paths, session),
                RepositoryRouting::Logical {
                    manifest,
                    selection,
                } => {
                    let targets =
                        resolve_logical_work_item_plan_repository_targets(lifecycle, &plan)
                            .map_err(|reason| routing_error_for_target_error(&reason))?;
                    let target_ids = targets.unwrap_or_default().keys().copied().collect();
                    let logical_id =
                        plan_session_repository_target(target_ids, &plan.id, &selection)?;
                    resolve_selected_logical_repository(
                        app_paths,
                        &session.project_id,
                        logical_id,
                        &manifest,
                        &selection,
                    )
                }
                RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
            }
        }
        WorkspaceType::WorkItem => {
            WorkItemRuntimeReader::new(app_paths.clone()).resolve_workspace(session)?;
            match routing {
                RepositoryRouting::Legacy { .. } => {
                    let physical_repository_id = lifecycle
                        .list_work_items(&session.project_id, &session.issue_id)?
                        .into_iter()
                        .find(|work_item| work_item.id == session.entity_id)
                        .map(|work_item| work_item.repository_id)
                        .or_else(|| {
                            IssueStore::new(app_paths.clone())
                                .get(&session.project_id, &session.issue_id)
                                .ok()
                                .and_then(|issue| issue.repo_id)
                        })
                        .ok_or_else(|| ProductStoreError::NotFound {
                            kind: "work_item",
                            id: session.entity_id.clone(),
                        })?;
                    resolve_legacy_physical_repository(
                        app_paths,
                        &session.project_id,
                        &physical_repository_id,
                    )
                }
                RepositoryRouting::Logical {
                    manifest,
                    selection,
                } => {
                    let work_item = lifecycle
                        .list_work_items(&session.project_id, &session.issue_id)?
                        .into_iter()
                        .find(|work_item| work_item.id == session.entity_id)
                        .ok_or_else(|| ProductStoreError::NotFound {
                            kind: "work_item",
                            id: session.entity_id.clone(),
                        })?;
                    let logical_id = work_item.target_repository_id.ok_or_else(|| {
                        routing_error(
                            RepositoryRoutingErrorCode::TargetMissing,
                            format!("work item {} has no target repository", work_item.id),
                        )
                    })?;
                    resolve_selected_logical_repository(
                        app_paths,
                        &session.project_id,
                        logical_id,
                        &manifest,
                        &selection,
                    )
                }
                RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
            }
        }
    }
}

fn resolve_selected_logical_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    logical_id: LogicalRepositoryId,
    manifest: &crate::product::logical_codebase::LogicalCodebaseManifest,
    selection: &crate::product::logical_codebase::IssueCodebaseSelection,
) -> Result<RepositoryRecord, ProductStoreError> {
    if selection.invalidation.is_some() {
        return Err(routing_error(
            RepositoryRoutingErrorCode::SelectionInvalidated,
            "issue codebase selection has been invalidated",
        ));
    }
    selection.validate_focus_subset().map_err(|error| {
        routing_error(
            RepositoryRoutingErrorCode::Inconsistent,
            format!("invalid issue codebase selection: {error}"),
        )
    })?;
    let selected_ids: BTreeSet<LogicalRepositoryId> = match selection.selection_policy {
        SelectionPolicy::AllMembers => manifest.member_ids.iter().copied().collect(),
        SelectionPolicy::Explicit => selection.resolve_effective_members().into_iter().collect(),
    };
    if !selected_ids.contains(&logical_id) {
        return Err(routing_error(
            RepositoryRoutingErrorCode::TargetUnknown,
            format!("logical repository target {logical_id:?} is not in the effective selection"),
        ));
    }
    let project = ProjectStore::new(app_paths.clone()).get(project_id)?;
    RepositoryStore::for_project(app_paths.clone(), &project)
        .resolve_logical_repository_strict(project_id, logical_id)
        .map(|(_, _, repository)| repository)
}

fn resolve_legacy_physical_repository(
    app_paths: &ProductAppPaths,
    project_id: &str,
    physical_repository_id: &str,
) -> Result<RepositoryRecord, ProductStoreError> {
    let project = ProjectStore::new(app_paths.clone()).get(project_id)?;
    let store = RepositoryStore::for_project(app_paths.clone(), &project);
    if let Ok((_, _, repository)) =
        store.resolve_legacy_physical_repository_if_dual(project_id, physical_repository_id)
    {
        return Ok(repository);
    }
    store
        .list(project_id)?
        .into_iter()
        .find(|repository| repository.id == physical_repository_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "repository",
            id: physical_repository_id.to_string(),
        })
}

/// 草稿 Story（Logical 路由、focus 未定、involved 空）的聚合根锚视图：
/// cwd 仍由唯一 authority resolver 冻结为 canonical root；target 锚定
/// `PolicyTarget::aggregate_root(provider_context_root)`（primary_checkout_id
/// =None → author_root_launch 走 aggregate_root 臂）。视图仅供 attach/launch
/// 目标锚定，不落盘、不参与写根授权（PlanningReadOnly 空 writable_roots
/// 不变）；logical_repository_id 取 manifest 逻辑身份（确定性，仅作 LC
/// 会话 gateway 注入谓词之用，不进入 PolicyTarget/成员解析）。
fn aggregate_root_view(
    manifest: &crate::product::logical_codebase::LogicalCodebaseManifest,
) -> RepositoryRecord {
    let root = manifest.provider_context_root.clone();
    RepositoryRecord {
        id: format!("lc_aggregate_root_view_{}", manifest.logical_codebase_id),
        project_id: manifest.project_id.clone(),
        name: "lc-aggregate-root".to_string(),
        path: root.clone(),
        repo_hash: String::new(),
        runtime_root: root,
        default_policy_preset: String::new(),
        default_provider_mode: String::new(),
        created_at: String::new(),
        logical_repository_id: Some(LogicalRepositoryId(manifest.logical_codebase_id)),
        primary_checkout_id: None,
        identity_schema_version: 0,
        updated_at: String::new(),
    }
}

fn resolve_issue_repository(
    app_paths: &ProductAppPaths,
    session: &WorkspaceSessionRecord,
) -> Result<RepositoryRecord, ProductStoreError> {
    let physical_repository_id = IssueStore::new(app_paths.clone())
        .get(&session.project_id, &session.issue_id)?
        .repo_id
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "repository",
            id: format!("issue:{}:repo_id", session.issue_id),
        })?;
    resolve_legacy_physical_repository(app_paths, &session.project_id, &physical_repository_id)
}

fn unique_ids(ids: Vec<LogicalRepositoryId>) -> BTreeSet<LogicalRepositoryId> {
    ids.into_iter().collect()
}

fn unique_target(
    target_ids: BTreeSet<LogicalRepositoryId>,
    entity_id: &str,
) -> Result<LogicalRepositoryId, ProductStoreError> {
    match target_ids.len() {
        0 => Err(routing_error(
            RepositoryRoutingErrorCode::TargetMissing,
            format!("{entity_id} has no unique logical repository target"),
        )),
        1 => Ok(*target_ids.first().expect("one target exists")),
        _ => Err(routing_error(
            RepositoryRoutingErrorCode::TargetAmbiguous,
            format!("{entity_id} has multiple logical repository targets"),
        )),
    }
}

/// REQ-COD-04（WP1 分流化）：plan 会话路由面的 target 解析。
///
/// - 0 target → TargetMissing fail-closed（保持现行，即使 focus 唯一也不回落）；
/// - 1 target → 唯一 target（现行语义零变化）；
/// - ≥2 target → 回落 selection focus 唯一解析（与创建面 0-target focus 语义
///   对称）；focus 不唯一保持 TargetAmbiguous（focus 面保留，不新造 plan 级
///   聚合根路由）。
fn plan_session_repository_target(
    target_ids: BTreeSet<LogicalRepositoryId>,
    entity_id: &str,
    selection: &crate::product::logical_codebase::IssueCodebaseSelection,
) -> Result<LogicalRepositoryId, ProductStoreError> {
    if target_ids.len() >= 2
        && let [focus_repository_id] = selection.focus_repository_ids.as_slice()
    {
        return Ok(*focus_repository_id);
    }
    unique_target(target_ids, entity_id)
}

fn routing_error_for_target_error(reason: &str) -> ProductStoreError {
    let code = if reason.contains("target_member_removed") || reason.contains("invalid members") {
        RepositoryRoutingErrorCode::Inconsistent
    } else if reason.contains("cannot resolve target") {
        RepositoryRoutingErrorCode::TargetUnknown
    } else {
        RepositoryRoutingErrorCode::TargetMissing
    };
    routing_error(code, reason)
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
    use super::*;
    use crate::product::logical_codebase::{
        IssueCodebaseSelection, LogicalCodebaseManifest, LogicalCodebaseStore,
    };

    fn write_manifest_fixture(paths: &ProductAppPaths, project_id: &str) {
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest(
                project_id,
                &LogicalCodebaseManifest::new(
                    project_id,
                    paths.root().join("aggregate-root"),
                    Vec::new(),
                ),
            )
            .unwrap();
    }

    #[test]
    fn load_routing_none_none_is_legacy() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let routing =
            RepositoryRouting::load_for_issue(&paths, "project_0001", "issue_0001").unwrap();
        assert!(matches!(routing, RepositoryRouting::Legacy { .. }));
    }

    #[test]
    fn load_routing_some_none_is_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        write_manifest_fixture(&paths, "project_0001");
        let routing =
            RepositoryRouting::load_for_issue(&paths, "project_0001", "issue_0001").unwrap();
        assert!(matches!(routing, RepositoryRouting::FailClosed { .. }));
    }

    #[test]
    fn plan_session_target_zero_targets_stays_target_missing_even_with_unique_focus() {
        // 双审定案：WorkItemPlan 面 0-target 保持 TargetMissing fail-closed 红线，
        // 即使 focus 唯一也不回落（与创建面 0-target focus 语义不对称是定案）。
        let focus = LogicalRepositoryId(uuid::Uuid::new_v4());
        let selection = IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![focus],
            Vec::new(),
            vec![focus],
            None,
        );

        let error =
            plan_session_repository_target(BTreeSet::new(), "work_item_plan_0001", &selection)
                .unwrap_err();

        assert_eq!(
            stable_routing_code(&error),
            "repository_routing_target_missing"
        );
    }

    #[test]
    fn plan_session_target_single_target_resolution_unchanged() {
        let target = LogicalRepositoryId(uuid::Uuid::new_v4());
        let selection = IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![target],
            Vec::new(),
            vec![target],
            None,
        );
        let targets = BTreeSet::from([target]);

        let resolved =
            plan_session_repository_target(targets, "work_item_plan_0001", &selection).unwrap();

        assert_eq!(resolved, target);
    }

    #[test]
    fn plan_session_target_multi_target_falls_back_to_unique_focus() {
        // REQ-COD-04（WP1 分流化）：plan 会话路由面——多 target 回落 selection
        // focus 唯一解析（与创建面 0-target focus 语义对称），不再一律 TargetAmbiguous。
        let api = LogicalRepositoryId(uuid::Uuid::new_v4());
        let web = LogicalRepositoryId(uuid::Uuid::new_v4());
        let selection = IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![api, web],
            Vec::new(),
            vec![api],
            None,
        );
        let targets = BTreeSet::from([api, web]);

        let resolved =
            plan_session_repository_target(targets, "work_item_plan_0001", &selection).unwrap();

        assert_eq!(resolved, api);
    }

    #[test]
    fn plan_session_target_multi_target_with_multi_focus_stays_ambiguous() {
        // focus 面保留：多 target + 多 focus → TargetAmbiguous（不新造 plan 级
        // 聚合根路由，不静默聚合）。
        let api = LogicalRepositoryId(uuid::Uuid::new_v4());
        let web = LogicalRepositoryId(uuid::Uuid::new_v4());
        let selection = IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![api, web],
            Vec::new(),
            vec![api, web],
            None,
        );
        let targets = BTreeSet::from([api, web]);

        let error =
            plan_session_repository_target(targets, "work_item_plan_0001", &selection).unwrap_err();

        assert_eq!(stable_routing_code(&error), "repository_routing_ambiguous");
    }

    #[test]
    fn plan_session_target_multi_target_without_focus_stays_ambiguous() {
        let api = LogicalRepositoryId(uuid::Uuid::new_v4());
        let web = LogicalRepositoryId(uuid::Uuid::new_v4());
        let selection = IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![api, web],
            Vec::new(),
            Vec::new(),
            None,
        );
        let targets = BTreeSet::from([api, web]);

        let error =
            plan_session_repository_target(targets, "work_item_plan_0001", &selection).unwrap_err();

        assert_eq!(stable_routing_code(&error), "repository_routing_ambiguous");
    }

    fn stable_routing_code(error: &ProductStoreError) -> &str {
        let ProductStoreError::InvalidRecord { reason, .. } = error else {
            panic!("expected repository_routing InvalidRecord, got {error:?}");
        };
        reason.split(':').next().unwrap_or_default()
    }

    /// 缺陷 #2（2026-10-02 全链 E2E）回归夹具：LC 路由（manifest+selection+
    /// 成员 git 仓）下的 Story 会话解析。
    struct LcStoryRoutingFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        manifest: LogicalCodebaseManifest,
        member_logical_id: LogicalRepositoryId,
        member_path: std::path::PathBuf,
    }

    impl LcStoryRoutingFixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let paths = ProductAppPaths::new(temp.path().join(".aria"));
            crate::product::project_store::ProjectStore::new(paths.clone())
                .create(crate::product::project_store::CreateProjectInput {
                    name: "lc-routing".to_string(),
                    description: None,
                })
                .unwrap();
            let root = temp.path().join("aggregate-root");
            let member_path = root.join("member-a");
            std::fs::create_dir_all(&member_path).unwrap();
            for args in [
                vec!["init", "--quiet"],
                vec!["config", "user.email", "routing@example.test"],
                vec!["config", "user.name", "Routing Fixture"],
            ] {
                let status = std::process::Command::new("git")
                    .args(&args)
                    .current_dir(&member_path)
                    .status()
                    .unwrap();
                assert!(status.success(), "git {:?} failed", args);
            }
            std::fs::write(member_path.join("README.md"), "# member-a\n").unwrap();
            for args in [
                vec!["add", "README.md"],
                vec!["commit", "--quiet", "-m", "initial commit"],
            ] {
                let status = std::process::Command::new("git")
                    .args(&args)
                    .current_dir(&member_path)
                    .status()
                    .unwrap();
                assert!(status.success(), "git {:?} failed", args);
            }
            let repository =
                crate::product::repository_store::RepositoryStore::with_logical_codebase_feature(
                    paths.clone(),
                    crate::product::logical_codebase::LogicalCodebaseFeature::enabled(),
                )
                .create(crate::product::repository_store::CreateRepositoryInput {
                    project_id: "project_0001".to_string(),
                    name: "member-a".to_string(),
                    path: member_path.clone(),
                    default_policy_preset: None,
                    default_provider_mode: None,
                    idempotency_key: "lc-story-routing-member-a".to_string(),
                })
                .unwrap();
            let member_logical_id = repository.logical_repository_id.expect("logical id");
            let manifest = LogicalCodebaseStore::new(paths.clone())
                .load_manifest("project_0001")
                .unwrap()
                .expect("manifest auto-created by member registration");
            crate::product::logical_codebase::IssueCodebaseSelectionStore::new(paths.clone())
                .save(&IssueCodebaseSelection::all_members(
                    "project_0001",
                    "issue_0001",
                    None,
                ))
                .unwrap();
            Self {
                _temp: temp,
                paths,
                manifest,
                member_logical_id,
                member_path,
            }
        }

        fn story_session(
            &self,
            involved: Vec<LogicalRepositoryId>,
            focus: Option<LogicalRepositoryId>,
        ) -> crate::product::models::WorkspaceSessionRecord {
            let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(self.paths.clone());
            let story = lifecycle
                .create_story_spec(crate::product::lifecycle_store::CreateStorySpecInput {
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    repository_id: String::new(),
                    title: "lc draft story".to_string(),
                    aggregate_codebase: Some(
                        crate::product::lifecycle_store::AggregateStorySpecScope {
                            logical_codebase_ref: self.manifest.logical_codebase_id,
                            effective_member_ids: self.manifest.member_ids.clone(),
                            involved_repository_ids: involved,
                            focus_repository_id: focus,
                        },
                    ),
                })
                .unwrap();
            lifecycle
                .create_workspace_session(
                    crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                        project_id: "project_0001".to_string(),
                        issue_id: "issue_0001".to_string(),
                        entity_id: story.id,
                        workspace_type: crate::product::models::WorkspaceType::Story,
                        author_provider: crate::product::models::ProviderName::ClaudeCode,
                        reviewer_provider: Some(crate::product::models::ProviderName::ClaudeCode),
                        review_rounds: 1,
                        superpowers_enabled: false,
                        openspec_enabled: false,
                        work_item_plan_options: None,
                    },
                )
                .unwrap()
        }
    }

    /// 红→绿主锁：草稿 LC Story（focus=None ∧ involved 空）attach 期解析不再
    /// TargetMissing，锚定聚合根视图（path=provider_context_root、无 checkout、
    /// logical 身份在场供 gateway 注入谓词）。
    #[test]
    fn draft_lc_story_anchors_at_aggregate_root_until_focus_determined() {
        let fixture = LcStoryRoutingFixture::new();
        let session = fixture.story_session(Vec::new(), None);
        let repository = workspace_repository_for_session(
            &fixture.paths,
            &LifecycleStore::new(fixture.paths.clone()),
            &session,
        )
        .expect("draft LC story must anchor at the aggregate root view");
        assert_eq!(repository.path, fixture.manifest.provider_context_root);
        assert!(repository.logical_repository_id.is_some());
        assert!(repository.primary_checkout_id.is_none());
    }

    /// 边界 1：focus 已定（回写后）→ 恢复既有成员解析，路径=成员 canonical
    /// checkout、logical 身份=成员。
    #[test]
    fn lc_story_with_focus_resolves_member_unchanged() {
        let fixture = LcStoryRoutingFixture::new();
        let session = fixture.story_session(
            vec![fixture.member_logical_id],
            Some(fixture.member_logical_id),
        );
        let repository = workspace_repository_for_session(
            &fixture.paths,
            &LifecycleStore::new(fixture.paths.clone()),
            &session,
        )
        .expect("focused LC story resolves the member checkout");
        assert_eq!(repository.path, fixture.member_path);
        assert_eq!(
            repository.logical_repository_id,
            Some(fixture.member_logical_id)
        );
    }

    /// 边界 2：focus=None 但 involved 非空（回写异常半态）→ 保持 TargetMissing
    /// fail-closed，不锚聚合根（防放宽）。
    #[test]
    fn lc_story_without_focus_but_involved_stays_target_missing() {
        let fixture = LcStoryRoutingFixture::new();
        let session = fixture.story_session(vec![fixture.member_logical_id], None);
        let error = workspace_repository_for_session(
            &fixture.paths,
            &LifecycleStore::new(fixture.paths.clone()),
            &session,
        )
        .unwrap_err();
        assert_eq!(stable_routing_code(&error), "repository_routing_target_missing");
    }
}
