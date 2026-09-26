use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_attempt_store::register_plan_amendment_delivery_mark_failpoint;
use crate::web::coding_ws_handler::delivery_ack::register_plan_amendment_socket_write;

use super::*;

/// 轮询 durable delivery marker 直到达到期望状态（观察任务是 detached 的，
/// marker 落盘与业务返回之间存在异步窗口）。
async fn poll_delivery_status(
    store: &CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    amendment_id: &str,
    expected: crate::product::coding_models::CodingPlanAmendmentDeliveryStatus,
) -> crate::product::coding_models::CodingPlanAmendmentDelivery {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Ok(delivery) = store.get_plan_amendment_delivery(attempt, amendment_id)
                && delivery.status == expected
            {
                return delivery;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("delivery status did not reach {expected:?}"))
}

/// REQ-WIGA-06：投递等待 ack 期间（marker=Pending）业务已 resume Running。
#[tokio::test]
async fn coding_amendment_delivery_pending_marker_does_not_block_business_resume() {
    let fixture = amendment_fixture().await;
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let resumed = engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect("pending delivery must not block business resume");
    assert_eq!(resumed.status, CodingAttemptStatus::Running);
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
        .await
        .expect("observation event must still be enqueued")
        .unwrap();
    assert!(matches!(
        event,
        CodingWsOutMessage::PlanAmendmentUpdated { .. }
    ));
    poll_delivery_status(
        &fixture.store,
        &resumed,
        &fixture.manifest.id,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Pending,
    )
    .await;
}

/// REQ-WIGA-06：零订阅者（channel 关闭）时业务照常 resume，观察层记真实
/// 未送达事实 Unsent（delivered_at=None），绝不伪造 Delivered。
#[tokio::test]
async fn coding_amendment_delivery_no_subscriber_marks_unsent_and_resumes() {
    let fixture = amendment_fixture().await;
    let (event_tx, event_rx) = mpsc::channel(1);
    drop(event_rx);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let resumed = engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect("receiver drop must not fail business");
    assert_eq!(resumed.status, CodingAttemptStatus::Running);
    let unsent = poll_delivery_status(
        &fixture.store,
        &resumed,
        &fixture.manifest.id,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Unsent,
    )
    .await;
    assert_eq!(unsent.delivered_at, None);
    let marker_path = fixture
        .store
        .attempt_dir(&resumed.project_id, &resumed.issue_id, &resumed.id)
        .join("amendment-event-deliveries")
        .join(format!("{}.json", fixture.manifest.id));
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(marker_path).unwrap()).unwrap();
    assert_eq!(marker["amendment_id"], fixture.manifest.id);
    assert_eq!(marker["status"], "unsent");
    assert_eq!(marker["delivered_at"], serde_json::json!(null));
}

/// REQ-WIGA-06：socket 写失败不产生业务错误，观察层收口 Unsent。
#[tokio::test]
async fn coding_amendment_delivery_socket_write_failure_marks_unsent_and_resumes() {
    let fixture = amendment_fixture().await;
    let attempt = fixture.attempt.clone();
    let manifest = fixture.manifest.clone();
    let store = fixture.store.clone();
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let apply = tokio::spawn(async move {
        CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx)
            .apply_plan_amendment(&attempt, &manifest)
            .await
    });

    let event = tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    crate::web::coding_ws_handler::delivery_ack::fail_plan_amendment_socket_write(&event);
    let resumed = apply
        .await
        .unwrap()
        .expect("socket write failure must not fail business");
    assert_eq!(resumed.status, CodingAttemptStatus::Running);
    poll_delivery_status(
        &fixture.store,
        &resumed,
        &fixture.manifest.id,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Unsent,
    )
    .await;
}

