use super::*;
use crate::product::cadence_skills::routing_reference::RoutingReferenceContext;
use crate::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateVerificationPlanInput, CreateWorkspaceSessionInput,
};
use crate::product::models::{
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, RepositoryProfileConfidence,
    VerificationCommand, VerificationCommandSafety, VerificationCommandSource,
    VerificationFallbackPolicy, VerificationScope, WorkItemKind, WorkItemPlanStatus, WorkspaceType,
};
use crate::web::workspace_ws_types::{ArtifactPayload, ArtifactVersion};
use std::fs;
use std::process::Command as StdCommand;

mod handoff_review_object;
mod plan_defect_prompt;
mod review_parser;
mod routing_contract;

use plan_defect_prompt::assert_plan_defect_output_contract;

#[test]
fn provider_projection_renderer_coding_prompt_integration_preserves_normative_context() {
    use crate::product::models::ProviderName;
    use crate::product::work_item_contract::canonical_contract_fixture;
    use crate::product::work_item_projection::{
        CoderExecutionEnvelope, WorkItemProjectionCompiler, renderer_for,
    };

    let contract = canonical_contract_fixture("wi_coding_prompt_integration");
    let projections = WorkItemProjectionCompiler
        .compile(&contract, "work_item_revision_prompt_integration")
        .unwrap();
    let envelope = CoderExecutionEnvelope {
        repository_state_ref: "repository_state_prompt_integration".to_string(),
        resolved_handoff_revision_ids: vec!["handoff_prompt_integration".to_string()],
        unit_run_id: "unit_run_prompt_integration".to_string(),
        previous_actionable_review: None,
        start_commit: Some("3333333333333333333333333333333333333333".to_string()),
    };

    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let rendered = renderer_for(&provider)
            .render_coder(&projections.coder, &envelope)
            .unwrap();

        assert!(
            rendered
                .text
                .contains("work_item_revision_prompt_integration")
        );
        assert!(rendered.text.contains("task_1"));
        assert!(rendered.text.contains("AC-001"));
        assert!(
            rendered
                .text
                .contains("repository_state_prompt_integration")
        );
        assert!(rendered.text.contains("handoff_prompt_integration"));
        assert_plan_defect_output_contract(&rendered.text, "plan_defect_findings");
        assert_eq!(rendered.content_hash.len(), 64);
    }
}

const FIXED_STACK_TERMS: &[&str] = &[
    "pnpm",
    "node_modules",
    "tsc",
    "vitest",
    "cargo",
    "crate",
    ".rs",
    "mvn",
    "gradle",
    ".java",
    "source set",
];

fn assert_no_fixed_stack_terms(prompt: &str) {
    for term in FIXED_STACK_TERMS {
        assert!(
            !prompt.contains(term),
            "prompt should not contain fixed stack term `{term}`:\n{prompt}"
        );
    }
}

fn assert_reviewer_browser_environment_boundary(prompt: &str) {
    assert!(prompt.contains("Reviewer 非 E2E 测试边界"));
    assert!(
        prompt.contains(
            "上述测试及其所需浏览器环境的安装、配置、缺失、失败或相关证据（包括缺少证据）"
        )
    );
    assert!(prompt.contains("均不得成为 finding，也不得导致 request_changes 或 blocked"));
    assert!(prompt.contains("不得作为 verdict 或 summary 中的否决理由"));
    assert!(prompt.contains("不得成为 Coder required_action 或任何返修要求"));
    assert!(prompt.contains(
        "即使 Work Item、Design Spec、Verification Plan、handoff 或 EvaluationContextPack 提到上述测试及其所需浏览器环境"
    ));
}

