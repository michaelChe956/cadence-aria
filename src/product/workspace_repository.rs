use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::{RepositoryRecord, WorkspaceSessionRecord, WorkspaceType};
use crate::product::repository_store::RepositoryStore;
use crate::product::work_item_runtime_reader::WorkItemRuntimeReader;

pub fn workspace_repository_for_session(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<RepositoryRecord, ProductStoreError> {
    let logical_repository_id = workspace_logical_repository_id(app_paths, lifecycle, session)?;
    let (_, _, repository) = RepositoryStore::new(app_paths.clone())
        .resolve_logical_repository(&session.project_id, logical_repository_id)?;
    Ok(repository)
}

fn workspace_logical_repository_id(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<LogicalRepositoryId, ProductStoreError> {
    match session.workspace_type {
        WorkspaceType::Story => lifecycle
            .list_story_specs(&session.project_id, &session.issue_id)?
            .into_iter()
            .find(|story| story.id == session.entity_id)
            .and_then(|story| story.focus_repository_id)
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "logical_repository",
                id: format!("story_spec:{}:focus_repository_id", session.entity_id),
            }),
        WorkspaceType::Design | WorkspaceType::WorkItemPlan => {
            issue_selection_logical_repository_id(app_paths, session)
        }
        WorkspaceType::WorkItem => {
            WorkItemRuntimeReader::new(app_paths.clone()).resolve_workspace(session)?;
            lifecycle
                .list_work_items(&session.project_id, &session.issue_id)?
                .into_iter()
                .find(|work_item| work_item.id == session.entity_id)
                .and_then(|work_item| work_item.target_repository_id)
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "logical_repository",
                    id: format!("work_item:{}:target_repository_id", session.entity_id),
                })
        }
    }
}

fn issue_selection_logical_repository_id(
    app_paths: &ProductAppPaths,
    session: &WorkspaceSessionRecord,
) -> Result<LogicalRepositoryId, ProductStoreError> {
    let issue_id = &session.issue_id;
    let path = app_paths
        .issue_root(&session.project_id, issue_id)
        .join("codebase-selection.json");
    let selection: IssueCodebaseSelection = crate::product::json_store::read_json(&path)?;
    let [logical_repository_id] = selection.focus.as_slice() else {
        return Err(ProductStoreError::Ambiguous {
            kind: "issue_codebase_selection",
            id: issue_id.clone(),
        });
    };
    Ok(*logical_repository_id)
}

#[derive(serde::Deserialize)]
struct IssueCodebaseSelection {
    focus: Vec<LogicalRepositoryId>,
}
