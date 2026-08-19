use std::path::PathBuf;

use crate::product::models::{
    IssuePhase, IssueRecord, IssueStatus, RepositoryRecord, WorkItemDraftCandidate,
    WorkItemDraftRecord, WorkItemDraftStatus, WorkItemGenerationMode,
};
use crate::product::work_item_split_engine::prompts::{
    WORK_ITEM_DRAFT_PROMPT_MAX_BYTES, WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES,
    build_outline_prompt, build_outline_revision_prompt, build_revision_prompt, build_split_prompt,
};
use crate::product::work_item_split_engine::schema::WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA;
use crate::product::work_item_split_engine::{
    RedoSpec, build_work_item_draft_invocation, design_context_gaps,
    extract_design_context_capabilities, parse_work_item_draft_output,
    parse_work_item_plan_outline_output,
};
use crate::web::types::GenerateWorkItemsRequest;

fn split_prompt_fixture() -> (GenerateWorkItemsRequest, IssueRecord, RepositoryRecord) {
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
    };
    let repository = RepositoryRecord {
        id: "repo_0001".to_string(),
        project_id: "project_0001".to_string(),
        name: "test-repo".to_string(),
        path: PathBuf::from("/tmp/repo"),
        repo_hash: "abc".to_string(),
        runtime_root: PathBuf::from("/tmp/repo"),
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

#[test]
fn build_split_prompt_includes_revision_feedback() {
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
        revision_feedback: Some("- [error] missing write scope\n".to_string()),
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
    };
    let repository = RepositoryRecord {
        id: "repo_0001".to_string(),
        project_id: "project_0001".to_string(),
        name: "test-repo".to_string(),
        path: PathBuf::from("/tmp/repo"),
        repo_hash: "abc".to_string(),
        runtime_root: PathBuf::from("/tmp/repo"),
        default_policy_preset: "default".to_string(),
        default_provider_mode: "default".to_string(),
        created_at: String::new(),
        updated_at: String::new(),
        logical_repository_id: None,
        primary_checkout_id: None,
        identity_schema_version: 0,
    };

    let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)", &RoutingReferenceContext::Legacy);

    assert!(
        prompt.contains("[revision_feedback]"),
        "prompt should contain revision feedback section: {prompt}"
    );
    assert!(
        prompt.contains("missing write scope"),
        "prompt should contain feedback content: {prompt}"
    );
}

#[test]
fn work_item_plan_outline_prompt_includes_runtime_contracts() {
    let (request, issue, repository) = split_prompt_fixture();

    let prompt = build_outline_prompt(
        &request,
        &issue,
        &repository,
        &["Story context [REQ-001]".to_string()],
        &["Design context [DEC-001]".to_string()],
        "src/product\nweb/src",
        &[],
        &[],
        &RoutingReferenceContext::Legacy,
    );

    assert!(prompt.contains("[openspec_contract]"));
    assert!(prompt.contains("[superpowers_contract]"));
    assert!(prompt.contains("[cadence_project_rules]"));
    assert!(prompt.contains("AGENTS.md"));
    assert!(prompt.contains("CLAUDE.md"));
    assert!(!prompt.contains(&["Cadence-", "skills/"].concat()));
    assert!(!prompt.contains("cadence-workflow"));
    assert!(prompt.contains("[allowed_outputs]"));
    assert!(prompt.contains("多任务拆解、任务追踪关系、依赖图、验收与验证建议"));
    assert!(prompt.contains("[forbidden_outputs]"));
    assert!(prompt.contains("代码实现、Story/Design 重写"));
    assert!(prompt.contains("writing-plans"));
    assert!(prompt.contains("任务拆分"));
    assert!(prompt.contains("追踪关系"));
    assert!(prompt.contains("Claude Code"));
    assert!(prompt.contains("Codex"));
    for required in [
        "40k",
        "50k",
        "最大内聚",
        "最少拆分",
        "优先合并",
        "必要外部/权限/前序结果中断点",
        "独立回滚边界",
        "独立验收边界",
        "上下文代理指标",
    ] {
        assert!(
            prompt.contains(required),
            "outline prompt must include `{required}`: {prompt}"
        );
    }
    assert!(!prompt.contains("1..19999"));
    assert!(prompt.contains("estimated_context_tokens"));
    assert!(prompt.contains("session_fit=\"fits_single_agent_session\""));
}

