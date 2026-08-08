use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_store::IssueStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{RepositoryRecord, WorkspaceSessionRecord, WorkspaceType};
use crate::product::repository_store::RepositoryStore;
use crate::product::work_item_runtime_reader::WorkItemRuntimeReader;

pub fn workspace_repository_for_session(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<RepositoryRecord, ProductStoreError> {
    let repository_id = workspace_repository_id(app_paths, lifecycle, session)?;
    RepositoryStore::new(app_paths.clone())
        .list(&session.project_id)?
        .into_iter()
        .find(|repository| repository.id == repository_id)
        .ok_or(ProductStoreError::NotFound {
            kind: "repository",
            id: repository_id,
        })
}

fn workspace_repository_id(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<String, ProductStoreError> {
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
            // Prefer the new logical identity when backfilled (logical codebase feature
            // enabled); otherwise fall back to the legacy physical repository_id.
            story
                .focus_repository_id
                .and_then(|logical_id| {
                    resolve_logical_to_physical(app_paths, &session.project_id, logical_id)
                })
                .or(story.repository_id.into())
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "repository",
                    id: format!("story_spec:{}:repository_id", session.entity_id),
                })
        }
        WorkspaceType::Design | WorkspaceType::WorkItemPlan => {
            issue_repository_id(app_paths, session)
        }
        WorkspaceType::WorkItem => {
            WorkItemRuntimeReader::new(app_paths.clone()).resolve_workspace(session)?;
            lifecycle
                .list_work_items(&session.project_id, &session.issue_id)?
                .into_iter()
                .find(|work_item| work_item.id == session.entity_id)
                .and_then(|work_item| {
                    work_item
                        .target_repository_id
                        .and_then(|logical_id| {
                            resolve_logical_to_physical(app_paths, &session.project_id, logical_id)
                        })
                        .or(work_item.repository_id.into())
                })
                .or_else(|| issue_repo_id(app_paths, session).ok())
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "repository",
                    id: format!("work_item:{}:repository_id", session.entity_id),
                })
        }
    }
}

fn issue_repository_id(
    app_paths: &ProductAppPaths,
    session: &WorkspaceSessionRecord,
) -> Result<String, ProductStoreError> {
    issue_repo_id(app_paths, session)
}

fn issue_repo_id(
    app_paths: &ProductAppPaths,
    session: &WorkspaceSessionRecord,
) -> Result<String, ProductStoreError> {
    IssueStore::new(app_paths.clone())
        .get(&session.project_id, &session.issue_id)?
        .repo_id
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "repository",
            id: format!("issue:{}:repo_id", session.issue_id),
        })
}

/// Resolves a logical repository identity to its current physical repository ID
/// via the dual-read authority. Returns None when the logical identity cannot
/// be resolved (e.g. feature disabled or migration not yet backfilled), letting
/// callers fall back to the legacy physical ID.
fn resolve_logical_to_physical(
    app_paths: &ProductAppPaths,
    project_id: &str,
    logical_id: crate::product::logical_codebase::LogicalRepositoryId,
) -> Option<String> {
    let store = RepositoryStore::new(app_paths.clone());
    store
        .resolve_logical_repository(project_id, logical_id)
        .ok()
        .map(|(_, _, repository)| repository.id)
}
