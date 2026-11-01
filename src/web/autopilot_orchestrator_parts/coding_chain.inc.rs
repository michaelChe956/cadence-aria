/// 批 2 R2：N0 未解除的 advance/coding_start 失败事实判定（kind=
/// automation_failed、stage∈{advance,coding_start}、plan/session 与绑定
/// 当前代互证）。
fn unresolved_drive_failure_fact(
    enrollment: &crate::product::models::automation::IssueAutomationEnrollment,
    plan_id: &str,
    session_id: &str,
) -> bool {
    use crate::product::models::automation::{AutomationAttentionKind, AutomationAttentionStage};
    enrollment.attention_facts.iter().any(|fact| {
        fact.kind == AutomationAttentionKind::AutomationFailed
            && matches!(
                fact.stage,
                AutomationAttentionStage::Advance | AutomationAttentionStage::CodingStart
            )
            && fact.plan_id.as_deref() == Some(plan_id)
            && fact.session_id.as_deref() == Some(session_id)
    })
}

/// 批 2 R2（REQ-WIGA-03/C5-CHAIN、REQ-MTG-03）：LC 绑定的共享服务驱动
/// 链——先落 run（prepare，稳定 command=enrollment/plan 身份）→ Enrolled
/// advance → 首启；首项 outcome 落盘/在途即 `drive_ready_units_for_plan`
/// fan-out 其余就绪桶（tick 与完成挂点同源复用，per-plan 串行锁同键）；
/// 终局按 run 冻结 targets 的新鲜状态聚合（单桶 Completed≠全 plan 完成）。
async fn enrolled_lc_coding_chain(
    state: &WebAppState,
    paths: &ProductAppPaths,
    enrollment: &crate::product::models::automation::IssueAutomationEnrollment,
    plan_id: &str,
) -> Result<Option<ReconcileOutcome>, String> {
    use crate::product::coding_plan_run::CodingPlanStartOutcome;
    use crate::product::coding_plan_run::{CodingPlanRunPhase, CodingPlanRunStore};
    use crate::web::handlers::{
        EnrolledPlanDriveIdentity, EnrolledPlanStartStep, drive_ready_units_for_plan,
        enrolled_plan_coding_start,
    };

    let Ok(identity) = EnrolledPlanDriveIdentity::load(paths, enrollment) else {
        // 旧代无版本化绑定/内部投影漂移：fail-closed 人工分诊（无绑定
        // 事实写者授权，不写 N0 事实）。
        return Ok(Some(ReconcileOutcome::NeedsHuman));
    };
    let (run, step) = match enrolled_plan_coding_start(state, &identity).await {
        Ok(result) => result,
        Err(error) => {
            // 业务失败沿 N0 写入事实后按停点分诊；事实写失败
            //（automation_attention_fact_write_failed）按 Err 上抛——
            // tick 可见诊断，不吞成静默 NeedsHuman（不冒充已通知）。
            if error.code == "automation_attention_fact_write_failed" {
                return Err(format!(
                    "enrolled plan coding chain attention fact write failed: {}: {}",
                    error.code, error.message
                ));
            }
            return Ok(Some(ReconcileOutcome::NeedsHuman));
        }
    };
    match &step {
        EnrolledPlanStartStep::AdvancedToReady => return Ok(Some(ReconcileOutcome::Advancing)),
        EnrolledPlanStartStep::Recorded(CodingPlanStartOutcome::NeedsHuman { .. })
        | EnrolledPlanStartStep::Existing(CodingPlanRunPhase::NeedsHuman) => {
            return Ok(Some(ReconcileOutcome::NeedsHuman));
        }
        _ => {}
    }

    // 首项 outcome 已落盘 / run 已在途：全图 fan-out（与完成挂点同函数、
    // 同 per-plan 串行锁；重复 tick 同 claim 零新启）。
    let run_store = CodingPlanRunStore::new(paths.clone());
    let mut run = run_store
        .get(&enrollment.project_id, &enrollment.issue_id, plan_id)
        .map_err(|error| format!("coding plan run unreadable: {error}"))?
        .unwrap_or(run);
    if matches!(
        run.phase,
        CodingPlanRunPhase::Started | CodingPlanRunPhase::AlreadyStarted
    ) {
        let _drive_guard = state
            .coding_runs
            .lock_named(&format!(
                "work_item_group:{}:{}:{}",
                enrollment.project_id, enrollment.issue_id, plan_id
            ))
            .await;
        drive_ready_units_for_plan(
            state,
            &enrollment.project_id,
            &enrollment.issue_id,
            plan_id,
            None,
        )
        .await
        .map_err(|error| {
            format!(
                "enrolled plan fan-out failed: {}: {}",
                error.code, error.message
            )
        })?;
        run = run_store
            .get(&enrollment.project_id, &enrollment.issue_id, plan_id)
            .map_err(|error| format!("coding plan run unreadable: {error}"))?
            .unwrap_or(run);
    }
    Ok(Some(aggregate_plan_targets_fresh_status(paths, &run)?))
}

