use sha2::{Digest, Sha256};

use super::{PlanCandidateIr, PlanCandidateMechanicalReport, WORK_ITEM_PLAN_COMPILER_VERSION};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreshnessError {
    SourceRevisionMismatch,
    CompilerVersionMismatch,
    MechanicalValidationFailed,
}

pub fn verify_publish_freshness(
    current_source: &str,
    ir: &PlanCandidateIr,
    mechanical_report: &PlanCandidateMechanicalReport,
) -> Result<(), FreshnessError> {
    let current_source_hash = hex::encode(Sha256::digest(current_source.as_bytes()));
    if ir.source_revision_hash != current_source_hash {
        return Err(FreshnessError::SourceRevisionMismatch);
    }
    if ir.compiler_version != WORK_ITEM_PLAN_COMPILER_VERSION {
        return Err(FreshnessError::CompilerVersionMismatch);
    }
    if mechanical_report.source_revision_hash != current_source_hash
        || mechanical_report.source_revision_hash != ir.source_revision_hash
        || mechanical_report.compiler_version != ir.compiler_version
        || mechanical_report.has_errors()
    {
        return Err(FreshnessError::MechanicalValidationFailed);
    }
    Ok(())
}
