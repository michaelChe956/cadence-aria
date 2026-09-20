use tokio::sync::mpsc;

use crate::product::coding_models::CodingExecutionStage;
use crate::web::coding_ws_handler::CodingWsOutMessage;
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

/// plan_amendment 激活路径的 fail-closed 门：无存活 socket 时不得给出 hub。
#[tokio::test]
async fn hub_sender_if_live_requires_live_socket() {
    let registry = CodingSocketRegistry::default();
    let key = CodingAttemptRunKey::new("project_0001", "issue_0001", "attempt_c");
    assert!(registry.hub_sender_if_live(&key).is_none());

    let (tx, rx) = mpsc::channel(16);
    let token = registry.register(&key, tx);
    assert!(registry.hub_sender_if_live(&key).is_some());

    registry.remove(&key, token);
    drop(rx);
    assert!(
        registry.hub_sender_if_live(&key).is_none(),
        "已关闭 socket 不得让 fail-closed 门误放行"
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
