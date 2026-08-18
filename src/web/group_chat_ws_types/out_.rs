use serde::{Deserialize, Serialize};

use crate::product::group_chat_engine::types::RoomEvent;

/// 群聊独立 WS 的服务端消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GroupChatWsOutMessage {
    /// seq 与事件一起发送，客户端可用它作为重连游标。
    RoomEvent {
        seq: u64,
        event: RoomEvent,
    },
    TurnStarted {
        role_instance_id: String,
    },
    TurnDelta {
        role_instance_id: String,
        delta: String,
    },
    TurnHeld {
        role_instance_id: String,
        reason: String,
    },
    Error {
        code: String,
        message: String,
    },
    Pong,
}
