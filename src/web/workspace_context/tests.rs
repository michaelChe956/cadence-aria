use super::builder::ensure_workspace_context_message;
use super::prompts::{output_schema_for, runtime_contract_for, workflow_discipline_for};
use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::lifecycle_store::{
    AppendSpecVersionInput, CreateDesignSpecInput, CreateIssueWorkItemPlanInput,
    CreateStorySpecInput, CreateWorkItemInput, CreateWorkspaceSessionInput, LifecycleStore,
};
use crate::product::models::{
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, LifecycleConfirmationStatus, ProviderName,
    WorkspaceMessageRecord, WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use tempfile::tempdir;

#[test]
fn all_workspace_artifact_outputs_require_artifact_fence() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
        WorkspaceType::WorkItemPlan,
    ] {
        let schema = output_schema_for(&workspace_type);
        assert!(
            schema.contains("```artifact fenced block"),
            "{workspace_type:?} output schema must require artifact fenced block"
        );
    }
}

#[test]
fn design_output_schema_uses_canonical_projection_headings() {
    let schema = output_schema_for(&WorkspaceType::Design);

    assert!(schema.contains("设计决策"));
    assert!(schema.contains("公共组件"));
    assert!(schema.contains("API 契约"));
    assert!(schema.contains("数据模型"));
    assert!(!schema.contains("关键决策"));
}

#[test]
fn story_and_design_runtime_contracts_do_not_inherit_work_item_plan_discipline() {
    for workspace_type in [WorkspaceType::Story, WorkspaceType::Design] {
        let contract = runtime_contract_for(&workspace_session_record(
            workspace_type.clone(),
            ProviderName::Codex,
        ));

        assert!(
            !contract.contains("writing-plans"),
            "{workspace_type:?} runtime contract must not mention writing-plans"
        );
        assert!(!contract.contains("必须按 writing-plans"));
        assert!(
            contract.contains("[forbidden_outputs]"),
            "{workspace_type:?} runtime contract should include explicit forbidden outputs"
        );
    }
}

#[test]
fn workspace_runtime_contract_includes_codegraph_mcp_reading_guidance() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let contract = runtime_contract_for(&workspace_session_record(
            workspace_type.clone(),
            ProviderName::ClaudeCode,
        ));

        assert!(contract.contains("CodeGraph MCP"), "{workspace_type:?}");
        assert!(
            contract.contains("mcp__codegraph__codegraph_explore"),
            "{workspace_type:?}"
        );
        assert!(contract.contains("ast-grep outline"), "{workspace_type:?}");
        assert!(contract.contains("降级"), "{workspace_type:?}");
    }
}

#[test]
fn work_item_output_schema_describes_single_task_not_issue_level_split() {
    let schema = output_schema_for(&WorkspaceType::WorkItem);

    assert!(schema.contains("实现步骤") || schema.contains("子步骤"));
    assert!(schema.contains("单个可执行任务"));
    assert!(schema.contains("20k"));
    assert!(schema.contains("禁止跨任务"));
    assert!(!schema.contains("任务拆分"));
}

#[test]
fn work_item_plan_output_schema_requires_single_session_task_sizing() {
    let schema = output_schema_for(&WorkspaceType::WorkItemPlan);

    assert!(schema.contains("任务拆分"));
    assert!(schema.contains("20k"));
    assert!(schema.contains("单个 Claude Code 或 Codex 会话"));
    assert!(schema.contains("继续拆分"));
}

#[test]
fn output_schemas_require_visible_source_id_traceability() {
    let story = output_schema_for(&WorkspaceType::Story);
    assert!(story.contains("source id") || story.contains("source ids"));
    assert!(story.contains("Issue"));
    assert!(story.contains("追踪"));

    let design = output_schema_for(&WorkspaceType::Design);
    assert!(design.contains("source id") || design.contains("source ids"));
    assert!(design.contains("Story Spec"));
    assert!(design.contains("追踪关系"));

    let work_item = output_schema_for(&WorkspaceType::WorkItem);
    assert!(work_item.contains("source id") || work_item.contains("source ids"));
    assert!(work_item.contains("Story/Design"));
    assert!(work_item.contains("追踪关系"));

    let work_item_plan = output_schema_for(&WorkspaceType::WorkItemPlan);
    assert!(work_item_plan.contains("source id") || work_item_plan.contains("source ids"));
    assert!(work_item_plan.contains("Story/Design"));
    assert!(work_item_plan.contains("追踪关系"));
}

