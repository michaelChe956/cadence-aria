// 经 automation_enrollment.rs 的 tests mod `include!` 引入（large_file_guard 行数门禁）。
//
// owner 2026-10-26 真机「开启即被终止 / 终止永远 409」回归的不变量与根修用例：
// ① 连续两次 re-enable 后 enrollment 仍 enabled 且 terminated_at 为空；
// ② 取代删除不产生 `entity-delete-terminate-*` 账本条目（针对本 enrollment）；
// ③ 无新用户动作时 `policy_revision` 不自行增长（reconcile 补偿 tick 幂等）；
// ④ 终止命令的「刷新前置条件续做」不得被判异 payload 死锁（终止入口根修）。

use tower::ServiceExt;

/// 终止命令账本前缀（`terminate_automation_bound_to_entity` 派生）。
fn entity_delete_terminate_prefix(enrollment_id: &str) -> String {
    format!("entity-delete-terminate-{enrollment_id}")
}

fn ledger_command_ids(enrollment: &IssueAutomationEnrollment) -> Vec<String> {
    enrollment
        .command_ledger
        .iter()
        .map(|entry| entry.command_id.clone())
        .collect()
}

/// enable → 显式 plan/session → bind，返回（绑定后 enrollment, plan, session）。
async fn seed_bound_enrollment(
    fixture: &super::super::automation_enrollment_test_support::Fixture,
    app: &axum::Router,
) -> (IssueAutomationEnrollment, String, String) {
    let enabled = response_json(put_enrollment(app, enrollment_body(fixture, 1, 1)).await).await;
    let revision = enabled["policy_revision"].as_u64().unwrap();
    let (plan_id, session_id) = create_plan_and_session(fixture, RunPolicy::Interactive);
    let bound = post_binding(
        app,
        serde_json::json!({
            "expected_revision": revision,
            "plan_id": plan_id,
            "session_id": session_id,
        }),
    )
    .await;
    assert_eq!(bound.status(), StatusCode::OK);
    let _ = response_json(bound).await;
    let enrollment = IssueAutomationStore::new(fixture.paths.clone())
        .get(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .unwrap();
    (enrollment, plan_id, session_id)
}

/// 人工重新开启（terminated → Enable 取代重开），返回响应体。
async fn re_enable(
    fixture: &super::super::automation_enrollment_test_support::Fixture,
    app: &axum::Router,
    command_id: &str,
) -> serde_json::Value {
    let current = IssueAutomationStore::new(fixture.paths.clone())
        .get(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .unwrap();
    let mut body = enrollment_body(fixture, 1, 1);
    body["expected_revision"] = serde_json::json!(current.policy_revision);
    body["command"]["command_id"] = serde_json::json!(command_id);
    let response = put_enrollment(app, body).await;
    let status = response.status();
    let payload = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "re-enable body: {payload}");
    payload
}

fn current_enrollment(
    fixture: &super::super::automation_enrollment_test_support::Fixture,
) -> IssueAutomationEnrollment {
    IssueAutomationStore::new(fixture.paths.clone())
        .get(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .unwrap()
}

/// 不变量①：连续两次 re-enable 之后 enrollment 仍 enabled、`terminated_at`
/// 为空（不得出现「开启即被终止」的自我抵消）。
#[tokio::test]
async fn re_enable_twice_keeps_enrollment_enabled_without_termination() {
    let fixture = seed_fixture(1, true);
    let app = fixture.router();
    let (bound, plan_1, _session_1) = seed_bound_enrollment(&fixture, &app).await;
    let enrollment_id = bound.enrollment_id.clone();

    for cycle in 1..=2 {
        // 人工终止（每次都是新命令：与前端/删除路径一致，command 身份按轮次换新）。
        let terminate = post_terminate(
            &app,
            &serde_json::json!({
                "command_id": format!("cmd-cycle-{cycle}-terminate"),
                "expected_enrollment_id": enrollment_id,
                "expected_policy_revision": current_enrollment(&fixture).policy_revision,
                "confirmed": true,
            }),
        )
        .await;
        assert_eq!(terminate.status(), StatusCode::OK);
        assert!(current_enrollment(&fixture).terminated_at.is_some());

        let payload = re_enable(&fixture, &app, &format!("cmd-cycle-{cycle}-reenable")).await;
        assert_eq!(payload["enabled"].as_bool(), Some(true));
        assert!(payload["terminated_at"].is_null());
        let after = current_enrollment(&fixture);
        assert!(after.enabled, "cycle {cycle}: enrollment must stay enabled");
        assert!(
            after.terminated_at.is_none(),
            "cycle {cycle}: re-enable must clear terminated_at, got {:?}",
            after.terminated_at
        );
        assert!(after.plan_id.is_none(), "supersede unbinds the old plan");

        // 换代推进（reconcile 补偿）后再进入下一轮，覆盖「重开后又被重开」。
        let root = fixture._root.path();
        let state = WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()));
        let fresh = current_enrollment(&fixture);
        ensure_enrolled_plan(&state, &fresh).await.unwrap();
        assert!(current_enrollment(&fixture).enabled);
        assert!(current_enrollment(&fixture).terminated_at.is_none());
    }
    // 首代 plan 已随取代删除，不复活。
    assert!(
        fixture
            .lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, &plan_1)
            .is_err(),
        "superseded plan must not revive"
    );
}

