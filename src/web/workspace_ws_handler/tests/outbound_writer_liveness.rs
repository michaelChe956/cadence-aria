// —— idle 掐线修复（3.6 F7 项 1）：双向活性口径 ——
// 服务器成功出站 = 连接健康：静默客户端（auto 流 driver 只收不发）在
// current_run=None 窗口不得被误掐；服务器也静默后才按既有语义回收。
use super::*;

use super::*;
use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

/// 收集型 sink：模拟「socket 写出成功」，供出站写泵单测观测。
#[derive(Default, Clone)]
struct CollectOutboundSink {
    sent: Arc<std::sync::Mutex<Vec<Message>>>,
}

impl futures_util::Sink<Message> for CollectOutboundSink {
    type Error = Infallible;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        self.sent.lock().unwrap().push(item);
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

/// 恒失败 sink：模拟对端不可达（socket 写出失败）。
struct FailingOutboundSink;

impl futures_util::Sink<Message> for FailingOutboundSink {
    type Error = std::io::Error;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Err(std::io::Error::other("sink unreachable")))
    }

    fn start_send(self: Pin<&mut Self>, _: Message) -> Result<(), Self::Error> {
        Err(std::io::Error::other("sink unreachable"))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Err(std::io::Error::other("sink unreachable")))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Err(std::io::Error::other("sink unreachable")))
    }
}

#[tokio::test]
async fn outbound_writer_keeps_silent_client_alive_while_server_writes() {
    // F3 运营验证实证（run1b/1c）：静默客户端在 current_run=None 窗口被
    // 1005 掐线。服务器持续出站期间连接是健康的，不得掐线。
    let liveness = Arc::new(Mutex::new(tokio::time::Instant::now()));
    let (writer_tx, writer_rx) = mpsc::channel::<OutboundControl>(8);
    let (idle_tx, mut idle_rx) = mpsc::channel::<OutboundControl>(1);
    let idle_task = spawn_idle_timeout_task(
        liveness.clone(),
        idle_tx,
        Arc::new(|| false), // current_run=None 窗口：is_active_run=false
        std::time::Duration::from_millis(40),
        std::time::Duration::from_millis(5),
    );
    let sink = CollectOutboundSink::default();
    let observed = sink.sent.clone();
    let writer_task = tokio::spawn(pump_outbound_controls(writer_rx, sink, liveness.clone()));

    // 模拟事件流 fixture：服务器每 15ms 成功写出一条出站消息，持续 ~150ms
    // （远超 40ms idle 阈值）；客户端全程静默（不触碰 liveness）。
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        writer_tx
            .send(OutboundControl::Text(
                "{\"type\":\"simulated_event\"}".to_string(),
            ))
            .await
            .expect("enqueue outbound");
    }

    // 静默客户端 + 服务器持续出站：最后一条写出后（阈值 40ms - 裕量）内不得掐线。
    // 窗口必须严格小于 idle 阈值，否则会把「停写后按期回收」误判为掐线。
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(25), idle_rx.recv())
            .await
            .is_err(),
        "idle timeout must not fire while the server keeps writing outbound messages"
    );
    assert_eq!(
        observed.lock().unwrap().len(),
        10,
        "simulated event stream must be written out"
    );

    // 服务器也静默 + 客户端静默 → 仍按期回收（真死连接语义保持）
    drop(writer_tx);
    let control = tokio::time::timeout(std::time::Duration::from_millis(200), idle_rx.recv())
        .await
        .expect("idle timeout after both sides silent")
        .expect("close control");
    assert!(matches!(control, OutboundControl::CloseDueToIdleTimeout));

    idle_task.abort();
    writer_task.abort();
}

#[tokio::test]
async fn outbound_writer_failed_write_does_not_refresh_liveness() {
    // 写失败（对端不可达）不算「成功出站」，不得刷新连接活跃时间：
    // 真死连接仍进入 idle 回收计时。
    let liveness = Arc::new(Mutex::new(
        tokio::time::Instant::now() - std::time::Duration::from_secs(600),
    ));
    let (writer_tx, writer_rx) = mpsc::channel::<OutboundControl>(1);
    let task = tokio::spawn(pump_outbound_controls(
        writer_rx,
        FailingOutboundSink,
        liveness.clone(),
    ));
    writer_tx
        .send(OutboundControl::Text("{}".to_string()))
        .await
        .expect("enqueue outbound");
    drop(writer_tx);
    let _ = task.await;

    assert!(
        liveness.lock().await.elapsed() > std::time::Duration::from_secs(300),
        "failed outbound writes must not refresh connection liveness"
    );
}
