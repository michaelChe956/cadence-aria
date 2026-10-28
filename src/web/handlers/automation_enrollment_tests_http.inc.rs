// 经 automation_enrollment.rs 的 tests mod `include!` 引入（large_file_guard
// 1200 行红线拆分，纯移动自原文件 1223-1849 行，内容逐字不变）。
    #[tokio::test]
    async fn automation_enrollment_http_get_returns_null_when_absent() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        let response = get_enrollment(&app).await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert!(payload.is_null());
    }

    #[tokio::test]
    async fn automation_enrollment_http_put_rejects_unconfirmed_or_stale_source() {
        let fixture = seed_fixture(1, false);
        let app = fixture.router();

        // design 未确认 → 422，且不落 enrollment 文件。
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));

        // 确认 design 后，引用不存在的版本（2 > current 1）→ 422，不猜 latest。
        fixture
            .lifecycle
            .update_spec_confirmation_status(
                PROJECT_ID,
                ISSUE_ID,
                &fixture.design_id,
                LifecycleConfirmationStatus::Confirmed,
            )
            .unwrap();
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 2)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));

        // 未知 story id 同样拒绝。
        let mut body = enrollment_body(&fixture, 1, 1);
        body["command"]["source"]["stories"][0]["id"] = serde_json::json!("story_spec_9999");
        let response = put_enrollment(&app, body).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));
    }

    #[tokio::test]
    async fn automation_enrollment_http_put_is_idempotent_and_conflicts_on_divergence() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        let first = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(first.status(), StatusCode::OK);
        let first_body = response_json(first).await;
        assert_eq!(first_body["policy_revision"], 1);
        assert!(first_body["enabled"].as_bool().unwrap());

        // 同键同 payload 重发：200，同 enrollment_id/revision。
        let second = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(second.status(), StatusCode::OK);
        let second_body = response_json(second).await;
        assert_eq!(second_body["enrollment_id"], first_body["enrollment_id"]);
        assert_eq!(second_body["policy_revision"], 1);

        // 异 options + 旧 revision → 409，details 携带当前 revision。
        let mut divergent = enrollment_body(&fixture, 1, 1);
        divergent["command"]["options"]["review_rounds"] = serde_json::json!(2);
        divergent["expected_revision"] = serde_json::json!(99);
        let response = put_enrollment(&app, divergent).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_enrollment_conflict");
        assert_eq!(payload["details"]["current_revision"], 1);

        // Disable 后以旧 revision Enable → 409。
        let disable = serde_json::json!({
            "expected_revision": 1,
            "command": {"type": "disable"}
        });
        let response = put_enrollment(&app, disable).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await["policy_revision"], 2);
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // GET 投影读取当前（禁用）记录。
        let response = get_enrollment(&app).await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert_eq!(payload["enabled"], false);
        assert_eq!(payload["policy_revision"], 2);
    }

    /// C5 Task 1：Enable 必带 target——缺 target 的请求在反序列化层即
    /// 422 拒绝（旧 `logical_repository_id` 字段被忽略、不能替代），
    /// 零 enrollment 写入。
    #[tokio::test]
    async fn automation_enrollment_http_put_requires_target() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let mut body = enrollment_body(&fixture, 1, 1);
        assert!(
            body["command"]
                .as_object_mut()
                .unwrap()
                .remove("target")
                .is_some()
        );
        // 旧字段存在也不能替代 target。
        body["command"]["logical_repository_id"] = serde_json::json!(SINGLE_LOGICAL_ID);
        let response = put_enrollment(&app, body).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));
    }

    #[tokio::test]
    async fn automation_enrollment_http_put_rejects_zero_target_and_mismatched_member_set() {
        // 0 logical member（无 manifest/selection → Legacy 路由）→ 422。
        let fixture = seed_fixture(0, true);
        let app = fixture.router();
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));

        // 2 logical member 下提交旧单成员 target → 与 canonical 集合不符，
        // 422 且零写入（批 2 R1：多成员授权必须全集合匹配）。
        let fixture = seed_fixture(2, true);
        let app = fixture.router();
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_enrollment_invalid_scope");
        assert!(!enrollment_file_exists(&fixture));
    }

    /// 批 2 R1 红测⑤：2 成员 selection 的 PUT 集合 target——乱序/重复输入
    /// 归一后与 canonical 全等即 200 落盘（enrollment/binding 冻结同一规范
    /// 形态；错配成员子集 422 零写入）。
    #[tokio::test]
    async fn automation_enrollment_http_put_accepts_normalized_member_set() {
        let fixture = seed_fixture(2, true);
        let app = fixture.router();
        let mut body = enrollment_body(&fixture, 1, 1);
        body["command"]["target"] = serde_json::json!({
            "kind": "logical_codebase_members",
            "logical_codebase_id": fixture_logical_codebase_id(&fixture),
            // 乱序 + 重复：反序列化归一为升序唯一后与 canonical 全等。
            "logical_repository_ids": [
                "00000000-0000-0000-0000-000000000002",
                "00000000-0000-0000-0000-000000000001",
                "00000000-0000-0000-0000-000000000002"
            ]
        });
        let response = put_enrollment(&app, body).await;
        assert_eq!(response.status(), StatusCode::OK);
        let enrollment = response_json(response).await;
        assert_eq!(enrollment["target"]["kind"], "logical_codebase_members");
        assert_eq!(
            enrollment["target"]["logical_repository_ids"],
            serde_json::json!([
                "00000000-0000-0000-0000-000000000001",
                "00000000-0000-0000-0000-000000000002"
            ])
        );
        assert_eq!(
            enrollment["binding_history"]["current"]["target"]["kind"],
            "logical_codebase_members"
        );
        assert!(enrollment_file_exists(&fixture));

        // 成员子集（缺一）→ canonical 不全等即 422 零写入。
        let mut subset = enrollment_body(&fixture, 1, 1);
        subset["command"]["target"] = serde_json::json!({
            "kind": "logical_codebase_members",
            "logical_codebase_id": fixture_logical_codebase_id(&fixture),
            "logical_repository_ids": ["00000000-0000-0000-0000-000000000001"]
        });
        let response = put_enrollment(&app, subset).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(response).await["code"],
            "automation_enrollment_invalid_scope"
        );
    }

    #[tokio::test]
    async fn automation_enrollment_http_binding_binds_explicit_interactive_plan_session() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let enrollment = response_json(enable).await;
        let revision = enrollment["policy_revision"].as_u64().unwrap();

        // manual 缺省 enrollment（无 plan_approval 字段）+ AutoIfValid 会话
        // → 409（REQ-WIGA-02 修订后口径：AutoIfValid 只对 auto 冻结放行）。
        let (auto_plan, auto_session) = create_plan_and_session(&fixture, RunPolicy::AutoIfValid);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": auto_plan,
                "session_id": auto_session,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // Interactive plan/session 显式绑定成功。
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bound = response_json(response).await;
        assert_eq!(bound["plan_id"], plan_id.as_str());
        assert_eq!(bound["session_id"], session_id.as_str());
        let bound_revision = bound["policy_revision"].as_u64().unwrap();

        // 同键重试 → 200 同 revision；重绑他者 plan → 409。
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": bound_revision,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await["policy_revision"],
            bound_revision
        );

        let (other_plan, other_session) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": bound_revision,
                "plan_id": other_plan,
                "session_id": other_session,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // 绑定目标不存在（未知 plan/session）→ 404。
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": bound_revision,
                "plan_id": "plan_9999",
                "session_id": "session_9999",
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn automation_enrollment_http_binding_rejects_plan_created_before_enrollment() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        // 旧 plan（先于 enrollment 创建）不能被认领 → 422。
        let (old_plan, old_session) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let revision = response_json(enable).await["policy_revision"]
            .as_u64()
            .unwrap();
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": old_plan,
                "session_id": old_session,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    // ---- C1 Task 2：显式重绑/换代 REST（REQ-WIGA-01、REQ-C1-TARGET-01）----

    #[tokio::test]
    async fn automation_enrollment_rebind_requires_current_binding() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        // 新式 enable：显式声明 target（logical 双级齐全）→ binding v1。
        let enable = put_enrollment(&app, enrollment_body_with_target(&fixture)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let enrolled = response_json(enable).await;
        assert_eq!(enrolled["binding_history"]["current"]["binding_version"], 1);

        // v1 显式绑定 plan/session。
        let revision = enrolled["policy_revision"].as_u64().unwrap();
        let (plan_1, session_1) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": plan_1,
                "session_id": session_1,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bound_revision = response_json(response).await["policy_revision"]
            .as_u64()
            .unwrap();

        // 合法 rebind → 200 accepted：current v2、previous 逐字保留 v1、
        // enrollment 投影换新代、policy_revision 递增（旧回执必然失配）。
        let (plan_2, session_2) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let rebind = rebind_body(
            &fixture,
            "rebind_cmd_0001",
            bound_revision,
            1,
            &plan_2,
            &session_2,
        );
        let response = post_rebind(&app, rebind.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert_eq!(payload["command_id"], "rebind_cmd_0001");
        assert_eq!(payload["state"], "accepted");
        let after = payload["enrollment"].clone();
        assert_eq!(after["binding_history"]["current"]["binding_version"], 2);
        assert_eq!(
            after["binding_history"]["current"]["plan_id"],
            plan_2.as_str()
        );
        assert_eq!(
            after["binding_history"]["previous"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            after["binding_history"]["previous"][0]["plan_id"],
            plan_1.as_str()
        );
        assert_eq!(
            after["binding_history"]["previous"][0]["session_id"],
            session_1.as_str()
        );
        assert_eq!(after["plan_id"], plan_2.as_str());
        assert!(after["policy_revision"].as_u64().unwrap() > bound_revision);

        // durable 只有一份当前版本：GET 投影与返回一致。
        assert_eq!(response_json(get_enrollment(&app).await).await, after);

        // 同 command 同 payload 重放 → replayed，durable 不变。
        let replay = post_rebind(&app, rebind.clone()).await;
        assert_eq!(replay.status(), StatusCode::OK);
        let replay_body = response_json(replay).await;
        assert_eq!(replay_body["state"], "replayed");
        assert_eq!(replay_body["enrollment"], after);
        assert_eq!(response_json(get_enrollment(&app).await).await, after);

        // 同 command 异 payload（完整换绑定目标）→ 409，durable 不变。
        let mut diverged = rebind.clone();
        diverged["binding"]["plan_id"] = serde_json::json!(plan_1);
        diverged["binding"]["session_id"] = serde_json::json!(session_1);
        let response = post_rebind(&app, diverged).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(response_json(get_enrollment(&app).await).await, after);

        // 旧 expected binding version（当前已是 v2）→ 409。
        let after_revision = after["policy_revision"].as_u64().unwrap();
        let stale_binding = rebind_body(
            &fixture,
            "rebind_cmd_0002",
            after_revision,
            1,
            &plan_1,
            &session_1,
        );
        let response = post_rebind(&app, stale_binding).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // 旧 expected policy revision → 409。
        let stale_policy = rebind_body(
            &fixture,
            "rebind_cmd_0003",
            bound_revision,
            2,
            &plan_1,
            &session_1,
        );
        let response = post_rebind(&app, stale_policy).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // 跨载体 target（logical → single）→ 422，durable 不变。
        let mut cross_carrier = rebind_body(
            &fixture,
            "rebind_cmd_0004",
            after_revision,
            2,
            &plan_1,
            &session_1,
        );
        cross_carrier["binding"]["target"] = serde_json::json!({
            "kind": "single_repository",
            "repository_id": "repo_physical_1",
        });
        let response = post_rebind(&app, cross_carrier).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // 逻辑 target 缺 logical_codebase_id 一级 → 422（serde fail-closed）。
        let mut incomplete = rebind_body(
            &fixture,
            "rebind_cmd_0005",
            after_revision,
            2,
            &plan_1,
            &session_1,
        );
        incomplete["binding"]["target"]
            .as_object_mut()
            .unwrap()
            .remove("logical_codebase_id");
        let response = post_rebind(&app, incomplete).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // plan/session 不属于 issue → 404。
        let unknown_target = rebind_body(
            &fixture,
            "rebind_cmd_0006",
            after_revision,
            2,
            "plan_9999",
            "session_9999",
        );
        let response = post_rebind(&app, unknown_target).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // 全部拒绝后 durable 仍只有一份当前版本。
        assert_eq!(response_json(get_enrollment(&app).await).await, after);
    }

    // ---- 批 2 P1：plan_approval 冻结与 AutoIfValid 绑定准入（REQ-WIGA-02 修订）----

    /// ①auto 授权冻结 plan_approval，并把 require_execution_plan_confirm 压
    /// false（不论弹窗复选框原值）；同 payload 幂等；旧 wire 缺字段按
    /// manual 冻结且手动复选框原样保留（③④）。
    #[tokio::test]
    async fn automation_enrollment_http_put_freezes_auto_plan_approval() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        // auto + 复选框 true → 冻结 auto 且 require_execution_plan_confirm=false。
        let mut body = enrollment_body_with_target(&fixture);
        body["command"]["options"]["plan_approval"] = serde_json::json!("auto");
        body["command"]["options"]["plan_options"]["require_execution_plan_confirm"] =
            serde_json::json!(true);
        let first = put_enrollment(&app, body.clone()).await;
        assert_eq!(first.status(), StatusCode::OK);
        let enrolled = response_json(first).await;
        assert_eq!(enrolled["options"]["plan_approval"], "auto");
        assert_eq!(
            enrolled["options"]["plan_options"]["require_execution_plan_confirm"],
            false
        );

        // 同 payload（复选框仍 true）重发 → 幂等：同 enrollment_id/revision。
        let second = put_enrollment(&app, body).await;
        assert_eq!(second.status(), StatusCode::OK);
        let replayed = response_json(second).await;
        assert_eq!(replayed["enrollment_id"], enrolled["enrollment_id"]);
        assert_eq!(replayed["policy_revision"], enrolled["policy_revision"]);
        assert_eq!(
            replayed["options"]["plan_options"]["require_execution_plan_confirm"],
            false
        );

        // 旧 wire 缺 plan_approval 字段 → 冻结 manual；复选框 true 原样保留。
        let manual_fixture = seed_fixture(1, true);
        let manual_app = manual_fixture.router();
        let mut manual = enrollment_body_with_target(&manual_fixture);
        manual["command"]["options"]["plan_options"]["require_execution_plan_confirm"] =
            serde_json::json!(true);
        let response = put_enrollment(&manual_app, manual).await;
        assert_eq!(response.status(), StatusCode::OK);
        let manual_enrolled = response_json(response).await;
        assert_eq!(manual_enrolled["options"]["plan_approval"], "manual");
        assert_eq!(
            manual_enrolled["options"]["plan_options"]["require_execution_plan_confirm"],
            true
        );
    }

    /// ②binding/rebind 只放行与当前 enabled enrollment 冻结策略一致的
    /// run policy：auto 冻结放行 AutoIfValid（自动链绑定准入）、拒
    /// Interactive（降级）；manual 冻结的 rebind 对 AutoIfValid 仍拒绝
    /// （AutoIfValid 不代表人工授权，错绑拒绝）。
    #[tokio::test]
    async fn automation_enrollment_binding_and_rebind_follow_frozen_plan_approval() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        let mut body = enrollment_body_with_target(&fixture);
        body["command"]["options"]["plan_approval"] = serde_json::json!("auto");
        body["command"]["options"]["plan_options"]["require_execution_plan_confirm"] =
            serde_json::json!(true);
        let enable = put_enrollment(&app, body).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let enrolled = response_json(enable).await;
        let revision = enrolled["policy_revision"].as_u64().unwrap();

        // 降级绑定：auto 冻结 + Interactive 会话 → 409。
        let (interactive_plan, interactive_session) =
            create_plan_and_session(&fixture, RunPolicy::Interactive);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": interactive_plan,
                "session_id": interactive_session,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // 冻结策略一致的 AutoIfValid → 显式绑定成功。
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::AutoIfValid);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bound = response_json(response).await;
        assert_eq!(bound["plan_id"], plan_id.as_str());
        assert_eq!(bound["session_id"], session_id.as_str());
        let bound_revision = bound["policy_revision"].as_u64().unwrap();
        let binding_version = bound["binding_history"]["current"]["binding_version"]
            .as_u64()
            .unwrap();

        // rebind 换代：新代 AutoIfValid → accepted。
        let (next_plan, next_session) = create_plan_and_session(&fixture, RunPolicy::AutoIfValid);
        let rebind = rebind_body(
            &fixture,
            "rebind_auto_0001",
            bound_revision,
            binding_version,
            &next_plan,
            &next_session,
        );
        let response = post_rebind(&app, rebind.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let accepted = response_json(response).await;
        assert_eq!(accepted["state"], "accepted");
        let after_revision = accepted["enrollment"]["policy_revision"].as_u64().unwrap();

        // 同 command 同 payload 重放 → replayed（幂等回归）。
        let response = post_rebind(&app, rebind).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await["state"], "replayed");

        // rebind 降级到 Interactive 新代 → 409，durable 不变。
        let (downgrade_plan, downgrade_session) =
            create_plan_and_session(&fixture, RunPolicy::Interactive);
        let downgrade = rebind_body(
            &fixture,
            "rebind_auto_0002",
            after_revision,
            binding_version + 1,
            &downgrade_plan,
            &downgrade_session,
        );
        let response = post_rebind(&app, downgrade).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response_json(get_enrollment(&app).await).await["policy_revision"],
            after_revision
        );

        // manual 冻结：rebind 对 AutoIfValid 新代仍拒绝（错绑）。
        let manual_fixture = seed_fixture(1, true);
        let manual_app = manual_fixture.router();
        let enable =
            put_enrollment(&manual_app, enrollment_body_with_target(&manual_fixture)).await;
        let manual_revision = response_json(enable).await["policy_revision"]
            .as_u64()
            .unwrap();
        let (manual_plan, manual_session) =
            create_plan_and_session(&manual_fixture, RunPolicy::Interactive);
        let response = post_binding(
            &manual_app,
            serde_json::json!({
                "expected_revision": manual_revision,
                "plan_id": manual_plan,
                "session_id": manual_session,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let manual_bound = response_json(response).await;
        let (wrong_plan, wrong_session) =
            create_plan_and_session(&manual_fixture, RunPolicy::AutoIfValid);
        let wrong = rebind_body(
            &manual_fixture,
            "rebind_manual_0001",
            manual_bound["policy_revision"].as_u64().unwrap(),
            1,
            &wrong_plan,
            &wrong_session,
        );
        let response = post_rebind(&manual_app, wrong).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

