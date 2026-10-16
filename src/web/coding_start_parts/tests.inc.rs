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
    ready_single_repository_attempt_fixture,
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

/// C5 Task 5 红测：单仓 enrollment 的 typed StartCoding 互证 fail-closed——
/// attempt 携带 LC target_snapshot（身份漂移）或 issue 换仓/改属 LC 后
/// 载体互证失败，均以 coding_start_enrollment_mismatch 拒绝，零 claim、
/// 零 runner（Review Focus 3）。
#[tokio::test]
async fn single_repository_start_coding_rejects_logical_snapshot_and_wrong_repository() {
    // ① snapshot 漂移：单仓 enrollment + attempt 携带 LC target_snapshot。
    let fixture = ready_single_repository_attempt_fixture().await;
    let attempt = fixture.attempt();
    let store = fixture.store();
    let mut polluted = attempt.clone();
    polluted.target_snapshot = Some(crate::product::coding_models::AttemptTargetSnapshot {
        logical_repository_id: crate::product::logical_codebase::LogicalRepositoryId(
            uuid::Uuid::new_v4(),
        ),
        checkout_id: crate::product::logical_codebase::RepositoryCheckoutId(
            uuid::Uuid::new_v4(),
        ),
        physical_repository_id: "physical-polluted".to_string(),
        canonical_path: std::path::PathBuf::from("/tmp/polluted"),
        git_dir_identity: "sha256:polluted".to_string(),
        revision: None,
        policy_digest: "polluted".to_string(),
        membership_revision: 1,
        captured_at: "2026-09-30T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    });
    store.write_coding_attempt_for_test(&polluted).unwrap();
    let error = start_coding_once(
        &fixture.state,
        &attempt.project_id,
        &attempt.issue_id,
        StartCodingCommand {
            attempt_id: attempt.id.clone(),
            command_id: "single-repo-start-snapshot".into(),
            origin: fixture.auto_origin(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "coding_start_enrollment_mismatch", "{error:?}");
    assert!(
        error.message().contains("single physical repository")
            || error.message().contains("snapshot"),
        "rejection must describe the cross-carrier drift: {error:?}"
    );
    let saved = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert!(saved.start_claim.is_none(), "rejection must not consume a claim");
    assert_eq!(
        fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
        0
    );

    // ② 错仓：issue.repo_id 改指另一已登记物理仓（不同 git 根，不触发
    // authority 409）→ 载体互证失败，等待项语义（重新绑定提示）。
    let fixture = ready_single_repository_attempt_fixture().await;
    let attempt = fixture.attempt();
    let repo_root = fixture.inner._root.path().join("repo-2");
    std::fs::create_dir_all(&repo_root).unwrap();
    let status = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&repo_root)
        .status()
        .unwrap();
    assert!(status.success(), "repo-2 must be a real git repo");
    let repos_path = fixture.inner.paths.project_root(PROJECT_ID).join("repos.json");
    let mut repositories: Vec<crate::product::models::RepositoryRecord> =
        crate::product::json_store::read_json(&repos_path).unwrap();
    let repo_two = crate::product::models::RepositoryRecord {
        id: "repo-2".to_string(),
        name: "repo-2".to_string(),
        path: repo_root,
        repo_hash: "sha256:fixture-repo-2".to_string(),
        runtime_root: fixture.inner._root.path().join("repo-2/.aria/runtime"),
        ..repositories[0].clone()
    };
    repositories.push(repo_two);
    crate::product::json_store::write_json(&repos_path, &repositories).unwrap();
    let issue_path = fixture
        .inner
        .paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("issue.json");
    let mut issue: serde_json::Value =
        crate::product::json_store::read_json(&issue_path).unwrap();
    issue["repo_id"] = serde_json::json!("repo-2");
    crate::product::json_store::write_json(&issue_path, &issue).unwrap();

    let error = start_coding_once(
        &fixture.state,
        &attempt.project_id,
        &attempt.issue_id,
        StartCodingCommand {
            attempt_id: attempt.id.clone(),
            command_id: "single-repo-start-wrong-repo".into(),
            origin: fixture.auto_origin(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "coding_start_enrollment_mismatch", "{error:?}");
    assert!(
        error.message().contains("carrier drift")
            || error.message().contains("re-bind")
            || error.message().contains("repository"),
        "rejection must point at the carrier drift: {error:?}"
    );
    assert_eq!(
        fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
        0
    );
}

/// C5 Task 5：单仓互证通过——attempt 无 target_snapshot 且 issue 当前
/// authority 载体解析为 enrollment 同一物理仓 → typed 首启可发生
///（provider 真实入口被 probe 暂停，不冒充启动）。
#[tokio::test]
async fn single_repository_start_coding_passes_mutual_verification() {
    let fixture = ready_single_repository_attempt_fixture().await;
    let attempt = fixture.attempt();
    let (probe, _entry, _hold) = paused_start_probe();
    let outcome = crate::web::coding_start::start_coding_once_with_probe(
        &fixture.state,
        &attempt.project_id,
        &attempt.issue_id,
        StartCodingCommand {
            attempt_id: attempt.id.clone(),
            command_id: "single-repo-start-ok".into(),
            origin: fixture.auto_origin(),
        },
        probe,
    )
    .await
    .expect("single-repository mutual verification passes");
    assert!(
        matches!(outcome, StartCodingOutcome::Started { .. }),
        "unexpected outcome: {outcome:?}"
    );
    assert_eq!(
        fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
        1
    );
}

/// C2 Task 3（REQ-CRO-03）：abort 置 retired 后，显式 restart 在同进程内
/// 清退役标记→重开 admission→spawn 新 runner，无需重启服务；同 command
/// 同 payload 重放首次 durable 结果（Replayed，不二次 spawn）；错版本
/// Rejected（不改 attempt、不启动 provider）。断言全部同步执行，被
/// spawn 的 runner 任务在单线程 runtime 下于测试结束前不推进。
#[tokio::test]
async fn restart_after_abort_readmits_attempt_in_same_process() {
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::CodingAttemptStore;
    use crate::product::coding_models::CodingAttemptStatus;
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::models::automation::OperationState;
    use crate::web::coding_start::{
        RestartCodingAttemptRequest, restart_coding_attempt,
    };

    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let state = crate::web::state::WebAppState::new(
        root.clone(),
        crate::web::runtime::WebRuntime::new_fake(root.clone()),
    );
    let paths = ProductAppPaths::new(root.join(".aria"));
    let store = CodingAttemptStore::new(paths.clone());
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: None,
            title: "restart issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("issue");
    let attempt = store
        .create_attempt(crate::product::coding_attempt_store::CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/restart-attempt".to_string(),
            worktree_path: None,
            provider_config_snapshot: crate::web::workspace_ws_types::ProviderConfigSnapshot {
                author: crate::product::models::ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 0,
        })
        .expect("attempt");
    let running = store
        .seed_running_attempt_for_test(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )
        .expect("running attempt");

    // abort：registry retired ＋ durable 终态（Aborted）。
    let key = CodingAttemptRunKey::from_attempt(&running);
    state.coding_runs.abort_attempt(&key).await;
    let mut aborted = running.clone();
    aborted.status = CodingAttemptStatus::Aborted;
    // 真实 abort 链（transition_to_terminal）清 admission ticket（admission.rs:223）。
    aborted.admission_ticket_consumed_at = None;
    store.write_coding_attempt_for_test(&aborted).expect("aborted");

    let request = RestartCodingAttemptRequest {
        command_id: "cmd-restart-1".to_string(),
        attempt_id: attempt.id.clone(),
        expected_attempt_version: aborted.version,
    };
    let result = restart_coding_attempt(
        &state,
        &attempt.project_id,
        &attempt.issue_id,
        request.clone(),
    )
    .await
    .expect("restart service");
    assert_eq!(result.state, OperationState::Accepted);
    assert_eq!(result.attempt_id, attempt.id);

    // durable 重开为 Running；退役围栏已清——新 runner 已在 registry 注册
    // （spawn 成功即 retired 不再拦截 insert_cancellable）。
    let current = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(current.status, CodingAttemptStatus::Running);
    assert_eq!(
        state.coding_runs.runner_count(&key),
        1,
        "new runner must be registered in the same process"
    );

    // 同 command 同 payload：重放首次 durable 结果，不改变状态。
    let replay = restart_coding_attempt(
        &state,
        &attempt.project_id,
        &attempt.issue_id,
        request,
    )
    .await
    .expect("replay");
    assert_eq!(replay.state, OperationState::Replayed);

    // 错版本（错对象）：Rejected，不改变当前 attempt。
    let wrong = restart_coding_attempt(
        &state,
        &attempt.project_id,
        &attempt.issue_id,
        RestartCodingAttemptRequest {
            command_id: "cmd-restart-2".to_string(),
            attempt_id: attempt.id.clone(),
            expected_attempt_version: current.version + 999,
        },
    )
    .await
    .expect("wrong version maps to Rejected result");
    assert_eq!(wrong.state, OperationState::Rejected);
    let after = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    assert_eq!(after.status, CodingAttemptStatus::Running);
}

/// 多仓热修：restart 租约预检按 attempt 路由收窄——兄弟仓预绑定锁
///（异仓、活跃异主）不得把本 attempt 的显式 restart 误判为
/// `coding_run_already_running` 停等；本仓自持终态残留按 G5 释放+重绑。
#[tokio::test]
async fn restart_multi_repo_attempt_passes_precheck_despite_sibling_prebound_lock() {
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::CodingAttemptStore;
    use crate::product::coding_models::{
        AttemptTargetSnapshot, CodingAttemptStatus,
    };
    use crate::product::lifecycle_store::{LifecycleStore, UpsertRepoSharedWorktreeInput};
    use crate::product::logical_codebase::{
        IssueCodebaseSelection, IssueCodebaseSelectionStore, LogicalCodebaseManifest,
        LogicalCodebaseStore, LogicalRepositoryId, RepositoryCheckoutId,
    };
    use crate::product::models::automation::OperationState;
    use crate::web::coding_start::{RestartCodingAttemptRequest, restart_coding_attempt};
    use uuid::Uuid;

    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().to_path_buf();
    let state = crate::web::state::WebAppState::new(
        root.clone(),
        crate::web::runtime::WebRuntime::new_fake(root.clone()),
    );
    let paths = ProductAppPaths::new(root.join(".aria"));
    // sibling UUID 恒小于 own：旧 issue 级 classify 按 UUID 升序必然先取
    // 兄弟仓预绑定锁（事故形态）。
    let own_repo = LogicalRepositoryId(Uuid::from_u128(2));
    let sibling_repo = LogicalRepositoryId(Uuid::from_u128(1));
    // 真实工件链（durable 重开 admission 重验快照身份与 policy）：
    // project → manifest → member/checkout → repos.json → policy。
    crate::product::project_store::ProjectStore::new(paths.clone())
        .create(crate::product::project_store::CreateProjectInput {
            name: "project".to_string(),
            description: None,
        })
        .expect("project");
    let authority = LogicalCodebaseStore::new(paths.clone());
    let manifest = LogicalCodebaseManifest::new(
        "project_0001",
        root.join("aggregate-root"),
        vec![own_repo, sibling_repo],
    );
    authority
        .save_manifest("project_0001", &manifest)
        .expect("manifest");
    let mut members = Vec::new();
    let mut snapshots = std::collections::HashMap::new();
    for (ordinal, (repo, physical)) in [
        (own_repo, "repository_0001"),
        (sibling_repo, "repository_0002"),
    ]
    .into_iter()
    .enumerate()
    {
        let repository_path = root.join(physical);
        let source_identity = crate::product::logical_codebase::RepositorySourceIdentity::from_git_parts(
            &repository_path,
            repository_path.join(".git"),
            None,
        );
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        authority
            .save_member(
                "project_0001",
                &crate::product::logical_codebase::CodebaseMemberRecord {
                    logical_repository_id: repo,
                    physical_repository_id: physical.to_string(),
                    alias: physical.to_string(),
                    role: "repository".to_string(),
                    ordinal: ordinal as u32 + 1,
                    source_identity: source_identity.clone(),
                    repo_type: crate::product::logical_codebase::RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: crate::product::logical_codebase::MemberStatus::Active,
                    created_at: "2026-10-14T00:00:00Z".to_string(),
                    updated_at: "2026-10-14T00:00:00Z".to_string(),
                },
            )
            .expect("member");
        authority
            .save_checkout(
                "project_0001",
                &crate::product::logical_codebase::RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: repo,
                    physical_repository_id: physical.to_string(),
                    kind: crate::product::logical_codebase::CheckoutKind::Main,
                    canonical_path: repository_path.clone(),
                    checkout_path_hash: "sha256:checkout".to_string(),
                    git_dir_identity: source_identity.git_dir_identity(),
                    revision: Some("abcdef".to_string()),
                    availability: crate::product::logical_codebase::CheckoutAvailability::Available,
                    observed_at: "2026-10-14T00:00:00Z".to_string(),
                    created_at: "2026-10-14T00:00:00Z".to_string(),
                    updated_at: "2026-10-14T00:00:00Z".to_string(),
                },
            )
            .expect("checkout");
        members.push(crate::product::models::RepositoryRecord {
            id: physical.to_string(),
            project_id: "project_0001".to_string(),
            name: physical.to_string(),
            path: repository_path.clone(),
            repo_hash: format!("sha256:{physical}"),
            runtime_root: std::path::PathBuf::from("runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: "2026-10-14T00:00:00Z".to_string(),
            updated_at: "2026-10-14T00:00:00Z".to_string(),
            logical_repository_id: Some(repo),
            primary_checkout_id: Some(checkout_id),
            identity_schema_version: 1,
        });
        snapshots.insert(
            repo,
            (
                checkout_id,
                repository_path,
                physical.to_string(),
                source_identity.git_dir_identity(),
            ),
        );
    }
    crate::product::json_store::write_json(
        &paths.project_root("project_0001").join("repos.json"),
        &members,
    )
    .expect("repos.json");
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![own_repo, sibling_repo],
            Vec::new(),
            vec![own_repo, sibling_repo],
            None,
        ))
        .expect("selection");
    let policy_store = crate::product::logical_codebase::AggregatePolicyArtifactStore::new(
        paths.clone(),
    );
    policy_store
        .ensure_bootstrap(&manifest)
        .expect("policy bootstrap");
    let policy_digest = policy_store
        .get("project_0001")
        .expect("policy")
        .expect("policy present")
        .digest;
    let snapshot = |repo: LogicalRepositoryId| AttemptTargetSnapshot {
        logical_repository_id: repo,
        checkout_id: snapshots[&repo].0,
        physical_repository_id: snapshots[&repo].2.clone(),
        canonical_path: snapshots[&repo].1.clone(),
        git_dir_identity: snapshots[&repo].3.clone(),
        revision: Some("abcdef".to_string()),
        policy_digest: policy_digest.clone(),
        membership_revision: manifest.membership_revision,
        captured_at: "2026-10-14T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    };
    let lifecycle = LifecycleStore::new(paths.clone());
    for (repo, name) in [(own_repo, "own-worktree"), (sibling_repo, "sibling-worktree")] {
        let worktree = root.join(name);
        std::fs::create_dir_all(&worktree).expect("worktree dir");
        lifecycle
            .upsert_repo_shared_worktree(UpsertRepoSharedWorktreeInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: repo,
                branch_name: "main".to_string(),
                worktree_path: worktree,
                base_branch: "main".to_string(),
            })
            .expect("repo shared worktree");
    }
    let store = CodingAttemptStore::new(paths.clone());
    let provider = crate::web::workspace_ws_types::ProviderConfigSnapshot {
        author: crate::product::models::ProviderName::Fake,
        reviewer: None,
        review_rounds: 0,
        permission_modes: Default::default(),
    };
    let own = store
        .create_attempt(crate::product::coding_attempt_store::CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/restart-multi-own".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider.clone(),
            target_snapshot: Some(snapshot(own_repo)),
            max_auto_rework: 0,
        })
        .expect("own attempt");
    let sibling = store
        .create_attempt(crate::product::coding_attempt_store::CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0002".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/restart-multi-sibling".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider,
            target_snapshot: Some(snapshot(sibling_repo)),
            max_auto_rework: 0,
        })
        .expect("sibling attempt");
    // 本仓自持（后随 abort 成为终态残留）；兄弟仓预绑定到未启动的
    // sibling attempt（Created 亦是活跃态）。
    for (repo, work_item_id, attempt_id) in [
        (own_repo, "work_item_0001", own.id.clone()),
        (sibling_repo, "work_item_0002", sibling.id.clone()),
    ] {
        let lease = lifecycle
            .try_acquire_repo_worktree_lock(
                "project_0001",
                "issue_0001",
                repo,
                work_item_id,
                &format!("repo_worktree_lease_{attempt_id}"),
            )
            .expect("acquire repo lease");
        assert!(lease.acquired);
        lifecycle
            .bind_repo_worktree_lock_to_attempt(
                "project_0001",
                "issue_0001",
                repo,
                work_item_id,
                &attempt_id,
            )
            .expect("bind repo lease");
    }
    let mut aborted = own.clone();
    aborted.status = CodingAttemptStatus::Aborted;
    aborted.admission_ticket_consumed_at = None;
    store
        .write_coding_attempt_for_test(&aborted)
        .expect("aborted own");

    let result = restart_coding_attempt(
        &state,
        "project_0001",
        "issue_0001",
        RestartCodingAttemptRequest {
            command_id: "cmd-restart-multi-1".to_string(),
            attempt_id: own.id.clone(),
            expected_attempt_version: aborted.version,
        },
    )
    .await
    .expect("restart service");
    assert_eq!(
        result.state,
        OperationState::Accepted,
        "sibling repo prebound lock must not stall restart: {:?}",
        result.reason
    );
    // 兄弟仓锁事实原样保留；本仓重绑回重启的 attempt。
    let sibling_record = lifecycle
        .get_repo_shared_worktree("project_0001", "issue_0001", sibling_repo)
        .expect("sibling record")
        .expect("sibling record present");
    assert_eq!(
        sibling_record.current_lock_owner_id.as_deref(),
        Some(sibling.id.as_str())
    );
    let own_record = lifecycle
        .get_repo_shared_worktree("project_0001", "issue_0001", own_repo)
        .expect("own record")
        .expect("own record present");
    assert_eq!(
        own_record.current_lock_owner_id.as_deref(),
        Some(own.id.as_str())
    );
}

/// lcg_t13(计划 Task 13 Step 1):plan 到 durable Ready 后不得自动为
/// 其他 target 派工——显式 StartCoding 是唯一 provider 启动入口。
/// ready 时不发任何命令:attempt 集合不增(other target 零新 attempt)、
/// runner 零注册(provider 启动计数 0)、attempt 保持 Ready;随后一条
/// 显式 Manual StartCoding 即可放行(probe 暂停 provider 入口,不冒充
/// 启动)。
#[tokio::test]
async fn lcg_t13_ready_plan_does_not_start_other_target_without_manual_command() {
    let fixture = ready_single_repository_attempt_fixture().await;
    let attempt = fixture.attempt();
    let store = fixture.store();
    let before_count = store
        .list_attempts_for_issue(&attempt.project_id, &attempt.issue_id)
        .expect("list attempts before settle")
        .len();
    let status_before = attempt.status.clone();

    // ready 后静置:无后台自动派工/自动 provider 启动。
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let after_count = store
        .list_attempts_for_issue(&attempt.project_id, &attempt.issue_id)
        .expect("list attempts after settle")
        .len();
    assert_eq!(
        after_count, before_count,
        "a ready plan must not auto-dispatch attempts for other targets"
    );
    assert_eq!(
        fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
        0,
        "provider start count before an explicit start_coding command must be 0"
    );
    let settled = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("settled attempt");
    assert_eq!(
        settled.status, status_before,
        "no background worker may move a ready attempt without a command"
    );

    // 显式 Manual StartCoding 仍是唯一放行入口(probe 暂停 provider 真实
    // 入口):Started 且恰一 runner——证明上方零启动不是门坏,而是没有
    // 命令就没有派工。
    let (probe, _entry, _hold) = paused_start_probe();
    let outcome = crate::web::coding_start::start_coding_once_with_probe(
        &fixture.state,
        &attempt.project_id,
        &attempt.issue_id,
        StartCodingCommand {
            attempt_id: attempt.id.clone(),
            command_id: "lcg-t13-manual-1".into(),
            origin: CodingStartOrigin::Manual,
        },
        probe,
    )
    .await
    .expect("explicit manual start coding must be admitted at Ready");
    assert!(matches!(outcome, StartCodingOutcome::Started { .. }), "{outcome:?}");
    assert_eq!(
        fixture.runner_count(&CodingAttemptRunKey::from_attempt(&attempt)),
        1
    );
}

/// k3 波2 F4（选型 A 读侧回退）：集合绑定的成员解析双形态读——
/// ①legacy 旧根现场（enrollment 冻结 manifest UUID，成员驻留 project 级
/// 旧根）：uuid 子树失败回退 issue 作用域旧根解析（修复前红：
/// logical_repository_manifest not found）；②per-LC uuid 子树现场直接
/// 命中；③成员不存在仍 fail-closed。
#[test]
fn set_enrollment_member_resolution_reads_legacy_root_fallback() {
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest,
        LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
        RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    };
    use crate::web::handlers::automation_enrollment_test_support::{
        ISSUE_ID as PROJECT_ISSUE_ID, PROJECT_ID, seed_fixture,
    };

    let resolve = |paths: &crate::product::app_paths::ProductAppPaths,
                   lc: &str,
                   member: LogicalRepositoryId| {
        crate::web::coding_start::resolve_set_enrollment_member(
            paths,
            PROJECT_ID,
            PROJECT_ISSUE_ID,
            lc,
            member,
        )
    };

    // ① legacy 旧根现场：seed_fixture 播种 project 级旧根 manifest/成员/
    ///checkout/repos.json；enrollment 冻结任意 manifest UUID 形态。
    let fixture = seed_fixture(2, true);
    let enrolled_lc = uuid::Uuid::new_v4().to_string();
    let member_one = LogicalRepositoryId(uuid::Uuid::from_u128(1));
    let (member, checkout, repository) =
        resolve(&fixture.paths, &enrolled_lc, member_one)
            .expect("legacy root fallback must resolve set enrollment member");
    assert_eq!(member.logical_repository_id, member_one);
    assert_eq!(checkout.kind, CheckoutKind::Main);
    assert_eq!(repository.logical_repository_id, Some(member_one));

    // ② per-LC uuid 子树现场：for_lc 权威子树直接命中（无回退参与）。
    let subtree_root = tempfile::tempdir().unwrap();
    let subtree_paths =
        crate::product::app_paths::ProductAppPaths::new(subtree_root.path().join(".aria"));
    let lc_uuid = uuid::Uuid::new_v4().to_string();
    let authority = LogicalCodebaseStore::for_lc(subtree_paths.clone(), &lc_uuid);
    let manifest =
        LogicalCodebaseManifest::new(PROJECT_ID, subtree_root.path().to_path_buf(), vec![member_one]);
    authority.save_manifest(PROJECT_ID, &manifest).unwrap();
    let now = "2026-10-16T00:00:00Z".to_string();
    let checkout_path = subtree_root.path().join("checkout-f4");
    std::fs::create_dir_all(&checkout_path).unwrap();
    let source_identity =
        RepositorySourceIdentity::from_git_parts(&checkout_path, checkout_path.join(".git"), None);
    let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
    authority
        .save_member(
            PROJECT_ID,
            &CodebaseMemberRecord {
                logical_repository_id: member_one,
                physical_repository_id: "physical-f4".to_string(),
                alias: "repo-f4".to_string(),
                role: "repository".to_string(),
                ordinal: 1,
                source_identity: source_identity.clone(),
                repo_type: RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![checkout_id],
                status: MemberStatus::Active,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .unwrap();
    authority
        .save_checkout(
            PROJECT_ID,
            &RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: member_one,
                physical_repository_id: "physical-f4".to_string(),
                kind: CheckoutKind::Main,
                canonical_path: checkout_path,
                checkout_path_hash: "sha256:checkout-f4".to_string(),
                git_dir_identity: source_identity.git_dir_identity(),
                revision: Some("abcdef".to_string()),
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            },
        )
        .unwrap();
    let (subtree_member, _, subtree_repository) =
        resolve(&subtree_paths, &lc_uuid, member_one)
            .expect("uuid subtree site must resolve directly");
    assert_eq!(subtree_member.logical_repository_id, member_one);
    assert_eq!(subtree_repository.id, "physical-f4");

    // ③ 成员不存在：两路皆失败 → 原始（uuid 子树）错误 fail-closed。
    let missing = LogicalRepositoryId(uuid::Uuid::from_u128(4242));
    assert!(
        resolve(&fixture.paths, &enrolled_lc, missing).is_err(),
        "nonexistent member must fail closed"
    );
    assert!(
        resolve(&subtree_paths, &lc_uuid, missing).is_err(),
    );
}
