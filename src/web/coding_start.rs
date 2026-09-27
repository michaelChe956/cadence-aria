//! P2 Task 3（tasks.md §3.1）：typed StartCoding 共用准入。
//!
//! 人工 WS 与自动编排共用同一首启临界区（D4 顺序：attempt reload → 状态
//! 矩阵/origin → enrolled 当前许可与精确 source/plan/revision/target →
//! `advance_is_ready_for_attempt`）。Task 3 交付只读准入：只有
//! Created+PrepareContext 可首启，Running/Completed 回 AlreadyStarted，
//! 等待/终态回 NeedsHuman（不重新认领）；SC 未 Ready 固定
//! `SC_CODING_REQUIRES_ADVANCE`，身份/策略/enrollment 漂移显式无启动
//! 错误。完整 claim+barrier 启动在 Task 4 接线；已获准的首启在此阶段
//! 返回 wiring 未接的诚实错误，不伪装 Started。

use crate::product::advance_store::{AdvanceStatus, AdvanceStore};
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAdmissionKind, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingStartOrigin, CodingStartRunPolicy,
};
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::json_store::validate_relative_id;
use crate::web::state::WebAppState;

/// 共用首启命令：`attempt_id` 定位 durable attempt；`command_id` 是首启
/// 幂等键（Task 4 durable claim 身份）；`origin` 决定授权链。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartCodingCommand {
    pub attempt_id: String,
    pub command_id: String,
    pub origin: CodingStartOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartCodingOutcome {
    /// 首启完成：runner 已过 barrier 放行（Task 4 起）。
    Started { attempt_id: String },
    /// 该 attempt 已有首启事实（claim/runner/终态完成），不重新认领。
    AlreadyStarted { attempt_id: String },
    /// 停等人：等待应答/终态失败/人工恢复/副作用不明分诊。
    NeedsHuman { attempt_id: String, reason: String },
}

/// 稳定错误面：`code()` 是调用方（WS 错码映射/编排器分诊）判断依据，
/// `message()` 携带真实原因；SC 未 Ready 恒 `SC_CODING_REQUIRES_ADVANCE`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartCodingError {
    code: String,
    message: String,
}

impl StartCodingError {
    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }

    fn requires_advance(message: impl Into<String>) -> Self {
        Self::new("SC_CODING_REQUIRES_ADVANCE", message)
    }
}

impl std::fmt::Display for StartCodingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for StartCodingError {}

