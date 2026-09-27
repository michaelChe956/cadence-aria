//! P1 WIGA Task 7/8 共享测试 fixture：enrollment 绑定 plan/session 的审批门
//! 与可编译链（accepted contract drafts + source/IR/report refs），供编排器
//! 停等人对照与 plan_confirmed_info 投影测试复用。仅测试构建编译。

#![cfg(test)]


    use crate::cross_cutting::streaming_provider::ProviderCommand;
    use crate::product::issue_automation_store::IssueAutomationStore;
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::product::models::workspace::WorkspaceSessionRecord;
    use crate::product::models::{
        SingleCandidatePhase, WorkItemDraftRecord, WorkItemDraftStatus, WorkItemGenerationMode,
        WorkItemKind, WorkItemOutline, WorkItemOutlineDependencyEdge, WorkItemOutlineSessionFit,
        WorkItemPlanDraftActiveIndex, WorkItemPlanOutline, WorkspaceSessionStatus,
    };
    use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason, RunPolicy};
    use crate::product::work_item_plan_store::WorkItemPlanStore;
    use crate::product::workspace_engine::SingleCandidateCompileCheckpoint;
    use crate::product::workspace_engine::WorkItemPlanCompileFinalizerCheckpoint;
    use crate::product::workspace_engine::WorkspaceSession;
    use crate::product::workspace_engine::tests::single_candidate_recovery::single_candidate_recovery_record;
    use crate::web::autopilot_orchestrator::{
        AutopilotOrchestrator, OrchestratorConfig, ReconcileOutcome,
    };
    use crate::web::state::WebAppState;
    use crate::web::handlers::automation_enrollment_test_support::{
        enrollment_body, put_enrollment, response_json, seed_fixture,
    };
    pub(crate) use crate::web::handlers::automation_enrollment_test_support::{ISSUE_ID, PROJECT_ID};
    use crate::web::handlers::lifecycle::plan_preparation::ensure_enrolled_plan;
    use crate::web::workspace_session::WorkspaceSessionManager;
    use crate::web::workspace_ws_handler::ProviderRunKind;
    use crate::web::workspace_ws_types::{
        ArtifactPayload, ChoiceOption, WorkItemGenerationModeDto, WorkItemPlanOutlineCandidateDto,
        WsOutMessage,
    };
    use axum::http::StatusCode;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use tower::ServiceExt;

    pub(crate) struct EnrolledGateFixture {
        pub(crate) inner: crate::web::handlers::automation_enrollment_test_support::Fixture,
        pub(crate) state: WebAppState,
        lifecycle: LifecycleStore,
        plan_id: String,
        pub(crate) session_id: String,
        manager: Arc<WorkspaceSessionManager>,
        token: u64,
        incarnation: String,
        gate_id: Option<String>,
    }

    /// 测试态恒健康 provider health 源：CI/测试机上探测不到 claude/codex 二进制
    /// 时真实 `ProviderHealthService` 恒 degraded，gateway spawn 前的
    /// `ensure_available` 会拒绝 fake registry 承接的 claude_code 方言。P2
    /// Fake 全链的 provider 启动经 provider gateway（引擎对带
    /// target_snapshot 的 attempt fail-closed 强制），须以恒健康 gate 重建
    /// gateway factory（仅测试 fixture，生产路径不受影响）。
    struct AlwaysHealthyProviderHealth;

    impl crate::cross_cutting::provider_availability_gate::ProviderHealthSource
        for AlwaysHealthyProviderHealth
    {
        fn snapshot(
            &self,
        ) -> std::sync::Arc<crate::cross_cutting::provider_health::ProviderHealthSnapshot> {
            let checked_at = chrono::Utc::now();
            let entry = |provider: crate::product::models::ProviderName| {
                crate::cross_cutting::provider_health::ProviderHealthEntry {
                    provider,
                    command: "fake-health".to_string(),
                    available: true,
                    version: Some("fake".to_string()),
                    reason_code: None,
                    reason: None,
                    checked_at,
                }
            };
            std::sync::Arc::new(
                crate::cross_cutting::provider_health::ProviderHealthSnapshot {
                    schema_version: 1,
                    generation: 1,
                    checked_at,
                    providers: vec![
                        entry(crate::product::models::ProviderName::ClaudeCode),
                        entry(crate::product::models::ProviderName::Codex),
                        entry(crate::product::models::ProviderName::Pi),
                        entry(crate::product::models::ProviderName::KimiCode),
                    ],
                },
            )
        }

        fn degraded(&self) -> bool {
            false
        }
    }

    fn fake_state_with_gateway(root: std::path::PathBuf) -> WebAppState {
        let state = WebAppState::new(
            root.clone(),
            crate::web::runtime::WebRuntime::new_fake(root.clone()),
        );
        let factory = crate::web::gateway_factory::LogicalCodebaseGatewayFactory::new(
            crate::product::app_paths::ProductAppPaths::new(root.join(".aria")),
            state.provider_registry.clone(),
            state.provider_adapter.clone(),
            std::sync::Arc::new(
                crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::with_host_readiness(
                    std::sync::Arc::new(AlwaysHealthyProviderHealth),
                    std::sync::Arc::new(
                        crate::cross_cutting::provider_availability_gate::AlwaysReadyProviderHost,
                    ),
                ),
            ),
        );
        state.with_gateway_factory(std::sync::Arc::new(factory))
    }

    impl EnrolledGateFixture {
        pub(crate) async fn new() -> Self {
            let inner = seed_fixture(1, true);
            let root_path = inner._root.path().to_path_buf();
            let state = fake_state_with_gateway(root_path.clone());
            // 子 WorkItem 上下文经 issue.repo_id 解析 repo-1：补进 repos.json
            //（seed_logical_codebase 重写后仅剩 physical 成员条目）。
            {
                let repos_path = inner.paths.project_root(PROJECT_ID).join("repos.json");
                let mut repositories: Vec<crate::product::models::RepositoryRecord> =
                    crate::product::json_store::read_json(&repos_path).unwrap();
                let repo_root = inner._root.path().join("repo-1");
                std::fs::create_dir_all(&repo_root).unwrap();
                let now = "2026-09-27T00:00:00Z".to_string();
                repositories.push(crate::product::models::RepositoryRecord {
                    id: crate::web::handlers::automation_enrollment_test_support::REPOSITORY_ID
                        .to_string(),
                    project_id: PROJECT_ID.to_string(),
                    name: "repo-1".to_string(),
                    path: repo_root,
                    repo_hash: "sha256:fixture-repo-1".to_string(),
                    runtime_root: inner._root.path().join("repo-1/.aria/runtime"),
                    default_policy_preset: "manual-write".to_string(),
                    default_provider_mode: "fake".to_string(),
                    created_at: now.clone(),
                    logical_repository_id: None,
                    primary_checkout_id: None,
                    identity_schema_version: 1,
                    updated_at: now,
                });
                crate::product::json_store::write_json(&repos_path, &repositories).unwrap();
            }
            // 追加一对「仓库指向有效 physical checkout」的确认 story/design：
            // 绑定 plan 的 IR target 由此解析（seed 的 repo-1 不在有效成员集，
            // compile 会在 target 有效性上 fail-closed）。
            let lifecycle_seed = LifecycleStore::new(inner.paths.clone());
            let story = lifecycle_seed
                .create_story_spec(crate::product::lifecycle_store::CreateStorySpecInput {
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    repository_id: "physical-checkout-enroll-a".to_string(),
                    title: "gate compile story".to_string(),
                    aggregate_codebase: None,
                })
                .unwrap();
            lifecycle_seed
                .append_version(crate::product::lifecycle_store::AppendSpecVersionInput {
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    entity_id: story.id.clone(),
                    markdown: "# gate story".to_string(),
                    provider_run_refs: Vec::new(),
                    review_refs: Vec::new(),
                    confirmed_by: None,
                })
                .unwrap();
            lifecycle_seed
                .update_spec_confirmation_status(
                    PROJECT_ID,
                    ISSUE_ID,
                    &story.id,
                    crate::product::models::LifecycleConfirmationStatus::Confirmed,
                )
                .unwrap();
            let design = lifecycle_seed
                .create_design_spec(crate::product::lifecycle_store::CreateDesignSpecInput {
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    story_spec_ids: vec![story.id.clone()],
                    title: "gate compile design".to_string(),
                    aggregate_codebase: None,
                })
                .unwrap();
            lifecycle_seed
                .append_version(crate::product::lifecycle_store::AppendSpecVersionInput {
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    entity_id: design.id.clone(),
                    markdown: "# gate design".to_string(),
                    provider_run_refs: Vec::new(),
                    review_refs: Vec::new(),
                    confirmed_by: None,
                })
                .unwrap();
            lifecycle_seed
                .update_spec_confirmation_status(
                    PROJECT_ID,
                    ISSUE_ID,
                    &design.id,
                    crate::product::models::LifecycleConfirmationStatus::Confirmed,
                )
                .unwrap();
            let body = serde_json::json!({
                "expected_revision": null,
                "command": {
                    "type": "enable",
                    "selection_key": "gate-fixture-1",
                    "source": {
                        "stories": [{"id": story.id, "version": 1}],
                        "designs": [{"id": design.id, "version": 1}]
                    },
                    "options": {
                        // 逻辑单 target 的 coding 引擎对带 target_snapshot 的 attempt
                        // 强制经 provider gateway（`logical_provider_gateway_required`
                        // fail-closed，Fake 无 gateway 方言不可用作 attempt provider）；
                        // 测试 provider 模式下 registry 将 claude_code 路由到
                        // TestControlledFakeStreamingProvider，行为等同 fake。
                        "author_provider": "claude_code",
                        "reviewer_provider": "claude_code",
                        "review_rounds": 1,
                        "superpowers_enabled": false,
                        "openspec_enabled": false,
                        "plan_options": {
                            "include_integration_tests": false,
                            "include_e2e_tests": false,
                            "force_frontend_backend_split": false,
                            "require_execution_plan_confirm": false
                        }
                    },
                    "logical_repository_id": crate::web::handlers::automation_enrollment_test_support::SINGLE_LOGICAL_ID
                }
            });
            let app = crate::web::app::build_web_router(state.clone());
            let enable = put_enrollment(&app, body).await;
            assert_eq!(enable.status(), StatusCode::OK);
            assert!(response_json(enable).await["enabled"].as_bool().unwrap());

            let lifecycle = LifecycleStore::new(inner.paths.clone());
            let store = IssueAutomationStore::new(inner.paths.clone());
            let enrollment = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
            // 真实 Task 4 补偿面：锁内唯一创建/绑定（不触发生成动作）。
            crate::web::handlers::lifecycle::plan_preparation::ensure_enrolled_plan(
                &state, &enrollment,
            )
            .await
            .expect("ensure enrolled plan");
            let bound = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
            let plan_id = bound.plan_id.expect("bound plan");
            let session_id = bound.session_id.expect("bound session");

            let manager = state
                .workspace_sessions
                .get_or_create(&session_id, || {
                    WorkspaceSessionManager::create(&state, &session_id)
                })
                .await
                .expect("bound session manager");
            // 已派发的生成 run：manager 唯一 run 面真实注册；provider 等待者
            // 持续消费 command 通道（choice 应答 Delivered 的真实对端）。
            let (_run_id, token, _cancel, command_rx, _node) = manager
                .start_run(ProviderRunKind::WorkItemPlanSingleCandidateAuthor, None)
                .await
                .expect("in-flight generation run");
            let incarnation = manager.active_run_incarnation().expect("run incarnation");
            tokio::spawn(deliver_choice_receipts(command_rx));
            Self {
                inner,
                state,
                lifecycle,
                plan_id,
                session_id,
                manager,
                token,
                incarnation,
                gate_id: None,
            }
        }

        /// 真实登记两张待答 choice 卡（run 挂起帧面，与 P0 part_02 同构）。
        pub(crate) async fn open_choice_with_two_questions(&self) {
            self.manager
                .register_pending_choice_frame(gate_choice_frame("choice-gate-1"));
            self.manager
                .register_pending_choice_frame(gate_choice_frame("choice-gate-2"));
        }

        pub(crate) fn provider_start_ledger(&self) -> usize {
            self.durable().provider_start_ledger.len()
        }

        pub(crate) fn durable(&self) -> WorkspaceSessionRecord {
            self.lifecycle
                .get_workspace_session(&self.session_id)
                .expect("durable bound session")
        }

        pub(crate) async fn reconcile(&self) -> Result<ReconcileOutcome, String> {
            let worker = AutopilotOrchestrator::new(self.state.clone(), Default::default());
            worker.reconcile(&self.state, PROJECT_ID, ISSUE_ID).await
        }

        /// 逐张经 P0 REST 作答并等待 Delivered；应答送达后 run 收尾。
        pub(crate) async fn human_answer_via_rest(&self) {
            let app = crate::web::app::build_web_router(self.state.clone());
            for choice_id in ["choice-gate-1", "choice-gate-2"] {
                let body = serde_json::json!({
                    "command_id": format!("cmd-{choice_id}"),
                    "expected_run_id": self.incarnation,
                    "answers": [
                        { "question_id": "q-1", "selected_option_ids": ["yes"], "free_text": null },
                        { "question_id": "q-2", "selected_option_ids": ["no"], "free_text": null },
                    ],
                });
                let (status, payload) = post_choice(&app, &self.session_id, choice_id, &body).await;
                assert_eq!(status, StatusCode::OK, "choice REST must deliver: {payload}");
                assert_eq!(payload["state"], "delivered", "{payload}");
            }
            self.manager.finish_run(self.token).await;
            assert!(
                self.manager.pending_choice_frames().is_empty(),
                "delivered choices must leave no pending frames"
            );
        }

        /// 把绑定 plan/session 铺成可编译的审批门形态（accepted contract
        /// drafts + active index + outline 候选 + source/IR/report 三 refs，
        /// 全部经真实 store/engine 面），随后两段 failpoint 经 P0 REST
        /// Approve 制造「compile 崩溃后人工恢复」的 durable 现场：
        /// 1) ProvenancePersisted 边界（落 approval+reservation，compile tx
        ///    尚未落盘——不落 recovery 节点的 422 根因）；
        /// 2) 按 part_03/part_09.rs:451、single_candidate_recovery.rs:700 先例
        ///    注册 FirstChildBindingEnsured finalizer failpoint，第二次 approve
        ///    replay 复用同 compile_id 抵达 finalizer（tx 已落盘）→ Err 分支
        ///    自动标记 RecoveryRequired 并落 recovery 节点，供人工
        ///    CompileRecovery Continue 恢复原事务。
        pub(crate) async fn fail_compile_after_human_approve(&mut self) {
            // 阶段一（durable 铺底）：accepted contract drafts + active index +
            // source/IR/report 三 refs + 审批门快照，全部走真实 store 面。
            let engine_arc = self.manager.engine();
            let mut engine = engine_arc.lock().await;
            engine.session.artifact = Some(gate_outline_candidate_payload());
            let plan_store = WorkItemPlanStore::new(self.inner.paths.clone());
            let draft_a = gate_draft_record(&self.plan_id, "outline_a", "draft_outline_a");
            let mut draft_b = gate_draft_record(&self.plan_id, "outline_b", "draft_outline_b");
            let mut required =
                crate::product::work_item_contract::canonical_contract_fixture("unused")
                    .input_contracts
                    .remove(0);
            required.provider_logical_work_item_id = "wi_a".to_string();
            required.contract_id = "contract.canonical".to_string();
            required.required_capabilities = vec!["stable_hash".to_string()];
            draft_b
                .candidate
                .canonical_contract_candidate
                .input_contracts
                .push(required);
            draft_b
                .candidate
                .canonical_contract_candidate
                .handoff_contract
                .provided_contract_refs
                .clear();
            plan_store
                .put_draft_record(&draft_a)
                .expect("put accepted draft a");
            plan_store
                .put_draft_record(&draft_b)
                .expect("put accepted draft b");
            plan_store
                .save_active_index(&gate_active_index(&self.plan_id))
                .expect("save accepted draft index");
            single_candidate_recovery_record(
                &self.lifecycle,
                &mut engine,
                SingleCandidatePhase::Approval,
                RunPolicy::Interactive,
            );
            let mut record = self.durable();
            record.status = WorkspaceSessionStatus::WaitingForHuman;
            record.human_gate_snapshot = Some(HumanGateSnapshot {
                findings: Vec::new(),
                repeated_fingerprints: Vec::new(),
                attempts_used: 0,
                manual_repairs_remaining: 1,
                trigger: HumanReason::NativeHumanRequired,
                resumable: false,
                accepted_feedback_turns: None,
            });
            crate::product::json_store::write_json(&self.session_path(&record.id), &record)
                .expect("persist approval session");
            drop(engine);

            // 阶段二（当前 manager）：choice 应答后 run 收尾会触发 idle 回收，
            // HTTP 侧按 registry 解析当前 manager；outline 候选与 failpoint
            // 必须登记在「真正执行 approve 的那个引擎」上。
            let state = self.state.clone();
            let session_id = self.session_id.clone();
            let manager = state
                .workspace_sessions
                .get_or_create(&session_id, || {
                    WorkspaceSessionManager::create(&state, &session_id)
                })
                .await
                .expect("current registry manager");
            let engine_arc = manager.engine();
            let mut engine = engine_arc.lock().await;
            engine.session.artifact = Some(gate_outline_candidate_payload());
            engine.session.stage = crate::product::workspace_engine::WorkspaceStage::HumanConfirm;
            engine.session.session_status = WorkspaceSessionStatus::WaitingForHuman;
            // 真实开门（时间线 HumanConfirm 节点 + durable WaitingForHuman）。
            engine
                .enter_human_confirm(Some("人工批准（compile 将在 finalizer 失败）".to_string()))
                .await;
            let gate_id = engine.active_timeline_node_id().expect("gate node");
            let _failpoint = engine.register_single_candidate_compile_failpoint(
                SingleCandidateCompileCheckpoint::ProvenancePersisted,
            );
            drop(engine);
            self.gate_id = Some(gate_id.clone());

            let app = crate::web::app::build_web_router(self.state.clone());
            let body = serde_json::json!({
                "type": "approve",
                "command_id": "cmd-approve-failpoint",
                "expected_gate_id": gate_id,
            });
            let (status, payload) = post_human_action(&app, &self.session_id, &body).await;
            assert!(
                !status.is_success(),
                "failpoint approve must not succeed: {status} {payload}"
            );
            let message = payload["message"].as_str().unwrap_or_default();
            assert!(
                payload["code"] == "single_candidate_approval_compile_failed"
                    || message.contains("compile failed; human gate remains open"),
                "failpoint approve must fail at the compile finalizer: {payload}"
            );
            let durable = self.durable();
            assert_ne!(durable.status, WorkspaceSessionStatus::Confirmed);
            assert_ne!(
                durable.single_candidate_phase,
                Some(SingleCandidatePhase::Completed)
            );

            // 阶段三（finalizer 边界 + replay）：ProvenancePersisted 后 compile
            // tx 未落盘，上面这次失败不落 recovery 节点（REST CompileRecovery
            // 422 根因）。从 durable reservation 读 compile_id 注册 finalizer
            // failpoint；第二次 approve 走 replay 复用同一 compile_id 抵达
            // finalizer（tx 已在绑定光标处落盘）→ Err 分支自动标记
            // RecoveryRequired 并落 recovery 节点。
            let compile_id = self
                .durable()
                .compile_reservation
                .as_ref()
                .expect("provenance boundary must leave durable compile reservation")
                .compile_id
                .clone();
            let replay_gate_id = {
                let engine = manager.engine();
                let engine = engine.lock().await;
                engine
                    .active_timeline_node_id()
                    .expect("reopened gate node after provenance failure")
            };
            let finalizer_failpoint = {
                let engine = manager.engine();
                let engine = engine.lock().await;
                engine.register_work_item_plan_compile_finalizer_failpoint(
                    &compile_id,
                    WorkItemPlanCompileFinalizerCheckpoint::FirstChildBindingEnsured,
                )
            };
            let replay_body = serde_json::json!({
                "type": "approve",
                "command_id": "cmd-approve-finalizer-failpoint",
                "expected_gate_id": replay_gate_id,
            });
            let (status, payload) =
                post_human_action(&app, &self.session_id, &replay_body).await;
            drop(finalizer_failpoint);
            assert!(
                !status.is_success(),
                "finalizer failpoint approve must not succeed: {status} {payload}"
            );
            let message = payload["message"].as_str().unwrap_or_default();
            assert!(
                payload["code"] == "single_candidate_approval_compile_failed"
                    || message.contains("compile failed; human gate remains open"),
                "replay approve must fail at the finalizer failpoint: {payload}"
            );
            let durable = self.durable();
            assert_ne!(durable.status, WorkspaceSessionStatus::Confirmed);
            assert_eq!(
                durable.status,
                WorkspaceSessionStatus::WaitingForHuman,
                "finalizer crash must leave the gate open for humans"
            );
            let recovery_gate_id = {
                let engine = manager.engine();
                let engine = engine.lock().await;
                engine
                    .active_timeline_node_id()
                    .expect("recovery node after finalizer crash")
            };
            assert_ne!(
                recovery_gate_id, replay_gate_id,
                "finalizer crash must activate a compile recovery node"
            );
        }

        /// 人工 CompileRecovery Continue（P0 REST）恢复原事务并提交，再经
        /// 人工 Approve 关门——终态 Confirmed + Completed + plan Confirmed。
        /// 前置：`fail_compile_after_human_approve` 已落 recovery 节点。
        pub(crate) async fn recover_and_confirm_compile(&self) {
            let state = self.state.clone();
            let session_id = self.session_id.clone();
            let manager = state
                .workspace_sessions
                .get_or_create(&session_id, || {
                    WorkspaceSessionManager::create(&state, &session_id)
                })
                .await
                .expect("current registry manager");
            let recovery_gate_id = {
                let engine = manager.engine();
                let engine = engine.lock().await;
                engine.active_timeline_node_id().expect("recovery gate node")
            };
            let app = crate::web::app::build_web_router(state.clone());
            let recovery_body = serde_json::json!({
                "type": "compile_recovery",
                "command_id": "cmd-recovery-continue",
                "expected_gate_id": recovery_gate_id,
                "action": "continue",
                "reason": null,
            });
            let (status, payload) = post_human_action(&app, &session_id, &recovery_body).await;
            assert_eq!(status, StatusCode::OK, "recovery continue: {payload}");
            assert_eq!(payload["state"], "accepted", "{payload}");

            // 恢复后门重新开启：再次人工 Approve（failpoint 已释放）→ Confirmed。
            let confirm_gate_id = {
                let engine = manager.engine();
                let engine = engine.lock().await;
                engine.active_timeline_node_id().expect("reopened gate node")
            };
            let approve_body = serde_json::json!({
                "type": "approve",
                "command_id": "cmd-approve-after-recovery",
                "expected_gate_id": confirm_gate_id,
            });
            let (status, payload) = post_human_action(&app, &session_id, &approve_body).await;
            assert_eq!(status, StatusCode::OK, "approve after recovery: {payload}");
            assert_eq!(payload["state"], "accepted", "{payload}");
            let durable = self.durable();
            assert_eq!(durable.status, WorkspaceSessionStatus::Confirmed);
            assert_eq!(
                durable.single_candidate_phase,
                Some(SingleCandidatePhase::Completed)
            );
        }

        /// 人工 Abandon（P0 REST）：终态后编排器零复活、零额外 provider。
        pub(crate) async fn abandon_via_rest(&self) {
            // 失败 approve 会重开门节点：以「当前 registry 引擎」的活跃门为准
            //（与 HTTP handler 同一解析口径，人手视角）。
            let state = self.state.clone();
            let session_id = self.session_id.clone();
            let manager = state
                .workspace_sessions
                .get_or_create(&session_id, || {
                    WorkspaceSessionManager::create(&state, &session_id)
                })
                .await
                .expect("current registry manager");
            let gate_id = {
                let engine = manager.engine();
                let engine = engine.lock().await;
                engine
                    .active_timeline_node_id()
                    .expect("reopened gate node")
            };
            let app = crate::web::app::build_web_router(self.state.clone());
            let body = serde_json::json!({
                "type": "abandon",
                "command_id": "cmd-abandon-terminal",
                "expected_gate_id": gate_id,
            });
            let (status, payload) = post_human_action(&app, &self.session_id, &body).await;
            assert_eq!(status, StatusCode::OK, "{payload}");
            assert_eq!(payload["state"], "accepted", "{payload}");
            assert_eq!(self.durable().status, WorkspaceSessionStatus::Terminated);
        }

        /// 当前 durable enrollment（真实 IssueAutomationStore 读取）。
        pub(crate) fn enrollment(
            &self,
        ) -> crate::product::models::automation::IssueAutomationEnrollment {
            IssueAutomationStore::new(self.inner.paths.clone())
                .get(PROJECT_ID, ISSUE_ID)
                .expect("load enrollment")
                .expect("enrollment present")
        }

        /// issue 下全部真实 coding attempt（CodingAttemptStore 读取）。
        pub(crate) fn coding_attempts(
            &self,
        ) -> Vec<crate::product::coding_models::CodingExecutionAttempt> {
            crate::product::coding_attempt_store::CodingAttemptStore::new(
                self.inner.paths.clone(),
            )
            .list_attempts_for_issue(PROJECT_ID, ISSUE_ID)
            .expect("list coding attempts")
        }

        /// 进程内已启动 coding runner 总数（真实 registry 事实，不含注册即撤）。
        pub(crate) fn coding_runner_count(&self) -> usize {
            self.coding_attempts()
                .iter()
                .map(|attempt| {
                    self.state.coding_runs.runner_count(
                        &crate::web::state::CodingAttemptRunKey::from_attempt(attempt),
                    )
                })
                .sum()
        }

        /// issue 下唯一 coding attempt（P2 Task 3+ 共享）：fixture 现场恰一
        /// attempt，多于一条即 fixture 构造编程错误。
        pub(crate) fn attempt(
            &self,
        ) -> crate::product::coding_models::CodingExecutionAttempt {
            let attempts = self.coding_attempts();
            assert_eq!(
                attempts.len(),
                1,
                "fixture expects exactly one coding attempt"
            );
            attempts.into_iter().next().unwrap()
        }

        /// 指定 attempt 的进程内 runner 数（真实 registry 事实）。
        pub(crate) fn runner_count(
            &self,
            key: &crate::web::state::CodingAttemptRunKey,
        ) -> usize {
            self.state.coding_runs.runner_count(key)
        }

        /// 真实 attempt store（Task 4+ 共享）。
        pub(crate) fn store(
            &self,
        ) -> crate::product::coding_attempt_store::CodingAttemptStore {
            crate::product::coding_attempt_store::CodingAttemptStore::new(
                self.inner.paths.clone(),
            )
        }

        /// 当前唯一 attempt 的 registry key（Task 4/6 共享）。
        pub(crate) fn attempt_key(&self) -> crate::web::state::CodingAttemptRunKey {
            crate::web::state::CodingAttemptRunKey::from_attempt(&self.attempt())
        }

        /// 绑定 plan id（Task 4/6 共享）。
        pub(crate) fn plan_id(&self) -> String {
            self.enrollment().plan_id.expect("bound plan")
        }

        /// 当前 enrollment 的自动首启 origin（冻结 policy 同源身份）。
        pub(crate) fn auto_origin(&self) -> crate::product::coding_models::CodingStartOrigin {
            let enrollment = self.enrollment();
            crate::product::coding_models::CodingStartOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
            }
        }

        /// Task 8 共享：复用原 `prepare_group_final_confirm_from_readiness`
        /// 重复准备（readiness 重写/节点复用由引擎真实行为决定）。
        pub(crate) async fn prepare_group_final_confirm_again(&self) {
            let attempt = self.attempt();
            let event_tx = self.state.coding_sockets.hub_sender(&self.attempt_key());
            let engine = crate::product::coding_workspace_engine::CodingWorkspaceEngine::new(
                self.store(),
                crate::product::git_workspace_service::GitWorkspaceService::new(),
                event_tx,
            );
            engine
                .prepare_group_final_confirm_from_readiness(&attempt)
                .await
                .expect("repeat group final confirm preparation");
        }

        /// Task 8 共享：人手 `handle_final_confirm`（不代点、不直改状态）。
        pub(crate) async fn confirm_final_by_human(&self) {
            let attempt = self.attempt();
            let event_tx = self.state.coding_sockets.hub_sender(&self.attempt_key());
            let engine = crate::product::coding_workspace_engine::CodingWorkspaceEngine::new(
                self.store(),
                crate::product::git_workspace_service::GitWorkspaceService::new(),
                event_tx,
            );
            engine
                .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .await
                .expect("human final confirm");
        }

        /// 「重启」进程替身：同 `.aria`、全新 WebAppState/runtime（内存态清零）。
        /// 与 new() 同源重建恒健康 gateway factory（重启后 gateway 链路同等可用）。
        pub(crate) fn restart_state(&self) -> WebAppState {
            fake_state_with_gateway(self.inner._root.path().to_path_buf())
        }

        pub(crate) fn session_path(&self, session_id: &str) -> std::path::PathBuf {
            self.inner
                .paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("workspace-sessions")
                .join(format!("{session_id}.json"))
        }
    }

    /// P2 Task 1 共享 fixture：真实 enrollment 绑定 + compile 崩溃人工恢复 +
    /// 确认链完整走完——只保证 durable Confirmed/已发布 compile，不冒称 Ready。
    /// advance 的 fork 基线解析在唯一 logical target 的 physical checkout 上
    /// 跑真实 git（三面同源 main→master 默认链），因此补真实 main 仓库。
    pub(crate) async fn confirmed_enrolled_fixture() -> EnrolledGateFixture {
        let mut fixture = EnrolledGateFixture::new().await;
        init_real_main_checkout(&fixture.inner.paths.root().join("checkout-enroll-a"));
        normalize_checkout_revision_to_unobserved(&fixture.inner.paths);
        fixture.fail_compile_after_human_approve().await;
        fixture.recover_and_confirm_compile().await;
        fixture
    }

    /// P2 Task 4 共享 fixture：Confirmed enrollment 经 Task 1 自动 advance
    /// 到 durable Ready——唯一 attempt 沿真实 journal lineage 创建、冻结
    /// AutoStartOnce policy，尚无任何 runner/provider 启动。
    pub(crate) async fn ready_enrolled_attempt_fixture() -> EnrolledGateFixture {
        let fixture = confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let plan_id = enrollment.plan_id.clone().expect("bound plan");
        let input = crate::product::advance_store::AdvanceInput {
            command_id: format!("wiga-advance-{}-{plan_id}", enrollment.enrollment_id),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id,
        };
        let outcome = crate::web::advance_plan::advance_plan(
            &fixture.state,
            input,
            crate::web::advance_plan::AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
            },
        )
        .await
        .expect("enrolled advance to ready");
        assert!(
            matches!(
                outcome,
                crate::product::advance_store::AdvanceOutcome::Completed { .. }
            ),
            "enrolled advance must complete: {outcome:?}"
        );
        fixture
    }

    /// P2 Task 8 共享 fixture：接 Task 4 已 Ready/claimed 的 attempt，经真实
    /// typed StartCoding（AutoStartOnce origin、stable command id）放行 Fake
    /// runner，沿实际 unit→handoff→review→readiness 业务入口跑到
    /// `WaitingForHuman + FinalConfirm`。失败即暴露真实缺失的 group 事实，
    /// 不手改 attempt status、不从别的 fixture 拷贝 snapshot。
    pub(crate) async fn complete_enrolled_group_waiting_for_final_confirm() -> EnrolledGateFixture {
        let fixture = ready_enrolled_attempt_fixture().await;
        let attempt = fixture.attempt();
        let outcome = crate::web::coding_start::start_coding_once(
            &fixture.state,
            PROJECT_ID,
            ISSUE_ID,
            crate::web::coding_start::StartCodingCommand {
                attempt_id: attempt.id.clone(),
                command_id: format!("wiga-start-{}", attempt.id),
                origin: fixture.auto_origin(),
            },
        )
        .await
        .expect("enrolled auto first start");
        assert!(
            matches!(
                outcome,
                crate::web::coding_start::StartCodingOutcome::Started { .. }
            ),
            "fake campaign must first-start exactly once: {outcome:?}"
        );
        tokio::time::timeout(std::time::Duration::from_secs(150), async {
            loop {
                let current = fixture.attempt();
                if current.status
                    == crate::product::coding_models::CodingAttemptStatus::WaitingForHuman
                    && current.stage
                        == crate::product::coding_models::CodingExecutionStage::FinalConfirm
                {
                    break;
                }
                assert!(
                    !matches!(
                        current.status,
                        crate::product::coding_models::CodingAttemptStatus::Failed
                            | crate::product::coding_models::CodingAttemptStatus::Aborted
                            | crate::product::coding_models::CodingAttemptStatus::AwaitingManualRecovery
                    ),
                    "fake campaign stopped unexpectedly at {:?}/{:?} reason={:?}",
                    current.status,
                    current.stage,
                    current.manual_recovery_reason
                );
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("fake campaign reaches human FinalConfirm");
        fixture
    }

    /// 在 physical checkout 上初始化真实 main 分支 git 仓库（空提交即可满足
    /// `resolve_advance_base_branch` 的 main→master 默认链验证）。
    fn init_real_main_checkout(repo: &std::path::Path) {
        fn git(repo: &std::path::Path, args: &[&str]) {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(args)
                .output()
                .expect("spawn git");
            assert!(
                output.status.success(),
                "git {args:?} in {repo:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::fs::create_dir_all(repo).expect("create checkout dir");
        git(repo, &["init"]);
        git(repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        git(repo, &["config", "user.email", "aria@example.com"]);
        git(repo, &["config", "user.name", "Aria Test"]);
        git(repo, &["commit", "--allow-empty", "-m", "advance base"]);
        // ReviewRequest 阶段对 aria/issues/{issue} 分支做 `git ls-remote origin`
        // 并 push：配一个本地 bare origin 满足远端存在性（不触网）。
        let origin = repo
            .parent()
            .map(|parent| parent.join("origin-enroll-a.git"))
            .expect("checkout parent");
        let _ = std::fs::remove_dir_all(&origin);
        git(repo, &["init", "--bare", origin.to_str().expect("origin path")]);
        git(
            repo,
            &[
                "remote",
                "add",
                "origin",
                origin.to_str().expect("origin path"),
            ],
        );
        git(repo, &["push", "origin", "main"]);
    }

    /// 对齐生产不变量（snapshot_validator 文档）：repository registration 与
    /// identity 迁移从不持久化 `RepositoryCheckoutRecord.revision`（恒 None），
    /// admission 只在 checkout 侧已观测（Some）时逐字比对。seed 写入的合成
    /// `Some("abcdef")` 会与 `init_real_main_checkout` 真实 HEAD 冻结出的快照
    /// revision 恒不一致 → admission 误判 `target_snapshot_identity_drifted`。
    /// 真仓就位后把权威 checkout 记录 revision 归一为未观测（None）。
    fn normalize_checkout_revision_to_unobserved(
        paths: &crate::product::app_paths::ProductAppPaths,
    ) {
        let lc_store = crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone());
        for checkout in lc_store.list_checkouts(PROJECT_ID).expect("list checkouts") {
            let mut patched = checkout.clone();
            patched.revision = None;
            lc_store
                .save_checkout(PROJECT_ID, &patched)
                .expect("normalize checkout revision");
        }
    }

    /// provider 等待者：真实消费 run command 通道，choice 应答即投递回执。
    pub(crate) async fn deliver_choice_receipts(mut rx: tokio::sync::mpsc::Receiver<ProviderCommand>) {
        while let Some(command) = rx.recv().await {
            if let ProviderCommand::ChoiceResponse {
                receipt: Some(receipt),
                ..
            } = command
            {
                receipt.mark_resolving();
                receipt.deliver();
            }
        }
    }

    pub(crate) fn gate_choice_frame(id: &str) -> WsOutMessage {
        WsOutMessage::ChoiceRequest {
            id: id.to_string(),
            prompt: "验收口径歧义需要用户裁定".to_string(),
            options: vec![ChoiceOption {
                id: "option-a".to_string(),
                label: "按全局口径".to_string(),
                description: None,
            }],
            allow_multiple: false,
            allow_free_text: false,
            questions: Vec::new(),
            source: "ask_user_question".to_string(),
        }
    }

    fn gate_outline() -> WorkItemPlanOutline {
        let item = |outline_id: &str, kind: WorkItemKind, scope: &str| WorkItemOutline {
            target_repository_id: None,
            outline_id: outline_id.to_string(),
            logical_work_item_id: format!("wi_{}", outline_id.strip_prefix("outline_").unwrap()),
            title: outline_id.to_string(),
            kind,
            goal: outline_id.to_string(),
            scope: vec![scope.to_string()],
            non_goals: Vec::new(),
            estimated_context_tokens: Some(12_000),
            session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
            source_story_spec_ids: vec!["story_001".to_string()],
            source_design_spec_ids: vec!["design_001".to_string()],
            exclusive_write_scopes: vec![scope.to_string()],
            forbidden_write_scopes: Vec::new(),
            depends_on: Vec::new(),
            verification_intent: vec![format!("cargo test --locked --lib {outline_id}")],
            trusted_verification_commands: Vec::new(),
            handoff_notes: format!("handoff {outline_id}"),
        };
        let mut outline = WorkItemPlanOutline {
            id: "outline_001".to_string(),
            project_id: "project_001".to_string(),
            issue_id: "issue_001".to_string(),
            source_story_spec_ids: vec!["story_001".to_string()],
            source_design_spec_ids: vec!["design_001".to_string()],
            strategy_summary: "test strategy".to_string(),
            work_item_outlines: vec![
                item("outline_a", WorkItemKind::Backend, "src/a.rs"),
                item("outline_b", WorkItemKind::Frontend, "web/b.ts"),
            ],
            dependency_graph: vec![WorkItemOutlineDependencyEdge {
                from_outline_id: "outline_a".to_string(),
                to_outline_id: "outline_b".to_string(),
            }],
            risks: Vec::new(),
            handoff_strategy: "handoff".to_string(),
            status: "draft".to_string(),
        };
        outline.work_item_outlines[1].depends_on = vec!["outline_a".to_string()];
        outline
    }

    pub(crate) fn gate_outline_candidate_payload() -> ArtifactPayload {
        ArtifactPayload::WorkItemPlanOutlineCandidate {
            outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
                outline: gate_outline(),
                design_context_gaps: vec![],
                validator_findings: vec![],
                context_blockers: vec![],
                current_generation_round_id: Some("round_0001".to_string()),
                selected_generation_mode: Some(WorkItemGenerationModeDto::Serial),
            }),
        }
    }

    pub(crate) fn gate_draft_record(plan_id: &str, outline_id: &str, draft_id: &str) -> WorkItemDraftRecord {
        let now = chrono::Utc::now().to_rfc3339();
        let logical_work_item_id = format!(
            "wi_{}",
            outline_id.strip_prefix("outline_").unwrap_or(outline_id)
        );
        let mut contract =
            crate::product::work_item_contract::canonical_contract_fixture(&logical_work_item_id);
        contract.identity.title = format!("{outline_id} draft");
        contract.identity.kind = "backend".to_string();
        contract.goal.summary = format!("实现 {outline_id}");
        contract.input_contracts.clear();
        contract.handoff_contract.provided_contract_refs.clear();
        contract.write_policy.exclusive_scopes = vec![format!("src/{outline_id}.rs")];
        contract.write_policy.forbidden_scopes.clear();
        contract.verification_checks[0].check_id = format!("cmd_{outline_id}");
        contract.verification_checks[0].command = Some(format!("cargo test --locked --lib {outline_id}"));
        WorkItemDraftRecord {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: plan_id.to_string(),
            draft_id: draft_id.to_string(),
            outline_id: outline_id.to_string(),
            generation_round_id: "round_0001".to_string(),
            batch_id: None,
            attempt_index: 1,
            outline_version_ref: "outline_001".to_string(),
            generation_mode: WorkItemGenerationMode::Serial,
            generation_diagnostics: None,
            candidate: crate::product::models::WorkItemDraftCandidate {
                target_repository_id: None,
                outline_id: outline_id.to_string(),
                logical_work_item_id,
                verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                    checks: contract.verification_checks.clone(),
                },
                canonical_contract_candidate: contract,
            },
            status: WorkItemDraftStatus::Accepted,
            active: true,
            superseded_by_draft_id: None,
            supersede_reason: None,
            copied_from_draft_id: None,
            review_node_id: None,
            review_verdict_ref: None,
            generated_from_node_id: "timeline_node_draft".to_string(),
            accepted_at: Some(now.clone()),
            superseded_at: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    pub(crate) fn gate_active_index(plan_id: &str) -> WorkItemPlanDraftActiveIndex {
        WorkItemPlanDraftActiveIndex {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: plan_id.to_string(),
            current_generation_round_id: "round_0001".to_string(),
            outline_state: "confirmed".to_string(),
            active_outline_id: None,
            outline_to_current_draft_id: BTreeMap::from([
                ("outline_a".to_string(), "draft_outline_a".to_string()),
                ("outline_b".to_string(), "draft_outline_b".to_string()),
            ]),
            draft_statuses: BTreeMap::from([
                (
                    "draft_outline_a".to_string(),
                    WorkItemDraftStatus::Accepted,
                ),
                (
                    "draft_outline_b".to_string(),
                    WorkItemDraftStatus::Accepted,
                ),
            ]),
            batches: vec![],
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub(crate) async fn post_json(
        app: &axum::Router,
        uri: String,
        body: &serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(uri)
 .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    pub(crate) async fn post_choice(
        app: &axum::Router,
        session_id: &str,
        choice_id: &str,
        body: &serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        post_json(
            app,
            format!("/api/workspace-sessions/{session_id}/choices/{choice_id}/response"),
            body,
        )
        .await
    }

    pub(crate) async fn post_human_action(
        app: &axum::Router,
        session_id: &str,
        body: &serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        post_json(app, format!("/api/workspace-sessions/{session_id}/human-actions"), body).await
    }
