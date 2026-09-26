use std::collections::BTreeMap;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::product::coding_models::CodingExecutionStage;
use crate::product::models::{AmendmentResumeMode, AmendmentResumeTarget, PlanAmendmentManifest};
use crate::web::coding_ws_handler::CodingWsOutMessage;
use crate::web::coding_ws_handler::delivery_ack::{
    confirm_plan_amendment_socket_write, fail_plan_amendment_socket_write,
    register_plan_amendment_socket_write,
};
use crate::web::state::{CodingAttemptRunKey, CodingSocketRegistry};

/// F-19：runner/engine 发射面契约——attempt 级 hub 把事件 fan-out 到当前
/// 所有存活 socket（对照 workspace 面 P2 broadcast 先例的 coding 面最小实现）。
fn stage_event() -> CodingWsOutMessage {
    CodingWsOutMessage::CodingStageChange {
        stage: CodingExecutionStage::Coding,
    }
}

#[tokio::test]
async fn hub_broadcasts_events_to_every_registered_socket() {
    let registry = CodingSocketRegistry::default();
    let key = CodingAttemptRunKey::new("project_0001", "issue_0001", "attempt_a");
    let (tx1, mut rx1) = mpsc::channel(16);
    let (tx2, mut rx2) = mpsc::channel(16);
    let token1 = registry.register(&key, tx1);
    let token2 = registry.register(&key, tx2);

    let hub = registry.hub_sender(&key);
    hub.send(stage_event()).await.expect("hub send");

    assert_eq!(rx1.recv().await, Some(stage_event()));
    assert_eq!(rx2.recv().await, Some(stage_event()));
    registry.remove(&key, token1);
    registry.remove(&key, token2);
}

/// F-19 红面一的 lib 级形态：驱动 socket 断开（唯一消费者死亡）后，hub 因
/// producer（runner）持有的 clone 继续存活；无消费者窗口事件被接受不阻塞；
/// 重连 socket 经旧 hub 的 fan-out 路由继续接收（路由目标按现存 socket 动态
/// 解析，与 hub 实例无关）。
#[tokio::test]
async fn hub_survives_driver_socket_disconnect_and_feeds_reconnect() {
    let registry = CodingSocketRegistry::default();
    let key = CodingAttemptRunKey::new("project_0001", "issue_0001", "attempt_b");
    let (driver_tx, driver_rx) = mpsc::channel(16);
    let token = registry.register(&key, driver_tx);
    let producer_hub = registry.hub_sender(&key);
    registry.remove(&key, token);
    drop(driver_rx);

    producer_hub
        .send(stage_event())
        .await
        .expect("send accepted while no consumer attached");

    let (reconnect_tx, mut reconnect_rx) = mpsc::channel(16);
    let reconnect_token = registry.register(&key, reconnect_tx);
    producer_hub
        .send(stage_event())
        .await
        .expect("hub send after reconnect");
    assert_eq!(
        reconnect_rx.recv().await,
        Some(stage_event()),
        "重连 socket 必须经旧 hub 路由继续收到 runner 事件"
    );
    registry.remove(&key, reconnect_token);
}

/// REQ-WIGA-06：plan_amendment 激活不再要求存活 socket；hub 可在零订阅者时
/// 建立，零 fan-out 由 delivery ack 立即结算失败（观察层记 Unsent）。
#[tokio::test]
async fn hub_zero_fanout_settles_amendment_waiter_unsent() {
    let registry = CodingSocketRegistry::default();
    let key = CodingAttemptRunKey::new("project_0001", "issue_0001", "attempt_e");
    let hub = registry.hub_sender(&key);
    let waiter = register_plan_amendment_socket_write("event_zero_fanout").unwrap();
    hub.send(plan_amendment_event("event_zero_fanout"))
        .await
        .unwrap();
    let error = tokio::time::timeout(Duration::from_millis(250), waiter.wait())
        .await
        .expect("zero fan-out must settle immediately")
        .expect_err("zero fan-out must not acknowledge delivery");
    assert!(
        error.to_string().contains("plan_amendment_socket_write_failed"),
        "unexpected settlement error: {error}"
    );
}

