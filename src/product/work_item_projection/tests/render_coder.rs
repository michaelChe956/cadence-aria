use super::{
    coder_execution_envelope_fixture, compiled_fixture, exact_json_sections, large_contract_fixture,
};
use crate::product::models::ProviderName;
use crate::product::work_item_projection::{
    CoderExecutionEnvelope, CoderWorkItemProjection, WorkItemProjectionCompiler, renderer_for,
};

const CODER_SECTION_TITLES: &[&str] = &[
    "Work Item Identity/Revision",
    "Objective",
    "Resolved Inputs",
    "Implementation Tasks",
    "Write Policy",
    "Acceptance Criteria",
    "Verification Checks",
    "Blocker Routing",
    "Handoff Requirements",
    "Execution Envelope",
    "Previous Review",
];

fn expected_coder_sections(
    projection: &CoderWorkItemProjection,
    envelope: &CoderExecutionEnvelope,
) -> Vec<(&'static str, serde_json::Value)> {
    vec![
        (
            "Work Item Identity/Revision",
            serde_json::json!({
                "work_item_revision_id": &projection.work_item_revision_id,
            }),
        ),
        (
            "Objective",
            serde_json::json!({
                "objective": &projection.objective,
            }),
        ),
        (
            "Resolved Inputs",
            serde_json::json!({
                "required_input_contracts": &projection.required_input_contracts,
            }),
        ),
        (
            "Implementation Tasks",
            serde_json::json!({
                "task_refs": &projection.task_refs,
                "tasks": &projection.tasks,
            }),
        ),
        (
            "Write Policy",
            serde_json::json!({
                "write_policy": &projection.write_policy,
                "commit_responsibility": "提交责任：先检查完整 Git 状态；仅根据本 Work Item 的 write_policy 精确暂存允许路径；创建本 Work Item 的提交。\n报告：列出暂存文件、提交 SHA、提交后的 Git 状态。\n禁止：不得使用无差别全量暂存；不得删除、清理或提交无法由当前 write_policy 解释的内容。遇到它们时保留并报告。",
            }),
        ),
        (
            "Acceptance Criteria",
            serde_json::json!({
                "acceptance_criteria": &projection.acceptance_criteria,
            }),
        ),
        (
            "Verification Checks",
            serde_json::json!({
                "verification_checks": &projection.verification_checks,
            }),
        ),
        (
            "Blocker Routing",
            serde_json::json!({
                "blocker_rules": &projection.blocker_rules,
            }),
        ),
        (
            "Handoff Requirements",
            serde_json::json!({
                "handoff_contract": &projection.handoff_contract,
            }),
        ),
        (
            "Execution Envelope",
            serde_json::json!({
                "repository_state_ref": &envelope.repository_state_ref,
                "resolved_handoff_revision_ids": &envelope.resolved_handoff_revision_ids,
                "unit_run_id": &envelope.unit_run_id,
                "start_commit": &envelope.start_commit,
            }),
        ),
        (
            "Previous Review",
            serde_json::json!({
                "previous_actionable_review": &envelope.previous_actionable_review,
            }),
        ),
    ]
}

fn assert_coder_section_oracle(
    text: &str,
    projection: &CoderWorkItemProjection,
    envelope: &CoderExecutionEnvelope,
) -> Vec<(String, String)> {
    let actual = exact_json_sections(text, CODER_SECTION_TITLES);
    let expected = expected_coder_sections(projection, envelope);
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
fn kimi_code_coder_renderer_uses_minimal_profile() {
    let compiled = compiled_fixture();
    let rendered = renderer_for(&ProviderName::KimiCode)
        .render_coder(&compiled.coder, &coder_execution_envelope_fixture())
        .expect("Kimi Code coder renderer must render");

    assert_eq!(
        rendered.renderer_version,
        "kimi-code-provider-projection-renderer-v1"
    );
    assert!(rendered.text.contains("Kimi Code"));
}

#[test]
fn provider_projection_renderer_coder_does_not_leak_reviewer_verdict_contract() {
    let compiled = compiled_fixture();
    let envelope = coder_execution_envelope_fixture();

    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let rendered = renderer_for(&provider)
            .render_coder(&compiled.coder, &envelope)
            .unwrap();
        assert!(
            !rendered
                .text
                .contains("最终审查结论必须只输出一个 JSON 对象"),
            "{provider:?} coder projection must not contain the reviewer verdict contract"
        );
        assert!(
            !rendered
                .text
                .contains("\"verdict\":\"approve|request_changes|blocked\""),
            "{provider:?}"
        );
    }
}

#[test]
fn provider_projection_renderer_coder_golden_sections_are_semantically_equal_across_providers() {
    let compiled = compiled_fixture();
    let envelope = coder_execution_envelope_fixture();
    let mut baseline = None;

    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let rendered = renderer_for(&provider)
            .render_coder(&compiled.coder, &envelope)
            .unwrap();
        let sections = assert_coder_section_oracle(&rendered.text, &compiled.coder, &envelope);

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
fn provider_projection_renderer_coder_large_fixture_never_truncates_ids_or_sections() {
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
            .render_coder(&compiled.coder, &coder_execution_envelope_fixture())
            .unwrap();
        assert_coder_section_oracle(
            &rendered.text,
            &compiled.coder,
            &coder_execution_envelope_fixture(),
        );
        for task in &contract.tasks {
            assert!(rendered.text.contains(&task.task_id), "{provider:?}");
        }
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
fn provider_projection_renderer_coder_empty_lists_keep_explicit_sections() {
    let mut projection = compiled_fixture().coder;
    projection.required_input_contracts.clear();
    projection.task_refs.clear();
    projection.tasks.clear();
    projection.write_policy.exclusive_scopes.clear();
    projection.write_policy.forbidden_scopes.clear();
    projection.acceptance_criteria.clear();
    projection.verification_checks.clear();
    projection.blocker_rules.clear();
    projection.handoff_contract.required_fields.clear();
    projection.handoff_contract.provided_contract_refs.clear();
    projection.handoff_contract.reviewer_check_refs.clear();

    let envelope = coder_execution_envelope_fixture();
    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let rendered = renderer_for(&provider)
            .render_coder(&projection, &envelope)
            .unwrap();
        assert_coder_section_oracle(&rendered.text, &projection, &envelope);
    }
}

#[test]
fn provider_projection_renderer_coder_empty_execution_envelope_fields_remain_explicit() {
    let mut envelope = coder_execution_envelope_fixture();
    envelope.resolved_handoff_revision_ids.clear();
    envelope.previous_actionable_review = None;
    envelope.start_commit = None;

    let projection = compiled_fixture().coder;
    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let rendered = renderer_for(&provider)
            .render_coder(&projection, &envelope)
            .unwrap();
        assert_coder_section_oracle(&rendered.text, &projection, &envelope);
    }
}