/// 首启状态矩阵：只有 Created+PrepareContext 可首启。
enum FirstStartStatus {
    Proceed,
    AlreadyStarted,
    NeedsHuman(&'static str),
}

fn first_start_status(attempt: &CodingExecutionAttempt) -> Option<FirstStartStatus> {
    match attempt.status {
        CodingAttemptStatus::Created => {
            if attempt.stage == CodingExecutionStage::PrepareContext {
                Some(FirstStartStatus::Proceed)
            } else {
                None
            }
        }
        CodingAttemptStatus::Running
        | CodingAttemptStatus::AwaitingPlanAmendment
        | CodingAttemptStatus::ApplyingPlanAmendment
        | CodingAttemptStatus::AmendmentApplyFailed
        | CodingAttemptStatus::Completed => Some(FirstStartStatus::AlreadyStarted),
        CodingAttemptStatus::WaitingForHuman => Some(FirstStartStatus::NeedsHuman(
            "coding attempt is waiting for a human action",
        )),
        CodingAttemptStatus::Blocked => Some(FirstStartStatus::NeedsHuman(
            "coding attempt is blocked awaiting a human gate response",
        )),
        CodingAttemptStatus::AwaitingManualRecovery => Some(FirstStartStatus::NeedsHuman(
            "coding attempt is awaiting manual recovery; use explicit RecoverCoding",
        )),
        CodingAttemptStatus::Failed | CodingAttemptStatus::Aborted => {
            Some(FirstStartStatus::NeedsHuman(
                "coding attempt reached a terminal state; use explicit RestartCoding",
            ))
        }
    }
}

/// 共用 typed StartCoding 服务（Task 3 只读准入面）。
pub async fn start_coding_once(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
    command: StartCodingCommand,
) -> Result<StartCodingOutcome, StartCodingError> {
    validate_relative_id(&command.command_id)
        .map_err(|error| StartCodingError::new(
            "coding_start_invalid_command_id",
            format!("invalid start coding command id: {error}"),
        ))?;
    let paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let coding_store = CodingAttemptStore::new(paths.clone());
    let attempt = coding_store
        .get_attempt(project_id, issue_id, &command.attempt_id)
        .map_err(|error| StartCodingError::new(
            "coding_start_attempt_load_failed",
            format!("load coding attempt for start failed: {error}"),
        ))?;

    // 状态矩阵（Task 4 将在锁内重读后再执行一次）。
    match first_start_status(&attempt) {
        Some(FirstStartStatus::Proceed) => {}
        Some(FirstStartStatus::AlreadyStarted) => {
            return Ok(StartCodingOutcome::AlreadyStarted {
                attempt_id: attempt.id.clone(),
            });
        }
        Some(FirstStartStatus::NeedsHuman(reason)) => {
            return Ok(StartCodingOutcome::NeedsHuman {
                attempt_id: attempt.id.clone(),
                reason: reason.to_string(),
            });
        }
        None => {
            return Err(StartCodingError::new(
                "coding_start_not_first_start_state",
                format!(
                    "attempt {} is Created but at stage {:?}; first start expects PrepareContext",
                    attempt.id, attempt.stage
                ),
            ));
        }
    }

    // origin 准入（只读；Task 4 在 enrollment/attempt 锁内再执行一次）。
    admit_start_origin(state, &paths, &coding_store, &attempt, &command.origin)?;

    // Task 4 在此接上 durable claim + registry reservation + 启动 barrier
    // （本任务不留下生产可调用的裸 runner 替代入口）。
    Err(StartCodingError::new(
        "coding_start_wiring_pending",
        "start admission passed but the first-start runner wiring lands in Task 4",
    ))
}

fn admit_start_origin(
    state: &WebAppState,
    paths: &ProductAppPaths,
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    origin: &CodingStartOrigin,
) -> Result<(), StartCodingError> {
    match origin {
        CodingStartOrigin::Manual => {
            // 手工保留 SC Ready 门但不强制 enrollment；非 SC legacy 判据不变。
            sc_advance_ready_gate(state, paths, attempt)
        }
        CodingStartOrigin::Enrolled {
            enrollment_id,
            policy_revision,
        } => {
            admit_enrolled_start(
                state,
                paths,
                coding_store,
                attempt,
                enrollment_id,
                *policy_revision,
            )?;
            sc_advance_ready_gate(state, paths, attempt)
        }
    }
}

/// 自动首启的精确授权链：attempt 冻结 policy ↔ origin ↔ 当前 durable
/// enrollment 三方一致，group journal 与 AdvanceRecord 的 plan/revision/
/// attempt 身份互证，多 target fail-closed（REQ-WIGA-03/04、REQ-MTG-03）。
fn admit_enrolled_start(
    state: &WebAppState,
    paths: &ProductAppPaths,
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    enrollment_id: &str,
    policy_revision: u64,
) -> Result<(), StartCodingError> {
    if attempt.admission_kind != CodingAdmissionKind::ScAdvance {
        return Err(StartCodingError::new(
            "coding_start_origin_mismatch",
            format!(
                "enrolled start requires an sc_advance attempt, got {:?}",
                attempt.admission_kind
            ),
        ));
    }
    let CodingStartRunPolicy::AutoStartOnce {
        enrollment_id: frozen_enrollment_id,
        policy_revision: frozen_policy_revision,
        source_plan_revision,
    } = &attempt.start_run_policy
    else {
        return Err(StartCodingError::new(
            "coding_start_origin_mismatch",
            "enrolled start requires a frozen auto_start_once policy on the attempt",
        ));
    };
    if frozen_enrollment_id != enrollment_id || *frozen_policy_revision != policy_revision {
        return Err(StartCodingError::new(
            "coding_start_policy_mismatch",
            format!(
                "start origin (enrollment {enrollment_id} revision {policy_revision}) does not \
                 match the frozen policy (enrollment {frozen_enrollment_id} revision \
                 {frozen_policy_revision})"
            ),
        ));
    }
    let enrollment = IssueAutomationStore::new(paths.clone())
        .get(&attempt.project_id, &attempt.issue_id)
        .map_err(|error| StartCodingError::new(
            "coding_start_enrollment_unreadable",
            format!("load enrollment for start failed: {error}"),
        ))?
        .ok_or_else(|| StartCodingError::new(
            "coding_start_enrollment_missing",
            "automation enrollment vanished before the coding start",
        ))?;
    if !enrollment.enabled {
        return Err(StartCodingError::new(
            "coding_start_enrollment_disabled",
            "automation enrollment is disabled; the frozen auto-start grant is not consumable",
        ));
    }
    if enrollment.enrollment_id != enrollment_id {
        return Err(StartCodingError::new(
            "coding_start_enrollment_mismatch",
            format!(
                "enrollment id drift: expected {enrollment_id}, actual {}",
                enrollment.enrollment_id
            ),
        ));
    }
    if enrollment.policy_revision != policy_revision {
        return Err(StartCodingError::new(
            "coding_start_enrollment_mismatch",
            format!(
                "enrollment policy revision drift: expected {policy_revision}, actual {}",
                enrollment.policy_revision
            ),
        ));
    }
    let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
        return Err(StartCodingError::new(
            "coding_start_origin_mismatch",
            "enrolled start requires the attempt to be bound to a work item group plan",
        ));
    };
    if enrollment.plan_id.as_deref() != Some(plan_id) {
        return Err(StartCodingError::new(
            "coding_start_enrollment_mismatch",
            format!(
                "enrollment plan drift: attempt bound to plan {plan_id}, enrollment has {:?}",
                enrollment.plan_id
            ),
        ));
    }
    // target 快照与 enrollment 的 logical repository 精确一致。
    if let Some(snapshot) = &attempt.target_snapshot
        && snapshot.logical_repository_id != enrollment.logical_repository_id
    {
        return Err(StartCodingError::new(
            "coding_start_enrollment_mismatch",
            format!(
                "attempt target snapshot points at logical repository {:?} but enrollment \
                 authorizes {:?}",
                snapshot.logical_repository_id, enrollment.logical_repository_id
            ),
        ));
    }
    // group journal lineage：journal.attempt / plan_binding 与本 attempt 及
    // 冻结 plan revision 互证。
    let journal = coding_store
        .get_group_initialization(&attempt.project_id, &attempt.issue_id, plan_id)
        .map_err(|error| StartCodingError::new(
            "coding_start_plan_binding_unreadable",
            format!("load group initialization journal failed: {error}"),
        ))?;
    if journal.attempt.id != attempt.id
        || journal.plan_binding.attempt_id != attempt.id
        || journal.plan_binding.plan_id != plan_id
    {
        return Err(StartCodingError::new(
            "coding_start_plan_binding_mismatch",
            format!(
                "group journal lineage does not match attempt {} for plan {plan_id}",
                attempt.id
            ),
        ));
    }
    if journal.plan_binding.bound_plan_revision_id != *source_plan_revision {
        return Err(StartCodingError::new(
            "coding_start_plan_binding_mismatch",
            format!(
                "journal bound plan revision {} differs from frozen source plan revision \
                 {source_plan_revision}",
                journal.plan_binding.bound_plan_revision_id
            ),
        ));
    }
    // AdvanceRecord：Ready、唯一 attempt/target、revision 与冻结 policy 互证。
    let advance_store = AdvanceStore::new(paths.clone());
    let record = advance_store
        .get_advance_for_plan(&attempt.project_id, &attempt.issue_id, plan_id)
        .map_err(|error| StartCodingError::new(
            "coding_start_advance_record_unreadable",
            format!("load advance record failed: {error}"),
        ))?;
    let Some(record) = record else {
        return Err(StartCodingError::requires_advance(format!(
            "no durable advance record exists for plan {plan_id}"
        )));
    };
    if record.target_attempts.len() > 1 {
        return Err(StartCodingError::new(
            "coding_start_multi_target",
            format!(
                "advance record for plan {plan_id} carries {} target attempts; automatic \
                 start requires exactly one",
                record.target_attempts.len()
            ),
        ));
    }
    if !record
        .target_attempts
        .iter()
        .any(|binding| binding.attempt_id == attempt.id)
    {
        return Err(StartCodingError::requires_advance(format!(
            "advance record for plan {plan_id} does not bind attempt {}",
            attempt.id
        )));
    }
    if record.plan_revision_id != *source_plan_revision {
        return Err(StartCodingError::new(
            "coding_start_plan_binding_mismatch",
            format!(
                "advance record plan revision {} differs from frozen source plan revision \
                 {source_plan_revision}",
                record.plan_revision_id
            ),
        ));
    }
    if record.status != AdvanceStatus::Ready {
        return Err(StartCodingError::requires_advance(format!(
            "advance record for plan {plan_id} is {:?}, not Ready",
            record.status
 )));
    }
    Ok(())
}

