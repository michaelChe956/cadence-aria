use crate::product::json_store::ProductStoreError;

use super::group_review_types::PromptBudgetBreakdown;

#[derive(Debug, thiserror::Error)]
pub(crate) enum GroupReviewExecutionError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("provider_protocol: {0}")]
    ProviderProtocol(String),
    #[error("user_cancelled")]
    UserCancelled,
    #[error("internal: {0}")]
    Internal(String),
}

pub(crate) const GROUP_REVIEW_FAILURE_REASON_CODES: [&str; 7] = [
    "capacity_exceeded",
    "material_overflow",
    "identity_missing",
    "shard_transport_exhausted",
    "reduction_transport_exhausted",
    "shard_output_invalid",
    "reduction_output_invalid",
];

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GroupReviewFailureGateDecision {
    ReplaceExisting,
    KeepExistingAppendEvidence,
    MergeEvidence,
}

#[cfg(test)]
pub(crate) fn decide_group_review_failure_gate(
    existing_reason_code: &str,
    incoming_reason_code: &str,
) -> GroupReviewFailureGateDecision {
    let existing_priority = group_review_failure_priority(existing_reason_code);
    let incoming_priority = group_review_failure_priority(incoming_reason_code);
    if incoming_priority > existing_priority {
        GroupReviewFailureGateDecision::ReplaceExisting
    } else if incoming_reason_code == existing_reason_code {
        GroupReviewFailureGateDecision::MergeEvidence
    } else {
        GroupReviewFailureGateDecision::KeepExistingAppendEvidence
    }
}

#[cfg(test)]
pub(crate) fn group_review_failure_priority(reason_code: &str) -> u8 {
    match reason_code {
        "capacity_exceeded" => 7,
        "material_overflow" => 6,
        "identity_missing" => 5,
        "reduction_output_invalid" => 4,
        "reduction_transport_exhausted" => 3,
        "shard_output_invalid" => 2,
        "shard_transport_exhausted" => 1,
        _ => 0,
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum GroupReviewOrchestrationError {
    #[error("capacity_exceeded")]
    CapacityExceeded,
    #[error("material_overflow")]
    MaterialOverflow { breakdown: PromptBudgetBreakdown },
    #[error("shard_output_invalid: {shard_id}")]
    ShardOutputInvalid { shard_id: String, raw_ref: String },
    #[error("shard_transport_exhausted: {shard_id}")]
    ShardTransportExhausted { shard_id: String },
    #[error("shard_in_progress: {shard_id}")]
    ShardInProgress { shard_id: String },
    #[error("reduction_in_progress")]
    ReductionInProgress,
    #[error("reduction_not_ready")]
    ReductionNotReady,
    #[error("reduction_transport_exhausted")]
    ReductionTransportExhausted,
    #[error("reduction_output_invalid: {raw_ref}")]
    ReductionOutputInvalid { raw_ref: String },
    #[error("shard_stale_audit")]
    ShardStaleAudit,
    #[error("reduction_stale")]
    ReductionStale,
    #[error("identity_missing")]
    // Legacy group review execution path; new group attempts use human final confirmation
    // (Task 3). Retained for Task 6 legacy compatibility reader; remove or keep in Task 6.
    #[allow(dead_code)]
    IdentityMissing,
    #[error("store: {0}")]
    Store(#[from] ProductStoreError),
    #[error("executor: {0}")]
    Executor(#[from] GroupReviewExecutionError),
}