/// REQ-WIGA-06：投递失败收口必须清理 ack 注册（同 event 可再次注册，供补投递
/// 重试）；once 补投递重发同一 event_id，真实回执才 Delivered。
#[tokio::test]
async fn coding_amendment_delivery_observation_once_reuses_event_and_cleans_registration() {
    let fixture = amendment_fixture().await;
    let (event_tx, event_rx) = mpsc::channel(8);
    drop(event_rx); // 模拟驱动 socket 断开：订阅者消失
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    // 业务先行收口：不被投递通道关闭阻塞或失败。
    let resumed = engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect("receiver drop must not fail business");
    assert_eq!(resumed.status, CodingAttemptStatus::Running);
    let unsent = poll_delivery_status(
        &fixture.store,
        &resumed,
        &fixture.manifest.id,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Unsent,
    )
    .await;

    // 失败收口必须清理 ack 注册：同 event 可再次注册（供补投递重试）。
    let retry = register_plan_amendment_socket_write(&unsent.event_id)
        .expect("failed channel wait must remove the stale ACK registration");
    drop(retry);

    // 补投递（once 直调）：真实回执才 Delivered，且重发同一 event_id。
    let (reconnect_tx, mut reconnect_rx) = mpsc::channel(8);
    let confirm_loop = tokio::spawn(async move {
        while let Some(event) = reconnect_rx.recv().await {
            crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(
                &event,
            );
        }
    });
    let reconnect_engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        reconnect_tx,
    );
    let status = reconnect_engine
        .deliver_plan_amendment_observation_once(&resumed, &fixture.manifest)
        .await
        .unwrap();
    assert_eq!(
        status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Delivered
    );
    confirm_loop.abort();
    let delivered = fixture
        .store
        .get_plan_amendment_delivery(&resumed, &fixture.manifest.id)
        .unwrap();
    assert_eq!(delivered.event_id, unsent.event_id);
    assert!(delivered.delivered_at.is_some());
}

/// send 成功但 Delivered 落盘被 failpoint 拦截：marker 停 Pending，补投递
/// （once 直调）重发同一 event_id 并达成 Delivered。
#[tokio::test]
async fn coding_amendment_delivery_retries_same_event_after_send_before_mark_failure() {
    let fixture = amendment_fixture().await;
    let failpoint = register_plan_amendment_delivery_mark_failpoint(
        &fixture.store,
        &fixture.attempt,
        &fixture.manifest.id,
    );
    let attempt = fixture.attempt.clone();
    let manifest = fixture.manifest.clone();
    let store = fixture.store.clone();
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let apply = tokio::spawn(async move {
        CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx)
            .apply_plan_amendment(&attempt, &manifest)
            .await
    });
    let first_event = tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let first_event_id = match &first_event {
        CodingWsOutMessage::PlanAmendmentUpdated {
            event_id,
            amendment,
        } => {
            assert_eq!(amendment.id, fixture.manifest.id);
            event_id.clone()
        }
        event => panic!("unexpected event after send-before-mark failure: {event:?}"),
    };
    crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(&first_event);
    // 同步观察任务收口：confirm 结算即移除注册表 entry（早于 waiter drop），
    // 不能以「注册可重获」为同步点；观察任务结束会释放其 engine clone——
    // 该 channel 的最后 sender——channel 关闭即 mark 尝试已被 failpoint 拦截。
    let observer_finished =
        tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
            .await
            .expect("observer must drain and close the delivery channel");
    assert!(observer_finished.is_none());
    let resumed = tokio::time::timeout(std::time::Duration::from_secs(2), apply)
        .await
        .expect("apply must finish independently of the observer")
        .unwrap()
        .expect("mark failpoint must not fail business");
    assert_eq!(resumed.status, CodingAttemptStatus::Running);
    assert_eq!(
        fixture
            .store
            .get_plan_amendment_delivery(&resumed, &fixture.manifest.id)
            .unwrap()
            .status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Pending
    );
    drop(failpoint);

    // 补投递（once 直调）：同 event_id 恰一次重发，真实回执达成 Delivered。
    let (reconnect_tx, mut reconnect_rx) = mpsc::channel(8);
    let reconnect_engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        reconnect_tx,
    );
    let recovery_attempt = resumed.clone();
    let recovery_manifest = fixture.manifest.clone();
    let once = tokio::spawn(async move {
        reconnect_engine
            .deliver_plan_amendment_observation_once(&recovery_attempt, &recovery_manifest)
            .await
    });
    let second_event = tokio::time::timeout(std::time::Duration::from_secs(2), reconnect_rx.recv())
        .await
        .expect("redelivery must enqueue the pending amendment event")
        .unwrap();
    let second_event_id = match &second_event {
        CodingWsOutMessage::PlanAmendmentUpdated {
            event_id,
            amendment,
        } => {
            assert_eq!(amendment.id, fixture.manifest.id);
            event_id.clone()
        }
        event => panic!("unexpected recovery event: {event:?}"),
    };
    crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(&second_event);
    let status = tokio::time::timeout(std::time::Duration::from_secs(2), once)
        .await
        .expect("same-event redelivery must finish")
        .unwrap()
        .unwrap();
    assert_eq!(
        status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Delivered
    );
    assert_eq!(second_event_id, first_event_id);
    assert_eq!(
        fixture
            .store
            .get_plan_amendment_delivery(&resumed, &fixture.manifest.id)
            .unwrap()
            .status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Delivered
    );
}

