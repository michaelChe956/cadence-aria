use chrono::Utc;

use crate::product::models::{
    HumanGateTurn, HumanGateTurnFailureClass, HumanGateTurnStatus, WorkspaceType,
};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::product::work_item_plan_policy::{
    CandidateRecoveryAction, CandidateRecoveryCommandRecord, CandidateSnapshotRecovery,
};
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

/// C1 Task 4：候选门恢复命令（REST/WS 共用的应用服务输入）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateRecoveryCommand {
    pub command_id: String,
    pub expected_gate_id: String,
    pub action: CandidateRecoveryAction,
}

/// C1 Task 4：候选门恢复结果。`Replayed` 携带首次 durable 命令记录与
/// 当前评估事实；恢复动作本身不扣预算、不启 provider、不建第二候选权威。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CandidateRecoveryOutcome {
    Accepted {
        facts: CandidateSnapshotRecovery,
    },
    NeedsHuman {
        facts: CandidateSnapshotRecovery,
    },
    Replayed {
        record: CandidateRecoveryCommandRecord,
        facts: CandidateSnapshotRecovery,
    },
}

impl super::WorkspaceEngine {
    /// C1 Task 4：durable 候选快照完整性评估（只读；refs 在场且可从权威
    /// source store 读回、预算/门事实在场才 complete）。不可读事实计入
    /// `missing` 诊断（fail-closed），不臆测候选内容。
    pub(crate) fn assess_candidate_snapshot(&self) -> Result<CandidateSnapshotRecovery, String> {
        use crate::product::models::WorkspaceSessionRecord;
        use crate::product::work_item_plan_policy::CandidateSnapshotRecovery;

        let store = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let record: WorkspaceSessionRecord = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        let gate_id = self.active_timeline_node_id();
        let mut missing = Vec::new();
        let mut completed_steps = Vec::new();

        if record.human_gate_snapshot.is_none() {
            missing.push("human_gate_snapshot_missing".to_string());
        }
        if gate_id.is_none() {
            missing.push("gate_node_missing".to_string());
        } else {
            completed_steps.push("gate_open".to_string());
        }
        let budget_remaining = record
            .human_gate_snapshot
            .as_ref()
            .map(|snapshot| snapshot.manual_repairs_remaining);
        match budget_remaining {
            Some(_) => completed_steps.push("budget_recorded".to_string()),
            None => missing.push("budget_missing".to_string()),
        }

        let scope = SourceStoreScope {
            project_id: record.project_id.clone(),
            issue_id: record.issue_id.clone(),
            plan_id: record.entity_id.clone(),
        };
        let source_store = WorkItemPlanSourceStore::new(store.app_paths());
        let mut source_revision_hash = None;
        match record.work_item_plan_source_revision_ref.as_deref() {
            Some(source_ref) => match source_store.get_source_revision(&scope, source_ref) {
                Ok(source) => {
                    source_revision_hash = Some(source.source_revision_hash);
                    completed_steps.push("candidate_source_persisted".to_string());
                }
                Err(error) => missing.push(format!("source_revision_unreadable: {error:?}")),
            },
            None => missing.push("source_revision_missing".to_string()),
        }
        match record.plan_candidate_ir_ref.as_deref() {
            Some(ir_ref) => match source_store.get_plan_candidate_ir(&scope, ir_ref) {
                Ok(_) => completed_steps.push("candidate_ir_persisted".to_string()),
                Err(error) => missing.push(format!("plan_candidate_ir_unreadable: {error:?}")),
            },
            None => missing.push("plan_candidate_ir_missing".to_string()),
        }
        match record.mechanical_report_ref.as_deref() {
            Some(report_ref) => match source_store.get_mechanical_report(&scope, report_ref) {
                Ok(_) => completed_steps.push("mechanical_report_persisted".to_string()),
                Err(error) => missing.push(format!("mechanical_report_unreadable: {error:?}")),
            },
            None => missing.push("mechanical_report_missing".to_string()),
        }

        let complete = missing.is_empty();
        Ok(CandidateSnapshotRecovery {
            complete,
            gate_id: gate_id.unwrap_or_default(),
            source_revision_ref: record.work_item_plan_source_revision_ref.clone(),
            source_revision_hash,
            plan_candidate_ir_ref: record.plan_candidate_ir_ref.clone(),
            mechanical_report_ref: record.mechanical_report_ref.clone(),
            budget_remaining,
            missing,
            completed_steps,
            commands: record
                .human_gate_snapshot
                .and_then(|snapshot| snapshot.candidate_recovery)
                .map(|label| label.commands)
                .unwrap_or_default(),
            assessed_at: Utc::now().to_rfc3339(),
        })
    }

    /// C1 Task 4：该会话是否处于候选审批门形态（label 只对 SC 审批门落盘，
    /// legacy/非审批门/旧快照缺席保持既有语义零回归）。
    fn candidate_recovery_gate_applies(&self) -> bool {
        use crate::product::models::{SingleCandidatePhase, WorkspaceType};
        self.session.workspace_type == WorkspaceType::WorkItemPlan
            && self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate
            && self.session.single_candidate_phase == Some(SingleCandidatePhase::Approval)
            && self.session.human_gate_snapshot.is_some()
    }

