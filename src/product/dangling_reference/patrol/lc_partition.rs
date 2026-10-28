//! 分区五：逻辑代码库 scope 巡检（由 patrol.rs 按职责原样拆出：纯移动 + 必要的
//! pub(super) 放宽，零逻辑变化）。

use super::*;

// ---------------------------------------------------------------------------
// 分区五：逻辑代码库 scope（新版子树 + 旧版别名，两路都扫）
// ---------------------------------------------------------------------------

/// 遍历单个 project 的逻辑代码库 scope 分区（REQ-DRG-08 两路都扫）。
pub(super) fn scan_logical_codebase_partition(
    ctx: &PatrolContext<'_>,
    project_id: &str,
    builder: &mut ReportBuilder,
) {
    let codebases_root = ctx.paths.logical_codebases_root(project_id);
    match read_dir_sorted(&codebases_root) {
        Ok(entries) => {
            for (path, is_dir) in entries {
                if !is_dir {
                    continue;
                }
                let Some(lc_id) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                scan_logical_codebase_scope(ctx, project_id, Some(lc_id.to_string()), builder);
            }
        }
        Err(error) => builder.push_undetermined(
            record_path_of(ctx.paths, &codebases_root),
            FIELD_PARTITION_LEVEL,
            format!("逻辑代码库分区不可读：{error}"),
        ),
    }

    // 旧版别名仓（`logical-codebase/`）可能与新版子树并存：两路都扫。
    let legacy_root = ctx.paths.logical_codebase_root(project_id);
    match path_exists(&legacy_root) {
        Ok(true) => scan_logical_codebase_scope(ctx, project_id, None, builder),
        Ok(false) => {}
        Err(error) => builder.push_undetermined(
            record_path_of(ctx.paths, &legacy_root),
            FIELD_PARTITION_LEVEL,
            format!("旧版逻辑代码库别名分区不可读：{error}"),
        ),
    }
}

/// 遍历单个 LC scope 下的记录文件。
fn scan_logical_codebase_scope(
    ctx: &PatrolContext<'_>,
    project_id: &str,
    lc_id: Option<String>,
    builder: &mut ReportBuilder,
) {
    let root = match lc_scope_root(ctx.paths, project_id, &lc_id) {
        Ok(root) => root,
        Err(error) => {
            builder.push_undetermined(
                format!(
                    "projects/{project_id}/{}",
                    lc_id.as_deref().unwrap_or("logical-codebase")
                ),
                FIELD_PARTITION_LEVEL,
                format!("逻辑代码库 scope 路径不可用：{error}"),
            );
            return;
        }
    };
    match path_exists(&root) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            builder.push_undetermined(
                record_path_of(ctx.paths, &root),
                FIELD_PARTITION_LEVEL,
                format!("逻辑代码库 scope 目录不可读：{error}"),
            );
            return;
        }
    }
    let scope = LcScopeContext::load(ctx.paths, project_id, lc_id, root.clone());
    let entries = match read_dir_sorted(&scope.root) {
        Ok(entries) => entries,
        Err(error) => {
            builder.push_undetermined(
                record_path_of(ctx.paths, &scope.root),
                FIELD_PARTITION_LEVEL,
                format!("逻辑代码库 scope 目录不可读：{error}"),
            );
            return;
        }
    };
    for (path, is_dir) in entries {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if is_dir {
            match name {
                LC_MEMBERS_DIR => scan_lc_members(ctx, &scope, &path, builder),
                LC_CHECKOUTS_DIR => scan_lc_checkouts(ctx, &scope, &path, builder),
                LC_AGGREGATE_DIR => scan_lc_aggregate_indexes(ctx, &scope, &path, builder),
                _ => {}
            }
        } else if is_json_file(&path) {
            if name == LC_MANIFEST_FILE {
                scan_lc_manifest(ctx, &scope, &path, builder);
            } else {
                // scope 根下其它记录（如 `record.json`）不在判据表内 → 未归类。
                builder.push_unclassified(record_path_of(ctx.paths, &path));
            }
        }
    }
}

