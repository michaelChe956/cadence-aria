use std::path::PathBuf;

use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::json_store::ProductStoreError;
use cadence_aria::product::lifecycle_store::{
    AppendSpecVersionInput, CreateDesignSpecInput, CreateProjectProviderDefaultsInput,
    CreateStorySpecInput, CreateWorkItemInput, CreateWorkspaceSessionInput, LifecycleStore,
    UpsertIssueSharedWorktreeInput,
};
use cadence_aria::product::models::{
    AgentRole, DesignKind, IssueSharedWorktreeStatus, LifecycleConfirmationStatus, NodeDetail,
    ProviderConversationRef, ProviderConversationRole, ProviderName, ProviderSnapshot,
    WorkItemContextBudget, WorkItemKind, WorkItemStatus, WorkspaceSessionStatus, WorkspaceType,
};
use cadence_aria::web::workspace_ws_types::{
    ArtifactVersion, ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    WorkspaceStage,
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
            ..Default::default()
        })
        .expect("work item");
    assert_eq!(work_item.story_spec_ids, vec![story.id]);
    assert_eq!(work_item.design_spec_ids, vec![design.id]);
    assert_eq!(work_item.plan_status.as_str(), "not_started");
}

#[test]
fn updates_work_item_execution_status_and_worktree_path() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let work_item = store
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "实现爬楼梯".to_string(),
            ..Default::default()
        })
        .expect("work item");

    let updated = store
        .update_work_item_execution_status(
            "project_0001",
            "issue_0001",
            &work_item.id,
            WorkItemStatus::Coding,
        )
        .expect("update status");
    assert_eq!(updated.execution_status, WorkItemStatus::Coding);

    let updated = store
        .update_work_item_worktree_path(
            "project_0001",
            "issue_0001",
            &work_item.id,
            Some(PathBuf::from("/tmp/aria-worktree")),
        )
        .expect("update worktree path");
    assert_eq!(
        updated.worktree_path.as_deref(),
        Some(std::path::Path::new("/tmp/aria-worktree"))
    );

    let reloaded = store
        .list_work_items("project_0001", "issue_0001")
        .expect("list work items");
    assert_eq!(reloaded[0].execution_status, WorkItemStatus::Coding);
    assert_eq!(
        reloaded[0].worktree_path.as_deref(),
        Some(std::path::Path::new("/tmp/aria-worktree"))
    );
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
fn workspace_session_provider_conversations_default_for_legacy_json() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = LifecycleStore::new(paths.clone());
    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .expect("create workspace session");

    let session_path = paths
        .root()
        .join("projects/project_0001/issues/issue_0001/workspace-sessions")
        .join(format!("{}.json", session.id));
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&session_path).unwrap()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("provider_conversations");
    std::fs::write(&session_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let reloaded = store
        .get_workspace_session(&session.id)
        .expect("reload legacy session");
    assert!(reloaded.provider_conversations.is_empty());
}

#[test]
fn updates_workspace_session_provider_conversations() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = LifecycleStore::new(paths);
    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .expect("create workspace session");

    let conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "claude-author-session".to_string(),
        updated_at: "2026-06-01T00:00:00Z".to_string(),
        last_node_id: Some("node-author-1".to_string()),
    }];

    let updated = store
        .replace_workspace_provider_conversations(&session.id, conversations.clone())
        .expect("persist provider conversations");

    assert_eq!(updated.provider_conversations, conversations);
    let reloaded = store
        .get_workspace_session(&session.id)
        .expect("reload session");
    assert_eq!(reloaded.provider_conversations, conversations);
}

#[test]
fn persists_workspace_timeline_nodes_and_artifact_versions() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("session");
    let node = TimelineNode {
        node_id: "timeline_node_001".to_string(),
        node_type: TimelineNodeType::AuthorRun,
        agent: Some(ProviderName::ClaudeCode),
        stage: WorkspaceStage::Running,
        round: None,
        status: TimelineNodeStatus::Completed,
        title: "Story Spec 生成".to_string(),
        summary: Some("生成完成".to_string()),
        started_at: "2026-05-19T00:00:00Z".to_string(),
        completed_at: Some("2026-05-19T00:01:00Z".to_string()),
        duration_ms: None,
        artifact_ref: Some("artifact_current".to_string()),
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 2,
        },
    };
    let version = ArtifactVersion {
        version: 1,
        markdown: "# Story Spec".to_string(),
        generated_by: ProviderName::ClaudeCode,
        reviewed_by: Some(ProviderName::Codex),
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-05-19T00:01:00Z".to_string(),
        source_node_id: "timeline_node_001".to_string(),
    };

    store
        .save_timeline_nodes(&session.id, std::slice::from_ref(&node))
        .expect("save timeline");
    store
        .append_artifact_version(&session.id, version.clone())
        .expect("append artifact version");

    assert_eq!(store.load_timeline_nodes(&session.id).unwrap(), vec![node]);
    assert_eq!(
        store.list_artifact_versions(&session.id).unwrap(),
        vec![version]
    );
}