#[test]
fn coding_prompt_requires_material_driven_execution_without_fixed_stack_terms() {
    let attempt = test_attempt("coding_attempt_0001");
    let context = CodingExecutionContext {
        work_item_markdown: Some(
            "# Final Compile Work Item\n\n实现配置读取，不指定语言或包管理器。".to_string(),
        ),
        verification_commands: vec!["./verify-local".to_string()],
    };

    let prompt = build_coding_prompt(
        &attempt,
        &context,
        None,
        None,
        &RoutingReferenceContext::Legacy,
    );

    assert_no_fixed_stack_terms(&prompt);
    assert!(prompt.contains("Coder 执行协议"));
    assert!(prompt.contains("[cadence_project_rules]"));
    assert!(prompt.contains("AGENTS.md"));
    assert!(prompt.contains("CLAUDE.md"));
    assert!(!prompt.contains(&["Cadence-", "skills/"].concat()));
    assert!(prompt.contains("executing-plans"));
    assert!(prompt.contains("test-driven-development"));
    assert!(!prompt.contains("cadence-workflow"));
    assert!(prompt.contains("执行清单"));
    assert!(prompt.contains("依赖初始化或环境诊断要求"));
    assert!(prompt.contains("不得用平台默认技术栈假设"));
    assert!(prompt.contains("完成报告要求"));
    assert!(!prompt.contains("Reviewer 非 E2E 测试边界"));
    assert!(!prompt.contains("上述测试及其所需浏览器环境"));
}

#[test]
fn coding_prompt_includes_final_compile_work_item_context_and_commands() {
    let attempt = test_attempt("coding_attempt_0001");
    let context = CodingExecutionContext {
        work_item_markdown: Some(
            "# Final Compile Work Item\n\n- Work Item ID: work_item_compile_001\n\n## Planned Implementation Context\n\nuse context.rs"
                .to_string(),
        ),
        verification_commands: vec![
            "cargo test --locked --lib coding_execution_context".to_string(),
        ],
    };

    let prompt = build_coding_prompt(
        &attempt,
        &context,
        None,
        None,
        &RoutingReferenceContext::Legacy,
    );

    assert!(prompt.contains("验证命令:"));
    assert!(prompt.contains("- cargo test --locked --lib coding_execution_context"));
    assert!(prompt.contains("已确认 Work Item:"));
    assert!(prompt.contains("# Final Compile Work Item"));
    assert!(prompt.contains("work_item_compile_001"));
    assert!(prompt.contains("Planned Implementation Context"));
    assert!(
        prompt.contains(
            "如果 Work Item、Source Draft Supplement、Verification Plan 已明确给出某项要求"
        )
    );
    assert!(
        prompt
            .contains("先列出你从 Work Item / Final Compile / Verification Plan 提取出的执行清单")
    );
}

#[test]
fn coding_delta_prompt_requires_material_driven_rework_without_fixed_stack_terms() {
    let attempt = test_attempt("coding_attempt_0001");
    let context = CodingExecutionContext::default();

    let prompt = build_coding_delta_prompt(
        &attempt,
        &context,
        None,
        None,
        &RoutingReferenceContext::Legacy,
    );

    assert_no_fixed_stack_terms(&prompt);
    assert!(prompt.contains("Coder 增量执行协议"));
    assert!(prompt.contains("[cadence_project_rules]"));
    assert!(prompt.contains("AGENTS.md"));
    assert!(prompt.contains("CLAUDE.md"));
    assert!(!prompt.contains(&["Cadence-", "skills/"].concat()));
    assert!(prompt.contains("executing-plans"));
    assert!(prompt.contains("test-driven-development"));
    assert!(!prompt.contains("cadence-workflow"));
    assert!(prompt.contains("人工修复意见优先级最高"));
    assert!(prompt.contains("reviewer findings"));
    assert!(prompt.contains("不得引入平台默认技术栈假设"));
    assert!(!prompt.contains("Reviewer 非 E2E 测试边界"));
    assert!(!prompt.contains("上述测试及其所需浏览器环境"));
}

#[test]
fn coding_prompts_require_completion_self_check_contract() {
    let attempt = test_attempt("coding_attempt_0001");
    let context = CodingExecutionContext::default();

    let full_prompt = build_coding_prompt(
        &attempt,
        &context,
        None,
        None,
        &RoutingReferenceContext::Legacy,
    );
    let delta_prompt = build_coding_delta_prompt(
        &attempt,
        &context,
        None,
        None,
        &RoutingReferenceContext::Legacy,
    );

    for prompt in [full_prompt, delta_prompt] {
        assert!(prompt.contains("完成报告要求"));
        assert!(prompt.contains("粘贴每条验证命令的完整输出"));
        assert!(prompt.contains("0 tests"));
        assert!(prompt.contains("running 0 tests"));
        assert!(prompt.contains("如果测试命令显示没有测试被执行"));
        assert!(prompt.contains("不能直接视为已覆盖"));
        assert!(prompt.contains("git diff --stat"));
        assert!(prompt.contains("git status --short"));
        assert!(prompt.contains("未跟踪文件"));
        assert!(prompt.contains("允许范围外"));
    }
}

