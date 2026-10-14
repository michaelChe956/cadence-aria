use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use super::*;
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::models::{
    HumanGateTurn, HumanGateTurnFailureClass, HumanGateTurnStatus,
    SingleCandidateCompileReservation, SingleCandidatePhase, StructuralAttemptReason,
    StructuralCheckpoint, StructuralCheckpointStatus, StructuralGovernanceState,
    StructuralGrantPhase, StructuralSettlementError, UserBudgetReceipt, UserBudgetReceiptKind,
    WorkspaceSessionRecord, WorkspaceType, structural_checkpoint_id,
};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::product::work_item_plan_source_store::{
    SourceStoreScope, validate_canonical_ref_for_scope,
};
use crate::product::workspace_engine::draft_batch::compile_support::PlanningStructureReport;
use crate::product::workspace_engine::freeze_persisted_structure_report;
use crate::web::workspace_ws_types::ArtifactVersion;

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

/// P2 GAP-E/G（Task 0.1）：人工显式重驱的失败 SC 单发认领结果。
pub enum ClaimScRecoveryOutcome {
    /// 本次认领成功写入（受理派发由调用方继续）。
    Claimed(WorkspaceSessionRecord),
    /// 同键重放：读回原认领（含其持久结果状态），不再派发。
    Existing(WorkspaceSessionRecord),
}

