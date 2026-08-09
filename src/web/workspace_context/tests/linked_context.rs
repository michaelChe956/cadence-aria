use super::*;

#[test]
fn story_workspace_context_codex_author_requires_request_user_input() {
    let root = tempdir().expect("root");
    let repo = tempdir().expect("repo");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "Repo".to_string(),
            path: repo.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: "codex-author-context-repository".to_string(),
        })
        .expect("repository");
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some(repository.id.clone()),
            title: "爬楼梯问题".to_string(),
            description: Some("使用 Python 实现 climb_stairs".to_string()),
            change_id: None,
        })
        .expect("issue");

    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            title: "爬楼梯问题 Story Spec".to_string(),
            aggregate_codebase: None,
        })
        .expect("story");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id,
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("session");

    let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
        .expect("workspace context");
    let context = &session.messages[0].content;

    assert!(context.contains("当前 author provider 是 Codex"));
    assert!(context.contains("必须使用结构化 requestUserInput"));
    assert!(context.contains("禁止输出文本 1/2/3 或 A/B/C 选择题"));
    assert!(context.contains("text_fallback 异常兜底"));
}

#[test]
fn design_workspace_context_includes_linked_story_markdown() {
    let root = tempdir().expect("root");
    let repo = tempdir().expect("repo");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "Repo".to_string(),
            path: repo.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: "linked-story-context-repository".to_string(),
        })
        .expect("repository");
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some(repository.id.clone()),
            title: "爬楼梯问题".to_string(),
            description: Some("使用 Python 实现 climb_stairs".to_string()),
            change_id: None,
        })
        .expect("issue");

    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            title: "爬楼梯问题 Story Spec".to_string(),
            aggregate_codebase: None,
        })
        .expect("story");
    lifecycle
        .append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id.clone(),
            markdown: "# 爬楼梯问题 Story Spec\n\n[REQ-001] 返回爬楼梯方法数。".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: Some("human".to_string()),
        })
        .expect("story version");
    lifecycle
        .update_spec_confirmation_status(
            "project_0001",
            "issue_0001",
            &story.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .expect("confirm story");
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "爬楼梯问题 Design Spec".to_string(),
        })
        .expect("design");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: design.id,
            workspace_type: WorkspaceType::Design,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("session");

    let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
        .expect("workspace context");
    let context = &session.messages[0].content;

    assert!(context.contains("- Story Spec: 爬楼梯问题 Story Spec (story_spec_0001)"));
    assert!(context.contains("当前版本: v1"));
    assert!(context.contains("````markdown"));
    assert!(context.contains("# 爬楼梯问题 Story Spec"));
    assert!(context.contains("[REQ-001] 返回爬楼梯方法数。"));
}

#[test]
fn work_item_workspace_context_rejects_a_missing_runtime_binding() {
    let root = tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "wi_library_export".to_string(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("session");

    let error = ensure_workspace_context_message(&app_paths, &lifecycle, session).unwrap_err();

    assert!(matches!(
        error,
        crate::product::json_store::ProductStoreError::IdentityMismatch {
            kind: "runtime_binding_missing",
            ..
        }
    ));
}

#[test]
fn work_item_workspace_context_does_not_create_a_legacy_context_on_binding_failure() {
    let root = tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "wi_library_export".to_string(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("session");

    let error =
        ensure_workspace_context_message(&app_paths, &lifecycle, session.clone()).unwrap_err();

    assert!(matches!(
        error,
        crate::product::json_store::ProductStoreError::IdentityMismatch {
            kind: "runtime_binding_missing",
            ..
        }
    ));
    assert!(
        lifecycle
            .get_workspace_session(&session.id)
            .unwrap()
            .messages
            .is_empty(),
        "Binding 缺失时不得持久化旧 Work Item Context"
    );
}

#[test]
fn existing_generation_brief_is_refreshed_when_linked_context_changes() {
    let root = tempdir().expect("root");
    let repo = tempdir().expect("repo");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "Repo".to_string(),
            path: repo.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: "linked-context-repository".to_string(),
        })
        .expect("repository");
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some(repository.id.clone()),
            title: "爬楼梯问题".to_string(),
            description: Some("使用 Python 实现 climb_stairs".to_string()),
            change_id: None,
        })
        .expect("issue");

    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            title: "爬楼梯问题 Story Spec".to_string(),
            aggregate_codebase: None,
        })
        .expect("story");
    lifecycle
        .append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id.clone(),
            markdown: "# 爬楼梯问题 Story Spec\n\n[REQ-001] 返回爬楼梯方法数。".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: Some("human".to_string()),
        })
        .expect("story version");
    lifecycle
        .update_spec_confirmation_status(
            "project_0001",
            "issue_0001",
            &story.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .expect("confirm story");
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: vec![story.id],
            title: "爬楼梯问题 Design Spec".to_string(),
        })
        .expect("design");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: design.id,
            workspace_type: WorkspaceType::Design,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("session");
    let stale_messages = vec![
        WorkspaceMessageRecord {
            role: "system".to_string(),
            content: "Workspace 生成任务已准备\n\n[system]\n你是 Aria 的候选 design 生成器。\n\n关联上下文:\n- Story Spec: 爬楼梯问题 Story Spec (story_spec_0001)".to_string(),
            created_at: "2026-05-27T00:00:00Z".to_string(),
        },
        WorkspaceMessageRecord {
            role: "user".to_string(),
            content: "开始生成 Design Spec".to_string(),
            created_at: "2026-05-27T00:00:01Z".to_string(),
        },
    ];
    let session = lifecycle
        .replace_workspace_messages(&session.id, stale_messages)
        .expect("replace stale messages");

    let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
        .expect("workspace context");

    assert_eq!(session.messages.len(), 2);
    assert_eq!(session.messages[1].content, "开始生成 Design Spec");
    assert!(
        session.messages[0]
            .content
            .contains("# 爬楼梯问题 Story Spec")
    );
    assert!(
        session.messages[0]
            .content
            .contains("[REQ-001] 返回爬楼梯方法数。")
    );
}