/// 不变量②：取代删除不产生 `entity-delete-terminate-*` 账本条目（针对本
/// enrollment）——取代删除是代际作废，不是「实体被删」，不得终止正在被重新
/// 开启的 enrollment。
#[tokio::test]
async fn supersede_deletion_never_terminates_the_reenabled_enrollment() {
    let fixture = seed_fixture(1, true);
    let app = fixture.router();
    let (bound, plan_1, _session_1) = seed_bound_enrollment(&fixture, &app).await;
    let enrollment_id = bound.enrollment_id.clone();

    let terminate = post_terminate(
        &app,
        &serde_json::json!({
            "command_id": "cmd-supersede-terminate",
            "expected_enrollment_id": enrollment_id,
            "expected_policy_revision": bound.policy_revision,
            "confirmed": true,
        }),
    )
    .await;
    assert_eq!(terminate.status(), StatusCode::OK);

    re_enable(&fixture, &app, "cmd-supersede-reenable").await;
    let after = current_enrollment(&fixture);
    assert!(after.enabled && after.terminated_at.is_none());

    // 取代删除的墓碑带触发原因（区别于用户面删除），且本 enrollment 账本里
    // 没有任何删除驱动的终止条目。
    let scope = crate::product::entity_deletion_store::EntityScope::new(
        PROJECT_ID,
        ISSUE_ID,
        crate::product::entity_deletion_store::DeletionEntityKind::Plan,
        &plan_1,
    );
    let tombstone =
        crate::product::entity_deletion_store::EntityDeletionStore::new(fixture.paths.clone())
            .get(&scope)
            .unwrap()
            .expect("supersede tombstone");
    assert_eq!(
        tombstone.trigger_reason.as_deref(),
        Some("re_enable_prepare_intent_rotation")
    );
    let prefix = entity_delete_terminate_prefix(&enrollment_id);
    assert!(
        ledger_command_ids(&after)
            .iter()
            .all(|command_id| !command_id.starts_with(&prefix)),
        "取代删除不得留下删除驱动的终止条目：{:?}",
        ledger_command_ids(&after)
    );

    // 幂等重入：对已被取代删除的 plan 再发一次删除，不改本 enrollment（不终止、
    // 不新增账本条目）。
    let revision_before = current_enrollment(&fixture).policy_revision;
    let ledger_before = ledger_command_ids(&current_enrollment(&fixture));
    let reentry = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/work-item-plans/{plan_1}"
                ))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reentry.status(), StatusCode::OK);
    let settled = current_enrollment(&fixture);
    assert!(settled.enabled && settled.terminated_at.is_none());
    assert_eq!(settled.policy_revision, revision_before);
    assert_eq!(ledger_command_ids(&settled), ledger_before);
}

