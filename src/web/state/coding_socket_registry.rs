use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};

use crate::web::coding_ws_handler::CodingWsOutMessage;
use crate::web::coding_ws_handler::delivery_ack::expect_plan_amendment_fan_out_writes;
use crate::web::coding_ws_handler::delivery_ack::fail_plan_amendment_socket_write;

use super::CodingAttemptRunKey;

/// hub 事件通道容量与单连接 socket 通道（socket.rs 的 1024）一致：runner→hub
/// 与 hub→socket 两跳的背压对齐既有单消费者直连路径，慢消费者仍向上游传播。
const CODING_EVENT_HUB_CAPACITY: usize = 1024;
/// barrier 控制通道容量：仅 socket 循环 flush 前串行使用，8 足够。
const CODING_HUB_BARRIER_CAPACITY: usize = 8;

#[derive(Clone, Default)]
pub struct CodingSocketRegistry {
    inner: Arc<Mutex<CodingSocketRegistryInner>>,
}

#[derive(Default)]
struct CodingSocketRegistryInner {
    next_token: u64,
    sockets: HashMap<CodingAttemptRunKey, BTreeMap<u64, mpsc::Sender<CodingWsOutMessage>>>,
    /// F-19：per-attempt 事件 hub。registry 持有的 clone 仅在「该 attempt 仍有
    /// 存活 socket」期间保活；runner/engine 持有的 event clone 使 hub 跨驱动
    /// 连接断开继续存活，重连 socket 经 fan-out 动态解析继续接收。
    hubs: HashMap<CodingAttemptRunKey, CodingEventHub>,
}

#[derive(Clone)]
struct CodingEventHub {
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    barrier_tx: mpsc::Sender<oneshot::Sender<()>>,
}

/// 新建 hub 时交由路由任务接管的两个接收端。
type FreshHubReceivers = (
    mpsc::Receiver<CodingWsOutMessage>,
    mpsc::Receiver<oneshot::Sender<()>>,
);

impl CodingSocketRegistry {
    pub fn register(
        &self,
        attempt_key: &CodingAttemptRunKey,
        sender: mpsc::Sender<CodingWsOutMessage>,
    ) -> u64 {
        let mut inner = self.inner.lock().expect("coding socket registry lock");
        inner.next_token += 1;
        let token = inner.next_token;
        inner
            .sockets
            .entry(attempt_key.clone())
            .or_default()
            .insert(token, sender);
        token
    }

    pub fn remove(&self, attempt_key: &CodingAttemptRunKey, token: u64) {
        let mut inner = self.inner.lock().expect("coding socket registry lock");
        if let Some(sockets) = inner.sockets.get_mut(attempt_key) {
            sockets.remove(&token);
            if sockets.is_empty() {
                inner.sockets.remove(attempt_key);
                // 最后一个 socket 断开：释放 registry 持有的 hub 保活引用。若
                // runner/engine 仍持有 event clone，hub 路由继续存活并向后续
                // 重连的 socket fan-out；全部 producer 退出后 hub channel 关闭、
                // 路由任务自然结束，不泄漏。
                inner.hubs.remove(attempt_key);
            }
        }
    }

    /// F-19：runner/engine 的事件发射面。返回 attempt 级 hub sender——事件经
    /// per-attempt 路由任务 fan-out 到当前所有存活 socket。此前 spawn 传入的
    /// 是驱动 socket 的私有 channel：唯一消费者断开即死、恢复后驱动失明。
    pub fn hub_sender(
        &self,
        attempt_key: &CodingAttemptRunKey,
    ) -> mpsc::Sender<CodingWsOutMessage> {
        let (hub, fresh_rx) = self.hub_channel(attempt_key);
        if let Some((hub_rx, barrier_rx)) = fresh_rx {
            self.spawn_hub_router(attempt_key, hub_rx, barrier_rx);
        }
        hub.event_tx
    }

    /// 保留「至少一个存活消费者」fail-closed 语义的发射面获取（plan_amendment
    /// 激活等）：无存活 socket 时返回 None，有则与 `hub_sender` 同款 hub。
    pub fn hub_sender_if_live(
        &self,
        attempt_key: &CodingAttemptRunKey,
    ) -> Option<mpsc::Sender<CodingWsOutMessage>> {
        {
            let mut inner = self.inner.lock().expect("coding socket registry lock");
            let has_live_socket = inner.sockets.get_mut(attempt_key).is_some_and(|sockets| {
                sockets.retain(|_, sender| !sender.is_closed());
                !sockets.is_empty()
            });
            if !has_live_socket {
                return None;
            }
        }
        Some(self.hub_sender(attempt_key))
    }

