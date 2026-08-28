use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use super::*;
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::models::{
    SingleCandidateCompileReservation, SingleCandidatePhase, WorkspaceSessionRecord, WorkspaceType,
};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::product::work_item_plan_source_store::{
    SourceStoreScope, validate_canonical_ref_for_scope,
};

pub(super) fn max_workspace_session_sequence(
    projects_root: &std::path::Path,
) -> Result<usize, ProductStoreError> {
    let mut max_sequence = 0usize;
    for project_path in super::child_directories(projects_root)? {
        let issues_root = project_path.join("issues");
        for issue_path in super::child_directories(&issues_root)? {
            let workspace_sessions_root = issue_path.join("workspace-sessions");
            for session_path in super::workspace_session_file_paths(&workspace_sessions_root)? {
                let Some(session) = super::read_workspace_session_record(&session_path)? else {
                    continue;
                };
                if let Some(sequence) = parse_sequential_id(&session.id, "workspace_session") {
                    max_sequence = max_sequence.max(sequence);
                }
            }
        }
    }
    Ok(max_sequence)
}

fn parse_sequential_id(value: &str, prefix: &str) -> Option<usize> {
    value
        .strip_prefix(prefix)
        .and_then(|suffix| suffix.strip_prefix('_'))
        .and_then(|suffix| suffix.parse().ok())
}

#[derive(Debug)]
pub enum CompileReservationError {
    Conflict,
    InvalidSession(String),
    PersistenceFailure(ProductStoreError),
}

impl std::fmt::Display for CompileReservationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict => formatter.write_str("SINGLE_CANDIDATE_COMPILE_RESERVATION_CONFLICT"),
            Self::InvalidSession(message) => write!(
                formatter,
                "SINGLE_CANDIDATE_COMPILE_RESERVATION_INVALID_SESSION: {message}"
            ),
            Self::PersistenceFailure(error) => write!(formatter, "persistence_failure: {error}"),
        }
    }
}

impl std::error::Error for CompileReservationError {}