#[test]
fn work_item_plan_outline_revision_prompt_includes_runtime_contracts() {
    let (request, issue, _repository) = split_prompt_fixture();

    let (prompt, _nonce) =
        build_outline_revision_prompt(&request, &issue, "补齐 forbidden_write_scopes", &RoutingReferenceContext::Legacy);

    assert!(prompt.contains("[openspec_contract]"));
    assert!(prompt.contains("[superpowers_contract]"));
    assert!(prompt.contains("[allowed_outputs]"));
    assert!(prompt.contains("多任务拆解、任务追踪关系、依赖图、验收与验证建议"));
    assert!(prompt.contains("[forbidden_outputs]"));
    assert!(prompt.contains("代码实现、Story/Design 重写"));
    assert!(prompt.contains("writing-plans"));
    assert!(prompt.contains("任务拆分"));
    assert!(prompt.contains("追踪关系"));
    assert!(prompt.contains("Claude Code"));
    assert!(prompt.contains("Codex"));
    for required in [
        "40k",
        "50k",
        "最大内聚",
        "最少拆分",
        "优先合并",
        "必要外部/权限/前序结果中断点",
        "独立回滚边界",
        "独立验收边界",
        "上下文代理指标",
    ] {
        assert!(
            prompt.contains(required),
            "outline prompt must include `{required}`: {prompt}"
        );
    }
    assert!(!prompt.contains("1..19999"));
    assert!(prompt.contains("estimated_context_tokens"));
    assert!(prompt.contains("session_fit=\"fits_single_agent_session\""));
}

#[test]
fn build_outline_revision_prompt_is_delta_only() {
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
    };

    let (prompt, nonce) = build_outline_revision_prompt(
        &request,
        &issue,
        "add dependency edge between backend and frontend",
        &RoutingReferenceContext::Legacy,
    );

    assert!(
        prompt.contains("[revision_feedback]"),
        "delta prompt should contain revision feedback section: {prompt}"
    );
    assert!(
        prompt.contains("add dependency edge between backend and frontend"),
        "delta prompt should contain feedback content: {prompt}"
    );
    assert!(
        !prompt.contains("[confirmed_story_specs]"),
        "delta prompt should not repeat full story/design context: {prompt}"
    );
    assert!(
        !prompt.contains("[repository_structure_summary]"),
        "delta prompt should not repeat repository structure: {prompt}"
    );
    assert!(
        prompt.contains(&format!("nonce=\"{nonce}\"")),
        "delta prompt should include nonce sentinel: {prompt}"
    );
    assert!(
        prompt.contains("\"outline\""),
        "delta prompt should include output schema: {prompt}"
    );
}

#[test]
fn design_context_capabilities_detects_required_sections() {
    let markdown = r#"
# 技术方案

## 架构概览
系统分层说明。

## Modules
模块拆分说明。

## Tech Stack
Rust + React。

## Test Strategy
cargo test 与 vitest。

## Key Paths
- src/product
- web/src

## Dependencies / Verification
外部依赖和验证约束。
"#;

    let capabilities = extract_design_context_capabilities(markdown);

    assert!(capabilities.has_architecture);
    assert!(capabilities.has_module_breakdown);
    assert!(capabilities.has_tech_stack);
    assert!(capabilities.has_test_strategy);
    assert!(capabilities.has_key_paths);
    assert!(design_context_gaps(&capabilities).is_empty());
}

#[test]
fn legacy_design_spec_gaps_are_injected_without_blocking() {
    let markdown = r#"
# 旧版设计

## Architecture
只有架构描述。

## 模块划分
有模块拆分，但没有测试策略和关键目录。
"#;

    let capabilities = extract_design_context_capabilities(markdown);
    let gaps = design_context_gaps(&capabilities);

    assert!(capabilities.has_architecture);
    assert!(capabilities.has_module_breakdown);
    assert_eq!(
        gaps,
        vec![
            "missing_tech_stack".to_string(),
            "missing_test_strategy".to_string(),
            "missing_key_paths".to_string()
        ]
    );
}

