use super::{
    compiled_fixture, exact_json_sections, large_contract_fixture, ordered_section_bodies,
    reviewer_execution_envelope_fixture,
};
use crate::product::models::ProviderName;
use crate::product::work_item_projection::{
    RenderedExecutionContext, ReviewerExecutionEnvelope, ReviewerWorkItemProjection,
    WorkItemProjectionCompiler, renderer_for,
};

/// 复用本文件既有 reviewer 渲染夹具渲染任意 provider 的 reviewer projection，
/// 供契约文案断言使用。
fn render_reviewer_fixture() -> RenderedExecutionContext {
    let compiled = compiled_fixture();
    let envelope = reviewer_execution_envelope_fixture();
    renderer_for(&ProviderName::Codex)
        .render_reviewer(&compiled.reviewer, &envelope)
        .expect("reviewer fixture must render")
}

const REVIEWER_SECTION_TITLES: &[&str] = &[
    "Work Item Identity/Revision",
    "Acceptance Criteria / Requirement Matrix",
    "Scope Policy",
    "Input Contract Checks",
    "Output Contract Checks",
    "Verification Evidence Rules",
    "Blocker Routing",
    "Review Execution Evidence",
];

const REVIEWER_PROCESS_EVIDENCE_BOUNDARY_SECTION_TITLE: &str = "Reviewer Process Evidence Boundary";

fn expected_reviewer_sections(
    projection: &ReviewerWorkItemProjection,
    envelope: &ReviewerExecutionEnvelope,
) -> Vec<(&'static str, serde_json::Value)> {
    vec![
        (
            "Work Item Identity/Revision",
            serde_json::json!({
                "work_item_revision_id": &projection.work_item_revision_id,
            }),
        ),
        (
            "Acceptance Criteria / Requirement Matrix",
            serde_json::json!({
                "criterion_refs": &projection.criterion_refs,
                "requirement_matrix": &projection.requirement_matrix,
            }),
        ),
        (
            "Scope Policy",
            serde_json::json!({
                "scope_policy": &projection.scope_policy,
            }),
        ),
        (
            "Input Contract Checks",
            serde_json::json!({
                "input_contract_checks": &projection.input_contract_checks,
            }),
        ),
        (
            "Output Contract Checks",
            serde_json::json!({
                "output_contract_checks": &projection.output_contract_checks,
            }),
        ),
        (
            "Verification Evidence Rules",
            serde_json::json!({
                "verification_evidence_rules": &projection.verification_evidence_rules,
            }),
        ),
        (
            "Blocker Routing",
            serde_json::json!({
                "blocker_routing": &projection.blocker_routing,
            }),
        ),
        (
            "Review Execution Evidence",
            serde_json::to_value(envelope).unwrap(),
        ),
    ]
}

fn assert_reviewer_section_oracle(
    text: &str,
    projection: &ReviewerWorkItemProjection,
    envelope: &ReviewerExecutionEnvelope,
) -> Vec<(String, String)> {
    let sections = ordered_section_bodies(text);
    let expected_titles = REVIEWER_SECTION_TITLES
        .iter()
        .copied()
        .chain(std::iter::once(
            REVIEWER_PROCESS_EVIDENCE_BOUNDARY_SECTION_TITLE,
        ))
        .collect::<Vec<_>>();
    assert_eq!(
        sections.iter().map(|(title, _)| title).collect::<Vec<_>>(),
        expected_titles,
        "reviewer mandatory section order/title drift"
    );

    let process_evidence_boundary = sections
        .last()
        .expect("process evidence boundary section")
        .1
        .as_str();
    assert!(
        process_evidence_boundary.contains("不得创建以过程事实为目的的 finding"),
        "reviewer projection must include the process-evidence boundary"
    );

    let json_sections = sections
        .into_iter()
        .filter(|(title, _)| title != REVIEWER_PROCESS_EVIDENCE_BOUNDARY_SECTION_TITLE)
        .map(|(title, body)| format!("## {title}\n\n{body}\n"))
        .collect::<String>();
    let actual = exact_json_sections(&json_sections, REVIEWER_SECTION_TITLES);
    let expected = expected_reviewer_sections(projection, envelope);
    assert_eq!(actual.len(), expected.len());

    for ((actual_title, _, actual_value), (expected_title, expected_value)) in
        actual.iter().zip(&expected)
    {
        assert_eq!(actual_title, expected_title);
        assert_eq!(actual_value, expected_value, "section {actual_title}");
    }

    actual
        .into_iter()
        .map(|(title, body, _)| (title, body))
        .collect()
}

