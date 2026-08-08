use super::*;

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
            idempotency_key: "work-item-plan-context-repository".to_string(),
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
