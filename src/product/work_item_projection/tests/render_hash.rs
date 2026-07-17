use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use super::{
    coder_execution_envelope_fixture, compiled_fixture, reviewer_execution_envelope_fixture,
};
use crate::product::models::ProviderName;
use crate::product::work_item_projection::render::{
    ProjectionRenderRole, ProjectionSection, ProjectionSectionId, validate_mandatory_sections,
};
use crate::product::work_item_projection::{
    CoderExecutionEnvelope, ProjectionRenderError, RenderedExecutionContext,
    ReviewerExecutionEnvelope, renderer_for,
};

#[test]
fn provider_projection_renderer_hash_is_exact_stable_and_sensitive_to_every_content_layer() {
    let compiled = compiled_fixture();
    let envelope = coder_execution_envelope_fixture();
    let first = renderer_for(&ProviderName::Codex)
        .render_coder(&compiled.coder, &envelope)
        .unwrap();
    let second = renderer_for(&ProviderName::Codex)
        .render_coder(&compiled.coder, &envelope)
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(
        first.content_hash,
        format!("{:x}", Sha256::digest(first.text.as_bytes()))
    );
    assert_eq!(first.content_hash.len(), 64);
    assert!(
        first
            .content_hash
            .chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
    );

    let mut changed_projection = compiled.coder.clone();
    changed_projection
        .objective
        .push_str(" with a changed objective");
    let projection_hash = renderer_for(&ProviderName::Codex)
        .render_coder(&changed_projection, &envelope)
        .unwrap()
        .content_hash;
    assert_ne!(first.content_hash, projection_hash);

    let mut changed_envelope = envelope.clone();
    changed_envelope.unit_run_id = "unit_run_changed".to_string();
    let envelope_hash = renderer_for(&ProviderName::Codex)
        .render_coder(&compiled.coder, &changed_envelope)
        .unwrap()
        .content_hash;
    assert_ne!(first.content_hash, envelope_hash);

    let provider_hashes = [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ]
    .into_iter()
    .map(|provider| {
        renderer_for(&provider)
            .render_coder(&compiled.coder, &envelope)
            .unwrap()
            .content_hash
    })
    .collect::<BTreeSet<_>>();
    assert_eq!(provider_hashes.len(), 3);
}

#[test]
fn provider_projection_renderer_versions_and_wrappers_are_provider_specific_and_stable() {
    let compiled = compiled_fixture();
    let expected = [
        (
            ProviderName::Codex,
            "codex-provider-projection-renderer-v1",
            "Codex",
        ),
        (
            ProviderName::ClaudeCode,
            "claude-code-provider-projection-renderer-v1",
            "Claude Code",
        ),
        (
            ProviderName::Fake,
            "fake-provider-projection-renderer-v1",
            "Fake",
        ),
    ];

    for (provider, expected_version, provider_label) in expected {
        let coder = renderer_for(&provider)
            .render_coder(&compiled.coder, &coder_execution_envelope_fixture())
            .unwrap();
        let reviewer = renderer_for(&provider)
            .render_reviewer(&compiled.reviewer, &reviewer_execution_envelope_fixture())
            .unwrap();

        assert_eq!(coder.renderer_version, expected_version);
        assert_eq!(reviewer.renderer_version, expected_version);
        assert!(coder.text.contains(provider_label));
        assert!(reviewer.text.contains(provider_label));
        assert!(coder.text.contains("Structured Output"));
        assert!(reviewer.text.contains("Structured Output"));
    }
}

#[test]
fn provider_projection_renderer_mandatory_section_failure_uses_typed_section_ids() {
    let sections = ProjectionSectionId::mandatory_for(ProjectionRenderRole::Coder)
        .iter()
        .copied()
        .filter(|section_id| *section_id != ProjectionSectionId::WritePolicy)
        .map(|section_id| ProjectionSection::new(section_id, "typed title", "typed body"))
        .collect::<Vec<_>>();

    let error = validate_mandatory_sections(ProjectionRenderRole::Coder, &sections).unwrap_err();

    assert_eq!(
        error,
        ProjectionRenderError::MandatorySectionMissing("Write Policy".to_string())
    );
}

#[test]
fn provider_projection_renderer_never_reads_human_projection_content() {
    let mut compiled = compiled_fixture();
    compiled.human.title = "HUMAN_ONLY_SENTINEL_TITLE".to_string();
    compiled.human.goal = "HUMAN_ONLY_SENTINEL_GOAL".to_string();
    compiled
        .human
        .completion_summary
        .push("HUMAN_ONLY_SENTINEL_COMPLETION".to_string());

    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let coder = renderer_for(&provider)
            .render_coder(&compiled.coder, &coder_execution_envelope_fixture())
            .unwrap();
        let reviewer = renderer_for(&provider)
            .render_reviewer(&compiled.reviewer, &reviewer_execution_envelope_fixture())
            .unwrap();
        for sentinel in [
            "HUMAN_ONLY_SENTINEL_TITLE",
            "HUMAN_ONLY_SENTINEL_GOAL",
            "HUMAN_ONLY_SENTINEL_COMPLETION",
        ] {
            assert!(!coder.text.contains(sentinel), "{provider:?}");
            assert!(!reviewer.text.contains(sentinel), "{provider:?}");
        }
    }
}

#[test]
fn provider_projection_renderer_execution_context_models_roundtrip_through_serde() {
    let coder = coder_execution_envelope_fixture();
    let reviewer = reviewer_execution_envelope_fixture();
    let rendered = renderer_for(&ProviderName::Fake)
        .render_coder(&compiled_fixture().coder, &coder)
        .unwrap();

    let coder_roundtrip: CoderExecutionEnvelope =
        serde_json::from_value(serde_json::to_value(&coder).unwrap()).unwrap();
    let reviewer_roundtrip: ReviewerExecutionEnvelope =
        serde_json::from_value(serde_json::to_value(&reviewer).unwrap()).unwrap();
    let rendered_roundtrip: RenderedExecutionContext =
        serde_json::from_value(serde_json::to_value(&rendered).unwrap()).unwrap();

    assert_eq!(coder_roundtrip, coder);
    assert_eq!(reviewer_roundtrip, reviewer);
    assert_eq!(rendered_roundtrip, rendered);
}
