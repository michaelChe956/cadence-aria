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
    /// 同一 CAS 持久化 Generate 阶段和 provider idempotency key；只有成功者可启动 provider。
    pub fn reserve_single_candidate_provider_start(
        &self,
        expected: &WorkspaceSessionRecord,
        provider_start_idempotency_key: &str,
    ) -> Result<(WorkspaceSessionRecord, bool), ProductStoreError> {
        if expected.workspace_type != WorkspaceType::WorkItemPlan
            || expected.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || provider_start_idempotency_key.trim().is_empty()
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "single_candidate_provider_start",
                reason: "invalid SingleCandidate provider reservation".to_string(),
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
            if matches!(
                stored.single_candidate_phase,
                Some(SingleCandidatePhase::Completed | SingleCandidatePhase::Failed)
            ) {
                return Ok((stored, false));
            }
            if !matches!(
                stored.single_candidate_phase,
                Some(SingleCandidatePhase::Prepare | SingleCandidatePhase::Generate)
            ) {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "single_candidate_provider_start",
                    reason: "provider start requires Prepare or Generate phase".to_string(),
                });
            }
            if stored
                .provider_start_ledger
                .iter()
                .any(|entry| entry.provider_start_idempotency_key == provider_start_idempotency_key)
            {
                return Ok((stored, false));
            }
            stored.single_candidate_phase = Some(SingleCandidatePhase::Generate);
            stored.status = crate::product::models::WorkspaceSessionStatus::Running;
            stored.provider_start_ledger.push(
                crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
                    provider_start_idempotency_key: provider_start_idempotency_key.to_string(),
                    started: true,
                },
            );
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &stored)?;
            Ok((stored, true))
        })
    }

    /// CAS 推进 SingleCandidate 的持久阶段。终态只能保持自身，防止重放重新启动工作流。
    pub fn compare_and_save_single_candidate_phase(
        &self,
        expected: &WorkspaceSessionRecord,
        phase: SingleCandidatePhase,
        status: crate::product::models::WorkspaceSessionStatus,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        if expected.workspace_type != WorkspaceType::WorkItemPlan
            || expected.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "single_candidate_phase",
                reason: "session is not a SingleCandidate WorkItemPlan".to_string(),
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
            if matches!(
                stored.single_candidate_phase,
                Some(SingleCandidatePhase::Completed | SingleCandidatePhase::Failed)
            ) && stored.single_candidate_phase != Some(phase.clone())
            {
                return Err(ProductStoreError::Conflict {
                    kind: "single_candidate_phase",
                    id: stored.id.clone(),
                });
            }
            stored.single_candidate_phase = Some(phase);
            stored.status = status;
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &stored)?;
            Ok(stored)
        })
    }

    /// Atomically bind generated immutable source and IR refs. Mechanical validation is
    /// deliberately a separate Evaluate CAS so a restart never treats generation as review.
    pub fn compare_and_save_single_candidate_generation(
        &self,
        expected: &WorkspaceSessionRecord,
        source_revision_ref: &str,
        plan_candidate_ir_ref: &str,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        if expected.workspace_type != WorkspaceType::WorkItemPlan
            || expected.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "single_candidate_generation",
                reason: "session is not a SingleCandidate WorkItemPlan".to_string(),
            });
        }
        let scope = SourceStoreScope {
            project_id: expected.project_id.clone(),
            issue_id: expected.issue_id.clone(),
            plan_id: expected.entity_id.clone(),
        };
        for (reference, kind) in [
            (source_revision_ref, "source_revision"),
            (plan_candidate_ir_ref, "plan_candidate_ir"),
        ] {
            validate_canonical_ref_for_scope(&scope, reference, kind).map_err(|error| {
                ProductStoreError::InvalidRecord {
                    kind: "single_candidate_generation",
                    reason: error.code().to_string(),
                }
            })?;
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
            let tuple_matches = stored.work_item_plan_source_revision_ref.as_deref()
                == Some(source_revision_ref)
                && stored.plan_candidate_ir_ref.as_deref() == Some(plan_candidate_ir_ref);
            if tuple_matches
                && stored.single_candidate_phase == Some(SingleCandidatePhase::Evaluate)
            {
                return Ok(stored);
            }
            let is_initial_generation = matches!(
                stored.single_candidate_phase,
                Some(SingleCandidatePhase::Prepare | SingleCandidatePhase::Generate)
            ) && stored.work_item_plan_source_revision_ref.is_none()
                && stored.plan_candidate_ir_ref.is_none()
                && stored.mechanical_report_ref.is_none();
            let is_repair_generation = stored.single_candidate_phase
                == Some(SingleCandidatePhase::Generate)
                && stored.work_item_plan_source_revision_ref.is_some()
                && stored.plan_candidate_ir_ref.is_some()
                && stored.mechanical_report_ref.is_some();
            if !is_initial_generation && !is_repair_generation {
                return Err(ProductStoreError::Conflict {
                    kind: "single_candidate_generation",
                    id: stored.id.clone(),
                });
            }
            stored.work_item_plan_source_revision_ref = Some(source_revision_ref.to_string());
            stored.plan_candidate_ir_ref = Some(plan_candidate_ir_ref.to_string());
            // A repaired source invalidates the previous report. It is restored only by
            // `compare_and_save_single_candidate_evaluation` after report persistence.
            if is_repair_generation {
                stored.mechanical_report_ref = None;
            }
            stored.single_candidate_phase = Some(SingleCandidatePhase::Evaluate);
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &stored)?;
            Ok(stored)
        })
    }

    /// Persist the immutable mechanical report produced by Evaluate. Invocation scope is
    /// materialized by reviewer startup ensure from the current ReviewerRun cycle.
    pub fn compare_and_save_single_candidate_evaluation(
        &self,
        expected: &WorkspaceSessionRecord,
        mechanical_report_ref: &str,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        if expected.workspace_type != WorkspaceType::WorkItemPlan
            || expected.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || expected.single_candidate_phase != Some(SingleCandidatePhase::Evaluate)
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "single_candidate_evaluation",
                reason: "session is not a SingleCandidate Evaluate WorkItemPlan".to_string(),
            });
        }
        let scope = SourceStoreScope {
            project_id: expected.project_id.clone(),
            issue_id: expected.issue_id.clone(),
            plan_id: expected.entity_id.clone(),
        };
        validate_canonical_ref_for_scope(&scope, mechanical_report_ref, "mechanical_report")
            .map_err(|error| ProductStoreError::InvalidRecord {
                kind: "single_candidate_evaluation",
                reason: error.code().to_string(),
            })?;
        if expected.work_item_plan_source_revision_ref.is_none()
            || expected.plan_candidate_ir_ref.is_none()
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "single_candidate_evaluation",
                reason: "generated source/IR refs are missing".to_string(),
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
            match stored.mechanical_report_ref.as_deref() {
                Some(existing) if existing == mechanical_report_ref => return Ok(stored),
                Some(_) => {
                    return Err(ProductStoreError::Conflict {
                        kind: "single_candidate_evaluation",
                        id: stored.id.clone(),
                    });
                }
                None => {}
            }
            stored.mechanical_report_ref = Some(mechanical_report_ref.to_string());
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &stored)?;
            Ok(stored)
        })
    }

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
                Some(existing)
                    if existing == reservation
                        && stored.publication_provenance_ref.as_deref()
                            == Some(reservation.publication_provenance_ref.as_str()) =>
                {
                    Ok(stored)
                }
                Some(_) => Err(ProductStoreError::Conflict {
                    kind: "single_candidate_compile_reservation",
                    id: session_id.to_string(),
                }),
                None => {
                    stored.compile_reservation = Some(reservation.clone());
                    stored.publication_provenance_ref =
                        Some(reservation.publication_provenance_ref.clone());
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

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::{
        CreateWorkspaceSessionInput, WorkItemPlanSessionOptions,
    };
    use crate::product::models::{ProviderName, WorkspaceType};
    use crate::product::work_item_plan_policy::{ReviewInvocationScope, RunHistory, RunPolicy};

    #[test]
    fn single_candidate_provider_start_reservation_is_one_shot_and_durable() {
        let temp = tempdir().unwrap();
        let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
        let session = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "entity_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(WorkItemPlanSessionOptions {
                    flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                    run_policy: RunPolicy::Interactive,
                    rollout_snapshot: true,
                }),
            })
            .expect("create candidate session");
        let key = "single_candidate_author:session:0";
        let (started, did_start) = store
            .reserve_single_candidate_provider_start(&session, key)
            .expect("reserve provider start");
        assert!(did_start);
        assert_eq!(
            started.single_candidate_phase,
            Some(SingleCandidatePhase::Generate)
        );
        assert_eq!(started.provider_start_ledger.len(), 1);
        let (replayed, did_replay) = store
            .reserve_single_candidate_provider_start(&started, key)
            .expect("replay provider start");
        assert!(!did_replay);
        assert_eq!(replayed, started);
    }

    #[test]
    fn single_candidate_evaluation_persists_report_without_upgrading_invocation_scope() {
        let temp = tempdir().unwrap();
        let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
        let mut session = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "entity_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(WorkItemPlanSessionOptions {
                    flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                    run_policy: RunPolicy::Interactive,
                    rollout_snapshot: true,
                }),
            })
            .expect("create single candidate session");
        session.single_candidate_phase = Some(SingleCandidatePhase::Evaluate);
        session.work_item_plan_source_revision_ref = Some(
            "project/project_0001/issue/issue_0001/plan/entity_0001/source_revision/source-001"
                .to_string(),
        );
        session.plan_candidate_ir_ref = Some(
            "project/project_0001/issue/issue_0001/plan/entity_0001/plan_candidate_ir/ir-001"
                .to_string(),
        );
        let initial_scope = ReviewInvocationScope::initial("review:node-a");
        session.review_invocation_scope = Some(initial_scope.clone());
        session.run_history = RunHistory {
            repairs_used: 1,
            ..RunHistory::default()
        };
        write_json(
            &store
                .workspace_sessions_root("project_0001", "issue_0001")
                .join(format!("{}.json", session.id)),
            &session,
        )
        .expect("seed evaluate session");

        let report_ref =
            "project/project_0001/issue/issue_0001/plan/entity_0001/mechanical_report/report-001";
        let expected = store
            .get_workspace_session(&session.id)
            .expect("load evaluate session");
        let saved = store
            .compare_and_save_single_candidate_evaluation(&expected, report_ref)
            .expect("persist mechanical report");

        assert_eq!(saved.mechanical_report_ref.as_deref(), Some(report_ref));
        assert_eq!(saved.review_invocation_scope, Some(initial_scope));
    }

    #[test]
    fn single_candidate_compile_id_uses_the_published_test_vector() {
        assert_eq!(
            single_candidate_compile_id(
                "session-001",
                "plan-001",
                "approval-001",
                "2026-08-27T12:34:56Z",
            ),
            "5a16e570210838318554c17b3ebd0c433c3001ce00adb7b8e9726d79aecf788e",
        );
    }

    #[test]
    fn single_candidate_approval_and_reservation_are_cas_bound_to_durable_refs() {
        let temp = tempdir().unwrap();
        let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
        let mut session = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "entity_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(WorkItemPlanSessionOptions {
                    flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                    run_policy: RunPolicy::Interactive,
                    rollout_snapshot: true,
                }),
            })
            .expect("create single candidate session");
        session.single_candidate_phase = Some(SingleCandidatePhase::Approval);
        session.work_item_plan_source_revision_ref = Some(
            "project/project_0001/issue/issue_0001/plan/entity_0001/source_revision/source-001"
                .to_string(),
        );
        session.plan_candidate_ir_ref = Some(
            "project/project_0001/issue/issue_0001/plan/entity_0001/plan_candidate_ir/ir-001"
                .to_string(),
        );
        session.mechanical_report_ref = Some(
            "project/project_0001/issue/issue_0001/plan/entity_0001/mechanical_report/report-001"
                .to_string(),
        );
        write_json(
            &store
                .workspace_sessions_root("project_0001", "issue_0001")
                .join(format!("{}.json", session.id)),
            &session,
        )
        .expect("seed durable candidate context");
        let approval_id = single_candidate_approval_attempt_id(
            &session.id,
            &session.entity_id,
            session
                .work_item_plan_source_revision_ref
                .as_deref()
                .unwrap(),
            session.plan_candidate_ir_ref.as_deref().unwrap(),
            session.mechanical_report_ref.as_deref().unwrap(),
        );
        let approved = store
            .compare_and_save_single_candidate_approval(
                &session,
                &approval_id,
                "2026-08-27T12:34:56Z",
            )
            .expect("approval CAS");
        assert_eq!(
            approved.approval_attempt_id.as_deref(),
            Some(approval_id.as_str())
        );
        assert_eq!(
            approved.approved_at.as_deref(),
            Some("2026-08-27T12:34:56Z")
        );

        let reservation = SingleCandidateCompileReservation {
            compile_id: single_candidate_compile_id(
                &approved.id,
                &approved.entity_id,
                &approval_id,
                "2026-08-27T12:34:56Z",
            ),
            now: "2026-08-27T12:34:56Z".to_string(),
            publication_provenance_ref: format!(
                "project/{}/issue/{}/plan/{}/publication_provenance/{}",
                approved.project_id,
                approved.issue_id,
                approved.entity_id,
                single_candidate_compile_id(
                    &approved.id,
                    &approved.entity_id,
                    &approval_id,
                    "2026-08-27T12:34:56Z",
                )
            ),
        };
        let reserved = store
            .put_compile_reservation_cas(
                "project_0001",
                "issue_0001",
                "entity_0001",
                &approved.id,
                &approved,
                &reservation,
            )
            .expect("reservation CAS");
        assert_eq!(reserved.compile_reservation.as_ref(), Some(&reservation));
        assert!(matches!(
            store.compare_and_save_single_candidate_approval(
                &session,
                &approval_id,
                "2026-08-27T12:34:56Z",
            ),
            Err(ProductStoreError::Conflict { .. })
        ));
    }
}
