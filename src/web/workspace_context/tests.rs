use super::builder::ensure_workspace_context_message;
use super::prompts::{
    constraint_summary_for, output_schema_for, runtime_contract_for, workflow_discipline_for,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::cadence_skills::routing_reference::RoutingReferenceContext;
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::lifecycle_store::{
    AppendSpecVersionInput, CreateDesignSpecInput, CreateIssueWorkItemPlanInput,
    CreateStorySpecInput, CreateWorkspaceSessionInput, LifecycleStore,
};
use crate::product::models::{
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, LifecycleConfirmationStatus, ProviderName,
    WorkspaceMessageRecord, WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use tempfile::tempdir;

mod linked_context;
mod work_item_plan_context;

fn seed_legacy_project(app_paths: &ProductAppPaths) {
    ProjectStore::new(app_paths.clone())
        .create(CreateProjectInput {
            name: "workspace context fixture".to_string(),
            description: None,
            multi_repo: false,
        })
        .expect("create project");
}

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
fn design_runtime_contract_allows_abstract_traceability_but_forbids_executable_testing() {
    let contract = runtime_contract_for(&workspace_session_record(
        WorkspaceType::Design,
        ProviderName::Codex,
    ));

    for required in [
        "抽象验收可追踪性",
        "仅说明 ID/关联，不描述如何测试或分配组件/文件的测试或验证职责",
    ] {
        assert!(
            contract.contains(required),
            "missing `{required}`: {contract}"
        );
    }
    for forbidden in [
        "测试计划",
        "测试范围或场景",
        "测试文件或模块",
        "测试框架或夹具",
        "测试命令",
        "构建命令",
        "执行 checklist",
        "组件或文件的测试或验证职责分配",
    ] {
        assert!(
            contract.contains(forbidden),
            "missing `{forbidden}`: {contract}"
        );
    }

    let story_contract = runtime_contract_for(&workspace_session_record(
        WorkspaceType::Story,
        ProviderName::Codex,
    ));
    assert!(story_contract.contains("任务拆分"));

    let work_item_schema = output_schema_for(&WorkspaceType::WorkItem);
    assert!(work_item_schema.contains("验证命令"));
}

#[test]
fn story_openspec_constraint_summary_preserves_explicit_input_boundaries_as_requirements() {
    let summary = constraint_summary_for(&workspace_session_record(
        WorkspaceType::Story,
        ProviderName::Codex,
    ));

    for required in [
        "文件/模块归属",
        "复用/依赖关系",
        "自动化验证责任",
        "稳定 [REQ-*]",
        "仅在范围段提及不足以覆盖",
    ] {
        assert!(
            summary.contains(required),
            "Story OpenSpec constraint summary must preserve `{required}`: {summary}"
        );
    }
}

#[test]
fn story_openspec_constraint_summary_requires_explicit_api_ownership() {
    let summary = constraint_summary_for(&workspace_session_record(
        WorkspaceType::Story,
        ProviderName::Codex,
    ));

    assert!(
        summary.contains("具体 API/行为"),
        "Story OpenSpec constraint summary must bind an explicitly owned API or behavior to its file/module: {summary}"
    );
}

#[test]
fn story_openspec_constraint_summary_requires_exception_scope() {
    let summary = constraint_summary_for(&workspace_session_record(
        WorkspaceType::Story,
        ProviderName::Codex,
    ));

    assert!(
        summary.contains("例外、优先级或集合包含关系"),
        "Story OpenSpec constraint summary must preserve explicit exception scope: {summary}"
    );
}

#[test]
fn workspace_author_workflows_directly_reference_cadence_routing_rules() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
        WorkspaceType::WorkItemPlan,
    ] {
        let workflow = workflow_discipline_for(
            &workspace_session_record(workspace_type.clone(), ProviderName::Codex),
            &RoutingReferenceContext::Legacy,
        );

        assert!(
            workflow.contains("[cadence_project_rules]"),
            "{workspace_type:?}"
        );
        assert_eq!(
            workflow.matches("[cadence_project_rules]").count(),
            1,
            "{workspace_type:?}"
        );
        assert!(
            workflow.contains("AGENTS.md") && workflow.contains("CLAUDE.md"),
            "{workspace_type:?}"
        );
        assert!(
            !workflow.contains(&["Cadence-", "skills/"].concat()),
            "{workspace_type:?}"
        );
        assert!(
            !workflow.contains("cadence-workflow"),
            "{workspace_type:?} must not depend on cadence-workflow"
        );
    }
}

