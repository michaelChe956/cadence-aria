// B2c（Task 5.5，tasks.md 5.5 / 设计 §5.3 sc_author 行）：SC 显式 fresh 的
// 返修轮 prompt 契约——全新 provider 会话（resume=None）无旧会话历史可依赖，
// 返修上下文必须自足：[previous_source] 内嵌上一版 source 全文 + 既有
// [review_revision] findings 回灌段；正常 resume 的修订 prompt（依赖旧会话
// 记忆「上一版」）保持零变化。大文件守卫拆分至本子文件内联。

/// 最小 prompt 夹具（对齐 work_item_split_engine tests 的 split_prompt_fixture
/// 形状；本模块自持，不跨测试树取依赖）。
fn anchor_t17_prompt_fixture() -> (
    crate::web::types::GenerateWorkItemsRequest,
    crate::product::models::IssueRecord,
    crate::product::models::RepositoryRecord,
) {
    use crate::product::models::{IssuePhase, IssueRecord, IssueStatus, RepositoryRecord};
    use crate::web::types::GenerateWorkItemsRequest;
    let request = GenerateWorkItemsRequest {
        title: "test plan".to_string(),
        story_spec_ids: vec![],
        design_spec_ids: vec![],
        include_integration_tests: None,
        include_e2e_tests: None,
        force_frontend_backend_split: None,
        require_execution_plan_confirm: None,
        author_provider: None,
        reviewer_provider: None,
        review_rounds: None,
        superpowers_enabled: None,
        openspec_enabled: None,
        revision_feedback: None,
    };
    let issue = IssueRecord {
        id: "issue_0001".to_string(),
        project_id: "project_0001".to_string(),
        repo_id: None,
        logical_codebase_id: None,
        title: "Test Issue".to_string(),
        description: None,
        change_id: "change_0001".to_string(),
        phase: IssuePhase::Clarification,
        status: IssueStatus::Draft,
        active_binding_id: None,
        created_at: String::new(),
        updated_at: String::new(),
        base_branch: None,
    };
    let repository = RepositoryRecord {
        id: "repo_0001".to_string(),
        project_id: "project_0001".to_string(),
        name: "test-repo".to_string(),
        path: std::path::PathBuf::from("/tmp/repo"),
        repo_hash: "abc".to_string(),
        runtime_root: std::path::PathBuf::from("/tmp/repo"),
        default_policy_preset: "default".to_string(),
        default_provider_mode: "default".to_string(),
        created_at: String::new(),
        updated_at: String::new(),
        logical_repository_id: None,
        primary_checkout_id: None,
        identity_schema_version: 0,
    };
    (request, issue, repository)
}

/// t17 author context（静态材料；两次构造供 fresh/ordinary 各自消费）。
fn anchor_t17_author_context()
-> crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext<'static> {
    static T17_PLAN_OPTIONS: crate::product::models::IssueWorkItemPlanOptions =
        crate::product::models::IssueWorkItemPlanOptions {
            include_integration_tests: false,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: false,
        };
    crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
        story_context: "story_spec_0001: level selection",
        design_context: "design_spec_0001: levels API",
        design_requirement_ids: &[],
        repository_structure: "src/product/levels; web/src/levels; tests/integration",
        language_rules: "## 语言规则\n\n- **必须使用中文** - t17 契约夹具。\n",
        plan_options: &T17_PLAN_OPTIONS,
        routing_context: &RoutingReferenceContext::Legacy,
        member_targets: None,
        change_order: &[],
    }
}

#[test]
fn anchor_t17_fresh_revision_prompt_carries_previous_source_and_review_context() {
    let (request, issue, repository) = anchor_t17_prompt_fixture();
    let verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "t17 reviewer 意见：WI-001 契约缺口需返修".to_string(),
        summary: "t17 SC 返修轮".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::MustFix,
            message: "t17 finding: capability 缺口".to_string(),
            evidence: "WI-001 Outputs".to_string(),
            required_action: "补 capability 声明".to_string(),
            category: None,
            class_hint: None,
            contract_field: None,
            machine_repair: None,
        }],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    let previous_source =
        "# Work Item Plan\n## Work Item WI-001: Backend API\n（t17 上一版 source 全文）\n";

    let fresh = crate::product::work_item_split_engine::prompts::
        build_work_item_plan_markdown_fresh_revision_prompt(
            &request,
            &issue,
            &repository,
            anchor_t17_author_context(),
            &verdict,
            previous_source,
        )
        .expect("fresh revision markdown prompt");
    let output_directive = "[output] 现在仅输出完整 markdown source。";
    assert!(fresh.ends_with(output_directive));
    assert_eq!(fresh.matches(output_directive).count(), 1);
    for required in [
        "[previous_source]",
        "t17 上一版 source 全文",
        "不依赖任何历史会话",
        // review 完整上下文与既有返修段同源保留。
        "[review_revision]",
        "t17 reviewer 意见：WI-001 契约缺口需返修",
        "t17 SC 返修轮",
        "[review_findings]",
        "t17 finding: capability 缺口",
        "补 capability 声明",
        "[revision_directives]",
    ] {
        assert!(
            fresh.contains(required),
            "fresh 返修 prompt 必须自足携带 {required}；prompt 尾部：{}",
            &fresh[fresh.len().saturating_sub(600)..]
        );
    }
    // previous_source 小节必须先于 review 回灌段（先看旧稿再逐条修复）。
    let previous_pos = fresh
        .find("[previous_source]")
        .expect("previous_source section");
    let review_pos = fresh
        .find("[review_revision]")
        .expect("review_revision section");
    assert!(previous_pos < review_pos);

    // 正常 resume 修订 prompt 零变化：不携带 previous_source 小节（依赖旧会话
    // 上下文），findings 回灌语义保留。
    let ordinary =
        crate::product::work_item_split_engine::prompts::
            build_work_item_plan_markdown_revision_prompt(
                &request, &issue, &repository, anchor_t17_author_context(), &verdict,
            )
        .expect("ordinary revision markdown prompt");
    assert!(
        !ordinary.contains("[previous_source]"),
        "正常 resume 修订 prompt 不得携带 previous_source 小节（行为零变化）"
    );
    assert!(ordinary.contains("t17 finding: capability 缺口"));
}

#[test]
fn anchor_t17_fresh_revision_prompt_keeps_hard_budget() {
    let (request, issue, repository) = anchor_t17_prompt_fixture();
    let plan_options = crate::product::models::IssueWorkItemPlanOptions {
        include_integration_tests: false,
        include_e2e_tests: false,
        force_frontend_backend_split: false,
        require_execution_plan_confirm: false,
    };
    let context =
        crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
            story_context: "story",
            design_context: "design",
            design_requirement_ids: &[],
            repository_structure: "src",
            language_rules: "规则",
            plan_options: &plan_options,
            routing_context: &RoutingReferenceContext::Legacy,
            member_targets: None,
            change_order: &[],
        };
    let verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "comments".to_string(),
        summary: "summary".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    let oversized_previous_source = "x".repeat(
        crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES,
    );
    let error = crate::product::work_item_split_engine::prompts::
        build_work_item_plan_markdown_fresh_revision_prompt(
            &request,
            &issue,
            &repository,
            context,
            &verdict,
            &oversized_previous_source,
        )
        .expect_err("oversized previous source must hit the hard prompt budget");
    assert!(
        error.contains("exceeds hard budget"),
        "预算错误必须可诊断，got: {error}"
    );
}