#[test]
fn output_schemas_require_structured_interaction_decisions_in_artifacts() {
    let story = output_schema_for(&WorkspaceType::Story);
    assert!(story.contains("结构化交互"));
    assert!(story.contains("用户确认决策"));
    assert!(story.contains("author-decision"));
    assert!(story.contains("[REQ-"));
    assert!(story.contains("[AC-"));
    assert!(story.contains("AskUserQuestion"));
    assert!(story.contains("requestUserInput"));
    assert!(story.contains("待确认项"));

    let design = output_schema_for(&WorkspaceType::Design);
    assert!(design.contains("结构化交互"));
    assert!(design.contains("用户确认决策"));
    assert!(design.contains("author-decision"));
    assert!(design.contains("[DEC-"));
    assert!(design.contains("追踪关系"));

    let work_item = output_schema_for(&WorkspaceType::WorkItem);
    assert!(work_item.contains("结构化交互"));
    assert!(work_item.contains("用户确认决策"));
    assert!(work_item.contains("author-decision"));
    assert!(work_item.contains("追踪关系"));
}

#[test]
fn work_item_workflow_discipline_describes_single_task_not_task_split() {
    let guidance = workflow_discipline_for(&workspace_session_record(
        WorkspaceType::WorkItem,
        ProviderName::Codex,
    ));

    assert!(guidance.contains("单个可执行任务"));
    assert!(!guidance.contains("任务拆分"));
}

#[test]
fn fake_story_provider_uses_daemon_pause_guidance_not_fake_tool_call() {
    let guidance = workflow_discipline_for(&workspace_session_record(
        WorkspaceType::Story,
        ProviderName::Fake,
    ));

    assert!(guidance.contains("daemon"));
    assert!(guidance.contains("text_fallback"));
    assert!(!guidance.contains("必须使用结构化 AskUserQuestion"));
    assert!(!guidance.contains("必须使用结构化 requestUserInput"));
}

#[test]
fn claude_code_story_context_requires_structured_ask_user_question() {
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
        })
        .expect("story");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id,
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("session");

    let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
        .expect("workspace context");
    let context = &session.messages[0].content;

    assert!(context.contains("当前 author provider 是 Claude Code"));
    assert!(context.contains("必须使用结构化 AskUserQuestion"));
    assert!(context.contains("禁止输出文本 A/B/C 选择题"));
    assert!(context.contains("text_fallback 异常兜底"));
    assert!(context.contains("只追加 compact QA"));
}

fn workspace_session_record(
    workspace_type: WorkspaceType,
    author_provider: ProviderName,
) -> WorkspaceSessionRecord {
    WorkspaceSessionRecord {
        id: "workspace_session_test".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "entity_0001".to_string(),
        workspace_type,
        status: WorkspaceSessionStatus::Open,
        author_provider,
        reviewer_provider: ProviderName::Codex,
        review_rounds: 1,
        superpowers_enabled: true,
        openspec_enabled: true,
        provider_conversations: Vec::new(),
        messages: Vec::new(),
        created_at: "2026-06-30T00:00:00Z".to_string(),
        updated_at: "2026-06-30T00:00:00Z".to_string(),
    }
}

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
fn work_item_workspace_context_includes_linked_design_markdown() {
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
            repository_id: repository.id.clone(),
            title: "爬楼梯问题 Story Spec".to_string(),
        })
        .expect("story");
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "爬楼梯问题 Design Spec".to_string(),
        })
        .expect("design");
    lifecycle
        .append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: design.id.clone(),
            markdown: "# 爬楼梯问题 Design Spec\n\n[DEC-001] 使用迭代动态规划。".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: Some("human".to_string()),
        })
        .expect("design version");
    lifecycle
        .update_spec_confirmation_status(
            "project_0001",
            "issue_0001",
            &design.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .expect("confirm design");
    let work_item = lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            story_spec_ids: vec![story.id],
            design_spec_ids: vec![design.id],
            title: "实现爬楼梯问题".to_string(),
            ..Default::default()
        })
        .expect("work item");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: work_item.id,
            workspace_type: WorkspaceType::WorkItem,
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

    assert!(context.contains("- Design Spec: 爬楼梯问题 Design Spec (design_spec_0001)"));
    assert!(context.contains("# 爬楼梯问题 Design Spec"));
    assert!(context.contains("[DEC-001] 使用迭代动态规划。"));
    assert!(context.contains("只使用 writing-plans 的计划结构要求"));
    assert!(context.contains("不要创建 docs/superpowers/plans 文件"));
    assert!(context.contains("不要询问 Subagent-Driven 或 Inline Execution"));
}

