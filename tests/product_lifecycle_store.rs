use cadence_aria::product::app_paths::ProductAppPaths;
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