/// 批 2 R2：run 冻结 targets 的新鲜状态聚合（`coding_chain_stage` 终局
/// 分诊）——任一失败/人工恢复 → NeedsHuman；任一人控门 → AwaitingHuman；
/// 其余（Created/Running/Completed/amendment）为执行或观察：单桶
/// Completed≠全 plan 完成（其余桶继续由 fan-out 驱动）。空绑定集
///（ready_bound 等待窗口）视为执行观察。
pub(crate) fn aggregate_plan_targets_fresh_status(
    paths: &ProductAppPaths,
    run: &crate::product::coding_plan_run::CodingPlanRun,
) -> Result<ReconcileOutcome, String> {
    use crate::product::coding_models::CodingAttemptStatus;

    let store = crate::product::coding_attempt_store::CodingAttemptStore::new(paths.clone());
    let mut awaiting_human = false;
    for binding in &run.target_attempts {
        let attempt = store
            .get_attempt(&run.project_id, &run.issue_id, &binding.attempt_id)
            .map_err(|error| format!("run target attempt unreadable: {error}"))?;
        match attempt.status {
            CodingAttemptStatus::Failed
            | CodingAttemptStatus::Aborted
            | CodingAttemptStatus::AwaitingManualRecovery => {
                return Ok(ReconcileOutcome::NeedsHuman);
            }
            CodingAttemptStatus::WaitingForHuman | CodingAttemptStatus::Blocked => {
                awaiting_human = true;
            }
            CodingAttemptStatus::Created
            | CodingAttemptStatus::Running
            | CodingAttemptStatus::Completed
            | CodingAttemptStatus::AwaitingPlanAmendment
            | CodingAttemptStatus::ApplyingPlanAmendment
            | CodingAttemptStatus::AmendmentApplyFailed => {}
        }
    }
    if awaiting_human {
        Ok(ReconcileOutcome::AwaitingHuman)
    } else {
        Ok(ReconcileOutcome::Coding)
    }
}

