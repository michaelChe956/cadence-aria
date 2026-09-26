use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tokio::sync::{mpsc, oneshot};

use crate::product::json_store::{ProductStoreError, validate_relative_id};

use super::CodingWsOutMessage;

struct SocketWriteAckEntry {
    registration_id: u64,
    sender: oneshot::Sender<bool>,
    /// F-19/k3-P2：fan-out 写结算份额。None=未登记（单写语义：首个 settle 即
    /// 结算）；Some(n)=还有 n 份 socket 写待结算——任一 confirm 立即结算成功，
    /// 递减到零（全部失败）才结算失败。
    pending_writes: Option<usize>,
}

pub(crate) struct PlanAmendmentSocketWriteWaiter {
    event_id: String,
    registration_id: u64,
    receiver: Option<oneshot::Receiver<bool>>,
}

static SOCKET_WRITE_ACKS: OnceLock<Mutex<HashMap<String, SocketWriteAckEntry>>> = OnceLock::new();
static NEXT_SOCKET_WRITE_ACK_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn register_plan_amendment_socket_write(
    event_id: &str,
) -> Result<PlanAmendmentSocketWriteWaiter, ProductStoreError> {
    validate_relative_id(event_id)?;
    let registration_id = NEXT_SOCKET_WRITE_ACK_ID.fetch_add(1, Ordering::Relaxed);
    let (sender, receiver) = oneshot::channel();
    let mut acknowledgements = socket_write_acknowledgements()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if acknowledgements.contains_key(event_id) {
        return Err(identity_mismatch(event_id));
    }
    acknowledgements.insert(
        event_id.to_string(),
        SocketWriteAckEntry {
            registration_id,
            sender,
            pending_writes: None,
        },
    );
    Ok(PlanAmendmentSocketWriteWaiter {
        event_id: event_id.to_string(),
        registration_id,
        receiver: Some(receiver),
    })
}

impl PlanAmendmentSocketWriteWaiter {
    pub(crate) async fn wait(mut self) -> Result<(), ProductStoreError> {
        let receiver = self
            .receiver
            .take()
            .ok_or_else(|| identity_mismatch(&self.event_id))?;
        match receiver.await {
            Ok(true) => Ok(()),
            Ok(false) | Err(_) => Err(ProductStoreError::Io(format!(
                "plan_amendment_socket_write_failed:{}",
                self.event_id
            ))),
        }
    }

    pub(crate) async fn wait_or_channel_closed(
        self,
        event_tx: &mpsc::Sender<CodingWsOutMessage>,
    ) -> Result<(), ProductStoreError> {
        let event_id = self.event_id.clone();
        tokio::select! {
            biased;
            _ = event_tx.closed() => Err(ProductStoreError::Io(format!(
                "plan_amendment_delivery_channel_closed:{event_id}"
            ))),
            result = self.wait() => result,
        }
    }
}

impl Drop for PlanAmendmentSocketWriteWaiter {
    fn drop(&mut self) {
        let mut acknowledgements = socket_write_acknowledgements()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if acknowledgements
            .get(&self.event_id)
            .is_some_and(|entry| entry.registration_id == self.registration_id)
        {
            acknowledgements.remove(&self.event_id);
        }
    }
}

pub(crate) fn confirm_plan_amendment_socket_write(message: &CodingWsOutMessage) {
    settle_plan_amendment_socket_write(message, true);
}

pub(crate) fn fail_plan_amendment_socket_write(message: &CodingWsOutMessage) {
    settle_plan_amendment_socket_write(message, false);
}

/// F-19/k3-P2：broadcast fan-out **发送前**登记该事件的写份额。socket 循环的
/// 写结算（confirm/fail）只会在事件进入 channel 之后发生，登记先行即无竞态。
/// 零份额（k3-P1：registry 无该 attempt 的 sockets entry / 全部关闭）在此
/// 立即结算失败，恢复旧直连路径 channel 关闭的快速失败语义。
pub(crate) fn expect_plan_amendment_fan_out_writes(message: &CodingWsOutMessage, writes: usize) {
    let CodingWsOutMessage::PlanAmendmentUpdated { event_id, .. } = message else {
        return;
    };
    let mut acknowledgements = socket_write_acknowledgements()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(entry) = acknowledgements.get_mut(event_id) else {
        return;
    };
    if entry.pending_writes.is_some() {
        return;
    }
    if writes == 0 {
        let entry = acknowledgements.remove(event_id).expect("entry present");
        let _ = entry.sender.send(false);
        return;
    }
    entry.pending_writes = Some(writes);
}

/// amendment 族统一补投递触发器（REQ-WIGA-06）：socket attach 与激活
/// early-return 共用，禁止各自内联第二份。以 transient
/// CodingWorkspaceEngine spawn `redeliver_undelivered_plan_amendments`，
/// 不阻塞调用方；失败仅 warn。
pub(crate) fn spawn_undelivered_amendment_redelivery(
    coding_store: crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: crate::product::coding_models::CodingExecutionAttempt,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
) {
    tokio::spawn(async move {
        let engine = crate::product::coding_workspace_engine::CodingWorkspaceEngine::new(
            coding_store,
            crate::product::git_workspace_service::GitWorkspaceService::new(),
            event_tx,
        );
        if let Err(error) = engine
            .redeliver_undelivered_plan_amendments(&attempt)
            .await
        {
            tracing::warn!(%error, "plan_amendment_delivery_redelivery_failed");
        }
    });
}

fn settle_plan_amendment_socket_write(message: &CodingWsOutMessage, written: bool) {
    let CodingWsOutMessage::PlanAmendmentUpdated { event_id, .. } = message else {
        return;
    };
    let mut acknowledgements = socket_write_acknowledgements()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(entry) = acknowledgements.get_mut(event_id) else {
        return;
    };
    // 任一成功立即结算成功；未登记份额的单写路径保持「首 settle 即结算」；
    // 登记过的份额递减，最后一份失败才结算失败。
    let settle_now = written
        || match entry.pending_writes {
            None => true,
            Some(remaining) if remaining <= 1 => true,
            Some(remaining) => {
                entry.pending_writes = Some(remaining - 1);
                false
            }
        };
    if settle_now {
        let entry = acknowledgements.remove(event_id).expect("entry present");
        let _ = entry.sender.send(written);
    }
}

fn socket_write_acknowledgements() -> &'static Mutex<HashMap<String, SocketWriteAckEntry>> {
    SOCKET_WRITE_ACKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn identity_mismatch(event_id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: "coding_plan_amendment_socket_write",
        id: event_id.to_string(),
    }
}
