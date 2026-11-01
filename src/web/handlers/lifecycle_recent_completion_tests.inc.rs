// P3 WIGA Task 1（tasks.md §4.1 / REQ-WIGA-07）：issue lifecycle 有界近期
// 完成目录的 Axum 请求测试。复用 EnrolledGateFixture 的真实 campaign 与
// build_web_router（无网络 smoke 同源）。经 lifecycle.rs 的 tests mod include!。
// axum/Body/Request/StatusCode/ServiceExt/CreateProductIssueInput 复用同 mod
// 内 lifecycle_tests.inc.rs 已有导入。

use crate::product::coding_attempt_store::{CodingAttemptStore, CreateGroupCodingAttemptInput};
use crate::product::coding_models::{
    CodingAdmissionKind, CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
    CodingStartClaim, CodingStartOrigin, CodingStartPhase, CodingStartRunPolicy,
    CodingTimelineNode, CodingTimelineNodeStatus, GroupFinalReadinessSnapshot,
    GroupFinalReadinessStatus, GroupFinalReadinessUnit, ReviewVerdict,
};
use crate::product::issue_store::IssueStore;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

/// URL query 中 `+` 会被解码为空格：RFC3339 时区偏移必须转义后拼接。
fn encode_query(value: &str) -> String {
    value.replace('+', "%2B")
}

/// 走真实 build_web_router 的 lifecycle GET；`query` 以 `&` 前缀追加。
async fn fetch_lifecycle(
    app: &axum::Router,
    issue_id: &str,
    query: &str,
) -> (StatusCode, serde_json::Value) {
    let uri = format!("/api/issues/{issue_id}/lifecycle?project_id={PROJECT_ID}{query}");
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

fn readiness_snapshot(attempt_id: &str, created_at: &str) -> GroupFinalReadinessSnapshot {
    GroupFinalReadinessSnapshot {
        attempt_id: attempt_id.to_string(),
        status: GroupFinalReadinessStatus::Complete,
        units: vec![GroupFinalReadinessUnit {
            unit_id: "coding_unit_0001".to_string(),
            logical_work_item_id: "work_item_0001".to_string(),
            unit_run_id: Some("coding_unit_0001_run".to_string()),
            start_commit: Some("start_0001".to_string()),
            completion_commit: Some("commit_0001".to_string()),
            commit_shas: vec!["sha_0001".to_string()],
            diff_ref: "diff_0001".to_string(),
            code_review_report_id: Some("code_review_0001".to_string()),
            review_verdict: Some(ReviewVerdict::Approve),
            review_summary: Some("审核通过".to_string()),
            review_findings: Some(Vec::new()),
            handoff_revision_id: Some("handoff_revision_0001".to_string()),
            plan_revision_id: Some("plan_revision_0001".to_string()),
            ..Default::default()
        }],
        diagnostics: Vec::new(),
        created_at: created_at.to_string(),
    }
}

/// 在独立 issue 落一条 durable 已认领 group attempt 的 FinalConfirm 事实
///（复用 coding_final_confirm_info 测试的直写口径，只测目录投影判据）。
fn seed_claimed_final_confirm_fact(
    store: &CodingAttemptStore,
    issue_id: &str,
    attempt_key: &str,
    origin: CodingStartOrigin,
    node_started_at: &str,
) {
    let created = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: issue_id.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: format!("aria/issues/{issue_id}"),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: crate::product::models::ProviderName::Fake,
                reviewer: Some(crate::product::models::ProviderName::Fake),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 1,
            start_run_policy: CodingStartRunPolicy::Manual,
        })
        .expect("create group attempt");
    let mut attempt = created.clone();
    attempt.id = attempt_key.to_string();
    attempt.scope = CodingAttemptScope::WorkItemGroup;
    attempt.admission_kind = CodingAdmissionKind::ScAdvance;
    attempt.work_item_group_id = Some("work_item_plan_0001".to_string());
    attempt.status = CodingAttemptStatus::WaitingForHuman;
    attempt.stage = CodingExecutionStage::FinalConfirm;
    attempt.start_claim = Some(CodingStartClaim {
        command_id: format!("claim-{attempt_key}"),
        origin,
        phase: CodingStartPhase::ProviderMayHaveStarted,
        claimed_at: node_started_at.to_string(),
    });
    store
        .delete_attempt(&created.project_id, &created.issue_id, &created.id)
        .expect("drop generated id record");
    store
        .write_coding_attempt_for_test(&attempt)
        .expect("seed attempt record");
    store
        .write_group_final_readiness_snapshot(
            &attempt,
            &readiness_snapshot(attempt_key, node_started_at),
        )
        .expect("seed readiness snapshot");
    store
        .save_timeline_node(
            &attempt,
            CodingTimelineNode {
                id: format!("{attempt_key}-node"),
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::FinalConfirm,
                title: crate::web::coding_final_confirm_info::CODING_FINAL_CONFIRM_WAITING_TITLE
                    .to_string(),
                status: CodingTimelineNodeStatus::Pending,
                agent_role: None,
                summary: None,
                started_at: node_started_at.to_string(),
                completed_at: None,
                artifact_refs: Vec::new(),
            },
        )
        .expect("seed final confirm node");
}

