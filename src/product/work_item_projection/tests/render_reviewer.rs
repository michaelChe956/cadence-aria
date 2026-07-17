use super::{
    compiled_fixture, large_contract_fixture, ordered_section_bodies,
    reviewer_execution_envelope_fixture,
};
use crate::product::models::ProviderName;
use crate::product::work_item_projection::{WorkItemProjectionCompiler, renderer_for};

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
        let sections = ordered_section_bodies(&rendered.text);
        assert_eq!(
            sections
                .iter()
                .map(|(title, _)| title.as_str())
                .collect::<Vec<_>>(),
            REVIEWER_SECTION_TITLES,
            "{provider:?}"
        );
        assert_eq!(
            sections[0].1,
            "{\n  \"work_item_revision_id\": \"work_item_revision_0001\"\n}"
        );
        assert_eq!(
            sections[7].1,
            "{\n  \"unit_run_id\": \"unit_run_0001\",\n  \"diff_ref\": \"diff_ref_0001\",\n  \"test_evidence_refs\": [\n    \"test_evidence_0001\",\n    \"test_evidence_0002\"\n  ],\n  \"handoff_revision_ids\": [\n    \"handoff_revision_0001\"\n  ],\n  \"contract_delta_refs\": [\n    \"contract_delta_0001\"\n  ],\n  \"completion_commit\": \"2222222222222222222222222222222222222222\"\n}"
        );

        for expected_ref in [
            "work_item_revision_0001",
            "AC-001",
            "REQ-CANONICAL-001",
            "contract.source",
            "contract.canonical",
            "check_canonical",
            "contract_invalid",
            "unit_run_0001",
            "diff_ref_0001",
            "test_evidence_0001",
            "handoff_revision_0001",
            "contract_delta_0001",
            "2222222222222222222222222222222222222222",
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
        for title in REVIEWER_SECTION_TITLES {
            assert!(rendered.text.contains(title), "{provider:?} lost {title}");
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

    let rendered = renderer_for(&ProviderName::Fake)
        .render_reviewer(&projection, &reviewer_execution_envelope_fixture())
        .unwrap();
    let sections = ordered_section_bodies(&rendered.text);

    assert_eq!(
        sections
            .iter()
            .map(|(title, _)| title.as_str())
            .collect::<Vec<_>>(),
        REVIEWER_SECTION_TITLES
    );
    for title in [
        "Acceptance Criteria / Requirement Matrix",
        "Input Contract Checks",
        "Output Contract Checks",
        "Verification Evidence Rules",
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
fn provider_projection_renderer_reviewer_empty_execution_envelope_lists_remain_explicit() {
    let mut envelope = reviewer_execution_envelope_fixture();
    envelope.test_evidence_refs.clear();
    envelope.handoff_revision_ids.clear();
    envelope.contract_delta_refs.clear();

    let rendered = renderer_for(&ProviderName::Fake)
        .render_reviewer(&compiled_fixture().reviewer, &envelope)
        .unwrap();
    let sections = ordered_section_bodies(&rendered.text);
    let review_evidence = &sections
        .iter()
        .find(|(title, _)| title == "Review Execution Evidence")
        .unwrap()
        .1;

    for field in ["\"unit_run_id\"", "\"diff_ref\"", "\"completion_commit\""] {
        assert!(review_evidence.contains(field), "missing {field}");
    }
    for field in [
        "\"test_evidence_refs\": []",
        "\"handoff_revision_ids\": []",
        "\"contract_delta_refs\": []",
    ] {
        assert!(review_evidence.contains(field), "missing explicit {field}");
    }
}
