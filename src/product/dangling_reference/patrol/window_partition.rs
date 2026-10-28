//! 分区二：reviewer 窗口巡检（由 patrol.rs 按职责原样拆出：纯移动 + 必要的
//! pub(super) 放宽，零逻辑变化）。

use super::*;

// ---------------------------------------------------------------------------
// 分区二：reviewer 窗口（活跃 + 归档）
// ---------------------------------------------------------------------------

/// 一条已解析的窗口记录（含记录路径）。
pub(super) struct WindowRecordRef {
    record_path: String,
    record: ReviewerWindowRecord,
}

/// 遍历 reviewer 窗口分区（`windows/` 与 `window-archive/` 两路都扫）。
pub(super) fn scan_window_partition(
    ctx: &PatrolContext<'_>,
    builder: &mut ReportBuilder,
) -> Vec<WindowRecordRef> {
    let mut windows = Vec::new();
    for dir_name in [WINDOWS_DIR, WINDOW_ARCHIVE_DIR] {
        let dir = ctx
            .paths
            .root()
            .join(REVIEWER_WINDOW_STORE_DIR)
            .join(dir_name);
        let entries = match read_dir_sorted(&dir) {
            Ok(entries) => entries,
            Err(error) => {
                builder.push_undetermined(
                    record_path_of(ctx.paths, &dir),
                    FIELD_PARTITION_LEVEL,
                    format!("reviewer 窗口分区不可读：{error}"),
                );
                continue;
            }
        };
        for (path, is_dir) in entries {
            if is_dir || !is_json_file(&path) {
                continue;
            }
            let record_path = record_path_of(ctx.paths, &path);
            // MUST 逐条读：既有 `list_windows` 会静默跳过坏记录（REQ-DRG-23 禁止）。
            match read_json::<ReviewerWindowRecord>(&path) {
                Ok(record) => windows.push(WindowRecordRef {
                    record_path,
                    record,
                }),
                Err(error) => builder.push_undetermined(
                    record_path,
                    FIELD_RECORD_LEVEL,
                    format!("记录解析失败：{error}"),
                ),
            }
        }
    }
    windows.sort_by(|left, right| left.record_path.cmp(&right.record_path));
    windows
}

/// 按 attempt 聚合窗口簇的 L2 结论（r3 N-9：任一同簇窗口不满足即整簇不删）。
pub(super) fn build_attempt_clusters(
    ctx: &PatrolContext<'_>,
    windows: &[WindowRecordRef],
    builder: &mut ReportBuilder,
) -> BTreeMap<String, AttemptClusterVerdict> {
    let mut grouped: BTreeMap<&str, Vec<&WindowRecordRef>> = BTreeMap::new();
    for window in windows {
        grouped
            .entry(window.record.attempt_id.as_str())
            .or_default()
            .push(window);
    }
    let mut clusters = BTreeMap::new();
    for (attempt_id, group) in grouped {
        match ctx.attempt_existence(attempt_id) {
            // 目标仍在 / 无法判定：不建簇（也不产发现）。
            Ok(TargetExistence::Exists) => continue,
            Ok(TargetExistence::Unknown) => continue,
            Ok(TargetExistence::Missing) => {}
            Err(error) => {
                for window in &group {
                    builder.push_undetermined(
                        window.record_path.clone(),
                        FIELD_ATTEMPT,
                        format!("attempt 存在性读取失败：{error}"),
                    );
                }
                continue;
            }
        }
        let mut l2_ready = true;
        // 摘要取「首个不满足四条件的窗口」，全部满足时取首个窗口（避免用一扇
        // 合规窗口的摘要掩盖同簇另一扇窗口的不成立条件）。
        let mut failing_summary: Option<String> = None;
        let mut first_summary = String::new();
        for window in &group {
            match window_l2_verdict(ctx.paths, &window.record, true) {
                Ok(verdict) => {
                    let holds = verdict.all_hold();
                    if first_summary.is_empty() {
                        first_summary = verdict.summary();
                    }
                    if !holds && failing_summary.is_none() {
                        failing_summary = Some(verdict.summary());
                    }
                    l2_ready &= holds;
                }
                Err(error) => {
                    builder.push_undetermined(
                        window.record_path.clone(),
                        FIELD_ATTEMPT,
                        format!("窗口 L2 四条件核对读取失败：{error}"),
                    );
                    l2_ready = false;
                }
            }
        }
        let summary = failing_summary.unwrap_or(first_summary);
        clusters.insert(
            attempt_id.to_string(),
            AttemptClusterVerdict { l2_ready, summary },
        );
    }
    clusters
}

/// 逐条产出窗口记录上的发现（悬空 attempt → 一条发现 / 不可判定 → 一条记录）。
pub(super) fn emit_window_findings(
    ctx: &PatrolContext<'_>,
    windows: &[WindowRecordRef],
    builder: &mut ReportBuilder,
) {
    for window in windows {
        builder.mark_covered(DanglingRecordType::ReviewerWindow);
        let reference = PatrolReference::attempt(window.record.attempt_id.clone());
        check_reference(
            ctx,
            &window.record_path,
            DanglingRecordType::ReviewerWindow,
            &reference,
            None,
            builder,
        );
    }
}