fn create_plain_issue(paths: &crate::product::app_paths::ProductAppPaths, title: &str) -> String {
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some(
                crate::web::handlers::automation_enrollment_test_support::REPOSITORY_ID.to_string(),
            ),
            logical_codebase_id: None,
            title: title.to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("create plain issue")
        .id
}

/// 主证据（§4.1）：campaign 跑到 FinalConfirm 等待后——
/// 1) 无 query：additive 字段为空、旧两 info 数组原样；
/// 2) `recent_since&recent_limit=1` 只回最新事实（coding 晚于 plan commit）；
/// 3) 重复 GET 幂等；人工确认后同 key 改「已最终确认」不换时间；
/// 4) 孤立 limit/非法 RFC3339/limit 0/33 均 422；错误 project/issue 404；
/// 5) 禁用 enrollment 后 plan 事实不重现；
/// 6) 跨 issue 不串授权；旧于 24h 窗口不返回；Manual attempt 不冒充。
#[tokio::test]
async fn issue_lifecycle_recent_completion_catalog_bounded_scoped_and_idempotent() {
    let fixture = Box::new(
        crate::web::wiga_gate_fixture::complete_enrolled_group_waiting_for_final_confirm().await,
    );
    let app = crate::web::app::build_web_router(fixture.state.clone());

    // 1) 无 query：旧字段仍在，新字段为空数组。
    let (status, plain) = fetch_lifecycle(&app, ISSUE_ID, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(plain["recent_completion_info"], serde_json::json!([]));
    assert_eq!(plain["plan_confirmed_info"].as_array().unwrap().len(), 1);
    assert_eq!(
        plain["coding_final_confirm_info"].as_array().unwrap().len(),
        1
    );
    let attempt_id = fixture.attempt().id.clone();
    let plan_key = plain["plan_confirmed_info"][0]["key"]
        .as_str()
        .unwrap()
        .to_string();
    let plan_occurred_at = plain["plan_confirmed_info"][0]["occurred_at"]
        .as_str()
        .unwrap()
        .to_string();
    let original_final_confirm_started_at = plain["coding_final_confirm_info"][0]["occurred_at"]
        .as_str()
        .unwrap()
        .to_string();

    // 2) 边界请求矩阵：孤立 limit、非法 RFC3339、limit 0/33 均 422。
    let since = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    for bad in [
        "&recent_limit=1".to_string(),
        "&recent_since=not-a-time".to_string(),
        format!("&recent_since={}&recent_limit=0", encode_query(&since)),
        format!("&recent_since={}&recent_limit=33", encode_query(&since)),
    ] {
        let (status, payload) = fetch_lifecycle(&app, ISSUE_ID, &bad).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}: {payload}");
    }

    // 3) 错误 project/issue：404，不串授权。
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/issues/{ISSUE_ID}/lifecycle?project_id=project_9999"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/issues/issue_9999/lifecycle?project_id={PROJECT_ID}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // 4) limit=1 只回最新一条（coding FinalConfirm 事实晚于 plan commit）。
    let (status, bounded) = fetch_lifecycle(
        &app,
        ISSUE_ID,
        &format!("&recent_since={}&recent_limit=1", encode_query(&since)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let bounded = bounded["recent_completion_info"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(bounded.len(), 1, "{bounded:?}");
    assert_eq!(bounded[0]["kind"], "coding_final_confirm");
    assert_eq!(bounded[0]["attempt_id"], attempt_id.as_str());
    assert_eq!(
        bounded[0]["occurred_at"],
        original_final_confirm_started_at.as_str()
    );
    assert_eq!(bounded[0]["final_confirmed"], false);
    assert_eq!(
        bounded[0]["title"],
        crate::web::coding_final_confirm_info::CODING_FINAL_CONFIRM_WAITING_TITLE
    );
    assert_eq!(bounded[0]["session_id"], serde_json::Value::Null);

    // 5) limit=2：coding 先于 plan（occurred_at DESC）；plan 条目字段对偶；
    //    重复 GET 幂等不漂移；48h since 被服务端钳制到 24h 窗内不丢事实。
    let full_query = format!("&recent_since={}&recent_limit=2", encode_query(&since));
    let (status, first_full) = fetch_lifecycle(&app, ISSUE_ID, &full_query).await;
    assert_eq!(status, StatusCode::OK);
    let (status, second_full) = fetch_lifecycle(&app, ISSUE_ID, &full_query).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        first_full["recent_completion_info"], second_full["recent_completion_info"],
        "重复 GET 必须得到相同目录"
    );
    let catalog = first_full["recent_completion_info"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(catalog.len(), 2, "{catalog:?}");
    assert_eq!(catalog[0]["kind"], "coding_final_confirm");
    assert_eq!(catalog[1]["kind"], "plan_confirmed");
    assert_eq!(catalog[1]["key"], plan_key.as_str());
    assert_eq!(catalog[1]["session_id"], fixture.session_id.as_str());
    assert_eq!(catalog[1]["attempt_id"], serde_json::Value::Null);
    assert_eq!(catalog[1]["final_confirmed"], serde_json::Value::Null);
    assert_eq!(catalog[1]["occurred_at"], plan_occurred_at.as_str());
    let since_48h = (chrono::Utc::now() - chrono::Duration::hours(48)).to_rfc3339();
    let (status, clamped) = fetch_lifecycle(
        &app,
        ISSUE_ID,
        &format!("&recent_since={}&recent_limit=2", encode_query(&since_48h)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        clamped["recent_completion_info"], first_full["recent_completion_info"],
        "服务端窗口钳制（max(query, now-24h)）不得丢当前 24h 内事实"
    );

    // 6) 人工最终确认后：同 key/同 occurred_at 只改标题与 final_confirmed。
    fixture.confirm_final_by_human().await;
    let (status, confirmed) = fetch_lifecycle(
        &app,
        ISSUE_ID,
        &format!("&recent_since={}&recent_limit=2", encode_query(&since)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let confirmed = confirmed["recent_completion_info"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(confirmed.len(), 2);
    assert_eq!(confirmed[0]["key"], catalog[0]["key"]);
    assert_eq!(
        confirmed[0]["occurred_at"],
        original_final_confirm_started_at.as_str()
    );
    assert_eq!(confirmed[0]["final_confirmed"], true);
    assert_eq!(
        confirmed[0]["title"],
        crate::web::coding_final_confirm_info::CODING_FINAL_CONFIRM_CONFIRMED_TITLE
    );
    assert_eq!(confirmed[1], catalog[1], "plan 条目不受 coding 确认影响");

    // 7) 禁用 enrollment：plan 事实不重现（目录只随当前 durable 投影）。
    let revision = fixture.enrollment().policy_revision;
    let disable = serde_json::json!({
        "expected_revision": revision,
        "command": {"type": "disable"}
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!(
                    "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment"
                ))
                .header("content-type", "application/json")
                .body(Body::from(disable.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let (status, disabled) = fetch_lifecycle(
        &app,
        ISSUE_ID,
        &format!("&recent_since={}&recent_limit=2", encode_query(&since)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let disabled = disabled["recent_completion_info"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(disabled.len(), 1, "禁用后只剩 coding 事实: {disabled:?}");
    assert_eq!(disabled[0]["kind"], "coding_final_confirm");

    // 8) 跨 issue 授权隔离 + 旧于窗口不返回 + Manual attempt 不冒充。
    let paths = fixture.inner.paths.clone();
    let old_issue = create_plain_issue(&paths, "近期目录-旧事实隔离");
    let manual_issue = create_plain_issue(&paths, "近期目录-手动不冒充");
    let store = CodingAttemptStore::new(paths.clone());
    seed_claimed_final_confirm_fact(
        &store,
        &old_issue,
        "attempt-recent-old",
        CodingStartOrigin::Enrolled {
            enrollment_id: "enrollment_recent_old".to_string(),
            policy_revision: 1,
            binding_version: None,
            target: None,
        },
        "2020-01-01T00:00:00+00:00",
    );
    seed_claimed_final_confirm_fact(
        &store,
        &manual_issue,
        "attempt-recent-manual",
        CodingStartOrigin::Manual,
        &(chrono::Utc::now() - chrono::Duration::minutes(30)).to_rfc3339(),
    );
    let window_since = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
    let (status, old_payload) = fetch_lifecycle(
        &app,
        &old_issue,
        &format!(
            "&recent_since={}&recent_limit=32",
            encode_query(&window_since)
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        old_payload["coding_final_confirm_info"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "源投影仍含旧事实（供旧客户端）：{}",
        old_payload["coding_final_confirm_info"]
    );
    assert_eq!(
        old_payload["recent_completion_info"],
        serde_json::json!([]),
        "旧于 24h 窗口的完成事实不得进入近期目录"
    );
    let (status, manual_payload) = fetch_lifecycle(
        &app,
        &manual_issue,
        &format!(
            "&recent_since={}&recent_limit=32",
            encode_query(&window_since)
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        manual_payload["coding_final_confirm_info"],
        serde_json::json!([])
    );
    assert_eq!(
        manual_payload["recent_completion_info"],
        serde_json::json!([])
    );
    // 主 issue 不被邻 issue 事实污染。
    let (status, main_payload) = fetch_lifecycle(
        &app,
        ISSUE_ID,
        &format!(
            "&recent_since={}&recent_limit=32",
            encode_query(&window_since)
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let main_catalog = main_payload["recent_completion_info"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(main_catalog.len(), 1);
    assert_eq!(main_catalog[0]["attempt_id"], attempt_id.as_str());
}
