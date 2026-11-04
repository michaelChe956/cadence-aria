// 经 automation_enrollment.rs 的 tests mod `include!` 引入（large_file_guard
// 1200 行红线拆分，纯移动自原文件 2350-2916 行，内容逐字不变）。
    /// k3 波5 F1（P2）：Disable 态（enabled=false 且未终止）不是托管——
    /// coding WS 管理投影 owner=client（与 REST ownership 同判据）、abort
    /// REST 不路由统一终止；Terminate 后 owner 仍 server（terminated 仍
    /// managed）。
    #[tokio::test]
    async fn disabled_enrollment_projects_client_owner_and_keeps_manual_abort() {
        use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
        use crate::product::models::automation::AutomationOwner;
        use crate::web::coding_ws_handler::build_coding_session_state;
        use crate::web::workspace_ws_types::ProviderConfigSnapshot;
        use tower::ServiceExt;

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
        let mut group = attempt.clone();
        group.work_item_group_id = Some(plan_id.clone());
        coding_store.write_coding_attempt_for_test(&group).unwrap();
        // 快照装配的 plan-repair reconcile 需要最小 lineage。
        crate::product::work_item_revision_store::WorkItemRevisionStore::new(fixture.paths.clone())
            .put_plan_lineage(
                &crate::product::models::work_item_revision::WorkItemPlanLineage {
                    id: plan_id.clone(),
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    story_spec_refs: Vec::new(),
                    design_spec_refs: Vec::new(),
                    active_revision_id: None,
                    active_amendment_id: None,
                    created_at: "2026-10-16T00:00:00Z".to_string(),
                    updated_at: "2026-10-16T00:00:00Z".to_string(),
                },
            )
            .unwrap();

        let ws_automation = |coding_store: &CodingAttemptStore| {
            let frame = build_coding_session_state(coding_store, group.clone()).unwrap();
            match frame {
                crate::web::coding_ws_handler::CodingWsOutMessage::CodingSessionState {
                    automation,
                    ..
                } => automation,
                _ => panic!("expected coding session state frame"),
            }
        };

        // Disable 态：owner=client（其余投影同判据），abort REST 不路由终止。
        let disable = serde_json::json!({
            "expected_revision": bound_revision,
            "command": {"type": "disable"}
        });
        let response = put_enrollment(&app, disable).await;
        assert_eq!(response.status(), StatusCode::OK);
        let disabled = ws_automation(&coding_store).expect("bound projection present");
        assert_eq!(disabled.owner, AutomationOwner::Client);
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/coding-attempts/{}/abort",
                        group.id
                    ))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let payload = response_json(response).await;
        assert_ne!(
            payload["code"], "automation_plan_termination_required",
            "disabled enrollment must not route single-attempt abort to termination"
        );

        // 重新 Enable 后终止：terminated 仍 managed（owner=server）。
        // 重开已有 enrollment 必须携带当前 revision（null 只用于首建）。
        let current_revision = IssueAutomationStore::new(fixture.paths.clone())
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap()
            .policy_revision;
        let mut re_enable = enrollment_body(&fixture, 1, 1);
        re_enable["expected_revision"] = serde_json::json!(current_revision);
        let enable = put_enrollment(&app, re_enable).await;
        let enable_status = enable.status();
        let enable_body = response_json(enable).await;
        assert_eq!(
            enable_status,
            StatusCode::OK,
            "re-enable body: {enable_body:?}"
        );
        let enabled_revision = enable_body["policy_revision"].as_u64().unwrap();
        let terminate = post_terminate(
            &app,
            &serde_json::json!({
                "command_id": "cmd-t3-f1-terminate",
                "expected_enrollment_id": enrollment["enrollment_id"],
                "expected_policy_revision": enabled_revision,
                "confirmed": true,
            }),
        )
        .await;
        assert_eq!(terminate.status(), StatusCode::OK);
        let terminated = ws_automation(&coding_store).expect("bound projection present");
        assert_eq!(terminated.owner, AutomationOwner::Server);
        assert!(terminated.terminated_at.is_some());
    }

    // ─── REQ-WIGA-09 修订：terminated 人工重新开启的收口门 ───

    /// 收口门红测：绑定 plan 仍有未终结 attempt（未收口运行事实）→ 409
    /// `automation_plan_cleanup_required` 零副作用（enrollment 仍 terminated、
    /// REQ-WPS-06 修订：terminated 人工重新开启**不再**因「仍有未终结
    /// attempt」永久阻断——取代删除路径先按既有终止收口语义把 attempt 收口
    /// 到终态，再随旧 plan 一并删除；旧 plan 消失、intent 轮换、新代不复用
    /// 旧资源（旧 plan 只留一条无内容删除事实）。
    #[tokio::test]
    async fn terminated_reenable_closes_and_supersedes_unfinished_attempts() {
        use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
        use crate::web::workspace_ws_types::ProviderConfigSnapshot;

        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enabled = response_json(put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await).await;
        let revision = enabled["policy_revision"].as_u64().unwrap();
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

        // 活跃 attempt（Created）归属该 plan：未收口运行事实。
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
        let mut linked = attempt.clone();
        linked.work_item_group_id = Some(plan_id.clone());
        coding_store.write_coding_attempt_for_test(&linked).unwrap();

        // 只走 T1 store CAS 终止（模拟引擎侧收口失败的真实 durable 状态：
        // terminated 落盘但活跃 attempt 未被收口）。REST 终止的成功路径会把
        // 每个 attempt 终结，本用例专测收口门对「未终结运行事实」的拦截。
        let store = IssueAutomationStore::new(fixture.paths.clone());
        store
            .terminate_plan(
                PROJECT_ID,
                ISSUE_ID,
                crate::product::models::automation::EnrollmentTerminationRequest {
                    command_id: "cmd-reenable-gate-terminate".to_string(),
                    expected_enrollment_id: enabled["enrollment_id"].as_str().unwrap().to_string(),
                    expected_policy_revision: bound_revision,
                    reason: "termination cleanup not finished".to_string(),
                },
            )
            .unwrap();
        let terminated = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        assert!(terminated.terminated_at.is_some());
        let terminated_revision = terminated.policy_revision;

        let mut reenable = enrollment_body(&fixture, 1, 1);
        reenable["expected_revision"] = serde_json::json!(terminated_revision);
        reenable["command"]["command_id"] = serde_json::json!("cmd-reenable-gate-1");
        let response = put_enrollment(&app, reenable).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "未终结 attempt 不得永久阻断取代（REQ-WPS-06）"
        );
        let payload = response_json(response).await;
        assert!(payload["terminated_at"].is_null());

        // 旧代 attempt 已收口并随旧 plan 删除（不复用旧 attempt identity）。
        let remaining = coding_store
            .list_attempts_for_plan(PROJECT_ID, ISSUE_ID, &plan_id)
            .unwrap();
        assert!(
            remaining.is_empty(),
            "旧代 attempt 必须随取代删除：{remaining:?}"
        );

        // 旧 plan 已删除，只留一条带触发原因的无内容删除事实。
        let scope = crate::product::entity_deletion_store::EntityScope::new(
            PROJECT_ID,
            ISSUE_ID,
            crate::product::entity_deletion_store::DeletionEntityKind::Plan,
            &plan_id,
        );
        let tombstone =
            crate::product::entity_deletion_store::EntityDeletionStore::new(fixture.paths.clone())
                .get(&scope)
                .unwrap()
                .expect("取代删除墓碑");
        assert_eq!(
            tombstone.phase,
            crate::product::entity_deletion_store::DeletionPhase::Deleted
        );
        assert_eq!(
            tombstone.trigger_reason.as_deref(),
            Some("re_enable_prepare_intent_rotation")
        );
        assert!(
            LifecycleStore::new(fixture.paths.clone())
                .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, &plan_id)
                .is_err(),
            "旧 plan 记录必须不存在"
        );

        // enrollment 已重新开启：终止态清空、绑定解绑（prepare_intent_id 轮换）。
        let current = IssueAutomationStore::new(fixture.paths.clone())
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        assert!(current.terminated_at.is_none());
        assert!(current.enabled);
        assert!(current.plan_id.is_none());
        assert_ne!(
            current.prepare_intent_id, terminated.prepare_intent_id,
            "prepare_intent_id 必须轮换（新链不复用被终止的 plan）"
        );
    }

    /// 收口门放行红测：绑定实体已删（墓碑）且无未终结 attempt（仅终态
    /// attempt）→ 允许人工显式取代，取代后 terminated_at 清空、revision+1、
    /// plan/session 解绑。
    #[tokio::test]
    async fn terminated_reenable_succeeds_when_bound_entity_is_tombstoned() {
        use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
        use crate::product::coding_models::CodingAttemptStatus;
        use crate::web::workspace_ws_types::ProviderConfigSnapshot;

        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enabled = response_json(put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await).await;
        let revision = enabled["policy_revision"].as_u64().unwrap();
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

        // 终态 attempt（Completed）不构成未收口运行事实。
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
        let mut linked = attempt.clone();
        linked.work_item_group_id = Some(plan_id.clone());
        linked.status = CodingAttemptStatus::Completed;
        coding_store.write_coding_attempt_for_test(&linked).unwrap();

        let terminate = post_terminate(
            &app,
            &serde_json::json!({
                "command_id": "cmd-reenable-tombstone-terminate",
                "expected_enrollment_id": enabled["enrollment_id"],
                "expected_policy_revision": bound_revision,
                "confirmed": true,
            }),
        )
        .await;
        assert_eq!(terminate.status(), StatusCode::OK);
        let terminated = IssueAutomationStore::new(fixture.paths.clone())
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        assert!(terminated.terminated_at.is_some());
        let terminated_revision = terminated.policy_revision;

        // 绑定实体删除（墓碑）：plan/session 记录消失，attempt 仍是终态。
        fixture
            .lifecycle
            .delete_issue_work_item_plan(PROJECT_ID, ISSUE_ID, &plan_id)
            .unwrap();

        let mut reenable = enrollment_body(&fixture, 1, 1);
        reenable["expected_revision"] = serde_json::json!(terminated_revision);
        reenable["command"]["command_id"] = serde_json::json!("cmd-reenable-tombstone-1");
        let response = put_enrollment(&app, reenable).await;
        let status = response.status();
        let payload = response_json(response).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "tombstoned binding without unfinished attempts must allow re-enable: {payload:?}"
        );
        assert!(payload["enabled"].as_bool().unwrap());
        assert!(payload["terminated_at"].is_null());
        assert_eq!(
            payload["policy_revision"].as_u64().unwrap(),
            terminated_revision + 1
        );
        assert!(
            payload["plan_id"].is_null(),
            "supersede unbinds the old plan"
        );
        assert!(payload["session_id"].is_null());
    }

    /// 主路径红测：REST 终止（收口成功）后，人工携带稳定 command_id 的
    /// Enable 以同 enrollment_id 取代重开——200、terminated_at 清空、
    /// revision+1、plan/session 解绑、binding 换代、账本留痕；同命令同
    /// payload 重放返回首个 durable 结果（revision 不再 +1、账本不新增）。
    #[tokio::test]
    async fn terminated_reenable_after_clean_termination_succeeds_and_replays() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enabled = response_json(put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await).await;
        let revision = enabled["policy_revision"].as_u64().unwrap();
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

        let terminate = post_terminate(
            &app,
            &serde_json::json!({
                "command_id": "cmd-reenable-clean-terminate",
                "expected_enrollment_id": enabled["enrollment_id"],
                "expected_policy_revision": bound_revision,
                "confirmed": true,
            }),
        )
        .await;
        assert_eq!(terminate.status(), StatusCode::OK);
        let terminate_payload = response_json(terminate).await;
        assert_eq!(terminate_payload["outcome"], "terminated");

        let store = IssueAutomationStore::new(fixture.paths.clone());
        let terminated = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        let terminated_revision = terminated.policy_revision;

        let mut reenable = enrollment_body(&fixture, 1, 1);
        reenable["expected_revision"] = serde_json::json!(terminated_revision);
        reenable["command"]["command_id"] = serde_json::json!("cmd-reenable-clean-1");
        let response = put_enrollment(&app, reenable.clone()).await;
        let status = response.status();
        let payload = response_json(response).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "clean termination allows re-enable: {payload:?}"
        );
        assert!(payload["enabled"].as_bool().unwrap());
        assert!(payload["terminated_at"].is_null());
        assert_eq!(
            payload["policy_revision"].as_u64().unwrap(),
            terminated_revision + 1
        );
        assert_eq!(payload["enrollment_id"], enabled["enrollment_id"]);
        assert!(payload["plan_id"].is_null());
        assert!(payload["session_id"].is_null());
        assert_eq!(
            payload["binding_history"]["current"]["binding_version"]
                .as_u64()
                .unwrap(),
            2
        );
        let ledger_entries: Vec<_> = payload["command_ledger"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["command_id"] == "cmd-reenable-clean-1")
            .collect();
        assert_eq!(ledger_entries.len(), 1);
        assert_eq!(ledger_entries[0]["state"], "accepted");

        // 同命令同 payload 重放：200 首个 durable 结果，零写入。
        let replay = put_enrollment(&app, reenable).await;
        assert_eq!(replay.status(), StatusCode::OK);
        let replay_payload = response_json(replay).await;
        assert_eq!(
            replay_payload, payload,
            "replay returns the durable record unchanged"
        );
    }

    /// 未收口运行事实红测：terminated 记录仍挂着 `stage=termination` 的
    /// 未收口事实（终止清理失败的真实产物）→ 409
    /// `automation_plan_cleanup_required` 零副作用；该事实 resolve 后即可
    /// 重新开启。
    #[tokio::test]
    async fn terminated_reenable_blocks_on_unclosed_run_facts() {
        use crate::product::models::automation::{
            AutomationAttentionFact, AutomationAttentionKind, AutomationAttentionStage,
        };

        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enabled = response_json(put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await).await;
        let revision = enabled["policy_revision"].as_u64().unwrap();
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

        let terminate = post_terminate(
            &app,
            &serde_json::json!({
                "command_id": "cmd-reenable-fact-terminate",
                "expected_enrollment_id": enabled["enrollment_id"],
                "expected_policy_revision": bound_revision,
                "confirmed": true,
            }),
        )
        .await;
        assert_eq!(terminate.status(), StatusCode::OK);

        // 终止清理失败的真实产物：绑定实体仍在，挂着 stage=termination 事实。
        let store = IssueAutomationStore::new(fixture.paths.clone());
        let terminated = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        let binding = terminated.binding_history.as_ref().unwrap().current.clone();
        let fact_key = format!("{plan_id}:cmd-reenable-fact-terminate:termination_cleanup_failed");
        store
            .upsert_automation_attention_fact(
                PROJECT_ID,
                ISSUE_ID,
                &binding,
                AutomationAttentionFact {
                    key: fact_key.clone(),
                    kind: AutomationAttentionKind::AutomationFailed,
                    stage: AutomationAttentionStage::Termination,
                    plan_id: Some(plan_id.clone()),
                    session_id: Some(session_id.clone()),
                    attempt_id: None,
                    reason_code: "termination_cleanup_failed".to_string(),
                    occurred_at: "2026-10-16T00:00:00Z".to_string(),
                    choice_id: None,
                    gate_id: None,
                },
            )
            .unwrap();
        let terminated_revision = terminated.policy_revision;

        let mut reenable = enrollment_body(&fixture, 1, 1);
        reenable["expected_revision"] = serde_json::json!(terminated_revision);
        reenable["command"]["command_id"] = serde_json::json!("cmd-reenable-fact-1");
        let response = put_enrollment(&app, reenable.clone()).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_plan_cleanup_required");
        assert_eq!(payload["details"]["reason"], "unclosed_run_facts");
        let current = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        assert!(current.terminated_at.is_some());
        assert_eq!(current.policy_revision, terminated_revision);

        // 收口后即可重新开启。
        store
            .resolve_automation_attention_fact(PROJECT_ID, ISSUE_ID, &binding, &fact_key)
            .unwrap();
        let response = put_enrollment(&app, reenable).await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert!(payload["enabled"].as_bool().unwrap());
        assert!(payload["terminated_at"].is_null());
    }

    // ─── REQ-WIGA-SWAP-02：冲突原因判别位 + HTTP 分流（块1 片3）────────
    //
    // 同一个 409 承载 9 类语义；判别位把「运行中不能换 payload」（该先终止）
    // 与「版本号过期」（该刷新后重试）分成两个稳定码，其余 7 类落回旧码
    // `automation_enrollment_conflict`（旧码被终止自愈逻辑钉住，不得改名）。

    /// 统一终止 REST 的 POST helper（与 automation_enrollment_conflict.rs 同形）。
    async fn post_terminate_request(
        app: &axum::Router,
        body: &serde_json::Value,
    ) -> axum::http::Response<axum::body::Body> {
        use tower::ServiceExt;

        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/terminate"
                    ))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    /// 播种一个已 enable 的 enrollment，返回 (fixture, app, enrollment, revision)。
    async fn seed_enabled_enrollment() -> (
        crate::web::handlers::automation_enrollment_test_support::Fixture,
        axum::Router,
        serde_json::Value,
        u64,
    ) {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let enrollment = response_json(enable).await;
        let revision = enrollment["policy_revision"].as_u64().unwrap();
        (fixture, app, enrollment, revision)
    }

    /// 换 provider（claude_code）的写入必须先过 Enable 前的角色链同源预检；
    /// 夹具默认 Fake 豁免，换 provider 需物化「已探测」的 Confirmed 行（与
    /// automation_reenable_supersede_tests 同构），否则 422
    /// `automation_role_chain_unsupported` 拦在 CAS 冲突判定之前。
    fn seed_claude_code_launch_capability(
        fixture: &crate::web::handlers::automation_enrollment_test_support::Fixture,
    ) {
        let paths = fixture.paths.clone();
        let lc_id = crate::product::logical_codebase::RepositoryAuthorityResolver::new(
            paths.clone(),
        )
        .resolve_for_issue(PROJECT_ID, ISSUE_ID)
        .expect("resolve carrier")
        .expect("logical carrier")
        .target
        .logical_codebase_id
        .expect("resolver-frozen lc id");
        crate::product::logical_codebase::ProviderCapabilityStore::for_lc(paths.clone(), &lc_id)
            .ensure_bootstrap(PROJECT_ID)
            .expect("capability bootstrap");
        super::super::automation_enrollment_test_support::seed_confirmed_capability_records(
            &paths,
            PROJECT_ID,
            Some(&lc_id),
            &[crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode],
        );
    }


    /// ① 运行中改 payload（换 author provider）必须落到「运行态限制」稳定码，
    /// 而不是笼统的 `automation_enrollment_conflict`——两者对应完全不同的
    /// 用户动作（该先终止自动化 vs 该刷新后重试）。
    #[tokio::test]
    async fn enrollment_write_with_changed_provider_while_running_reports_payload_immutable() {
        let (fixture, app, _enrollment, revision) = seed_enabled_enrollment().await;
        seed_claude_code_launch_capability(&fixture);

        let mut body = enrollment_body(&fixture, 1, 1);
        body["expected_revision"] = serde_json::json!(revision);
        body["command"]["options"]["author_provider"] = serde_json::json!("claude_code");

        let response = put_enrollment(&app, body).await;
        let status = response.status();
        let payload = response_json(response).await;
        assert_eq!(status, StatusCode::CONFLICT, "{payload}");
        assert_eq!(
            payload["code"], "automation_enrollment_payload_immutable",
            "运行中改 payload 必须分流到运行态限制码：{payload}"
        );
    }

    /// ② 版本号过期必须落到「可重试」稳定码 `automation_enrollment_revision_stale`。
    #[tokio::test]
    async fn enrollment_write_with_stale_revision_reports_revision_stale() {
        let (fixture, app, _enrollment, revision) = seed_enabled_enrollment().await;

        let mut body = enrollment_body(&fixture, 1, 1);
        body["expected_revision"] = serde_json::json!(revision + 1);

        let response = put_enrollment(&app, body).await;
        let status = response.status();
        let payload = response_json(response).await;
        assert_eq!(status, StatusCode::CONFLICT, "{payload}");
        assert_eq!(
            payload["code"], "automation_enrollment_revision_stale",
            "版本过期必须分流到可重试码：{payload}"
        );
    }

    /// ③ 两条路径必须返回**不同**的稳定码（否则分流没生效，等于仍是一个 409）。
    #[tokio::test]
    async fn payload_immutable_and_revision_stale_report_distinct_codes() {
        let (fixture, app, _enrollment, revision) = seed_enabled_enrollment().await;
        seed_claude_code_launch_capability(&fixture);

        let mut changed_provider = enrollment_body(&fixture, 1, 1);
        changed_provider["expected_revision"] = serde_json::json!(revision);
        changed_provider["command"]["options"]["author_provider"] = serde_json::json!("claude_code");
        let first = put_enrollment(&app, changed_provider).await;
        let first_status = first.status();
        let first_payload = response_json(first).await;

        let mut stale_revision = enrollment_body(&fixture, 1, 1);
        stale_revision["expected_revision"] = serde_json::json!(revision + 1);
        let second = put_enrollment(&app, stale_revision).await;
        let second_status = second.status();
        let second_payload = response_json(second).await;

        assert_eq!(first_status, StatusCode::CONFLICT, "{first_payload}");
        assert_eq!(second_status, StatusCode::CONFLICT, "{second_payload}");
        assert_ne!(
            first_payload["code"], second_payload["code"],
            "运行中改 payload 与版本过期必须落到不同稳定码：{first_payload} / {second_payload}"
        );
    }

    /// ④ 其余 7 类冲突必须**落回旧码** `automation_enrollment_conflict`
    /// （旧码被终止自愈逻辑钉住，改名会静默失效）。本例走绑定面的
    /// 「绑定身份/版本不符」类。
    #[tokio::test]
    async fn binding_write_conflict_keeps_legacy_conflict_code() {
        let (fixture, app, _enrollment, revision) = seed_enabled_enrollment().await;
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::Interactive);

        let bound = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision + 1,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        let status = bound.status();
        let payload = response_json(bound).await;
        assert_eq!(status, StatusCode::CONFLICT, "{payload}");
        assert_eq!(
            payload["code"], "automation_enrollment_conflict",
            "非运行态限制/非版本过期的冲突必须落回旧码：{payload}"
        );
    }

    /// ⑤ 终止确认面遇到授权版本冲突 → 仍是旧码 `automation_enrollment_conflict`
    /// （终止自愈仍生效；终止路径不按判别位分流）。
    #[tokio::test]
    async fn terminate_confirm_with_stale_revision_keeps_legacy_conflict_code() {
        let (_fixture, app, enrollment, revision) = seed_enabled_enrollment().await;

        let response = post_terminate_request(
            &app,
            &serde_json::json!({
                "command_id": "cmd-terminate-stale-revision",
                "expected_enrollment_id": enrollment["enrollment_id"],
                "expected_policy_revision": revision + 1,
                "confirmed": true,
            }),
        )
        .await;
        let status = response.status();
        let payload = response_json(response).await;
        assert_eq!(status, StatusCode::CONFLICT, "{payload}");
        assert_eq!(
            payload["code"], "automation_enrollment_conflict",
            "终止确认面必须仍是旧码（终止自愈依赖旧码）：{payload}"
        );
    }

    /// ⑥ 判别位**没被吃掉**：`CasResolution` → `EnrollmentError` 这一跳必须
    /// 保住原因；判别位一旦丢失，两条路径都会退回旧码（断言即红）。
    #[tokio::test]
    async fn conflict_kind_survives_the_store_to_http_hop() {
        let (fixture, app, _enrollment, revision) = seed_enabled_enrollment().await;
        seed_claude_code_launch_capability(&fixture);

        let mut changed_provider = enrollment_body(&fixture, 1, 1);
        changed_provider["expected_revision"] = serde_json::json!(revision);
        changed_provider["command"]["options"]["author_provider"] = serde_json::json!("claude_code");
        let first = put_enrollment(&app, changed_provider).await;
        let first_payload = response_json(first).await;

        let mut stale_revision = enrollment_body(&fixture, 1, 1);
        stale_revision["expected_revision"] = serde_json::json!(revision + 1);
        let second = put_enrollment(&app, stale_revision).await;
        let second_payload = response_json(second).await;

        assert_eq!(
            first_payload["code"], "automation_enrollment_payload_immutable",
            "判别位在 store→HTTP 途中被吃掉：{first_payload}"
        );
        assert_eq!(
            second_payload["code"], "automation_enrollment_revision_stale",
            "判别位在 store→HTTP 途中被吃掉：{second_payload}"
        );
    }