/// `sha256("single_candidate_approval" + NUL + session + NUL + plan + NUL + refs...)`.
pub fn single_candidate_approval_attempt_id(
    session_id: &str,
    plan_id: &str,
    source_revision_ref: &str,
    plan_candidate_ir_ref: &str,
    mechanical_report_ref: &str,
) -> String {
    let canonical = [
        "single_candidate_approval",
        session_id,
        plan_id,
        source_revision_ref,
        plan_candidate_ir_ref,
        mechanical_report_ref,
    ]
    .join("\0");
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

/// Stable compile identity is bound to the persisted Approval tuple, never process time.
pub fn single_candidate_compile_id(
    session_id: &str,
    plan_id: &str,
    approval_attempt_id: &str,
    approved_at: &str,
) -> String {
    let canonical = [
        "single_candidate_compile",
        session_id,
        plan_id,
        approval_attempt_id,
        approved_at,
    ]
    .join("\0");
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

fn valid_single_candidate_session(
    session: &WorkspaceSessionRecord,
) -> Result<(SourceStoreScope, String, String, String), String> {
    if session.workspace_type != WorkspaceType::WorkItemPlan
        || session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        || session.single_candidate_phase != Some(SingleCandidatePhase::Approval)
    {
        return Err("session is not a single-candidate Approval WorkItemPlan".to_string());
    }
    let scope = SourceStoreScope {
        project_id: session.project_id.clone(),
        issue_id: session.issue_id.clone(),
        plan_id: session.entity_id.clone(),
    };
    let source_ref = session
        .work_item_plan_source_revision_ref
        .clone()
        .ok_or_else(|| "source revision ref is missing".to_string())?;
    let ir_ref = session
        .plan_candidate_ir_ref
        .clone()
        .ok_or_else(|| "plan candidate IR ref is missing".to_string())?;
    let report_ref = session
        .mechanical_report_ref
        .clone()
        .ok_or_else(|| "mechanical report ref is missing".to_string())?;
    validate_canonical_ref_for_scope(&scope, &source_ref, "source_revision")
        .map_err(|error| error.code().to_string())?;
    validate_canonical_ref_for_scope(&scope, &ir_ref, "plan_candidate_ir")
        .map_err(|error| error.code().to_string())?;
    validate_canonical_ref_for_scope(&scope, &report_ref, "mechanical_report")
        .map_err(|error| error.code().to_string())?;
    Ok((scope, source_ref, ir_ref, report_ref))
}

impl LifecycleStore {
    pub fn compare_and_save_single_candidate_approval(
        &self,
        expected: &WorkspaceSessionRecord,
        approval_attempt_id: &str,
        approved_at: &str,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let (_, source_ref, ir_ref, report_ref) = valid_single_candidate_session(expected)
            .map_err(|reason| ProductStoreError::InvalidRecord {
                kind: "single_candidate_approval",
                reason,
            })?;
        if DateTime::parse_from_rfc3339(approved_at).is_err() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "single_candidate_approval",
                reason: "approved_at must be RFC3339".to_string(),
            });
        }
        let expected_attempt = single_candidate_approval_attempt_id(
            &expected.id,
            &expected.entity_id,
            &source_ref,
            &ir_ref,
            &report_ref,
        );
        if approval_attempt_id != expected_attempt {
            return Err(ProductStoreError::InvalidRecord {
                kind: "single_candidate_approval",
                reason: "approval_attempt_id does not match durable references".to_string(),
            });
        }
        let session_path = self.find_workspace_session_path(&expected.id)?;
        with_exclusive_lock(&session_path, || {
            let mut stored: WorkspaceSessionRecord = read_json(&session_path)?;
            if stored != *expected {
                return Err(ProductStoreError::Conflict {
                    kind: "workspace_session",
                    id: expected.id.clone(),
                });
            }
            match (&stored.approval_attempt_id, &stored.approved_at) {
                (Some(existing_id), Some(existing_at))
                    if existing_id == approval_attempt_id && existing_at == approved_at =>
                {
                    return Ok(stored);
                }
                (None, None) => {}
                _ => {
                    return Err(ProductStoreError::Conflict {
                        kind: "single_candidate_approval",
                        id: stored.id.clone(),
                    });
                }
            }
            stored.approval_attempt_id = Some(approval_attempt_id.to_string());
            stored.approved_at = Some(approved_at.to_string());
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &stored)?;
            Ok(stored)
        })
    }

    pub fn put_compile_reservation_cas(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        session_id: &str,
        expected: &WorkspaceSessionRecord,
        reservation: &SingleCandidateCompileReservation,
    ) -> Result<WorkspaceSessionRecord, CompileReservationError> {
        if expected.id != session_id
            || expected.project_id != project_id
            || expected.issue_id != issue_id
            || expected.entity_id != plan_id
        {
            return Err(CompileReservationError::InvalidSession(
                "parameter scope does not match expected session".to_string(),
            ));
        }
        let (_, source_ref, ir_ref, report_ref) = valid_single_candidate_session(expected)
            .map_err(CompileReservationError::InvalidSession)?;
        let approval_attempt_id = expected.approval_attempt_id.as_deref().ok_or_else(|| {
            CompileReservationError::InvalidSession("approval_attempt_id is missing".to_string())
        })?;
        let approved_at = expected.approved_at.as_deref().ok_or_else(|| {
            CompileReservationError::InvalidSession("approved_at is missing".to_string())
        })?;
        if DateTime::parse_from_rfc3339(approved_at).is_err()
            || approval_attempt_id
                != single_candidate_approval_attempt_id(
                    &expected.id,
                    &expected.entity_id,
                    &source_ref,
                    &ir_ref,
                    &report_ref,
                )
        {
            return Err(CompileReservationError::InvalidSession(
                "durable Approval tuple is invalid".to_string(),
            ));
        }
        let compile_id = single_candidate_compile_id(
            &expected.id,
            &expected.entity_id,
            approval_attempt_id,
            approved_at,
        );
        let provenance_ref = format!(
            "project/{project_id}/issue/{issue_id}/plan/{plan_id}/publication_provenance/{compile_id}"
        );
        if reservation.compile_id != compile_id
            || reservation.now != approved_at
            || reservation.publication_provenance_ref != provenance_ref
        {
            return Err(CompileReservationError::InvalidSession(
                "reservation does not match durable Approval tuple".to_string(),
            ));
        }
        let session_path = self
            .find_workspace_session_path(session_id)
            .map_err(CompileReservationError::PersistenceFailure)?;
        with_exclusive_lock(&session_path, || {
            let mut stored: WorkspaceSessionRecord = read_json(&session_path)?;
            if stored != *expected {
                return Err(ProductStoreError::Conflict {
                    kind: "workspace_session",
                    id: session_id.to_string(),
                });
            }
            match &stored.compile_reservation {
                Some(existing) if existing == reservation => Ok(stored),
                Some(_) => Err(ProductStoreError::Conflict {
                    kind: "single_candidate_compile_reservation",
                    id: session_id.to_string(),
                }),
                None => {
                    stored.compile_reservation = Some(reservation.clone());
                    stored.updated_at = Utc::now().to_rfc3339();
                    write_json(&session_path, &stored)?;
                    Ok(stored)
                }
            }
        })
        .map_err(|error| match error {
            ProductStoreError::Conflict { .. } => CompileReservationError::Conflict,
            other => CompileReservationError::PersistenceFailure(other),
        })
    }
}
