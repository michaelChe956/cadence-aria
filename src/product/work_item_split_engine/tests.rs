use std::path::PathBuf;

use crate::product::models::{
    IssuePhase, IssueRecord, IssueStatus, RepositoryRecord, WorkItemDraftCandidate,
    WorkItemDraftRecord, WorkItemDraftStatus, WorkItemGenerationMode, WorkItemKind,
};
use crate::product::work_item_split_engine::prompts::{
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
    };

    let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)");

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
        prompt.contains("\"from_outline_id\"") && prompt.contains("\"to_outline_id\""),
        "outline prompt schema must name dependency edge fields: {prompt}"
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
    );
    let (revision_prompt, _) = build_outline_revision_prompt(&request, &issue, "补充前后端依赖边");

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
    );
    let (revision_prompt, _) =
        build_outline_revision_prompt(&request, &issue, "修复 exclusive_write_scopes 重叠");

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
}

#[test]
fn outline_parser_accepts_valid_sentinel_json() {
    let parsed =
        parse_work_item_plan_outline_output(valid_outline_author_output()).expect("outline");

    assert!(parsed.context_blockers.is_empty());
    let outline = parsed.outline.expect("outline payload");
    assert_eq!(outline.work_item_outlines[0].outline_id, "outline_backend");
    assert_eq!(
        outline.dependency_graph[0].from_outline_id,
        "outline_backend"
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
fn single_item_prompt_contains_accepted_previous_context() {
    let outline = parse_work_item_plan_outline_output(valid_outline_author_output())
        .expect("outline output")
        .outline
        .expect("outline");
    let accepted_backend = sample_draft_record(
        "draft_backend",
        "outline_backend",
        WorkItemDraftCandidate {
            outline_id: "outline_backend".to_string(),
            title: "后端 API".to_string(),
            kind: WorkItemKind::Backend,
            goal: "实现 API".to_string(),
            implementation_context: "定义 GET /api/session/status".to_string(),
            exclusive_write_scopes: vec!["src/product/**".to_string()],
            forbidden_write_scopes: vec!["web/**".to_string()],
            depends_on_outline_ids: vec![],
            required_handoff_from_outline_ids: vec![],
            handoff_summary: "后端输出 SessionStatusDto".to_string(),
            verification_plan: serde_json::json!({"commands": []}),
        },
    );

    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_frontend",
        WorkItemGenerationMode::Serial,
        &[accepted_backend],
        Some("补充错误态"),
    )
    .expect("draft invocation");

    assert!(invocation.prompt.contains("outline_frontend"));
    assert!(invocation.prompt.contains("serial"));
    assert!(invocation.prompt.contains("SessionStatusDto"));
    assert!(invocation.prompt.contains("直接依赖 draft 完整内容"));
    assert!(invocation.prompt.contains("补充错误态"));
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
    )
    .expect("draft invocation");

    assert!(invocation.prompt.contains("不得输出 work_item_id"));
    assert!(invocation.prompt.contains("不得修改 Outline"));
    assert!(
        invocation
            .prompt
            .contains("只能输出一个 WorkItemDraftCandidate")
    );
}

#[test]
fn single_item_prompt_requires_required_gates_as_string_id_array() {
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
    )
    .expect("draft invocation");

    assert!(
        invocation
            .prompt
            .contains("required_gates 必须是字符串数组"),
        "draft prompt must explicitly state required_gates uses string ids: {}",
        invocation.prompt
    );
    assert!(
        invocation
            .prompt
            .contains("\"required_gates\":[\"cmd_unit\"]"),
        "draft prompt must include a minimal valid required_gates example: {}",
        invocation.prompt
    );
    assert!(
        invocation
            .prompt
            .contains("不要输出 required_gates gate 对象"),
        "draft prompt must forbid gate object output: {}",
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

#[test]
fn build_split_prompt_inlines_schema_and_kind_guidance() {
    // 回归 Bug: prompt 曾引用不存在的 `src/product/work_item_split_output_schema.json`,
    // 而 WORK_ITEM_SPLIT_OUTPUT_SCHEMA 常量未注入 prompt,导致 provider 不知道
    // `kind` 是必填字段,按习惯输出 `type` 触发 `missing field kind`。
    // 修复后 prompt 必须内联 schema 正文并给出 kind 合法取值。
    let (request, issue, repository) = split_prompt_fixture();
    let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)");

    assert!(
        !prompt.contains("work_item_split_output_schema.json"),
        "prompt must not reference a non-existent schema file path: {prompt}"
    );
    // schema 正文必须内联进 prompt(取 schema 常量里的标志性片段)。
    assert!(
        prompt.contains("\"kind\""),
        "prompt must inline the schema's `kind` property: {prompt}"
    );
    assert!(
        prompt.contains("\"required\""),
        "prompt must inline the schema's `required` clause: {prompt}"
    );
    // kind 合法取值引导(provider 必须知道有哪些枚举值可选)。
    for kind_value in [
        "backend",
        "frontend",
        "integration",
        "e2e",
        "docs",
        "infra",
        "other",
    ] {
        assert!(
            prompt.contains(kind_value),
            "prompt must list kind value `{kind_value}`: {prompt}"
        );
    }
}

