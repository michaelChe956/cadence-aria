//! 分区四：实体身份台账巡检（由 patrol.rs 按职责原样拆出：纯移动 + 必要的
//! pub(super) 放宽，零逻辑变化）。

use super::*;

// ---------------------------------------------------------------------------
// 分区四：实体身份台账（`state/entity-identity/sessions/*.json`）
// ---------------------------------------------------------------------------

/// 遍历实体身份台账分区。
pub(super) fn scan_entity_identity_partition(ctx: &PatrolContext<'_>, builder: &mut ReportBuilder) {
    let identity_root = ctx.paths.state_root().join(ENTITY_IDENTITY_DIR);
    // 分区根下的其它 json（非 sessions/）不在判据表内 → 未归类（只报不删）。
    match read_dir_sorted(&identity_root) {
        Ok(entries) => {
            for (path, is_dir) in entries {
                if !is_dir && is_json_file(&path) {
                    builder.push_unclassified(record_path_of(ctx.paths, &path));
                }
            }
        }
        Err(error) => builder.push_undetermined(
            record_path_of(ctx.paths, &identity_root),
            FIELD_PARTITION_LEVEL,
            format!("实体身份台账分区不可读：{error}"),
        ),
    }

    let sessions_root = identity_root.join(ENTITY_SESSIONS_DIR);
    let entries = match read_dir_sorted(&sessions_root) {
        Ok(entries) => entries,
        Err(error) => {
            builder.push_undetermined(
                record_path_of(ctx.paths, &sessions_root),
                FIELD_PARTITION_LEVEL,
                format!("实体身份台账 sessions 目录不可读：{error}"),
            );
            return;
        }
    };
    for (path, is_dir) in entries {
        if is_dir || !is_json_file(&path) {
            continue;
        }
        let record_path = record_path_of(ctx.paths, &path);
        let value: serde_json::Value = match read_json(&path) {
            Ok(value) => value,
            Err(error) => {
                builder.push_undetermined(
                    record_path,
                    FIELD_RECORD_LEVEL,
                    format!("记录解析失败：{error}"),
                );
                continue;
            }
        };
        builder.mark_covered(DanglingRecordType::EntityIdentitySessionBinding);
        let entity_kind = json_str_field(&value, "entity_kind");
        // 按 `entity_kind` 值分派到判据表命中的那一行；表外值不猜引用语义。
        let Some(target_kind) = entity_kind_target_kind(&entity_kind) else {
            continue;
        };
        let entity_id = json_str_field(&value, "entity_id");
        if entity_id.is_empty() {
            continue;
        }
        let Some(field) = field_path_for(
            DanglingRecordType::EntityIdentitySessionBinding,
            target_kind,
        ) else {
            continue;
        };
        let reference = PatrolReference::new(
            field,
            target_kind,
            entity_id,
            &json_str_field(&value, "project_id"),
            &json_str_field(&value, "issue_id"),
        );
        check_reference(
            ctx,
            &record_path,
            DanglingRecordType::EntityIdentitySessionBinding,
            &reference,
            None,
            builder,
        );
    }
}

/// 台账条目的 `entity_kind` 值 → 引用目标类型（判据表按值分派）。
fn entity_kind_target_kind(entity_kind: &str) -> Option<DanglingTargetKind> {
    match entity_kind {
        "story" => Some(DanglingTargetKind::StorySpec),
        "design" => Some(DanglingTargetKind::DesignSpec),
        "plan" => Some(DanglingTargetKind::Plan),
        _ => None,
    }
}

/// 通用 JSON 字符串字段（缺失 / 非字符串 → 空串）。
fn json_str_field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}
