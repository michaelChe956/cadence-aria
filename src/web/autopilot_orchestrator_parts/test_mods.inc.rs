// ---------------------------------------------------------------------------
// P1 WIGA Task 7：无 driver choice/人工门/compile recovery 的停等人对照。
// fixture 全部走真实面：enrollment PUT（P0 REST）→ `ensure_enrolled_plan`
//（Task 4 锁内唯一创建/绑定）→ manager 唯一 run 注册 + provider 等待者
// 消费 choice 应答回执；人工侧经 P0 REST 作答/批准，approve 由现有
// finalizer failpoint 在 compile 终结点失败——编排器必须全程只观察。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod task7_gates {
    use super::*;
    use crate::web::wiga_gate_fixture::*;

    #[tokio::test]
    async fn automation_reconcile_waits_for_choice_and_failed_compile() {
        let mut fixture = Box::new(EnrolledGateFixture::new().await);
        fixture.open_choice_with_two_questions().await;
        let before = fixture.provider_start_ledger();
        assert!(matches!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::AwaitingHuman
        ));
        assert_eq!(fixture.provider_start_ledger(), before);
        fixture.human_answer_via_rest().await;
        fixture.fail_compile_after_human_approve().await;
        assert!(matches!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::AwaitingHuman
        ));
        assert_eq!(fixture.provider_start_ledger(), before);
        // Abandon 后终态：编排器不复活、不追加 provider run。
        fixture.abandon_via_rest().await;
        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::NeedsHuman
        );
        assert_eq!(fixture.provider_start_ledger(), before);
    }
}

// ---------------------------------------------------------------------------
// P2 Task 5（tasks.md §3.1）：Confirmed plan 无 socket 独立 advance 到
// Ready，下一轮才 stable-id AutoStartOnce；重复唤醒不重启同一 attempt。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod p2_coding_chain {
    use super::*;
    use crate::web::state::CodingAttemptRunKey;
    use crate::web::wiga_gate_fixture::*;

    #[tokio::test]
    async fn confirmed_enrollment_advances_then_starts_without_coding_socket() {
        let fixture = confirmed_enrolled_fixture().await;
        let worker = AutopilotOrchestrator::new(fixture.state.clone(), Default::default());
        assert_eq!(
            worker.reconcile(&fixture.state, PROJECT_ID, ISSUE_ID).await.unwrap(),
            ReconcileOutcome::Advancing
        );
        let ready = fixture.coding_attempts().into_iter().next().unwrap();
        assert_eq!(
            fixture.state.coding_runs.runner_count(
                &CodingAttemptRunKey::from_attempt(&ready)
            ),
            0,
            "advance must not sneak provider start into the same transition"
        );
        assert_eq!(
            worker.reconcile(&fixture.state, PROJECT_ID, ISSUE_ID).await.unwrap(),
            ReconcileOutcome::Coding
        );
        assert_eq!(
            fixture
                .state
                .coding_runs
                .runner_count(&CodingAttemptRunKey::from_attempt(&ready)),
            1
        );
        // 重复唤醒：同 attempt 不重启（durable claim + registry 单飞）。
        assert_eq!(
            worker.reconcile(&fixture.state, PROJECT_ID, ISSUE_ID).await.unwrap(),
            ReconcileOutcome::Coding
        );

        assert_eq!(
            fixture.coding_attempts().into_iter().next().unwrap().id,
            ready.id
        );
        assert!(fixture
            .coding_attempts()
            .into_iter()
            .next()
            .unwrap()
            .start_claim
            .is_some());
    }


    /// D2：Confirmed 后 disable 先胜——未消费的 AutoStartOnce/advance 许可
    /// 不再生效，零 provider。
    #[tokio::test]
    async fn disabled_after_confirm_yields_no_enrollment_before_any_start() {
        let fixture = confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        crate::product::issue_automation_store::IssueAutomationStore::new(
            fixture.inner.paths.clone(),
        )
        .compare_and_set(
            PROJECT_ID,
            ISSUE_ID,
            Some(enrollment.policy_revision),
            crate::product::models::automation::EnrollmentWriteCommand::Disable,
        )
        .expect("disable enrollment");
        let worker = AutopilotOrchestrator::new(fixture.state.clone(), Default::default());
        assert_eq!(
            worker.reconcile(&fixture.state, PROJECT_ID, ISSUE_ID).await.unwrap(),
            ReconcileOutcome::NoEnrollment
        );
        assert!(fixture.coding_attempts().is_empty());
        assert_eq!(fixture.coding_runner_count(), 0);
    }
}