#[test]
fn coding_prompt_preserves_stack_terms_when_they_come_from_work_item_material() {
    let attempt = test_attempt("coding_attempt_0001");
    let context = CodingExecutionContext {
        work_item_markdown: Some(
            "# Final Compile Work Item\n\n验证命令必须运行 `cargo fmt --check` 和 `mvn test`。"
                .to_string(),
        ),
        verification_commands: vec!["cargo fmt --check".to_string(), "mvn test".to_string()],
    };

    let prompt = build_coding_prompt(
        &attempt,
        &context,
        None,
        None,
        &RoutingReferenceContext::Legacy,
    );

    assert!(prompt.contains("cargo fmt --check"));
    assert!(prompt.contains("mvn test"));
    assert!(prompt.contains("已确认 Work Item"));
}

#[test]
fn reviewer_test_scope_contract_forbids_e2e_findings_without_restricting_other_tests() {
    let contract = reviewer_test_scope_contract();

    assert_no_fixed_stack_terms(contract);
    assert_reviewer_browser_environment_boundary(contract);
    assert!(contract.contains("单元测试"));
    assert!(contract.contains("非浏览器自动化的集成测试"));
    assert!(contract.contains("编译、构建、类型检查、静态分析、格式检查或 lint"));
    assert!(contract.contains("不受 Verification Plan 已列命令的严格限制"));
    assert!(contract.contains("E2E"));
    assert!(contract.contains("Playwright"));
    assert!(contract.contains("浏览器自动化测试"));
    assert!(contract.contains("request_changes 或 blocked"));
}

#[test]
fn reviewer_process_evidence_boundary_contract_excludes_unobservable_unrepairable_process_facts() {
    let contract = reviewer_process_evidence_boundary_contract();

    assert!(contract.contains("无法从当前 diff、验证命令输出、handoff 字段或人工检查结果观测"));
    assert!(contract.contains("实现完成后即使 Coder 返修也无法产出该证据"));
    assert!(contract.contains("不得创建以过程事实为目的的 finding"));
    assert!(contract.contains("不得作为 verdict 或 summary 中的否决理由"));
    assert!(contract.contains("不得成为 Coder required_action 或任何返修要求"));
    assert!(contract.contains(
        "即使 Work Item、Design Spec、Verification Plan、handoff 或 EvaluationContextPack 提到上述过程事实，也不得转换"
    ));
    assert!(contract.contains("测试文件是否存在"));
    assert!(contract.contains("测试是否覆盖需求场景"));
    assert!(contract.contains("验证命令是否真实执行且非零"));
    assert!(contract.contains("测试输出是否与实现自相矛盾"));
    assert!(contract.contains("Forbidden Write Scopes 是否被越过"));
}

#[test]
fn reviewer_material_protocols_define_evidence_kinds_without_weakening_verification_findings() {
    const NON_ZERO_TEST_EXECUTION_SEMANTICS: &str = "non_zero_test_execution 表示验证命令执行时实际运行了非零数量的测试，是当前可观测的执行结果；它不表达测试曾先失败、不表达提交顺序、不表达任何开发时序。";

    let code_review = code_review_material_protocol(&RoutingReferenceContext::Legacy);
    let group_final_review = group_final_review_material_protocol(&RoutingReferenceContext::Legacy);

    for protocol in [&code_review, &group_final_review] {
        assert!(protocol.contains(NON_ZERO_TEST_EXECUTION_SEMANTICS));
        assert!(protocol.contains("source_diff 表示最终代码状态"));
        assert!(protocol.contains("manual_check 仅表示人工检查结果"));
        assert!(protocol.contains("handoff_field 仅表示交接字段的存在与内容"));
    }

    assert!(code_review.contains("缺少 required 验证命令的执行证据，必须作为 finding 记录"));
    assert!(code_review.contains("测试输出显示没有实际测试被执行，不能把它当作有效覆盖"));
    assert!(group_final_review.contains("验证证据缺失"));
    assert!(group_final_review.contains("必须 request_changes 或 blocked"));
    assert!(
        coding_execution_protocol(&RoutingReferenceContext::Legacy)
            .contains("写代码前调用 test-driven-development")
    );
    assert!(
        coding_delta_execution_protocol(&RoutingReferenceContext::Legacy)
            .contains("写代码前调用 test-driven-development")
    );
}

