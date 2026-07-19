use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tokio::sync::oneshot;

use crate::product::json_store::{ProductStoreError, validate_relative_id};

use super::CodingWsOutMessage;

struct SocketWriteAckEntry {
    registration_id: u64,
    sender: oneshot::Sender<bool>,
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

fn settle_plan_amendment_socket_write(message: &CodingWsOutMessage, written: bool) {
    let CodingWsOutMessage::PlanAmendmentUpdated { event_id, .. } = message else {
        return;
    };
    let entry = socket_write_acknowledgements()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(event_id);
    if let Some(entry) = entry {
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