#[test]
fn outline_author_prompt_forbids_full_work_items_and_repository_profile() {
    let (request, issue, repository) = split_prompt_fixture();
    let prompt = build_outline_prompt(
        &request,
        &issue,
        &repository,
        &["story context".to_string()],
        &["design context".to_string()],
        "(empty)",
        &["missing_test_strategy".to_string()],
        &[],
        &RoutingReferenceContext::Legacy,
    );

    assert!(prompt.contains("只能输出 WorkItemPlan Outline"));
    assert!(prompt.contains("不得输出完整 Work Item"));
    assert!(prompt.contains("不得输出 VerificationPlan"));
    assert!(prompt.contains("不得输出 repository_profile"));
    assert!(prompt.contains("不得输出 parallel_groups"));
    assert!(prompt.contains("context_blockers"));
    assert!(prompt.contains("missing_test_strategy"));
    assert!(prompt.contains("<ARIA_STRUCTURED_OUTPUT nonce=\""));
    assert!(
        prompt.contains("\"outline_id\""),
        "outline prompt schema must name the required outline item id field: {prompt}"
    );
    assert!(
        prompt.contains("不要输出 dependency_graph"),
        "outline prompt must make depends_on the only provider dependency source: {prompt}"
    );
    assert!(
        !prompt.contains("\"from_outline_id\"") && !prompt.contains("\"to_outline_id\""),
        "outline provider schema must not expose derived dependency edge fields: {prompt}"
    );
    assert!(
        prompt.contains("不要输出 implementation plan")
            || prompt.contains("不要输出 Implementation Plan"),
        "outline prompt must explicitly steer away from old implementation-plan fields: {prompt}"
    );
}

#[test]
fn outline_author_prompts_make_context_blockers_outline_alternative() {
    let (request, issue, repository) = split_prompt_fixture();
    let prompt = build_outline_prompt(
        &request,
        &issue,
        &repository,
        &["story context".to_string()],
        &["design context".to_string()],
        "(empty)",
        &["missing_test_strategy".to_string()],
        &[],
        &RoutingReferenceContext::Legacy,
    );
    let (revision_prompt, _) = build_outline_revision_prompt(&request, &issue, "补充前后端依赖边", &RoutingReferenceContext::Legacy);

    for prompt in [prompt, revision_prompt] {
        assert!(
            prompt.contains("如果能输出完整 outline，不得输出非空 context_blockers"),
            "outline prompt must forbid mixed outline/context_blockers output: {prompt}"
        );
        assert!(
            prompt.contains("只有完全无法产出 outline 时才输出 context_blockers"),
            "outline prompt must reserve context_blockers for blocker-only output: {prompt}"
        );
        assert!(
            prompt.contains("路径不确定性写入 risks 或 handoff_notes"),
            "outline prompt must steer non-blocking uncertainty into outline fields: {prompt}"
        );
    }
}

#[test]
fn outline_author_prompts_require_dependency_write_scope_partitioning() {
    let (request, issue, repository) = split_prompt_fixture();
    let prompt = build_outline_prompt(
        &request,
        &issue,
        &repository,
        &["story context".to_string()],
        &["design context".to_string()],
        "(empty)",
        &[],
        &[],
        &RoutingReferenceContext::Legacy,
    );
    let (revision_prompt, _) =
        build_outline_revision_prompt(&request, &issue, "修复 exclusive_write_scopes 重叠", &RoutingReferenceContext::Legacy);

    for prompt in [prompt, revision_prompt] {
        assert!(
            prompt.contains("依赖链上的 exclusive_write_scopes 必须互斥"),
            "outline prompt must explain dependent write scopes cannot overlap: {prompt}"
        );
        assert!(
            prompt.contains(
                "integration/e2e 测试 outline 只能拥有与实现目录不共享前缀的测试、fixtures、mock 或 CI 配置路径"
            ),
            "outline prompt must steer test outlines away from implementation scopes: {prompt}"
        );
        assert!(
            prompt.contains(
                "不要让 outline_frontend 与 outline_integration_tests 同时拥有 web/src/**"
            ),
            "outline prompt must include the common frontend/integration overlap anti-pattern: {prompt}"
        );
        assert!(
            prompt.contains("不要把 web/src/**/*.test.tsx 交给 integration/e2e outline"),
            "outline prompt must avoid colocated frontend tests as integration exclusive scopes: {prompt}"
        );
    }
}