#[test]
fn workspace_author_stops_when_required_superpowers_are_unavailable() {
    let mut session = workspace_session_record(WorkspaceType::Story, ProviderName::Codex);
    session.superpowers_enabled = false;

    let workflow = workflow_discipline_for(&session, &RoutingReferenceContext::Legacy);

    assert!(workflow.contains("当前 provider 环境未启用必调 Superpowers Skill；必须停止并报告"));
    assert!(!workflow.contains("Superpowers 未启用；仍需显式说明假设"));
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
fn work_item_output_schema_describes_single_task_and_forbids_issue_level_split() {
    let schema = output_schema_for(&WorkspaceType::WorkItem);

    assert!(schema.contains("实现步骤") || schema.contains("子步骤"));
    assert!(schema.contains("40k"));
    assert!(schema.contains("50k"));
    assert!(schema.contains("单个可执行任务"));
    assert!(schema.contains("禁止跨任务"));
    assert!(!schema.contains("20k"));
    assert!(schema.contains("禁止 heading"));
    assert!(schema.contains("任务拆分"));
}

#[test]
fn work_item_plan_output_schema_requires_single_session_task_sizing() {
    let schema = output_schema_for(&WorkspaceType::WorkItemPlan);

    for required in ["40k", "50k", "最大内聚", "最少拆分", "优先合并"] {
        assert!(schema.contains(required), "missing `{required}`: {schema}");
    }
    assert!(schema.contains("单个 Claude Code 或 Codex 会话"));
    assert!(schema.contains("继续拆分"));
    assert!(!schema.contains("20k"));
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
    let guidance = workflow_discipline_for(
        &workspace_session_record(WorkspaceType::WorkItem, ProviderName::Codex),
        &RoutingReferenceContext::Legacy,
    );

    assert!(guidance.contains("单个可执行任务"));
    assert!(!guidance.contains("任务拆分"));
}

#[test]
fn fake_story_provider_uses_daemon_pause_guidance_not_fake_tool_call() {
    let guidance = workflow_discipline_for(
        &workspace_session_record(WorkspaceType::Story, ProviderName::Fake),
        &RoutingReferenceContext::Legacy,
    );

    assert!(guidance.contains("daemon"));
    assert!(guidance.contains("text_fallback"));
    assert!(!guidance.contains("必须使用结构化 AskUserQuestion"));
    assert!(!guidance.contains("必须使用结构化 requestUserInput"));
}

#[test]
fn pi_story_context_requires_ask_user_tool() {
    let guidance = workflow_discipline_for(
        &workspace_session_record(WorkspaceType::Story, ProviderName::Pi),
        &RoutingReferenceContext::Legacy,
    );

    assert!(guidance.contains("当前 author provider 是 Pi"));
    assert!(guidance.contains("使用 `ask_user` 工具提问并等待回答"));
    assert!(guidance.contains("禁止输出文本 A/B/C 选择题"));
    assert!(!guidance.contains("Pi 未声明原生结构化交互能力"));
}

#[tokio::test]
async fn claude_code_story_context_requires_structured_ask_user_question() {
    let root = tempdir().expect("root");
    let repo = tempdir().expect("repo");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    seed_legacy_project(&app_paths);
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "Repo".to_string(),
            path: repo.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: "claude-code-story-context-repository".to_string(),
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
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("session");

    let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
        .await
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
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        superpowers_enabled: true,
        openspec_enabled: true,
        work_item_runtime_binding: None,
        provider_conversations: Vec::new(),
        messages: Vec::new(),
        created_at: "2026-06-30T00:00:00Z".to_string(),
        updated_at: "2026-06-30T00:00:00Z".to_string(),
    }
}

#[test]
fn workflow_discipline_logical_declares_policy_envelope() {
    let context =
        crate::product::cadence_skills::routing_reference::RoutingReferenceContext::Logical(
            crate::product::cadence_skills::routing_reference::LogicalPolicyReference {
                policy_id: "policy/project_0001/logical_0001/3".into(),
                policy_revision: 3,
                policy_digest: "sha256:abc123".into(),
                authority_root: "/data/aria/aggregate/policy".into(),
            },
        );
    let workflow = workflow_discipline_for(
        &workspace_session_record(WorkspaceType::Story, ProviderName::Codex),
        &context,
    );

    assert!(
        workflow.contains("authority_root: /data/aria/aggregate/policy"),
        "{workflow}"
    );
    assert!(
        workflow.contains("policy_id: policy/project_0001/logical_0001/3"),
        "{workflow}"
    );
    assert!(workflow.contains("policy_revision: 3"), "{workflow}");
    assert!(workflow.contains("sha256:abc123"), "{workflow}");
    assert!(workflow.contains("不作为政策正文"), "{workflow}");
    assert!(workflow.contains("只报告阻塞"), "{workflow}");
}

