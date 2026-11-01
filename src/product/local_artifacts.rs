//! 平台自产本地产物目录——**单一事实源**（P1-1）。
//!
//! 两处消费者必须同源，此前各自维护列表已经出现漂移（evidence_token 只写
//! `.aria/` 时代 git_workspace_service 的 commit 过滤只认 `.aria` 与
//! `.aria/coding-artifacts/`，`.aria/evidence-token`、`.codegraph/…` 等漏网）：
//!
//! - [`crate::product::logical_codebase::evidence_token::ensure_repo_exclude`]：
//!   向仓库公共 exclude（`<common-gitdir>/info/exclude`）幂等追加忽略行
//!   （[`exclude_lines`]）＋一段说明注释（[`notice_text`]）。
//! - [`crate::product::git_workspace_service`] 的
//!   `should_exclude_from_work_item_commit`：commit 前把误入暂存区的平台产物
//!   unstage——exclude 未就位或产物已被跟踪时的第二道防线，判定用
//!   [`is_platform_local_artifact_path`]。
//!
//! [`PLATFORM_LOCAL_ARTIFACT_DIRS`] 是唯一列表，exclude 行与路径判定全部由它
//! 推导，改列表只改这一处。

/// 平台/工具链自产本地产物目录（worktree 根相对、无尾斜杠）。
///
/// 逐项来源（都是平台或工具链在仓内生成的产物，用户不应对其提交）：
/// - `.aria`：Aria 注入/运行目录（会话令牌、web 端点、注入脚本）。
/// - `.codegraph`：CodeGraph 索引库目录（`codegraph init` 产物；全新 worktree
///   按项目规则初始化 CodeGraph 时同样落在 worktree 根）。
/// - `.verify`：本地验证产物目录（平台提供的验证流程产物；现场：执行 agent
///   在 worktree 内跑验证后留下的产物目录）。
/// - `.worktrees`：平台自建的 aria worktree 命名空间根
///   （`<repo>/.worktrees/aria-issues|aria-work-items|aria-pointer/…`）。
///
/// 这些目录的忽略语义是**整仓**的：主 checkout 与所有 linked worktree 里出现
/// 的同名目录都是平台产物。
pub(crate) const PLATFORM_LOCAL_ARTIFACT_DIRS: [&str; 4] =
    [".aria", ".codegraph", ".verify", ".worktrees"];

/// 追加进仓库公共 exclude 的忽略行：每目录一行、带尾斜杠（目录语义），由
/// [`PLATFORM_LOCAL_ARTIFACT_DIRS`] 推导，与本模块外的任何列表无关。
pub(crate) fn exclude_lines() -> impl Iterator<Item = String> {
    PLATFORM_LOCAL_ARTIFACT_DIRS
        .iter()
        .map(|dir| format!("{dir}/"))
}

/// 判定 git 输出（`status --porcelain` / `diff --cached --name-only` 等）里的
/// worktree 根相对路径是否落在平台本地产物目录内。
///
/// 与 exclude 行的 gitignore 语义保持一致：无前导斜杠/中斜杠的**目录**模式
/// 在**任意层级**匹配（`.aria/` 同样忽略 `src/.aria/x`），但尾斜杠模式只
/// 匹配**目录**——名为 `.aria` / `.codegraph` / `.verify` / `.worktrees` 的
/// **普通文件**不是平台产物目录，不命中（否则提交过滤会静默剔除用户文件，
/// 交付事实与实际提交不一致）。
///
/// 判定：路径位于同名目录**之下**才算命中——同名组件不是路径末段时命中；
/// 末段带尾斜杠（git status 的目录形态 `?? .aria/`）视为目录，同样命中。
pub(crate) fn is_platform_local_artifact_path(path: &str) -> bool {
    let is_directory_entry = path.ends_with('/');
    let trimmed = path.trim_end_matches('/');
    let mut components = trimmed.split('/');
    if !is_directory_entry {
        // 末段是文件路径成分：git 的 `.verify/` 不匹配同名普通文件。
        components.next_back();
    }
    components.any(|component| PLATFORM_LOCAL_ARTIFACT_DIRS.contains(&component))
}