// ---------------------------------------------------------------------------
// P2 Task 10（tasks.md §3.4）：跨层 campaign——人工批准 Confirmed plan 后，
// 无页面 reconcile 独立 advance→单发首启→Fake runner 后台跑到 durable
// `WaitingForHuman ∧ FinalConfirm`；编排器不代点，人手 handle_final_confirm
// 才 Completed。503 失败分诊单独成证（GAP-E/G/H：durable 诊断 + 零自动重驱）。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod p2_campaign {
    use super::*;
    use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionStage};
    use crate::web::wiga_gate_fixture::*;

    #[tokio::test]
    async fn p2_campaign_requires_human_final_confirm_after_socketless_run() {
        let mut fixture = p2_enrolled_campaign_fixture().await;
        fixture.confirm_plan_by_human().await;
        fixture.reconcile_until_coding_waiting_for_human().await;
        let attempt = fixture.attempt();
        assert_eq!(attempt.stage, CodingExecutionStage::FinalConfirm);
        assert_eq!(attempt.status, CodingAttemptStatus::WaitingForHuman);
        assert_eq!(fixture.runner_start_claims(&attempt.id), 1);
        // 等待期间编排器只观察（人工 Final Confirm 不被代点、不隐式重驱）。
        let worker = AutopilotOrchestrator::new(fixture.gate.state.clone(), Default::default());
        assert_eq!(
            worker
                .reconcile(&fixture.gate.state, PROJECT_ID, ISSUE_ID)
                .await
                .unwrap(),
            ReconcileOutcome::AwaitingHuman
        );
        assert_eq!(fixture.attempt().status, CodingAttemptStatus::WaitingForHuman);
        fixture.gate.confirm_final_by_human().await;
        assert_eq!(fixture.attempt().status, CodingAttemptStatus::Completed);
        assert_eq!(fixture.runner_start_claims(&attempt.id), 1);
    }

    /// GAP-E/G/H campaign 侧：reviewer 网关 503 失败 durable 收敛 + 可见诊断，
    /// 编排器分诊 NeedsHuman 且重复唤醒零重驱（provider 账目不增）。
    #[tokio::test]
    async fn p2_campaign_gateway_503_reviewer_failure_is_human_triage_without_redrive() {
        let mut fixture = p2_enrolled_campaign_fixture().await;
        let failed_node_id = fixture.gate.fail_reviewer_with_gateway_503().await;
        // GAP-H：分类诊断落 durable 节点摘要，脱敏（不泄凭据）。
        let summary = fixture
            .gate
            .timeline_nodes()
            .iter()
            .find(|node| node.node_id == failed_node_id)
            .unwrap()
            .summary
            .clone()
            .unwrap_or_default();
        assert!(
            summary.contains("provider_gateway_503_no_accounts"),
            "503 diagnostic must be durable: {summary}"
        );
        assert!(!summary.contains("secret"), "diagnostic must redact credentials");
        let before = fixture.gate.provider_start_ledger();
        let worker = AutopilotOrchestrator::new(fixture.gate.state.clone(), Default::default());
        for _ in 0..3 {
            assert_eq!(
                worker
                    .reconcile(&fixture.gate.state, PROJECT_ID, ISSUE_ID)
                    .await
                    .unwrap(),
                ReconcileOutcome::NeedsHuman
            );
        }
        assert_eq!(
            fixture.gate.provider_start_ledger(),
            before,
            "repeated reconcile must not re-drive the failed reviewer"
        );
        assert_eq!(fixture.manual_issue_runner_start_claims(), 0);
    }
}