#[test]
fn kimi_code_reviewer_renderer_uses_minimal_profile() {
    let compiled = compiled_fixture();
    let rendered = renderer_for(&ProviderName::KimiCode)
        .render_reviewer(&compiled.reviewer, &reviewer_execution_envelope_fixture())
        .expect("Kimi Code reviewer renderer must render");

    assert_eq!(
        rendered.renderer_version,
        "kimi-code-provider-projection-renderer-v1"
    );
    assert!(rendered.text.contains("Kimi Code"));
}

#[test]
fn provider_projection_renderer_reviewer_declares_verdict_json_output_contract() {
    let compiled = compiled_fixture();
    let envelope = reviewer_execution_envelope_fixture();

    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let rendered = renderer_for(&provider)
            .render_reviewer(&compiled.reviewer, &envelope)
            .unwrap();
        assert!(
            rendered
                .text
                .contains("\"verdict\":\"approve|request_changes|blocked\""),
            "{provider:?} reviewer projection must declare the verdict JSON schema"
        );
        assert!(
            rendered
                .text
                .contains("最终审查结论必须只输出一个 JSON 对象"),
            "{provider:?}"
        );
        assert!(rendered.text.contains("除最终结论 JSON 外"), "{provider:?}");
        assert!(rendered.text.contains("不得出现 { 或 }"), "{provider:?}");
        assert!(
            rendered
                .text
                .contains("不要输出 Markdown 代码块或自然语言总结"),
            "{provider:?}"
        );
    }
}

#[test]
fn provider_projection_renderer_reviewer_golden_sections_are_semantically_equal_across_providers() {
    let compiled = compiled_fixture();
    let envelope = reviewer_execution_envelope_fixture();
    let mut baseline = None;

    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let rendered = renderer_for(&provider)
            .render_reviewer(&compiled.reviewer, &envelope)
            .unwrap();
        let sections =
            assert_reviewer_section_oracle(&rendered.text, &compiled.reviewer, &envelope);

        if let Some(expected) = &baseline {
            assert_eq!(
                &sections, expected,
                "{provider:?} changed normative sections"
            );
        } else {
            baseline = Some(sections);
        }
    }
}

#[test]
fn provider_projection_renderer_reviewer_large_fixture_never_truncates_ids_or_sections() {
    let contract = large_contract_fixture();
    let compiled = WorkItemProjectionCompiler
        .compile(&contract, "work_item_revision_large")
        .unwrap();

    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let rendered = renderer_for(&provider)
            .render_reviewer(&compiled.reviewer, &reviewer_execution_envelope_fixture())
            .unwrap();
        assert_reviewer_section_oracle(
            &rendered.text,
            &compiled.reviewer,
            &reviewer_execution_envelope_fixture(),
        );
        for criterion in &contract.acceptance_criteria {
            assert!(
                rendered.text.contains(&criterion.criterion_id),
                "{provider:?}"
            );
        }
        for check in &contract.verification_checks {
            assert!(rendered.text.contains(&check.check_id), "{provider:?}");
        }
        for blocker in &contract.blocker_rules {
            assert!(rendered.text.contains(&blocker.reason_code), "{provider:?}");
        }
        for input in &contract.input_contracts {
            assert!(rendered.text.contains(&input.contract_id), "{provider:?}");
        }
        for output in &contract.output_contracts {
            assert!(rendered.text.contains(&output.contract_id), "{provider:?}");
        }
    }
}

