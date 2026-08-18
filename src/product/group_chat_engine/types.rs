use serde::{Deserialize, Serialize};

use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
use crate::product::models::ProviderName;

/// 群聊中角色的固定业务身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupChatRoleKey {
    Author,
    FrontendDesign,
    BackendDesign,
    Reviewer,
    Researcher,
}

/// 绑定到 provider 的一个群聊角色实例。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleInstance {
    pub id: String,
    pub role_key: GroupChatRoleKey,
    pub provider: ProviderName,
    pub display_name: String,
    pub permission_mode: ProviderPermissionMode,
    pub seen_cursor: u64,
    pub injection_watermark: u64,
}

/// 产物线内草稿槽的稳定字符串标识。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DraftSlotKey(pub String);

/// 草稿槽中当前的版本化 Markdown 草稿。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDraft {
    pub version: u32,
    pub markdown: String,
    pub author_role_id: String,
    pub based_on_events: u64,
}

/// 某个草稿槽的排他认领记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub holder_role_id: String,
    pub claimed_at: String,
}

/// 产物线中可独立认领与版本化的草稿槽。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftSlot {
    pub slot_key: DraftSlotKey,
    pub current: Option<ArtifactDraft>,
    pub claim: Option<Claim>,
}

/// 产物线的业务类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactLineKind {
    IssueRefinement,
    StorySpec,
    DesignSpec,
}

/// 一条产物线及其草稿槽和已定稿版本引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactLine {
    pub kind: ArtifactLineKind,
    pub drafts: Vec<DraftSlot>,
    pub finalized_versions: Vec<String>,
    /// 对应的生命周期实体；群聊首次定稿后写入并在后续定稿中复用。
    #[serde(default)]
    pub entity_id: Option<String>,
    /// 看板桥接 workspace session；群聊首次定稿后写入并复用。
    #[serde(default)]
    pub bridge_session_id: Option<String>,
}

/// Agent 发言关联的草稿版本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub line: ArtifactLineKind,
    pub slot: DraftSlotKey,
    pub version: u32,
}

/// 聊天室追加式时间线事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomEvent {
    UserMessage {
        text: String,
        mentions: Vec<String>,
    },
    AgentMessage {
        role_instance_id: String,
        text: String,
        artifact_ref: Option<ArtifactRef>,
        cursor_after: u64,
    },
    ClaimEvent {
        role_instance_id: String,
        line: ArtifactLineKind,
        slot_key: DraftSlotKey,
        claimed: bool,
    },
    HeldEvent {
        role_instance_id: String,
        reason: String,
        cursor_after: u64,
    },
    FinalizeEvent {
        artifact_line: ArtifactLineKind,
        version: String,
        included_slots: Vec<DraftSlotKey>,
    },
    SystemNotice {
        text: String,
    },
}

/// 群聊会话的生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupChatSessionStatus {
    Active,
    Finalized,
    Archived,
}

/// 群聊会话的可恢复快照记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupChatSessionRecord {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub status: GroupChatSessionStatus,
    pub roles: Vec<RoleInstance>,
    pub artifact_lines: Vec<ArtifactLine>,
    pub created_at: String,
    pub updated_at: String,
}