#[test]
fn save_and_load_node_detail() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("session");
    let detail = NodeDetail {
        node_id: "node-1".to_string(),
        session_id: session.id.clone(),
        node_type: TimelineNodeType::AuthorRun,
        status: TimelineNodeStatus::Completed,
        agent_role: Some(AgentRole::Author),
        provider: Some(ProviderSnapshot {
            name: "claude_code".to_string(),
            model: "claude-opus-4-7".to_string(),
        }),
        prompt: Some("Workspace 类型: Story Spec".to_string()),
        messages: vec![],
        streaming_content: "streaming".to_string(),
        execution_events: vec![],
        permission_events: vec![],
        verdict: None,
        artifact_ref: None,
        is_revision: false,
        base_artifact_ref: None,
        started_at: "2026-05-20T14:30:00Z".to_string(),
        ended_at: None,
    };

    store
        .save_node_detail(&session.id, "node-1", &detail)
        .expect("save node detail");
    let loaded = store
        .load_node_detail(&session.id, "node-1")
        .expect("load node detail");

    assert_eq!(loaded.node_id, "node-1");
    assert_eq!(loaded.streaming_content, "streaming");
    assert_eq!(
        store
            .list_node_detail_ids(&session.id)
            .expect("list node detail ids"),
        vec!["node-1".to_string()]
    );
}

#[test]
fn load_missing_node_detail_returns_not_found() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("session");

    let err = store.load_node_detail(&session.id, "node-x").unwrap_err();

    assert!(matches!(err, ProductStoreError::NotFound { .. }));
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

#[test]
fn workspace_session_lookup_ignores_unrelated_json_files() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = LifecycleStore::new(paths.clone());

    store
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

    let workspace_sessions_root = paths
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("workspace-sessions");
    std::fs::write(
        workspace_sessions_root.join("notes.json"),
        r#"{ "not": "a session" }"#,
    )
    .expect("write unrelated json");

    let session = store
        .get_workspace_session("workspace_session_0001")
        .expect("session lookup");
    assert_eq!(session.id, "workspace_session_0001");

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

    assert_eq!(second.id, "workspace_session_0002");
}

#[test]
fn persists_issue_shared_worktree_and_active_lock() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let shared = store
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: PathBuf::from("/tmp/repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");

    assert_eq!(shared.status, IssueSharedWorktreeStatus::Ready);
    assert_eq!(shared.current_active_work_item_id, None);

    let locked = store
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
        .expect("lock");
    assert_eq!(
        locked.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );

    let reloaded = store
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("reload")
        .expect("shared worktree exists");
    assert_eq!(
        reloaded.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );

    let released = store
        .release_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
        .expect("release");
    assert_eq!(released.current_active_work_item_id, None);
}

#[test]
fn rejects_lock_when_another_work_item_is_active() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    store
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: PathBuf::from("/tmp/repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    store
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
        .expect("first lock");

    let error = store
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0002")
        .expect_err("second lock should fail");

    assert!(format!("{error}").contains("issue_worktree_active"));
}

#[test]
fn marks_issue_shared_worktree_last_completed_work_item() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    store
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: PathBuf::from("/tmp/repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");

    let updated = store
        .mark_issue_worktree_completed_item("project_0001", "issue_0001", "work_item_0001")
        .expect("mark completed");

    assert_eq!(
        updated.last_completed_work_item_id.as_deref(),
        Some("work_item_0001")
    );
}

#[test]
fn create_work_item_persists_split_fields() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let work_item = store
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "后端 API".to_string(),
            work_item_set_id: Some("work_item_set_0001".to_string()),
            kind: WorkItemKind::Backend,
            sequence_hint: Some(10),
            depends_on: Vec::new(),
            exclusive_write_scopes: vec!["src/product/**".to_string()],
            forbidden_write_scopes: vec!["web/**".to_string()],
            context_budget: WorkItemContextBudget::default(),
            required_handoff_from: Vec::new(),
            verification_plan_ref: Some("verification_plan_work_item_0001".to_string()),
            require_execution_plan_confirm: false,
        })
        .expect("work item");

    assert_eq!(
        work_item.work_item_set_id.as_deref(),
        Some("work_item_set_0001")
    );
    assert_eq!(work_item.kind, WorkItemKind::Backend);
    assert_eq!(work_item.exclusive_write_scopes, vec!["src/product/**"]);
}
