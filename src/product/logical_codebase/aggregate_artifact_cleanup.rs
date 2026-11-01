//! 删除时清理 aria 放在用户工作区的产物（owner 2026-10-29）。
//!
//! 清理对象**只有两类**：
//!
//! 1. `<canonical_root>/policy/<project_id>/<logical_codebase_id>/` —— 政策版本正文目录
//!    （`canonical_root` = `manifest.provider_context_root`；`logical_codebase_id`
//!    是 manifest 的 UUID 身份，与产品 record id 不同）。
//! 2. 每个成员仓根目录的 `.aria-pointer.md`（`POINTER_FILE_NAME`）—— 成员仓指针。
//!
//! 明确**不**清理：`AGENTS.md` / `CLAUDE.md` / `.claude/` / `.agents/` / `.mcp.json`
//! / `.gitignore` / `cadence/` / `openspec/` / `.omp/` / `.codex/` / `.kimi-code/`
//! / `.pi/` —— 这些是 Cadence-skills 产物，属于项目自己。
//!
//! 铁律：
//!
//! - **幂等**：缺失即跳过，重复清理不报错；
//! - **fail-soft**：失败只 `warn`（记入 [`AggregateArtifactCleanupReport::skipped`]），
//!   绝不阻断删除；
//! - **路径安全**：中间组件与终组件都不得是符号链接（`symlink_metadata` 不跟随），
//!   删除目标必须落在 `canonical_root` 之内。
//!
//! 软删 vs 硬删的分工（见 `LogicalCodebaseStore::delete_soft` 注释）：
//! 政策正文是 restore 的前提（restore 后 gateway 必须还能读到已发布正文），
//! 因此**软删不清政策正文**，只在不可恢复的硬删路径清——删项目，或
//! `LogicalCodebaseStore::purge`（单 LC 彻底删除）；软删只清成员仓指针
//! （指针可由下次指针发布重新生成，不参与 restore 的任何不变量）。

use std::path::{Path, PathBuf};

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::validate_relative_id;
use crate::product::logical_codebase::pointer_publication_coordinator::POINTER_FILE_NAME;
use crate::product::logical_codebase::store::{LogicalCodebaseStore, legacy_logical_codebase_id};
use crate::product::logical_codebase::types::{CheckoutAvailability, CheckoutKind};

/// 一次工作区产物清理的结果。清理永不失败，只记录做了什么、跳过了什么。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AggregateArtifactCleanupReport {
    /// 已删除的政策版本目录。
    pub removed_policy_dirs: Vec<PathBuf>,
    /// 已删除的成员仓指针文件。
    pub removed_pointer_files: Vec<PathBuf>,
    /// 跳过或失败的路径与原因（只 warn，不阻断删除）。
    pub skipped: Vec<(PathBuf, String)>,
}

impl AggregateArtifactCleanupReport {
    /// 本次清理没有任何动作（幂等重跑的判定依据）。
    pub fn is_noop(&self) -> bool {
        self.removed_policy_dirs.is_empty()
            && self.removed_pointer_files.is_empty()
            && self.skipped.is_empty()
    }

    /// 记录一次跳过/失败并 `warn`（清理永不阻断删除）。
    fn skip(&mut self, path: PathBuf, reason: impl Into<String>) {
        let reason = reason.into();
        tracing::warn!(path = %path.display(), %reason, "aggregate artifact cleanup skipped");
        self.skipped.push((path, reason));
    }

    fn merge(&mut self, other: Self) {
        self.removed_policy_dirs.extend(other.removed_policy_dirs);
        self.removed_pointer_files
            .extend(other.removed_pointer_files);
        self.skipped.extend(other.skipped);
    }
}

/// 清理单个 LC 的工作区产物：政策正文目录 + 成员仓指针文件。
///
/// 幂等：缺失即跳过；失败只 warn，绝不返回错误。
pub fn cleanup_logical_codebase_artifacts(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
) -> AggregateArtifactCleanupReport {
    let (report, _) = cleanup_logical_codebase(paths, project_id, logical_codebase_id, true);
    report
}

/// 只清理成员仓指针文件（软删路径专用：政策正文必须留给 restore）。
///
/// 幂等：缺失即跳过；失败只 warn，绝不返回错误。
pub fn cleanup_logical_codebase_pointer_files(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
) -> AggregateArtifactCleanupReport {
    let (report, _) = cleanup_logical_codebase(paths, project_id, logical_codebase_id, false);
    report
}

