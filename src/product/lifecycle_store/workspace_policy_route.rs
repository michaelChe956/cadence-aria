//! policy route / policy failure 的 durable 持久化(从 `workspace.rs` 拆出,
//! 保持文件行数守卫)。两者共用同一 policy_diagnostics 落盘机制。

use chrono::Utc;

use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{WorkspaceSessionRecord, WorkspaceSessionStatus};

use super::LifecycleStore;

pub struct PolicyRoutePersist {
    pub status: WorkspaceSessionStatus,
    /// SingleCandidate 由同一 CAS 与策略结果一起推进；legacy 始终保持 `None`。
    pub single_candidate_phase: Option<crate::product::models::SingleCandidatePhase>,
    pub run_history: crate::product::work_item_plan_policy::RunHistory,
    pub scope: Option<crate::product::work_item_plan_policy::ReviewInvocationScope>,
    pub gate: Option<crate::product::work_item_plan_policy::HumanGateSnapshot>,
    pub diagnostics: Vec<crate::product::work_item_plan_policy::PolicyDiagnostic>,
    pub repair_reservation: Option<crate::product::work_item_plan_policy::RepairReservation>,
    pub provider_start_ledger: Vec<crate::product::work_item_plan_policy::ProviderStartLedgerEntry>,
}

impl LifecycleStore {
    /// Atomically projects a policy failure terminal state (status=Failed +
    /// diagnostics) into the durable session record. `finish_policy_failure`
    /// previously went through `update_workspace_session_status`, which persists
    /// only the status — fail-closed AbortFatal diagnostics stayed memory-only.
    /// This keeps the same durable diagnostics mechanism as
    /// `compare_and_save_policy_route` (I-1 round3 F-C).
    pub fn fail_workspace_session_policy(
        &self,
        session_id: &str,
        diagnostics: &[crate::product::work_item_plan_policy::PolicyDiagnostic],
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let locked_session_path = session_path.clone();
        with_exclusive_lock(&session_path, move || {
            let mut session: WorkspaceSessionRecord = read_json(&locked_session_path)?;
            session.status = WorkspaceSessionStatus::Failed;
            session.human_gate_snapshot = None;
            session.policy_diagnostics = diagnostics.to_vec();
            session.updated_at = Utc::now().to_rfc3339();
            write_json(&locked_session_path, &session)?;
            Ok(session)
        })
    }

    /// Atomically projects a policy route into the durable session record.
    /// The expected record check prevents stale websocket workers from
    /// overwriting a newer route; callers must reload and re-evaluate on clash.
    pub fn compare_and_save_policy_route(
        &self,
        expected: &WorkspaceSessionRecord,
        persist: PolicyRoutePersist,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let PolicyRoutePersist {
            status,
            single_candidate_phase,
            run_history,
            scope,
            gate,
            diagnostics,
            repair_reservation,
            provider_start_ledger,
        } = persist;
        validate_relative_id(&expected.id)?;
        let session_path = self.find_workspace_session_path(&expected.id)?;
        let locked_session_path = session_path.clone();
        with_exclusive_lock(&session_path, move || {
            let mut stored: WorkspaceSessionRecord = read_json(&locked_session_path)?;
            if stored != *expected {
                return Err(ProductStoreError::Conflict {
                    kind: "workspace_session",
                    id: expected.id.clone(),
                });
            }
            stored.status = status;
            if stored.flow_kind
                == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
            {
                stored.single_candidate_phase = single_candidate_phase;
            }
            stored.run_history = run_history;
            stored.review_invocation_scope = scope;
            stored.human_gate_snapshot = gate;
            stored.policy_diagnostics = diagnostics;
            stored.repair_reservation = repair_reservation;
            stored.provider_start_ledger = provider_start_ledger;
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(&locked_session_path, &stored)?;
            Ok(stored)
        })
    }
}