/// 生命周期收口：producer 退出且全部 socket 断开后，旧 hub 路由随 channel
/// 关闭自然结束；后续 hub_sender 必须给出可工作的新 hub（无陈旧 entry）。
#[tokio::test]
async fn fresh_hub_serves_after_producers_and_sockets_gone() {
    let registry = CodingSocketRegistry::default();
    let key = CodingAttemptRunKey::new("project_0001", "issue_0001", "attempt_d");
    let (tx, mut rx) = mpsc::channel(16);
    let token = registry.register(&key, tx);
    let hub = registry.hub_sender(&key);
    hub.send(stage_event()).await.expect("hub send");
    assert_eq!(rx.recv().await, Some(stage_event()));

    registry.remove(&key, token);
    drop(hub);
    drop(rx);
    tokio::task::yield_now().await;

    let (tx2, mut rx2) = mpsc::channel(16);
    let token2 = registry.register(&key, tx2);
    let hub2 = registry.hub_sender(&key);
    hub2.send(stage_event()).await.expect("new hub send");
    assert_eq!(
        rx2.recv().await,
        Some(stage_event()),
        "新一轮 hub 必须正常 fan-out，不得命中已死路由"
    );
    registry.remove(&key, token2);
}

fn plan_amendment_event(event_id: &str) -> CodingWsOutMessage {
    CodingWsOutMessage::PlanAmendmentUpdated {
        event_id: event_id.to_string(),
        amendment: Box::new(PlanAmendmentManifest {
            id: format!("plan_amendment_{event_id}"),
            repair_request_id: format!("plan_repair_request_{event_id}"),
            previous_plan_revision_id: "plan_revision_0001".to_string(),
            new_plan_revision_id: "plan_revision_0002".to_string(),
            revised_work_items: BTreeMap::new(),
            superseded_revisions: Vec::new(),
            dependency_graph_changes: Vec::new(),
            contract_deltas: Vec::new(),
            unaffected_units: Vec::new(),
            revalidation_required_units: Vec::new(),
            stale_units: Vec::new(),
            replacement_units: BTreeMap::new(),
            resume_target: AmendmentResumeTarget {
                logical_work_item_id: format!("work_item_{event_id}"),
                mode: AmendmentResumeMode::Reexecute,
            },
            created_at: "2026-09-20T00:00:00Z".to_string(),
        }),
    }
}

/// k3 P1 红测：驱动断开后 runner 持 hub clone 保活，amendment 事件广播时
/// registry 无 sockets entry——必须快速 fail 该事件的 socket-write ack
/// （等价旧直连路径 channel 关闭的快速失败），不得让 waiter 无限阻塞。
#[tokio::test]
async fn hub_without_live_sockets_fails_plan_amendment_delivery_quickly() {
    let registry = CodingSocketRegistry::default();
    let key = CodingAttemptRunKey::new("project_0001", "issue_0001", "attempt_p1");
    let (tx, rx) = mpsc::channel(16);
    let token = registry.register(&key, tx);
    let hub = registry.hub_sender(&key);
    registry.remove(&key, token);
    drop(rx);

    let waiter =
        register_plan_amendment_socket_write("plan_amendment_f19_p1").expect("delivery waiter");
    hub.send(plan_amendment_event("plan_amendment_f19_p1"))
        .await
        .expect("hub send without live sockets");

    let outcome = tokio::time::timeout(Duration::from_secs(2), waiter.wait()).await;
    assert!(
        outcome.is_ok_and(|result| result.is_err()),
        "无存活消费者必须快速失败，不得让 amendment waiter 无限阻塞"
    );
}

/// k3 P2 红测：PlanAmendmentUpdated fan-out 到多个 socket，一败一胜必须按
/// 成功结算（任一 socket 写成功即 confirm；全部失败才 fail）——单份 ack
/// 首 settle 胜出会把成功的驱动写误判为失败并中断 amendment。
#[tokio::test]
async fn plan_amendment_fan_out_one_failure_one_success_confirms() {
    let registry = CodingSocketRegistry::default();
    let key = CodingAttemptRunKey::new("project_0001", "issue_0001", "attempt_p2");
    let (tx_a, _rx_a) = mpsc::channel(16);
    let (tx_b, mut rx_b) = mpsc::channel(16);
    let token_a = registry.register(&key, tx_a);
    let token_b = registry.register(&key, tx_b);
    let hub = registry.hub_sender(&key);

    let event = plan_amendment_event("plan_amendment_f19_p2");
    let waiter =
        register_plan_amendment_socket_write("plan_amendment_f19_p2").expect("delivery waiter");
    hub.send(event.clone()).await.expect("hub send");
    assert!(
        matches!(
            rx_b.recv().await,
            Some(CodingWsOutMessage::PlanAmendmentUpdated { .. })
        ),
        "广播必须送达存活 socket"
    );

    fail_plan_amendment_socket_write(&event);
    confirm_plan_amendment_socket_write(&event);

    let outcome = tokio::time::timeout(Duration::from_secs(2), waiter.wait())
        .await
        .expect("delivery ack must settle");
    assert!(
        outcome.is_ok(),
        "一败一胜必须按成功结算（任一成功即 confirm），不得被首个失败误判"
    );
    registry.remove(&key, token_a);
    registry.remove(&key, token_b);
}
