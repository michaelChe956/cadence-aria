mod canonical;
mod profile;
mod projection;
mod refs;
mod rules;
mod types;

pub use canonical::canonical_validator;
pub use profile::phase1_profile_validator;
pub use projection::projection_validator;
pub use refs::{record_superseded_artifact_ref, validate_input_artifact_ref};
pub use rules::{artifact_validation_matrix, artifact_validation_rule};
pub use types::{
    ArtifactContent, ArtifactContentFamily, ArtifactIndex, ArtifactValidateError,
    ArtifactValidationRule, ConstraintBundleIndex, ProjectionIndex, ProviderRunIndex,
    TraceabilityIndex, ValidationResult, WorkPackageId,
};
