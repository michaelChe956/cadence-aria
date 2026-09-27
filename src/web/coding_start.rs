//! P2 Task 3/4（tasks.md §3.1）：typed StartCoding 共用首启服务。
//!
//! 人工 WS 与自动编排共用同一首启临界区，严格 D4 顺序：attempt reload →
//! 首启状态矩阵/origin → enrolled 当前许可与精确 source/plan/revision/
//! target（enrollment 文件锁内复核并消费 durable 单发 claim，锁序
//! enrollment lock → attempt lock，锁内仅同步操作）→
//! `advance_is_ready_for_attempt`（SC 未 Ready 固定
//! `SC_CODING_REQUIRES_ADVANCE`）→ claim + `try_reserve_attempt` → 持久
//! barrier（RunnerRegistered → ProviderMayHaveStarted → 放行）→ 既有
//! runner。claim 是 attempt 同文件不可复位身份：同 command 幂等续启、异
//! command AlreadyStarted、`ProviderMayHaveStarted` 后副作用不可证明转
//! 人工分诊；registry reservation 丢失不回滚已消费许可。

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
/// 幂等键（durable claim 身份）；`origin` 决定授权链。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartCodingCommand {
    pub attempt_id: String,
    pub command_id: String,
    pub origin: CodingStartOrigin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartCodingOutcome {
    /// 首启完成：runner 已过 barrier 放行。
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

/// 状态矩阵到 outcome 的直接映射（无启动动作）：`Ok(None)` 表示可继续
/// 首启；Created 但非首启相位是显式错误（fail-closed，不吞为 outcome）。
fn first_start_short_circuit(
    attempt: &CodingExecutionAttempt,
) -> Result<Option<StartCodingOutcome>, StartCodingError> {
    match first_start_status(attempt) {
        Some(FirstStartStatus::Proceed) => Ok(None),
        Some(FirstStartStatus::AlreadyStarted) => Ok(Some(StartCodingOutcome::AlreadyStarted {
            attempt_id: attempt.id.clone(),
        })),
        Some(FirstStartStatus::NeedsHuman(reason)) => Ok(Some(StartCodingOutcome::NeedsHuman {
            attempt_id: attempt.id.clone(),
            reason: reason.to_string(),
        })),
        None => Err(StartCodingError::new(
            "coding_start_not_first_start_state",
            format!(
                "attempt {} is Created but at stage {:?}; first start expects PrepareContext",
                attempt.id, attempt.stage
            ),
        )),
    }
}

/// 共用 typed StartCoding 服务（人工 WS 与自动编排同一入口）。
pub async fn start_coding_once(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
    command: StartCodingCommand,
) -> Result<StartCodingOutcome, StartCodingError> {
    start_coding_attempt(state, project_id, issue_id, command, None).await
}

/// 测试注入口：与生产同一流程，仅在 runner 放行处挂 provider 入口 probe
/// （控制中窗，不冒充启动）。
#[cfg(test)]
pub(crate) async fn start_coding_once_with_probe(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
    command: StartCodingCommand,
    probe: crate::web::coding_ws_handler::CodingRunnerStartProbe,
) -> Result<StartCodingOutcome, StartCodingError> {
    start_coding_attempt(state, project_id, issue_id, command, Some(probe)).await
}

async fn start_coding_attempt(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
    command: StartCodingCommand,
    probe: Option<crate::web::coding_ws_handler::CodingRunnerStartProbe>,
) -> Result<StartCodingOutcome, StartCodingError> {
    use crate::product::coding_attempt_store::ClaimCodingStartOutcome;
    use crate::product::coding_models::CodingStartPhase;

    validate_relative_id(&command.command_id).map_err(|error| StartCodingError::new(
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

    // D4 ①（锁外快路径）：首启状态矩阵。
    if let Some(outcome) = first_start_short_circuit(&attempt)? {
        return Ok(outcome);
    }

    // D4 ①：attempt 临界区（进程内串行；跨进程由 durable claim 收口）。
    let attempt_key = crate::web::state::CodingAttemptRunKey::from_attempt(&attempt);
    let _attempt_guard = state.coding_runs.lock_attempt(&attempt_key).await;
    let attempt = coding_store
        .get_attempt(project_id, issue_id, &command.attempt_id)
        .map_err(|error| StartCodingError::new(
            "coding_start_attempt_load_failed",
            format!("reload coding attempt for start failed: {error}"),
        ))?;
    if let Some(outcome) = first_start_short_circuit(&attempt)? {
        return Ok(outcome);
    }

    // D4 ②③④：origin 准入与 durable 单发 claim——enrolled 路径在 enrollment
    // 文件锁内复核当前许可并消费 claim（disable 竞争的单一_linear化点；
    // 锁序 enrollment lock → attempt lock，锁内仅同步操作）。
    let claim: ClaimCodingStartOutcome = match &command.origin {
        CodingStartOrigin::Manual => {
            // 手工保留 SC Ready 门但不强制 enrollment；非 SC legacy 判据不变。
            sc_advance_ready_gate(&paths, &attempt)?;
            coding_store
                .claim_coding_start(&attempt, &command.command_id, &command.origin)
                .map_err(|error| StartCodingError::new(
                    "coding_start_claim_failed",
                    format!("claim coding start failed: {error}"),
                ))?
        }
        CodingStartOrigin::Enrolled {
            enrollment_id,
            policy_revision,
        } => {
            let automation = IssueAutomationStore::new(paths.clone());
            let locked = automation
                .with_current_enrollment_locked(
                    &attempt.project_id,
                    &attempt.issue_id,
                    |enrollment| {
                        let admission =
                            verify_frozen_policy(&attempt, enrollment_id, *policy_revision)
                                .and_then(|plan_id| {
                                    verify_current_enrollment(
                                        enrollment,
                                        &attempt,
                                        enrollment_id,
                                        *policy_revision,
                                        &plan_id,
                                    )
                                })
                                .and_then(|()| {
                                    verify_journal_and_record(&paths, &coding_store, &attempt)
                                })
                                .and_then(|()| sc_advance_ready_gate(&paths, &attempt));
                        let admission = admission.and_then(|()| {
                            coding_store
                                .claim_coding_start(
                                    &attempt,
                                    &command.command_id,
                                    &command.origin,
                                )
                                .map_err(|error| StartCodingError::new(
                                    "coding_start_claim_failed",
                                    format!("claim coding start failed: {error}"),
                                ))
                        });
                        Ok(admission)
                    },
                )
                .map_err(|error| match error {
                    crate::product::json_store::ProductStoreError::NotFound { .. } => {
                        StartCodingError::new(
                            "coding_start_enrollment_missing",
                            "automation enrollment vanished before the coding start",
                        )
                    }
                    other => StartCodingError::new(
                        "coding_start_enrollment_unreadable",
                        format!("load enrollment for start failed: {other}"),
                    ),
                })?;
            locked?
        }
    };

    // claim 分诊：同 command 幂等续启；异 command 只见已消费事实；
    // ProviderMayHaveStarted 后不可证明零外部副作用 → 人工分诊，不复位。
    let claimed_attempt = match claim {
        ClaimCodingStartOutcome::Claimed(saved) => saved,
        ClaimCodingStartOutcome::Existing(saved) => {
            let existing = saved
                .start_claim
                .clone()
                .expect("existing coding start claim must be present");
            if existing.command_id != command.command_id || existing.origin != command.origin {
                return Ok(StartCodingOutcome::AlreadyStarted { attempt_id: saved.id });
            }
            match existing.phase {
                CodingStartPhase::Claimed | CodingStartPhase::RunnerRegistered => saved,
                CodingStartPhase::ProviderMayHaveStarted => {
                    // 可信在途事实：attempt 已 Running、本进程仍有 runner/预约，
                    // 或 role run ledger 已有 provider 启动证据——交由在途 run /
                    // 恢复协议，绝不二次首启。
                    let ledger_started = coding_store
                        .list_role_runs(
                            &saved.project_id,
                            &saved.issue_id,
                            &saved.id,
                        )
                        .map(|runs| !runs.is_empty())
                        .unwrap_or(false);
                    if saved.status == CodingAttemptStatus::Running
                        || ledger_started
                        || state.coding_runs.attempt_is_reserved_or_running(&attempt_key)
                    {
                        return Ok(StartCodingOutcome::AlreadyStarted { attempt_id: saved.id });
                    }
                    let marked = coding_store
                        .advance_coding_start_phase(
                            &saved,
                            &command.command_id,
                            CodingStartPhase::NeedsHuman,
                        )
                        .map_err(|error| StartCodingError::new(
                            "coding_start_claim_phase_failed",
                            format!("mark manual triage failed: {error}"),
                        ))?;
                    return Ok(StartCodingOutcome::NeedsHuman {
                        attempt_id: marked.id,
                        reason: "first start crossed the provider barrier with unproven \
                                 external side effects; manual triage required"
                            .to_string(),
                    });
                }
                CodingStartPhase::NeedsHuman => {
                    return Ok(StartCodingOutcome::NeedsHuman {
                        attempt_id: saved.id,
                        reason:
                            "first start requires manual triage (durable needs-human claim phase)"
                                .to_string(),
                    });
                }
            }
        }
    };

    // D4 ⑤：内存 registry reservation（拿不到即本进程已有 runner/预约——
    // 不消费新许可地回 AlreadyStarted；reservation 丢失不回滚已消费 claim）。
    let Some(reservation) = state.coding_runs.try_reserve_attempt(&attempt_key) else {
        return Ok(StartCodingOutcome::AlreadyStarted {
            attempt_id: claimed_attempt.id,
        });
    };

    // D4 ⑥：持久 barrier（RunnerRegistered → ProviderMayHaveStarted → 放行）
    // 后交既有 runner。
    let event_tx = state.coding_sockets.hub_sender(&attempt_key);
    let spawned = match probe {
        #[cfg(test)]
        Some(probe) => crate::web::coding_ws_handler::spawn_coding_runner_first_start_reserved_with_probe(
            state.clone(),
            coding_store.clone(),
            event_tx,
            claimed_attempt.clone(),
            reservation,
            &command.command_id,
            probe,
        ),
        #[cfg(test)]
        None => crate::web::coding_ws_handler::spawn_coding_runner_first_start_reserved(
            state.clone(),
            coding_store.clone(),
            event_tx,
            claimed_attempt.clone(),
            reservation,
            &command.command_id,
        ),
        #[cfg(not(test))]
        _ => crate::web::coding_ws_handler::spawn_coding_runner_first_start_reserved(
            state.clone(),
            coding_store.clone(),
            event_tx,
            claimed_attempt.clone(),
            reservation,
            &command.command_id,
        ),
    };
    match spawned {
        Ok(_) => Ok(StartCodingOutcome::Started {
            attempt_id: claimed_attempt.id,
        }),
        Err(error) => Err(StartCodingError::new(
            "coding_start_runner_activation_failed",
            format!("first-start runner activation failed: {error}"),
        )),
    }
}

/// attempt 冻结 policy 与 origin 身份互证（不读 enrollment）。
fn verify_frozen_policy(
    attempt: &CodingExecutionAttempt,
    enrollment_id: &str,
    policy_revision: u64,
) -> Result<String, StartCodingError> {
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
        source_plan_revision: _,
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
    let Some(plan_id) = attempt.work_item_group_id.clone() else {
        return Err(StartCodingError::new(
            "coding_start_origin_mismatch",
            "enrolled start requires the attempt to be bound to a work item group plan",
        ));
    };
    Ok(plan_id)
}

/// 当前 durable enrollment 与 origin/attempt 的精确互证（enrolled 锁内
/// 调用；disable 后未消费的 AutoStartOnce 不再生效）。
fn verify_current_enrollment(
    enrollment: &crate::product::models::automation::IssueAutomationEnrollment,
    attempt: &CodingExecutionAttempt,
    enrollment_id: &str,
    policy_revision: u64,
    plan_id: &str,
) -> Result<(), StartCodingError> {
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
    if enrollment.plan_id.as_deref() != Some(plan_id) {
        return Err(StartCodingError::new(
            "coding_start_enrollment_mismatch",
            format!(
                "enrollment plan drift: attempt bound to plan {plan_id}, enrollment has {:?}",
                enrollment.plan_id
            ),
        ));
    }
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
    Ok(())
}

/// group journal lineage 与 AdvanceRecord 的 plan/revision/attempt 身份
/// 互证（多 target fail-closed，REQ-WIGA-03/04、REQ-MTG-03）。
fn verify_journal_and_record(
    paths: &ProductAppPaths,
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<(), StartCodingError> {
    let CodingStartRunPolicy::AutoStartOnce {
        source_plan_revision, ..
    } = &attempt.start_run_policy
    else {
        return Err(StartCodingError::new(
            "coding_start_origin_mismatch",
            "enrolled start requires a frozen auto_start_once policy on the attempt",
        ));
    };
    let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
        return Err(StartCodingError::new(
            "coding_start_origin_mismatch",
            "enrolled start requires the attempt to be bound to a work item group plan",
        ));
    };
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
    if record.attempt_id.as_deref() != Some(attempt.id.as_str())
        && !record
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


    fn paused_start_probe() -> (
        crate::web::coding_ws_handler::CodingRunnerStartProbe,
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let (entry_tx, entry_rx) = tokio::sync::oneshot::channel();
        let (continue_tx, continue_rx) = tokio::sync::oneshot::channel();
        (
            crate::web::coding_ws_handler::CodingRunnerStartProbe {
                events: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                provider_entry_tx: entry_tx,
                continue_rx,
            },
            entry_rx,
            continue_tx,
        )
    }

    /// Task 4 主证据：两个「进程」（独立 registry）手工/自动并发首启同一
    /// attempt——durable claim 单发，恰一 Started，输家 AlreadyStarted，
/// 输家 registry 零 runner；provider 真实入口被 probe 暂停（不冒充启动）。
    #[tokio::test]
    async fn manual_and_auto_claim_same_attempt_once_across_registries() {
        let fixture = crate::web::wiga_gate_fixture::ready_enrolled_attempt_fixture().await;
        let attempt = fixture.attempt();
        let state_a = fixture.state.clone();
        let state_b = fixture.restart_state();
        let (manual_probe, _manual_entry, _manual_hold) = paused_start_probe();
        let (auto_probe, _auto_entry, _auto_hold) = paused_start_probe();
        let manual = crate::web::coding_start::start_coding_once_with_probe(
            &state_a,
            &attempt.project_id,
            &attempt.issue_id,
            StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: "manual-1".into(),
                origin: CodingStartOrigin::Manual,
            },
            manual_probe,
        );
        let auto = crate::web::coding_start::start_coding_once_with_probe(
            &state_b,
            &attempt.project_id,
            &attempt.issue_id,
            StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: "auto-1".into(),
                origin: fixture.auto_origin(),
            },
            auto_probe,
        );
        let (manual, auto) = tokio::join!(manual, auto);
        let started =
            usize::from(matches!(manual, Ok(StartCodingOutcome::Started { .. })))
                + usize::from(matches!(auto, Ok(StartCodingOutcome::Started { .. })));
        assert_eq!(
            started, 1,
            "exactly one first start across registries: manual={manual:?} auto={auto:?}"
        );
        let saved = fixture
            .store()
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap();
        let claim = saved.start_claim.as_ref().expect("durable single-flight claim");
        let (winner_state, loser_state, winner_command, loser) =
            if matches!(manual, Ok(StartCodingOutcome::Started { .. })) {
                (&state_a, &state_b, "manual-1", &auto)
            } else {
                (&state_b, &state_a, "auto-1", &manual)
            };
        assert_eq!(claim.command_id, winner_command);
        assert!(
            matches!(loser, Ok(StartCodingOutcome::AlreadyStarted { .. })),
            "loser must observe the durable claim: {loser:?}"
        );
        let key = CodingAttemptRunKey::from_attempt(&attempt);
        assert_eq!(winner_state.coding_runs.runner_count(&key), 1);
        assert_eq!(loser_state.coding_runs.runner_count(&key), 0);
        assert_eq!(
            claim.phase,
            crate::product::coding_models::CodingStartPhase::ProviderMayHaveStarted
        );
    }

    /// barrier 中窗一/二：claim 持久但未跨 provider barrier（Claimed /
    /// RunnerRegistered）——同 command 重放续启同一身份，异 command 只见
    /// AlreadyStarted，绝不二次首启。
    #[tokio::test]
    async fn same_command_resumes_before_provider_barrier() {
        use crate::product::coding_attempt_store::ClaimCodingStartOutcome;
        use crate::product::coding_models::CodingStartPhase;
        let fixture = crate::web::wiga_gate_fixture::ready_enrolled_attempt_fixture().await;
        let attempt = fixture.attempt();
        let origin = fixture.auto_origin();
        let store = fixture.store();
        let key = CodingAttemptRunKey::from_attempt(&attempt);

        // 窗口一：claim 后 / registry 激活前。
        let ClaimCodingStartOutcome::Claimed(claimed) = store
            .claim_coding_start(&attempt, "wiga-start-x", &origin)
            .expect("seed claim at window one")
        else {
            panic!("seed claim must succeed");
        };
        let drift = crate::web::coding_start::start_coding_once(
            &fixture.state,
            &attempt.project_id,
            &attempt.issue_id,
            StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: "wiga-start-other".into(),
                origin: origin.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            drift,
            StartCodingOutcome::AlreadyStarted {
                attempt_id: attempt.id.clone()
            }
        );
        let (probe, _entry, _hold) = paused_start_probe();
        let resumed = crate::web::coding_start::start_coding_once_with_probe(
            &fixture.state,
            &attempt.project_id,
            &attempt.issue_id,
            StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: "wiga-start-x".into(),
                origin: origin.clone(),
            },
            probe,
        )
        .await
        .unwrap();
        assert_eq!(resumed, StartCodingOutcome::Started { attempt_id: attempt.id.clone() });
        assert_eq!(fixture.runner_count(&key), 1);
        let saved = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap();
        assert_eq!(
            saved.start_claim.unwrap().phase,
            CodingStartPhase::ProviderMayHaveStarted
        );
    }

    /// barrier 中窗三：ProviderMayHaveStarted 已持久但无可信 Running/ledger
    /// 事实——外部副作用不可证明，人工分诊（NeedsHuman），claim 不复位、
    /// 零二次放行。
    #[tokio::test]
    async fn provider_may_have_started_window_routes_to_manual_triage() {
        use crate::product::coding_attempt_store::ClaimCodingStartOutcome;
        use crate::product::coding_models::CodingStartPhase;
        let fixture = crate::web::wiga_gate_fixture::ready_enrolled_attempt_fixture().await;
        let attempt = fixture.attempt();
        let origin = fixture.auto_origin();
        let store = fixture.store();
        let key = CodingAttemptRunKey::from_attempt(&attempt);
        let ClaimCodingStartOutcome::Claimed(claimed) = store
            .claim_coding_start(&attempt, "wiga-start-z", &origin)
            .expect("seed claim")
        else {
            panic!("seed claim must succeed");
        };
        store
            .advance_coding_start_phase(&claimed, "wiga-start-z", CodingStartPhase::ProviderMayHaveStarted)
            .expect("seed provider-may-have-started window");

        let same = crate::web::coding_start::start_coding_once(
            &fixture.state,
            &attempt.project_id,
            &attempt.issue_id,
            StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: "wiga-start-z".into(),
                origin: origin.clone(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            &same,
            StartCodingOutcome::NeedsHuman { reason, .. }
                if reason.contains("manual triage")
        ));
        assert_eq!(fixture.runner_count(&key), 0);
        let saved = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap();
        assert_eq!(
            saved.start_claim.unwrap().phase,
            CodingStartPhase::NeedsHuman
        );

        // 异 command 同样只见已消费的 claim。
        let other = crate::web::coding_start::start_coding_once(
            &fixture.state,
            &attempt.project_id,
            &attempt.issue_id,
            StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: "wiga-start-other".into(),
                origin,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            other,
            StartCodingOutcome::AlreadyStarted {
                attempt_id: attempt.id.clone()
            }
        );
    }

    /// barrier 中窗二：RunnerRegistered 已持久——同 command 续启、异 command
    /// AlreadyStarted（窗口二与窗口一仅 durable phase 不同）。
    #[tokio::test]
    async fn runner_registered_window_resumes_same_command_only() {
        use crate::product::coding_attempt_store::ClaimCodingStartOutcome;
        use crate::product::coding_models::CodingStartPhase;
        let fixture = crate::web::wiga_gate_fixture::ready_enrolled_attempt_fixture().await;
        let attempt = fixture.attempt();
        let origin = fixture.auto_origin();
        let store = fixture.store();
        let ClaimCodingStartOutcome::Claimed(claimed) = store
            .claim_coding_start(&attempt, "wiga-start-y", &origin)
            .expect("seed claim")
        else {
            panic!("seed claim must succeed");
        };
        store
            .advance_coding_start_phase(&claimed, "wiga-start-y", CodingStartPhase::RunnerRegistered)
            .expect("seed runner-registered window");
        let (probe, _entry, _hold) = paused_start_probe();
        let resumed = crate::web::coding_start::start_coding_once_with_probe(
            &fixture.state,
            &attempt.project_id,
            &attempt.issue_id,
            StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: "wiga-start-y".into(),
                origin: origin.clone(),
            },
            probe,
        )
        .await
        .unwrap();
        assert_eq!(resumed, StartCodingOutcome::Started { attempt_id: attempt.id.clone() });
        assert_eq!(
            fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
            1
        );
        let saved = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap();
        let claim = saved.start_claim.unwrap();
        assert_eq!(claim.command_id, "wiga-start-y");
        assert_eq!(claim.phase, CodingStartPhase::ProviderMayHaveStarted);
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
