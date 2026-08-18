use std::time::Duration;

use chrono::{DateTime, Utc};
use thiserror::Error;

use super::roles::writable_slots;
use super::types::{ArtifactLine, Claim, DraftSlotKey, RoleInstance, RoomEvent};

/// 草稿槽认领的默认空闲超时：十分钟无产出即自动释放。
pub const DEFAULT_CLAIM_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// 草稿槽认领或释放时的业务错误。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClaimError {
    /// 角色无权操作指定草稿槽。
    #[error("角色 {role_id} 无权操作草稿槽 {slot_key}")]
    NotWritableSlot { role_id: String, slot_key: String },
    /// 指定草稿槽不在产物线中。
    #[error("未找到草稿槽 {slot_key}")]
    SlotNotFound { slot_key: String },
    /// 草稿槽已被其他角色排他认领。
    #[error("草稿槽 {slot_key} 已由角色 {holder_role_id} 认领")]
    SlotAlreadyClaimed {
        slot_key: String,
        holder_role_id: String,
    },
    /// 尝试释放尚未被认领的草稿槽。
    #[error("草稿槽 {slot_key} 尚未被认领")]
    SlotNotClaimed { slot_key: String },
    /// 只有当前认领者可以主动释放草稿槽。
    #[error("角色 {role_id} 不是草稿槽 {slot_key} 的认领者")]
    NotClaimHolder { role_id: String, slot_key: String },
}

/// 原子认领产物线中的一个草稿槽，并返回应追加到时间线的认领事件。
///
/// `&mut ArtifactLine` 使检查槽状态和写入认领在同一可变借用内完成；协调器对
/// 事件落盘串行化后，不会出现两个角色同时成功认领同一草稿槽的中间状态。
pub fn try_claim(
    line: &mut ArtifactLine,
    slot_key: &DraftSlotKey,
    role: &RoleInstance,
    now: DateTime<Utc>,
) -> Result<RoomEvent, ClaimError> {
    ensure_writable(slot_key, role)?;

    let slot = find_slot_mut(line, slot_key)?;
    if let Some(claim) = &slot.claim {
        return Err(ClaimError::SlotAlreadyClaimed {
            slot_key: slot_key.0.clone(),
            holder_role_id: claim.holder_role_id.clone(),
        });
    }

    slot.claim = Some(Claim {
        holder_role_id: role.id.clone(),
        claimed_at: now.to_rfc3339(),
    });
    Ok(claim_event(line, slot_key, &role.id, true))
}

/// 主动释放当前角色已认领的草稿槽，并返回应追加到时间线的释放事件。
pub fn release(
    line: &mut ArtifactLine,
    slot_key: &DraftSlotKey,
    role: &RoleInstance,
) -> Result<RoomEvent, ClaimError> {
    ensure_writable(slot_key, role)?;

    let slot = find_slot_mut(line, slot_key)?;
    let Some(claim) = &slot.claim else {
        return Err(ClaimError::SlotNotClaimed {
            slot_key: slot_key.0.clone(),
        });
    };
    if claim.holder_role_id != role.id {
        return Err(ClaimError::NotClaimHolder {
            role_id: role.id.clone(),
            slot_key: slot_key.0.clone(),
        });
    }

    slot.claim = None;
    Ok(claim_event(line, slot_key, &role.id, false))
}