#[test]
fn provider_projection_renderer_reviewer_empty_lists_keep_explicit_sections() {
    let mut projection = compiled_fixture().reviewer;
    projection.criterion_refs.clear();
    projection.requirement_matrix.clear();
    projection.scope_policy.exclusive_scopes.clear();
    projection.scope_policy.forbidden_scopes.clear();
    projection.input_contract_checks.clear();
    projection.output_contract_checks.clear();
    projection.verification_evidence_rules.clear();
    projection.blocker_routing.clear();

    let envelope = reviewer_execution_envelope_fixture();
    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let rendered = renderer_for(&provider)
            .render_reviewer(&projection, &envelope)
            .unwrap();
        assert_reviewer_section_oracle(&rendered.text, &projection, &envelope);
    }
}

#[test]
fn provider_projection_renderer_reviewer_empty_execution_envelope_lists_remain_explicit() {
    let mut envelope = reviewer_execution_envelope_fixture();
    envelope.handoff_revision_ids.clear();
    envelope.contract_delta_refs.clear();

    let projection = compiled_fixture().reviewer;
    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let rendered = renderer_for(&provider)
            .render_reviewer(&projection, &envelope)
            .unwrap();
        assert_reviewer_section_oracle(&rendered.text, &projection, &envelope);
    }
}

#[test]
fn provider_projection_renderer_reviewer_process_evidence_boundary_is_mandatory() {
    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let compiled = compiled_fixture();
        let envelope = reviewer_execution_envelope_fixture();
        let rendered = renderer_for(&provider)
            .render_reviewer(&compiled.reviewer, &envelope)
            .expect("reviewer projection must render");
        let sections = ordered_section_bodies(&rendered.text);
        assert!(
            sections
                .iter()
                .any(|(title, _)| title == REVIEWER_PROCESS_EVIDENCE_BOUNDARY_SECTION_TITLE),
            "{provider:?} reviewer projection must render the process-evidence boundary section"
        );
        assert!(
            rendered.text.contains("non_zero_test_execution 表示验证命令执行时实际运行了非零数量的测试，是当前可观测的执行结果；它不表达测试曾先失败、不表达提交顺序、不表达任何开发时序。"),
            "{provider:?} reviewer projection must state non_zero_test_execution semantics"
        );
    }
}

#[test]
fn reviewer_contract_bounds_implementation_defect_fields() {
    let rendered = render_reviewer_fixture();
    let text = &rendered.text;
    // 三条断言各自独立锚定 render.rs Reviewer 契约中两条新增文案的核心子串，
    // 使任一断言失败都能捕获“新文案被删除”这一回归。
    //
    // 断言 1：锚定第一条新文案——implementation_defect 字段边界
    // （禁止填写计划类字段）。子串 `的 finding 禁止填写` 在全仓仅来自
    // render.rs 的该新增文案，旧契约行（findings 必须包含…）不含“禁止填写”。
    assert!(
        text.contains("defect_class=implementation_defect 的 finding 禁止填写"),
        "contract must forbid plan defect fields on implementation_defect findings"
    );
    // 断言 2：锚定第二条新文案——implementation_defect 证据写入
    // message 与 required_action 的自然语言描述。子串在全仓仅来自该新增文案。
    assert!(
        text.contains("证据写入 message 与 required_action 的自然语言描述"),
        "contract must name message/required_action as the evidence outlet for implementation_defect"
    );
    // 断言 3：锚定第二条新文案同区域——只有计划类缺陷才允许携带
    // plan_defect_evidence 与路由字段。枚举子串在全仓仅来自该新增文案。
    assert!(
        text.contains(
            "current_work_item_invalid、upstream_contract_invalid、dependency_graph_invalid"
        ),
        "contract must enumerate plan defect classes that may carry plan_defect_evidence"
    );
}