#[test]
fn code_review_material_protocol_requires_material_derived_checklist() {
    let protocol = code_review_material_protocol(&RoutingReferenceContext::Legacy);

    assert!(
        protocol.contains("从“原始需求上下文”和 EvaluationContextPack 中提取本次任务的审查清单")
    );
    assert!(protocol.contains("CoderEvidencePack"));
    assert!(protocol.contains("不得重复执行 required verification commands"));
    assert!(protocol.contains("required 验证命令的执行证据"));
    assert!(protocol.contains("测试输出显示没有实际测试被执行"));
    assert!(protocol.contains("不得提出执行材料之外的技术栈默认要求"));
    assert!(protocol.contains("当前 Unit 的 completion commit 与 HandoffRevision"));
    assert!(protocol.contains("在 Code Review approve 后才生成"));
    assert!(protocol.contains("Code Review 前为空是正常状态"));
    assert!(protocol.contains("不得据此创建 finding、request_changes 或 blocked"));
    assert!(protocol.contains("首个用户可见消息必须是工作流路由回执"));
    assert!(protocol.contains("最终审查结论必须只输出一个 JSON 对象"));
    assert!(protocol.contains("不要输出 Markdown、解释、验证报告或表格"));
}

#[test]
fn review_prompts_list_exact_finding_severity_values() {
    for protocol in [
        code_review_material_protocol(&RoutingReferenceContext::Legacy),
        group_final_review_material_protocol(&RoutingReferenceContext::Legacy),
    ] {
        assert!(protocol.contains("verdict 只能使用 approve、request_changes、blocked"));
        assert!(protocol.contains("severity 只能使用 error、warning、info"));
        assert!(protocol.contains("verdict=blocked 时，阻塞 finding 使用 severity=error"));
        assert!(protocol.contains("不得使用 severity=blocked"));
    }
}

#[test]
fn coding_lifecycle_protocols_reuse_the_canonical_cadence_routing_reference() {
    for protocol in [
        coding_execution_protocol(&RoutingReferenceContext::Legacy),
        coding_delta_execution_protocol(&RoutingReferenceContext::Legacy),
        code_review_material_protocol(&RoutingReferenceContext::Legacy),
        group_final_review_material_protocol(&RoutingReferenceContext::Legacy),
    ] {
        assert!(
            protocol.contains("[cadence_project_rules]"),
            "every coding lifecycle prompt must use the shared canonical routing reference"
        );
        assert!(protocol.contains("AGENTS.md"));
        assert!(protocol.contains("CLAUDE.md"));
        assert!(!protocol.contains(&["Cadence-", "skills/"].concat()));
    }
}