// ---------------------------------------------------------------------------
// C5 Task 8（tasks.md 5.1）：A01/A02 端到端联验。A02 主链＝单仓自动全链
// （人确认 design 后 Enable → 编排器 reconcile 补偿 prepare（Legacy 分支）
// → 生成 → 人工门批准 → enrolled advance（Task 4 双分支）→ typed 首启
// （Task 5 互证）→ coding → `WaitingForHuman ∧ FinalConfirm` 停等）；
// A01 手动对照＝未 Enable 单仓 issue 补偿扫描零动作。A05 恢复闭环由
// `repository_initialization_resume_http_roundtrip_recovers_after_gateway_outage`
// （tests/it_web）承载；A10 由 Task 3 实名（kimi_code_and_pi_are_statically_
// rejected 等）与 p2_campaign 503 实名复验承载。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod c5_acceptance {
    use super::*;
    use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionStage};
    use crate::web::wiga_gate_fixture::*;

    #[tokio::test]
    async fn c5_acceptance_single_repository_chain_reaches_final_confirm() {
        let mut fixture = c5_single_repository_campaign_fixture().await;

        // A01：未 Enable 的单仓 manual issue——补偿扫描零动作（无自动
        // plan、无首启 claim；reconcile 对其返回 NoEnrollment）。
        assert_eq!(fixture.manual_issue_auto_plans(), 0);
        assert_eq!(fixture.manual_issue_runner_start_claims(), 0);

        // A02 主链：人工门批准（模拟驾驶舱 Approve）后，仅靠无页面
        // reconcile 推进到 FinalConfirm 停等（coding socket 关闭态驱动）。
        fixture.confirm_plan_by_human().await;
        fixture.reconcile_until_coding_waiting_for_human().await;
        let attempt = fixture.attempt();
        assert_eq!(attempt.stage, CodingExecutionStage::FinalConfirm);
        assert_eq!(attempt.status, CodingAttemptStatus::WaitingForHuman);
        // 全程 provider 首启恰一次（durable 单发 claim 不可复位）。
        assert_eq!(fixture.runner_start_claims(&attempt.id), 1);
        // 单仓红线：attempt 无 logical target snapshot，全链未创建任何
        // LC manifest/selection/checkout 存储（无 gateway 强制面——gateway
        // 仅对带 snapshot 的 attempt 生效，Task 5 互证实名测试守护）。
        assert!(attempt.target_snapshot.is_none());
        assert!(
            !crate::product::logical_codebase::LogicalCodebaseStore::new(
                fixture.gate.inner.paths.clone()
            )
            .has_any_storage(PROJECT_ID)
            .expect("logical codebase storage probe"),
            "single-repository chain must not create LC manifest/selection/snapshot"
        );

        // 停等期间编排器只观察：FinalConfirm 不被代点、不隐式重驱。
        let worker = AutopilotOrchestrator::new(fixture.gate.state.clone(), Default::default());
        assert_eq!(
            worker
                .reconcile(&fixture.gate.state, PROJECT_ID, ISSUE_ID)
                .await
                .unwrap(),
            ReconcileOutcome::AwaitingHuman
        );
        assert_eq!(
            fixture.attempt().status,
            CodingAttemptStatus::WaitingForHuman
        );
        assert_eq!(fixture.runner_start_claims(&attempt.id), 1);

        // 人手 FinalConfirm → 原链继续至 Completed（首启账目不增）。
        fixture.gate.confirm_final_by_human().await;
        assert_eq!(fixture.attempt().status, CodingAttemptStatus::Completed);
        assert_eq!(fixture.runner_start_claims(&attempt.id), 1);

        // A01（收口）：全链结束后补偿扫描对 manual issue 仍零动作。
        assert_eq!(fixture.manual_issue_auto_plans(), 0);
        assert_eq!(fixture.manual_issue_runner_start_claims(), 0);
    }
}