/// 不变量③：无新用户动作时 `policy_revision` 不自行增长——reconcile 的
/// `ensure_enrolled_plan` 补偿连续两 tick 幂等（绑定一代只 +1），terminated
/// 后 reconcile 判定 NoEnrollment 零写入。
#[tokio::test]
async fn re_enable_revision_stays_stable_across_reconcile_ticks() {
    let fixture = seed_fixture(1, true);
    let app = fixture.router();
    let (bound, _plan_1, _session_1) = seed_bound_enrollment(&fixture, &app).await;
    let terminate = post_terminate(
        &app,
        &serde_json::json!({
            "command_id": "cmd-stable-terminate",
            "expected_enrollment_id": bound.enrollment_id,
            "expected_policy_revision": bound.policy_revision,
            "confirmed": true,
        }),
    )
    .await;
    assert_eq!(terminate.status(), StatusCode::OK);
    re_enable(&fixture, &app, "cmd-stable-reenable").await;

    let root = fixture._root.path();
    let state = WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()));
    // tick 1：建立并绑定新代 plan/session（一次）。
    let fresh = current_enrollment(&fixture);
    ensure_enrolled_plan(&state, &fresh).await.unwrap();
    let after_bind = current_enrollment(&fixture);
    assert!(after_bind.enabled);
    assert!(after_bind.plan_id.is_some(), "补偿 tick 建立新代绑定");
    // tick 2/3：无用户动作，绑定与 revision 全部不变。
    for tick in 2..=3 {
        let fresh = current_enrollment(&fixture);
        ensure_enrolled_plan(&state, &fresh).await.unwrap();
        let after = current_enrollment(&fixture);
        assert_eq!(
            after.policy_revision, after_bind.policy_revision,
            "tick {tick} must not bump policy_revision without a user action"
        );
        assert_eq!(after.plan_id, after_bind.plan_id, "tick {tick} 不得换绑");
        assert_eq!(
            after
                .binding_history
                .as_ref()
                .map(|history| history.current.binding_version),
            after_bind
                .binding_history
                .as_ref()
                .map(|history| history.current.binding_version),
            "tick {tick} 不得换代"
        );
    }

    // 终止后 reconcile 补偿零写入（terminated 阻断全部自动路径）。
    let revision_bound = current_enrollment(&fixture).policy_revision;
    let terminated = post_terminate(
        &app,
        &serde_json::json!({
            "command_id": "cmd-stable-terminate-2",
            "expected_enrollment_id": bound.enrollment_id,
            "expected_policy_revision": revision_bound,
            "confirmed": true,
        }),
    )
    .await;
    assert_eq!(terminated.status(), StatusCode::OK);
    let terminated_snapshot = current_enrollment(&fixture);
    let terminated_revision = terminated_snapshot.policy_revision;
    let tick = ensure_enrolled_plan(&state, &terminated_snapshot).await;
    assert!(
        tick.is_err(),
        "terminated enrollment must not prepare plans"
    );
    assert_eq!(
        current_enrollment(&fixture).policy_revision,
        terminated_revision,
        "terminated reconcile tick must be write-free"
    );
}

