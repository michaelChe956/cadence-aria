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
        validate_amendment_gate_record(expected, "human_gate_amendment_reopen")?;
        if expected.status != WorkspaceSessionStatus::Confirmed {
            return Err(ProductStoreError::Conflict {
                kind: "human_gate_amendment_reopen",
                id: expected.id.clone(),
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
            stored.status = WorkspaceSessionStatus::WaitingForHuman;
            stored.updated_at = chrono::Utc::now().to_rfc3339();
            write_json(&session_path, &stored)?;
            Ok(stored)
        })
    }

    /// Close a reopened amendment gate back to its post-approval terminal
    /// status after the amendment was applied (or cancelled). Only accepts the
    /// reopen signature (WaitingForHuman + phase Completed + retained gate
    /// snapshot); the initial-approval gate (phase Approval) never flows
    /// through here. Callers must ensure no non-terminal human gate turn is
    /// in flight before closing.
    pub fn compare_and_close_reopened_amendment_gate(
        &self,
        expected: &WorkspaceSessionRecord,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_amendment_gate_record(expected, "human_gate_amendment_close")?;
        if expected.status != WorkspaceSessionStatus::WaitingForHuman {
            return Err(ProductStoreError::Conflict {
                kind: "human_gate_amendment_close",
                id: expected.id.clone(),
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
) -> Result<(), ProductStoreError> {
    validate_relative_id(&record.id)?;
    if record.workspace_type != WorkspaceType::WorkItemPlan
        || record.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        || record.single_candidate_phase != Some(SingleCandidatePhase::Completed)
        || record.human_gate_snapshot.is_none()
    {
        return Err(ProductStoreError::Conflict {
            kind,
            id: record.id.clone(),
        });
    }
    Ok(())
}
