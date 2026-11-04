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
        let crashed = tokio::spawn(async move { advance_plan(&state, request, origin).await });
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
        let crashed = tokio::spawn(async move { advance_plan(&state, request, origin).await });
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
    let started = usize::from(matches!(manual, Ok(StartCodingOutcome::Started { .. })))
        + usize::from(matches!(auto, Ok(StartCodingOutcome::Started { .. })));
    assert_eq!(
        started, 1,
        "exactly one first start across registries: manual={manual:?} auto={auto:?}"
    );
    let saved = fixture
        .store()
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    let claim = saved
        .start_claim
        .as_ref()
        .expect("durable single-flight claim");
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
    let ClaimCodingStartOutcome::Claimed(_claimed) = store
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
    assert_eq!(
        resumed,
        StartCodingOutcome::Started {
            attempt_id: attempt.id.clone()
        }
    );
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
        .advance_coding_start_phase(
            &claimed,
            "wiga-start-z",
            CodingStartPhase::ProviderMayHaveStarted,
        )
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
    assert_eq!(
        resumed,
        StartCodingOutcome::Started {
            attempt_id: attempt.id.clone()
        }
    );
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
fn enrolled_origin(fixture: &EnrolledGateFixture) -> CodingStartOrigin {
    let enrollment = fixture.enrollment();
    CodingStartOrigin::Enrolled {
        enrollment_id: enrollment.enrollment_id.clone(),
        policy_revision: enrollment.policy_revision,
        binding_version: enrollment
            .binding_history
            .as_ref()
            .map(|history| history.current.binding_version),
        target: enrollment.target.clone(),
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
    assert_eq!(
        result.unwrap_err().code(),
        "coding_start_enrollment_disabled"
    );
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
                command_id: None,
                selection_key: before.selection_key.clone(),
                source: before.source.clone(),
                options: before.options.clone(),
                target: before
                    .target
                    .clone()
                    .expect("fixture enrollment declares a target"),
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
        crate::product::coding_attempt_store::CodingAttemptStore::new(fixture.inner.paths.clone());
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


include!("tests_parts/restart_and_multirepo.inc.rs");