    /// 因果序 barrier：阻塞到「调用时刻之前已入 hub 的事件」全部 fan-out 进
    /// 各 socket channel。socket 循环在 engine 操作后、flush/直写快照前调用，
    /// 恢复旧直连路径下「engine 事件先于快照」的 wire 顺序（hub 多了一跳异步
    /// 转发，无 barrier 时直写快照会抢在事件前到达客户端）。
    pub async fn wait_until_hub_drained(&self, attempt_key: &CodingAttemptRunKey) {
        let barrier_tx = {
            let inner = self.inner.lock().expect("coding socket registry lock");
            inner
                .hubs
                .get(attempt_key)
                .map(|hub| hub.barrier_tx.clone())
        };
        let Some(barrier_tx) = barrier_tx else {
            return;
        };
        let (ack_tx, ack_rx) = oneshot::channel();
        if barrier_tx.send(ack_tx).await.is_ok() {
            let _ = ack_rx.await;
        }
    }

    fn hub_channel(
        &self,
        attempt_key: &CodingAttemptRunKey,
    ) -> (CodingEventHub, Option<FreshHubReceivers>) {
        let mut inner = self.inner.lock().expect("coding socket registry lock");
        if let Some(hub) = inner.hubs.get(attempt_key) {
            return (hub.clone(), None);
        }
        let (event_tx, event_rx) = mpsc::channel(CODING_EVENT_HUB_CAPACITY);
        let (barrier_tx, barrier_rx) = mpsc::channel(CODING_HUB_BARRIER_CAPACITY);
        let hub = CodingEventHub {
            event_tx: event_tx.clone(),
            barrier_tx,
        };
        inner.hubs.insert(attempt_key.clone(), hub.clone());
        (hub, Some((event_rx, barrier_rx)))
    }

    fn spawn_hub_router(
        &self,
        attempt_key: &CodingAttemptRunKey,
        mut hub_rx: mpsc::Receiver<CodingWsOutMessage>,
        mut barrier_rx: mpsc::Receiver<oneshot::Sender<()>>,
    ) {
        let registry = self.clone();
        let attempt_key = attempt_key.clone();
        tokio::spawn(async move {
            // 单路由顺序转发保证 per-socket FIFO；hub 发送端全部释放（runner
            // 退出且 registry 已摘除保活引用）时 recv 返回 None，任务结束。
            // biased 轮询保证 barrier ack 只在 hub 瞬时空转时放行，即 barrier
            // 之前入队的事件必已 fan-out 完毕（因果序 barrier 语义）。
            let mut barrier_open = true;
            loop {
                tokio::select! {
                    biased;
                    maybe_event = hub_rx.recv() => match maybe_event {
                        Some(event) => registry.broadcast(&attempt_key, &event).await,
                        None => break,
                    },
                    maybe_barrier = barrier_rx.recv(), if barrier_open => match maybe_barrier {
                        Some(ack) => {
                            let _ = ack.send(());
                        }
                        None => {
                            barrier_open = false;
                        }
                    },
                }
            }
        });
    }

    /// hub 路由 fan-out：目标按 registry 现存 socket 动态解析（与 hub 实例无关，
    /// 旧 hub 的路由同样能把事件送到新 socket）。发送前先向 delivery ack 登记
    /// 该事件的写份额：任一 socket 写成功即 confirm、全部失败才 fail；零份额
    /// （k3-P1：无 sockets entry / 全部关闭）由登记侧立即结算失败——等价旧
    /// 直连路径下 channel 关闭的快速失败语义，amendment waiter 不悬挂。
    async fn broadcast(&self, attempt_key: &CodingAttemptRunKey, event: &CodingWsOutMessage) {
        let targets: Vec<_> = {
            let mut inner = self.inner.lock().expect("coding socket registry lock");
            match inner.sockets.get_mut(attempt_key) {
                Some(sockets) => {
                    sockets.retain(|_, sender| !sender.is_closed());
                    if sockets.is_empty() {
                        inner.sockets.remove(attempt_key);
                        Vec::new()
                    } else {
                        sockets.values().cloned().collect()
                    }
                }
                None => Vec::new(),
            }
        };
        expect_plan_amendment_fan_out_writes(event, targets.len());
        let mut failed_sends = 0usize;
        for target in targets {
            if target.send(event.clone()).await.is_err() {
                failed_sends += 1;
            }
        }
        for _ in 0..failed_sends {
            fail_plan_amendment_socket_write(event);
        }
    }
}
