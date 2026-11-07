use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{
    CodingAttemptStore, register_plan_amendment_delivery_mark_failpoint,
};
use crate::product::coding_models::{CodingAttemptStatus, CodingPlanAmendmentDeliveryStatus};
use crate::web::app::build_web_router;
use crate::web::runtime::WebRuntime;
use crate::web::state::{CodingAttemptRunKey, WebAppState};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

type TestWsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// 取基座 plan 级 group attempt。
///
/// 用 `list_attempts_for_work_item_group`（`group.rs:106`，public）而**不是**
/// `get_attempt_for_work_item_group(..., target)`：后者按
/// `attempt_target_bucket(attempt) == target` 过滤，而
/// `attempt_target_bucket`（`group.rs:132-140`）在**快照在场时取
/// `logical_repository_id`**——本夹具给 attempt 补上与权威一致的冻结快照后
/// 桶从 `None` 变 `Some(logical_repository_id)`，写死 `None` 的查找口径
/// 就会落空（第二层 `unwrap()` panic）。`list_attempts_for_work_item_group`
/// 只按 `work_item_group_id` 过滤、不看桶 ⇒ 与「该 attempt 有没有快照」解耦，
/// 两种形态都命中同一条基座 attempt（基座只创建这一条 group attempt）。
fn plan_repair_attempt(
    store: &CodingAttemptStore,
) -> crate::product::coding_models::CodingExecutionAttempt {
    store
        .list_attempts_for_work_item_group("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .expect("plan repair attempt list")
        .into_iter()
        .next()
        .expect("plan repair attempt must exist")
}

async fn receive_json(ws: &mut TestWsStream, phase: &str) -> Value {
    loop {
        match timeout(Duration::from_secs(3), ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => return serde_json::from_str(&text).unwrap(),
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(error))) => panic!("{phase}: websocket receive failed: {error}"),
            Ok(None) => panic!("{phase}: websocket closed before expected message"),
            Err(_) => panic!("{phase}: websocket timed out before expected message"),
        }
    }
}

async fn receive_type(ws: &mut TestWsStream, expected_type: &str) -> Value {
    loop {
        let message = receive_json(ws, expected_type).await;
        if message["type"] == expected_type {
            return message;
        }
    }
}

