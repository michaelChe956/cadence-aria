// ---------------------------------------------------------------------------
// F-23（跨重启僵尸 run）：manager 创建面恢复
// ---------------------------------------------------------------------------

fn zombie_author_run_node() -> crate::web::workspace_ws_types::TimelineNode {
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use crate::product::models::WorkspaceRolePermissionModes;
    use crate::web::workspace_ws_handler::ProviderName;
    use crate::web::workspace_ws_types::{
        ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType,
        WorkspaceStage as WsWorkspaceStage,
    };

    TimelineNode {
        node_id: "timeline_node_002".to_string(),
        node_type: TimelineNodeType::AuthorRun,
        agent: Some(ProviderName::ClaudeCode),
        stage: WsWorkspaceStage::Running,
        round: Some(1),
        status: TimelineNodeStatus::Active,
        title: "Author 生成".to_string(),
        summary: None,
        started_at: "2026-09-20T08:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 0,
            permission_modes: WorkspaceRolePermissionModes {
                author: ProviderPermissionMode::Auto,
                reviewer: ProviderPermissionMode::Auto,
            },
        },
        retry: None,
    }
}

/// F-23 现场锚（0482 三小时僵尸）：跨服务器重启后 durable 会话停在
/// running + active author_run 节点，无任何恢复臂触达——manager 创建时必须
/// 落 AbortedByDisconnect 终态并整流回 prepare_context（retry 面可恢复）。
#[tokio::test]
async fn manager_creation_recovers_stale_running_session_after_process_restart() {
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::lifecycle_store::{
        CreateStorySpecInput, CreateWorkspaceSessionInput, LifecycleStore,
    };
    use crate::product::models::WorkspaceType;
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
    use crate::product::workspace_engine::WorkspaceStage;
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;
    use crate::web::workspace_ws_handler::ProviderName;
    use crate::web::workspace_ws_types::{TimelineNodeStatus, TimelineNodeType};

    let root = tempfile::tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let project = ProjectStore::new(app_paths.clone())
        .create(CreateProjectInput {
            name: "f23 zombie recovery".to_string(),
            description: None,
        })
        .expect("project");
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: project.id.clone(),
            name: "f23 repository".to_string(),
            path: root.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: Some("fake".to_string()),
            idempotency_key: "f23-zombie-recovery-repository".to_string(),
        })
        .expect("repository");
    let issue = IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: Some(repository.id.clone()),
            logical_codebase_id: None,
            title: "f23 zombie issue".to_string(),
            description: None,
            change_id: None,
        })
        .expect("issue");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: project.id.clone(),
            issue_id: issue.id.clone(),
            repository_id: repository.id,
            title: "f23 zombie story".to_string(),
            aggregate_codebase: None,
        })
        .expect("story");
    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: project.id.clone(),
            issue_id: issue.id.clone(),
            entity_id: story.id,
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("workspace session");
    lifecycle
        .save_timeline_nodes(&session_record.id, &[zombie_author_run_node()])
        .expect("persist zombie running timeline");

    let manager = WorkspaceSessionManager::create(
        &WebAppState::with_provider_registry(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            ProviderRegistry::new(),
        ),
        &session_record.id,
    )
    .await
    .expect("create manager for zombie session");

    let engine_arc = manager.engine();
    let engine = engine_arc.lock().await;
    assert_eq!(
        engine.current_stage(),
        WorkspaceStage::PrepareContext,
        "跨重启僵尸必须整流回 prepare_context，不得永久楔死在 running"
    );

    assert!(
        engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_id == "timeline_node_002"
                && node.status == TimelineNodeStatus::Failed),
        "僵尸 author_run 节点必须落失败终态：{:?}",
        engine
            .timeline_nodes
            .iter()
            .map(|node| (node.node_id.clone(), node.status.clone()))
            .collect::<Vec<_>>()
    );
    assert!(
        engine
            .timeline_nodes
            .iter()
            .any(|node| node.node_type == TimelineNodeType::AbortedByDisconnect),
        "恢复必须追加 AbortedByDisconnect 标记节点（retry_interrupted_run 恢复面依赖它）"
    );
}