#[test]
fn build_split_prompt_allows_readable_stream_before_final_sentinel() {
    let (request, issue, repository) = split_prompt_fixture();
    let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)");

    assert!(prompt.contains("<ARIA_STRUCTURED_OUTPUT nonce=\""));
    assert!(prompt.contains("</ARIA_STRUCTURED_OUTPUT nonce=\""));
    assert!(prompt.contains("可以在最终结构化 JSON 前输出简短、可读的拆分过程"));
    assert!(prompt.contains("最后必须输出一个 nonce sentinel JSON block"));
    assert!(prompt.contains("后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT"));
    assert!(prompt.contains("不要输出 Markdown code fence"));
}

#[test]
fn split_prompt_requests_progress_before_long_operations() {
    let (request, issue, repository) = split_prompt_fixture();
    let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)");

    assert!(prompt.contains("长时间分析、探索代码库或自动修正前"));
    assert!(prompt.contains("先输出一行简短可读状态"));
    assert!(prompt.contains("每完成一组探索后输出一句当前发现摘要"));
}

#[test]
fn build_revision_prompt_inlines_schema_and_kind_guidance() {
    let (request, issue, repository) = split_prompt_fixture();
    let redo_specs = vec![RedoSpec {
        old_id: "work_item_0001".to_string(),
        feedback: "拆得太粗".to_string(),
    }];
    let prompt = build_revision_prompt(
        &request,
        &issue,
        &repository,
        &[],
        &redo_specs,
        &[],
        &[],
        "(empty)",
    );

    assert!(
        !prompt.contains("work_item_split_output_schema.json"),
        "revision prompt must not reference a non-existent schema file path: {prompt}"
    );
    assert!(
        prompt.contains("\"kind\""),
        "revision prompt must inline the schema's `kind` property: {prompt}"
    );
    assert!(
        prompt.contains("\"required\""),
        "revision prompt must inline the schema's `required` clause: {prompt}"
    );
    for kind_value in [
        "backend",
        "frontend",
        "integration",
        "e2e",
        "docs",
        "infra",
        "other",
    ] {
        assert!(
            prompt.contains(kind_value),
            "revision prompt must list kind value `{kind_value}`: {prompt}"
        );
    }
}

#[test]
fn build_revision_prompt_allows_readable_stream_before_final_sentinel() {
    let (request, issue, repository) = split_prompt_fixture();
    let redo_specs = vec![RedoSpec {
        old_id: "work_item_0001".to_string(),
        feedback: "拆得太粗".to_string(),
    }];
    let prompt = build_revision_prompt(
        &request,
        &issue,
        &repository,
        &[],
        &redo_specs,
        &[],
        &[],
        "(empty)",
    );

    assert!(prompt.contains("<ARIA_STRUCTURED_OUTPUT nonce=\""));
    assert!(prompt.contains("</ARIA_STRUCTURED_OUTPUT nonce=\""));
    assert!(prompt.contains("可以在最终结构化 JSON 前输出简短、可读的拆分过程"));
    assert!(prompt.contains("最后必须输出一个 nonce sentinel JSON block"));
    assert!(prompt.contains("后端只解析最后一个 nonce 匹配的 <ARIA_STRUCTURED_OUTPUT"));
    assert!(prompt.contains("不要输出 Markdown code fence"));
}

