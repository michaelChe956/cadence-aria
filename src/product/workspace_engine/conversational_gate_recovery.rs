use chrono::Utc;

use crate::product::models::{
    HumanGateTurn, HumanGateTurnFailureClass, HumanGateTurnStatus, WorkspaceType,
};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::product::work_item_plan_source_store::{SourceStoreScope, WorkItemPlanSourceStore};

pub(crate) use super::conversational_gate::HUMAN_GATE_PROVIDER_MAX_ATTEMPTS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HumanGateRecoveryAction {
    WaitForProvider,
    ResumeSameTurn {
        next_attempt_no: u32,
    },
    CompletedRevision,
    MarkFailed {
        failure_class: HumanGateTurnFailureClass,
    },
}

/// Classify a durable turn without allocating a new turn or changing the
/// session budget. A running provider is left alone; a dead provider resumes
/// the same logical turn until the fixed attempt limit is reached.
pub(crate) fn recover_human_gate_turn(
    turn: &HumanGateTurn,
    provider_is_running: bool,
) -> Result<HumanGateRecoveryAction, String> {
    if turn.attempt_no == 0 {
        return Err("human gate turn attempt_no must start at 1".to_string());
    }
    if turn.budget_reserved != 1 {
        return Err("human gate turn budget_reserved must be exactly 1".to_string());
    }

    match turn.status {
        HumanGateTurnStatus::Reserved => {
            // A reserved record is durable proof that provider start has not
            // happened yet. Resume attempt 1 on reconnect; do not deadlock the
            // single-flight turn waiting for a process that already exited.
            if turn.attempt_no != 1 {
                return Err(format!(
                    "reserved human gate turn must have attempt_no 1, got {}",
                    turn.attempt_no
                ));
            }
            Ok(HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no: 1 })
        }
        HumanGateTurnStatus::Running => {
            if provider_is_running {
                return Ok(HumanGateRecoveryAction::WaitForProvider);
            }
            if turn.attempt_no < HUMAN_GATE_PROVIDER_MAX_ATTEMPTS {
                return Ok(HumanGateRecoveryAction::ResumeSameTurn {
                    next_attempt_no: turn.attempt_no + 1,
                });
            }
            Ok(HumanGateRecoveryAction::MarkFailed {
                failure_class: HumanGateTurnFailureClass::ProviderErr,
            })
        }
        HumanGateTurnStatus::Completed | HumanGateTurnStatus::Failed => Err(format!(
            "terminal human gate turn {} does not require recovery",
            turn.turn_id
        )),
    }
}

/// Verify that recovery only appends events. Existing durable event values are
/// compared byte-for-byte by callers through `PartialEq`; no event is removed
/// or rewritten as part of recovery.
#[cfg(test)]
pub(crate) fn assert_human_gate_event_prefix_immutable<T: PartialEq + std::fmt::Debug>(
    event_prefix: &[T],
    recovered_events: &[T],
) -> Result<(), String> {
    if recovered_events.len() < event_prefix.len() {
        return Err(format!(
            "human gate recovery removed {} durable events",
            event_prefix.len() - recovered_events.len()
        ));
    }
    if recovered_events[..event_prefix.len()] != *event_prefix {
        return Err("human gate recovery rewrote a durable event prefix".to_string());
    }
    Ok(())
}

pub(crate) fn provider_run_kind_for_human_gate(
    flow_kind: WorkItemPlanFlowKind,
    turn_id: &str,
) -> Result<super::ProviderRunKind, String> {
    if turn_id.trim().is_empty() {
        return Err("human gate turn_id must not be blank".to_string());
    }
    match flow_kind {
        WorkItemPlanFlowKind::SingleCandidate => {
            Ok(super::ProviderRunKind::HumanGateScManualRevision {
                turn_id: turn_id.to_string(),
                prompt: String::new(),
            })
        }
        WorkItemPlanFlowKind::Legacy => Err(
            "human gate provider runs are only supported for single-candidate work-item plans"
                .to_string(),
        ),
    }
}

pub(crate) fn revision_artifact_ref_from_versions(
    versions: &[crate::web::workspace_ws_types::ArtifactVersion],
) -> Result<String, String> {
    let current = versions
        .iter()
        .filter(|version| version.is_current)
        .collect::<Vec<_>>();
    if current.len() != 1 {
        return Err(format!(
            "human gate revision recovery requires exactly one current artifact version, found {}",
            current.len()
        ));
    }
    Ok(format!("artifact_version_{:03}", current[0].version))
}

fn current_revision_source_hash(
    store: &super::LifecycleStore,
    session: &crate::product::models::WorkspaceSessionRecord,
) -> Result<Option<String>, String> {
    let Some(source_ref) = session.work_item_plan_source_revision_ref.as_deref() else {
        return Ok(None);
    };
    let scope = SourceStoreScope {
        project_id: session.project_id.clone(),
        issue_id: session.issue_id.clone(),
        plan_id: session.entity_id.clone(),
    };
    let source_store = WorkItemPlanSourceStore::new(store.app_paths());
    source_store
        .get_source_revision(&scope, source_ref)
        .map(|source| Some(source.source_revision_hash))
        .map_err(|error| format!("{error:?}"))
}

