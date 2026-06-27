use tempfile::TempDir;

use crate::product::app_paths::ProductAppPaths;
use crate::product::models::{ProviderName, WorkspaceType};

use super::*;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const REPOSITORY_ID: &str = "repository_0001";

fn setup() -> (TempDir, LifecycleStore) {
    let tmp = TempDir::new().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    (tmp, store)
}

fn create_session(
    store: &LifecycleStore,
    entity_id: &str,
    workspace_type: WorkspaceType,
) -> crate::product::models::WorkspaceSessionRecord {
    store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: entity_id.to_string(),
            workspace_type,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap()
}

#[test]
fn delete_story_spec_removes_record_versions_session_and_timeline() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "Session expired story".to_string(),
        })
        .unwrap();
    store
        .append_version(AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: story.id.clone(),
            markdown: "story markdown".to_string(),
            provider_run_refs: vec![],
            review_refs: vec![],
            confirmed_by: None,
        })
        .unwrap();
    let session = create_session(&store, &story.id, WorkspaceType::Story);
    store.save_timeline_nodes(&session.id, &[]).unwrap();
    let versions_root = store.versions_root(PROJECT_ID, ISSUE_ID, &story.id);
    let timeline_root = store
        .workspace_timeline_root_for_session(&session.id)
        .unwrap();

    store
        .delete_story_spec(PROJECT_ID, ISSUE_ID, &story.id)
        .unwrap();

    assert!(
        store
            .list_story_specs(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_versions(PROJECT_ID, ISSUE_ID, &story.id)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );
    assert!(!versions_root.exists());
    assert!(!timeline_root.exists());
}

#[test]
fn delete_design_spec_removes_record_versions_session_and_timeline() {
    let (_tmp, store) = setup();
    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            title: "Frontend design".to_string(),
        })
        .unwrap();
    store
        .append_version(AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: design.id.clone(),
            markdown: "design markdown".to_string(),
            provider_run_refs: vec![],
            review_refs: vec![],
            confirmed_by: None,
        })
        .unwrap();
    let session = create_session(&store, &design.id, WorkspaceType::Design);
    store.save_timeline_nodes(&session.id, &[]).unwrap();
    let versions_root = store.versions_root(PROJECT_ID, ISSUE_ID, &design.id);
    let timeline_root = store
        .workspace_timeline_root_for_session(&session.id)
        .unwrap();

    store
        .delete_design_spec(PROJECT_ID, ISSUE_ID, &design.id)
        .unwrap();

    assert!(
        store
            .list_design_specs(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_versions(PROJECT_ID, ISSUE_ID, &design.id)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );
    assert!(!versions_root.exists());
    assert!(!timeline_root.exists());
}

#[test]
fn delete_work_item_removes_record_session_and_timeline() {
    let (_tmp, store) = setup();
    let work_item = store
        .create_work_item(CreateWorkItemInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "Implement prompt component".to_string(),
            ..Default::default()
        })
        .unwrap();
    let session = create_session(&store, &work_item.id, WorkspaceType::WorkItem);
    store.save_timeline_nodes(&session.id, &[]).unwrap();
    let timeline_root = store
        .workspace_timeline_root_for_session(&session.id)
        .unwrap();

    store
        .delete_work_item(PROJECT_ID, ISSUE_ID, &work_item.id)
        .unwrap();

    assert!(
        store
            .list_work_items(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .is_empty()
    );
    assert!(!timeline_root.exists());
}