/// LC manifest：`member_ids[]` → 逻辑成员；`active_aggregate_index_id` → 聚合索引。
fn scan_lc_manifest(
    ctx: &PatrolContext<'_>,
    scope: &LcScopeContext,
    path: &Path,
    builder: &mut ReportBuilder,
) {
    let record_path = record_path_of(ctx.paths, path);
    let manifest: LogicalCodebaseManifest = match read_json(path) {
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
    builder.mark_covered(DanglingRecordType::LogicalCodebaseMember);
    let mut references = Vec::new();
    for member_id in &manifest.member_ids {
        references.push(PatrolReference::new(
            "member_ids[]",
            DanglingTargetKind::LogicalMember,
            member_id.0.to_string(),
            &scope.project_id,
            "",
        ));
    }
    // 现场可能未写该字段：缺失即不产引用（MUST NOT 当成「目标不存在」）。
    if let Some(active) = manifest.active_aggregate_index_id.as_deref()
        && !active.is_empty()
    {
        references.push(PatrolReference::new(
            "active_aggregate_index_id",
            DanglingTargetKind::AggregateIndex,
            active,
            &scope.project_id,
            "",
        ));
    }
    for reference in &references {
        check_reference(
            ctx,
            &record_path,
            DanglingRecordType::LogicalCodebaseMember,
            reference,
            Some(scope),
            builder,
        );
    }
}

/// LC 成员记录：`checkout_ids[]` → checkout（成员自身 id 不是引用）。
fn scan_lc_members(
    ctx: &PatrolContext<'_>,
    scope: &LcScopeContext,
    dir: &Path,
    builder: &mut ReportBuilder,
) {
    for (path, is_dir) in read_dir_or_undetermined(ctx, dir, builder) {
        if is_dir || !is_json_file(&path) {
            continue;
        }
        let record_path = record_path_of(ctx.paths, &path);
        let member: CodebaseMemberRecord = match read_json(&path) {
            Ok(member) => member,
            Err(error) => {
                builder.push_undetermined(
                    record_path,
                    FIELD_RECORD_LEVEL,
                    format!("记录解析失败：{error}"),
                );
                continue;
            }
        };
        builder.mark_covered(DanglingRecordType::LogicalCodebaseMember);
        for checkout_id in &member.checkout_ids {
            let reference = PatrolReference::new(
                "checkout_ids[]",
                DanglingTargetKind::Checkout,
                checkout_id.0.to_string(),
                &scope.project_id,
                "",
            );
            check_reference(
                ctx,
                &record_path,
                DanglingRecordType::LogicalCodebaseMember,
                &reference,
                Some(scope),
                builder,
            );
        }
    }
}

/// LC checkout 记录：`logical_repository_id` → 逻辑成员（自身 checkout_id 不是引用）。
fn scan_lc_checkouts(
    ctx: &PatrolContext<'_>,
    scope: &LcScopeContext,
    dir: &Path,
    builder: &mut ReportBuilder,
) {
    for (path, is_dir) in read_dir_or_undetermined(ctx, dir, builder) {
        if is_dir || !is_json_file(&path) {
            continue;
        }
        let record_path = record_path_of(ctx.paths, &path);
        let checkout: RepositoryCheckoutRecord = match read_json(&path) {
            Ok(checkout) => checkout,
            Err(error) => {
                builder.push_undetermined(
                    record_path,
                    FIELD_RECORD_LEVEL,
                    format!("记录解析失败：{error}"),
                );
                continue;
            }
        };
        builder.mark_covered(DanglingRecordType::LogicalCodebaseMember);
        let reference = PatrolReference::new(
            "logical_repository_id",
            DanglingTargetKind::LogicalMember,
            checkout.logical_repository_id.0.to_string(),
            &scope.project_id,
            "",
        );
        check_reference(
            ctx,
            &record_path,
            DanglingRecordType::LogicalCodebaseMember,
            &reference,
            Some(scope),
            builder,
        );
    }
}

/// LC 聚合索引记录：`member_snapshots[].checkout_id` → checkout、
/// `member_snapshots[].logical_repository_id` → 逻辑成员。
fn scan_lc_aggregate_indexes(
    ctx: &PatrolContext<'_>,
    scope: &LcScopeContext,
    dir: &Path,
    builder: &mut ReportBuilder,
) {
    for (path, is_dir) in read_dir_or_undetermined(ctx, dir, builder) {
        if is_dir || !is_json_file(&path) {
            continue;
        }
        let record_path = record_path_of(ctx.paths, &path);
        let record: AggregateIndexRecord = match read_json(&path) {
            Ok(record) => record,
            Err(error) => {
                builder.push_undetermined(
                    record_path,
                    FIELD_RECORD_LEVEL,
                    format!("记录解析失败：{error}"),
                );
                continue;
            }
        };
        builder.mark_covered(DanglingRecordType::LogicalCodebaseMember);
        for snapshot in &record.member_snapshots {
            let member = PatrolReference::new(
                "member_snapshots[].logical_repository_id",
                DanglingTargetKind::LogicalMember,
                snapshot.logical_repository_id.0.to_string(),
                &scope.project_id,
                "",
            );
            check_reference(
                ctx,
                &record_path,
                DanglingRecordType::LogicalCodebaseMember,
                &member,
                Some(scope),
                builder,
            );
            let checkout = PatrolReference::new(
                "member_snapshots[].checkout_id",
                DanglingTargetKind::Checkout,
                snapshot.checkout_id.0.to_string(),
                &scope.project_id,
                "",
            );
            check_reference(
                ctx,
                &record_path,
                DanglingRecordType::LogicalCodebaseMember,
                &checkout,
                Some(scope),
                builder,
            );
        }
    }
}