#[tokio::test]
async fn code_review_prompt_uses_compiled_work_item_without_artifact_version() {
    let tmp = tempdir().expect("tempdir");
    let worktree = tmp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree dir");
    init_prompt_git_repo(&worktree);
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    let verification_plan_id = "verification_plan_0001";

    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Compiled reviewer context".to_string(),
            planned_implementation_context: Some("compiled implementation context".to_string()),
            kind: WorkItemKind::Backend,
            exclusive_write_scopes: vec!["src/product/**".to_string()],
            forbidden_write_scopes: vec!["tests/**".to_string()],
            verification_plan_ref: Some(verification_plan_id.to_string()),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create work item");
    lifecycle
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some(verification_plan_id.to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            repository_profile_ref: None,
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "cmd_compiled_context".to_string(),
                label: "compiled context test".to_string(),
                command: "cargo test --locked --lib compiled_context".to_string(),
                cwd: ".".to_string(),
                purpose: "prove reviewer loads compiled context".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: Vec::new(),
            required_gates: vec!["cmd_compiled_context".to_string()],
            risk_notes: Vec::new(),
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .expect("create verification plan");

    let store = CodingAttemptStore::new(paths);
    let (tx, _rx) = mpsc::channel(1);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let mut attempt = test_attempt("coding_attempt_0001");
    attempt.worktree_path = Some(worktree.clone());

    let prompt = engine
        .build_code_review_prompt(&attempt, &worktree, None, &RoutingReferenceContext::Legacy)
        .await
        .expect("code review prompt");

    assert!(prompt.contains("compiled implementation context"));
    assert!(prompt.contains("[cadence_project_rules]"));
    assert!(prompt.contains("AGENTS.md"));
    assert!(prompt.contains("CLAUDE.md"));
    assert!(!prompt.contains(&["Cadence-", "skills/"].concat()));
    assert!(prompt.contains("requesting-code-review"));
    assert!(prompt.contains("首个用户可见消息必须是工作流路由回执"));
    assert!(prompt.contains("最终审查结论必须只输出一个 JSON 对象"));
    assert!(prompt.contains("Reviewer 过程证据边界"));
    assert!(prompt.contains("不得创建以过程事实为目的的 finding"));
    assert!(!prompt.contains("\n只输出 JSON："));
    assert!(!prompt.contains("cadence-workflow"));
    assert!(prompt.contains("tests/**"));
    assert!(prompt.contains("cargo test --locked --lib compiled_context"));
    assert!(!prompt.contains("未找到 Work Item markdown"));
}

#[tokio::test]
async fn non_group_internal_review_prompt_includes_both_reviewer_boundaries() {
    let tmp = tempdir().expect("tempdir");
    let worktree = tmp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree dir");
    init_prompt_git_repo(&worktree);

    let store = CodingAttemptStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let (tx, _rx) = mpsc::channel(1);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let attempt = test_attempt("coding_attempt_internal_review_boundary");
    let review_request = ReviewRequest {
        id: "review_request_internal_review_boundary".to_string(),
        attempt_id: attempt.id.clone(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: attempt.base_branch.clone(),
        branch_name: attempt.branch_name.clone(),
        commit_sha: "deadbeef".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: Vec::new(),
        created_at: "2026-07-28T00:00:00Z".to_string(),
        updated_at: "2026-07-28T00:00:00Z".to_string(),
        push_error: None,
    };

    let prompt = engine
        .build_internal_pr_review_prompt(
            &attempt,
            &review_request,
            &worktree,
            None,
            &RoutingReferenceContext::Legacy,
        )
        .await
        .expect("non-group internal review prompt");

    assert!(prompt.contains("Reviewer 非 E2E 测试边界"));
    assert!(prompt.contains("Reviewer 过程证据边界"));
    assert!(prompt.contains("不得创建以过程事实为目的的 finding"));
}

#[tokio::test]
async fn group_final_review_prompt_includes_process_evidence_boundary() {
    let (_root, _store, attempt, engine, _event_rx) =
        super::plan_defect_entrypoints::prepared_group_review_fixture();

    let prompt = engine
        .build_group_internal_pr_review_prompt_for_test(&attempt)
        .await
        .expect("group final review prompt");

    assert!(prompt.contains("Reviewer 过程证据边界"));
    assert!(prompt.contains("不得创建以过程事实为目的的 finding"));
    assert!(prompt.contains("non_zero_test_execution 表示验证命令执行时实际运行了非零数量的测试，是当前可观测的执行结果；它不表达测试曾先失败、不表达提交顺序、不表达任何开发时序。"));
}

#[tokio::test]
async fn first_group_code_review_uses_base_branch_without_head_commit() {
    let tmp = tempdir().expect("tempdir");
    let worktree = tmp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree dir");
    init_prompt_git_repo(&worktree);
    let base_commit = prompt_git_stdout(&worktree, &["rev-parse", "HEAD"]);
    fs::write(worktree.join("first_unit.txt"), "first unit\n").expect("first unit file");

    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    LifecycleStore::new(paths.clone())
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "First unit".to_string(),
            planned_implementation_context: Some("review first unit".to_string()),
            ..Default::default()
        })
        .expect("create first work item");
    let store = CodingAttemptStore::new(paths);
    let (tx, _rx) = mpsc::channel(1);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let mut attempt = test_attempt("coding_attempt_0001");
    attempt.scope = CodingAttemptScope::WorkItemGroup;
    attempt.work_item_id = "work_item_0001".to_string();
    attempt.current_work_item_id = Some("work_item_0001".to_string());
    attempt.active_unit_id = Some("coding_unit_0001".to_string());
    attempt.base_branch = base_commit;
    attempt.head_commit = None;
    attempt.worktree_path = Some(worktree.clone());

    let prompt = engine
        .build_code_review_prompt(&attempt, &worktree, None, &RoutingReferenceContext::Legacy)
        .await
        .expect("first group review uses base branch");

    assert!(prompt.contains("first_unit.txt"));
}