/// 根修④（owner 真机死路）：前端终止命令身份按 enrollment 稳定派生
/// （`cmd-fe-terminate-<enrollment_id>`），revition 漂移后按服务端 409 details
/// 刷新身份重试——该重试只刷新乐观前置条件，仍是同一条终止命令，MUST 收敛
/// 而不是永久 409。已终止时重提回报首次 durable 结果（Replayed，零写入）；
/// 重新开启后再重提按新前置条件精确 CAS 落盘。
#[tokio::test]
async fn termination_command_retries_after_revision_drift_converge() {
    let fixture = seed_fixture(1, true);
    let app = fixture.router();
    let (bound, _plan_1, _session_1) = seed_bound_enrollment(&fixture, &app).await;
    let enrollment_id = bound.enrollment_id.clone();
    let command_id = format!("cmd-fe-terminate-{enrollment_id}");

    let first = post_terminate(
        &app,
        &serde_json::json!({
            "command_id": command_id,
            "expected_enrollment_id": enrollment_id,
            "expected_policy_revision": bound.policy_revision,
            "confirmed": true,
        }),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let terminated = current_enrollment(&fixture);
    let terminated_at = terminated.terminated_at.clone().expect("terminated");

    // 同命令、前置条件漂移（模拟前端自愈刷新后的重试）：回报首次 durable
    // 结果，revision 不再增长、账本不新增。
    let replay = post_terminate(
        &app,
        &serde_json::json!({
            "command_id": command_id,
            "expected_enrollment_id": enrollment_id,
            "expected_policy_revision": terminated.policy_revision + 7,
            "confirmed": true,
        }),
    )
    .await;
    let replay_status = replay.status();
    let replay_body = response_json(replay).await;
    assert_eq!(
        replay_status,
        StatusCode::OK,
        "refreshed-precondition retry must converge, got {replay_body}"
    );
    assert_eq!(replay_body["state"], "replayed");
    let replayed = current_enrollment(&fixture);
    assert_eq!(replayed.policy_revision, terminated.policy_revision);
    assert_eq!(
        replayed.terminated_at.as_deref(),
        Some(terminated_at.as_str())
    );
    assert_eq!(replayed.command_ledger, terminated.command_ledger);

    // 重新开启后再用同一命令（带刷新后的当前 revision）终止：按新前置条件
    // 精确 CAS 落盘（终止入口必须长期可用，不得被历史命令身份钉死）。
    re_enable(&fixture, &app, "cmd-fe-reenable-after-terminate").await;
    let enabled = current_enrollment(&fixture);
    assert!(enabled.enabled && enabled.terminated_at.is_none());
    let second = post_terminate(
        &app,
        &serde_json::json!({
            "command_id": command_id,
            "expected_enrollment_id": enrollment_id,
            "expected_policy_revision": enabled.policy_revision,
            "confirmed": true,
        }),
    )
    .await;
    let second_status = second.status();
    let second_body = response_json(second).await;
    assert_eq!(
        second_status,
        StatusCode::OK,
        "terminate after re-enable must apply, got {second_body}"
    );
    assert_eq!(second_body["state"], "accepted");
    let retried = current_enrollment(&fixture);
    assert!(retried.terminated_at.is_some());
    assert_eq!(retried.policy_revision, enabled.policy_revision + 1);
}

// ==== 悬空源重授权（删除 story/design 后 terminated 记录的重新开启）====
//
// 真机缺陷：级联删除 story/design 不覆盖 automation enrollment 的 source 引用，
// terminated 记录留下悬空 source；人工重新开启必须逐字段回传终止记录 →
// 「story spec … not found」422 死路。裁定语义：
//   ① 旧 source 引用的实体已删（悬空）→ 原授权失效，允许按当前已确认
//      source 重新授权，binding 换代留痕（旧代 source 进 previous）；
//   ② 旧 source 仍有效且相同 → 既有等 payload 重新开启（上方用例已覆盖）；
//   ③ 旧 source 仍有效但不同 → 维持逐字段相等 409（防误改授权范围）。

/// 播种一组新的已确认 story/design（供悬空重授权的「当前 source」）。
fn seed_confirmed_spec_pair(
    fixture: &super::super::automation_enrollment_test_support::Fixture,
    title: &str,
) -> (String, String) {
    use crate::product::lifecycle_store::{
        AppendSpecVersionInput, CreateDesignSpecInput, CreateStorySpecInput,
    };
    let story = fixture
        .lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: super::super::automation_enrollment_test_support::REPOSITORY_ID
                .to_string(),
            title: format!("{title} story"),
            aggregate_codebase: None,
        })
        .unwrap();
    fixture
        .lifecycle
        .append_version(AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: story.id.clone(),
            markdown: format!("# {title} story"),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .unwrap();
    fixture
        .lifecycle
        .update_spec_confirmation_status(
            PROJECT_ID,
            ISSUE_ID,
            &story.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .unwrap();
    let design = fixture
        .lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: format!("{title} design"),
            aggregate_codebase: None,
        })
        .unwrap();
    fixture
        .lifecycle
        .append_version(AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: design.id.clone(),
            markdown: format!("# {title} design"),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .unwrap();
    fixture
        .lifecycle
        .update_spec_confirmation_status(
            PROJECT_ID,
            ISSUE_ID,
            &design.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .unwrap();
    (story.id, design.id)
}

/// DELETE 已确认实体（分级确认 REQ-DEL-04：previously-confirmed 需 confirmed=true）。
async fn delete_confirmed_spec(
    app: &axum::Router,
    segment: &str,
    id: &str,
) -> (StatusCode, serde_json::Value) {
    use tower::ServiceExt;
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri(format!(
                    "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/{segment}/{id}?confirmed=true"
                ))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
    )
}