#[test]
fn work_item_plan_prompts_require_current_scope_verification_closure() {
    let (request, issue, repository) = split_prompt_fixture();
    let initial_outline_prompt = build_outline_prompt(
        &request,
        &issue,
        &repository,
        &["story context".to_string()],
        &["design context".to_string()],
        "(empty)",
        &[],
        &[],
        &RoutingReferenceContext::Legacy,
    );
    let (revision_outline_prompt, _) =
        build_outline_revision_prompt(&request, &issue, "修复验证可达性", &RoutingReferenceContext::Legacy);
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    let draft_prompt = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation")
    .prompt;

    for prompt in [initial_outline_prompt, revision_outline_prompt, draft_prompt] {
        // C 去重后 draft 段写作“当前项 exclusive_write_scopes”（无“的”）；
        // 三种 prompt 共有的保留语义子串如下。
        assert!(
            prompt.contains("exclusive_write_scopes 和已完成 depends_on handoff 下实际可执行"),
            "Work Item Plan author prompt must require a verification closure executable at the current dependency point: {prompt}"
        );
        assert!(
            prompt.contains("后续 Work Item 才会提供的注册、接线、生成或部署"),
            "Work Item Plan author prompt must reject verification that relies on later wiring: {prompt}"
        );
    }
}

#[test]
fn outline_output_schema_makes_outline_and_context_blockers_mutually_exclusive() {
    let schema: serde_json::Value =
        serde_json::from_str(WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA).expect("schema json");

    assert!(
        schema.get("anyOf").is_none(),
        "outline schema must not allow mixed outline/context_blockers output"
    );
    let one_of = schema["oneOf"].as_array().expect("schema oneOf");
    assert_eq!(one_of.len(), 2);
    assert_eq!(
        one_of[0]["properties"]["context_blockers"]["maxItems"],
        serde_json::json!(0)
    );
    assert_eq!(
        one_of[1]["properties"]["context_blockers"]["minItems"],
        serde_json::json!(1)
    );
    assert_eq!(one_of[1]["not"]["required"], serde_json::json!(["outline"]));
    let outline_properties = schema["properties"]["outline"]["properties"]
        .as_object()
        .expect("outline properties");
    assert!(
        !outline_properties.contains_key("dependency_graph"),
        "outline provider schema must not expose dependency_graph"
    );
    let outline_required = schema["properties"]["outline"]["required"]
        .as_array()
        .expect("outline required");
    assert!(
        !outline_required.contains(&serde_json::json!("dependency_graph")),
        "outline provider schema must not require dependency_graph"
    );
    let outline_item =
        &schema["properties"]["outline"]["properties"]["work_item_outlines"]["items"];
    assert_eq!(
        outline_item["properties"]["estimated_context_tokens"]["maximum"],
        serde_json::json!(50000)
    );
    assert_eq!(
        outline_item["properties"]["session_fit"]["enum"],
        serde_json::json!(["fits_single_agent_session"])
    );
    assert!(outline_item["required"]
        .as_array()
        .expect("required array")
        .contains(&serde_json::json!("estimated_context_tokens")));
    assert!(outline_item["required"]
        .as_array()
        .expect("required array")
        .contains(&serde_json::json!("session_fit")));
}

#[test]
fn outline_parser_accepts_valid_sentinel_json() {
    let mut output = valid_outline_author_output();
    output["outline"]
        .as_object_mut()
        .expect("outline object")
        .remove("dependency_graph");

    let parsed = parse_work_item_plan_outline_output(output).expect("outline");

    assert!(parsed.context_blockers.is_empty());
    let outline = parsed.outline.expect("outline payload");
    assert_eq!(outline.work_item_outlines[0].outline_id, "outline_backend");
    assert_eq!(
        outline.work_item_outlines[0].estimated_context_tokens,
        Some(12_000)
    );
    assert_eq!(
        outline.dependency_graph[0].from_outline_id,
        "outline_backend"
    );
    assert_eq!(outline.dependency_graph[0].to_outline_id, "outline_frontend");
}

#[test]
fn outline_parser_rejects_provider_dependency_graph_field() {
    let mut output = valid_outline_author_output();
    output["outline"]["dependency_graph"] = serde_json::json!([
        {
            "from_outline_id": "outline_backend",
            "to_outline_id": "outline_frontend"
        }
    ]);
    let error = parse_work_item_plan_outline_output(output).expect_err("forbidden");

    assert_eq!(error.code, "outline_forbidden_field");
    assert!(
        error
            .message
            .contains("dependency_graph"),
        "error should identify dependency_graph, got {}",
        error.message
    );
}

#[test]
fn outline_parser_rejects_verification_plan_or_work_item_id() {
    let mut output = valid_outline_author_output();
    output["outline"]["work_item_outlines"][0]["verification_plan"] =
        serde_json::json!({"commands": []});

    let error = parse_work_item_plan_outline_output(output).expect_err("forbidden field");
    assert_eq!(error.code, "outline_forbidden_field");

    let mut output = valid_outline_author_output();
    output["outline"]["work_item_outlines"][0]["work_item_id"] =
        serde_json::json!("work_item_0001");

    let error = parse_work_item_plan_outline_output(output).expect_err("forbidden field");
    assert_eq!(error.code, "outline_forbidden_field");
}