fn durable_revision_artifact_ref(
    store: &super::LifecycleStore,
    session: &crate::product::models::WorkspaceSessionRecord,
    turn: &HumanGateTurn,
) -> Result<Option<String>, String> {
    if session.workspace_type != WorkspaceType::WorkItemPlan
        || session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        || turn.result_artifact_ref.is_some()
    {
        return Ok(None);
    }
    let (Some(source_ref), Some(ir_ref), Some(report_ref)) = (
        session.work_item_plan_source_revision_ref.as_deref(),
        session.plan_candidate_ir_ref.as_deref(),
        session.mechanical_report_ref.as_deref(),
    ) else {
        return Ok(None);
    };
    // A turn reserved for a previous candidate must never be completed from a
    // later round's refs. Empty hashes identify pre-source-hash records and are
    // deliberately fail-closed rather than guessing their candidate.
    if turn.source_hash.is_empty() {
        return Ok(None);
    }
    let scope = SourceStoreScope {
        project_id: session.project_id.clone(),
        issue_id: session.issue_id.clone(),
        plan_id: session.entity_id.clone(),
    };
    let source_store = WorkItemPlanSourceStore::new(store.app_paths());
    let source = source_store
        .get_source_revision(&scope, source_ref)
        .map_err(|error| format!("{error:?}"))?;
    if source.source_revision_hash != turn.source_hash {
        return Ok(None);
    }
    source_store
        .get_plan_candidate_ir(&scope, ir_ref)
        .map_err(|error| format!("{error:?}"))?;
    source_store
        .get_mechanical_report(&scope, report_ref)
        .map_err(|error| format!("{error:?}"))?;
    let versions = store
        .list_artifact_versions(&session.id)
        .map_err(|error| error.to_string())?;
    let artifact_ref = revision_artifact_ref_from_versions(&versions)?;
    let current = versions
        .iter()
        .find(|version| version.is_current)
        .expect("revision_artifact_ref_from_versions checked exactly one current version");
    if current.created_at <= turn.created_at {
        return Ok(None);
    }
    Ok(Some(artifact_ref))
}
impl super::WorkspaceEngine {
    /// Reconcile all durable non-terminal turns after a websocket/process restart.
    /// The provider marker is deliberately supplied by the runtime; durable turn
    /// state alone determines whether to wait, resume the same turn, or fail it.
    pub(crate) fn recover_human_gate_turns(
        &mut self,
        provider_is_running: bool,
    ) -> Result<Vec<(String, HumanGateRecoveryAction)>, String> {
        let Some(store) = self.lifecycle_store.as_ref() else {
            return Ok(Vec::new());
        };
        let mut expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        let turns = store
            .list_human_gate_turns(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        let mut actions = Vec::new();
        for turn in turns {
            if !matches!(
                turn.status,
                HumanGateTurnStatus::Reserved | HumanGateTurnStatus::Running
            ) {
                continue;
            }
            if let Some(current_source_hash) = current_revision_source_hash(store, &expected)?
                && !turn.source_hash.is_empty()
                && current_source_hash != turn.source_hash
            {
                let mut failed = turn.clone();
                failed.status = HumanGateTurnStatus::Failed;
                failed.failure_class = Some(HumanGateTurnFailureClass::ValidationReject);
                failed.updated_at = Utc::now().to_rfc3339();
                expected = store
                    .update_human_gate_turn(&expected, failed)
                    .map_err(|error| error.to_string())?;
                actions.push((
                    turn.turn_id,
                    HumanGateRecoveryAction::MarkFailed {
                        failure_class: HumanGateTurnFailureClass::ValidationReject,
                    },
                ));
                continue;
            }
            if let Some(result_artifact_ref) = (turn.status == HumanGateTurnStatus::Running)
                .then(|| durable_revision_artifact_ref(store, &expected, &turn))
                .transpose()?
                .flatten()
            {
                let mut completed = turn.clone();
                completed.status = HumanGateTurnStatus::Completed;
                completed.result_artifact_ref = Some(result_artifact_ref);
                completed.failure_class = None;
                completed.updated_at = Utc::now().to_rfc3339();
                expected = store
                    .update_human_gate_turn(&expected, completed)
                    .map_err(|error| error.to_string())?;
                actions.push((turn.turn_id, HumanGateRecoveryAction::CompletedRevision));
                continue;
            }
            let action = recover_human_gate_turn(&turn, provider_is_running)?;
            match &action {
                HumanGateRecoveryAction::WaitForProvider
                | HumanGateRecoveryAction::CompletedRevision => {}
                HumanGateRecoveryAction::ResumeSameTurn { next_attempt_no } => {
                    let mut resumed = turn.clone();
                    resumed.status = HumanGateTurnStatus::Running;
                    resumed.attempt_no = *next_attempt_no;
                    resumed.updated_at = Utc::now().to_rfc3339();
                    expected = store
                        .update_human_gate_turn(&expected, resumed)
                        .map_err(|error| error.to_string())?;
                }
                HumanGateRecoveryAction::MarkFailed { failure_class } => {
                    let mut failed = turn.clone();
                    failed.status = HumanGateTurnStatus::Failed;
                    failed.failure_class = Some(failure_class.clone());
                    failed.updated_at = Utc::now().to_rfc3339();
                    expected = store
                        .update_human_gate_turn(&expected, failed)
                        .map_err(|error| error.to_string())?;
                }
            }
            actions.push((turn.turn_id, action));
        }
        self.session.provider_start_ledger = expected.provider_start_ledger;
        self.session.human_gate_snapshot = expected.human_gate_snapshot;
        Ok(actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_gate_event_prefix_helper_rejects_rewrite_and_truncation() {
        let prefix = ["open", "completed"];
        assert!(
            assert_human_gate_event_prefix_immutable(&prefix, &["open", "completed", "failed"])
                .is_ok()
        );
        assert!(assert_human_gate_event_prefix_immutable(&prefix, &["open"]).is_err());
        assert!(assert_human_gate_event_prefix_immutable(&prefix, &["open", "failed"]).is_err());
    }
}