/// REQ-ROUTE-01 两态化：plan_repair 基座夹具（`test_controls/plan_repair/seed.rs`）
/// 已落 project/repository_0001/issue_plan_0001 **与完整 LC 权威**（manifest +
/// member + 主 checkout + selection + policy + repos.json 逻辑归属回填）。
/// child WS attach 的 workspace context 另缺三段：
/// ① 别名位 `record.json`（authority resolver 的 LC 存在性凭证；`ProjectStore::get`
/// 内置的幂等 `migrate_legacy` 会把项目级 legacy 布局落成别名 LC record）；
/// ② active(→Degraded) 聚合索引（planning context 链路的 `active_required` 前置）；
/// ③ **attempt 的冻结 `target_snapshot`** —— 基座 attempt 以 `target_snapshot: None`
/// 落盘；路由既已是 Logical，`classify_worktree_lease_for_attempt` 便要求快照
/// （无快照 ⇒ evidence `target_snapshot_missing_for_logical` ⇒ `LeaseUnknown`
/// fail-closed ⇒ amendment 投递面起不来）。
/// ⇒ 这里只补这三段，**不重写权威**（重写会与基座 manifest 的
/// `logical_codebase_id` 冲突 ⇒ `IdentityMismatch`）。
///
/// ⚠️ ③ 的**副作用**：快照落盘后该 attempt 的 target 桶从 `None` 变为
/// `Some(logical_repository_id)`（`attempt_target_bucket`，`group.rs:132-140`）
/// ⇒ 此后取这条 attempt 必须用**不看桶**的 `list_attempts_for_work_item_group`
/// （见 [`plan_repair_attempt`]），写死 `None` 桶的
/// `get_attempt_for_work_item_group` 会落空。
fn seed_lc_authority_for_plan_repair(root: &std::path::Path) {
    let app_paths = ProductAppPaths::new(root.join(".aria"));
    // 幂等触发 migrate_legacy：项目级 legacy 布局 → 别名 LC record.json。
    crate::product::project_store::ProjectStore::new(app_paths.clone())
        .get("project_0001")
        .expect("plan repair project record");
    super::lc_fixture_support::ensure_planning_index_and_policy(&app_paths, "project_0001");
    // ③ 与权威一致的冻结快照：权威的 checkout / manifest / policy 事实逐字取自
    // store（`snapshot_for`），故 `validate_snapshot_fields` 必然通过。
    let store = crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone());
    let attempt = plan_repair_attempt(&store);
    let logical = crate::product::logical_codebase::LogicalCodebaseStore::new(app_paths.clone());
    let manifest = logical
        .load_manifest("project_0001")
        .expect("plan repair manifest")
        .expect("plan repair manifest must exist");
    let member_id = *manifest
        .member_ids
        .first()
        .expect("plan repair manifest must reference a member");
    let checkout_id = logical
        .list_checkouts("project_0001")
        .expect("plan repair checkouts")
        .into_iter()
        .find(|checkout| checkout.logical_repository_id == member_id)
        .map(|checkout| checkout.checkout_id)
        .expect("plan repair member must have a recorded checkout");
    let snapshot = crate::product::logical_codebase::logical_test_fixture::snapshot_for(
        &app_paths,
        "project_0001",
        member_id,
        checkout_id,
    );
    let mut attempt = attempt;
    attempt.target_snapshot = Some(snapshot);
    store
        .write_coding_attempt_for_test(&attempt)
        .expect("plan repair attempt snapshot");
    // 带快照的 attempt 只按**仓维**分类租约（`run_exclusion.rs`
    // `classify_worktree_lease_for_attempt`）：仓维记录缺失 ⇒ evidence
    // `worktree record not found for attempt route` ⇒ 虽不拦截（无租约事实），
    // 但 advance / amendment 推进面要「仓维锁已绑定到本 attempt」这一事实 ⇒
    // 这里按 advance 预绑定语义建仓维记录并把锁绑到该 attempt。
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(app_paths.clone());
    let worktree_path = attempt
        .worktree_path
        .clone()
        .expect("plan repair attempt worktree path");
    crate::product::logical_codebase::logical_test_fixture::upsert_repo_worktree(
        &lifecycle,
        "project_0001",
        "issue_plan_0001",
        member_id,
        &worktree_path,
    );
    crate::product::logical_codebase::logical_test_fixture::bind_repo_lock_to_attempt(
        &lifecycle,
        "project_0001",
        "issue_plan_0001",
        member_id,
        &attempt
            .current_work_item_id
            .clone()
            .expect("plan repair attempt current work item"),
        &attempt.id,
    );
}