/// 并发恢复恰一路 durable delivery：注册互斥使败者不发送，同 event_id 只
/// 投递一次；双双业务 Ok + Running。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coding_amendment_concurrent_recovery_reconciles_one_durable_delivery() {
    let mut fixture = amendment_fixture().await;
    let failpoint = register_plan_amendment_delivery_mark_failpoint(
        &fixture.store,
        &fixture.attempt,
        &fixture.manifest.id,
    );
    // 种子：apply（failpoint 拦 Delivered 落盘）-> Running + marker Pending。
    // 自建 channel（不走 fixture 消费者）：观察任务结束即释放该 channel 的
    // 最后 sender，recv 返回 None 是「mark 尝试已被 failpoint 拦截」的确定性
    // 同步点（confirm 结算移除注册表 entry 早于 waiter drop，注册探测不可靠）。
    let (seed_tx, mut seed_rx) = mpsc::channel(8);
    let seed_engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), seed_tx);
    seed_engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .expect("seed pending delivery must not fail business");
    drop(seed_engine);
    let seeded_event = tokio::time::timeout(std::time::Duration::from_secs(2), seed_rx.recv())
        .await
        .expect("seed observation event must be enqueued")
        .unwrap();
    crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(&seeded_event);
    let observer_finished =
        tokio::time::timeout(std::time::Duration::from_secs(2), seed_rx.recv())
            .await
            .expect("seed observer must drain and close the delivery channel");
    assert!(observer_finished.is_none());
    drop(failpoint);
    let failed = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let (left_event_tx, mut left_event_rx) = mpsc::channel(8);
    let left_engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        left_event_tx,
    );
    let (right_event_tx, mut right_event_rx) = mpsc::channel(8);
    let right_engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        right_event_tx,
    );
    let left_attempt = failed.clone();
    let left_manifest = fixture.manifest.clone();
    let left =
        tokio::spawn(async move { left_engine.apply_plan_amendment(&left_attempt, &left_manifest).await });
    let right_attempt = failed.clone();
    let right_manifest = fixture.manifest.clone();
    let right = tokio::spawn(
        async move { right_engine.apply_plan_amendment(&right_attempt, &right_manifest).await },
    );
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::select! {
            event = left_event_rx.recv() => event.unwrap(),
            event = right_event_rx.recv() => event.unwrap(),
        }
    })
    .await
    .expect("one recovery must reach socket delivery");
    crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(&event);

    let left = tokio::time::timeout(std::time::Duration::from_secs(2), left)
        .await
        .expect("left recovery deadlocked")
        .unwrap();
    let right = tokio::time::timeout(std::time::Duration::from_secs(2), right)
        .await
        .expect("right recovery deadlocked")
        .unwrap();

    assert_eq!(left.unwrap().status, CodingAttemptStatus::Running);
    assert_eq!(right.unwrap().status, CodingAttemptStatus::Running);
    assert!(left_event_rx.try_recv().is_err());
    assert!(right_event_rx.try_recv().is_err());
    let event_id = match event {
        CodingWsOutMessage::PlanAmendmentUpdated {
            event_id,
            amendment,
        } => {
            assert_eq!(amendment.id, fixture.manifest.id);
            event_id
        }
        event => panic!("unexpected concurrent recovery event: {event:?}"),
    };
    let delivery = fixture
        .store
        .get_plan_amendment_delivery(&failed, &fixture.manifest.id)
        .unwrap();
    assert_eq!(delivery.event_id, event_id);
    assert_eq!(
        delivery.status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Delivered
    );
}

/// REQ-WIGA-06 Review Focus 2：观察端反压（事件通道容量占满）不得改写业务
/// 终态——apply 仍在有界时间内返回 Running，marker 停 Pending。
#[tokio::test]
async fn coding_amendment_delivery_slow_observer_never_backpressures_business() {
    let fixture = amendment_fixture().await;
    let (event_tx, _event_rx) = mpsc::channel(1);
    // 占满唯一槽位：观察投递的 send 将长时间阻塞在容量上。
    let _outstanding_permit = event_tx.clone().reserve_owned().await.unwrap();
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let resumed = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        engine
            .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
            .await
    })
    .await
    .expect("slow observer must not backpressure business")
    .expect("apply must succeed");
    assert_eq!(resumed.status, CodingAttemptStatus::Running);
    poll_delivery_status(
        &fixture.store,
        &resumed,
        &fixture.manifest.id,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Pending,
    )
    .await;
}