impl LifecycleStore {
    /// 同一 CAS 持久化 Generate 阶段和 provider idempotency key；只有成功者可启动 provider。
    pub fn reserve_single_candidate_provider_start(
        &self,
        expected: &WorkspaceSessionRecord,
        provider_start_idempotency_key: &str,
    ) -> Result<(WorkspaceSessionRecord, bool), ProductStoreError> {
        let admission_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
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
                Some(SingleCandidatePhase::Completed)
            ) {
                return Ok((stored, false));
            }
            if stored.single_candidate_phase == Some(SingleCandidatePhase::Failed) {
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
                .repair_reservation
                .as_ref()
                .is_some_and(|reservation| {
                    reservation.provider_start_idempotency_key == provider_start_idempotency_key
                        && reservation.state
                            == crate::product::work_item_plan_policy::RepairReservationState::Reserved
                })
            {
                stored
                    .repair_reservation
                    .as_mut()
                    .expect("repair reservation checked above")
                    .state = crate::product::work_item_plan_policy::RepairReservationState::ProviderStarted;
                stored.updated_at = Utc::now().to_rfc3339();
                write_json(&session_path, &stored)?;
                return Ok((stored, true));
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
                    provider: None,
                    started_at: None,
                },
            );
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &stored)?;
            Ok((stored, true))
        })
        })
    }

    /// 用户显式重新生成时，原子重臂失败的 SingleCandidate 会话；已完成会话保持拒绝。
    pub fn rearm_failed_single_candidate_for_start_generation(
        &self,
        expected: &WorkspaceSessionRecord,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let admission_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
            if expected.workspace_type != WorkspaceType::WorkItemPlan
                || expected.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "single_candidate_rearm",
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
                if stored.single_candidate_phase == Some(SingleCandidatePhase::Completed) {
                    return Err(ProductStoreError::Conflict {
                        kind: "single_candidate_rearm",
                        id: stored.id.clone(),
                    });
                }
                if stored.single_candidate_phase == Some(SingleCandidatePhase::Failed) {
                    stored.single_candidate_phase = Some(SingleCandidatePhase::Prepare);
                    stored.status = crate::product::models::WorkspaceSessionStatus::Open;
                    stored.updated_at = Utc::now().to_rfc3339();
                    write_json(&session_path, &stored)?;
                }
                Ok(stored)
            })
        })
    }

    /// 在 session 文件锁内冻结 `(failed_node_id, command_id)` 单发认领：
    /// 仅受理 durable Failed 的 SingleCandidate 会话；同键返回 Existing，
    /// 异键 Conflict（409）；认领不可复位。
    pub fn claim_failed_sc_run_retry(
        &self,
        session_id: &str,
        failed_node_id: &str,
        command_id: &str,
    ) -> Result<ClaimScRecoveryOutcome, ProductStoreError> {
        let (session_path, admission_scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
            with_exclusive_lock(&session_path, || {
                let mut stored: WorkspaceSessionRecord = read_json(&session_path)?;
                if stored.workspace_type != WorkspaceType::WorkItemPlan
                    || stored.flow_kind != WorkItemPlanFlowKind::SingleCandidate
                    || stored.single_candidate_phase != Some(SingleCandidatePhase::Failed)
                {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "sc_recovery_claim",
                        reason: "session is not a failed SingleCandidate WorkItemPlan".to_string(),
                    });
                }
                if let Some(claim) = stored.sc_recovery_claim.as_ref() {
                    if claim.failed_node_id == failed_node_id && claim.command_id == command_id {
                        return Ok(ClaimScRecoveryOutcome::Existing(stored));
                    }
                    return Err(ProductStoreError::Conflict {
                        kind: "sc_recovery_claim",
                        id: stored.id,
                    });
                }
                stored.sc_recovery_claim = Some(crate::product::models::ScRecoveryClaim {
                    failed_node_id: failed_node_id.to_string(),
                    command_id: command_id.to_string(),
                    state: crate::product::models::ScRecoveryClaimState::Accepted,
                });
                stored.updated_at = Utc::now().to_rfc3339();
                write_json(&session_path, &stored)?;
                Ok(ClaimScRecoveryOutcome::Claimed(stored))
            })
        })
    }

    /// 派发外部副作用是否发生不可证明时，把认领结果固化为 NeedsHuman
    ///（保留认领、不重试、不复位；人工分诊）。
    pub fn mark_failed_sc_recovery_needs_human(
        &self,
        session_id: &str,
        failed_node_id: &str,
        command_id: &str,
    ) -> Result<(), ProductStoreError> {
        let (session_path, admission_scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
            with_exclusive_lock(&session_path, || {
                let mut stored: WorkspaceSessionRecord = read_json(&session_path)?;
                let matches_claim = stored.sc_recovery_claim.as_ref().is_some_and(|claim| {
                    claim.failed_node_id == failed_node_id && claim.command_id == command_id
                });
                if !matches_claim {
                    return Err(ProductStoreError::Conflict {
                        kind: "sc_recovery_claim",
                        id: stored.id,
                    });
                }
                if let Some(claim) = stored.sc_recovery_claim.as_mut() {
                    claim.state = crate::product::models::ScRecoveryClaimState::NeedsHuman;
                }
                stored.updated_at = Utc::now().to_rfc3339();
                write_json(&session_path, &stored)?;
                Ok(())
            })
        })
    }

    /// CAS 推进 SingleCandidate 的持久阶段。终态只能保持自身，防止重放重新启动工作流。
    pub fn compare_and_save_single_candidate_phase(
        &self,
        expected: &WorkspaceSessionRecord,
        phase: SingleCandidatePhase,
        status: crate::product::models::WorkspaceSessionStatus,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let admission_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
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
        let admission_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
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
                let is_initial_generation =
                    matches!(
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
                    // F-54 B1：修复轮换 refs 即产生新候选，旧 Approval 元组对新 refs 失效。
                    // 不清空会在下次 approve 命中 compile 的元组冲突臂（attempt_id 按新
                    // refs 重算 ≠ 旧元组）→ durable Failed 污染门相位（0009 事故根因）。
                    // 清空后下次 approve 走 (None, None) 全新元组分支。
                    stored.approval_attempt_id = None;
                    stored.approved_at = None;
                    stored.compile_reservation = None;
                }
                stored.single_candidate_phase = Some(SingleCandidatePhase::Evaluate);
                stored.updated_at = Utc::now().to_rfc3339();
                write_json(&session_path, &stored)?;
                Ok(stored)
            })
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_human_gate_revision(
        &self,
        expected: &WorkspaceSessionRecord,
        mut turn: crate::product::models::HumanGateTurn,
        source_revision_ref: &str,
        plan_candidate_ir_ref: &str,
        mechanical_report_ref: &str,
        artifact_versions: &[ArtifactVersion],
        result_artifact_ref: &str,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let admission_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
            use crate::product::models::{HumanGateTurnStatus, WorkspaceType};
            if expected.workspace_type != WorkspaceType::WorkItemPlan
                || expected.flow_kind != WorkItemPlanFlowKind::SingleCandidate
                || turn.status != HumanGateTurnStatus::Completed
            {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "human_gate_revision",
                    reason: "invalid completed human gate revision".to_string(),
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
                (mechanical_report_ref, "mechanical_report"),
            ] {
                validate_canonical_ref_for_scope(&scope, reference, kind).map_err(|error| {
                    ProductStoreError::InvalidRecord {
                        kind: "human_gate_revision",
                        reason: error.code().to_string(),
                    }
                })?;
            }
            let session_path = self.find_workspace_session_path(&expected.id)?;
            let turn_path = self.human_gate_turn_path(expected, &turn.turn_id)?;
            with_exclusive_lock(&session_path, || {
                let stored: WorkspaceSessionRecord = read_json(&session_path)?;
                if stored != *expected {
                    return Err(ProductStoreError::Conflict {
                        kind: "workspace_session",
                        id: expected.id.clone(),
                    });
                }
                let existing = read_json(&turn_path)?;
                Self::validate_human_gate_turn(&existing, &stored.id, &turn.turn_id)?;
                let immutable_fields_match = existing.session_id == turn.session_id
                    && existing.command_id == turn.command_id
                    && existing.feedback_text == turn.feedback_text
                    && existing.budget_reserved == turn.budget_reserved
                    && existing.source_hash == turn.source_hash
                    && existing.created_at == turn.created_at;
                if !immutable_fields_match
                    || existing.status == HumanGateTurnStatus::Completed
                    || existing.status == HumanGateTurnStatus::Failed
                {
                    return Err(ProductStoreError::Conflict {
                        kind: "human_gate_turn",
                        id: turn.turn_id.clone(),
                    });
                }
                if existing.attempt_no != turn.attempt_no {
                    return Err(ProductStoreError::Conflict {
                        kind: "human_gate_turn",
                        id: turn.turn_id.clone(),
                    });
                }
                turn.updated_at = Utc::now().to_rfc3339();
                let original_versions = self.list_artifact_versions(&stored.id)?;
                if original_versions != *artifact_versions {
                    self.save_artifact_versions(&stored.id, artifact_versions)?;
                }
                let mut next = stored;
                // F-54 B1 同构（诊断冻结）：门内修订真正轮换 refs 即产生新候选，
                // 旧 Approval 绑定对新 refs 失效，必须随轮换一并清空——否则复评
                // 通过后的 confirm 在 compile 冲突臂以新 refs 重算 attempt id ≠
                // 旧元组 → `single_candidate_approval_conflict`，门永久卡死；旧
                // compile_reservation/publication_provenance_ref 同属旧编译代次，
                // 一并失效，下次 approve 走 (None, None) 全新元组并建立新代次。
                // refs 未轮换（同 source 重放，source-only 内容寻址 refs 不变）时
                // 现元组仍与 refs 精确绑定，原样保留。
                let refs_rotated = next.work_item_plan_source_revision_ref.as_deref()
                    != Some(source_revision_ref)
                    || next.plan_candidate_ir_ref.as_deref() != Some(plan_candidate_ir_ref)
                    || next.mechanical_report_ref.as_deref() != Some(mechanical_report_ref);
                next.work_item_plan_source_revision_ref = Some(source_revision_ref.to_string());
                next.plan_candidate_ir_ref = Some(plan_candidate_ir_ref.to_string());
                next.mechanical_report_ref = Some(mechanical_report_ref.to_string());
                if refs_rotated {
                    next.approval_attempt_id = None;
                    next.approved_at = None;
                    next.compile_reservation = None;
                    next.publication_provenance_ref = None;
                }
                next.updated_at = turn.updated_at.clone();
                if let Err(error) = write_json(&session_path, &next) {
                    let _ = self.save_artifact_versions(&next.id, &original_versions);
                    return Err(error);
                }
                turn.result_artifact_ref = Some(result_artifact_ref.to_string());
                if let Err(error) = write_json(&turn_path, &turn) {
                    let _ = write_json(&session_path, &expected);
                    let _ = self.save_artifact_versions(&next.id, &original_versions);
                    return Err(error);
                }
                Ok(next)
            })
        })
    }

    /// materialized by reviewer startup ensure from the current ReviewerRun cycle.
    pub fn compare_and_save_single_candidate_evaluation(
        &self,
        expected: &WorkspaceSessionRecord,
        mechanical_report_ref: &str,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let admission_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
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
                    Some(existing) => {
                        // report_id 内容寻址化后，同内容重放会轮换 id（旧会话
                        // 持旧格式 ref 或早前内容寻址 ref）。ref 不等时按两 ref
                        // 各自加载 record，比对剔除 id/content_hash 后的全量
                        // 内容摘要：等价内容接受 ref 轮换（Evaluate 重入幂等
                        // 与内容寻址化前一致）；内容真变或任一 ref 悬空/不可
                        // 解析（无法证明等价）维持 Conflict fail-closed。
                        if !equivalent_mechanical_reports(
                            self.app_paths(),
                            &scope,
                            existing,
                            mechanical_report_ref,
                        ) {
                            return Err(ProductStoreError::Conflict {
                                kind: "single_candidate_evaluation",
                                id: stored.id.clone(),
                            });
                        }
                    }
                    None => {}
                }
                stored.mechanical_report_ref = Some(mechanical_report_ref.to_string());
                stored.updated_at = Utc::now().to_rfc3339();
                write_json(&session_path, &stored)?;
                Ok(stored)
            })
        })
    }

    pub fn compare_and_save_single_candidate_approval(
        &self,
        expected: &WorkspaceSessionRecord,
        approval_attempt_id: &str,
        approved_at: &str,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let admission_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
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
        let admission_scope = self
            .entity_scope_for_session(expected)
            .map_err(CompileReservationError::PersistenceFailure)?;
        let outcome = self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
            let session_path = self.find_workspace_session_path(session_id)?;
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
        });
        outcome.map_err(|error| match error {
            ProductStoreError::Conflict { .. } => CompileReservationError::Conflict,
            other => CompileReservationError::PersistenceFailure(other),
        })
    }

    /// SG4b（plan v1.1 §0.5 / REQ-SG-04）：合法结构修订交付。与
    /// `complete_human_gate_revision` 分离——turn 保持 Running（完成化由
    /// 评审/finalize 链推进，不提前 Completed）；refs 轮换、checkpoint、
    /// deliver receipt 与 grant phase→Reviewing 同一 session CAS。
    /// 只接受已含 `Some` 合法 payload 的同代 refs（§0.6.2），deliver 经
    /// SG4a 结算核心 settle 恰一次（REQ-CG-02：结构合法且同代 refs 提交
    /// 后同 turn settle 一次为 delivered_charge）。
    #[allow(clippy::too_many_arguments)]
    pub fn commit_structurally_valid_revision(
        &self,
        expected: &WorkspaceSessionRecord,
        mut turn: HumanGateTurn,
        source_revision_ref: &str,
        plan_candidate_ir_ref: &str,
        mechanical_report_ref: &str,
        artifact_versions: &[ArtifactVersion],
        result_artifact_ref: &str,
        structure_report: &PlanningStructureReport,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let admission_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
            if expected.workspace_type != WorkspaceType::WorkItemPlan
                || expected.flow_kind != WorkItemPlanFlowKind::SingleCandidate
                || turn.status != HumanGateTurnStatus::Running
            {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "structural_revision_commit",
                    reason: "commit requires a Running single-candidate human gate turn"
                        .to_string(),
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
                (mechanical_report_ref, "mechanical_report"),
            ] {
                validate_canonical_ref_for_scope(&scope, reference, kind).map_err(|error| {
                    ProductStoreError::InvalidRecord {
                        kind: "structural_revision_commit",
                        reason: error.code().to_string(),
                    }
                })?;
            }
            if !structure_report.structurally_valid {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "structural_revision_commit",
                    reason: "structure report is not structurally valid".to_string(),
                });
            }
            // 同源复验：三 refs 装载 + Some 合法 payload + 传入 SG1 报告经
            // SG3a.1 冻结单源与持久 payload 全等。
            let (source_record, payload) = load_sealed_structure_generation(
                self.app_paths(),
                &scope,
                source_revision_ref,
                plan_candidate_ir_ref,
                mechanical_report_ref,
                structure_report,
            )?;
            let expected_checkpoint_id = structural_checkpoint_id(
                &turn.turn_id,
                source_revision_ref,
                plan_candidate_ir_ref,
                mechanical_report_ref,
            );

            let session_path = self.find_workspace_session_path(&expected.id)?;
            let turn_path = self.human_gate_turn_path(expected, &turn.turn_id)?;
            with_exclusive_lock(&session_path, || {
                let stored: WorkspaceSessionRecord = read_json(&session_path)?;
                if stored != *expected {
                    return Err(ProductStoreError::Conflict {
                        kind: "workspace_session",
                        id: expected.id.clone(),
                    });
                }
                let existing: HumanGateTurn = read_json(&turn_path)?;
                Self::validate_human_gate_turn(&existing, &stored.id, &turn.turn_id)?;
                let immutable_fields_match = existing.session_id == turn.session_id
                    && existing.command_id == turn.command_id
                    && existing.feedback_text == turn.feedback_text
                    && existing.budget_reserved == turn.budget_reserved
                    && existing.source_hash == turn.source_hash
                    && existing.created_at == turn.created_at;
                if !immutable_fields_match
                    || existing.status != HumanGateTurnStatus::Running
                    || existing.attempt_no != turn.attempt_no
                {
                    return Err(ProductStoreError::Conflict {
                        kind: "human_gate_turn",
                        id: turn.turn_id.clone(),
                    });
                }
                let state = stored.structural_governance_state.as_ref().ok_or_else(|| {
                    ProductStoreError::InvalidRecord {
                        kind: "structural_governance_state",
                        reason: "structural governance state missing".to_string(),
                    }
                })?;
                let grant = state.current_grant.as_ref().ok_or_else(|| {
                    ProductStoreError::InvalidRecord {
                        kind: "structural_revision_grant",
                        reason: "no active structural revision grant".to_string(),
                    }
                })?;
                let Some(grant_turn_id) = grant.turn_id.as_deref() else {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "structural_revision_grant",
                        id: turn.turn_id.clone(),
                    });
                };
                if grant_turn_id != turn.turn_id {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "structural_revision_grant",
                        id: turn.turn_id.clone(),
                    });
                }
                if let Some(grant_command_id) = grant.command_id.as_deref()
                    && grant_command_id != existing.command_id
                {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "structural_revision_grant",
                        id: existing.command_id.clone(),
                    });
                }
                // 幂等重放窗口：前次 commit 已落（同代 refs/checkpoint/receipt）
                // 时只补 turn 结果绑定，零新结算（跨文件裂缝按 durable 证据恢复）。
                let receipt_settled_generation =
                    state
                        .receipt_for_turn(&turn.turn_id)
                        .is_some_and(|receipt| {
                            receipt.kind == UserBudgetReceiptKind::DeliveredCharge
                                && receipt.source_revision_ref.as_deref()
                                    == Some(source_revision_ref)
                                && receipt.plan_candidate_ir_ref.as_deref()
                                    == Some(plan_candidate_ir_ref)
                                && receipt.structure_report_ref.as_deref()
                                    == Some(mechanical_report_ref)
                        });
                let replay = receipt_settled_generation
                    && stored.work_item_plan_source_revision_ref.as_deref()
                        == Some(source_revision_ref)
                    && stored.plan_candidate_ir_ref.as_deref() == Some(plan_candidate_ir_ref)
                    && stored.mechanical_report_ref.as_deref() == Some(mechanical_report_ref)
                    && state
                        .structure_checkpoints
                        .iter()
                        .any(|checkpoint| checkpoint.checkpoint_id == expected_checkpoint_id);
                if !replay {
                    match grant.phase {
                        // 评审/终态/业务门阶段无第二次交付（同代重放除外）。
                        Some(
                            StructuralGrantPhase::Reviewing
                            | StructuralGrantPhase::Resolved
                            | StructuralGrantPhase::SystemFault
                            | StructuralGrantPhase::BusinessGate,
                        ) => {
                            return Err(ProductStoreError::Conflict {
                                kind: "structural_revision_grant",
                                id: grant.grant_id.clone(),
                            });
                        }
                        _ => {}
                    }
                }

                let original_versions = self.list_artifact_versions(&stored.id)?;
                let mut next = stored.clone();
                // refs 真正轮换即产生新候选：旧 Approval/编译代次整组失效
                // （与 complete_human_gate_revision 同语义，F-54 B1）。
                let refs_rotated = next.work_item_plan_source_revision_ref.as_deref()
                    != Some(source_revision_ref)
                    || next.plan_candidate_ir_ref.as_deref() != Some(plan_candidate_ir_ref)
                    || next.mechanical_report_ref.as_deref() != Some(mechanical_report_ref);
                next.work_item_plan_source_revision_ref = Some(source_revision_ref.to_string());
                next.plan_candidate_ir_ref = Some(plan_candidate_ir_ref.to_string());
                next.mechanical_report_ref = Some(mechanical_report_ref.to_string());
                if refs_rotated {
                    next.approval_attempt_id = None;
                    next.approved_at = None;
                    next.compile_reservation = None;
                    next.publication_provenance_ref = None;
                }
                // deliver settle 恰一次（同 CAS；SG4a 结算核心单源）。
                if !replay {
                    let receipt = UserBudgetReceipt {
                        kind: UserBudgetReceiptKind::DeliveredCharge,
                        turn_id: turn.turn_id.clone(),
                        command_id: existing.command_id.clone(),
                        attempt_reason: Some(StructuralAttemptReason::HumanRevision),
                        source_revision_ref: Some(source_revision_ref.to_string()),
                        plan_candidate_ir_ref: Some(plan_candidate_ir_ref.to_string()),
                        structure_report_ref: Some(mechanical_report_ref.to_string()),
                        context_digest: Some(payload.context_digest.clone()),
                        reason: None,
                        settled_at: Utc::now().to_rfc3339(),
                    };
                    let snapshot = next.human_gate_snapshot.as_mut();
                    next.structural_governance_state
                        .as_mut()
                        .expect("state checked above")
                        .settle_turn_receipt(snapshot, &turn.turn_id, &receipt)
                        .map_err(|error| match error {
                            StructuralSettlementError::TerminalConflict => {
                                ProductStoreError::Conflict {
                                    kind: "user_budget_receipt",
                                    id: turn.turn_id.clone(),
                                }
                            }
                            StructuralSettlementError::CommandMismatch => {
                                ProductStoreError::IdentityMismatch {
                                    kind: "user_budget_hold",
                                    id: existing.command_id.clone(),
                                }
                            }
                            StructuralSettlementError::NoActiveHold => {
                                ProductStoreError::InvalidRecord {
                                    kind: "user_budget_hold",
                                    reason: "no active hold for turn".to_string(),
                                }
                            }
                        })?;
                }
                {
                    let state = next
                        .structural_governance_state
                        .as_mut()
                        .expect("state checked above");
                    let grant = state.current_grant.as_mut().expect("grant checked above");
                    // SG6a.3（REQ-SG-05「已终态 grant 拒旁路」）：commit→终态
                    // 断点崩溃重放不把已终态（Resolved/SystemFault/BusinessGate）
                    // grant 降级回 Reviewing——终态相位按 durable 证据保持。
                    match grant.phase {
                        Some(
                            StructuralGrantPhase::Resolved
                            | StructuralGrantPhase::SystemFault
                            | StructuralGrantPhase::BusinessGate,
                        ) => {}
                        _ => grant.phase = Some(StructuralGrantPhase::Reviewing),
                    }
                    grant.candidate_source_revision_ref = Some(source_revision_ref.to_string());
                    grant.plan_candidate_ir_ref = Some(plan_candidate_ir_ref.to_string());
                    grant.mechanical_report_ref = Some(mechanical_report_ref.to_string());
                    state.current_structure_report_ref = Some(mechanical_report_ref.to_string());
                    state.current_context_digest = Some(payload.context_digest.clone());
                    let checkpoint = StructuralCheckpoint {
                        checkpoint_id: expected_checkpoint_id.clone(),
                        source_revision_ref: Some(source_revision_ref.to_string()),
                        plan_candidate_ir_ref: Some(plan_candidate_ir_ref.to_string()),
                        structure_report_ref: Some(mechanical_report_ref.to_string()),
                        artifact_version: artifact_versions.last().map(|version| version.version),
                        source_hash: Some(source_record.source_revision_hash.clone()),
                        context_digest: Some(payload.context_digest.clone()),
                        authorization_hash: Some(grant.authorization_hash.clone()),
                        receipt_ref: Some(format!("user_budget_receipt:{}", turn.turn_id)),
                        review_scope: None,
                        status: StructuralCheckpointStatus::Active,
                        recorded_at: Utc::now().to_rfc3339(),
                    };
                    state.record_structure_checkpoint(checkpoint);
                }
                if original_versions != *artifact_versions {
                    self.save_artifact_versions(&stored.id, artifact_versions)?;
                }
                next.updated_at = Utc::now().to_rfc3339();
                // turn 保持 Running（与 complete 分离）：只绑结果 ref。
                turn.result_artifact_ref = Some(result_artifact_ref.to_string());
                turn.updated_at = next.updated_at.clone();
                if let Err(error) = write_json(&session_path, &next) {
                    let _ = self.save_artifact_versions(&next.id, &original_versions);
                    return Err(error);
                }
                if let Err(error) = write_json(&turn_path, &turn) {
                    let _ = write_json(&session_path, &expected);
                    let _ = self.save_artifact_versions(&next.id, &original_versions);
                    return Err(error);
                }
                Ok(next)
            })
        })
    }

    /// SG4b（plan v1.1 §0.5 / REQ-SG-04）：grant 终态化。turn
    /// Completed/Failed 且取 review 或耗尽 typed 证据；不提前 Complete、
    /// 不新开 author（终态 grant）；grant 结束不清 history（checkpoint/
    /// receipt 保留）。
    pub fn finalize_structural_revision_grant(
        &self,
        expected: &WorkspaceSessionRecord,
        turn: HumanGateTurn,
        grant_id: &str,
        review_node_id: Option<&str>,
        review_scope_digest: Option<&str>,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let admission_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
            if expected.workspace_type != WorkspaceType::WorkItemPlan
                || expected.flow_kind != WorkItemPlanFlowKind::SingleCandidate
                || !matches!(
                    turn.status,
                    HumanGateTurnStatus::Completed | HumanGateTurnStatus::Failed
                )
            {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "structural_revision_finalize",
                    reason: "finalize requires a Completed or Failed single-candidate turn"
                        .to_string(),
                });
            }
            let session_path = self.find_workspace_session_path(&expected.id)?;
            let turn_path = self.human_gate_turn_path(expected, &turn.turn_id)?;
            with_exclusive_lock(&session_path, || {
                let stored: WorkspaceSessionRecord = read_json(&session_path)?;
                if stored != *expected {
                    return Err(ProductStoreError::Conflict {
                        kind: "workspace_session",
                        id: expected.id.clone(),
                    });
                }
                let existing: HumanGateTurn = read_json(&turn_path)?;
                Self::validate_human_gate_turn(&existing, &stored.id, &turn.turn_id)?;
                let turn_matches = existing.session_id == turn.session_id
                    && existing.command_id == turn.command_id
                    && existing.feedback_text == turn.feedback_text
                    && existing.budget_reserved == turn.budget_reserved
                    && existing.source_hash == turn.source_hash
                    && existing.created_at == turn.created_at
                    && existing.status == turn.status
                    && existing.attempt_no == turn.attempt_no
                    && existing.failure_class == turn.failure_class;
                if !turn_matches {
                    return Err(ProductStoreError::Conflict {
                        kind: "human_gate_turn",
                        id: turn.turn_id.clone(),
                    });
                }
                let state = stored.structural_governance_state.as_ref().ok_or_else(|| {
                    ProductStoreError::InvalidRecord {
                        kind: "structural_governance_state",
                        reason: "structural governance state missing".to_string(),
                    }
                })?;
                let grant = state.current_grant.as_ref().ok_or_else(|| {
                    ProductStoreError::InvalidRecord {
                        kind: "structural_revision_grant",
                        reason: "no active structural revision grant".to_string(),
                    }
                })?;
                if grant.grant_id != grant_id {
                    return Err(ProductStoreError::Conflict {
                        kind: "structural_revision_grant",
                        id: grant_id.to_string(),
                    });
                }
                if grant.turn_id.as_deref() != Some(turn.turn_id.as_str()) {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "structural_revision_grant",
                        id: turn.turn_id.clone(),
                    });
                }
                if let Some(grant_command_id) = grant.command_id.as_deref()
                    && grant_command_id != existing.command_id
                {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "structural_revision_grant",
                        id: existing.command_id.clone(),
                    });
                }
                // 幂等重放：grant 已按同证据终态化 → 零写返回。
                let already_finalized = match turn.status {
                    HumanGateTurnStatus::Completed => {
                        grant.phase == Some(StructuralGrantPhase::Resolved)
                            && grant.review_identity.as_deref() == review_node_id
                    }
                    _ => grant.phase == Some(StructuralGrantPhase::SystemFault),
                };
                if already_finalized {
                    return Ok(stored);
                }

                let mut next = stored.clone();
                match turn.status {
                    HumanGateTurnStatus::Completed => {
                        // typed review 证据：评审节点 + scope digest（绑定同
                        // refs/context）缺一不可，不提前 Resolved。
                        let (Some(node_id), Some(scope_digest)) =
                            (review_node_id, review_scope_digest)
                        else {
                            return Err(ProductStoreError::InvalidRecord {
                                kind: "structural_revision_finalize",
                                reason: "completed turn requires review join evidence".to_string(),
                            });
                        };
                        // durable 分账证据：commit 已结算 delivered receipt。
                        if !state
                            .receipt_for_turn(&turn.turn_id)
                            .is_some_and(|receipt| {
                                receipt.kind == UserBudgetReceiptKind::DeliveredCharge
                            })
                        {
                            return Err(ProductStoreError::InvalidRecord {
                                kind: "structural_revision_finalize",
                                reason: "completed turn has no delivered receipt".to_string(),
                            });
                        }
                        let state = next
                            .structural_governance_state
                            .as_mut()
                            .expect("state checked above");
                        // Active checkpoint 回填 review scope；既有不同 scope →
                        // 绑定完整性冲突（同 checkpoint 不重绑第二次评审身份）。
                        if let Some(checkpoint) =
                            state.structure_checkpoints.iter_mut().find(|checkpoint| {
                                checkpoint.status == StructuralCheckpointStatus::Active
                            })
                        {
                            match checkpoint.review_scope.as_deref() {
                                None => checkpoint.review_scope = Some(scope_digest.to_string()),
                                Some(existing_scope) if existing_scope == scope_digest => {}
                                Some(_) => {
                                    return Err(ProductStoreError::Conflict {
                                        kind: "structural_checkpoint",
                                        id: checkpoint.checkpoint_id.clone(),
                                    });
                                }
                            }
                        }
                        let grant = state.current_grant.as_mut().expect("grant checked above");
                        grant.phase = Some(StructuralGrantPhase::Resolved);
                        grant.review_identity = Some(node_id.to_string());
                    }
                    HumanGateTurnStatus::Failed => {
                        // 耗尽 typed 证据：released receipt（未交付结构耗尽的
                        // hold 已按 REQ-CG-02 释放）。SG6a.3（REQ-CG-03「到顶
                        // 从未deliver时Failed(validation_reject)」）：失败类
                        // 收敛为 ValidationReject（结构 attempt 耗尽不冒充用户
                        // 预算耗尽，REQ-CG-04「结构耗尽不得假批准」）；保留
                        // BudgetExhausted 兼容 SG4b 既有合同。
                        if turn.failure_class != Some(HumanGateTurnFailureClass::BudgetExhausted)
                            && turn.failure_class
                                != Some(HumanGateTurnFailureClass::ValidationReject)
                        {
                            return Err(ProductStoreError::InvalidRecord {
                                kind: "structural_revision_finalize",
                                reason: "failed turn requires structural exhaustion evidence"
                                    .to_string(),
                            });
                        }
                        if !state
                            .receipt_for_turn(&turn.turn_id)
                            .is_some_and(|receipt| {
                                receipt.kind == UserBudgetReceiptKind::ReleasedStructural
                            })
                        {
                            return Err(ProductStoreError::InvalidRecord {
                                kind: "structural_revision_finalize",
                                reason: "exhausted turn has no released_structural receipt"
                                    .to_string(),
                            });
                        }
                        next.structural_governance_state
                            .as_mut()
                            .expect("state checked above")
                            .current_grant
                            .as_mut()
                            .expect("grant checked above")
                            .phase = Some(StructuralGrantPhase::SystemFault);
                    }
                    _ => unreachable!("terminal status checked at entry"),
                }
                next.updated_at = Utc::now().to_rfc3339();
                write_json(&session_path, &next)?;
                Ok(next)
            })
        })
    }

    /// SG4b（plan v1.1 §0.5 / REQ-SG-04）：整组历史恢复（纯状态回绑）。
    /// 按当前 Design/member/options/baseline 派生的 SG1 合法报告对
    /// checkpoint 三 refs 同源重验；缺 join/漂移/新反馈未满足一律拒绝
    /// 回退；回绑同 CAS 失效旧审批，不复活批准、不把旧 receipt 计本轮、
    /// 零 author 启动（ledger/attempt/prompt 不动）。
    pub fn restore_structural_checkpoint(
        &self,
        expected: &WorkspaceSessionRecord,
        checkpoint: &StructuralCheckpoint,
        current_report: &PlanningStructureReport,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let admission_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
            if expected.workspace_type != WorkspaceType::WorkItemPlan
                || expected.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "structural_checkpoint_restore",
                    reason: "session is not a SingleCandidate WorkItemPlan".to_string(),
                });
            }
            // 整组三 refs 完整（缺任一＝历史证据不全——只有 Markdown 或缺
            // IR/report 的历史版本禁止回退，仅可作 author 参考）。
            let source_revision_ref = checkpoint.source_revision_ref.clone().ok_or_else(|| {
                ProductStoreError::InvalidRecord {
                    kind: "structural_checkpoint_restore",
                    reason: "checkpoint source_revision_ref is missing".to_string(),
                }
            })?;
            let plan_candidate_ir_ref =
                checkpoint.plan_candidate_ir_ref.clone().ok_or_else(|| {
                    ProductStoreError::InvalidRecord {
                        kind: "structural_checkpoint_restore",
                        reason: "checkpoint plan_candidate_ir_ref is missing".to_string(),
                    }
                })?;
            let structure_report_ref =
                checkpoint.structure_report_ref.clone().ok_or_else(|| {
                    ProductStoreError::InvalidRecord {
                        kind: "structural_checkpoint_restore",
                        reason: "checkpoint structure_report_ref is missing".to_string(),
                    }
                })?;
            let checkpoint_source_hash =
                checkpoint
                    .source_hash
                    .clone()
                    .ok_or_else(|| ProductStoreError::InvalidRecord {
                        kind: "structural_checkpoint_restore",
                        reason: "checkpoint source_hash is missing".to_string(),
                    })?;
            let scope = SourceStoreScope {
                project_id: expected.project_id.clone(),
                issue_id: expected.issue_id.clone(),
                plan_id: expected.entity_id.clone(),
            };
            for (reference, kind) in [
                (source_revision_ref.as_str(), "source_revision"),
                (plan_candidate_ir_ref.as_str(), "plan_candidate_ir"),
                (structure_report_ref.as_str(), "mechanical_report"),
            ] {
                validate_canonical_ref_for_scope(&scope, reference, kind).map_err(|error| {
                    ProductStoreError::InvalidRecord {
                        kind: "structural_checkpoint_restore",
                        reason: error.code().to_string(),
                    }
                })?;
            }
            // 当前上下文重验：以调用方按当前 Design/member/options/baseline
            // 派生的 SG1 报告对三 refs 同源复验（缺 payload/IR/report、hash
            // 或 context 漂移全部 fail-closed）。
            let (source_record, payload) = load_sealed_structure_generation(
                self.app_paths(),
                &scope,
                &source_revision_ref,
                &plan_candidate_ir_ref,
                &structure_report_ref,
                current_report,
            )?;
            if !current_report.structurally_valid {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "structural_checkpoint_restore",
                    reason: "current structure report is not structurally valid".to_string(),
                });
            }
            if checkpoint_source_hash != source_record.source_revision_hash
                || checkpoint.context_digest.as_deref() != Some(payload.context_digest.as_str())
            {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "structural_checkpoint_restore",
                    reason: "checkpoint hash/context digest does not match the durable generation"
                        .to_string(),
                });
            }
            let session_path = self.find_workspace_session_path(&expected.id)?;
            with_exclusive_lock(&session_path, || {
                let stored: WorkspaceSessionRecord = read_json(&session_path)?;
                if stored != *expected {
                    return Err(ProductStoreError::Conflict {
                        kind: "workspace_session",
                        id: expected.id.clone(),
                    });
                }
                // 新反馈未满足（活动 hold）禁止整组回退：本轮存在未交付业务
                // 请求，历史结构只能作 author 指令参考，不得回退假交付。
                if let Some(state) = stored.structural_governance_state.as_ref()
                    && let Some(hold) = state.user_holds.first()
                {
                    return Err(ProductStoreError::Conflict {
                        kind: "structural_checkpoint_restore",
                        id: hold.turn_id.clone(),
                    });
                }
                let mut next = stored.clone();
                // 纯状态回绑（0 author）：refs 整组回绑 + 旧审批代次同 CAS 失效。
                next.work_item_plan_source_revision_ref = Some(source_revision_ref.clone());
                next.plan_candidate_ir_ref = Some(plan_candidate_ir_ref.clone());
                next.mechanical_report_ref = Some(structure_report_ref.clone());
                next.approval_attempt_id = None;
                next.approved_at = None;
                next.compile_reservation = None;
                next.publication_provenance_ref = None;
                {
                    let state = next
                        .structural_governance_state
                        .get_or_insert_with(StructuralGovernanceState::default);
                    // review join：仅已绑定同 refs/context 的评审可保
                    // Reviewing；无法 join → Assessing 复评，不伪造已评审；
                    // 终态 grant 不因回绑重开（REQ-SG-04）。
                    let review_joins = state.current_grant.as_ref().is_some_and(|grant| {
                        grant.review_identity.is_some()
                            && grant.candidate_source_revision_ref.as_deref()
                                == Some(source_revision_ref.as_str())
                            && grant.plan_candidate_ir_ref.as_deref()
                                == Some(plan_candidate_ir_ref.as_str())
                            && grant.mechanical_report_ref.as_deref()
                                == Some(structure_report_ref.as_str())
                    });
                    if let Some(grant) = state.current_grant.as_mut() {
                        let terminal = matches!(
                            grant.phase,
                            Some(
                                StructuralGrantPhase::Resolved
                                    | StructuralGrantPhase::SystemFault
                                    | StructuralGrantPhase::BusinessGate
                            )
                        );
                        if !terminal {
                            grant.candidate_source_revision_ref = Some(source_revision_ref.clone());
                            grant.plan_candidate_ir_ref = Some(plan_candidate_ir_ref.clone());
                            grant.mechanical_report_ref = Some(structure_report_ref.clone());
                            grant.phase = Some(if review_joins {
                                StructuralGrantPhase::Reviewing
                            } else {
                                StructuralGrantPhase::Assessing
                            });
                        }
                    }
                    state.current_structure_report_ref = Some(structure_report_ref.clone());
                    state.current_context_digest = Some(payload.context_digest.clone());
                    // checkpoint 回 Active（同 id 幂等，旧 Active 转 Superseded）；
                    // 旧 receipt 不新计（user_receipts 不动）。
                    state.record_structure_checkpoint(checkpoint.clone());
                }
                next.updated_at = Utc::now().to_rfc3339();
                write_json(&session_path, &next)?;
                Ok(next)
            })
        })
    }
}

