use serde::{Deserialize, Serialize};

use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
use crate::product::group_chat_engine::types::{ArtifactLineKind, DraftSlotKey, GroupChatRoleKey};
use crate::product::models::ProviderName;

/// 群聊独立 WS 的客户端消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GroupChatWsInMessage {
    SendMessage {
        text: String,
        #[serde(default)]
        mentions: Vec<String>,
        #[serde(default)]
        draft_slot: Option<DraftSlotKey>,
    },
    AddRole {
        role_key: GroupChatRoleKey,
        provider: ProviderName,
        #[serde(default)]
        display_name: Option<String>,
        #[serde(default)]
        permission_mode: Option<ProviderPermissionMode>,
    },
    Finalize {
        line_kind: ArtifactLineKind,
        #[serde(default, alias = "included_slots_override")]
        included_slots: Option<Vec<DraftSlotKey>>,
        #[serde(default)]
        confirmed_by: Option<String>,
    },
    Ping,
}
