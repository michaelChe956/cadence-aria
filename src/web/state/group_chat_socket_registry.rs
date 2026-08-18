use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};

use crate::web::group_chat_ws_types::GroupChatWsOutMessage;

/// 群聊 WS 的轻量进程内扇出注册表。
///
/// 事件的可靠来源是 GroupChatStore；这里仅负责把当前连接的消息送到各 socket。
#[derive(Clone, Default)]
pub struct GroupChatSocketRegistry {
    sockets: Arc<Mutex<HashMap<String, Vec<mpsc::Sender<GroupChatWsOutMessage>>>>>,
}

impl GroupChatSocketRegistry {
    pub async fn subscribe(&self, session_id: String) -> mpsc::Receiver<GroupChatWsOutMessage> {
        let (tx, rx) = mpsc::channel(128);
        self.sockets
            .lock()
            .await
            .entry(session_id)
            .or_default()
            .push(tx);
        rx
    }

    pub async fn publish(&self, session_id: &str, message: GroupChatWsOutMessage) {
        let mut sockets = self.sockets.lock().await;
        let Some(listeners) = sockets.get_mut(session_id) else {
            return;
        };
        listeners.retain(|sender| !sender.is_closed());
        for sender in listeners.iter() {
            let _ = sender.try_send(message.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publishes_only_to_subscribers_of_same_session() {
        let registry = GroupChatSocketRegistry::default();
        let mut room = registry.subscribe("room-1".to_owned()).await;
        let mut other = registry.subscribe("room-2".to_owned()).await;
        registry
            .publish("room-1", GroupChatWsOutMessage::Pong)
            .await;
        assert_eq!(room.recv().await, Some(GroupChatWsOutMessage::Pong));
        assert!(other.try_recv().is_err());
    }
}
