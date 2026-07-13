use super::*;
use crate::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateVerificationPlanInput, CreateWorkspaceSessionInput,
};
use crate::product::models::{
    IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, RepositoryProfileConfidence,
    VerificationCommand, VerificationCommandSafety, VerificationCommandSource,
    VerificationFallbackPolicy, VerificationScope, WorkItemKind, WorkItemPlanStatus,
};
use crate::web::workspace_ws_types::{ArtifactPayload, ArtifactVersion};
use std::fs;
use std::process::Command as StdCommand;

#[test]
fn derive_reason_code_prefers_explicit() {
    let report = blocked_report_with(Vec::new(), vec!["S018".to_string()]);
    let reason = derive_testing_blocked_reason_code(
        Some("high_risk_test_step_requires_permission".to_string()),
        &report,
    );
    assert_eq!(reason, "high_risk_test_step_requires_permission");
}

#[test]
fn derive_reason_code_uses_missing_when_present() {
    let report = blocked_report_with(vec!["unit".to_string()], vec!["S018".to_string()]);
    assert_eq!(
        derive_testing_blocked_reason_code(None, &report),
        "missing_required_steps"
    );
}

#[test]
fn derive_reason_code_uses_skipped_when_only_skipped() {
    let report = blocked_report_with(Vec::new(), vec!["S018".to_string(), "S027".to_string()]);
    assert_eq!(
        derive_testing_blocked_reason_code(None, &report),
        "skipped_required_steps"
    );
}

#[test]
fn derive_reason_code_falls_back_to_testing_blocked() {
    let report = blocked_report_with(Vec::new(), Vec::new());
    assert_eq!(
        derive_testing_blocked_reason_code(None, &report),
        "testing_blocked"
    );
}