// ---------------------------------------------------------------------------
// G8（终局关闸缺口）：enrolled 自动链 generate 相位委托返修接力孤儿重驱。
// 现场（issue_0005）：review 裁决返修 → TriggerAggregateRepair 原子预领
// （repair_reservation=Reserved＋ledger 追加）→ 进程重启，接力 spawn 未
// 落地 → manager 重建回落 generate 相位人工门（无快照无轮次）→ REST/WS
// 批准面按相位门语义 fail-closed（409 正确），编排器 admission
// WaitingForHuman 恒不作答＝自动链死锁。Reserved 未被接力消费＝provider
// 可证明未启动：编排器重驱 WorkItemPlanSingleCandidateAuthor，预留 CAS
//（Reserved→ProviderStarted 恰一次）保证不二次启动。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod g8_delegated_orphan {
    use super::*;
    use crate::product::models::SingleCandidatePhase;
    use crate::product::models::WorkspaceSessionStatus;
    use crate::product::work_item_plan_policy::{
        ProviderStartLedgerEntry, RepairReservation, RepairReservationState,
    };
    use crate::web::handlers::automation_enrollment_test_support::{
        ISSUE_ID, PROJECT_ID, enrollment_body, put_enrollment, response_json, seed_fixture,
    };

    fn seed_delegated_orphan_record(paths: &crate::product::app_paths::ProductAppPaths) -> String {
        let store = IssueAutomationStore::new(paths.clone());
        let session_id = store
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .expect("enrollment bound")
            .session_id
            .expect("bound session");
        let session_path = paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("workspace-sessions")
            .join(format!("{session_id}.json"));
        let mut session: crate::product::models::WorkspaceSessionRecord =
            crate::product::json_store::read_json(&session_path).unwrap();
        // 现场 orphan 形态（issue_0005/auto_a2738e2b 落盘口径）：初代已启动、
        // 返修预领 Reserved 未消费、相位 Generate、durable Running
        //（重建 manager 后由 F2 恢复回落人工门）。
        session.single_candidate_phase = Some(SingleCandidatePhase::Generate);
        session.status = WorkspaceSessionStatus::Running;
        session.provider_start_ledger = vec![
            ProviderStartLedgerEntry {
                provider_start_idempotency_key: format!("single_candidate_author:{session_id}:0"),
                started: true,
                provider: None,
                started_at: None,
            },
            ProviderStartLedgerEntry {
                provider_start_idempotency_key: format!("single_candidate_author:{session_id}:1"),
                started: true,
                provider: None,
                started_at: None,
            },
        ];
        session.repair_reservation = Some(RepairReservation {
            token: format!("single_candidate_author_repair:{session_id}:1"),
            owner_session_id: session_id.clone(),
            owner_run_id: "review_scope_v1:g8-fixture".to_string(),
            provider_start_idempotency_key: format!("single_candidate_author:{session_id}:1"),
            state: RepairReservationState::Reserved,
            commit_id: None,
        });
        crate::product::json_store::write_json(&session_path, &session).unwrap();
        session_id
    }

    fn durable_reservation_state(
        paths: &crate::product::app_paths::ProductAppPaths,
        session_id: &str,
    ) -> RepairReservationState {
        let session_path = paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("workspace-sessions")
            .join(format!("{session_id}.json"));
        let session: crate::product::models::WorkspaceSessionRecord =
            crate::product::json_store::read_json(&session_path).unwrap();
        session
            .repair_reservation
            .expect("reservation persists")
            .state
    }

    #[tokio::test]
    async fn enrolled_delegated_rerun_orphan_is_redriven_exactly_once_by_reconcile() {
        // 与 p1 四中窗测试互斥（本测试驱动 ensure_enrolled_plan/reconcile，
        // 并行会互偷进程级 crash-window 注册）。
        let _crash_window_serial = super::CRASH_WINDOW_SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let inner = seed_fixture(1, true);
        let root_path = inner._root.path().to_path_buf();
        let state = WebAppState::new(
            root_path.clone(),
            crate::web::runtime::WebRuntime::new_fake(root_path.clone()),
        );
        let app = crate::web::app::build_web_router(state.clone());
        let enable = put_enrollment(&app, enrollment_body(&inner, 1, 1)).await;
        assert_eq!(enable.status(), axum::http::StatusCode::OK);
        assert!(response_json(enable).await["enabled"].as_bool().unwrap());
        let store = IssueAutomationStore::new(inner.paths.clone());
        let enrollment = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        crate::web::handlers::lifecycle::plan_preparation::ensure_enrolled_plan(
            &state,
            &enrollment,
        )
        .await
        .expect("ensure enrolled plan");
        let session_id = seed_delegated_orphan_record(&inner.paths);

        // 重启替身：全新 state（无 manager/无 run），编排器按 durable 事实分诊。
        let restart = WebAppState::new(
            root_path.clone(),
            crate::web::runtime::WebRuntime::new_fake(root_path.clone()),
        );
        let worker = AutopilotOrchestrator::new(
            restart.clone(),
            OrchestratorConfig {
                max_issues_per_tick: 32,
                ..Default::default()
            },
        );
        let outcome = worker.reconcile(&restart, PROJECT_ID, ISSUE_ID).await.unwrap();
        assert_eq!(outcome, ReconcileOutcome::Generating);

        // 预留 CAS 被接力消费：Reserved → ProviderStarted（provider 启动
        // 判据；ledger 不追加新代＝不二次启动）。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if matches!(
                durable_reservation_state(&inner.paths, &session_id),
                RepairReservationState::ProviderStarted
            ) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "delegated rerun reservation was never consumed by the re-drive"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let session = crate::product::lifecycle_store::LifecycleStore::new(inner.paths.clone())
            .get_workspace_session(&session_id)
            .unwrap();
        assert_eq!(session.provider_start_ledger.len(), 2);

        // #10 语义（终态失败 run 释放注册，3be1aa49）：重驱 run 以 Message
        // 终态失败退场后必须释放 manager 注册；无附着的 manager 随
        // finish_run 自回收出 registry。等释放可观察后再断言后续分诊，
        // 避免与 run 任务尾部 finish_run 竞态。
        let release_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let released = restart
                .workspace_sessions
                .peek(&session_id)
                .await
                .is_none_or(|manager| !manager.is_active_run());
            if released {
                break;
            }
            assert!(
                std::time::Instant::now() < release_deadline,
                "failed re-drive run never released its registration"
            );
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        // 预留已消费（ProviderStarted）后不再重驱（Reserved 判据幂等收口；
        // 恰一次仍由下方 ledger 断言保证）。期望值按 #10 新语义重钉：终态
        // 失败 run 已释放注册，二次 reconcile 无活 run——旧断言 Generating
        // 依赖的正是泄漏注册造成的 AlreadyActive 幻象（is_active_run 把死
        // run 误报在途）；durable 现状＝phase Generate＋终态失败回落 status
        // Open，孤儿判据不再命中，admission 停等人工 → AwaitingHuman。
        let second = worker.reconcile(&restart, PROJECT_ID, ISSUE_ID).await.unwrap();
        assert_eq!(second, ReconcileOutcome::AwaitingHuman);
        let session_after = crate::product::lifecycle_store::LifecycleStore::new(
            inner.paths.clone(),
        )
        .get_workspace_session(&session_id)
        .unwrap();
        assert_eq!(session_after.provider_start_ledger.len(), 2);
    }
}