#[test]
fn single_item_prompt_projects_direct_dependency_within_provider_budget() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    let mut accepted_candidate =
        parse_work_item_draft_output(canonical_author_output("outline_backend", "wi_backend"))
            .expect("canonical backend draft");
    accepted_candidate
        .canonical_contract_candidate
        .output_contracts[0]
        .contract_id = "SessionStatusDto".to_string();
    accepted_candidate
        .canonical_contract_candidate
        .handoff_contract
        .required_fields = vec!["handoff-required-field-sentinel".to_string()];
    accepted_candidate
        .canonical_contract_candidate
        .tasks[0]
        .statement = "task-must-not-leak-into-direct-dependency-sentinel".to_string();
    accepted_candidate
        .canonical_contract_candidate
        .acceptance_criteria[0]
        .statement = "acceptance-must-not-leak-into-direct-dependency-sentinel".to_string();
    let accepted_backend = sample_draft_record(
        "draft_backend",
        "outline_backend",
        accepted_candidate,
    );

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_frontend",
        WorkItemGenerationMode::Serial,
        &[accepted_backend],
        Some("补充错误态"),
        &RoutingReferenceContext::Legacy,
    )
    .expect("direct dependency context must stay within the provider budget");

    assert!(
        invocation.prompt.len() < WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES,
        "direct dependency prompt must stay within the quality budget: {} bytes",
        invocation.prompt.len()
    );
    assert!(invocation.prompt.contains("SessionStatusDto"));
    let direct_dependency_start = invocation
        .prompt
        .find("[直接依赖的可消费交接合同]\n")
        .expect("direct dependency section");
    let direct_dependency_end = invocation
        .prompt
        .find("[其他已 accepted draft 摘要]")
        .expect("next prompt section");
    let direct_dependency_section =
        &invocation.prompt[direct_dependency_start..direct_dependency_end];
    assert!(direct_dependency_section.contains("handoff-required-field-sentinel"));
    assert!(!direct_dependency_section.contains("task-must-not-leak-into-direct-dependency-sentinel"));
    assert!(!direct_dependency_section
        .contains("acceptance-must-not-leak-into-direct-dependency-sentinel"));
    assert!(!invocation.prompt.contains("\"project_id\""));
    assert!(!invocation.prompt.contains("\"accepted_at\""));
}

/// 生成恰好 `chars` 个汉字的中文长文本（UTF-8 每字 3 字节），用于真实规模 fixture。
fn realistic_zh(unit: &str, chars: usize) -> String {
    unit.chars().cycle().take(chars).collect()
}

