use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::product::json_store::ProductStoreError;

use super::types::{RoleInstance, RoomEvent};

/// 时间线中的持久化记录。
///
/// 序号由存储层赋值，而不是由事件本身提供，避免业务事件携带可被误用的序号。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TimelineEntry {
    seq: u64,
    event: RoomEvent,
}

/// 追加一条已经完成序列化的事件，并同步文件内容到稳定存储。
pub(crate) fn append_event(
    path: &Path,
    seq: u64,
    event: &RoomEvent,
) -> Result<(), ProductStoreError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }

    let entry = TimelineEntry {
        seq,
        event: event.clone(),
    };
    let mut bytes = serde_json::to_vec(&entry)
        .map_err(|error| ProductStoreError::Json(format!("serialize timeline event: {error}")))?;
    bytes.push(b'\n');

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| ProductStoreError::Io(format!("open {}: {error}", path.display())))?;
    file.write_all(&bytes)
        .map_err(|error| ProductStoreError::Io(format!("write {}: {error}", path.display())))?;
    file.flush()
        .map_err(|error| ProductStoreError::Io(format!("flush {}: {error}", path.display())))?;
    file.sync_all()
        .map_err(|error| ProductStoreError::Io(format!("sync {}: {error}", path.display())))?;
    Ok(())
}

/// 读取追加式时间线；不存在的文件代表尚未产生事件。
pub(crate) fn read_entries(path: &Path) -> Result<Vec<(u64, RoomEvent)>, ProductStoreError> {
    let records: Vec<TimelineEntry> = read_jsonl(path)?;
    Ok(records
        .into_iter()
        .map(|record| (record.seq, record.event))
        .collect())
}

fn read_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, ProductStoreError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?;
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).map_err(|error| {
                ProductStoreError::Json(format!("parse {}: {error}", path.display()))
            })
        })
        .collect()
}

/// 按时间线事件中的权威 cursor 更新角色快照。
///
/// injection_watermark 不是事件字段，重放时必须保守归零；它代表事件实际进入
/// prompt 的位置，不能从 AgentMessage/HeldEvent 的 seen cursor 推断。
pub(crate) fn replay_role_cursors(roles: &mut [RoleInstance], entries: &[(u64, RoomEvent)]) {
    // 注入水位只表示 prompt 实际消费到的位置，事件中没有该字段，因此恢复时不能
    // 用 cursor_after 猜测；统一归零后由后续上下文组装重新推进，宁可重复也不遗漏。
    for role in roles.iter_mut() {
        role.injection_watermark = 0;
    }
    apply_seen_cursors(roles, entries);
}

/// 只将事件中的 seen cursor 应用到快照，保留快照已有的注入水位。
///
/// 追加事件更新缓存时使用此函数；只有从崩溃恢复的 load 流程才会清零注入水位。
pub(crate) fn update_role_cursors(roles: &mut [RoleInstance], entries: &[(u64, RoomEvent)]) {
    apply_seen_cursors(roles, entries);
}

fn apply_seen_cursors(roles: &mut [RoleInstance], entries: &[(u64, RoomEvent)]) {
    for (_, event) in entries {
        let (role_id, cursor_after) = match event {
            RoomEvent::AgentMessage {
                role_instance_id,
                cursor_after,
                ..
            }
            | RoomEvent::HeldEvent {
                role_instance_id,
                cursor_after,
                ..
            } => (role_instance_id, *cursor_after),
            _ => continue,
        };
        if let Some(role) = roles.iter_mut().find(|role| role.id == *role_id) {
            role.seen_cursor = cursor_after;
        }
    }
}

pub(crate) fn next_seq(entries: &[(u64, RoomEvent)]) -> u64 {
    entries
        .iter()
        .map(|(seq, _)| *seq)
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}
