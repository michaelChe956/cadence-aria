//! 分区三：reviewer 快照巡检（由 patrol.rs 按职责原样拆出：纯移动 + 必要的
//! pub(super) 放宽，零逻辑变化）。

use super::*;

// ---------------------------------------------------------------------------
// 分区三：reviewer 快照（`windows/<id>/manifest.json`）
// ---------------------------------------------------------------------------

/// reviewer 快照 manifest 的最小投影：巡检只需 `attempt_id`（其余字段由 ③ 层持有）。
#[derive(Debug, serde::Deserialize)]
struct SnapshotManifestProjection {
    attempt_id: String,
}

/// 遍历 reviewer 快照分区。
pub(super) fn scan_snapshot_partition(ctx: &PatrolContext<'_>, builder: &mut ReportBuilder) {
    let windows_root = ctx
        .paths
        .root()
        .join(REVIEWER_SNAPSHOT_STORE_DIR)
        .join(SNAPSHOT_WINDOWS_DIR);
    let entries = match read_dir_sorted(&windows_root) {
        Ok(entries) => entries,
        Err(error) => {
            builder.push_undetermined(
                record_path_of(ctx.paths, &windows_root),
                FIELD_PARTITION_LEVEL,
                format!("reviewer 快照分区不可读：{error}"),
            );
            return;
        }
    };
    for (window_dir, is_dir) in entries {
        if !is_dir {
            // `windows/` 顶层散落的 json 不是判据表内记录 → 未归类（只报不删）。
            if is_json_file(&window_dir) {
                builder.push_unclassified(record_path_of(ctx.paths, &window_dir));
            }
            continue;
        }
        let children = match read_dir_sorted(&window_dir) {
            Ok(children) => children,
            Err(error) => {
                builder.push_undetermined(
                    record_path_of(ctx.paths, &window_dir),
                    FIELD_PARTITION_LEVEL,
                    format!("reviewer 快照窗口目录不可读：{error}"),
                );
                continue;
            }
        };
        for (child, child_is_dir) in children {
            if child_is_dir || !is_json_file(&child) {
                continue;
            }
            let file_name = child
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if file_name != SNAPSHOT_MANIFEST_FILE {
                // 同目录其它记录（如 `restore-cursor.json`）不在判据表内 → 未归类。
                builder.push_unclassified(record_path_of(ctx.paths, &child));
                continue;
            }
            scan_snapshot_manifest(ctx, &child, builder);
        }
    }
}

/// 单个快照 manifest：`attempt_id` 走两路核对。
fn scan_snapshot_manifest(ctx: &PatrolContext<'_>, path: &Path, builder: &mut ReportBuilder) {
    let record_path = record_path_of(ctx.paths, path);
    let manifest: SnapshotManifestProjection = match read_json(path) {
        Ok(manifest) => manifest,
        Err(error) => {
            builder.push_undetermined(
                record_path,
                FIELD_RECORD_LEVEL,
                format!("记录解析失败：{error}"),
            );
            return;
        }
    };
    builder.mark_covered(DanglingRecordType::ReviewerSnapshotManifest);
    let Some(field) = field_path_for(
        DanglingRecordType::ReviewerSnapshotManifest,
        DanglingTargetKind::Attempt,
    ) else {
        return;
    };
    let reference = PatrolReference::attempt(manifest.attempt_id);
    let reference = PatrolReference { field, ..reference };
    check_reference(
        ctx,
        &record_path,
        DanglingRecordType::ReviewerSnapshotManifest,
        &reference,
        None,
        builder,
    );
}