/// 判断两个 mechanical report canonical ref 是否指向等价内容：各按
/// 完整校验链加载 record，比对剔除 `id`/`content_hash` 后的全量内容
/// 摘要（`content_addressed_id`）。任一 ref 悬空/校验失败（无法证明
/// 等价）返回 false，由调用方 fail-closed。
fn equivalent_mechanical_reports(
    paths: crate::product::app_paths::ProductAppPaths,
    scope: &SourceStoreScope,
    existing_ref: &str,
    incoming_ref: &str,
) -> bool {
    let store = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(paths);
    let Ok(existing) = store.get_mechanical_report(scope, existing_ref) else {
        return false;
    };
    let Ok(incoming) = store.get_mechanical_report(scope, incoming_ref) else {
        return false;
    };
    matches!(
        (existing.content_addressed_id(), incoming.content_addressed_id()),
        (Ok(existing_id), Ok(incoming_id)) if existing_id == incoming_id
    )
}

/// SG4b（plan §0.6.2）：装载三 refs 并同源复验同代结构报告——commit 与
/// restore 只接受已含 `Some` 合法 payload 的同代 refs：store 完整校验链
/// （scope/身份/hash/编译器/digest 闭包）之上，以 SG3a.1 冻结单源对传入
/// SG1 报告重投影，与持久 payload 全等比较；任一漂移/缺证 fail-closed。
fn load_sealed_structure_generation(
    paths: crate::product::app_paths::ProductAppPaths,
    scope: &SourceStoreScope,
    source_revision_ref: &str,
    plan_candidate_ir_ref: &str,
    mechanical_report_ref: &str,
    structure_report: &PlanningStructureReport,
) -> Result<
    (
        crate::product::work_item_plan_source_store::SourceRevisionRecord,
        crate::product::planning_failure::PersistedPlanningStructureReport,
    ),
    ProductStoreError,
