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
                    let logical_id = story.focus_repository_id.ok_or_else(|| {
                        routing_error(
                            RepositoryRoutingErrorCode::TargetMissing,
                            format!("story {} has no focus repository", story.id),
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
}
