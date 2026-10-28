    mod b2_p4 {
        use super::*;
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::models::automation::{
            AutomationAttentionFact, AutomationAttentionKind, AutomationAttentionStage,
            EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand, PlanApprovalPolicy,
            PlanGenerationPhase,
        };
        use crate::product::models::{WorkspaceMessageRecord, WorkspaceType};
        use crate::product::work_item_plan_policy::{
            HumanGateSnapshot, HumanReason, RunPolicy, WorkItemPlanFlowKind,
        };
        use crate::web::handlers::automation_enrollment_test_support::{
            ISSUE_ID, PROJECT_ID, seed_fixture,
        };

        struct ContinueFixture {
            _root: tempfile::TempDir,
            state: WebAppState,
            lifecycle: LifecycleStore,
            store: IssueAutomationStore,
            binding: crate::product::models::automation::EnrollmentBindingIdentity,
            enrollment_id: String,
            plan_id: String,
            session_id: String,
        }

        impl ContinueFixture {
            fn enrollment(&self) -> crate::product::models::automation::IssueAutomationEnrollment {
                self.store
                    .get(PROJECT_ID, ISSUE_ID)
                    .expect("read enrollment")
                    .expect("enrollment present")
            }

            fn record(&self) -> crate::product::models::WorkspaceSessionRecord {
                self.lifecycle
                    .get_workspace_session(&self.session_id)
                    .expect("durable record")
            }

            fn write_record(&self, record: &crate::product::models::WorkspaceSessionRecord) {
                crate::product::json_store::write_json(
                    &self
                        .lifecycle
                        .app_paths()
                        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                        .join("workspace-sessions")
                        .join(format!("{}.json", record.id)),
                    record,
                )
                .expect("persist session json");
            }

            fn binding_json(&self) -> serde_json::Value {
                serde_json::to_value(&self.binding).expect("binding json")
            }

            async fn manager(
                &self,
            ) -> std::sync::Arc<crate::web::workspace_session::WorkspaceSessionManager>
            {
                self.state
                    .workspace_sessions
                    .get_or_create(&self.session_id, || {
                        crate::web::workspace_session::WorkspaceSessionManager::create(
                            &self.state,
                            &self.session_id,
                        )
                    })
                    .await
                    .expect("manager")
            }
        }

        /// gate-passing WorkItemPlan 候选（唯一通过 artifact fence 选择器，
        /// 供修订回合取候选基线）。
        fn candidate_message() -> WorkspaceMessageRecord {
            let markdown = "# Work Item Plan\n\n\
                ## 计划范围\n覆盖 provider 失败后的自动化继续。source id: story_spec_0001。\n\n\
                ## 任务拆分\n- [TASK-001] 恢复链。\n\n\
                ## 依赖图\n无。\n\n\
                ## 验证计划\ncargo test --locked。\n\n\
                ## 执行顺序\n先恢复。\n\n\
                ## 风险\n无。\n\n\
                ## 追踪关系\nsource ids: Story Spec story_spec_0001, Design Spec design_spec_0001。\n\
                [TASK-001] -> [REQ-001]\n";
            WorkspaceMessageRecord {
                role: "assistant".to_string(),
                content: format!("```artifact\n{markdown}```\n"),
                created_at: "2026-10-16T00:00:00Z".to_string(),
            }
        }

        fn gate_snapshot(budget: u32) -> HumanGateSnapshot {
            HumanGateSnapshot {
                candidate_recovery: None,
                findings: Vec::new(),
                repeated_fingerprints: Vec::new(),
                attempts_used: 0,
                manual_repairs_remaining: budget,
                trigger: HumanReason::NativeHumanRequired,
                resumable: true,
                accepted_feedback_turns: Some(0),
            }
        }

        /// auto 授权 + AutoIfValid SC 会话 + 精确绑定的最小 durable 现场。
        async fn continue_fixture(
            approval: PlanApprovalPolicy,
            status: crate::product::models::WorkspaceSessionStatus,
            phase: crate::product::models::SingleCandidatePhase,
            snapshot: Option<HumanGateSnapshot>,
            with_candidate: bool,
        ) -> ContinueFixture {
            let inner = seed_fixture(1, true);
            let root_path = inner._root.path().to_path_buf();
            let state = WebAppState::new(
                root_path.clone(),
                crate::web::runtime::WebRuntime::new_fake(root_path.clone()),
            );
            let paths = crate::product::app_paths::ProductAppPaths::new(root_path.join(".aria"));
            let store = IssueAutomationStore::new(paths.clone());
            let lifecycle = LifecycleStore::new(paths.clone());
            store
                .compare_and_set(
                    PROJECT_ID,
                    ISSUE_ID,
                    None,
                    EnrollmentWriteCommand::Enable {
                        command_id: None,
                        selection_key: "continue-fixture".to_string(),
                        source: EnrollmentSource {
                            stories: Vec::new(),
                            designs: Vec::new(),
                        },
                        options: EnrollmentOptions {
                            author_provider: crate::product::models::ProviderName::Fake,
                            reviewer_provider: crate::product::models::ProviderName::Fake,
                            review_rounds: 0,
                            superpowers_enabled: false,
                            openspec_enabled: false,
                            plan_approval: approval,
                            plan_options: crate::product::models::IssueWorkItemPlanOptions {
                                include_integration_tests: false,
                                include_e2e_tests: false,
                                force_frontend_backend_split: false,
                                require_execution_plan_confirm: false,
                            },
                        },
                        target:
                            crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
                                repository_id: "repo-1".to_string(),
                            },
                    },
                )
                .expect("enable enrollment");
            let plan_id = "issue_work_item_plan_continue_1".to_string();
            // manager 创建前置（ensure_workspace_context_message）需要 plan 记录。
            lifecycle
                .create_issue_work_item_plan(
                    crate::product::lifecycle_store::CreateIssueWorkItemPlanInput {
                        id: Some(plan_id.clone()),
                        project_id: PROJECT_ID.to_string(),
                        issue_id: ISSUE_ID.to_string(),
                        source_story_spec_ids: vec![inner.story_id.clone()],
                        source_design_spec_ids: Vec::new(),
                        options: crate::product::models::IssueWorkItemPlanOptions {
                            include_integration_tests: false,
                            include_e2e_tests: false,
                            force_frontend_backend_split: false,
                            require_execution_plan_confirm: false,
                        },
                        status: crate::product::models::IssueWorkItemPlanStatus::Draft,
                        work_item_ids: Vec::new(),
                        repository_profile_ref: None,
                        verification_plan_ids: Vec::new(),
                        dependency_graph: Vec::new(),
                        created_from_provider_run: None,
                        validator_findings: Vec::new(),
                    },
                )
                .expect("create plan record");
            let mut record = lifecycle
                .create_workspace_session(
                    crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                        project_id: PROJECT_ID.to_string(),
                        issue_id: ISSUE_ID.to_string(),
                        entity_id: plan_id.clone(),
                        workspace_type: WorkspaceType::WorkItemPlan,
                        author_provider: crate::product::models::ProviderName::Fake,
                        reviewer_provider: None,
                        review_rounds: 0,
                        superpowers_enabled: false,
                        openspec_enabled: false,
                        work_item_plan_options: Some(
                            crate::product::lifecycle_store::WorkItemPlanSessionOptions {
                                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                                run_policy: RunPolicy::AutoIfValid,
                                rollout_snapshot: false,
                            },
                        ),
                    },
                )
                .expect("create session");
            record.status = status;
            record.single_candidate_phase = Some(phase);
            record.human_gate_snapshot = snapshot;
            if with_candidate {
                record.messages = vec![candidate_message()];
            }
            crate::product::json_store::write_json(
                &lifecycle
                    .app_paths()
                    .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                    .join("workspace-sessions")
                    .join(format!("{}.json", record.id)),
                &record,
            )
            .expect("persist session json");
            store
                .bind_plan(PROJECT_ID, ISSUE_ID, 1, &plan_id, &record.id)
                .expect("bind plan");
            let enrollment = store
                .get(PROJECT_ID, ISSUE_ID)
                .expect("read enrollment")
                .expect("enrollment present");
            let binding = crate::web::advance_plan::load_current_enrollment_binding(&enrollment)
                .expect("current binding");
            ContinueFixture {
                _root: inner._root,
                state,
                lifecycle,
                store,
                binding,
                enrollment_id: enrollment.enrollment_id.clone(),
                plan_id,
                session_id: record.id,
            }
        }

        fn continue_uri() -> String {
            format!(
                "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/continue-plan"
            )
        }

        fn control_uri() -> String {
            format!("/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/control")
        }

        async fn get_json(app: &axum::Router, uri: String) -> (StatusCode, serde_json::Value) {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method(axum::http::Method::GET)
                        .uri(uri)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let payload = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, payload)
        }

        async fn wait_for<
            F: Fn(&crate::product::models::automation::IssueAutomationEnrollment) -> bool,
        >(
            fixture: &ContinueFixture,
            predicate: F,
            label: &str,
        ) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                let enrollment = fixture.enrollment();
                if predicate(&enrollment) {
                    return;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out waiting for {label}"
                );
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }

        /// TDD ①⑨：Failed 复用 SC 失败重臂＋start_generation；旧 intent 重置
        /// Claimed 再按真实启动推进；继续成功 resolve 停点事实＋wake。
        #[tokio::test]
        async fn continue_plan_failed_session_rearms_resets_intent_and_resolves_fact() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::Failed,
                crate::product::models::SingleCandidatePhase::Failed,
                None,
                false,
            )
            .await;
            // 预置旧 generation intent（ProviderDispatched——恢复后必须先重置）。
            fixture
                .store
                .claim_plan_generation(PROJECT_ID, ISSUE_ID, &fixture.enrollment_id)
                .expect("claim intent");
            fixture
                .store
                .mark_plan_generation_phase(
                    PROJECT_ID,
                    ISSUE_ID,
                    &fixture.enrollment_id,
                    PlanGenerationPhase::ProviderDispatched,
                )
                .expect("stale intent phase");
            // 预置 router 已写的 provider 失败事实（继续成功即 resolve）。
            let failed_key =
                crate::web::plan_generation::plan_failed_node_fact_key(&fixture.session_id);
            fixture
                .store
                .upsert_automation_attention_fact(
                    PROJECT_ID,
                    ISSUE_ID,
                    &fixture.binding,
                    AutomationAttentionFact {
                        key: failed_key.clone(),
                        kind: AutomationAttentionKind::AutomationFailed,
                        stage: AutomationAttentionStage::Plan,
                        plan_id: Some(fixture.plan_id.clone()),
                        session_id: Some(fixture.session_id.clone()),
                        attempt_id: None,
                        reason_code: "plan_provider_failed".to_string(),
                        occurred_at: "2026-10-16T00:00:00Z".to_string(),
                        choice_id: None,
                        gate_id: None,
                    },
                )
                .expect("seed failed fact");

            let app = crate::web::app::build_web_router(fixture.state.clone());
            let body = serde_json::json!({
                "command_id": "cmd-continue-1",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": failed_key,
            });
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(status, StatusCode::OK, "continue accepted: {payload}");
            assert_eq!(payload["state"], "accepted", "{payload}");
            assert_eq!(payload["session_id"], fixture.session_id, "{payload}");

            // 旧 intent 已按真实启动推进（越过 Claimed；Fake run 真实落账）。
            let ledger_before = fixture.record().provider_start_ledger.len();
            wait_for(
                &fixture,
                |_| fixture.record().provider_start_ledger.len() > ledger_before,
                "provider start after continue",
            )
            .await;
            let enrollment = fixture.enrollment();
            let intent =
                crate::web::plan_generation::generation_intent(&fixture.state, &enrollment)
                    .expect("intent read")
                    .expect("intent present");
            match intent {
                crate::product::issue_automation_store::PlanGenerationIntentRead::Current(
                    intent,
                ) => {
                    assert!(
                        matches!(
                            intent.phase,
                            PlanGenerationPhase::EngineStarted
                                | PlanGenerationPhase::ProviderDispatched
                        ),
                        "intent must be re-driven from Claimed, got {:?}",
                        intent.phase
                    );
                }
                crate::product::issue_automation_store::PlanGenerationIntentRead::LegacyUnbound {
                    ..
                } => panic!("legacy intent without target cannot drive automatic chains"),
            }
            // 继续成功 resolve：失败事实消失。
            wait_for(
                &fixture,
                |enrollment| {
                    !enrollment
                        .attention_facts
                        .iter()
                        .any(|fact| fact.key == failed_key)
                },
                "failed fact resolved",
            )
            .await;
        }

        /// TDD ④：同键同负载重放读首次结果（零重复派发）；同键异负载 409；
        /// 停点键不符 409 且零副作用（无账本项）。
        #[tokio::test]
        async fn continue_plan_command_ledger_replay_and_conflict() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::Failed,
                crate::product::models::SingleCandidatePhase::Failed,
                None,
                false,
            )
            .await;
            fixture
                .store
                .claim_plan_generation(PROJECT_ID, ISSUE_ID, &fixture.enrollment_id)
                .expect("claim intent");
            let failed_key =
                crate::web::plan_generation::plan_failed_node_fact_key(&fixture.session_id);
            let app = crate::web::app::build_web_router(fixture.state.clone());

            // 停点键不符 → 409、零副作用（命令账本不落项）。
            let wrong = serde_json::json!({
                "command_id": "cmd-continue-mismatch",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": "plan_gate:not_the_stop:human_confirm",
            });
            let (status, payload) = post_json(&app, continue_uri(), &wrong).await;
            assert_eq!(status, StatusCode::CONFLICT, "stop key mismatch: {payload}");
            assert_eq!(payload["code"], "automation_continue_stop_key_mismatch");
            assert!(
                fixture.enrollment().command_ledger.is_empty(),
                "mismatch must not claim the command"
            );

            let body = serde_json::json!({
                "command_id": "cmd-continue-ledger",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": failed_key,
            });
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(status, StatusCode::OK, "first continue: {payload}");
            let ledger_after_first = fixture.record().provider_start_ledger.len();
            wait_for(
                &fixture,
                |_| fixture.record().provider_start_ledger.len() > ledger_after_first,
                "first continue drives",
            )
            .await;
            let ledger_count = fixture.record().provider_start_ledger.len();

            // 同键同负载 → replayed，不再派发。
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(status, StatusCode::OK, "replay: {payload}");
            assert_eq!(payload["state"], "replayed", "{payload}");
            assert_eq!(
                fixture.record().provider_start_ledger.len(),
                ledger_count,
                "replay must not dispatch again"
            );

            // 同键异负载 → 409。
            let mutated = serde_json::json!({
                "command_id": "cmd-continue-ledger",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": failed_key,
                "context": "不同的负载",
            });
            let (status, payload) = post_json(&app, continue_uri(), &mutated).await;
            assert_eq!(
                status,
                StatusCode::CONFLICT,
                "same command different payload must 409: {payload}"
            );
            assert_eq!(payload["code"], "automation_continue_command_conflict");
        }

        /// TDD ⑤：terminated（N0 字段非空）拒绝继续与派发，零副作用。
        #[tokio::test]
        async fn continue_plan_rejects_terminated_enrollment() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::Failed,
                crate::product::models::SingleCandidatePhase::Failed,
                None,
                false,
            )
            .await;
            let mut enrollment = fixture.enrollment();
            enrollment.terminated_at = Some("2026-10-16T01:00:00Z".to_string());
            crate::product::json_store::write_json(
                &fixture
                    .lifecycle
                    .app_paths()
                    .issue_root(PROJECT_ID, ISSUE_ID)
                    .join("automation-enrollment.json"),
                &enrollment,
            )
            .expect("mark terminated");
            let failed_key =
                crate::web::plan_generation::plan_failed_node_fact_key(&fixture.session_id);
            let app = crate::web::app::build_web_router(fixture.state.clone());
            let body = serde_json::json!({
                "command_id": "cmd-continue-term",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": failed_key,
            });
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(status, StatusCode::CONFLICT, "terminated: {payload}");
            assert_eq!(payload["code"], "automation_plan_terminated");
            assert!(
                fixture.enrollment().command_ledger.is_empty(),
                "terminated continue must be claim-free"
            );
            assert_eq!(
                fixture.record().single_candidate_phase,
                Some(crate::product::models::SingleCandidatePhase::Failed),
                "terminated continue must not rearm"
            );
        }

        /// TDD ②：评审超限（StoppedNeedsHuman）同 session CAS WaitingForHuman＋
        /// 预算耗尽显式继续只开一个新说明回合（TurnOpened）；成功 resolve
        /// 门事实；说明为空拒绝。
        #[tokio::test]
        async fn continue_plan_stopped_gate_reopens_single_turn_and_resolves_fact() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::StoppedNeedsHuman,
                crate::product::models::SingleCandidatePhase::Approval,
                Some(gate_snapshot(0)),
                true,
            )
            .await;
            let gate_key = crate::web::plan_generation::plan_gate_fact_key(&fixture.session_id);
            fixture
                .store
                .upsert_automation_attention_fact(
                    PROJECT_ID,
                    ISSUE_ID,
                    &fixture.binding,
                    AutomationAttentionFact {
                        key: gate_key.clone(),
                        kind: AutomationAttentionKind::ReviewLimit,
                        stage: AutomationAttentionStage::Plan,
                        plan_id: Some(fixture.plan_id.clone()),
                        session_id: Some(fixture.session_id.clone()),
                        attempt_id: None,
                        reason_code: "plan_review_manual_budget_exhausted".to_string(),
                        occurred_at: "2026-10-16T00:00:00Z".to_string(),
                        choice_id: None,
                        gate_id: None,
                    },
                )
                .expect("seed gate fact");
            let app = crate::web::app::build_web_router(fixture.state.clone());

            // control 只读投影：真实 stop_key/stage/allowed/context_required。
            let (status, payload) = get_json(&app, control_uri()).await;
            assert_eq!(status, StatusCode::OK, "control: {payload}");
            assert_eq!(payload["managed"], true, "{payload}");
            assert_eq!(payload["stage"], "stopped_needs_human", "{payload}");
            assert_eq!(payload["stop_key"], gate_key, "{payload}");
            assert_eq!(
                payload["allowed_actions"],
                serde_json::json!(["continue_plan"]),
                "{payload}"
            );
            assert_eq!(payload["context_required"], true, "{payload}");
            assert!(
                fixture
                    .enrollment()
                    .attention_facts
                    .iter()
                    .any(|fact| fact.key == gate_key),
                "control GET must not write facts"
            );

            // 说明为空 → 422，零副作用。
            let blank = serde_json::json!({
                "command_id": "cmd-gate-blank",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": gate_key,
            });
            let (status, payload) = post_json(&app, continue_uri(), &blank).await;
            assert_eq!(
                status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "blank context must 422: {payload}"
            );
            assert_eq!(payload["code"], "automation_continue_context_required");
            assert_eq!(
                fixture.record().status,
                crate::product::models::WorkspaceSessionStatus::StoppedNeedsHuman,
                "blank context must not reopen the gate"
            );

            let body = serde_json::json!({
                "command_id": "cmd-gate-continue",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": gate_key,
                "context": "评审口径按新约束放宽，继续按说明修订。",
            });
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(status, StatusCode::OK, "gate continue: {payload}");
            assert_eq!(payload["state"], "accepted", "{payload}");

            // 同 session 原地重开（WaitingForHuman）＋单说明回合（TurnOpened
            // 的 durable turn 携带 command/说明）。
            let record = fixture.record();
            assert_eq!(
                record.status,
                crate::product::models::WorkspaceSessionStatus::WaitingForHuman
            );
            let turn = fixture
                .lifecycle
                .get_human_gate_turn_by_command_id(&fixture.session_id, "cmd-gate-continue")
                .expect("turn read")
                .expect("turn opened by continue");
            assert_eq!(turn.budget_reserved, 1, "explicit continue opens one turn");

            // 成功 resolve：门事实消失。
            wait_for(
                &fixture,
                |enrollment| {
                    !enrollment
                        .attention_facts
                        .iter()
                        .any(|fact| fact.key == gate_key)
                },
                "gate fact resolved",
            )
            .await;
        }

        /// k3 波3 发现1 fix：needs_human 收尾把命令账本落 NeedsHuman，同键
        /// 重放回执 state=needs_human（不把人工分诊停等读成成功重放）；
        /// accepted 收尾的重放语义不变（见账本重放用例）。
        #[tokio::test]
        async fn continue_plan_needs_human_outcome_replays_needs_human() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::StoppedNeedsHuman,
                crate::product::models::SingleCandidatePhase::Approval,
                Some(gate_snapshot(1)),
                // 不带候选产物：修订回合因候选缺失被引擎 typed 拒绝 →
                // continue 收尾必然 needs_human（确定性停等现场）。
                false,
            )
            .await;
            let gate_key = crate::web::plan_generation::plan_gate_fact_key(&fixture.session_id);
            let app = crate::web::app::build_web_router(fixture.state.clone());
            let body = serde_json::json!({
                "command_id": "cmd-gate-needs-human",
                "expected_binding": fixture.binding_json(),
                "expected_stop_key": gate_key,
                "context": "按新口径继续修订。",
            });
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(status, StatusCode::OK, "needs_human finish: {payload}");
            assert_eq!(payload["state"], "needs_human", "{payload}");

            // durable 账本状态＝NeedsHuman（SC retry mark_needs_human 先例）。
            let enrollment_after = fixture.enrollment();
            let entry = enrollment_after
                .command_ledger
                .iter()
                .find(|entry| entry.command_id == "cmd-gate-needs-human")
                .expect("ledger entry committed");
            assert_eq!(
                entry.state,
                crate::product::models::automation::OperationState::NeedsHuman
            );

            // 同键同负载重放：读首次 durable 结果 → needs_human（非 replayed）。
            let (status, payload) = post_json(&app, continue_uri(), &body).await;
            assert_eq!(
                status,
                StatusCode::OK,
                "replay after needs_human: {payload}"
            );
            assert_eq!(
                payload["state"], "needs_human",
                "replay must surface the durable needs_human result, not a fake replay: {payload}"
            );
        }

        /// TDD ⑦：plan_generation intent NeedsHuman / EngineStarted(ProviderDispatched)
        /// 无活 run 两分支写 automation_failed（稳定 key、手动零写入）。
        #[tokio::test]
        async fn plan_generation_needs_human_branches_write_automation_failed_fact() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::Open,
                crate::product::models::SingleCandidatePhase::Prepare,
                None,
                false,
            )
            .await;
            fixture
                .store
                .claim_plan_generation(PROJECT_ID, ISSUE_ID, &fixture.enrollment_id)
                .expect("claim intent");
            fixture
                .store
                .mark_plan_generation_phase(
                    PROJECT_ID,
                    ISSUE_ID,
                    &fixture.enrollment_id,
                    PlanGenerationPhase::NeedsHuman,
                )
                .expect("intent needs human");
            let enrollment = fixture.enrollment();
            let outcome = crate::web::plan_generation::start_plan_generation_once(
                &fixture.state,
                &enrollment,
            )
            .await
            .expect("observe");
            assert_eq!(
                outcome,
                crate::web::plan_generation::PlanGenerationOutcome::NeedsHuman
            );
            let key = crate::web::plan_generation::plan_generation_failed_fact_key(
                &fixture.enrollment_id,
                &fixture.plan_id,
            );
            let facts = fixture.enrollment().attention_facts;
            let fact = facts
                .iter()
                .find(|fact| fact.key == key)
                .expect("automation_failed fact written");
            assert_eq!(fact.kind, AutomationAttentionKind::AutomationFailed);
            assert_eq!(fact.stage, AutomationAttentionStage::Plan);
            assert_eq!(fact.reason_code, "plan_generation_needs_human");
            assert_eq!(
                fact.session_id.as_deref(),
                Some(fixture.session_id.as_str())
            );

            // 第二分支：EngineStarted 无活 run → 转 NeedsHuman 并写事实。
            let fixture2 = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::Open,
                crate::product::models::SingleCandidatePhase::Prepare,
                None,
                false,
            )
            .await;
            fixture2
                .store
                .claim_plan_generation(PROJECT_ID, ISSUE_ID, &fixture2.enrollment_id)
                .expect("claim intent");
            fixture2
                .store
                .mark_plan_generation_phase(
                    PROJECT_ID,
                    ISSUE_ID,
                    &fixture2.enrollment_id,
                    PlanGenerationPhase::EngineStarted,
                )
                .expect("engine started without run");
            let enrollment2 = fixture2.enrollment();
            let outcome2 = crate::web::plan_generation::start_plan_generation_once(
                &fixture2.state,
                &enrollment2,
            )
            .await
            .expect("observe");
            assert_eq!(
                outcome2,
                crate::web::plan_generation::PlanGenerationOutcome::NeedsHuman
            );
            let facts2 = fixture2.enrollment().attention_facts;
            let fact2 = facts2
                .iter()
                .find(|fact| {
                    fact.key
                        == crate::web::plan_generation::plan_generation_failed_fact_key(
                            &fixture2.enrollment_id,
                            &fixture2.plan_id,
                        )
                })
                .expect("dispatch-unproven fact written");
            assert_eq!(fact2.reason_code, "plan_generation_dispatch_unproven");

            // 手动零写入：manual 冻结的同形状停点不落事实。
            let fixture3 = continue_fixture(
                PlanApprovalPolicy::Manual,
                crate::product::models::WorkspaceSessionStatus::Open,
                crate::product::models::SingleCandidatePhase::Prepare,
                None,
                false,
            )
            .await;
            // 手动授权会话为 Interactive：改写策略后重绑一致性由准入拒绝，
            // 此处只验事实面——intent NeedsHuman 不写事实。
            let enrollment3 = fixture3.enrollment();
            let _ = crate::web::plan_generation::start_plan_generation_once(
                &fixture3.state,
                &enrollment3,
            )
            .await;
            assert!(
                fixture3.enrollment().attention_facts.is_empty(),
                "manual enrollment must not receive plan attention facts"
            );
        }

        /// TDD ⑧：router 观察 choice/HumanGateOpened/durable Failed 写
        /// plan_question/review_limit/automation_failed；HumanGateClosed resolve；
        /// 禁用 enrollment 零写入。
        #[tokio::test]
        async fn router_plan_stop_facts_upsert_and_resolve() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::StoppedNeedsHuman,
                crate::product::models::SingleCandidatePhase::Approval,
                Some(gate_snapshot(0)),
                false,
            )
            .await;
            let manager = fixture.manager().await;
            let engine_tx = manager.engine_tx();
            let gate_key = crate::web::plan_generation::plan_gate_fact_key(&fixture.session_id);

            // HumanGateOpened + 预算耗尽 → review_limit。
            engine_tx
                .send(
                    crate::product::workspace_engine::EngineEvent::HumanGateOpened {
                        stage: "human_confirm".to_string(),
                    },
                )
                .await
                .expect("send gate open");
            wait_for(
                &fixture,
                |enrollment| {
                    enrollment
                        .attention_facts
                        .iter()
                        .any(|fact| fact.key == gate_key)
                },
                "gate fact written",
            )
            .await;
            let fact = fixture
                .enrollment()
                .attention_facts
                .into_iter()
                .find(|fact| fact.key == gate_key)
                .unwrap();
            assert_eq!(fact.kind, AutomationAttentionKind::ReviewLimit);

            // 预算未耗尽（重写快照）→ 同 key upsert 为 plan_question（不增项）。
            let mut record = fixture.record();
            record.human_gate_snapshot = Some(gate_snapshot(2));
            fixture.write_record(&record);
            engine_tx
                .send(
                    crate::product::workspace_engine::EngineEvent::HumanGateOpened {
                        stage: "human_confirm".to_string(),
                    },
                )
                .await
                .expect("send gate open again");
            wait_for(
                &fixture,
                |enrollment| {
                    enrollment.attention_facts.iter().any(|fact| {
                        fact.key == gate_key && fact.kind == AutomationAttentionKind::PlanQuestion
                    })
                },
                "gate fact refined to plan_question",
            )
            .await;
            assert_eq!(
                fixture
                    .enrollment()
                    .attention_facts
                    .iter()
                    .filter(|fact| fact.key == gate_key)
                    .count(),
                1,
                "stable key must not duplicate facts"
            );

            // 挂起 choice → plan_question（focus choice_id）。
            engine_tx
                .send(crate::product::workspace_engine::EngineEvent::ChoiceRequest {
                    id: "choice-p4-1".to_string(),
                    prompt: "选择实现路径".to_string(),
                    options: Vec::new(),
                    allow_multiple: false,
                    allow_free_text: true,
                    questions: Vec::new(),
                    source: crate::cross_cutting::streaming_provider::ChoiceRequestSource::AskUserQuestion,
                })
                .await
                .expect("send choice");
            let choice_key = crate::web::plan_generation::plan_choice_fact_key(
                &fixture.session_id,
                "choice-p4-1",
            );
            wait_for(
                &fixture,
                |enrollment| {
                    enrollment
                        .attention_facts
                        .iter()
                        .any(|fact| fact.key == choice_key)
                },
                "choice fact written",
            )
            .await;
            let choice_fact = fixture
                .enrollment()
                .attention_facts
                .into_iter()
                .find(|fact| fact.key == choice_key)
                .unwrap();
            assert_eq!(choice_fact.kind, AutomationAttentionKind::PlanQuestion);
            assert_eq!(choice_fact.choice_id.as_deref(), Some("choice-p4-1"));

            // 门关闭 → 门事实 resolve（choice 事实保留）。
            engine_tx
                .send(
                    crate::product::workspace_engine::EngineEvent::HumanGateClosed {
                        decision: "abandon".to_string(),
                        stage: "human_confirm".to_string(),
                    },
                )
                .await
                .expect("send gate close");
            wait_for(
                &fixture,
                |enrollment| {
                    !enrollment
                        .attention_facts
                        .iter()
                        .any(|fact| fact.key == gate_key)
                },
                "gate fact resolved on close",
            )
            .await;
            assert!(
                fixture
                    .enrollment()
                    .attention_facts
                    .iter()
                    .any(|fact| fact.key == choice_key),
                "choice fact must survive gate close"
            );

            // durable Failed 收敛 → automation_failed（provider 失败）。
            let mut record = fixture.record();
            record.status = crate::product::models::WorkspaceSessionStatus::Failed;
            record.single_candidate_phase =
                Some(crate::product::models::SingleCandidatePhase::Failed);
            fixture.write_record(&record);
            engine_tx
                .send(
                    crate::product::workspace_engine::EngineEvent::TimelineNodeUpdated {
                        node_id: "node_failed_p4".to_string(),
                        status: crate::web::workspace_ws_types::TimelineNodeStatus::Failed,
                        summary: Some("provider unavailable".to_string()),
                        completed_at: None,
                    },
                )
                .await
                .expect("send failed node");
            let failed_key =
                crate::web::plan_generation::plan_failed_node_fact_key(&fixture.session_id);
            wait_for(
                &fixture,
                |enrollment| {
                    enrollment
                        .attention_facts
                        .iter()
                        .any(|fact| fact.key == failed_key)
                },
                "provider failure fact written",
            )
            .await;

            // 禁用 enrollment → 零写入（后续门开不落新事实）。
            let mut enrollment = fixture.enrollment();
            enrollment.enabled = false;
            crate::product::json_store::write_json(
                &fixture
                    .lifecycle
                    .app_paths()
                    .issue_root(PROJECT_ID, ISSUE_ID)
                    .join("automation-enrollment.json"),
                &enrollment,
            )
            .expect("disable enrollment");
            let before = fixture.enrollment().attention_facts.len();
            engine_tx
                .send(
                    crate::product::workspace_engine::EngineEvent::HumanGateOpened {
                        stage: "human_confirm".to_string(),
                    },
                )
                .await
                .expect("send gate open on disabled");
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            assert_eq!(
                fixture.enrollment().attention_facts.len(),
                before,
                "disabled enrollment must not receive new plan facts"
            );
        }

        /// TDD ③：control 无 enrollment → managed=false；GET 全程零写入。
        #[tokio::test]
        async fn control_reports_unmanaged_without_enrollment() {
            let inner = seed_fixture(1, true);
            let root_path = inner._root.path().to_path_buf();
            let state = WebAppState::new(
                root_path.clone(),
                crate::web::runtime::WebRuntime::new_fake(root_path.clone()),
            );
            let app = crate::web::app::build_web_router(state);
            let (status, payload) = get_json(&app, control_uri()).await;
            assert_eq!(status, StatusCode::OK, "control unmanaged: {payload}");
            assert_eq!(payload["managed"], false, "{payload}");
            assert_eq!(payload["stage"], "unmanaged", "{payload}");
            assert!(payload["stop_key"].is_null(), "{payload}");
            assert!(payload["allowed_actions"].as_array().unwrap().is_empty());
        }

        /// X 修复（读取方降级）：绑定会话已被删除（实体级联删除清盘）时，
        /// control 投影降级为 unmanaged，不得把已删 id 抛成 404/500。
        #[tokio::test]
        async fn control_degrades_when_bound_session_deleted() {
            let fixture = continue_fixture(
                PlanApprovalPolicy::Auto,
                crate::product::models::WorkspaceSessionStatus::Open,
                crate::product::models::SingleCandidatePhase::Prepare,
                None,
                false,
            )
            .await;
            let root_path = fixture._root.path().to_path_buf();
            let state = WebAppState::new(
                root_path.clone(),
                crate::web::runtime::WebRuntime::new_fake(root_path.clone()),
            );
            let app = crate::web::app::build_web_router(state);
            // 模拟级联删除后的磁盘状态：会话文件已清、enrollment 仍指向它。
            std::fs::remove_file(
                fixture
                    .lifecycle
                    .app_paths()
                    .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                    .join("workspace-sessions")
                    .join(format!("{}.json", fixture.session_id)),
            )
            .expect("remove bound session file");

            let (status, payload) = get_json(&app, control_uri()).await;
            assert_eq!(status, StatusCode::OK, "control degrade: {payload}");
            assert_eq!(payload["managed"], false, "{payload}");
            assert_eq!(payload["stage"], "unmanaged", "{payload}");
        }
    }