/// 清理项目下**所有** LC 的工作区产物（硬删项目路径）。
///
/// 覆盖活着的 LC 与已软删（墓碑）的 LC —— 项目删除后两者的 app 数据都会消失，
/// 留在工作区的产物必须一并收掉。最后对每个已知 `canonical_root` 扫尾：
/// `policy/<project_id>/` 整目录删除（含孤儿/墓碑 LC 的政策目录）。
///
/// 幂等：缺失即跳过；失败只 warn，绝不返回错误。
pub fn cleanup_project_artifacts(
    paths: &ProductAppPaths,
    project_id: &str,
) -> AggregateArtifactCleanupReport {
    let mut report = AggregateArtifactCleanupReport::default();
    let mut canonical_roots: Vec<PathBuf> = Vec::new();
    for logical_codebase_id in project_logical_codebase_ids(paths, project_id) {
        let (single, canonical_root) =
            cleanup_logical_codebase(paths, project_id, &logical_codebase_id, true);
        if let Some(root) = canonical_root
            && !canonical_roots.contains(&root)
        {
            canonical_roots.push(root);
        }
        report.merge(single);
    }
    // 扫尾：本项目所有 LC 都被删除 → 各 canonical root 下的 `policy/<project_id>/`
    // 整目录（含墓碑 LC / 孤儿 LC 的政策正文）一并清掉。
    for canonical_root in canonical_roots {
        remove_policy_project_dir(&canonical_root, project_id, &mut report);
    }
    report
}

/// 单个 LC 的清理；`include_policy_body` 为假时只清成员仓指针。
///
/// 返回 `(报告, 参与清理的 canonical_root)`。
fn cleanup_logical_codebase(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
    include_policy_body: bool,
) -> (AggregateArtifactCleanupReport, Option<PathBuf>) {
    let mut report = AggregateArtifactCleanupReport::default();
    let store = LogicalCodebaseStore::for_lc(paths.clone(), logical_codebase_id);
    let mut canonical_root = None;
    if include_policy_body {
        canonical_root = remove_policy_body(&store, project_id, logical_codebase_id, &mut report);
    }
    remove_member_pointer_files(&store, project_id, logical_codebase_id, &mut report);
    (report, canonical_root)
}

/// 删除该 LC 的政策版本目录；返回参与删除的 `canonical_root`。
fn remove_policy_body(
    store: &LogicalCodebaseStore,
    project_id: &str,
    logical_codebase_id: &str,
    report: &mut AggregateArtifactCleanupReport,
) -> Option<PathBuf> {
    let manifest = match store.load_lc_manifest(project_id, logical_codebase_id) {
        Ok(Some(manifest)) => manifest,
        // 没有 manifest：政策目录的 LC 身份不可知，交由项目级扫尾处理。
        Ok(None) => return None,
        Err(error) => {
            report.skip(
                store
                    .paths()
                    .logical_codebase_record_root(project_id, logical_codebase_id),
                format!("load manifest: {error}"),
            );
            return None;
        }
    };
    let canonical_root = manifest.provider_context_root;
    let lc_identity = manifest.logical_codebase_id.to_string();
    remove_policy_dir(&canonical_root, project_id, &lc_identity, report);
    Some(canonical_root)
}

/// 删除 `<canonical_root>/policy/<project_id>/<lc_identity>/`。
///
/// 中间组件（`policy/`、`<project_id>/`）与终组件都不得是符号链接：不跟随链接，
/// 也不越过 `canonical_root`。缺失即跳过（幂等）。
fn remove_policy_dir(
    canonical_root: &Path,
    project_id: &str,
    lc_identity: &str,
    report: &mut AggregateArtifactCleanupReport,
) {
    if let Err(error) = validate_relative_id(project_id) {
        report.skip(
            canonical_root.join("policy"),
            format!("invalid project id: {error}"),
        );
        return;
    }
    if let Err(error) = validate_relative_id(lc_identity) {
        report.skip(
            canonical_root.join("policy"),
            format!("invalid logical codebase identity: {error}"),
        );
        return;
    }
    let policy_root = canonical_root.join("policy");
    let project_dir = policy_root.join(project_id);
    for component in [&policy_root, &project_dir] {
        match std::fs::symlink_metadata(component) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    report.skip(
                        component.clone(),
                        "policy path component is a symlink; skipped to stay inside the canonical root",
                    );
                    return;
                }
                if !metadata.is_dir() {
                    report.skip(
                        component.clone(),
                        "policy path component is not a directory; skipped",
                    );
                    return;
                }
            }
            // 缺失 = 没有政策正文可清（幂等）。
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                report.skip(component.clone(), format!("stat: {error}"));
                return;
            }
        }
    }
    let target = project_dir.join(lc_identity);
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                report.skip(
                    target,
                    "policy directory is a symlink; skipped to stay inside the canonical root",
                );
                return;
            }
            let outcome = if metadata.is_dir() {
                // `remove_dir_all` 不跟随目录内的符号链接：链接只删链接本身。
                std::fs::remove_dir_all(&target)
            } else {
                std::fs::remove_file(&target)
            };
            match outcome {
                Ok(()) => report.removed_policy_dirs.push(target),
                Err(error) => report.skip(target, format!("remove: {error}")),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => report.skip(target, format!("stat: {error}")),
    }
    // 空的项目政策目录一并收掉；同项目仍有其它 LC 时目录非空，忽略即可。
    let _ = std::fs::remove_dir(&project_dir);
}