#[test]
fn revision_prompt_requests_progress_before_long_operations() {
    let (request, issue, repository) = split_prompt_fixture();
    let redo_specs = vec![RedoSpec {
        old_id: "work_item_0001".to_string(),
        feedback: "拆得太粗".to_string(),
    }];
    let prompt = build_revision_prompt(
        &request,
        &issue,
        &repository,
        &[],
        &redo_specs,
        &[],
        &[],
        "(empty)",
    );

    assert!(prompt.contains("长时间分析、探索代码库或自动修正前"));
    assert!(prompt.contains("先输出一行简短可读状态"));
    assert!(prompt.contains("每完成一组探索后输出一句当前发现摘要"));
}

fn valid_outline_author_output() -> serde_json::Value {
    serde_json::json!({
        "outline": {
            "id": "outline_artifact_1",
            "project_id": "project_0001",
            "issue_id": "issue_0001",
            "source_story_spec_ids": ["story_spec_0001"],
            "source_design_spec_ids": ["design_spec_0001"],
            "strategy_summary": "先后端后前端",
            "work_item_outlines": [
                {
                    "outline_id": "outline_backend",
                    "title": "后端 API",
                    "kind": "backend",
                    "goal": "实现 API",
                    "scope": ["src/product"],
                    "non_goals": [],
                    "source_story_spec_ids": ["story_spec_0001"],
                    "source_design_spec_ids": ["design_spec_0001"],
                    "exclusive_write_scopes": ["src/product/**"],
                    "forbidden_write_scopes": ["web/**"],
                    "depends_on": [],
                    "verification_intent": ["cargo test --locked --lib api"],
                    "handoff_notes": "提供 API contract"
                },
                {
                    "outline_id": "outline_frontend",
                    "title": "前端 UI",
                    "kind": "frontend",
                    "goal": "接入 API",
                    "scope": ["web/src"],
                    "non_goals": [],
                    "source_story_spec_ids": ["story_spec_0001"],
                    "source_design_spec_ids": ["design_spec_0001"],
                    "exclusive_write_scopes": ["web/src/**"],
                    "forbidden_write_scopes": ["src/product/**"],
                    "depends_on": ["outline_backend"],
                    "verification_intent": ["pnpm -C web test"],
                    "handoff_notes": "消费 API contract"
                }
            ],
            "dependency_graph": [
                {
                    "from_outline_id": "outline_backend",
                    "to_outline_id": "outline_frontend"
                }
            ],
            "risks": [],
            "handoff_strategy": "后端输出 contract 给前端",
            "status": "draft"
        },
        "context_blockers": []
    })
}

fn valid_work_item_draft_candidate_json(outline_id: &str) -> serde_json::Value {
    serde_json::json!({
        "outline_id": outline_id,
        "title": "后端 API",
        "kind": "backend",
        "goal": "实现 API",
        "implementation_context": "实现 API handler 与 product service。",
        "exclusive_write_scopes": ["src/product/**"],
        "forbidden_write_scopes": ["web/**"],
        "depends_on_outline_ids": [],
        "required_handoff_from_outline_ids": [],
        "handoff_summary": "输出 SessionStatusDto",
        "verification_plan": {
            "commands": [
                {
                    "id": "cmd_backend",
                    "label": "cargo test",
                    "command": "cargo test --locked --lib session",
                    "cwd": "",
                    "purpose": "验证后端 API",
                    "required": true,
                    "timeout_seconds": 120,
                    "safety": "approved",
                    "source": "local"
                }
            ],
            "manual_checks": [],
            "required_gates": ["cmd_backend"]
        }
    })
}

fn sample_draft_record(
    draft_id: &str,
    outline_id: &str,
    candidate: WorkItemDraftCandidate,
) -> WorkItemDraftRecord {
    WorkItemDraftRecord {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: "plan_0001".to_string(),
        draft_id: draft_id.to_string(),
        outline_id: outline_id.to_string(),
        generation_round_id: "round_001".to_string(),
        batch_id: None,
        attempt_index: 1,
        outline_version_ref: "artifact://outline/1".to_string(),
        generation_mode: WorkItemGenerationMode::Serial,
        candidate,
        status: WorkItemDraftStatus::Accepted,
        active: true,
        superseded_by_draft_id: None,
        supersede_reason: None,
        copied_from_draft_id: None,
        review_node_id: None,
        review_verdict_ref: None,
        generated_from_node_id: "node_draft_run".to_string(),
        accepted_at: Some("2026-06-22T10:00:00Z".to_string()),
        superseded_at: None,
        created_at: "2026-06-22T10:00:00Z".to_string(),
        updated_at: "2026-06-22T10:00:00Z".to_string(),
    }
}