    /// C1 Task 4（REQ-C1-GATE-01）：relay/observer 之前的候选快照完整性
    /// 落盘（生产门开启路径调用）。只写 additive label；CAS 失败不阻塞
    /// 门开启（评估可由恢复动作重导），仅发可见错误事件。
    pub(crate) async fn persist_candidate_snapshot_before_relay(&mut self) {
        if !self.candidate_recovery_gate_applies() {
            return;
        }
        let Ok(facts) = self.assess_candidate_snapshot() else {
            return;
        };
        let Some(store) = self.lifecycle_store.clone() else {
            return;
        };
        let Ok(expected) = store.get_workspace_session(&self.session.session_id) else {
            return;
        };
        match store.compare_and_save_candidate_recovery(&expected, facts) {
            Ok(saved) => {
                self.session.human_gate_snapshot = saved.human_gate_snapshot;
            }
            Err(error) => {
                let _ = self
                    .event_tx
                    .send(super::EngineEvent::Error {
                        message: format!(
                            "persist candidate snapshot label lost the durable race: {error}"
                        ),
                    })
                    .await;
            }
        }
    }

    /// C1 Task 4（REQ-C1-GATE-01/02）：候选门恢复应用服务（REST/WS 同一
    /// 入口）。恢复/重建只恢复原门事实或从权威 source/IR/report 重建呈现，
    /// 不扣预算、不启 provider、不建第二候选权威；同 command 同负载重放
    /// 首次 durable 结果，异 payload fail-closed；旧 gate id 零副作用拒绝。
    pub(crate) async fn recover_candidate_gate(
        &mut self,
        command: CandidateRecoveryCommand,
    ) -> Result<CandidateRecoveryOutcome, String> {
        use crate::product::models::{OperationState, WorkspaceSessionStatus, WorkspaceType};
        use crate::product::work_item_plan_policy::{
            CandidateRecoveryCommandRecord, CandidateSnapshotRecovery,
        };

        super::conversational_gate::validate_command_id(&command.command_id)?;
        let store = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        if expected.workspace_type != WorkspaceType::WorkItemPlan
            || expected.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || expected.status != WorkspaceSessionStatus::WaitingForHuman
            || expected.human_gate_snapshot.is_none()
        {
            return Err(
                "CANDIDATE_RECOVERY_GATE_CLOSED: candidate recovery requires an open single-candidate human gate"
                    .to_string(),
            );
        }
        // 门身份比对在评估/落盘之前：不匹配零副作用（不误答过期门）。
        let current_gate = self.active_timeline_node_id();
        if current_gate.as_deref() != Some(command.expected_gate_id.as_str()) {
            return Err(format!(
                "CANDIDATE_RECOVERY_GATE_MISMATCH: expected gate {} but current gate is {:?}",
                command.expected_gate_id, current_gate
            ));
        }

        // 幂等：同 command 同负载重放首次 durable 结果，异 payload fail-closed。
        let mut facts = self.assess_candidate_snapshot()?;
        if let Some(record) = facts
            .commands
            .iter()
            .find(|record| record.command_id == command.command_id)
            .cloned()
        {
            if record.action != command.action {
                return Err(format!(
                    "CANDIDATE_RECOVERY_COMMAND_CONFLICT: command_id {} already recorded with a different payload",
                    command.command_id
                ));
            }
            return Ok(CandidateRecoveryOutcome::Replayed { record, facts });
        }

        // 评估 + 动作语义：complete → Accepted；Rebuild 允许在权威 refs 可读
        // 时代重建内存呈现（durable 候选权威不动）；否则 NeedsHuman。
        let authoritative_refs_readable = facts.source_revision_ref.is_some()
            && facts.plan_candidate_ir_ref.is_some()
            && facts.mechanical_report_ref.is_some()
            && !facts.missing.iter().any(|item| {
                item.starts_with("source_revision_unreadable")
                    || item.starts_with("plan_candidate_ir_unreadable")
                    || item.starts_with("mechanical_report_unreadable")
            });
        let state = if facts.complete {
            OperationState::Accepted
        } else if command.action == CandidateRecoveryAction::Rebuild && authoritative_refs_readable
        {
            OperationState::Accepted
        } else {
            OperationState::NeedsHuman
        };
        if state == OperationState::Accepted
            && command.action == CandidateRecoveryAction::Rebuild
            && !facts.complete
        {
            // 从权威 source 重建内存呈现面（不写 durable artifact 版本，
            // 不新建候选权威——呈现面重开进程后可再次重建）。
            let scope = SourceStoreScope {
                project_id: expected.project_id.clone(),
                issue_id: expected.issue_id.clone(),
                plan_id: expected.entity_id.clone(),
            };
            let source_store = WorkItemPlanSourceStore::new(store.app_paths());
            if let Some(source_ref) = expected.work_item_plan_source_revision_ref.as_deref()
                && let Ok(source) = source_store.get_source_revision(&scope, source_ref)
            {
                self.session.artifact =
                    Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
                        markdown: source.source,
                        diff: None,
                    });
            }
        }

        let record = CandidateRecoveryCommandRecord {
            command_id: command.command_id.clone(),
            action: command.action,
            state,
            missing: facts.missing.clone(),
            recorded_at: Utc::now().to_rfc3339(),
        };
        facts.commands.push(record.clone());
        let saved = store
            .compare_and_save_candidate_recovery(&expected, facts.clone())
            .map_err(|error| error.to_string())?;
        self.session.human_gate_snapshot = saved.human_gate_snapshot.clone();
        let facts: CandidateSnapshotRecovery = saved
            .human_gate_snapshot
            .and_then(|snapshot| snapshot.candidate_recovery)
            .unwrap_or(facts);
        Ok(match state {
            OperationState::Accepted => CandidateRecoveryOutcome::Accepted { facts },
            _ => CandidateRecoveryOutcome::NeedsHuman { facts },
        })
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