/// 释放全部超时未产出的草稿槽，并返回应追加到时间线的释放事件。
///
/// 持久化的时间戳不合法时保守地保留认领，避免因损坏的数据而让另一角色覆写
/// 尚未确认失效的草稿；该异常会记录诊断日志。
pub fn release_expired(
    lines: &mut [ArtifactLine],
    now: DateTime<Utc>,
    timeout: Duration,
) -> Vec<RoomEvent> {
    let mut events = Vec::new();

    for line in lines {
        let line_kind = line.kind;
        for slot in &mut line.drafts {
            let Some(claim) = &slot.claim else {
                continue;
            };
            let Ok(claimed_at) = DateTime::parse_from_rfc3339(&claim.claimed_at) else {
                tracing::warn!(
                    slot_key = %slot.slot_key.0,
                    holder_role_id = %claim.holder_role_id,
                    claimed_at = %claim.claimed_at,
                    "草稿槽认领时间格式无效，保留认领"
                );
                continue;
            };
            let elapsed = now.signed_duration_since(claimed_at.with_timezone(&Utc));
            let Ok(elapsed) = elapsed.to_std() else {
                continue;
            };
            if elapsed < timeout {
                continue;
            }

            let holder_role_id = slot
                .claim
                .take()
                .expect("已确认草稿槽存在认领")
                .holder_role_id;
            events.push(RoomEvent::ClaimEvent {
                role_instance_id: holder_role_id,
                line: line_kind,
                slot_key: slot.slot_key.clone(),
                claimed: false,
            });
        }
    }

    events
}

fn ensure_writable(slot_key: &DraftSlotKey, role: &RoleInstance) -> Result<(), ClaimError> {
    if writable_slots(role.role_key).contains(slot_key) {
        Ok(())
    } else {
        Err(ClaimError::NotWritableSlot {
            role_id: role.id.clone(),
            slot_key: slot_key.0.clone(),
        })
    }
}

fn find_slot_mut<'a>(
    line: &'a mut ArtifactLine,
    slot_key: &DraftSlotKey,
) -> Result<&'a mut super::types::DraftSlot, ClaimError> {
    line.drafts
        .iter_mut()
        .find(|slot| slot.slot_key == *slot_key)
        .ok_or_else(|| ClaimError::SlotNotFound {
            slot_key: slot_key.0.clone(),
        })
}

