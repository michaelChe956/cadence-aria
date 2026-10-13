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
    /// B2d（Task 5.6，设计 §5.3 SC 纪律）：turn 被 active LC recovery
    ///（sc_manual_revision 等待/fresh）阻塞——不自动 ResumeSameTurn、
    /// 不增 attempt、不 spawn；显式 fresh 授权后继续同 turn。已真实
    /// Completed 的结果仍走原 CompletedRevision 恢复核对（不当成新 fresh）。
    BlockedByLcRecovery {
        recovery_id: String,
    },
    /// SG6a.1 P2 修复轮（Main 审查裁定）：结构拒收已同 turn claim
    /// attempt2（活动 grant phase=Attempt2Claimed，冻结机器指令 prompt 身份
    /// 在案）但 L1 交付/终结尚未收口——recovery 车道不 ResumeSameTurn
    ///（空 prompt 重跑原反馈 prompt 会错消费唯一 attempt2，违背
    /// REQ-CG-03 机器指令驱动）、不 MarkFailed、不增 attempt、不 spawn；
    /// 终结收口（含 release/门交互恢复）归 SG6a.3。
    AwaitingStructuralClosure,
}

/// Classify a durable turn without allocating a new turn or changing the
/// session budget. A running provider is left alone; a dead provider resumes
/// the same logical turn until the fixed attempt limit is reached.
///
/// `structural_attempt_claimed`（SG6a.1 P2）：session 活动 grant 绑定本 turn
/// 且 phase=Attempt2Claimed（结构拒收已 claim attempt2）——由调用方从
/// durable 聚合状态读取，判定面保持纯函数。
pub(crate) fn recover_human_gate_turn(
    turn: &HumanGateTurn,
    provider_is_running: bool,
    structural_attempt_claimed: bool,
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
            // SG6a.1 P2（Main 审查裁定）：结构拒收已同 turn claim attempt2
            //（活动 grant phase=Attempt2Claimed）——attempt2 属于冻结机器
            // 指令 prompt，不属空 prompt 的原反馈重跑；recovery 车道不
            // resume/不 MarkFailed/不增 attempt，终结收口归 SG6a.3。
            if structural_attempt_claimed {
                return Ok(HumanGateRecoveryAction::AwaitingStructuralClosure);
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
    /// B2d（Task 5.6，设计 §5.3 SC 纪律）：被 active LC recovery（sc_manual_
    /// revision 等待/fresh）阻塞的 turn 只做「已真实 Completed 结果」的原
    /// 恢复核对，其余一律跳过（不 ResumeSameTurn/不增 attempt/不 spawn）；
    /// Resolved 后既有恢复语义回归。
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
        let blocked_sc_manual = blocked_sc_manual_turn(expected.lc_resume_recovery.as_ref());
        let mut actions = Vec::new();
        for turn in turns {
            if !matches!(
                turn.status,
                HumanGateTurnStatus::Reserved | HumanGateTurnStatus::Running
            ) {
                continue;
            }
            if let Some((blocked_turn_id, recovery_id)) = &blocked_sc_manual
                && blocked_turn_id == &turn.turn_id
            {
                // 已真实 Completed 结果仍原恢复核对（不当成新 fresh）。
                if let Some(action) =
                    complete_turn_from_durable_artifact(store, &mut expected, &turn)?
                {
                    actions.push((turn.turn_id, action));
                } else {
                    actions.push((
                        turn.turn_id,
                        HumanGateRecoveryAction::BlockedByLcRecovery {
                            recovery_id: recovery_id.clone(),
                        },
                    ));
                }
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
            if let Some(action) = complete_turn_from_durable_artifact(store, &mut expected, &turn)?
            {
                actions.push((turn.turn_id, action));
                continue;
            }
            // SG6a.1 P2：活动 grant 绑定本 turn 且 phase=Attempt2Claimed =
            // 结构拒收已 claim attempt2（冻结机器指令 prompt 身份在案）。
            let structural_attempt_claimed = expected
                .structural_governance_state
                .as_ref()
                .and_then(|state| state.current_grant.as_ref())
                .is_some_and(|grant| {
                    grant.turn_id.as_deref() == Some(turn.turn_id.as_str())
                        && grant.phase
                            == Some(
                                crate::product::models::workspace::StructuralGrantPhase::Attempt2Claimed,
                            )
                });
            let action =
                recover_human_gate_turn(&turn, provider_is_running, structural_attempt_claimed)?;
            match &action {
                HumanGateRecoveryAction::WaitForProvider
                | HumanGateRecoveryAction::CompletedRevision
                | HumanGateRecoveryAction::BlockedByLcRecovery { .. }
                | HumanGateRecoveryAction::AwaitingStructuralClosure => {}
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

/// B2d（Task 5.6，设计 §5.3 SC 纪律）：SC 人工修订 turn 的 native mismatch
/// 稳定等待判别码——与 gateway 冻结绑定家族 `RESUME_FINGERPRINT_UNAVAILABLE`
/// 同码（manual turn 的 mismatch 即旧 native 在而冻结绑定不可用/已吊销，
/// 与 sc_author 同族）。
pub(crate) const SC_MANUAL_NATIVE_MISMATCH_WAITING_REASON: &str =
    crate::product::logical_codebase::provider_gateway::RESUME_FINGERPRINT_UNAVAILABLE;

impl super::WorkspaceEngine {
    /// B2d（Task 5.6，设计 §5.3 sc_manual_revision 行）：SC 人工修订 turn
    /// 驱动前的 native mismatch precheck——`FrozenBindingUnavailable`（旧
    /// native 在而冻结绑定不可用/已吊销）返回 `Ok(Some(reason))`（caller
    /// 转 typed 等待：不 fail turn、不 rearm）；出生/健康 resume 返回
    /// `Ok(None)` 照常驱动（零变化）；其余读取错误原样上浮（fail-closed，
    /// 与 `build_work_item_plan_streaming_input` 内同调用的传播语义一致）。
    pub(crate) fn precheck_sc_manual_revision_resume(
        &self,
    ) -> Result<Option<&'static str>, String> {
        let provider = self.session.author_provider.clone();
        match self.provider_resume_session_candidate(
            crate::product::models::ProviderConversationRole::Author,
            &provider,
        ) {
            Ok(_) => Ok(None),
            Err(super::lifecycle::NativeResumeCandidateError::FrozenBindingUnavailable) => {
                Ok(Some(SC_MANUAL_NATIVE_MISMATCH_WAITING_REASON))
            }
            Err(error) => Err(format!(
                "sc manual revision resume precheck failed: {error}"
            )),
        }
    }

    /// B2d（Task 5.6，设计 §5.3 SC 纪律②，plan 接口字典）：SC manual turn
    /// native mismatch 的 durable 等待收口——手写 ScManualRevision tagged
    /// record（同 turn 的 command_id/source_hash/status/attempt_no/
    /// budget_reserved、关联 human_gate_reservation key、candidate/source
    /// revision ref、同 turn feedback 全量冻结）后 CAS Waiting，再清被取代
    /// 旧会话引用并幂等收口等待面（`reconcile_lc_resume_waiting`）。turn/
    /// gate 快照/预算/provider_start_ledger 不动——等待不等价 Failed 业务
    /// turn；显式 fresh 的物理 run/node/native 是新 native 证据，**不是**
    /// 新 turn/attempt/budget。failed_provider_node_id 只取旧会话
    /// conversation 的 last_node（绝不取活动门节点——门保持开态）。gate
    /// 快照内嵌于 session record（单一权威），无独立 ref——attempt/budget
    /// 已由 payload 逐字段冻结。assembly digest/ref 仅在 OrderedChunks
    /// 交付形态存在（inline 交付零持久化），等待记录不伪造。
    pub(crate) async fn persist_sc_manual_revision_superseded_wait(
        &mut self,
        turn_id: &str,
        reason: &str,
    ) -> Result<crate::product::models::LcResumeRecoveryRecord, String> {
        use crate::product::models::{
            LcRecoveryBusinessContext, LcRecoveryRunContext, LcResumeRecoveryRecord,
            LcResumeRecoveryState, ProviderConversationRole,
        };

        let store = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| {
                format!(
                    "load session for sc manual superseded wait failed ({}): {error}",
                    self.session.session_id
                )
            })?;
        let turn = store
            .get_human_gate_turn(&self.session.session_id, turn_id)
            .map_err(|error| {
                format!("load human gate turn for sc manual wait failed ({turn_id}): {error}")
            })?;
        if !matches!(
            turn.status,
            crate::product::models::HumanGateTurnStatus::Reserved
                | crate::product::models::HumanGateTurnStatus::Running
        ) {
            return Err(format!(
                "sc manual superseded wait requires an active turn, got {:?} ({turn_id})",
                turn.status
            ));
        }
        let role = ProviderConversationRole::Author;
        let provider = self.session.author_provider.clone();
        let conversation = self
            .session
            .provider_conversations
            .iter()
            .find(|conversation| conversation.role == role && conversation.provider == provider)
            .or_else(|| {
                expected.provider_conversations.iter().find(|conversation| {
                    conversation.role == role && conversation.provider == provider
                })
            });
        let superseded_native_session_id = conversation.and_then(|conversation| {
            let native_id = conversation.provider_session_id.trim();
            (!native_id.is_empty()).then(|| native_id.to_string())
        });
        let failed_provider_node_id = conversation
            .and_then(|conversation| conversation.last_node_id.clone())
            .unwrap_or_default();
        let human_gate_reservation_key = expected
            .human_gate_reservation
            .as_ref()
            .filter(|reservation| reservation.turn_id == turn.turn_id)
            .map(|reservation| reservation.provider_start_idempotency_key.clone());
        let run_context = LcRecoveryRunContext::ScManualRevision {
            business: LcRecoveryBusinessContext {
                entity_id: self.session.entity_id.clone(),
                plan_id: None,
                workspace_type: self.session.workspace_type.clone(),
                business_repository_physical_id: None,
                business_repository_logical_id: None,
                business_checkout_id: None,
                business_repository_path: self
                    .session
                    .repository_path
                    .clone()
                    .map(|path| path.to_string_lossy().to_string()),
                source_story_ids: Vec::new(),
                source_design_ids: Vec::new(),
                plan_options_ref: None,
                artifact_refs: Vec::new(),
                review_refs: Vec::new(),
                author_provider: provider.clone(),
                effective_reviewer_provider: self.session.reviewer_provider.clone(),
                original_node_id: (!failed_provider_node_id.is_empty())
                    .then(|| failed_provider_node_id.clone()),
                original_role_run_seq: None,
            },
            turn_id: turn.turn_id.clone(),
            command_id: turn.command_id.clone(),
            source_hash: turn.source_hash.clone(),
            turn_status: turn.status.clone(),
            attempt_no: turn.attempt_no,
            budget_reserved: turn.budget_reserved,
            human_gate_reservation_key,
            gate_snapshot_ref: None,
            source_revision_ref: expected.work_item_plan_source_revision_ref.clone(),
            feedback_text: Some(turn.feedback_text.clone()),
            assembly_digest: None,
            assembly_ref: None,
        };
        let now = chrono::Utc::now().to_rfc3339();
        let record = LcResumeRecoveryRecord {
            schema_version: 1,
            recovery_id: format!("lc_resume_recovery_{}", uuid::Uuid::new_v4().simple()),
            failed_provider_node_id,
            reason_code: reason.to_string(),
            role: role.clone(),
            provider: provider.clone(),
            superseded_native_session_id: superseded_native_session_id.clone(),
            previous_fingerprint_digest: self
                .logical_launch_fingerprints
                .get(&self.session.author_provider)
                .map(|fingerprint| fingerprint.digest.clone()),
            previous_session_status: expected.status.clone(),
            previous_stage: self.session.stage.as_str().to_string(),
            run_context,
            state: LcResumeRecoveryState::Waiting,
            operation_id: None,
            fresh_node_id: None,
            fresh_native_session_id: None,
            fresh_role_run_seq: None,
            fresh_operations: Vec::new(),
            detected_at: now.clone(),
            updated_at: now,
        };
        let saved = store
            .compare_and_save_lc_resume_state(&expected, record)
            .map_err(|error| {
                format!(
                    "persist sc manual superseded wait failed ({}): {error}",
                    self.session.session_id
                )
            })?;
        // 内存镜像同步（A2a 合同：durable 以 CAS 锁内 stored 为准）。
        self.session.session_status = saved.status.clone();
        self.session.lc_resume_recovery = saved.lc_resume_recovery.clone();
        // 被取代旧会话引用清理（显式 fresh 不再携带旧 resume id）+ 等待面
        // 幂等收口；turn/gate 快照/预算/ledger 不动。
        if let Some(native_id) = superseded_native_session_id.as_deref() {
            self.clear_superseded_provider_session(role, &provider, native_id)
                .await;
        }
        self.reconcile_lc_resume_waiting().await;
        saved.lc_resume_recovery.ok_or_else(|| {
            "persist sc manual superseded wait returned record without recovery".to_string()
        })
    }
}

/// B2d（Task 5.6）：active（非 Resolved）sc_manual_revision recovery 阻塞的
/// turn 身份——(turn_id, recovery_id)。其余 tag 不阻塞（本包只接管 manual
/// turn 的等待语义；整会话级自动链阻断由 manager `recover_on_creation`
/// 的既有 LC waiting 守卫承担）。
fn blocked_sc_manual_turn(
    recovery: Option<&crate::product::models::LcResumeRecoveryRecord>,
) -> Option<(String, String)> {
    let recovery = recovery?;
    if recovery.state == crate::product::models::LcResumeRecoveryState::Resolved {
        return None;
    }
    match &recovery.run_context {
        crate::product::models::LcRecoveryRunContext::ScManualRevision { turn_id, .. } => {
            Some((turn_id.clone(), recovery.recovery_id.clone()))
        }
        _ => None,
    }
}

/// 「已真实 Completed 结果」的原恢复核对（唯一写口）：Running turn 若已有
/// 晚于其创建的 current artifact 版本（candidate hash 对账通过），则按原
/// 语义落 Completed；无结果返回 None（caller 走各自分类/跳过路径）。
fn complete_turn_from_durable_artifact(
    store: &super::LifecycleStore,
    expected: &mut crate::product::models::WorkspaceSessionRecord,
    turn: &HumanGateTurn,
) -> Result<Option<HumanGateRecoveryAction>, String> {
    if turn.status != HumanGateTurnStatus::Running {
        return Ok(None);
    }
    let Some(result_artifact_ref) = durable_revision_artifact_ref(store, expected, turn)? else {
        return Ok(None);
    };
    let mut completed = turn.clone();
    completed.status = HumanGateTurnStatus::Completed;
    completed.result_artifact_ref = Some(result_artifact_ref);
    completed.failure_class = None;
    completed.updated_at = Utc::now().to_rfc3339();
    *expected = store
        .update_human_gate_turn(expected, completed)
        .map_err(|error| error.to_string())?;
    Ok(Some(HumanGateRecoveryAction::CompletedRevision))
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
        let state = if facts.complete
            || (command.action == CandidateRecoveryAction::Rebuild && authoritative_refs_readable)
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