/// 对齐 session_0003 实测锚点（outline JSON ~1.7KB、依赖投影 ~1.1KB）的中文 outline fixture：
/// goal ~50 汉字、scope 3×~40、non_goals 3×~30、verification_intent 3×~45、handoff_notes ~40。
fn realistic_chinese_outline_author_output() -> serde_json::Value {
    serde_json::json!({
        "outline": {
            "id": "outline_artifact_realistic",
            "project_id": "project_0001",
            "issue_id": "issue_0001",
            "source_story_spec_ids": ["story_spec_0001"],
            "source_design_spec_ids": ["design_spec_0001"],
            "strategy_summary": "先落地领域模块，再以真实规模中文单元测试收口",
            "work_item_outlines": [
                {
                    "outline_id": "outline_backend",
                    "logical_work_item_id": "wi_backend",
                    "title": "领域模块",
                    "kind": "backend",
                    "goal": "实现领域模块 API",
                    "scope": ["src/product"],
                    "non_goals": [],
                    "estimated_context_tokens": 12000,
                    "session_fit": "fits_single_agent_session",
                    "source_story_spec_ids": ["story_spec_0001"],
                    "source_design_spec_ids": ["design_spec_0001"],
                    "exclusive_write_scopes": ["src/product/**"],
                    "forbidden_write_scopes": ["web/**"],
                    "depends_on": [],
                    "verification_intent": ["cargo test --locked --lib api"],
                    "trusted_verification_commands": [{
                        "command": "cargo test --locked --lib canonical_work_item_",
                        "cwd": ".",
                        "purpose": "验证 canonical contract",
                        "source_ref": "design_spec_0001#verification"
                    }],
                    "handoff_notes": "提供 API contract"
                },
                {
                    "outline_id": "outline_unit_tests",
                    "logical_work_item_id": "wi_unit_tests",
                    "title": "真实规模中文单元测试",
                    "kind": "integration",
                    "goal": realistic_zh("为工作项拆分引擎的草稿提示词瘦身提供真实规模中文目标描述覆盖，", 50),
                    "scope": [
                        realistic_zh("覆盖提示词模板固定开销缩减后的端到端行为与既有封闭字段契约回归验证场景，", 40),
                        realistic_zh("覆盖直接依赖交接合同投影在真实中文规模下的字节预算与语义保留场景，", 40),
                        realistic_zh("覆盖可信验证命令目录为空与满载两种边界下的阻断路由与降级行为场景，", 40)
                    ],
                    "non_goals": [
                        realistic_zh("不修改路由参考全文与后端持久化状态字段的既有语义，", 30),
                        realistic_zh("不调整质量预算常量与硬兜底上限的既有取值，", 30),
                        realistic_zh("不引入面向编码者投影或评审者投影的提前渲染逻辑，", 30)
                    ],
                    "estimated_context_tokens": 14000,
                    "session_fit": "fits_single_agent_session",
                    "source_story_spec_ids": ["story_spec_0001"],
                    "source_design_spec_ids": ["design_spec_0001"],
                    "exclusive_write_scopes": ["src/product/work_item_split_engine/tests/**"],
                    "forbidden_write_scopes": ["src/product/work_item_split_engine/prompts.rs"],
                    "depends_on": ["outline_backend"],
                    "verification_intent": [
                        realistic_zh("运行定向单元测试确认缩减后提示词仍包含封闭字段契约与自检段落全文，", 45),
                        realistic_zh("运行契约回归测试确认字段白名单与唯一输出哨兵段落语义不发生回退，", 45),
                        realistic_zh("运行真实规模中文预算测试确认余量目标与质量预算阈值同时满足要求，", 45)
                    ],
                    "trusted_verification_commands": [{
                        "command": "cargo test --locked --lib work_item_split_engine",
                        "cwd": ".",
                        "purpose": "回归拆分引擎契约",
                        "source_ref": "design_spec_0001#verification"
                    }],
                    "handoff_notes": realistic_zh("交接说明包含可直接消费的合同字段、验证命令来源证据与回滚边界，", 40)
                }
            ],
            "risks": [],
            "handoff_strategy": "领域模块输出 contract 给单元测试项",
            "status": "draft"
        },
        "context_blockers": []
    })
}

#[test]
fn realistic_chinese_serial_prompt_stays_within_quality_budget() {
    // fixture 对齐 session_0003 实测锚点：current outline JSON 实测 1,891 B（目标 ~1.7KB）、
    // 直接依赖投影实测 1,119 B（目标 ~1.1KB）。
    // 阈值 = 实测 post-slim prompt 10,941 B + 800 余量 = 11,741（< 质量预算，现 12,600）；
    // A–E 瘦身分段实测合计节省 980 B（fixture 规模无关的模板固定开销）。
    let outline = parse_work_item_plan_outline_output(realistic_chinese_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    let mut accepted_candidate =
        parse_work_item_draft_output(canonical_author_output("outline_backend", "wi_backend"))
            .expect("canonical backend draft");
    accepted_candidate
        .canonical_contract_candidate
        .output_contracts[0]
        .capabilities = vec![
        realistic_zh("提供稳定可消费的会话状态读取能力与契约化数据结构字段，", 50),
        realistic_zh("提供幂等的工作项状态迁移与独立回滚边界保证能力，", 50),
        realistic_zh("提供结构化的验证结果汇总与阻断路由信号输出能力，", 50),
    ];
    accepted_candidate
        .canonical_contract_candidate
        .handoff_contract
        .required_fields = vec![
        "commit_sha".to_string(),
        "module_api_version".to_string(),
        "session_status_dto".to_string(),
        "verification_report_ref".to_string(),
        "rollback_boundary".to_string(),
        "consumer_contract_hash".to_string(),
    ];
    let accepted_module_draft = sample_draft_record(
        "draft_backend",
        "outline_backend",
        accepted_candidate,
    );

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_unit_tests",
        WorkItemGenerationMode::Serial,
        &[accepted_module_draft],
        None,
        &RoutingReferenceContext::Legacy,
    )
    .expect("realistic serial prompt must stay invocable");
    // Slimmed margin target, raised twice for canonical field contract clarity:
    // 11_741 -> 11_800 spelled out `required_evidence` as an array (+45 bytes);
    // 11_800 -> 11_900 named both verification check owners (+104 bytes) after Pi
    // emitted only `verification_plan.checks` and omitted the contract copy.
    // 11_900 -> 12_500 aligned the draft prompt with the current validator hard
    // rules (+516 bytes: mandatory operational_gate blocker when the trusted
    // catalog is empty; non-empty verbatim target_contract_refs for plan_repair
    // routes), under the raised quality budget 12_600.
    // Runtime limits are untouched (hard backstop 65_536, quality budget 12_600).
    assert!(
        invocation.prompt.len() < 12_500,
        "realistic Chinese serial prompt must stay within the slimmed margin target: {} bytes",
        invocation.prompt.len()
    );
    assert!(
        invocation.prompt.len() < WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES,
        "quality budget: {} bytes",
        invocation.prompt.len()
    );
    // 契约关键语义必须保留
    assert!(invocation.prompt.contains("[canonical_field_contract]"));
    assert!(invocation.prompt.contains("verification_plan"));
    assert!(invocation.prompt.contains("operational_gate"));
    assert!(invocation.prompt.contains("[cadence_project_rules]"));
    assert!(invocation.prompt.contains("AGENTS.md"));
    assert!(invocation.prompt.contains("CLAUDE.md"));
    assert!(
        !invocation
            .prompt
            .contains(&["Cadence-", "skills/"].concat())
    );
    assert!(invocation.prompt.contains("[直接依赖的可消费交接合同]"));
    assert!(invocation.prompt.contains("ARIA_STRUCTURED_OUTPUT"));
}