#[test]
fn workflow_discipline_legacy_matches_legacy_reference() {
    let workflow = workflow_discipline_for(
        &workspace_session_record(WorkspaceType::Story, ProviderName::Codex),
        &crate::product::cadence_skills::routing_reference::RoutingReferenceContext::Legacy,
    );
    let legacy =
        crate::product::cadence_skills::routing_reference::direct_cadence_routing_rules_reference(
            &crate::product::cadence_skills::routing_reference::RoutingReferenceContext::Legacy,
        );
    assert!(workflow.contains(&legacy), "{workflow}");
    assert_eq!(workflow.matches("[cadence_project_rules]").count(), 1);
}

#[test]
fn routing_reference_context_for_project_derives_logical_from_stores() {
    let root = tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let aggregate_root = root.path().join("aggregate-root");
    // 裁决①：provider_context_root 必须可达（fs::canonicalize 探测），测试需真实建目录。
    std::fs::create_dir_all(&aggregate_root).expect("create aggregate root");
    let manifest = crate::product::logical_codebase::LogicalCodebaseManifest::new(
        "project_0001",
        aggregate_root.clone(),
        Vec::new(),
    );
    crate::product::logical_codebase::LogicalCodebaseStore::new(app_paths.clone())
        .save_manifest("project_0001", &manifest)
        .expect("save manifest");
    let artifact =
        crate::product::logical_codebase::AggregatePolicyArtifactStore::new(app_paths.clone())
            .ensure_bootstrap(&manifest)
            .expect("ensure bootstrap");

    let context = super::builder::routing_reference_context_for_project(&app_paths, "project_0001")
        .expect("routing reference context");

    match context {
        crate::product::cadence_skills::routing_reference::RoutingReferenceContext::Logical(p) => {
            assert_eq!(p.policy_id, artifact.policy_id);
            assert_eq!(p.policy_revision, artifact.revision);
            assert_eq!(p.policy_digest, artifact.digest);
            // M1：authority_root 与 gateway_factory/envelope 表示一致（canonicalize 后形态）。
            assert_eq!(
                p.authority_root,
                std::fs::canonicalize(&aggregate_root)
                    .expect("canonicalize aggregate root")
                    .to_string_lossy()
            );
        }
        crate::product::cadence_skills::routing_reference::RoutingReferenceContext::Legacy => {
            panic!("expected logical routing reference context with manifest + artifact");
        }
    }
}

#[test]
fn routing_reference_context_for_project_defaults_to_legacy_without_manifest() {
    let root = tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));

    let context = super::builder::routing_reference_context_for_project(&app_paths, "project_0001")
        .expect("routing reference context");

    assert!(matches!(
        context,
        crate::product::cadence_skills::routing_reference::RoutingReferenceContext::Legacy
    ));
}

#[test]
fn routing_reference_context_for_project_propagates_corrupted_manifest_error() {
    let root = tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let logical_root = app_paths.logical_codebase_root("project_0001");
    std::fs::create_dir_all(&logical_root).expect("create logical root");
    std::fs::write(logical_root.join("manifest.json"), "{ not valid json")
        .expect("corrupt manifest");

    let result = super::builder::routing_reference_context_for_project(&app_paths, "project_0001");

    assert!(
        result.is_err(),
        "corrupted manifest must fail-closed instead of silently falling back to Legacy: {result:?}"
    );
}

#[test]
fn routing_reference_context_for_project_propagates_corrupted_artifact_error() {
    let root = tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let aggregate_root = root.path().join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).expect("create aggregate root");
    let manifest = crate::product::logical_codebase::LogicalCodebaseManifest::new(
        "project_0001",
        aggregate_root,
        Vec::new(),
    );
    crate::product::logical_codebase::LogicalCodebaseStore::new(app_paths.clone())
        .save_manifest("project_0001", &manifest)
        .expect("save manifest");
    crate::product::logical_codebase::AggregatePolicyArtifactStore::new(app_paths.clone())
        .ensure_bootstrap(&manifest)
        .expect("ensure bootstrap");
    std::fs::write(
        app_paths.aggregate_policy_artifact_path("project_0001"),
        "{ not valid json",
    )
    .expect("corrupt artifact");

    let result = super::builder::routing_reference_context_for_project(&app_paths, "project_0001");

    assert!(
        result.is_err(),
        "corrupted aggregate policy artifact must fail-closed instead of silently falling back to Legacy: {result:?}"
    );
}

#[test]
fn routing_reference_context_for_project_fails_closed_when_provider_context_root_unreachable() {
    let root = tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let manifest = crate::product::logical_codebase::LogicalCodebaseManifest::new(
        "project_0001",
        root.path().join("does-not-exist"),
        Vec::new(),
    );
    crate::product::logical_codebase::LogicalCodebaseStore::new(app_paths.clone())
        .save_manifest("project_0001", &manifest)
        .expect("save manifest");

    let result = super::builder::routing_reference_context_for_project(&app_paths, "project_0001");

    assert!(
        result.is_err(),
        "unreachable provider_context_root must fail-closed instead of silently falling back to Legacy: {result:?}"
    );
}