/// 写入仓库公共 exclude 追加段头部的说明注释（单行、幂等只写一次）。
///
/// 用户打开 `info/exclude` 看到陌生忽略行时，这行说明产物来源与「可安全保留、
/// 请勿提交」的语义，避免误删或误提交。
pub(crate) fn notice_text() -> &'static str {
    "# aria: 以下本地产物忽略行由 Aria 平台维护（见 src/product/local_artifacts.rs），可安全保留，请勿提交这些目录"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclude_lines_derive_from_platform_dirs() {
        let lines: Vec<String> = exclude_lines().collect();
        assert_eq!(
            lines,
            [".aria/", ".codegraph/", ".verify/", ".worktrees/"],
            "exclude lines must derive from PLATFORM_LOCAL_ARTIFACT_DIRS"
        );
        assert_eq!(lines.len(), PLATFORM_LOCAL_ARTIFACT_DIRS.len());
    }

    /// 顶层与任意嵌套层级的平台目录都命中（与 gitignore 无前导斜杠模式的
    /// 任意层级匹配语义一致），含此前的漏网路径 `.aria/evidence-token`。
    #[test]
    fn platform_local_artifact_paths_are_detected_at_any_depth() {
        let hits = [
            // git 的目录形态带尾斜杠（`git status` 报 `?? .aria/`）；裸目录名
            // `.aria` 与同名普通文件不可区分，一律不命中（见 F3 用例）。
            ".aria/",
            ".aria/evidence-token",
            ".aria/coding-artifacts/attempt-1/report.md",
            ".codegraph/index.db",
            ".verify/report.txt",
            ".worktrees/aria-issues/issue_0001/scratch.txt",
            "src/web/.codegraph/index.db",
            "docs/.verify/output.txt",
        ];
        for path in hits {
            assert!(
                is_platform_local_artifact_path(path),
                "`{path}` must be a platform local artifact path"
            );
        }

        let misses = [
            "",
            "README.md",
            "aria/evidence-token",
            "src/aria/x",
            "package.worktrees.json",
            "src/.verifyx/y",
            "codegraph/index.db",
            "src/main.rs.pycx",
        ];
        for path in misses {
            assert!(
                !is_platform_local_artifact_path(path),
                "`{path}` must NOT be a platform local artifact path"
            );
        }
    }

    /// F3：同名**普通文件**不是平台产物目录——git 的 `.verify/`（尾斜杠）只
    /// 匹配目录，末段同名文件不命中；目录形态（末段带尾斜杠）与目录之下
    /// 的任意层级路径命中。
    #[test]
    fn platform_local_artifact_path_ignores_same_named_plain_files() {
        let hits = [
            ".aria/",
            ".aria/evidence-token",
            "foo/.verify/bar",
            ".verify/x",
            "foo/.verify/",
            "src/.codegraph/index.db",
        ];
        for path in hits {
            assert!(
                is_platform_local_artifact_path(path),
                "`{path}` must be a platform local artifact path"
            );
        }

        let misses = [
            ".verify",
            ".aria",
            ".codegraph",
            ".worktrees",
            "foo/.verify",
            "d/.verify",
        ];
        for path in misses {
            assert!(
                !is_platform_local_artifact_path(path),
                "`{path}` (plain file with a platform artifact name) must NOT be a platform local artifact path"
            );
        }
    }

    /// 说明注释必须是合法单行 exclude 注释（`#` 开头、无换行）。
    #[test]
    fn notice_text_is_single_comment_line() {
        let notice = notice_text();
        assert!(
            notice.starts_with('#'),
            "notice must be a comment: {notice}"
        );
        assert!(!notice.contains('\n'), "notice must be one line: {notice}");
        assert!(notice.trim().len() > 1, "notice must not be bare `#`");
    }
}
