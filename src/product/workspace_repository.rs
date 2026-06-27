use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_store::IssueStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{RepositoryRecord, WorkspaceSessionRecord, WorkspaceType};
use crate::product::repository_store::RepositoryStore;

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
        WorkspaceType::Story => lifecycle
            .list_story_specs(&session.project_id, &session.issue_id)?
            .into_iter()
            .find(|story| story.id == session.entity_id)
            .map(|story| story.repository_id)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "story_spec",
                id: session.entity_id.clone(),
            }),
        WorkspaceType::Design => IssueStore::new(app_paths.clone())
            .get(&session.project_id, &session.issue_id)?
            .repo_id
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "repository",
                id: format!("issue:{}:repo_id", session.issue_id),
            }),
        WorkspaceType::WorkItem => lifecycle
            .list_work_items(&session.project_id, &session.issue_id)?
            .into_iter()
            .find(|work_item| work_item.id == session.entity_id)
            .map(|work_item| work_item.repository_id)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "work_item",
                id: session.entity_id.clone(),
            }),
        WorkspaceType::WorkItemPlan => IssueStore::new(app_paths.clone())
            .get(&session.project_id, &session.issue_id)?
            .repo_id
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "repository",
                id: format!("issue:{}:repo_id", session.issue_id),
            }),
    }
}