#[test]
fn work_item_workspace_context_includes_source_draft_plan_context() {
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
        })
        .expect("repository");
    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some(repository.id.clone()),
            title: "Provider 依赖安装".to_string(),
            description: Some("检查并安装 provider CLI".to_string()),
            change_id: None,
        })
        .expect("issue");

    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id.clone(),
            title: "Provider 依赖 Story Spec".to_string(),
        })
        .expect("story");
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "Provider 依赖 Design Spec".to_string(),
        })
        .expect("design");
    let work_item = lifecycle
        .create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id,
            story_spec_ids: vec![story.id],
            design_spec_ids: vec![design.id],
            title: "Provider 依赖核心服务".to_string(),
            work_item_set_id: Some("issue_work_item_plan_0001".to_string()),
            source_work_item_plan_id: Some("issue_work_item_plan_0001".to_string()),
            source_outline_id: Some("outline_backend".to_string()),
            source_draft_id: Some("draft_backend".to_string()),
            planned_implementation_context: Some(
                "实现 provider dependency core，先写 TDD 单测。".to_string(),
            ),
            planned_handoff_summary: Some(
                "交付 ProviderDependencyService 与 provider catalog。".to_string(),
            ),
            ..Default::default()
        })
        .expect("work item");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: work_item.id,
            workspace_type: WorkspaceType::WorkItem,
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

    assert!(context.contains("[work_item_plan_source]"));
    assert!(context.contains("source_work_item_plan_id: issue_work_item_plan_0001"));
    assert!(context.contains("source_outline_id: outline_backend"));
    assert!(context.contains("source_draft_id: draft_backend"));
    assert!(context.contains("planned_implementation_context"));
    assert!(context.contains("实现 provider dependency core"));
    assert!(context.contains("planned_handoff_summary"));
    assert!(context.contains("交付 ProviderDependencyService"));
    assert!(context.contains("[openspec_contract]"));
    assert!(context.contains("[superpowers_contract]"));
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

#[test]
fn work_item_plan_context_message_includes_plan_brief_and_workspace_type() {
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
            repository_id: repository.id.clone(),
            title: "爬楼梯问题 Story Spec".to_string(),
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
    lifecycle
        .append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: design.id.clone(),
            markdown: "# 爬楼梯问题 Design Spec\n\n[DEC-001] 使用迭代动态规划。".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: Some("human".to_string()),
        })
        .expect("design version");
    lifecycle
        .update_spec_confirmation_status(
            "project_0001",
            "issue_0001",
            &design.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .expect("confirm design");
    let plan = lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: None,
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: vec![story.id],
            source_design_spec_ids: vec![design.id],
            options: IssueWorkItemPlanOptions {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("plan");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: plan.id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
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

    assert!(context.contains("候选 work item plan 生成器"));
    assert!(context.contains("Workspace 类型: Work Item Plan"));
    assert!(context.contains("runtime_role=workspace_work_item_plan"));
    assert!(context.contains("node_id=WORK_ITEM_PLAN"));
    assert!(context.contains(&plan.id));
    assert!(context.contains("```artifact fenced block"));
    assert!(context.contains("- Story Spec: 爬楼梯问题 Story Spec (story_spec_0001)"));
    assert!(context.contains("- Design Spec: 爬楼梯问题 Design Spec (design_spec_0001)"));
}
