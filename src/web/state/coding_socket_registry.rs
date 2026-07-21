use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::web::coding_ws_handler::CodingWsOutMessage;

use super::CodingAttemptRunKey;

#[derive(Clone, Default)]
pub struct CodingSocketRegistry {
    inner: Arc<Mutex<CodingSocketRegistryInner>>,
}

#[derive(Default)]
struct CodingSocketRegistryInner {
    next_token: u64,
    sockets: HashMap<CodingAttemptRunKey, BTreeMap<u64, mpsc::Sender<CodingWsOutMessage>>>,
}

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

    pub fn sender(
        &self,
        attempt_key: &CodingAttemptRunKey,
    ) -> Option<mpsc::Sender<CodingWsOutMessage>> {
        let mut inner = self.inner.lock().expect("coding socket registry lock");
        let sockets = inner.sockets.get_mut(attempt_key)?;
        sockets.retain(|_, sender| !sender.is_closed());
        let sender = sockets.last_key_value().map(|(_, sender)| sender.clone());
        if sockets.is_empty() {
            inner.sockets.remove(attempt_key);
        }
        sender
    }

    pub fn remove(&self, attempt_key: &CodingAttemptRunKey, token: u64) {
        let mut inner = self.inner.lock().expect("coding socket registry lock");
        if let Some(sockets) = inner.sockets.get_mut(attempt_key) {
            sockets.remove(&token);
            if sockets.is_empty() {
                inner.sockets.remove(attempt_key);
            }
        }
    }
}
