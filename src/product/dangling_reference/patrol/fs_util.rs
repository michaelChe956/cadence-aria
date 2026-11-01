//! 巡检共用只读工具（由 patrol.rs 按职责原样拆出：纯移动 + 必要的
//! pub(super) 放宽，零逻辑变化）。

use super::*;
/// 目录枚举；目录不可读 → 记一条不可判定并返回空表（不静默跳过）。
pub(super) fn read_dir_or_undetermined(
    ctx: &PatrolContext<'_>,
    dir: &Path,
    builder: &mut ReportBuilder,
) -> Vec<(PathBuf, bool)> {
    match read_dir_sorted(dir) {
        Ok(entries) => entries,
        Err(error) => {
            builder.push_undetermined(
                record_path_of(ctx.paths, dir),
                FIELD_PARTITION_LEVEL,
                format!("分区目录不可读：{error}"),
            );
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// 只读基础工具
// ---------------------------------------------------------------------------

/// 按 scope 解析逻辑代码库 store（`None` = 旧版别名仓，解析 legacy 根）。
pub(super) fn logical_codebase_store(
    paths: &ProductAppPaths,
    lc_id: &Option<String>,
) -> LogicalCodebaseStore {
    match lc_id {
        Some(lc_id) => LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone()),
        None => LogicalCodebaseStore::new(paths.clone()),
    }
}

/// 按 scope 解析聚合索引 store。
pub(super) fn aggregate_index_store(
    paths: &ProductAppPaths,
    lc_id: &Option<String>,
) -> AggregateIndexStore {
    match lc_id {
        Some(lc_id) => AggregateIndexStore::for_lc(paths.clone(), lc_id.clone()),
        None => AggregateIndexStore::new(paths.clone()),
    }
}

/// 状态根下的 project id 列表（读根失败上抛：连结构都不通时无巡检可言）。
pub(super) fn project_ids(paths: &ProductAppPaths) -> Result<Vec<String>, ProductStoreError> {
    Ok(child_directory_names(paths.projects_root())?
        .into_iter()
        .collect())
}

/// 单个 project 的 issue id 列表（读失败由调用方记为不可判定）。
pub(super) fn issue_ids(
    paths: &ProductAppPaths,
    project_id: &str,
) -> Result<Vec<String>, ProductStoreError> {
    Ok(
        child_directory_names(paths.project_root(project_id).join("issues"))?
            .into_iter()
            .collect(),
    )
}

/// 列出目录下的子目录名（目录不存在 → 空表；其余 IO 失败上抛）。
pub(super) fn child_directory_names(dir: PathBuf) -> Result<BTreeSet<String>, ProductStoreError> {
    let mut names = BTreeSet::new();
    for (path, is_dir) in read_dir_sorted(&dir)? {
        if !is_dir {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            names.insert(name.to_string());
        }
    }
    Ok(names)
}

/// 枚举目录下的子项（按路径升序，保证逐次可重现）。
///
/// 目录不存在按「空目录」处理；其余 IO 失败上抛（MUST NOT 被当成「空」）。
pub(super) fn read_dir_sorted(dir: &Path) -> Result<Vec<(PathBuf, bool)>, ProductStoreError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "read {}: {error}",
                dir.display()
            )));
        }
    };
    let mut items = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", dir.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} entry type: {error}",
                entry.path().display()
            ))
        })?;
        items.push((entry.path(), file_type.is_dir()));
    }
    items.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(items)
}

/// 路径是否是 `.json` 文件候选（目录由调用方区分）。
pub(super) fn is_json_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("json")
}

/// 存在性判定（stat）：IO 失败上抛为 `ProductStoreError::Io`，
/// MUST NOT 被当成「不存在」。
pub(super) fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    path.try_exists()
        .map_err(|error| ProductStoreError::Io(format!("stat {}: {error}", path.display())))
}

/// 记录路径（相对应用状态根 `paths.root()`）。
pub(super) fn record_path_of(paths: &ProductAppPaths, path: &Path) -> String {
    path.strip_prefix(paths.root())
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// 非空 id 的借用（空串 id 不构成引用）。
pub(super) fn non_empty(id: &Option<String>) -> Vec<String> {
    id.as_ref()
        .filter(|id| !id.is_empty())
        .map(|id| vec![id.clone()])
        .unwrap_or_default()
}
