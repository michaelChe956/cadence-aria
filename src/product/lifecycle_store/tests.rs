use tempfile::TempDir;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::models::{
    ProviderName, StorySpecRecord, WorkItemRuntimeBinding, WorkspaceType,
};

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

fn runtime_binding() -> WorkItemRuntimeBinding {
    WorkItemRuntimeBinding {
        plan_id: "work_item_plan_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        logical_work_item_id: "wi_library_export".to_string(),
        work_item_revision_id: "work_item_revision_0001".to_string(),
        projection_bundle_id: "work_item_projection_bundle_0001".to_string(),
        verification_plan_revision_id: "verification_plan_revision_0001".to_string(),
        canonical_contract_hash: "sha256:contract".to_string(),
        projection_compiler_version: "projection-compiler-v1".to_string(),
        human_projection_hash: "sha256:human".to_string(),
        coder_projection_hash: "sha256:coder".to_string(),
        reviewer_projection_hash: "sha256:reviewer".to_string(),
    }
}

#[test]
fn ensure_work_item_runtime_binding_persists_and_replays_the_same_binding() {
    let (_tmp, store) = setup();
    let session = create_session(&store, "wi_library_export", WorkspaceType::WorkItem);
    let binding = runtime_binding();

    let first = store
        .ensure_work_item_runtime_binding(&session.id, &binding)
        .unwrap();
    let replay = store
        .ensure_work_item_runtime_binding(&session.id, &binding)
        .unwrap();

    assert_eq!(first.work_item_runtime_binding.as_ref(), Some(&binding));
    assert_eq!(replay, first);
}

#[test]
fn ensure_work_item_runtime_binding_rejects_a_different_binding() {
    let (_tmp, store) = setup();
    let session = create_session(&store, "wi_library_export", WorkspaceType::WorkItem);
    let binding = runtime_binding();
    store
        .ensure_work_item_runtime_binding(&session.id, &binding)
        .unwrap();
    let mut different = binding;
    different.work_item_revision_id = "work_item_revision_0002".to_string();

    let error = store
        .ensure_work_item_runtime_binding(&session.id, &different)
        .unwrap_err();

    assert!(matches!(
        error,
        ProductStoreError::IdentityMismatch {
            kind: "work_item_runtime_binding",
            ..
        }
    ));
}

#[test]
fn ensure_work_item_runtime_binding_rejects_story_and_design_sessions() {
    let (_tmp, store) = setup();
    let binding = runtime_binding();

    for workspace_type in [WorkspaceType::Story, WorkspaceType::Design] {
        let session = create_session(&store, "shared_entity", workspace_type);
        let error = store
            .ensure_work_item_runtime_binding(&session.id, &binding)
            .unwrap_err();

        assert!(matches!(
            error,
            ProductStoreError::IdentityMismatch {
                kind: "workspace_session_type",
                ..
            }
        ));
    }
}

#[test]
fn ensure_version_repairs_current_version_without_appending_duplicate() {
    let (_tmp, store) = setup();
    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "Recover current version".to_string(),
        })
        .unwrap();
    let input = AppendSpecVersionInput {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        entity_id: story.id.clone(),
        markdown: "# Story Spec\n\nRecovered markdown".to_string(),
        provider_run_refs: vec![],
        review_refs: vec![],
        confirmed_by: None,
    };
    store.append_version(input.clone()).unwrap();

    let story_path = store
        .story_specs_root(PROJECT_ID, ISSUE_ID)
        .join(format!("{}.json", story.id));
    let mut stale_story: StorySpecRecord = read_json(&story_path).unwrap();
    stale_story.current_version = None;
    write_json(&story_path, &stale_story).unwrap();

    let ensured = store.ensure_version(input).unwrap();

    assert_eq!(ensured.version, 1);
    assert_eq!(
        store
            .list_versions(PROJECT_ID, ISSUE_ID, &story.id)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .list_story_specs(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .into_iter()
            .find(|record| record.id == story.id)
            .and_then(|record| record.current_version),
        Some(1)
    );
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

/// change `remove-work-item-handoff` 工作包 1.12：
/// 移除交接摘要引用后，work item 的完成 commit 记录必须仍可写入并读取。
/// 原摘要更新函数曾是 `completion_commit` 的唯一写入点，
/// 不能随交接摘要一并删除。
#[test]
fn work_item_completion_commit_is_persisted_and_readable() {
    let (_tmp, store) = setup();
    let work_item = store
        .create_work_item(CreateWorkItemInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "Work item with completion commit".to_string(),
            ..Default::default()
        })
        .unwrap();

    let updated = store
        .update_work_item_completion_commit(
            PROJECT_ID,
            ISSUE_ID,
            &work_item.id,
            Some("abc1234".to_string()),
        )
        .expect("write completion commit");
    assert_eq!(updated.completion_commit.as_deref(), Some("abc1234"));

    let reloaded = store
        .list_work_items(PROJECT_ID, ISSUE_ID)
        .expect("list work items")
        .into_iter()
        .find(|item| item.id == work_item.id)
        .expect("work item exists");
    assert_eq!(
        reloaded.completion_commit.as_deref(),
        Some("abc1234"),
        "完成 commit 必须持久化并可读"
    );
}