/// 情形①（悬空重授权）：级联删除原授权源后，人工重新开启按**当前已确认
/// source** 显式重新授权——200 取代生效、binding 换代留痕（旧代悬空 source
/// 只读进 previous）；新 source 的三条判定（存在/已确认/版本精确匹配）不放宽。
#[tokio::test]
async fn reenable_with_dangling_source_resigns_on_current_confirmed_source() {
    let fixture = seed_fixture(1, true);
    let app = fixture.router();
    let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
    assert_eq!(enable.status(), StatusCode::OK);
    let enabled = response_json(enable).await;
    let enrollment_id = enabled["enrollment_id"].as_str().unwrap().to_string();
    let terminate = post_terminate(
        &app,
        &serde_json::json!({
            "command_id": "cmd-dangling-terminate",
            "expected_enrollment_id": enrollment_id,
            "expected_policy_revision": enabled["policy_revision"],
            "confirmed": true,
        }),
    )
    .await;
    assert_eq!(terminate.status(), StatusCode::OK);
    let before = current_enrollment(&fixture);
    let binding_version_before = before
        .binding_history
        .as_ref()
        .map(|history| history.current.binding_version)
        .unwrap_or(0);

    // 级联删除原授权源（先删引用 story 的 design，再删 story——上游放行）。
    let (status, body) = delete_confirmed_spec(&app, "design-specs", &fixture.design_id).await;
    assert_eq!(status, StatusCode::OK, "delete design: {body}");
    let (status, body) = delete_confirmed_spec(&app, "story-specs", &fixture.story_id).await;
    assert_eq!(status, StatusCode::OK, "delete story: {body}");

    // 新一代已确认授权源。
    let (story2, design2) = seed_confirmed_spec_pair(&fixture, "second");

    // 负例：新 source 引用已删实体 → 422（三条判定不因悬空重授权放宽），
    // 零副作用。
    let mut bad = enrollment_body(&fixture, 1, 1);
    bad["expected_revision"] = serde_json::json!(before.policy_revision);
    bad["command"]["command_id"] = serde_json::json!("cmd-dangling-bad-source");
    bad["command"]["source"]["stories"][0]["id"] = serde_json::json!(fixture.story_id.clone());
    bad["command"]["source"]["designs"][0]["id"] = serde_json::json!(design2.clone());
    let response = put_enrollment(&app, bad).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // 悬空重授权：source＝当前已确认新对，options/target 与终止记录逐字段相等
    //（悬空路径不放宽 options——REQ-DRG-18；正常重开路径的 options 漂移按
    // 换代放行，见下方 reenable_with_new_provider_is_accepted_in_one_step）。
    let mut resign = enrollment_body(&fixture, 1, 1);
    resign["expected_revision"] = serde_json::json!(before.policy_revision);
    resign["command"]["command_id"] = serde_json::json!("cmd-dangling-resign");
    resign["command"]["source"] = serde_json::json!({
        "stories": [{"id": story2.clone(), "version": 1}],
        "designs": [{"id": design2.clone(), "version": 1}],
    });
    let response = put_enrollment(&app, resign).await;
    let status = response.status();
    let payload = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "dangling resign body: {payload}");
    assert_eq!(payload["enabled"], true);
    assert!(payload["terminated_at"].is_null());

    let after = current_enrollment(&fixture);
    assert!(after.enabled && after.terminated_at.is_none());
    assert_eq!(after.source.stories.len(), 1);
    assert_eq!(after.source.stories[0].id, story2);
    assert_eq!(after.source.designs[0].id, design2);
    // 留痕＝binding 换代：旧代（悬空 source）只读进 previous，版本 +1。
    let history = after.binding_history.as_ref().expect("binding history");
    assert_eq!(
        history.current.binding_version,
        binding_version_before + 1,
        "悬空重授权必须换代（generation +1）"
    );
    let previous = history
        .previous
        .last()
        .expect("old generation archived in previous");
    assert_eq!(previous.source, before.source, "旧代悬空 source 留痕可追溯");
    assert_eq!(previous.binding_version, binding_version_before);
    assert_eq!(history.current.source, after.source);
}

