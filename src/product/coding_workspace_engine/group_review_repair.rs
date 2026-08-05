use super::group_review_orchestrator::GroupReviewExecutionError;
use super::review_parser::parse_group_review_payload;
use crate::product::coding_models::{CodingExecutionStage, ReviewVerdict};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepairOutput {
    pub(crate) repaired_output: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum RepairFidelityError {
    #[error("repair_missing_marker")]
    MissingMarker,
    #[error("repair_verdict_mismatch")]
    VerdictMismatch,
    #[error("repair_forbidden_approve")]
    ForbiddenApprove,
    #[error("repair_invalid_payload")]
    InvalidPayload,
    #[error("repair_finding_not_subtraceable")]
    FindingNotSubtraceable,
    #[error("repair_evidence_not_subtraceable")]
    EvidenceNotSubtraceable,
    #[error("repair_target_not_subtraceable")]
    TargetNotSubtraceable,
    #[error("repair_too_many_findings")]
    TooManyFindings,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RepairError {
    #[error("repair_input_too_large")]
    InputTooLarge,
    #[error("repair_executor: {0}")]
    Executor(#[from] GroupReviewExecutionError),
    #[error("repair_fidelity: {0}")]
    Fidelity(#[from] RepairFidelityError),
}

pub(crate) fn validate_repair_fidelity(
    raw_output: &str,
    repaired_output: &str,
    max_findings: usize,
) -> Result<(), RepairFidelityError> {
    let raw_verdict = verdict_marker(raw_output).ok_or(RepairFidelityError::MissingMarker)?;
    let repaired_verdict =
        verdict_marker(repaired_output).ok_or(RepairFidelityError::MissingMarker)?;
    if repaired_verdict == ReviewVerdict::Approve {
        return Err(RepairFidelityError::ForbiddenApprove);
    }
    if repaired_verdict != raw_verdict {
        return Err(RepairFidelityError::VerdictMismatch);
    }
    let payload =
        parse_group_review_payload(repaired_output, CodingExecutionStage::InternalPrReview)
            .map_err(|_| RepairFidelityError::InvalidPayload)?;
    if payload.findings.len() > max_findings {
        return Err(RepairFidelityError::TooManyFindings);
    }
    if payload.findings.iter().any(|finding| {
        !raw_output.contains(&finding.message)
            || finding
                .evidence
                .iter()
                .any(|evidence| !raw_output.contains(evidence))
            || finding.plan_defect_evidence.iter().any(|evidence| {
                !raw_output.contains(&evidence.source_ref)
                    || !raw_output.contains(&evidence.message)
                    || !raw_output.contains(&evidence.kind)
            })
    }) {
        if payload
            .findings
            .iter()
            .any(|finding| !raw_output.contains(&finding.message))
        {
            return Err(RepairFidelityError::FindingNotSubtraceable);
        }
        return Err(RepairFidelityError::EvidenceNotSubtraceable);
    }
    if payload.findings.iter().any(|finding| {
        finding.repair_target.as_ref().is_some_and(|target| {
            let kind = match target.kind {
                crate::product::models::RepairTargetKind::CurrentWorkItem => "current_work_item",
                crate::product::models::RepairTargetKind::UpstreamWorkItem => "upstream_work_item",
                crate::product::models::RepairTargetKind::Subgraph => "subgraph",
            };
            !raw_output.contains(kind)
                || target
                    .logical_work_item_ids
                    .iter()
                    .chain(target.work_item_revision_ids.iter())
                    .any(|id| !raw_output.contains(id))
        })
    }) {
        return Err(RepairFidelityError::TargetNotSubtraceable);
    }
    Ok(())
}

fn verdict_marker(output: &str) -> Option<ReviewVerdict> {
    output.lines().find_map(|line| {
        let value = line.trim().strip_prefix("GROUP_REVIEW_VERDICT:")?.trim();
        match value {
            "approve" => Some(ReviewVerdict::Approve),
            "request_changes" => Some(ReviewVerdict::RequestChanges),
            "blocked" => Some(ReviewVerdict::Blocked),
            _ => None,
        }
    })
}