#[test]
fn serial_prompt_above_legacy_11000_limit_remains_invocable_below_hard_backstop() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    let mut accepted_candidate =
        parse_work_item_draft_output(canonical_author_output("outline_backend", "wi_backend"))
            .expect("canonical backend draft");
    accepted_candidate
        .canonical_contract_candidate
        .output_contracts[0]
        .contract_id = "SessionStatusDto".to_string();
    accepted_candidate
        .canonical_contract_candidate
        .handoff_contract
        .required_fields = vec!["handoff-required-field-sentinel".to_string()];
    accepted_candidate
        .canonical_contract_candidate
        .tasks[0]
        .statement = "task-must-not-leak-into-direct-dependency-sentinel".to_string();
    accepted_candidate
        .canonical_contract_candidate
        .acceptance_criteria[0]
        .statement = "acceptance-must-not-leak-into-direct-dependency-sentinel".to_string();
    let accepted_backend = sample_draft_record(
        "draft_backend",
        "outline_backend",
        accepted_candidate,
    );

    // 与 single_item_prompt_projects_direct_dependency_within_provider_budget 相同的 fixture；
    // 追加 6,000 字节 feedback，使总 prompt 超过旧 11,000 上限但远低于 64KB 硬兜底。
    let feedback = "f".repeat(6_000);
    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_frontend",
        WorkItemGenerationMode::Serial,
        &[accepted_backend],
        Some(&feedback),
        &RoutingReferenceContext::Legacy,
    )
    .expect("prompt above the legacy 11000-byte limit must remain invocable below the 64KB hard backstop");
    assert!(
        invocation.prompt.len() > 11_000,
        "fixture must actually exceed the legacy limit: {} bytes",
        invocation.prompt.len()
    );
    assert!(
        invocation.prompt.len() < WORK_ITEM_DRAFT_PROMPT_MAX_BYTES,
        "fixture must stay below the hard backstop: {} bytes",
        invocation.prompt.len()
    );
}

