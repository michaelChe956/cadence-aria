use super::types::{ArtifactContentFamily, ArtifactValidationRule};
use crate::protocol::artifacts::ArtifactKind;

pub fn artifact_validation_matrix() -> Vec<ArtifactValidationRule> {
    ArtifactKind::all_phase1()
        .map(|artifact_kind| artifact_validation_rule(artifact_kind).expect("known artifact kind"))
        .collect()
}

pub fn artifact_validation_rule(artifact_kind: ArtifactKind) -> Option<ArtifactValidationRule> {
    let requires_projection = matches!(
        artifact_kind,
        ArtifactKind::Spec | ArtifactKind::Design | ArtifactKind::Plan
    );
    let requires_phase1_profile = matches!(
        artifact_kind,
        ArtifactKind::DispatchPackage
            | ArtifactKind::CodingReport
            | ArtifactKind::TestingReport
            | ArtifactKind::CodeReviewReport
            | ArtifactKind::IntegrationReport
            | ArtifactKind::FinalReview
            | ArtifactKind::FinalSummary
    );
    Some(ArtifactValidationRule {
        artifact_kind,
        requires_canonical: true,
        requires_projection,
        requires_phase1_profile,
        content_family: if artifact_kind.is_markdown_canonical() {
            ArtifactContentFamily::Markdown
        } else {
            ArtifactContentFamily::Json
        },
    })
}
