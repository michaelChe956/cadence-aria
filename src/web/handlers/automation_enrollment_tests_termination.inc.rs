// 经 automation_enrollment.rs 的 tests mod `include!` 引入（large_file_guard
// 1200 行红线拆分，纯移动自原文件 1850-2349 行，内容逐字不变）。
    // ---- P1 WIGA Task 4：enrollment-bound 唯一创建与半提交恢复 ----

    use crate::product::issue_automation_store::IssueAutomationStore;
    use crate::product::json_store::write_json;
    use crate::product::lifecycle_store::{CreateIssueWorkItemPlanInput, LifecycleStore};
    use crate::product::models::automation::{IssueAutomationEnrollment, PreparedPlanIntent};
    use crate::product::models::{
        IssueWorkItemPlan, IssueWorkItemPlanStatus, WorkspaceSessionRecord,
    };
    use crate::web::error::ApiResult;
    use crate::web::handlers::lifecycle::plan_preparation::ensure_enrolled_plan;
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;

    impl super::super::automation_enrollment_test_support::Fixture {
        /// 中窗模拟：按当前 enrollment 派生并持久化真实意图文件，再执行共用
        /// prepare 的 plan 写入步骤（无 session、无绑定）——崩溃于 plan 已写、
        /// session 未写之间，不伪造绑定。
        fn seed_intent_and_plan_without_session(&self, expected_plan_id: &str) {
            let enrollment = IssueAutomationStore::new(self.paths.clone())
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap();
            let target = enrollment
                .target
                .clone()
                .expect("fixture enrollment declares a target");
            let intent = PreparedPlanIntent::from_enrollment(&enrollment, &target);
            assert_eq!(intent.plan_id, expected_plan_id);
            write_json(
                &self
                    .paths
                    .issue_root(PROJECT_ID, ISSUE_ID)
                    .join("automation-plan-intent.json"),
                &intent,
            )
            .unwrap();
            LifecycleStore::new(self.paths.clone())
                .ensure_issue_work_item_plan_with_identity(CreateIssueWorkItemPlanInput {
                    id: Some(intent.plan_id.clone()),
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    source_story_spec_ids: intent
                        .source
                        .stories
                        .iter()
                        .map(|reference| reference.id.clone())
                        .collect(),
                    source_design_spec_ids: intent
                        .source
                        .designs
                        .iter()
                        .map(|reference| reference.id.clone())
                        .collect(),
                    options: intent.options.plan_options.clone(),
                    status: IssueWorkItemPlanStatus::Draft,
                    work_item_ids: Vec::new(),
                    repository_profile_ref: None,
                    verification_plan_ids: Vec::new(),
                    dependency_graph: Vec::new(),
                    created_from_provider_run: None,
                    validator_findings: Vec::new(),
                })
                .unwrap();
        }

        /// 以重建的 state/store 执行后台补偿，返回补偿后的 enrollment 投影。
        async fn ensure_enrolled_plan(&self) -> ApiResult<IssueAutomationEnrollment> {
            let root = self._root.path();
            let state =
                WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()));
            let enrollment = IssueAutomationStore::new(self.paths.clone())
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap();
            ensure_enrolled_plan(&state, &enrollment).await?;
            IssueAutomationStore::new(self.paths.clone())
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .ok_or_else(|| {
                    crate::web::error::ApiError::runtime(
                        "automation_enrollment_not_found",
                        "enrollment vanished after compensation",
                        serde_json::json!({}),
                    )
                })
        }

        fn plans(&self) -> Vec<IssueWorkItemPlan> {
            self.lifecycle
                .list_issue_work_item_plans(PROJECT_ID, ISSUE_ID)
                .unwrap()
        }

        fn sessions(&self) -> Vec<WorkspaceSessionRecord> {
            self.lifecycle
                .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
                .unwrap()
        }
    }

    #[tokio::test]
    async fn automation_prepare_recovers_plan_without_session_or_rebinding() {
        let fixture = seed_fixture(1, true);
        let enrolled = response_json(
            put_enrollment(&fixture.router(), enrollment_body_with_target(&fixture)).await,
        )
        .await;
        let intent_id = enrolled["prepare_intent_id"].as_str().unwrap();
        let expected_plan_id = format!("issue_work_item_plan_auto_{intent_id}");
        fixture.seed_intent_and_plan_without_session(&expected_plan_id);

        // 重建进程后的两次补偿：绑定同一 plan/session，plan 文件不被覆盖。
        let recovered = fixture.ensure_enrolled_plan().await.unwrap();
        let again = fixture.ensure_enrolled_plan().await.unwrap();
        assert_eq!(recovered.plan_id, again.plan_id);
        assert_eq!(recovered.session_id, again.session_id);
        assert_eq!(
            recovered.plan_id.as_deref(),
            Some(expected_plan_id.as_str())
        );
        assert_eq!(
            recovered.session_id.as_deref(),
            Some(format!("workspace_session_auto_{intent_id}").as_str())
        );
        assert_eq!(fixture.plans().len(), 1);
        assert_eq!(fixture.sessions().len(), 1);
        assert_eq!(fixture.sessions()[0].run_policy, RunPolicy::Interactive);
    }

    /// Disable → 异 payload 重开：旧 intent 冻结快照与新授权不匹配 → 补偿
    /// fail-closed，原 plan 内容/创建时间不变。
    #[tokio::test]
    async fn automation_prepare_fails_closed_on_divergent_reopen() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        put_enrollment(&app, enrollment_body_with_target(&fixture)).await;
        fixture.ensure_enrolled_plan().await.unwrap();
        let plan_before = fixture.plans().remove(0);

        let revision = IssueAutomationStore::new(fixture.paths.clone())
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
        let mut divergent = enrollment_body_with_target(&fixture);
        divergent["command"]["options"]["review_rounds"] = serde_json::json!(2);
        divergent["expected_revision"] = serde_json::json!(revision + 1);
        let response = put_enrollment(&app, divergent).await;
        assert_eq!(response.status(), StatusCode::OK);

        let error = fixture.ensure_enrolled_plan().await.unwrap_err();
        assert_eq!(error.code, "automation_enrollment_conflict");
        let plans = fixture.plans();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].id, plan_before.id);
        assert_eq!(plans[0].created_at, plan_before.created_at);
        assert_eq!(plans[0].options, plan_before.options);
    }

    // ─── 批 2 T3：统一终止 REST（REQ-WIGA-09/11、REQ-WIGA-08/12）───

    async fn post_terminate(
        app: &axum::Router,
        body: &serde_json::Value,
    ) -> axum::http::Response<axum::body::Body> {
        use axum::body::Body;
        use tower::ServiceExt;

        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/terminate"
                    ))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    /// T3 红测①：未带 confirmed:true → 428 `automation_plan_termination_required`
    ///（details 含 confirm_required/plan/enrollment 身份），零副作用。
    #[tokio::test]
    async fn terminate_rest_requires_confirmation_with_zero_side_effects() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let enrollment = response_json(enable).await;
        let revision = enrollment["policy_revision"].as_u64().unwrap();
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let bound = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(bound.status(), StatusCode::OK);

        let response = post_terminate(
            &app,
            &serde_json::json!({
                "command_id": "cmd-t3-terminate-1",
                "expected_enrollment_id": enrollment["enrollment_id"],
                "expected_policy_revision": revision + 1,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_plan_termination_required");
        assert_eq!(payload["details"]["confirm_required"], true);
        assert_eq!(payload["details"]["plan_id"], plan_id.as_str());
        assert_eq!(
            payload["details"]["enrollment_id"],
            enrollment["enrollment_id"]
        );

        // 零副作用：enrollment 未终止未禁用，session 仍 Open。
        let current = IssueAutomationStore::new(fixture.paths.clone())
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        assert!(current.enabled);
        assert!(current.terminated_at.is_none());
        assert_eq!(
            crate::product::lifecycle_store::LifecycleStore::new(fixture.paths.clone())
                .get_workspace_session(&session_id)
                .unwrap()
                .status,
            crate::product::models::WorkspaceSessionStatus::Open,
        );
    }

    /// T3 红测①②⑤⑥⑧：确认后整 plan 终止（session Terminated、三类旧事实
    /// 清空）、同 command 重放幂等（terminated_at 冻结、不复活）、terminated
    /// 仍 managed（同源管理投影）。
    #[tokio::test]
    async fn terminate_rest_confirmed_terminates_and_replays_idempotently() {
        use crate::product::models::WorkspaceSessionStatus;
        use crate::product::models::automation::{
            AutomationAttentionFact, AutomationAttentionKind, AutomationAttentionStage,
        };

        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        let enrollment = response_json(enable).await;
        let revision = enrollment["policy_revision"].as_u64().unwrap();
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let bound = response_json(
            post_binding(
                &app,
                serde_json::json!({
                    "expected_revision": revision,
                    "plan_id": plan_id,
                    "session_id": session_id,
                }),
            )
            .await,
        )
        .await;
        let bound_revision = bound["policy_revision"].as_u64().unwrap();

        // 播种当前绑定 plan 的三类旧事实之一（plan_question）。
        let store = IssueAutomationStore::new(fixture.paths.clone());
        let durable = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        let binding = durable.binding_history.as_ref().unwrap().current.clone();
        store
            .upsert_automation_attention_fact(
                PROJECT_ID,
                ISSUE_ID,
                &binding,
                AutomationAttentionFact {
                    key: format!("{plan_id}:cmd-seed:plan_question"),
                    kind: AutomationAttentionKind::PlanQuestion,
                    stage: AutomationAttentionStage::Plan,
                    plan_id: Some(plan_id.clone()),
                    session_id: Some(session_id.clone()),
                    attempt_id: None,
                    reason_code: "test".to_string(),
                    occurred_at: "2026-10-16T00:00:00Z".to_string(),
                    choice_id: None,
                    gate_id: None,
                },
            )
            .unwrap();

        let body = serde_json::json!({
            "command_id": "cmd-t3-terminate-2",
            "expected_enrollment_id": enrollment["enrollment_id"],
            "expected_policy_revision": bound_revision,
            "confirmed": true,
        });
        let response = post_terminate(&app, &body).await;
        assert_eq!(response.status(), StatusCode::OK);
        let first = response_json(response).await;
        assert_eq!(first["outcome"], "terminated");
        assert_eq!(first["state"], "accepted");
        assert_eq!(first["plan_id"], plan_id.as_str());
        let terminated_at = first["terminated_at"].as_str().unwrap().to_string();

        // session Terminated；enrollment 不可恢复终止；三类旧事实清空。
        assert_eq!(
            crate::product::lifecycle_store::LifecycleStore::new(fixture.paths.clone())
                .get_workspace_session(&session_id)
                .unwrap()
                .status,
            WorkspaceSessionStatus::Terminated,
        );
        let durable = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        assert!(!durable.enabled);
        assert_eq!(
            durable.terminated_at.as_deref(),
            Some(terminated_at.as_str())
        );
        assert!(
            durable.attention_facts.is_empty(),
            "termination must clear the bound plan's stale attention facts"
        );

        // terminated 仍 managed（同源管理投影；REST/WS 同一口径）。
        let ownership = store
            .ownership_for_ids(PROJECT_ID, ISSUE_ID, &session_id)
            .unwrap();
        assert_eq!(
            ownership.owner,
            crate::product::models::automation::AutomationOwner::Server
        );
        assert_eq!(
            ownership.terminated_at.as_deref(),
            Some(terminated_at.as_str())
        );

        // 同 command 同 payload 重放：Replayed、terminated_at 冻结、幂等。
        let replay = post_terminate(&app, &body).await;
        assert_eq!(replay.status(), StatusCode::OK);
        let replayed = response_json(replay).await;
        assert_eq!(replayed["state"], "replayed");
        assert_eq!(replayed["terminated_at"], terminated_at.as_str());

        // 重启后不复活：新进程替身 reconcile → NoEnrollment（terminated）。
        let state = crate::web::state::WebAppState::new(
            fixture._root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(fixture._root.path().to_path_buf()),
        );
        let worker = crate::web::autopilot_orchestrator::AutopilotOrchestrator::new(
            state,
            Default::default(),
        );
        let outcome = worker
            .reconcile(
                &crate::web::state::WebAppState::new(
                    fixture._root.path().to_path_buf(),
                    crate::web::runtime::WebRuntime::new_fake(fixture._root.path().to_path_buf()),
                ),
                PROJECT_ID,
                ISSUE_ID,
            )
            .await
            .unwrap();
        assert_eq!(
            outcome,
            crate::web::autopilot_orchestrator::ReconcileOutcome::NoEnrollment
        );
    }

    /// T3 红测②（C2 旧 abort REST 路由）：自动绑定 plan 上的旧 abort 入口
    /// 统一返回终止确认要求（零副作用）；manual/无绑定不走 428。
    #[tokio::test]
    async fn abort_rest_routes_managed_attempt_to_unified_termination() {
        use tower::ServiceExt;

        use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
        use crate::web::workspace_ws_types::ProviderConfigSnapshot;

        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        let enrollment = response_json(enable).await;
        let revision = enrollment["policy_revision"].as_u64().unwrap();
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let bound = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(bound.status(), StatusCode::OK);

        let coding_store = CodingAttemptStore::new(fixture.paths.clone());
        let attempt = coding_store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: crate::product::models::provider::ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 1,
            })
            .unwrap();
        // 独立 manual 对照 attempt（不同 work_item，无 group 绑定）。
        let manual_attempt = coding_store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0002".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/work-items/work_item_0002/attempt-1".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: crate::product::models::provider::ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 1,
            })
            .unwrap();
        let mut managed = attempt.clone();
        managed.work_item_group_id = Some(plan_id.clone());
        coding_store
            .write_coding_attempt_for_test(&managed)
            .unwrap();

        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/coding-attempts/{}/abort",
                        managed.id
                    ))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_plan_termination_required");
        assert_eq!(payload["details"]["plan_id"], plan_id.as_str());
        // 零副作用：attempt 状态未被单 attempt abort 改写。
        assert_eq!(
            coding_store
                .get_attempt(PROJECT_ID, ISSUE_ID, &managed.id)
                .unwrap()
                .status,
            crate::product::coding_models::CodingAttemptStatus::Created,
        );

        // manual 对照：无绑定 attempt 不返回 428（原语义错误面或成功）。
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/coding-attempts/{}/abort",
                        manual_attempt.id
                    ))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let manual_payload = response_json(response).await;
        assert_ne!(
            manual_payload["code"], "automation_plan_termination_required",
            "manual attempt abort must keep the original semantics"
        );
    }