/// SC Ready 门（StartCoding/RestartCoding 共用语义）：未绑定 group、记录
/// 缺失/未 Ready/attempt 不匹配、读取失败均映射 `SC_CODING_REQUIRES_ADVANCE`
/// 并携带真实原因；非 SC admission 恒放行（旧手工判据不变）。
fn sc_advance_ready_gate(
    _state: &WebAppState,
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<(), StartCodingError> {
    if attempt.admission_kind != CodingAdmissionKind::ScAdvance {
        return Ok(());
    }
    let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
        return Err(StartCodingError::requires_advance(
            "sc_advance attempt has no work item group plan binding",
        ));
    };
    match AdvanceStore::new(paths.clone()).advance_is_ready_for_attempt(
        &attempt.project_id,
        &attempt.issue_id,
        plan_id,
        &attempt.id,
    ) {
        Ok(true) => Ok(()),
        Ok(false) => Err(StartCodingError::requires_advance(format!(
            "advance record for plan {plan_id} is not durable-ready for attempt {}",
            attempt.id
        ))),
        Err(error) => Err(StartCodingError::requires_advance(format!(
            "cannot verify durable advance readiness: {error}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use crate::product::advance_store::AdvanceInput;
    use crate::product::coding_models::CodingStartOrigin;
    use crate::product::issue_automation_store::IssueAutomationStore;
    use crate::product::models::automation::EnrollmentWriteCommand;
    use crate::product::workspace_engine::{
        AdvanceInitializationFailpoint, AdvanceInitializationFailpointMode,
        register_advance_initialization_failpoint,
    };
    use crate::web::advance_plan::{AdvancePlanOrigin, advance_plan};
    use crate::web::coding_start::{StartCodingCommand, StartCodingOutcome, start_coding_once};
    use crate::web::state::CodingAttemptRunKey;
    use crate::web::wiga_gate_fixture::{
        EnrolledGateFixture, ISSUE_ID, PROJECT_ID, confirmed_enrolled_fixture,
    };

    /// Task 3 共享 fixture：Confirmed enrollment 经真实 advance 链取得
    /// 「有 journal lineage、尚未 Ready」的 attempt——先在 JournalPrepared
    /// 打断立起真实 lineage，重放在 UnitsMaterialized 打断（attempt 已随
    /// advance 真实落盘，record 仍 Initializing）。绝不注入无 lineage 的
    /// Created 壳。
    pub(crate) async fn enrolled_attempt_before_ready() -> EnrolledGateFixture {
        let fixture = confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let plan_id = enrollment.plan_id.clone().expect("bound plan");
        let input = AdvanceInput {
            command_id: format!("wiga-advance-{}-{plan_id}", enrollment.enrollment_id),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: plan_id.clone(),
        };
        let origin = AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
        };
        {
            let _failpoint = register_advance_initialization_failpoint(
                &input,
                AdvanceInitializationFailpoint::JournalPrepared,
                AdvanceInitializationFailpointMode::Crash,
            );
            let state = fixture.state.clone();
            let request = input.clone();
            let origin = origin.clone();
            let crashed = tokio::spawn(async move {
                advance_plan(&state, request, origin).await
            });
            assert!(
                crashed.await.is_err(),
                "JournalPrepared crash must interrupt the first advance"
            );
        }
        {
            let _failpoint = register_advance_initialization_failpoint(
                &input,
                AdvanceInitializationFailpoint::UnitsMaterialized,
                AdvanceInitializationFailpointMode::Crash,
            );
            let state = fixture.state.clone();
            let request = input.clone();
            let crashed = tokio::spawn(async move {
                advance_plan(&state, request, origin).await
            });
            assert!(
                crashed.await.is_err(),
                "UnitsMaterialized crash must interrupt the replay before Ready"
            );
        }
        fixture
    }

    fn enrolled_origin(
        fixture: &EnrolledGateFixture,
    ) -> CodingStartOrigin {
        let enrollment = fixture.enrollment();
        CodingStartOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
        }
    }

    #[tokio::test]
    async fn start_coding_rejects_sc_before_durable_ready() {
        let fixture = enrolled_attempt_before_ready().await;
        let attempt = fixture.attempt();
        let result = start_coding_once(
            &fixture.state,
            &attempt.project_id,
            &attempt.issue_id,
            StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: "auto-start-1".into(),
                origin: enrolled_origin(&fixture),
            },
        )
        .await;
        assert_eq!(result.unwrap_err().code(), "SC_CODING_REQUIRES_ADVANCE");
        assert_eq!(
            fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
            0
        );
    }

    /// 手工首启同样保留 SC Ready 门（不强制 enrollment）；非 SC legacy 的
    /// 旧手工判据不因此收窄。
    #[tokio::test]
    async fn manual_start_coding_keeps_sc_ready_gate() {
        let fixture = enrolled_attempt_before_ready().await;
        let attempt = fixture.attempt();
        let result = start_coding_once(
            &fixture.state,
            &attempt.project_id,
            &attempt.issue_id,
            StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: "manual-start-1".into(),
                origin: CodingStartOrigin::Manual,
            },
        )
        .await;
        assert_eq!(result.unwrap_err().code(), "SC_CODING_REQUIRES_ADVANCE");
        assert_eq!(
            fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
            0
        );
    }

    /// D2：禁用先胜——未消费的 AutoStartOnce 许可不生效，显式无启动错误。
    #[tokio::test]
    async fn enrolled_start_coding_fails_closed_when_enrollment_disabled() {
        let fixture = enrolled_attempt_before_ready().await;
        let attempt = fixture.attempt();
        let origin = enrolled_origin(&fixture);
        let store = IssueAutomationStore::new(fixture.inner.paths.clone());
        let before = fixture.enrollment();
        store
            .compare_and_set(
                PROJECT_ID,
                ISSUE_ID,
                Some(before.policy_revision),
                EnrollmentWriteCommand::Disable,
            )
            .expect("disable enrollment");
        let result = start_coding_once(
            &fixture.state,
            &attempt.project_id,
            &attempt.issue_id,
            StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: "auto-start-2".into(),
                origin,
            },
        )
        .await;
        assert_eq!(result.unwrap_err().code(), "coding_start_enrollment_disabled");
        assert_eq!(
            fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
            0
        );
    }

    /// D2：禁用→重开（revision 漂移）后，旧 AutoStartOnce 身份不得洗白。
    #[tokio::test]
    async fn enrolled_start_coding_rejects_stale_identity_after_reopen() {
        let fixture = enrolled_attempt_before_ready().await;
        let attempt = fixture.attempt();
        let stale_origin = enrolled_origin(&fixture);
        let store = IssueAutomationStore::new(fixture.inner.paths.clone());
        let before = fixture.enrollment();
        store
            .compare_and_set(
                PROJECT_ID,
                ISSUE_ID,
                Some(before.policy_revision),
                EnrollmentWriteCommand::Disable,
            )
            .expect("disable enrollment");
        store
            .compare_and_set(
                PROJECT_ID,
                ISSUE_ID,
                Some(before.policy_revision + 1),
                EnrollmentWriteCommand::Enable {
                    selection_key: before.selection_key.clone(),
                    source: before.source.clone(),
                    options: before.options.clone(),
                    logical_repository_id: before.logical_repository_id.clone(),
                },
            )
            .expect("re-enable enrollment");
        assert!(fixture.enrollment().enabled);
        let result = start_coding_once(
            &fixture.state,
            &attempt.project_id,
            &attempt.issue_id,
            StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: "auto-start-3".into(),
                origin: stale_origin,
            },
        )
        .await;
        let error = result.unwrap_err();
        assert_eq!(error.code(), "coding_start_enrollment_mismatch");
        assert!(error.message().contains("policy revision"), "{error:?}");
        assert_eq!(
            fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
            0
        );
    }

    /// 已在途/终态的 attempt 不重新认领：Running 回 AlreadyStarted，等待/
    /// 终态回 NeedsHuman，均零 runner。
    #[tokio::test]
    async fn started_or_terminal_attempts_are_not_reclaimed() {
        let fixture = enrolled_attempt_before_ready().await;
        let store =
            crate::product::coding_attempt_store::CodingAttemptStore::new(
                fixture.inner.paths.clone(),
            );
        let attempt = fixture.attempt();

        let mut running = attempt.clone();
        running.status = crate::product::coding_models::CodingAttemptStatus::Running;
        store.write_coding_attempt_for_test(&running).unwrap();
        let outcome = start_coding_once(
            &fixture.state,
            &attempt.project_id,
            &attempt.issue_id,
            StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: "manual-running".into(),
                origin: CodingStartOrigin::Manual,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            outcome,
            StartCodingOutcome::AlreadyStarted {
                attempt_id: attempt.id.clone()
            }
        );

        let mut waiting = attempt.clone();
        waiting.status = crate::product::coding_models::CodingAttemptStatus::WaitingForHuman;
        store.write_coding_attempt_for_test(&waiting).unwrap();
        let outcome = start_coding_once(
            &fixture.state,
            &attempt.project_id,
            &attempt.issue_id,
            StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: "manual-waiting".into(),
                origin: CodingStartOrigin::Manual,
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            outcome,
            StartCodingOutcome::NeedsHuman { ref attempt_id, .. }
                if *attempt_id == attempt.id
        ));
        assert_eq!(
            fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
            0
        );
    }
}
