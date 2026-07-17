use super::{
    coder_execution_envelope_fixture, compiled_fixture, large_contract_fixture,
    ordered_section_bodies,
};
use crate::product::models::ProviderName;
use crate::product::work_item_projection::{WorkItemProjectionCompiler, renderer_for};

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
        let sections = ordered_section_bodies(&rendered.text);
        let titles = sections
            .iter()
            .map(|(title, _)| title.as_str())
            .collect::<Vec<_>>();

        assert_eq!(titles, CODER_SECTION_TITLES, "{provider:?}");
        assert_eq!(
            sections[0].1,
            "{\n  \"work_item_revision_id\": \"work_item_revision_0001\"\n}"
        );
        assert_eq!(
            sections[1].1,
            "{\n  \"objective\": \"Provide the canonical work item contract\"\n}"
        );
        assert_eq!(
            sections[10].1,
            "{\n  \"previous_actionable_review\": \"Address finding review_finding_0001\"\n}"
        );

        for expected_ref in [
            "work_item_revision_0001",
            "contract.source",
            "wi_upstream",
            "task_1",
            "REQ-CANONICAL-001",
            "AC-001",
            "check_canonical",
            "contract_invalid",
            "contract.canonical",
            "repository_state_0001",
            "handoff_revision_0001",
            "handoff_revision_0002",
            "unit_run_0001",
            "1111111111111111111111111111111111111111",
        ] {
            assert!(
                rendered.text.contains(expected_ref),
                "{provider:?} lost {expected_ref}"
            );
        }

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
        for title in CODER_SECTION_TITLES {
            assert!(rendered.text.contains(title), "{provider:?} lost {title}");
        }
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

    let rendered = renderer_for(&ProviderName::Fake)
        .render_coder(&projection, &coder_execution_envelope_fixture())
        .unwrap();
    let sections = ordered_section_bodies(&rendered.text);

    assert_eq!(
        sections
            .iter()
            .map(|(title, _)| title.as_str())
            .collect::<Vec<_>>(),
        CODER_SECTION_TITLES
    );
    for title in [
        "Resolved Inputs",
        "Implementation Tasks",
        "Acceptance Criteria",
        "Verification Checks",
        "Blocker Routing",
    ] {
        let body = &sections
            .iter()
            .find(|(actual, _)| actual == title)
            .unwrap()
            .1;
        assert!(body.contains("[]"), "{title} should render an explicit []");
    }
}

#[test]
fn provider_projection_renderer_coder_empty_execution_envelope_fields_remain_explicit() {
    let mut envelope = coder_execution_envelope_fixture();
    envelope.resolved_handoff_revision_ids.clear();
    envelope.previous_actionable_review = None;
    envelope.start_commit = None;

    let rendered = renderer_for(&ProviderName::Fake)
        .render_coder(&compiled_fixture().coder, &envelope)
        .unwrap();
    let sections = ordered_section_bodies(&rendered.text);
    let execution_envelope = &sections
        .iter()
        .find(|(title, _)| title == "Execution Envelope")
        .unwrap()
        .1;
    let previous_review = &sections
        .iter()
        .find(|(title, _)| title == "Previous Review")
        .unwrap()
        .1;

    assert!(execution_envelope.contains("\"repository_state_ref\""));
    assert!(execution_envelope.contains("\"resolved_handoff_revision_ids\": []"));
    assert!(execution_envelope.contains("\"unit_run_id\""));
    assert!(execution_envelope.contains("\"start_commit\": null"));
    assert!(previous_review.contains("\"previous_actionable_review\": null"));
}
