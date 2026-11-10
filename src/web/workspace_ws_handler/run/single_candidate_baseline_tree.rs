//! SC 作者轮的**plan 基线树事实装载面**（D5 事前 / REQ-WSC-06
//! 「基线树事实直供」）。
//!
//! 从 `single_candidate.rs` 拆出，独立承载：
//! - 成员目录清单的构造（仓库根顶层目录、排序、排除 `.git`）—— 与 D3 成员仓
//!   摘要同一来源；
//! - 基线树装载（`plan_preflight::plan_baseline_tree`，与 Evaluate 期预校验
//!   **同函数同源**，不新增第二套基线口径）与渲染
//!   （`author_baseline_tree_section` 的预算与失败语义）。
//!
//! 失败语义：基线树不可得（无仓 issue / 装载失败 / 空树）⇒ 该段**缺席**，
//! 作者轮**不被阻断**；既有 fail-closed 点仍在 Evaluate 阶段。

use std::path::Path;

use crate::product::lifecycle_store::LifecycleStore;

/// 装载并渲染作者 prompt 的 `baseline:` 段文本。
///
/// 基线树不可得时返回 `None`（该段缺席，作者轮不被阻断）。
pub(crate) fn baseline_tree_section(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    repository_path: &Path,
) -> Option<String> {
    let member_dirs = member_dirs(repository_path);
    crate::product::work_item_split_engine::prompts::author_baseline_tree_section(
        crate::product::workspace_engine::plan_preflight::plan_baseline_tree(
            lifecycle, project_id, issue_id,
        ),
        &member_dirs,
    )
}

/// 成员目录清单：仓库根顶层目录（排序、排除 `.git`）—— 与 D3 成员仓摘要同源。
fn member_dirs(repository_path: &Path) -> Vec<String> {
    let mut dirs: Vec<String> = std::fs::read_dir(repository_path)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter(|name| name != ".git")
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    dirs
}