/// 情形③（旧 source 仍有效但不同）：story 版本演进（实体仍在）→ 非悬空，
/// 按当前版本提交的重新开启因 **source 漂移** 维持 409（options 漂移在该
/// 正常重开路径按换代放行——REQ-WIGA-SWAP-01），零副作用。
#[tokio::test]
async fn reenable_with_valid_but_divergent_source_stays_conflict() {
    let fixture = seed_fixture(1, true);
    let app = fixture.router();
    let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
    assert_eq!(enable.status(), StatusCode::OK);
    let enabled = response_json(enable).await;
    let terminate = post_terminate(
        &app,
        &serde_json::json!({
            "command_id": "cmd-divergent-terminate",
            "expected_enrollment_id": enabled["enrollment_id"],
            "expected_policy_revision": enabled["policy_revision"],
            "confirmed": true,
        }),
    )
    .await;
    assert_eq!(terminate.status(), StatusCode::OK);
    let before = current_enrollment(&fixture);

    // 旧 source 引用的实体仍在，但版本演进到 2（有效、与终止记录不同）。
    fixture
        .lifecycle
        .append_version(crate::product::lifecycle_store::AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: fixture.story_id.clone(),
            markdown: "# story v2".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .unwrap();

    // 按当前版本（story v2）提交重新开启：非悬空 → source 漂移维持 409
    //（options/target 与终止记录一致，不涉 options 放宽面）。
    let mut divergent = enrollment_body(&fixture, 2, 1);
    divergent["expected_revision"] = serde_json::json!(before.policy_revision);
    divergent["command"]["command_id"] = serde_json::json!("cmd-divergent-reenable");
    let response = put_enrollment(&app, divergent).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let payload = response_json(response).await;
    assert_eq!(payload["code"], "automation_enrollment_conflict");

    // 零副作用：terminated 事实与授权原样。
    let after = current_enrollment(&fixture);
    assert!(after.terminated_at.is_some());
    assert!(!after.enabled);
    assert_eq!(after.policy_revision, before.policy_revision);
    assert_eq!(after.source, before.source);
    assert_eq!(after.options, before.options);
}

/// 块1 片2（REQ-WIGA-SWAP-01）④：终止态**正常路径**上带不同 provider 的
/// Enable 一步被接受——HTTP 200＋enabled=true＋terminated_at 清空＋换代
/// 留痕（binding_version+1、旧代 fake provider 进 previous、旧 plan 作废
/// 删除）。不再要求「先同 payload 取代再 Disable→Enable」两步换 provider。
#[tokio::test]
async fn reenable_with_new_provider_is_accepted_in_one_step() {
    let fixture = seed_fixture(1, true);
    let app = fixture.router();
    let (bound, plan_1, _session_1) = seed_bound_enrollment(&fixture, &app).await;
    // 新 provider（claude_code）的正向能力证据：bootstrap＋Confirmed 行
    //（与 automation_target 测试族同构）——Fake 仅测试运行豁免，换 provider
    // 必须过 Enable 前的完整角色链同源预检。
    let paths = fixture.paths.clone();
    crate::product::logical_codebase::ProviderCapabilityStore::new(paths.clone())
        .ensure_bootstrap(PROJECT_ID)
        .expect("capability bootstrap");
    super::super::automation_enrollment_test_support::seed_confirmed_capability_records(
        &paths,
        PROJECT_ID,
        &[crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode],
    );

    let terminate = post_terminate(
        &app,
        &serde_json::json!({
            "command_id": "cmd-swap-provider-terminate",
            "expected_enrollment_id": bound.enrollment_id,
            "expected_policy_revision": current_enrollment(&fixture).policy_revision,
            "confirmed": true,
        }),
    )
    .await;
    assert_eq!(terminate.status(), StatusCode::OK);
    let terminated = current_enrollment(&fixture);
    assert!(terminated.terminated_at.is_some());
    let binding_version_before = terminated
        .binding_history
        .as_ref()
        .map(|history| history.current.binding_version)
        .unwrap_or(0);

    // 带 provider 漂移（options 漂移）的重新开启：一步 200。
    let mut swap = enrollment_body(&fixture, 1, 1);
    swap["expected_revision"] = serde_json::json!(terminated.policy_revision);
    swap["command"]["command_id"] = serde_json::json!("cmd-swap-provider-reenable");
    swap["command"]["options"]["author_provider"] = serde_json::json!("claude_code");
    swap["command"]["options"]["reviewer_provider"] = serde_json::json!("claude_code");
    let response = put_enrollment(&app, swap).await;
    let status = response.status();
    let payload = response_json(response).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "provider swap re-enable must be accepted in one step: {payload}"
    );
    assert_eq!(payload["enabled"], true);
    assert!(payload["terminated_at"].is_null());

    let after = current_enrollment(&fixture);
    assert!(after.enabled && after.terminated_at.is_none());
    assert_eq!(
        after.options.author_provider,
        crate::product::models::ProviderName::ClaudeCode
    );
    assert_eq!(
        after.options.reviewer_provider,
        crate::product::models::ProviderName::ClaudeCode
    );
    // 换代留痕：binding_version+1，新代 provider 生效，旧代（fake）进 previous。
    let history = after.binding_history.as_ref().expect("binding history");
    assert_eq!(history.current.binding_version, binding_version_before + 1);
    assert_eq!(
        history.current.author_provider,
        crate::product::models::ProviderName::ClaudeCode
    );
    let previous = history
        .previous
        .last()
        .expect("old generation archived in previous");
    assert_eq!(previous.binding_version, binding_version_before);
    assert_eq!(
        previous.author_provider,
        crate::product::models::ProviderName::Fake
    );
    // 旧 plan 已随取代作废删除，不复活。
    assert!(
        fixture
            .lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, &plan_1)
            .is_err(),
        "superseded plan must not survive the provider swap"
    );
}