> {
    let store = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(paths);
    let seal_error = |error: crate::product::work_item_plan_source_store::SourceStoreError| {
        ProductStoreError::InvalidRecord {
            kind: "structural_generation",
            reason: error.code().to_string(),
        }
    };
    let source = store
        .get_source_revision(scope, source_revision_ref)
        .map_err(seal_error)?;
    let ir = store
        .get_plan_candidate_ir(scope, plan_candidate_ir_ref)
        .map_err(seal_error)?;
    let report = store
        .get_mechanical_report(scope, mechanical_report_ref)
        .map_err(seal_error)?;
    if ir.source_revision_id != source.id
        || report.source_revision_id != source.id
        || report.ir_id != ir.id
    {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "structural_generation",
            id: mechanical_report_ref.to_string(),
        });
    }
    let Some(payload) = report.structure_report else {
        return Err(ProductStoreError::InvalidRecord {
            kind: "structural_generation",
            reason: "mechanical report has no sealed structure payload (legacy generation)"
                .to_string(),
        });
    };
    let frozen = freeze_persisted_structure_report(structure_report, &source.id, &ir.id).map_err(
        |reason| ProductStoreError::InvalidRecord {
            kind: "structural_generation",
            reason: format!("re-freeze current structure report failed: {reason}"),
        },
    )?;
    if frozen != payload {
        return Err(ProductStoreError::InvalidRecord {
            kind: "structural_generation",
            reason: "structure report does not match the sealed same-generation payload"
                .to_string(),
        });
    }
    Ok((source, payload))
}

#[cfg(test)]
#[path = "workspace_single_candidate_tests.rs"]
mod tests;