#[tokio::test]
async fn group_code_review_uses_previous_unit_head_commit_as_diff_base() {
    let tmp = tempdir().expect("tempdir");
    let worktree = tmp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree dir");
    init_prompt_git_repo(&worktree);
    let base_commit = prompt_git_stdout(&worktree, &["rev-parse", "HEAD"]);
    fs::write(worktree.join("previous_unit.txt"), "previous unit\n").expect("previous unit file");
    run_prompt_git(&worktree, &["add", "."]);
    run_prompt_git(&worktree, &["commit", "-m", "previous unit"]);
    let previous_unit_commit = prompt_git_stdout(&worktree, &["rev-parse", "HEAD"]);
    fs::write(worktree.join("current_unit.txt"), "current unit\n").expect("current unit file");

    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    LifecycleStore::new(paths.clone())
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0002".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Current unit".to_string(),
            planned_implementation_context: Some("review current unit only".to_string()),
            ..Default::default()
        })
        .expect("create current work item");
    let store = CodingAttemptStore::new(paths);
    let (tx, _rx) = mpsc::channel(1);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let mut attempt = test_attempt("coding_attempt_0001");
    attempt.scope = CodingAttemptScope::WorkItemGroup;
    attempt.work_item_id = "work_item_0001".to_string();
    attempt.current_work_item_id = Some("work_item_0002".to_string());
    attempt.active_unit_id = Some("coding_unit_0002".to_string());
    attempt.base_branch = base_commit;
    attempt.head_commit = Some(previous_unit_commit);
    attempt.worktree_path = Some(worktree.clone());

    let prompt = engine
        .build_code_review_prompt(&attempt, &worktree, None, &RoutingReferenceContext::Legacy)
        .await
        .expect("code review prompt");

    assert!(prompt.contains("current_unit.txt"));
    assert!(!prompt.contains("previous_unit.txt"));
}

#[tokio::test]
async fn later_group_code_review_rejects_missing_head_commit() {
    let tmp = tempdir().expect("tempdir");
    let worktree = tmp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree dir");
    init_prompt_git_repo(&worktree);
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    LifecycleStore::new(paths.clone())
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0002".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Current unit".to_string(),
            planned_implementation_context: Some("review current unit only".to_string()),
            ..Default::default()
        })
        .expect("create current work item");
    let store = CodingAttemptStore::new(paths);
    let (tx, _rx) = mpsc::channel(1);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let mut attempt = test_attempt("coding_attempt_0001");
    attempt.scope = CodingAttemptScope::WorkItemGroup;
    attempt.current_work_item_id = Some("work_item_0002".to_string());
    attempt.active_unit_id = Some("coding_unit_0002".to_string());
    attempt.head_commit = None;
    attempt.worktree_path = Some(worktree.clone());

    let error = engine
        .build_code_review_prompt(&attempt, &worktree, None, &RoutingReferenceContext::Legacy)
        .await
        .expect_err("group review requires previous unit commit");

    assert!(matches!(
        error,
        CodingWorkspaceEngineError::CompletionCommitMissing(ref attempt_id)
            if attempt_id == "coding_attempt_0001"
    ));
}