#[tokio::test]
async fn repeated_confirmation_recovers_delivery_mark_failure_without_duplicate_application() {
    let root = tempfile::tempdir().unwrap();
    let runtime = crate::web::test_controls::PlanRepairFixtureRuntime::seed(
        root.path(),
        crate::web::test_controls::PlanRepairFixtureControl::default(),
    )
    .await
    .unwrap();
    let identity = runtime.drive_until_awaiting_confirmation().await.unwrap();
    seed_lc_authority_for_plan_repair(root.path());
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = plan_repair_attempt(&store);
    crate::web::coding_ws_handler::build_coding_session_state(&store, attempt.clone())
        .expect("plan repair fixture must build the initial coding websocket state");
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let failpoint =
        register_plan_amendment_delivery_mark_failpoint(&store, &attempt, &identity.amendment_id);
    let coding_url = format!(
        "ws://{addr}/ws/projects/{}/issues/{}/coding-attempts/{}",
        attempt.project_id, attempt.issue_id, attempt.id
    );
    let (mut coding_ws, _) = connect_async(coding_url).await.unwrap();
    assert_eq!(
        receive_json(&mut coding_ws, "initial coding state").await["type"],
        "coding_session_state"
    );
    let child_url = format!("ws://{addr}/api/ws/workspace/{}", identity.child_session_id);
    let (mut child_ws, _) = connect_async(child_url).await.unwrap();
    let initial_child = receive_json(&mut child_ws, "initial plan repair child state").await;
    assert_eq!(initial_child["type"], "session_state");
    assert_eq!(
        initial_child["stage"], "human_confirm",
        "plan amendment child must accept confirmation: {initial_child}"
    );
    let confirmation = || {
        Message::Text(
            json!({
                "type": "confirm_plan_amendment",
                "amendment_id": identity.amendment_id,
            })
            .to_string()
            .into(),
        )
    };

    child_ws.send(confirmation()).await.unwrap();
    let _first_event = receive_type(&mut coding_ws, "plan_amendment_updated").await;
    // REQ-WIGA-06：mark failpoint 不再令业务失败——attempt 直达 Running，
    // marker 停 Pending（Delivered 落盘被 failpoint 拦截），runner 唯一。
    timeout(Duration::from_secs(3), async {
        loop {
            let current = store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap();
            if current.status == CodingAttemptStatus::Running
                && state.coding_runs.runner_count(&attempt_key) == 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        store
            .get_plan_amendment_delivery(&attempt, &identity.amendment_id)
            .unwrap()
            .status,
        CodingPlanAmendmentDeliveryStatus::Pending
    );
    drop(failpoint);

    // 重复确认：runner 已在推进（early-return 触发补投递）；首轮被拦的
    // mark 与补投递竞争（注册互斥 + mark 幂等），最终收敛真实 Delivered。
    child_ws.send(confirmation()).await.unwrap();
    timeout(Duration::from_secs(3), async {
        loop {
            let current = store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap();
            let delivery = store
                .get_plan_amendment_delivery(&current, &identity.amendment_id)
                .unwrap();
            if current.status == CodingAttemptStatus::Running
                && delivery.status == CodingPlanAmendmentDeliveryStatus::Delivered
                && state.coding_runs.runner_count(&attempt_key) == 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        store
            .list_amendment_application_journals(&attempt)
            .unwrap()
            .len(),
        1
    );

    coding_ws.close(None).await.ok();
    child_ws.close(None).await.ok();
    server.abort();
}

#[tokio::test]
async fn zero_socket_plan_amendment_activation_resumes_attempt_with_unsent_delivery() {
    let root = tempfile::tempdir().unwrap();
    let runtime = crate::web::test_controls::PlanRepairFixtureRuntime::seed(
        root.path(),
        crate::web::test_controls::PlanRepairFixtureControl::default(),
    )
    .await
    .unwrap();
    let identity = runtime.drive_until_awaiting_confirmation().await.unwrap();
    seed_lc_authority_for_plan_repair(root.path());
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = plan_repair_attempt(&store);
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);

    // 全程不连接 coding WS：只从 workspace 子会话确认。
    let child_url = format!("ws://{addr}/api/ws/workspace/{}", identity.child_session_id);
    let (mut child_ws, _) = connect_async(child_url).await.unwrap();
    let initial_child = receive_json(&mut child_ws, "initial plan repair child state").await;
    assert_eq!(initial_child["type"], "session_state");
    child_ws
        .send(Message::Text(
            json!({
                "type": "confirm_plan_amendment",
                "amendment_id": identity.amendment_id,
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let resumed = timeout(Duration::from_secs(3), async {
        loop {
            let current = store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap();
            if current.status == CodingAttemptStatus::Running
                && state.coding_runs.runner_count(&attempt_key) == 1
            {
                return current;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("zero-socket activation must resume the attempt (REQ-WIGA-06)");

    let delivery = timeout(Duration::from_secs(3), async {
        loop {
            let delivery = store
                .get_plan_amendment_delivery(&resumed, &identity.amendment_id)
                .unwrap();
            if delivery.status == CodingPlanAmendmentDeliveryStatus::Unsent {
                return delivery;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("no live socket must leave a durable unsent fact, never Delivered");
    assert_eq!(delivery.delivered_at, None);

    // 重连补投递（REQ-WIGA-06）：迟到的观察者收到同一 event，真实写 ack 后才 Delivered。
    let coding_url = format!(
        "ws://{addr}/ws/projects/{}/issues/{}/coding-attempts/{}",
        attempt.project_id, attempt.issue_id, resumed.id
    );
    let (mut coding_ws, _) = connect_async(coding_url).await.unwrap();
    assert_eq!(
        receive_json(&mut coding_ws, "attach snapshot").await["type"],
        "coding_session_state"
    );
    let amendment_event = receive_type(&mut coding_ws, "plan_amendment_updated").await;
    assert_eq!(
        amendment_event["event_id"],
        serde_json::json!(delivery.event_id)
    );
    let delivered = timeout(Duration::from_secs(3), async {
        loop {
            let current = store
                .get_plan_amendment_delivery(&resumed, &identity.amendment_id)
                .unwrap();
            if current.status == CodingPlanAmendmentDeliveryStatus::Delivered {
                return current;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("real socket write acknowledgement must mark Delivered");
    assert!(delivered.delivered_at.is_some());
    coding_ws.close(None).await.ok();

    server.abort();
}