#[test]
fn testing_result_review_gate_copy_routes_to_code_reviewer() {
    let accept_action = testing_result_review_gate_actions()
        .into_iter()
        .find(|action| action.action_id == "accept_testing_result")
        .expect("accept testing result action");
    assert_eq!(accept_action.label, "结果可用，进入 Code Reviewer");

    let mut report = blocked_report_with(Vec::new(), Vec::new());
    report.overall_status = TestingOverallStatus::Passed;
    report.plan_summary = None;
    let description = testing_result_review_description(&report);
    assert!(description.contains("进入 Code Reviewer"));
    assert!(!description.contains("Analyst"));
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

    let prompt = build_coding_prompt(&attempt, &context, None, None);

    assert_no_fixed_stack_terms(&prompt);
    assert!(prompt.contains("Coder 执行协议"));
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

    let prompt = build_coding_prompt(&attempt, &context, None, None);

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

    let prompt = build_coding_delta_prompt(&attempt, &context, None, None);

    assert_no_fixed_stack_terms(&prompt);
    assert!(prompt.contains("Coder 增量执行协议"));
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

    let full_prompt = build_coding_prompt(&attempt, &context, None, None);
    let delta_prompt = build_coding_delta_prompt(&attempt, &context, None, None);

    for prompt in [full_prompt, delta_prompt] {
        assert!(prompt.contains("完成报告要求"));
        assert!(prompt.contains("粘贴每条验证命令的完整输出"));
        assert!(prompt.contains("0 tests"));
        assert!(prompt.contains("running 0 tests"));
        assert!(prompt.contains("如果测试命令显示没有测试被执行"));
        assert!(prompt.contains("不能直接视为已覆盖"));
        assert!(prompt.contains("git diff --stat"));
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

    let prompt = build_coding_prompt(&attempt, &context, None, None);

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
fn code_review_material_protocol_requires_material_derived_checklist() {
    let protocol = code_review_material_protocol();

    assert!(
        protocol.contains("从“原始需求上下文”和 EvaluationContextPack 中提取本次任务的审查清单")
    );
    assert!(protocol.contains("CoderEvidencePack"));
    assert!(protocol.contains("不得重复执行 required verification commands"));
    assert!(protocol.contains("required 验证命令的执行证据"));
    assert!(protocol.contains("测试输出显示没有实际测试被执行"));
    assert!(protocol.contains("不得提出执行材料之外的技术栈默认要求"));
    assert!(protocol.contains("Return ONLY a single JSON object"));
    assert!(protocol.contains("No markdown, no explanations"));
}

#[test]
fn review_prompts_list_exact_finding_severity_values() {
    for protocol in [
        code_review_material_protocol(),
        group_final_review_material_protocol(),
    ] {
        assert!(protocol.contains("verdict 只能使用 approve、request_changes、blocked"));
        assert!(protocol.contains("severity 只能使用 error、warning、info"));
        assert!(protocol.contains("verdict=blocked 时，阻塞 finding 使用 severity=error"));
        assert!(protocol.contains("不得使用 severity=blocked"));
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
            planned_handoff_summary: Some("compiled handoff context".to_string()),
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
        .build_code_review_prompt(&attempt, &worktree, None)
        .await
        .expect("code review prompt");

    assert!(prompt.contains("compiled implementation context"));
    assert!(prompt.contains("tests/**"));
    assert!(prompt.contains("cargo test --locked --lib compiled_context"));
    assert!(!prompt.contains("未找到 Work Item markdown"));
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
            planned_handoff_summary: Some("stale handoff".to_string()),
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
            planned_handoff_summary: Some("current handoff".to_string()),
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

    let coding_prompt = build_coding_prompt(
        &attempt,
        &CodingExecutionContext {
            work_item_markdown: Some("# Current active work item".to_string()),
            verification_commands: Vec::new(),
        },
        None,
        None,
    );
    assert!(coding_prompt.contains("Work Item: work_item_0002"));
    assert!(!coding_prompt.contains("Work Item: work_item_0001"));

    let review_prompt = engine
        .build_code_review_prompt(&attempt, &worktree, None)
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
fn group_final_review_material_protocol_requires_group_handoff_checks() {
    let protocol = group_final_review_material_protocol();

    assert!(protocol.contains("Completed Units"));
    assert!(protocol.contains("unit handoff"));
    assert!(protocol.contains("ReviewRequest 已 push 的 commit"));
    assert!(protocol.contains("Forbidden Write Scopes"));
    assert!(protocol.contains("source_stage=group_final_review"));
}

#[test]
fn review_parser_preserves_findings_with_common_aliases() {
    let payload = r#"{
      "verdict": "request_changes",
      "summary": "needs changes",
      "findings": [
        {
          "file": "src/lib.rs",
          "line": 42,
          "description": "missing validation",
          "recommendation": "add validation"
        }
      ]
    }"#;

    let parsed = parse_review_payload(payload, CodingExecutionStage::CodeReview);

    assert_eq!(parsed.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(
        parsed.findings[0].severity,
        crate::product::coding_models::FindingSeverity::Warning
    );
    assert_eq!(
        parsed.findings[0].source_stage,
        CodingExecutionStage::CodeReview
    );
    assert_eq!(parsed.findings[0].file_path.as_deref(), Some("src/lib.rs"));
    assert_eq!(parsed.findings[0].message, "missing validation");
    assert_eq!(
        parsed.findings[0].required_action.as_deref(),
        Some("add validation")
    );
}

#[test]
fn review_parser_accepts_blocked_finding_severity_as_error() {
    let payload = r#"{
      "verdict": "blocked",
      "summary": "dependency handoff blocker",
      "findings": [
        {
          "severity": "blocked",
          "file_path": "src/web/runtime/provider.rs",
          "line": 109,
          "message": "shared gate is not wired",
          "required_action": "inject the shared gate",
          "source_stage": "code_review"
        }
      ]
    }"#;

    let parsed = parse_review_payload(payload, CodingExecutionStage::CodeReview);

    assert_eq!(parsed.verdict, ReviewVerdict::Blocked);
    assert_eq!(parsed.summary, "dependency handoff blocker");
    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(
        parsed.findings[0].severity,
        crate::product::coding_models::FindingSeverity::Error
    );
    assert_eq!(parsed.findings[0].message, "shared gate is not wired");
}

#[test]
fn review_parser_distinguishes_schema_error_from_json_syntax_error() {
    let schema_error = parse_review_payload(
        r#"{"verdict":"blocked","findings":[{"severity":"unexpected"}]}"#,
        CodingExecutionStage::CodeReview,
    );
    assert!(schema_error.summary.contains("review JSON Schema 校验失败"));
    assert!(schema_error.summary.contains("unknown variant"));

    let syntax_error = parse_review_payload(
        r#"{"verdict":"blocked","findings":["#,
        CodingExecutionStage::CodeReview,
    );
    assert!(syntax_error.summary.contains("review 输出不是有效 JSON"));
    assert!(!syntax_error.summary.contains("Schema 校验失败"));
}

#[test]
fn review_parser_accepts_fenced_json_with_reviewer_blocker_severity() {
    let payload = r#"Reviewer summary before the structured payload.

```json
{
  "verdict": "request_changes",
  "summary": "orphaned modules",
  "findings": [
    {
      "severity": "blocker",
      "file_path": "src/web/mod.rs",
      "message": "module is not wired",
      "required_action": "declare the module"
    }
  ]
}
```"#;

    let parsed = parse_review_payload(payload, CodingExecutionStage::CodeReview);

    assert_eq!(parsed.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(parsed.summary, "orphaned modules");
    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(
        parsed.findings[0].severity,
        crate::product::coding_models::FindingSeverity::Error
    );
    assert_eq!(
        parsed.findings[0].required_action.as_deref(),
        Some("declare the module")
    );
}

#[test]
fn review_parser_accepts_group_final_review_source_stage_alias() {
    let payload = r#"{
      "verdict": "request_changes",
      "summary": "group final review found one issue",
      "findings": [
        {
          "severity": "error",
          "file_path": "src/group.rs",
          "message": "handoff contract is not closed",
          "source_stage": "group_final_review"
        }
      ]
    }"#;

    let parsed = parse_review_payload(payload, CodingExecutionStage::InternalPrReview);

    assert_eq!(parsed.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(
        parsed.findings[0].source_stage,
        CodingExecutionStage::InternalPrReview
    );
}

#[test]
fn internal_review_prompt_requires_openspec_and_superpowers() {
    let internal_contract = provider_runtime_contract("InternalReviewer");
    assert!(internal_contract.contains("InternalReviewer"));
    assert!(internal_contract.contains("[openspec_contract]"));
    assert!(internal_contract.contains("[superpowers_contract]"));
}

#[test]
fn dangerous_test_plan_step_requires_permission_or_blocks() {
    let plan = TestPlan {
        id: "test_plan_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        role_run_id: None,
        run_no: None,
        summary: "dangerous checks".to_string(),
        context_warnings: Vec::new(),
        assumptions: Vec::new(),
        steps: vec![crate::product::coding_models::TestPlanStep {
            id: "destructive".to_string(),
            title: "destructive command".to_string(),
            intent: "should require approval".to_string(),
            required: true,
            tool: crate::product::coding_models::TestPlanTool::RunCommand,
            risk_level: crate::product::coding_models::TestPlanRiskLevel::High,
            command_or_tool_input: serde_json::json!({
                "command": ["rm", "-rf", "/tmp/some-target"]
            }),
            evidence_expectation: "must not run without approval".to_string(),
            related_requirements: Vec::new(),
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
        }],
        created_at: "2026-06-10T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
    };
    let call = ProviderToolCall {
        id: "run_command_0001".to_string(),
        tool_name: "run_command".to_string(),
        input: serde_json::json!({
            "step_id": "destructive",
            "command": ["rm", "-rf", "/tmp/some-target"]
        }),
    };

    assert_eq!(
        high_risk_test_step_block_reason(&plan, &call),
        Some("high_risk_test_step_requires_permission")
    );
}

#[test]
fn tester_execute_prompt_blocks_insufficient_test_plan_without_replanning() {
    let plan = TestPlan {
        id: "test_plan_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        role_run_id: None,
        run_no: None,
        summary: "execute fixed plan".to_string(),
        context_warnings: Vec::new(),
        assumptions: Vec::new(),
        steps: vec![crate::product::coding_models::TestPlanStep {
            id: "step_t1".to_string(),
            title: "read targeted context".to_string(),
            intent: "verify available context".to_string(),
            required: true,
            tool: crate::product::coding_models::TestPlanTool::ReadFile,
            risk_level: crate::product::coding_models::TestPlanRiskLevel::Low,
            command_or_tool_input: serde_json::json!({"path": "src/lib.rs"}),
            evidence_expectation: "source evidence".to_string(),
            related_requirements: vec!["REQ-001".to_string()],
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
        }],
        created_at: "2026-06-10T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
    };

    let prompt = build_tester_execute_plan_prompt(
        &test_attempt("coding_attempt_0001"),
        &plan,
        r#"{"source_artifacts":{"design_specs":[]}}"#,
    );

    assert!(prompt.contains("Do not generate new TestPlan steps during execute_test_plan"));
    assert!(prompt.contains("provider_analysis prefixed by \"test_plan_insufficient:\""));
    assert!(prompt.contains("mark the affected required step blocked"));
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