fn plan_amendment_event_id(event: &CodingWsOutMessage) -> String {
    match event {
        CodingWsOutMessage::PlanAmendmentUpdated { event_id, .. } => event_id.clone(),
        event => panic!("unexpected amendment delivery event: {event:?}"),
    }
}

#[tokio::test]
async fn coding_amendment_delivery_store_unsent_never_fakes_delivered() {
    let fixture = amendment_fixture().await;
    let attempt = fixture.attempt.clone();
    let seeded = fixture
        .store
        .load_or_prepare_plan_amendment_delivery(&attempt, &fixture.manifest.id)
        .unwrap();
    assert_eq!(
        seeded.status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Pending
    );

    let unsent = fixture
        .store
        .mark_plan_amendment_delivery_unsent(&attempt, &fixture.manifest.id, &seeded.event_id)
        .unwrap();
    assert_eq!(
        unsent.status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Unsent
    );
    assert_eq!(unsent.delivered_at, None);
    // 幂等：重复 Unsent 不改写。
    let again = fixture
        .store
        .mark_plan_amendment_delivery_unsent(&attempt, &fixture.manifest.id, &seeded.event_id)
        .unwrap();
    assert_eq!(
        again.status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Unsent
    );

    // 真实 ack 后不可降级：Delivered 之上 Unsent 必须原样返回。
    let delivered = fixture
        .store
        .mark_plan_amendment_delivery_delivered(&attempt, &fixture.manifest.id, &seeded.event_id)
        .unwrap();
    assert_eq!(
        delivered.status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Delivered
    );
    let guarded = fixture
        .store
        .mark_plan_amendment_delivery_unsent(&attempt, &fixture.manifest.id, &seeded.event_id)
        .unwrap();
    assert_eq!(
        guarded.status,
        crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Delivered
    );
    assert!(guarded.delivered_at.is_some());

    // 异 event_id => IdentityMismatch。
    let error = fixture
        .store
        .mark_plan_amendment_delivery_unsent(&attempt, &fixture.manifest.id, "other_event")
        .unwrap_err();
    assert!(matches!(
        error,
        crate::product::json_store::ProductStoreError::IdentityMismatch { .. }
    ));
}

#[tokio::test]
async fn coding_amendment_delivery_store_lists_own_deliveries_and_rejects_foreign() {
    let fixture = amendment_fixture().await;
    let attempt = fixture.attempt.clone();
    fixture
        .store
        .load_or_prepare_plan_amendment_delivery(&attempt, &fixture.manifest.id)
        .unwrap();
    let listed = fixture.store.list_plan_amendment_deliveries(&attempt).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].amendment_id, fixture.manifest.id);
    assert_eq!(
        listed[0].event_id,
        format!(
            "coding_plan_amendment_updated_{}_{}",
            attempt.id, fixture.manifest.id
        )
    );

    // 异 attempt 身份的文件必须 fail-closed（不静默跳过）。
    let delivery_dir = fixture
        .store
        .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .join("amendment-event-deliveries");
    let foreign = serde_json::json!({
        "id": "coding_plan_amendment_delivery_foreign_amendment_x",
        "event_id": "coding_plan_amendment_updated_foreign_amendment_x",
        "attempt_id": "attempt_other",
        "amendment_id": "amendment_x",
        "status": "pending",
        "delivered_at": null,
        "created_at": "2026-09-26T00:00:00Z",
        "updated_at": "2026-09-26T00:00:00Z",
    });
    std::fs::write(
        delivery_dir.join("amendment_x.json"),
        serde_json::to_vec(&foreign).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        fixture.store.list_plan_amendment_deliveries(&attempt).unwrap_err(),
        crate::product::json_store::ProductStoreError::IdentityMismatch { .. }
    ));
}

#[tokio::test]
async fn coding_amendment_delivery_store_rejects_unsent_with_delivered_at() {
    let fixture = amendment_fixture().await;
    let attempt = fixture.attempt.clone();
    fixture
        .store
        .load_or_prepare_plan_amendment_delivery(&attempt, &fixture.manifest.id)
        .unwrap();
    let marker_path = fixture
        .store
        .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .join("amendment-event-deliveries")
        .join(format!("{}.json", fixture.manifest.id));
    let mut marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&marker_path).unwrap()).unwrap();
    marker["status"] = "unsent".into();
    marker["delivered_at"] = "2026-09-26T00:00:00Z".into();
    std::fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
    assert!(matches!(
        fixture
            .store
            .get_plan_amendment_delivery(&attempt, &fixture.manifest.id)
            .unwrap_err(),
        crate::product::json_store::ProductStoreError::IdentityMismatch { .. }
    ));
}