/// 物理单仓（及旧代无 target enrollment）的单 attempt 自动链（P2 Task 5
/// 原语义，R2 保留）：未 Ready 先 stable-id `advance_plan`（到 Ready 即
/// 止）；Ready 后按 group journal 定位唯一 attempt，按状态分诊或 stable-id
/// AutoStartOnce（`wiga-start-{attempt_id}`）。失败沿 N0 写
/// `automation_failed` 事实（瞬态准入争用除外，下一 tick 重试）。
async fn single_attempt_coding_chain(
    state: &WebAppState,
    paths: &ProductAppPaths,
    enrollment: &crate::product::models::automation::IssueAutomationEnrollment,
    plan_id: &str,
) -> Result<Option<ReconcileOutcome>, String> {
    use crate::product::advance_store::{
        AdvanceInput, AdvanceOutcome, AdvanceStatus, AdvanceStore,
    };
    use crate::product::coding_models::{CodingAttemptStatus, CodingStartOrigin};
    use crate::product::models::automation::AutomationAttentionStage;
    use crate::web::advance_plan::{AdvancePlanOrigin, advance_plan};
    use crate::web::coding_start::{StartCodingCommand, StartCodingOutcome, start_coding_once};
    use crate::web::handlers::upsert_automation_drive_failure_fact;

    let project_id = enrollment.project_id.clone();
    let issue_id = enrollment.issue_id.clone();
    let advance_command_id = format!("wiga-advance-{}-{plan_id}", enrollment.enrollment_id);

    let advance_ready = AdvanceStore::new(paths.clone())
        .get_advance_for_plan(&project_id, &issue_id, plan_id)
        .map_err(|error| format!("advance record unreadable: {error}"))?
        .is_some_and(|record| record.status == AdvanceStatus::Ready);
    if !advance_ready {
        let outcome = advance_plan(
            state,
            AdvanceInput {
                command_id: advance_command_id.clone(),
                project_id: project_id.clone(),
                issue_id: issue_id.clone(),
                plan_id: plan_id.to_string(),
            },
            AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
            },
        )
        .await;
        let proceed = match &outcome {
            Ok(AdvanceOutcome::Completed { record, .. })
                if record.status == AdvanceStatus::Ready =>
            {
                return Ok(Some(ReconcileOutcome::Advancing));
            }
            Ok(AdvanceOutcome::Replayed { record }) if record.status == AdvanceStatus::Ready => {
                true
            }
            Ok(AdvanceOutcome::Rejected { code, .. })
                if code == "ADVANCE_PLAN_ADMISSION_CONTENDED" =>
            {
                // 瞬态准入争用：不写事实，下一 tick 重试。
                return Ok(Some(ReconcileOutcome::NeedsHuman));
            }
            other => {
                // R2（REQ-WIGA-12）：advance 失败写 N0 事实（checkpoint 取
                // record.updated_at，无 record 首次冻结）；写失败上抛。
                let (reason_code, checkpoint) = match other {
                    Ok(AdvanceOutcome::Rejected { record, code, .. }) => {
                        (code.clone(), record.as_ref().map(|r| r.updated_at.clone()))
                    }
                    Ok(AdvanceOutcome::Completed { record, .. })
                    | Ok(AdvanceOutcome::Replayed { record }) => (
                        "enrolled_advance_not_ready".to_string(),
                        Some(record.updated_at.clone()),
                    ),
                    Err(_) => (
                        "enrolled_advance_failed".to_string(),
                        AdvanceStore::new(paths.clone())
                            .get_advance_for_plan(&project_id, &issue_id, plan_id)
                            .ok()
                            .flatten()
                            .map(|record| record.updated_at),
                    ),
                };
                upsert_automation_drive_failure_fact(
                    paths,
                    enrollment,
                    AutomationAttentionStage::Advance,
                    plan_id,
                    &advance_command_id,
                    None,
                    &reason_code,
                    checkpoint,
                )?;
                return Ok(Some(ReconcileOutcome::NeedsHuman));
            }
        };
        if !proceed {
            return Ok(Some(ReconcileOutcome::NeedsHuman));
        }
    }

    // Ready：group journal 只用于定位唯一 attempt 身份；状态必须从 attempt
    // store 读新鲜值——journal 内嵌快照冻结于初始化时点（恒 Created/
    // PrepareContext），据其分诊会在 attempt 已停等人时重入首启路径，
    // 把 durable claim 误降级 NeedsHuman。
    let journal = crate::product::coding_attempt_store::CodingAttemptStore::new(paths.clone())
        .get_group_initialization(&project_id, &issue_id, plan_id)
        .map_err(|error| format!("group initialization journal unreadable: {error}"))?;
    let attempt = journal.attempt;
    let attempt = crate::product::coding_attempt_store::CodingAttemptStore::new(paths.clone())
        .get_attempt(&project_id, &issue_id, &attempt.id)
        .map_err(|error| format!("group attempt unreadable: {error}"))?;
    match attempt.status {
        CodingAttemptStatus::Running
        | CodingAttemptStatus::AwaitingPlanAmendment
        | CodingAttemptStatus::ApplyingPlanAmendment
        | CodingAttemptStatus::AmendmentApplyFailed
        | CodingAttemptStatus::Completed => Ok(Some(ReconcileOutcome::Coding)),
        CodingAttemptStatus::WaitingForHuman | CodingAttemptStatus::Blocked => {
            Ok(Some(ReconcileOutcome::AwaitingHuman))
        }
        CodingAttemptStatus::Failed
        | CodingAttemptStatus::Aborted
        | CodingAttemptStatus::AwaitingManualRecovery => Ok(Some(ReconcileOutcome::NeedsHuman)),
        CodingAttemptStatus::Created => {
            let command_id = format!("wiga-start-{}", attempt.id);
            let start = start_coding_once(
                state,
                &project_id,
                &issue_id,
                StartCodingCommand {
                    attempt_id: attempt.id.clone(),
                    command_id: command_id.clone(),
                    origin: CodingStartOrigin::Enrolled {
                        enrollment_id: enrollment.enrollment_id.clone(),
                        policy_revision: enrollment.policy_revision,
                        // C1 Task 1：claim 携带当前 binding 身份（旧
                        // enrollment 无声明时 None，消费侧 fail-closed）。
                        binding_version: enrollment
                            .binding_history
                            .as_ref()
                            .map(|history| history.current.binding_version),
                        target: enrollment.target.clone(),
                    },
                },
            )
            .await;
            match start {
                Ok(StartCodingOutcome::Started { .. })
                | Ok(StartCodingOutcome::AlreadyStarted { .. }) => {
                    Ok(Some(ReconcileOutcome::Coding))
                }
                Ok(StartCodingOutcome::NeedsHuman { attempt_id, reason }) => {
                    // R2（REQ-WIGA-12）：人控分诊是 coding_start 失败停点。
                    upsert_automation_drive_failure_fact(
                        paths,
                        enrollment,
                        AutomationAttentionStage::CodingStart,
                        plan_id,
                        &command_id,
                        Some(attempt_id),
                        &reason,
                        attempt
                            .start_claim
                            .as_ref()
                            .map(|claim| claim.claimed_at.clone()),
                    )?;
                    Ok(Some(ReconcileOutcome::NeedsHuman))
                }
                Err(error) => {
                    upsert_automation_drive_failure_fact(
                        paths,
                        enrollment,
                        AutomationAttentionStage::CodingStart,
                        plan_id,
                        &command_id,
                        Some(attempt.id.clone()),
                        error.code(),
                        attempt
                            .start_claim
                            .as_ref()
                            .map(|claim| claim.claimed_at.clone()),
                    )?;
                    Ok(Some(ReconcileOutcome::NeedsHuman))
                }
            }
        }
    }
}
