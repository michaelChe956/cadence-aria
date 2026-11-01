use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    SingleCandidatePhase, WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;

impl super::LifecycleStore {
    /// Reopen the single human gate on the ORIGINAL single-candidate plan
    /// session in amendment context (REQ-GCE-03 scenario 2, D11). Reachable
    /// only for a session that already passed the initial approval compile
    /// (Confirmed + phase Completed) while its gate snapshot — and therefore
    /// `manual_repairs_remaining` — is still retained. The reopen keeps the
    /// snapshot, phase and every other field byte-identical: the budget
    /// continues from the snapshot and no second gate instance is created.
    pub fn compare_and_reopen_amendment_gate(
        &self,
        expected: &WorkspaceSessionRecord,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_amendment_gate_record(
            expected,
            "human_gate_amendment_reopen",
            &[SingleCandidatePhase::Completed],
        )?;
        if expected.status != WorkspaceSessionStatus::Confirmed {
            return Err(ProductStoreError::Conflict {
                kind: "human_gate_amendment_reopen",
                id: expected.id.clone(),
            });
        }

        // D1.3b（fix-cascade-deletion plan §0.4）：最外层 scope 准入——expected
        // 记录域在 session flock 外先取 scope；锁内 reopen CAS 零变化。
        let scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            self.compare_and_reopen_amendment_gate_in_admission(expected)
        })
    }

    /// `compare_and_reopen_amendment_gate` 的已准入内层（D1.3b：外层已持
    /// scope 共享准入/已按删除事实拒；session flock 内原 CAS 语义不变）。
    fn compare_and_reopen_amendment_gate_in_admission(
        &self,
        expected: &WorkspaceSessionRecord,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let session_path = self.find_workspace_session_path(&expected.id)?;
        with_exclusive_lock(&session_path, || {
            let mut stored: WorkspaceSessionRecord = read_json(&session_path)?;
            if stored != *expected {
                return Err(ProductStoreError::Conflict {
                    kind: "workspace_session",
                    id: expected.id.clone(),
                });
            }
            stored.status = WorkspaceSessionStatus::WaitingForHuman;
            stored.updated_at = chrono::Utc::now().to_rfc3339();
            write_json(&session_path, &stored)?;
            Ok(stored)
        })
    }

    /// Close a reopened amendment gate back to its post-approval terminal
    /// status after the amendment was applied (or cancelled). Accepts the
    /// reopen signature (WaitingForHuman + phase Completed + retained gate
    /// snapshot) and, since SG6（REQ-CG-03:74 verdict 收口）, the closure-
    /// rebuilt Approval signature（WaitingForHuman + phase Approval + retained
    /// snapshot）——合法修订交付后 turn 由原 reviewer/synthetic Pass 收口，
    /// 收口 verdict 经 Evaluate 路由重建 Approval 门，重开门在 resume 时
    /// 以该签名等待关回。The initial-approval gate never flows through here:
    /// the sole production caller reaches close only from a lineage-verified,
    /// just-Applied `PlanAmendmentContext` resume bound to this exact plan
    /// session. Callers must ensure no non-terminal human gate turn is
    /// in flight before closing.
    pub fn compare_and_close_reopened_amendment_gate(
        &self,
        expected: &WorkspaceSessionRecord,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_amendment_gate_record(
            expected,
            "human_gate_amendment_close",
            &[
                SingleCandidatePhase::Completed,
                SingleCandidatePhase::Approval,
            ],
        )?;
        if expected.status != WorkspaceSessionStatus::WaitingForHuman {
            return Err(ProductStoreError::Conflict {
                kind: "human_gate_amendment_close",
                id: expected.id.clone(),
            });
        }

        // D1.3b（fix-cascade-deletion plan §0.4）：最外层 scope 准入——expected
        // 记录域在 session flock 外先取 scope；锁内 close CAS 零变化。
        let scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            self.compare_and_close_reopened_amendment_gate_in_admission(expected)
        })
    }

    /// `compare_and_close_reopened_amendment_gate` 的已准入内层（D1.3b：外层
    /// 已持 scope 共享准入/已按删除事实拒；session flock 内原 CAS 语义不变）。
    fn compare_and_close_reopened_amendment_gate_in_admission(
        &self,
        expected: &WorkspaceSessionRecord,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let session_path = self.find_workspace_session_path(&expected.id)?;
        with_exclusive_lock(&session_path, || {
            let mut stored: WorkspaceSessionRecord = read_json(&session_path)?;
            if stored != *expected {
                return Err(ProductStoreError::Conflict {
                    kind: "workspace_session",
                    id: expected.id.clone(),
                });
            }
            stored.status = WorkspaceSessionStatus::Confirmed;
            stored.updated_at = chrono::Utc::now().to_rfc3339();
            write_json(&session_path, &stored)?;
            Ok(stored)
        })
    }
}

fn validate_amendment_gate_record(
    record: &WorkspaceSessionRecord,
    kind: &'static str,
    accepted_phases: &[SingleCandidatePhase],
) -> Result<(), ProductStoreError> {
    validate_relative_id(&record.id)?;
    if record.workspace_type != WorkspaceType::WorkItemPlan
        || record.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        || !record
            .single_candidate_phase
            .as_ref()
            .is_some_and(|phase| accepted_phases.contains(phase))
        || record.human_gate_snapshot.is_none()
    {
        return Err(ProductStoreError::Conflict {
            kind,
            id: record.id.clone(),
        });
    }
    Ok(())
}