fn claim_event(
    line: &ArtifactLine,
    slot_key: &DraftSlotKey,
    role_instance_id: &str,
    claimed: bool,
) -> RoomEvent {
    RoomEvent::ClaimEvent {
        role_instance_id: role_instance_id.into(),
        line: line.kind,
        slot_key: slot_key.clone(),
        claimed,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{ClaimError, DEFAULT_CLAIM_TIMEOUT, release, release_expired, try_claim};
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use crate::product::group_chat_engine::roles::{
        DESIGN_FRONTEND_SLOT, DESIGN_SUMMARY_SLOT, STORY_FULL_SLOT,
    };
    use crate::product::group_chat_engine::types::{
        ArtifactLine, ArtifactLineKind, DraftSlot, DraftSlotKey, GroupChatRoleKey, RoleInstance,
        RoomEvent,
    };
    use crate::product::models::ProviderName;

    fn role(id: &str, role_key: GroupChatRoleKey) -> RoleInstance {
        RoleInstance {
            id: id.into(),
            role_key,
            provider: ProviderName::Fake,
            display_name: id.into(),
            permission_mode: ProviderPermissionMode::Auto,
            seen_cursor: 0,
            injection_watermark: 0,
        }
    }

    fn line(kind: ArtifactLineKind, slots: &[&str]) -> ArtifactLine {
        ArtifactLine {
            kind,
            drafts: slots
                .iter()
                .map(|slot| DraftSlot {
                    slot_key: DraftSlotKey((*slot).into()),
                    current: None,
                    claim: None,
                })
                .collect(),
            finalized_versions: vec![],
            entity_id: None,
            bridge_session_id: None,
        }
    }

    fn at(minute: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 18, 12, minute, 0)
            .single()
            .expect("合法的测试时间")
    }

    #[test]
    fn 同槽互斥而不同槽可并行认领() {
        let mut story = line(ArtifactLineKind::StorySpec, &[STORY_FULL_SLOT]);
        let author_one = role("author-1", GroupChatRoleKey::Author);
        let author_two = role("author-2", GroupChatRoleKey::Author);
        let story_slot = DraftSlotKey(STORY_FULL_SLOT.into());

        assert_eq!(
            try_claim(&mut story, &story_slot, &author_one, at(0)),
            Ok(RoomEvent::ClaimEvent {
                role_instance_id: "author-1".into(),
                line: ArtifactLineKind::StorySpec,
                slot_key: story_slot.clone(),
                claimed: true,
            })
        );
        assert!(matches!(
            try_claim(&mut story, &story_slot, &author_two, at(1)),
            Err(ClaimError::SlotAlreadyClaimed {
                holder_role_id,
                ..
            }) if holder_role_id == "author-1"
        ));

        let mut design = line(
            ArtifactLineKind::DesignSpec,
            &[DESIGN_FRONTEND_SLOT, DESIGN_SUMMARY_SLOT],
        );
        let frontend = role("frontend-1", GroupChatRoleKey::FrontendDesign);
        let summary_slot = DraftSlotKey(DESIGN_SUMMARY_SLOT.into());
        let frontend_slot = DraftSlotKey(DESIGN_FRONTEND_SLOT.into());

        assert!(try_claim(&mut design, &summary_slot, &author_one, at(0)).is_ok());
        assert!(try_claim(&mut design, &frontend_slot, &frontend, at(0)).is_ok());
        assert!(design.drafts.iter().all(|slot| slot.claim.is_some()));
    }

    #[test]
    fn 只读角色不能认领草稿槽() {
        let mut story = line(ArtifactLineKind::StorySpec, &[STORY_FULL_SLOT]);
        let reviewer = role("reviewer-1", GroupChatRoleKey::Reviewer);

        assert!(matches!(
            try_claim(
                &mut story,
                &DraftSlotKey(STORY_FULL_SLOT.into()),
                &reviewer,
                at(0),
            ),
            Err(ClaimError::NotWritableSlot { .. })
        ));
        assert!(story.drafts[0].claim.is_none());
    }

    #[test]
    fn 超过默认超时会自动释放并产生释放事件() {
        let mut story = line(ArtifactLineKind::StorySpec, &[STORY_FULL_SLOT]);
        let author = role("author-1", GroupChatRoleKey::Author);
        let story_slot = DraftSlotKey(STORY_FULL_SLOT.into());
        try_claim(&mut story, &story_slot, &author, at(0)).expect("认领成功");

        assert!(
            release_expired(
                std::slice::from_mut(&mut story),
                at(9),
                DEFAULT_CLAIM_TIMEOUT,
            )
            .is_empty()
        );
        assert_eq!(
            release_expired(
                std::slice::from_mut(&mut story),
                at(10),
                DEFAULT_CLAIM_TIMEOUT,
            ),
            vec![RoomEvent::ClaimEvent {
                role_instance_id: "author-1".into(),
                line: ArtifactLineKind::StorySpec,
                slot_key: story_slot,
                claimed: false,
            }]
        );
        assert!(story.drafts[0].claim.is_none());
    }

    #[test]
    fn 释放只能由认领者执行且会产生释放事件() {
        let mut story = line(ArtifactLineKind::StorySpec, &[STORY_FULL_SLOT]);
        let author = role("author-1", GroupChatRoleKey::Author);
        let other_author = role("author-2", GroupChatRoleKey::Author);
        let story_slot = DraftSlotKey(STORY_FULL_SLOT.into());
        try_claim(&mut story, &story_slot, &author, at(0)).expect("认领成功");

        assert!(matches!(
            release(&mut story, &story_slot, &other_author),
            Err(ClaimError::NotClaimHolder { .. })
        ));
        assert_eq!(
            release(&mut story, &story_slot, &author),
            Ok(RoomEvent::ClaimEvent {
                role_instance_id: "author-1".into(),
                line: ArtifactLineKind::StorySpec,
                slot_key: story_slot,
                claimed: false,
            })
        );
        assert!(story.drafts[0].claim.is_none());
    }
}
