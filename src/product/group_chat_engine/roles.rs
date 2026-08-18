use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
use crate::protocol::contracts::AdapterRole;

use super::types::{DraftSlotKey, GroupChatRoleKey};

/// Issue 澄清稿草稿槽。
pub const ISSUE_FULL_SLOT: &str = "issue_full";
/// Story Spec 草稿槽。
pub const STORY_FULL_SLOT: &str = "story_full";
/// Design Spec 前端分节草稿槽。
pub const DESIGN_FRONTEND_SLOT: &str = "design_frontend";
/// Design Spec 后端分节草稿槽。
pub const DESIGN_BACKEND_SLOT: &str = "design_backend";
/// Design Spec 汇总草稿槽。
pub const DESIGN_SUMMARY_SLOT: &str = "design_summary";

/// 返回角色是否具备产物草稿写权限。
pub fn can_write_artifacts(role_key: GroupChatRoleKey) -> bool {
    matches!(
        role_key,
        GroupChatRoleKey::Author
            | GroupChatRoleKey::FrontendDesign
            | GroupChatRoleKey::BackendDesign
    )
}

/// 返回角色可以认领和写入的草稿槽。
pub fn writable_slots(role_key: GroupChatRoleKey) -> Vec<DraftSlotKey> {
    let slots = match role_key {
        GroupChatRoleKey::Author => &[ISSUE_FULL_SLOT, STORY_FULL_SLOT, DESIGN_SUMMARY_SLOT][..],
        GroupChatRoleKey::FrontendDesign => &[DESIGN_FRONTEND_SLOT][..],
        GroupChatRoleKey::BackendDesign => &[DESIGN_BACKEND_SLOT][..],
        GroupChatRoleKey::Reviewer | GroupChatRoleKey::Researcher => &[][..],
    };
    slots
        .iter()
        .map(|slot| DraftSlotKey((*slot).to_owned()))
        .collect()
}

/// 将群聊业务角色映射到 Adapter 协议角色。
pub fn adapter_role_for(role_key: GroupChatRoleKey) -> AdapterRole {
    if can_write_artifacts(role_key) {
        AdapterRole::Executor
    } else {
        AdapterRole::Reviewer
    }
}

/// 返回默认群聊阵容及其 provider 权限档位。
///
/// provider 绑定由创建会话或 UI 添加角色时完成；这里仅声明默认角色和执行权限。
pub fn default_lineup() -> Vec<(GroupChatRoleKey, ProviderPermissionMode)> {
    vec![
        (GroupChatRoleKey::Author, ProviderPermissionMode::Auto),
        (
            GroupChatRoleKey::Reviewer,
            ProviderPermissionMode::Supervised,
        ),
        (
            GroupChatRoleKey::Researcher,
            ProviderPermissionMode::Supervised,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_mapping_covers_write_permissions_slots_and_adapter_roles() {
        let expected = [
            (
                GroupChatRoleKey::Author,
                true,
                vec![ISSUE_FULL_SLOT, STORY_FULL_SLOT, DESIGN_SUMMARY_SLOT],
                AdapterRole::Executor,
            ),
            (
                GroupChatRoleKey::FrontendDesign,
                true,
                vec![DESIGN_FRONTEND_SLOT],
                AdapterRole::Executor,
            ),
            (
                GroupChatRoleKey::BackendDesign,
                true,
                vec![DESIGN_BACKEND_SLOT],
                AdapterRole::Executor,
            ),
            (
                GroupChatRoleKey::Reviewer,
                false,
                Vec::new(),
                AdapterRole::Reviewer,
            ),
            (
                GroupChatRoleKey::Researcher,
                false,
                Vec::new(),
                AdapterRole::Reviewer,
            ),
        ];

        for (role, can_write, slots, adapter_role) in expected {
            assert_eq!(can_write_artifacts(role), can_write, "{role:?}");
            assert_eq!(
                writable_slots(role),
                slots
                    .into_iter()
                    .map(str::to_owned)
                    .map(DraftSlotKey)
                    .collect::<Vec<_>>(),
                "{role:?}"
            );
            assert_eq!(adapter_role_for(role), adapter_role, "{role:?}");
        }
    }

    #[test]
    fn default_lineup_uses_auto_for_author_and_supervised_for_read_only_roles() {
        assert_eq!(
            default_lineup(),
            vec![
                (GroupChatRoleKey::Author, ProviderPermissionMode::Auto),
                (
                    GroupChatRoleKey::Reviewer,
                    ProviderPermissionMode::Supervised
                ),
                (
                    GroupChatRoleKey::Researcher,
                    ProviderPermissionMode::Supervised
                ),
            ]
        );
    }
}