#[test]
fn serial_prompt_above_hard_backstop_is_rejected() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    let mut accepted_candidate =
        parse_work_item_draft_output(canonical_author_output("outline_backend", "wi_backend"))
            .expect("canonical backend draft");
    accepted_candidate
        .canonical_contract_candidate
        .output_contracts[0]
        .contract_id = "SessionStatusDto".to_string();
    accepted_candidate
        .canonical_contract_candidate
        .handoff_contract
        .required_fields = vec!["handoff-required-field-sentinel".to_string()];
    accepted_candidate
        .canonical_contract_candidate
        .tasks[0]
        .statement = "task-must-not-leak-into-direct-dependency-sentinel".to_string();
    accepted_candidate
        .canonical_contract_candidate
        .acceptance_criteria[0]
        .statement = "acceptance-must-not-leak-into-direct-dependency-sentinel".to_string();
    let accepted_backend = sample_draft_record(
        "draft_backend",
        "outline_backend",
        accepted_candidate,
    );

    let feedback = "f".repeat(70_000);
    let error = build_work_item_draft_invocation(
        &outline,
        "outline_frontend",
        WorkItemGenerationMode::Serial,
        &[accepted_backend],
        Some(&feedback),
        &RoutingReferenceContext::Legacy,
    )
    .expect_err("prompt above the 64KB hard backstop must fail closed");
    assert_eq!(error.code, "work_item_draft_prompt_too_large");
    assert_eq!(error.details["max_prompt_bytes"], 65_536);
    assert!(
        error.details["prompt_bytes"].as_u64().expect("prompt_bytes") >= 65_536,
        "details must report the actual prompt size: {}",
        error.details
    );
}

#[test]
fn single_item_prompt_forbids_work_item_id_and_outline_changes() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    assert!(invocation.prompt.contains("不得输出 work_item_id"));
    // B 去重后合并为单条：修改/新增/删除/重命名 Outline 全部禁止。
    assert!(invocation.prompt.contains("不得修改、新增、删除或重命名 Outline"));
    // 输出唯一性并入 nonce sentinel 条款（语义保留）。
    assert!(
        invocation
            .prompt
            .contains("返回唯一 Canonical Contract Candidate JSON")
    );
}

#[test]
fn single_item_prompt_requires_executable_plan_runtime_contracts() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    assert!(invocation.prompt.contains("[openspec_contract]"));
    assert!(invocation.prompt.contains("[superpowers_contract]"));
    assert!(invocation.prompt.contains("[allowed_outputs]"));
    assert!(invocation.prompt.contains("多任务拆解、任务追踪关系、依赖图、验收与验证建议"));
    assert!(invocation.prompt.contains("[forbidden_outputs]"));
    assert!(invocation.prompt.contains("代码实现、Story/Design 重写"));
    assert!(invocation.prompt.contains("writing-plans"));
    assert!(invocation.prompt.contains("TDD"));
    assert!(invocation.prompt.contains("implementation_context"));
    assert!(invocation.prompt.contains("canonical_contract"));
    assert!(invocation.prompt.contains("handoff_contract"));
    // verification_plan 字段契约改由 [canonical_field_contract] 简写记号承载（语义保留）。
    assert!(invocation.prompt.contains("verification_plan: obj{checks:"));
    assert!(invocation.prompt.contains("estimated_context_tokens"));
    assert!(invocation.prompt.contains("单个 Claude Code/Codex 会话"));
    assert!(invocation.prompt.contains("50k"));
    assert!(!invocation.prompt.contains("小于 20k"));
}

#[test]
fn single_item_prompt_requires_verification_checks_as_exact_execution_view() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_backend",
        WorkItemGenerationMode::Serial,
        &[],
        None,
        &RoutingReferenceContext::Legacy,
    )
    .expect("draft invocation");

    // 逐字段复制硬规则与 field_contract 重复，已删除；执行视图精确一致性由 [self_check] 保留。
    assert!(
        invocation
            .prompt
            .contains("verification_plan 与 canonical checks 的逐字段同序相等"),
        "draft prompt must require an exact execution view: {}",
        invocation.prompt
    );
    assert!(
        invocation
            .prompt
            .contains("non_zero_test_execution_required"),
        "draft prompt must include the full typed verification check: {}",
        invocation.prompt
    );
    assert!(
        !invocation.prompt.contains("required_gates"),
        "draft prompt must not retain the legacy gate projection: {}",
        invocation.prompt
    );
}

#[test]
fn single_item_parser_rejects_multiple_work_items() {
    let error = parse_work_item_draft_output(serde_json::json!({
        "drafts": [
            valid_work_item_draft_candidate_json("outline_backend"),
            valid_work_item_draft_candidate_json("outline_frontend")
        ]
    }))
    .expect_err("multiple drafts must be rejected");

    assert_eq!(error.code, "work_item_draft_multiple_items");
}

#[test]
fn single_item_parser_rejects_backend_status_fields() {
    let mut output = serde_json::json!({
        "draft": valid_work_item_draft_candidate_json("outline_backend")
    });
    output["draft"]["status"] = serde_json::json!("accepted");

    let error = parse_work_item_draft_output(output).expect_err("status must be rejected");
    assert_eq!(error.code, "work_item_draft_forbidden_field");
}
