use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::json_store::ProductStoreError;
use cadence_aria::product::lifecycle_store::{
    AppendSpecVersionInput, CreateDesignSpecInput, CreateProjectProviderDefaultsInput,
    CreateStorySpecInput, CreateWorkItemInput, CreateWorkspaceSessionInput, LifecycleStore,
};
use cadence_aria::product::models::{
    DesignKind, LifecycleConfirmationStatus, ProviderName, WorkspaceSessionStatus, WorkspaceType,
};
use tempfile::tempdir;

#[test]
fn creates_story_design_work_item_and_versions_with_source_links() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "登录用户看到会话过期提示".to_string(),
        })
        .expect("story");
    assert_eq!(story.id, "story_spec_0001");
    assert_eq!(
        story.confirmation_status,
        LifecycleConfirmationStatus::Draft
    );

    let story_version = store
        .append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id.clone(),
            markdown: "# Story\n\n会话过期提示。".to_string(),
            provider_run_refs: vec!["run_story_0001".to_string()],
            review_refs: vec!["review_round_0001".to_string()],
            confirmed_by: None,
        })
        .expect("story version");
    assert_eq!(story_version.version, 1);

    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            design_kind: DesignKind::Frontend,
            title: "会话过期前端设计".to_string(),
        })
        .expect("design");
    assert_eq!(design.story_spec_ids, vec![story.id.clone()]);

    let work_item = store
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            design_spec_ids: vec![design.id.clone()],
            title: "实现会话过期提示".to_string(),
        })
        .expect("work item");
    assert_eq!(work_item.story_spec_ids, vec![story.id]);
    assert_eq!(work_item.design_spec_ids, vec![design.id]);
    assert_eq!(work_item.plan_status.as_str(), "not_started");
}

#[test]
fn persists_workspace_session_and_project_provider_defaults() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let defaults = store
        .upsert_project_provider_defaults(CreateProjectProviderDefaultsInput {
            project_id: "project_0001".to_string(),
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("defaults");
    assert_eq!(defaults.review_rounds, 2);

    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: false,
        })
        .expect("session");

    assert_eq!(session.id, "workspace_session_0001");
    assert_eq!(session.status, WorkspaceSessionStatus::Open);
    assert_eq!(
        store
            .list_workspace_sessions("project_0001", "issue_0001")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn append_version_uses_max_existing_version_without_overwriting_after_gap() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = LifecycleStore::new(paths.clone());

    let story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Story with version gap".to_string(),
        })
        .expect("story");

    store
        .append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id.clone(),
            markdown: "first version".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .expect("v1");
    store
        .append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id.clone(),
            markdown: "second version sentinel".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .expect("v2");

    let version_1_path = paths
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("versions")
        .join(&story.id)
        .join("version_0001.json");
    std::fs::remove_file(version_1_path).expect("remove v1");

    let version = store
        .append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id.clone(),
            markdown: "third version".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .expect("v3");

    assert_eq!(version.version, 3);
    assert_eq!(version.id, "version_0003");

    let versions = store
        .list_versions("project_0001", "issue_0001", &story.id)
        .expect("versions");
    let existing_v2 = versions
        .iter()
        .find(|version| version.id == "version_0002")
        .expect("v2 remains");
    assert_eq!(existing_v2.version, 2);
    assert_eq!(existing_v2.markdown, "second version sentinel");
}

#[test]
fn append_version_rejects_missing_spec_entity() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let error = store
        .append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_9999".to_string(),
            markdown: "orphan version".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .expect_err("missing spec should be rejected");

    assert!(matches!(
        error,
        ProductStoreError::NotFound {
            kind: "spec",
            ref id
        } if id == "story_spec_9999"
    ));
    assert!(
        store
            .list_versions("project_0001", "issue_0001", "story_spec_9999")
            .expect("versions")
            .is_empty()
    );
}

#[test]
fn list_helpers_ignore_json_directories() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = LifecycleStore::new(paths.clone());

    store
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Real story".to_string(),
        })
        .expect("story");

    let stray_dir = paths
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("story-specs")
        .join("stray.json");
    std::fs::create_dir_all(stray_dir).expect("stray json dir");

    let stories = store
        .list_story_specs("project_0001", "issue_0001")
        .expect("stories");
    assert_eq!(stories.len(), 1);
    assert_eq!(stories[0].id, "story_spec_0001");

    let next_story = store
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Next real story".to_string(),
        })
        .expect("next story");
    assert_eq!(next_story.id, "story_spec_0002");
}

#[test]
fn workspace_session_ids_are_unique_across_issues() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let first = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: false,
        })
        .expect("first session");
    let second = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0002".to_string(),
            entity_id: "story_spec_0002".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: false,
        })
        .expect("second session");
    let third = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0002".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0003".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: false,
        })
        .expect("third session");

    assert_eq!(first.id, "workspace_session_0001");
    assert_eq!(second.id, "workspace_session_0002");
    assert_eq!(third.id, "workspace_session_0003");
}
