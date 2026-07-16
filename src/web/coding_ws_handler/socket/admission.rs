use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::coding_workspace_engine::recoverable_failed_code_review;
use crate::web::state::{CodingAttemptRunKey, CodingRunRegistry};

use super::{CodingWsInMessage, is_coding_ws_message_allowed};

pub(crate) fn failed_code_review_recovery_request(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    message: &CodingWsInMessage,
) -> bool {
    let CodingWsInMessage::GateResponse {
        gate_id, action_id, ..
    } = message
    else {
        return false;
    };
    if action_id != "retry_review" {
        return false;
    }
    matches!(
        recoverable_failed_code_review(coding_store, attempt),
        Ok(Some(recovery)) if recovery.gate_id == *gate_id
    )
}

pub(crate) fn unfinished_failed_code_review_recovery_message_allowed(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    message: &CodingWsInMessage,
) -> Option<bool> {
    let journal = match coding_store.get_failed_code_review_recovery_journal(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    ) {
        Ok(Some(journal)) if !journal.is_completed() => journal,
        Ok(_) => return None,
        Err(_) => return Some(false),
    };
    Some(matches!(
        message,
        CodingWsInMessage::GateResponse {
            gate_id,
            action_id,
            ..
        } if gate_id == &journal.expected_gate_id && action_id == "retry_review"
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodingMessageAdmission {
    Allowed,
    FailedReviewRecovery,
    Rejected,
}

pub(crate) fn coding_message_admission(
    coding_store: &CodingAttemptStore,
    coding_runs: &CodingRunRegistry,
    attempt: &CodingExecutionAttempt,
    message: &CodingWsInMessage,
) -> CodingMessageAdmission {
    if matches!(
        message,
        CodingWsInMessage::CodingHello { .. } | CodingWsInMessage::CodingPing
    ) {
        return CodingMessageAdmission::Allowed;
    }
    if coding_runs.has_active_recovery_reservation(&CodingAttemptRunKey::from_attempt(attempt)) {
        return CodingMessageAdmission::Rejected;
    }
    let unfinished_recovery_message_allowed =
        unfinished_failed_code_review_recovery_message_allowed(coding_store, attempt, message);
    let failed_review_recovery =
        failed_code_review_recovery_request(coding_store, attempt, message);
    if matches!(unfinished_recovery_message_allowed, Some(false))
        || (unfinished_recovery_message_allowed == Some(true) && !failed_review_recovery)
        || (!is_coding_ws_message_allowed(&attempt.status, &attempt.stage, message)
            && !failed_review_recovery)
    {
        return CodingMessageAdmission::Rejected;
    }
    if failed_review_recovery {
        CodingMessageAdmission::FailedReviewRecovery
    } else {
        CodingMessageAdmission::Allowed
    }
}