#[tokio::test]
async fn group_attempt_prompts_use_current_work_item_id() {
    let tmp = tempdir().expect("tempdir");
    let worktree = tmp.path().join("worktree");
    fs::create_dir_all(&worktree).expect("worktree dir");
    init_prompt_git_repo(&worktree);
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "First stale work item".to_string(),
            planned_implementation_context: Some("stale context".to_string()),
            ..Default::default()
        })
        .expect("create stale work item");
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some("work_item_0002".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Current active work item".to_string(),
            planned_implementation_context: Some("current implementation context".to_string()),
            ..Default::default()
        })
        .expect("create current work item");
    for (entity_id, markdown) in [
        ("work_item_0001", "# First stale work item"),
        ("work_item_0002", "# Current active work item"),
    ] {
        let session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: entity_id.to_string(),
                workspace_type: WorkspaceType::WorkItem,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .expect("create work item session");
        lifecycle
            .append_artifact_version(
                &session.id,
                ArtifactVersion {
                    version: 1,
                    payload: ArtifactPayload::Markdown {
                        markdown: markdown.to_string(),
                        diff: None,
                    },
                    generated_by: ProviderName::Codex,
                    reviewed_by: Some(ProviderName::ClaudeCode),
                    review_verdict: None,
                    confirmed_by: Some("user".to_string()),
                    is_current: true,
                    created_at: "2026-07-07T00:00:00Z".to_string(),
                    source_node_id: "node_0001".to_string(),
                },
            )
            .expect("append artifact");
    }
    lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some("work_item_plan_0001".to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: IssueWorkItemPlanStatus::Confirmed,
            work_item_ids: vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .expect("create group plan");

    let store = CodingAttemptStore::new(paths);
    let (tx, _rx) = mpsc::channel(1);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let mut attempt = test_attempt("coding_attempt_0001");
    attempt.scope = CodingAttemptScope::WorkItemGroup;
    attempt.work_item_id = "work_item_0001".to_string();
    attempt.current_work_item_id = Some("work_item_0002".to_string());
    attempt.work_item_group_id = Some("work_item_plan_0001".to_string());
    attempt.active_unit_id = Some("coding_unit_0002".to_string());
    attempt.branch_name = "aria/issues/issue_0001".to_string();
    attempt.worktree_path = Some(worktree.clone());
    attempt.head_commit = Some(prompt_git_stdout(&worktree, &["rev-parse", "HEAD"]));

    let coding_prompt = build_coding_prompt(
        &attempt,
        &CodingExecutionContext {
            work_item_markdown: Some("# Current active work item".to_string()),
            verification_commands: Vec::new(),
        },
        None,
        None,
        &RoutingReferenceContext::Legacy,
    );
    assert!(coding_prompt.contains("Work Item: work_item_0002"));
    assert!(!coding_prompt.contains("Work Item: work_item_0001"));

    let review_prompt = engine
        .build_code_review_prompt(&attempt, &worktree, None, &RoutingReferenceContext::Legacy)
        .await
        .expect("code review prompt");
    assert_reviewer_browser_environment_boundary(&review_prompt);
    assert!(review_prompt.contains("Playwright"));
    assert!(review_prompt.contains("单元测试"));
    assert!(!coding_prompt.contains("Reviewer 非 E2E 测试边界"));
    assert!(review_prompt.contains("Work Item: work_item_0002"));
    assert!(review_prompt.contains("Current active work item"));
    assert!(!review_prompt.contains("Work Item: work_item_0001"));
    assert!(!review_prompt.contains("First stale work item"));
}

#[test]
fn group_final_review_material_protocol_requires_handoff_revision_checks() {
    let protocol = group_final_review_material_protocol(&RoutingReferenceContext::Legacy);

    assert!(protocol.contains("Completed Units"));
    assert!(protocol.contains("HandoffRevision"));
    assert!(protocol.contains("Provided Contracts"));
    assert!(protocol.contains("Provided Capabilities"));
    assert!(protocol.contains("ReviewRequest 已 push 的 commit"));
    assert!(protocol.contains("Forbidden Write Scopes"));
    assert!(protocol.contains("source_stage=group_final_review"));
}

#[test]
fn internal_review_prompt_requires_openspec_and_superpowers() {
    let internal_contract = provider_runtime_contract("InternalReviewer");
    assert!(internal_contract.contains("InternalReviewer"));
    assert!(internal_contract.contains("[openspec_contract]"));
    assert!(internal_contract.contains("[superpowers_contract]"));
}

fn init_prompt_git_repo(repo: &std::path::Path) {
    run_prompt_git(repo, &["init"]);
    run_prompt_git(repo, &["config", "user.email", "aria@example.com"]);
    run_prompt_git(repo, &["config", "user.name", "Aria Test"]);
    fs::write(repo.join("src.txt"), "initial\n").expect("seed file");
    run_prompt_git(repo, &["add", "."]);
    run_prompt_git(repo, &["commit", "-m", "initial"]);
}

fn run_prompt_git(cwd: &std::path::Path, args: &[&str]) {
    let output = StdCommand::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("git {} failed to start: {error}", args.join(" ")));
    if !output.status.success() {
        panic!(
            "git {} failed\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn prompt_git_stdout(cwd: &std::path::Path, args: &[&str]) -> String {
    let output = StdCommand::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("git {} failed to start: {error}", args.join(" ")));
    if !output.status.success() {
        panic!(
            "git {} failed\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
