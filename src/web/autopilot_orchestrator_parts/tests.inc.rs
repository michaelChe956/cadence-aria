    use axum::http::StatusCode;

    use super::*;
    use crate::web::handlers::automation_enrollment_test_support::{
        ISSUE_ID, PROJECT_ID, enrollment_body, put_enrollment, response_json, seed_fixture,
    };
    use crate::web::state::WebAppState;

    struct OrchestratorFixture {
        inner: crate::web::handlers::automation_enrollment_test_support::Fixture,
        state: WebAppState,
        manual_issue_id: String,
    }

    impl OrchestratorFixture {
        /// 真实播种：project + issue + 单成员 logical codebase + 已确认
        /// story/design + enabled enrollment；另加一个未授权手工 issue。
        async fn new() -> Self {
            let inner = seed_fixture(1, true);
            let root_path = inner._root.path().to_path_buf();
            let state = WebAppState::new(
                root_path.clone(),
                crate::web::runtime::WebRuntime::new_fake(root_path),
            );
            let app = crate::web::app::build_web_router(state.clone());
            let enable = put_enrollment(&app, enrollment_body(&inner, 1, 1)).await;
            assert_eq!(enable.status(), StatusCode::OK);
            assert!(response_json(enable).await["enabled"].as_bool().unwrap());

            // 未授权手工对照 issue（同 project）。
            let manual = crate::product::issue_store::IssueStore::new(inner.paths.clone())
                .create(crate::product::issue_store::CreateProductIssueInput {
                    project_id: PROJECT_ID.to_string(),
                    repo_id: Some("repo-1".to_string()),
                    // 归属必填（REQ-ROUTE-01）：夹具经 seed_fixture 播种 project 级
                    // LC（legacy 布局）⇒ issue 归属 project 的 legacy 别名代码库。
                    logical_codebase_id: crate::product::logical_codebase::store::legacy_logical_codebase_id(
                        PROJECT_ID,
                    ),
                    title: "manual issue".to_string(),
                    description: None,
                    change_id: None,
                    base_branch: None,
                })
                .unwrap();
            Self {
                inner,
                state,
                manual_issue_id: manual.id,
            }
        }

        fn lifecycle(&self) -> crate::product::lifecycle_store::LifecycleStore {
            crate::product::lifecycle_store::LifecycleStore::new(self.inner.paths.clone())
        }

        fn bound_plans(&self) -> Vec<crate::product::models::IssueWorkItemPlan> {
            self.lifecycle()
                .list_issue_work_item_plans(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .into_iter()
                .filter(|plan| plan.id.starts_with("issue_work_item_plan_auto_"))
                .collect()
        }

        fn manual_issue_plans(&self) -> Vec<crate::product::models::IssueWorkItemPlan> {
            self.lifecycle()
                .list_issue_work_item_plans(PROJECT_ID, &self.manual_issue_id)
                .unwrap()
        }

        fn sessions(&self) -> Vec<crate::product::models::WorkspaceSessionRecord> {
            self.lifecycle()
                .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
                .unwrap()
        }

        /// 绑定 plan 名下的 session（无绑定时为空——崩溃现场观察用）。
        fn sessions_for_bound_plan(&self) -> Vec<crate::product::models::WorkspaceSessionRecord> {
            let binding = self
                .automation_store()
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap();
            match binding.plan_id {
                Some(plan_id) => self
                    .sessions()
                    .into_iter()
                    .filter(|session| session.entity_id == plan_id)
                    .collect(),
                None => Vec::new(),
            }
        }

        /// 绑定 plan session 的 durable provider 启动账目条数。
        fn provider_start_count(&self) -> usize {
            let binding = self
                .automation_store()
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap();
            match binding.session_id {
                Some(session_id) => self
                    .lifecycle()
                    .get_workspace_session(&session_id)
                    .unwrap()
                    .provider_start_ledger
                    .len(),
                None => 0,
            }
        }

        /// 绑定自动化 session 的 durable 记录（重启后 RunPolicy 断言用）。
        fn automation_session(&self) -> crate::product::models::WorkspaceSessionRecord {
            let session_id = self
                .automation_store()
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap()
                .session_id
                .expect("bound automation session");
            self.lifecycle().get_workspace_session(&session_id).unwrap()
        }

        /// 「重启」：全新 WebAppState/runtime（进程替身）——内存态清零，
        /// 只保留磁盘 durable 事实。
        fn restart_state(&self) -> WebAppState {
            let root = self.inner._root.path().to_path_buf();
            WebAppState::new(
                root.clone(),
                crate::web::runtime::WebRuntime::new_fake(root),
            )
        }

        /// 重启后两轮有界扫描补偿 + 一轮定向分诊，返回绑定 issue 的最终
        /// reconcile 结果。
        async fn restart_and_reconcile(&self) -> ReconcileOutcome {
            let state = self.restart_state();
            let worker = AutopilotOrchestrator::new(
                state.clone(),
                OrchestratorConfig {
                    max_issues_per_tick: 32,
                    ..Default::default()
                },
            );
            worker.reconcile_all_once().await.expect("restart scan 1");
            worker.reconcile_all_once().await.expect("restart scan 2");
            worker
                .reconcile(&state, PROJECT_ID, ISSUE_ID)
                .await
                .expect("restart targeted reconcile")
        }

        /// 等待重启后派发的 provider run 把启动账目落盘（reserve 先于任何
        /// provider 交互，有界轮询即可）。
        async fn await_provider_start(&self) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while self.provider_start_count() == 0 {
                assert!(
                    std::time::Instant::now() < deadline,
                    "provider start ledger never persisted after restart"
                );
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }

        fn automation_store(&self) -> IssueAutomationStore {
            IssueAutomationStore::new(self.inner.paths.clone())
        }

        fn worker(&self) -> AutopilotOrchestrator {
            AutopilotOrchestrator::new(
                self.state.clone(),
                OrchestratorConfig {
                    max_issues_per_tick: 32,
                    ..Default::default()
                },
            )
        }
    }

    #[tokio::test]
    async fn automation_reconcile_scans_only_enrolled_issues_and_reuses_binding() {
        let fixture = OrchestratorFixture::new().await;
        let worker = fixture.worker();
        // 关闭 browser/WS、不送任何事件：两轮补偿只依赖 durable 事实。
        worker.reconcile_all_once().await.unwrap();
        worker.reconcile_all_once().await.unwrap();

        let bound = fixture
            .automation_store()
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        assert!(bound.plan_id.is_some());
        assert!(bound.session_id.is_some());
        assert_eq!(fixture.bound_plans().len(), 1);
        assert!(fixture.manual_issue_plans().is_empty());
        // 绑定 session 恰一且 Interactive。
        let sessions = fixture.sessions();
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].run_policy,
            crate::product::work_item_plan_policy::RunPolicy::Interactive
        );
    }

    /// Disable 先于动作认领：零启动、零 plan。
    #[tokio::test]
    async fn automation_reconcile_ignores_disabled_enrollment() {
        let fixture = OrchestratorFixture::new().await;
        let worker = fixture.worker();
        let app = crate::web::app::build_web_router(fixture.state.clone());
        let revision = fixture
            .automation_store()
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap()
            .policy_revision;
        let disable = serde_json::json!({
            "expected_revision": revision,
            "command": {"type": "disable"}
        });
        let response = put_enrollment(&app, disable).await;
        assert_eq!(response.status(), StatusCode::OK);

        let outcome = worker
            .reconcile(&fixture.state, PROJECT_ID, ISSUE_ID)
            .await
            .unwrap();
        assert_eq!(outcome, ReconcileOutcome::NoEnrollment);
        assert!(fixture.bound_plans().is_empty());
    }

    /// 换源重开（保留旧绑定）：fail-closed NeedsHuman，不自动生成、不另建链。
    #[tokio::test]
    async fn automation_reconcile_rejects_divergent_reopen_as_needs_human() {
        let fixture = OrchestratorFixture::new().await;
        let worker = fixture.worker();
        worker
            .reconcile(&fixture.state, PROJECT_ID, ISSUE_ID)
            .await
            .unwrap();
        assert_eq!(fixture.bound_plans().len(), 1);

        let app = crate::web::app::build_web_router(fixture.state.clone());
        let revision = fixture
            .automation_store()
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap()
            .policy_revision;
        let disable = serde_json::json!({
            "expected_revision": revision,
            "command": {"type": "disable"}
        });
        assert_eq!(put_enrollment(&app, disable).await.status(), StatusCode::OK);
        let mut divergent = enrollment_body(&fixture.inner, 1, 1);
        divergent["command"]["options"]["review_rounds"] = serde_json::json!(2);
        divergent["expected_revision"] = serde_json::json!(revision + 1);
        assert_eq!(
            put_enrollment(&app, divergent).await.status(),
            StatusCode::OK
        );

        let outcome = worker
            .reconcile(&fixture.state, PROJECT_ID, ISSUE_ID)
            .await
            .unwrap();
        assert_eq!(outcome, ReconcileOutcome::NeedsHuman);
        assert_eq!(fixture.bound_plans().len(), 1);
    }

    /// P1 WIGA Task 10（2.4）：四中窗一次性中断——进程在 intent/plan/
    /// session 落盘后、检查点 EngineStarted 落盘后崩溃，重启（全新
    /// state/runtime）只凭 durable 事实补偿后：恰一绑定 plan/session、
    /// provider 绝不重发（可证明窗口恰一次派发；EngineStarted 后不可证明
    /// 则 fail-closed 人工分诊零派发）、RunPolicy 恒 Interactive
    /// （REQ-WIGA-03、REQ-WIGA-02）。
    #[tokio::test]
    async fn automation_p1_crash_windows_keep_one_plan_and_never_reissue_provider() {
        use crate::product::issue_automation_store::automation_crash_window::{self, CrashWindow};

        // crash-window 注入是进程级全局：与同样驱动 ensure_enrolled_plan /
        // reconcile 的 g8_delegated_orphan 测试互斥（并行会互偷窗口）。
        let _crash_window_serial = super::CRASH_WINDOW_SERIAL.lock().await;

        for window in [
            CrashWindow::AfterIntentSaved,
            CrashWindow::AfterPlanSaved,
            CrashWindow::AfterSessionSaved,
            CrashWindow::AfterEngineStarted,
        ] {
            let fixture = OrchestratorFixture::new().await;
            // 「进程」在窗口处崩溃：链路以错误中止，durable 停在中窗。
            let interrupt = automation_crash_window::register(window);
            let message = fixture
                .worker()
                .reconcile(&fixture.state, PROJECT_ID, ISSUE_ID)
                .await
                .expect_err("interrupt must abort the in-flight pass");
            assert!(
                message.contains("ensure enrolled plan failed")
                    || message.contains("automation_crash_window"),
                "{window:?}: crash must abort mid-chain, got: {message}"
            );
            drop(interrupt); // 崩溃即进程消失（含未触发的注册）。

            // 崩溃现场逐窗核对（durable 事实，不带内存态）。
            let binding_after_crash = fixture
                .automation_store()
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap();
            match window {
                CrashWindow::AfterIntentSaved => {
                    assert!(binding_after_crash.plan_id.is_none());
                    assert!(fixture.bound_plans().is_empty());
                    assert!(fixture.sessions().is_empty());
                }
                CrashWindow::AfterPlanSaved => {
                    assert!(binding_after_crash.plan_id.is_none(), "绑定未写回");
                    assert_eq!(fixture.bound_plans().len(), 1, "plan 已落盘");
                    assert!(
                        fixture.sessions_for_bound_plan().is_empty(),
                        "session 尚未创建"
                    );
                }
                CrashWindow::AfterSessionSaved => {
                    assert!(binding_after_crash.plan_id.is_none(), "绑定未写回");
                    let plans = fixture.bound_plans();
                    assert_eq!(plans.len(), 1);
                    let orphan_plan_id = plans[0].id.clone();
                    assert_eq!(
                        fixture
                            .sessions()
                            .into_iter()
                            .filter(|session| session.entity_id == orphan_plan_id)
                            .count(),
                        1,
                        "session 已落盘但绑定未写回"
                    );
                }
                CrashWindow::AfterEngineStarted => {
                    assert!(binding_after_crash.plan_id.is_some());
                    assert_eq!(fixture.bound_plans().len(), 1);
                    assert_eq!(fixture.sessions_for_bound_plan().len(), 1);
                    assert_eq!(fixture.provider_start_count(), 0, "崩溃先于 provider 派发");
                }
            }

            // 重启：全新 state/runtime 只凭 durable 事实补偿。
            let outcome = fixture.restart_and_reconcile().await;
            assert_eq!(fixture.bound_plans().len(), 1, "{window:?}: 恰一 plan");
            assert_eq!(
                fixture.sessions_for_bound_plan().len(),
                1,
                "{window:?}: 恰一 session"
            );
            assert_eq!(
                fixture.automation_session().run_policy,
                crate::product::work_item_plan_policy::RunPolicy::Interactive,
                "{window:?}: 绑定 session 恒 Interactive"
            );
            match window {
                // 可证明 provider 未被触达的窗口：重启后正常恰一次派发。
                CrashWindow::AfterIntentSaved
                | CrashWindow::AfterPlanSaved
                | CrashWindow::AfterSessionSaved => {
                    fixture.await_provider_start().await;
                    assert_eq!(
                        fixture.provider_start_count(),
                        1,
                        "{window:?}: 恰一次 provider 启动"
                    );
                }
                // 越过 EngineStarted 后崩溃：无法证明 provider 未被触达，
                // fail-closed 人工分诊，绝不重发。
                CrashWindow::AfterEngineStarted => {
                    assert_eq!(outcome, ReconcileOutcome::NeedsHuman);
                    assert_eq!(
                        fixture.provider_start_count(),
                        0,
                        "{window:?}: 不可证明时不重发 provider"
                    );
                    // 再次「重启」多轮补偿：仍不重发。
                    let again = fixture.restart_and_reconcile().await;
                    assert_eq!(again, ReconcileOutcome::NeedsHuman);
                    assert_eq!(fixture.provider_start_count(), 0);
                }
            }
        }
    }

    // ─── 批 2 R2（REQ-WIGA-03/C5-CHAIN、REQ-MTG-03、REQ-WIGA-12）───
    // LC（单成员/集合）绑定改走 plan_start 共享服务：先落 CodingPlanRun
    // 再开独立根、首启沿 deterministic_start_command_id（删 wiga-start
    // 旁路）、Enrolled origin、tick 同 claim 幂等；advance/coding_start
    // 失败写 N0 automation_failed 事实且未解除即停点。

    /// R2 ①②④：单成员 LC 绑定——tick1 先落 run（Advancing、零 runner），
    /// tick2 经共享服务 Enrolled 首启（claim origin=Enrolled、command 为
    /// run.command_id 的确定性派生而非 wiga-start 旁路、单桶物化恰一条
    /// target 绑定），tick3 同 claim 零重启。
    #[tokio::test]
    async fn r2_lc_chain_creates_run_first_and_starts_enrolled_origin() {
        use crate::product::coding_plan_run::{
            CodingPlanRunPhase, CodingPlanRunStore, CodingPlanSelection,
        };

        let fixture = crate::web::wiga_gate_fixture::confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let plan_id = enrollment.plan_id.clone().expect("bound plan");

        // tick1：advance 到 Ready 即止——run 已先落盘、零 runner。
        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::Advancing
        );
        let run_store = CodingPlanRunStore::new(fixture.inner.paths.clone());
        let run = run_store
            .get(PROJECT_ID, ISSUE_ID, &plan_id)
            .unwrap()
            .expect("coding plan run persisted before any root opens");
        assert_eq!(
            run.command_id,
            format!("wiga-advance-{}-{plan_id}", enrollment.enrollment_id),
            "run command uses the stable enrollment/plan identity"
        );
        assert_eq!(run.phase, CodingPlanRunPhase::Prepared);
        assert_eq!(fixture.coding_runner_count(), 0);

        // tick2：共享服务 bind→mark→Enrolled start→record。
        assert_eq!(fixture.reconcile().await.unwrap(), ReconcileOutcome::Coding);
        let run = run_store
            .get(PROJECT_ID, ISSUE_ID, &plan_id)
            .unwrap()
            .expect("run survives the start tick");
        let attempt = fixture.attempt();
        // 单实际桶：引擎空 target_attempts，由共享服务物化恰一条绑定。
        assert_eq!(run.target_attempts.len(), 1);
        assert_eq!(
            run.target_attempts[0].attempt_id, attempt.id,
            "materialized binding carries the real single-bucket attempt"
        );
        let claim = attempt.start_claim.as_ref().expect("enrolled first start");
        assert_eq!(claim.origin, fixture.auto_origin());
        let expected_command = CodingPlanSelection::deterministic_start_command_id(
            PROJECT_ID,
            ISSUE_ID,
            &plan_id,
            &run.command_id,
            &run.target_attempts[0].target_repository_id,
        );
        assert_eq!(
            claim.command_id, expected_command,
            "first start follows the deterministic command; the wiga-start bypass is gone"
        );
        assert_ne!(claim.command_id, format!("wiga-start-{}", attempt.id));
        assert_eq!(fixture.runner_count(&fixture.attempt_key()), 1);

        // tick3：重复唤醒同 claim、零重启。
        assert_eq!(fixture.reconcile().await.unwrap(), ReconcileOutcome::Coding);
        assert_eq!(fixture.runner_count(&fixture.attempt_key()), 1);
        let replay = fixture.attempt();
        assert_eq!(
            replay.start_claim.as_ref().unwrap().command_id,
            expected_command
        );
    }

    /// R2 ⑥：advance 链失败（plan 进入在途 amendment → 共享服务 prepare
    /// fail-closed）写 N0 `automation_failed(stage=advance)` 事实；未解除
    /// 事实令后续 tick 停在该点（不自动 retry）、同 key 不增项、
    /// occurred_at 首次冻结。
    #[tokio::test]
    async fn r2_advance_failure_writes_fact_and_gates_following_ticks() {
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::models::automation::{
            AutomationAttentionKind, AutomationAttentionStage,
        };
        use crate::product::work_item_revision_store::WorkItemRevisionStore;

        let fixture = crate::web::wiga_gate_fixture::confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let plan_id = enrollment.plan_id.clone().expect("bound plan");
        // 篡改现场：plan 出现在途 amendment（durable lineage 事实）——
        // Confirmed session/compile 投影不受影响，共享服务 prepare
        // fail-closed（coding_plan_run_active_amendment）。
        let revision_store = WorkItemRevisionStore::new(fixture.inner.paths.clone());
        let mut lineage = revision_store
            .get_plan_lineage(PROJECT_ID, ISSUE_ID, &plan_id)
            .unwrap();
        lineage.active_amendment_id = Some("amendment_r2_tamper".to_string());
        crate::product::json_store::write_json(
            &fixture
                .inner
                .paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("work-item-revisions")
                .join(&plan_id)
                .join("lineage.json"),
            &lineage,
        )
        .unwrap();

        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::NeedsHuman
        );

        let store = IssueAutomationStore::new(fixture.inner.paths.clone());
        let saved = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        let command_id = format!("wiga-advance-{}-{plan_id}", enrollment.enrollment_id);
        let expected_key = format!("{plan_id}:{command_id}:advance_failed");
        let fact = saved
            .attention_facts
            .iter()
            .find(|fact| fact.key == expected_key)
            .unwrap_or_else(|| panic!("advance failure fact missing: {:?}", saved.attention_facts));
        assert_eq!(fact.kind, AutomationAttentionKind::AutomationFailed);
        assert_eq!(fact.stage, AutomationAttentionStage::Advance);
        assert_eq!(fact.plan_id.as_deref(), Some(plan_id.as_str()));
        assert_eq!(fact.session_id.as_deref(), enrollment.session_id.as_deref());
        assert!(!fact.reason_code.is_empty());
        let frozen_at = fact.occurred_at.clone();
        assert!(
            fixture.coding_attempts().is_empty(),
            "failed advance must not create attempts"
        );

        // 后续 tick：未解除事实停点（不自动 retry）、同 key 单条、时间冻结。
        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::NeedsHuman
        );
        let saved = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        let matches: Vec<_> = saved
            .attention_facts
            .iter()
            .filter(|fact| fact.key == expected_key)
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].occurred_at, frozen_at);
        assert!(fixture.coding_attempts().is_empty());
    }

    /// R2 ③：Ready 后冻结策略被篡改为 Manual（不给旧 Manual 原地补授权）
    /// ——共享服务仍以 Enrolled 身份准入，fail-closed 零 provider 并写
    /// coding_start 失败事实。
    #[tokio::test]
    async fn r2_tampered_manual_policy_rejects_start_without_downgrade() {
        use crate::product::coding_models::CodingStartRunPolicy;
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::models::automation::{
            AutomationAttentionKind, AutomationAttentionStage,
        };

        let fixture = crate::web::wiga_gate_fixture::confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let plan_id = enrollment.plan_id.clone().expect("bound plan");
        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::Advancing
        );
        // 篡改：把 Ready attempt 的冻结策略改写为 Manual。
        let attempt = fixture.attempt();
        let mut tampered = attempt.clone();
        tampered.start_run_policy = CodingStartRunPolicy::Manual;
        fixture
            .store()
            .write_coding_attempt_for_test(&tampered)
            .unwrap();

        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::NeedsHuman
        );
        assert_eq!(
            fixture.runner_count(&fixture.attempt_key()),
            0,
            "tampered policy must yield zero provider and no manual downgrade"
        );
        let saved = IssueAutomationStore::new(fixture.inner.paths.clone())
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        assert!(
            saved.attention_facts.iter().any(|fact| {
                fact.kind == AutomationAttentionKind::AutomationFailed
                    && fact.stage == AutomationAttentionStage::CodingStart
                    && fact.plan_id.as_deref() == Some(plan_id.as_str())
            }),
            "coding_start failure fact must be persisted: {:?}",
            saved.attention_facts
        );
    }

    /// k3 波3 fix-2：N0 事实写失败（enrollment 文件不可写→upsert Store
    /// 错误）不得吞成静默 NeedsHuman——该 tick 必须以 Err 产出可见诊断
    ///（automation_attention_fact_write_failed）；恢复可写后同一现场
    /// 正常写事实并按停点分诊。
    #[tokio::test]
    async fn r2_attention_fact_write_failure_surfaces_as_tick_error() {
        use crate::product::work_item_revision_store::WorkItemRevisionStore;

        let fixture = crate::web::wiga_gate_fixture::confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let plan_id = enrollment.plan_id.clone().expect("bound plan");
        // 同 r2_advance_failure 用例的在途 amendment 注入：prepare fail-closed
        // → 服务内尝试写 advance 失败事实。
        let revision_store = WorkItemRevisionStore::new(fixture.inner.paths.clone());
        let mut lineage = revision_store
            .get_plan_lineage(PROJECT_ID, ISSUE_ID, &plan_id)
            .unwrap();
        lineage.active_amendment_id = Some("amendment_r2_k3".to_string());
        crate::product::json_store::write_json(
            &fixture
                .inner
                .paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("work-item-revisions")
                .join(&plan_id)
                .join("lineage.json"),
            &lineage,
        )
        .unwrap();
        // N0 写入口不可写（issue 目录只读→锁文件/落盘 IO 失败形态；
        // write_json 为临时文件+rename，单文件 chmod 拦不住）。
        let enrollment_dir = fixture.inner.paths.issue_root(PROJECT_ID, ISSUE_ID);
        let mut readonly = std::fs::metadata(&enrollment_dir).unwrap().permissions();
        readonly.set_readonly(true);
        std::fs::set_permissions(&enrollment_dir, readonly).unwrap();

        let failure = fixture
            .reconcile()
            .await
            .expect_err("fact write failure must surface as a tick error, not silent needs_human");
        assert!(
            failure.contains("automation_attention_fact_write_failed"),
            "tick diagnostic must carry the fact-write failure code: {failure}"
        );

        // 恢复可写：同一现场正常写事实并分诊停点（不冒充已通知）。
        let mut writable = std::fs::metadata(&enrollment_dir).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)] // 测试恢复可写:unix 下 false 即加回写位。
        writable.set_readonly(false);
        std::fs::set_permissions(&enrollment_dir, writable).unwrap();
        assert_eq!(
            fixture.reconcile().await.unwrap(),
            ReconcileOutcome::NeedsHuman
        );
        let saved = crate::product::issue_automation_store::IssueAutomationStore::new(
            fixture.inner.paths.clone(),
        )
        .get(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .unwrap();
        assert!(
            saved.attention_facts.iter().any(|fact| fact.key
                == format!(
                    "{plan_id}:wiga-advance-{}-{plan_id}:advance_failed",
                    enrollment.enrollment_id
                )),
            "restored write path must persist the advance failure fact: {:?}",
            saved.attention_facts
        );
    }

    /// Y（真机缺陷）：enrollment 的 durable 绑定 plan 被删除后，reconcile 不得
    /// 继续 Err 无限重试——判 NeedsHuman 停点，并同源 CAS 把 enrollment 置
    /// terminated；此后显式跳过（自愈为 NoEnrollment，服务重启也不再重试）。
    #[tokio::test]
    async fn reconcile_terminates_enrollment_when_bound_plan_was_deleted() {
        let fixture = OrchestratorFixture::new().await;
        let worker = fixture.worker();
        let enrollment = fixture
            .automation_store()
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        crate::web::handlers::lifecycle::plan_preparation::ensure_enrolled_plan(
            &fixture.state,
            &enrollment,
        )
        .await
        .unwrap();
        let bound = fixture
            .automation_store()
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        let plan_id = bound.plan_id.clone().expect("bound plan");
        fixture
            .lifecycle()
            .delete_issue_work_item_plan(PROJECT_ID, ISSUE_ID, &plan_id)
            .unwrap();
        assert!(
            fixture.bound_plans().is_empty(),
            "bound plan must really be deleted"
        );

        assert_eq!(
            worker
                .reconcile(&fixture.state, PROJECT_ID, ISSUE_ID)
                .await
                .unwrap(),
            ReconcileOutcome::NeedsHuman,
            "deleted binding must be a human stop point, not an endless Err"
        );
        let terminated = fixture
            .automation_store()
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        assert!(
            terminated.terminated_at.is_some(),
            "deleted binding must terminate the enrollment"
        );
        assert!(!terminated.enabled);
        // 自愈：terminated 是显式跳过，不再每 tick 重试 ensure。
        assert_eq!(
            worker
                .reconcile(&fixture.state, PROJECT_ID, ISSUE_ID)
                .await
                .unwrap(),
            ReconcileOutcome::NoEnrollment
        );
    }

    /// Y：绑定会话（plan 仍在）被删除同样不可自愈——与 plan 删除同源收口。
    #[tokio::test]
    async fn reconcile_terminates_enrollment_when_bound_session_was_deleted() {
        let fixture = OrchestratorFixture::new().await;
        let worker = fixture.worker();
        let enrollment = fixture
            .automation_store()
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        crate::web::handlers::lifecycle::plan_preparation::ensure_enrolled_plan(
            &fixture.state,
            &enrollment,
        )
        .await
        .unwrap();
        let bound = fixture
            .automation_store()
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        let session_id = bound.session_id.clone().expect("bound session");
        std::fs::remove_file(
            fixture
                .inner
                .paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("workspace-sessions")
                .join(format!("{session_id}.json")),
        )
        .unwrap();

        assert_eq!(
            worker
                .reconcile(&fixture.state, PROJECT_ID, ISSUE_ID)
                .await
                .unwrap(),
            ReconcileOutcome::NeedsHuman
        );
        let terminated = fixture
            .automation_store()
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        assert!(terminated.terminated_at.is_some());
        assert_eq!(
            worker
                .reconcile(&fixture.state, PROJECT_ID, ISSUE_ID)
                .await
                .unwrap(),
            ReconcileOutcome::NoEnrollment
        );
    }

    /// Y：普通错误（enrollment 不可读）不得每 tick 重试——首轮尝试、第二轮
    /// 在退避窗口内跳过（本轮只处理另一个 issue），避免高频刷屏。
    #[tokio::test]
    async fn reconcile_all_once_backs_off_repeated_errors() {
        let fixture = OrchestratorFixture::new().await;
        let worker = fixture.worker();
        std::fs::write(
            fixture
                .inner
                .paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("automation-enrollment.json"),
            "{not json",
        )
        .unwrap();

        assert_eq!(worker.reconcile_all_once().await.unwrap(), 2);
        assert_eq!(
            worker.reconcile_all_once().await.unwrap(),
            1,
            "failing issue must be skipped inside the backoff window"
        );
    }

    /// Y：退避时长指数增长并封顶（2s → 4s → 8s … 上限 5 分钟）。
    #[test]
    fn reconcile_backoff_delay_grows_exponentially_with_cap() {
        assert_eq!(
            reconcile_backoff_delay(1),
            std::time::Duration::from_secs(2)
        );
        assert_eq!(
            reconcile_backoff_delay(2),
            std::time::Duration::from_secs(4)
        );
        assert_eq!(
            reconcile_backoff_delay(3),
            std::time::Duration::from_secs(8)
        );
        assert_eq!(
            reconcile_backoff_delay(u32::MAX),
            std::time::Duration::from_secs(300)
        );
    }