/// 删除 `<canonical_root>/policy/<project_id>/` 整个目录（项目删除后的扫尾）。
fn remove_policy_project_dir(
    canonical_root: &Path,
    project_id: &str,
    report: &mut AggregateArtifactCleanupReport,
) {
    if let Err(error) = validate_relative_id(project_id) {
        report.skip(
            canonical_root.join("policy"),
            format!("invalid project id: {error}"),
        );
        return;
    }
    let policy_root = canonical_root.join("policy");
    match std::fs::symlink_metadata(&policy_root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                report.skip(
                    policy_root,
                    "policy root is a symlink; skipped to stay inside the canonical root",
                );
                return;
            }
            if !metadata.is_dir() {
                report.skip(policy_root, "policy root is not a directory; skipped");
                return;
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            report.skip(policy_root, format!("stat: {error}"));
            return;
        }
    }
    let target = policy_root.join(project_id);
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                report.skip(
                    target,
                    "policy project directory is a symlink; skipped to stay inside the canonical root",
                );
                return;
            }
            let outcome = if metadata.is_dir() {
                std::fs::remove_dir_all(&target)
            } else {
                std::fs::remove_file(&target)
            };
            match outcome {
                Ok(()) => report.removed_policy_dirs.push(target),
                Err(error) => report.skip(target, format!("remove: {error}")),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => report.skip(target, format!("stat: {error}")),
    }
}

/// 删除该 LC 每个成员仓主 checkout 根目录下的 `.aria-pointer.md`。
fn remove_member_pointer_files(
    store: &LogicalCodebaseStore,
    project_id: &str,
    logical_codebase_id: &str,
    report: &mut AggregateArtifactCleanupReport,
) {
    let members = match store.list_lc_members(project_id, logical_codebase_id) {
        Ok(members) => members,
        Err(error) => {
            report.skip(
                store
                    .paths()
                    .logical_codebase_record_root(project_id, logical_codebase_id),
                format!("list members: {error}"),
            );
            return;
        }
    };
    for member in members {
        for checkout_id in &member.checkout_ids {
            let checkout = match store.load_checkout(project_id, *checkout_id) {
                Ok(Some(checkout)) => checkout,
                Ok(None) => continue,
                Err(error) => {
                    report.skip(
                        store
                            .paths()
                            .logical_codebase_record_root(project_id, logical_codebase_id),
                        format!("load checkout {checkout_id:?}: {error}"),
                    );
                    continue;
                }
            };
            // 指针只发布在成员仓的主 checkout 根目录（与指针发布协调器同口径）。
            if checkout.kind != CheckoutKind::Main
                || checkout.availability != CheckoutAvailability::Available
            {
                continue;
            }
            remove_pointer_file(&checkout.canonical_path.join(POINTER_FILE_NAME), report);
        }
    }
}

/// 删除一个成员仓指针文件；缺失即跳过，目录形态一律不删。
fn remove_pointer_file(pointer: &Path, report: &mut AggregateArtifactCleanupReport) {
    match std::fs::symlink_metadata(pointer) {
        Ok(metadata) => {
            if metadata.is_dir() {
                report.skip(
                    pointer.to_path_buf(),
                    "pointer path is a directory; skipped",
                );
                return;
            }
            match std::fs::remove_file(pointer) {
                // `remove_file` 删的是链接本身，不跟随链接指向的目标。
                Ok(()) => report.removed_pointer_files.push(pointer.to_path_buf()),
                Err(error) => report.skip(pointer.to_path_buf(), format!("remove: {error}")),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => report.skip(pointer.to_path_buf(), format!("stat: {error}")),
    }
}

/// 枚举项目下的全部 LC record id（含已软删的墓碑 LC 与 legacy 别名 LC）。
fn project_logical_codebase_ids(paths: &ProductAppPaths, project_id: &str) -> Vec<String> {
    let mut ids = Vec::new();
    if let Ok(entries) = std::fs::read_dir(paths.logical_codebases_root(project_id)) {
        for entry in entries.flatten() {
            let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            if !is_dir {
                continue;
            }
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            // 锁文件等隐藏条目不是 LC。
            if name.starts_with('.') {
                continue;
            }
            ids.push(name.to_string());
        }
    }
    // legacy 别名 LC 的子树在 legacy 根下，不在 `logical-codebases/` 里。
    if paths
        .logical_codebase_root(project_id)
        .join("manifest.json")
        .exists()
    {
        ids.push(legacy_logical_codebase_id(project_id));
    }
    ids.sort();
    ids.dedup();
    ids
}

#[cfg(test)]
include!("aggregate_artifact_cleanup_tests.inc.rs");
