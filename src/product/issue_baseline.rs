//! issue 基准分支解析（per-issue-base-branch，REQ-PIB-01/02/03 共用语义）。
//!
//! 三面同源的单一解析点：创建校验（REST）、author 上下文基线（T2）、coding fork
//! 与 C1 核对（T3）全部经 `resolve_effective_base_branch` 取同一结果：
//! - `Some(branch)`：显式锁定值——本地 `refs/heads/<branch>` 存在性校验
//!   （`show-ref --verify`，精确命名空间，非 DWIM；tags/远端引用/SHA 不通过），
//!   不可解析即 fail-closed（不回退、不猜替代分支）；
//! - `None`：默认链 `main → master → 无默认`（存量记录零迁移共用同一链）。
//!
//! 列表口径（REQ-PIB-01）：仅本地 `refs/heads/*`（`for-each-ref` 稳定排序），
//! 不隐式 fetch、不混入远端跟踪引用/tags——「可选=可校验=可消费」三面同集
//! （pib-oracle 裁决三）。全部为参数化 argv 的只读 git 查询，不触工作区。

use std::path::Path;

/// 基线解析失败（fail-closed 诊断，不猜替代分支）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueBaselineError {
    /// 显式锁定的分支在本地仓库不可解析（被删/改名/从未存在）。
    BranchMissing { branch: String },
    /// 存量 `None` 且仓库无 main/master 本地分支：无默认值可推断。
    NoDefaultBranch,
    /// 仓库路径不可用（非 git 仓库 / git 执行失败）。
    GitUnavailable { detail: String },
}

impl IssueBaselineError {
    /// 面向用户的确定性诊断（区分「分支不存在」与「无法推断默认基线」）。
    pub fn diagnosis(&self) -> String {
        match self {
            IssueBaselineError::BranchMissing { branch } => format!(
                "基准分支不存在：{branch}（issue 基线已锁定，分支被删或改名；不回退不猜替代分支，请恢复分支或新建 Issue）"
            ),
            IssueBaselineError::NoDefaultBranch => {
                "无法推断默认基准分支：仓库无 main/master 本地分支，且 Issue 未显式设置 base_branch（存量记录；请显式选择基准分支创建新 Issue）".to_string()
            }
            IssueBaselineError::GitUnavailable { detail } => {
                format!("基准分支解析所需 git 仓库不可用：{detail}")
            }
        }
    }
}

impl std::fmt::Display for IssueBaselineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.diagnosis())
    }
}

impl std::error::Error for IssueBaselineError {}

fn run_git(repo_path: &Path, args: &[&str]) -> Result<std::process::Output, IssueBaselineError> {
    std::process::Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| IssueBaselineError::GitUnavailable {
            detail: format!("git {:?} in {}: {error}", args, repo_path.display()),
        })
}

/// 本地分支存在性：`refs/heads/<branch>` 精确校验。
/// git 自身故障（非「引用不存在」）按 `GitUnavailable` 上浮，不吞为 false。
pub fn local_branch_exists(repo_path: &Path, branch: &str) -> Result<bool, IssueBaselineError> {
    let reference = format!("refs/heads/{branch}");
    let output = run_git(
        repo_path,
        &["show-ref", "--verify", "--quiet", "--", &reference],
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        code => Err(IssueBaselineError::GitUnavailable {
            detail: format!(
                "git show-ref {reference} exited with {code:?}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        }),
    }
}

/// 默认链：`main → master → 无`。
pub fn default_local_branch(repo_path: &Path) -> Result<Option<String>, IssueBaselineError> {
    for candidate in ["main", "master"] {
        if local_branch_exists(repo_path, candidate)? {
            return Ok(Some(candidate.to_string()));
        }
    }
    Ok(None)
}

/// 有效基线解析（三面同源的唯一入口）。
///
/// - `Some(branch)`（非空白）：存在性校验通过 → `Ok(branch)`；
///   不可解析 → `Err(BranchMissing)`（fail-closed，不进默认链）；
/// - `None`/空白：默认链解析；皆无 → `Err(NoDefaultBranch)`。
pub fn resolve_effective_base_branch(
    repo_path: &Path,
    base_branch: Option<&str>,
) -> Result<String, IssueBaselineError> {
    match base_branch.map(str::trim).filter(|value| !value.is_empty()) {
        Some(branch) => {
            if local_branch_exists(repo_path, branch)? {
                Ok(branch.to_string())
            } else {
                Err(IssueBaselineError::BranchMissing {
                    branch: branch.to_string(),
                })
            }
        }
        None => default_local_branch(repo_path)?.ok_or(IssueBaselineError::NoDefaultBranch),
    }
}

/// 全部本地分支（`refs/heads/*`，`refname` 稳定排序，去 `refs/heads/` 前缀）。
/// 不隐式 fetch、不列远端跟踪引用与 tags（pib-oracle 裁决三）。
pub fn list_local_branches(repo_path: &Path) -> Result<Vec<String>, IssueBaselineError> {
    let output = run_git(
        repo_path,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "--sort=refname",
            "refs/heads/",
        ],
    )?;
    if !output.status.success() {
        return Err(IssueBaselineError::GitUnavailable {
            detail: format!(
                "git for-each-ref refs/heads/ failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| line.strip_prefix("refs/heads/"))
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn git_repo_with_branches(initial_branch: &str, extra_branches: &[&str]) -> TempDir {
        let dir = TempDir::new().expect("temp repo");
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .stdin(std::process::Stdio::null())
                .status()
                .expect("git fixture command");
            assert!(status.success(), "git {args:?}");
        };
        run(&["init", "-b", initial_branch]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test User"]);
        std::fs::write(dir.path().join("README.md"), "base\n").expect("write readme");
        run(&["add", "README.md"]);
        run(&["commit", "-m", "base"]);
        for branch in extra_branches {
            run(&["branch", branch]);
        }
        dir
    }

    #[test]
    fn resolve_explicit_branch_requires_local_existence() {
        let repo = git_repo_with_branches("main", &["feature/x"]);
        assert_eq!(
            resolve_effective_base_branch(repo.path(), Some("feature/x")).unwrap(),
            "feature/x"
        );
        let error = resolve_effective_base_branch(repo.path(), Some("gone")).unwrap_err();
        assert_eq!(
            error,
            IssueBaselineError::BranchMissing {
                branch: "gone".to_string()
            }
        );
        assert!(error.diagnosis().contains("gone"));
    }

    #[test]
    fn resolve_explicit_branch_rejects_non_heads_references() {
        let repo = git_repo_with_branches("main", &[]);
        // tags 与远端跟踪引用不是可选基线（refs/heads 精确命名空间）。
        std::process::Command::new("git")
            .args(["tag", "v1"])
            .current_dir(repo.path())
            .status()
            .expect("tag");
        std::process::Command::new("git")
            .args([
                "update-ref",
                "refs/remotes/origin/remote-only",
                "refs/heads/main",
            ])
            .current_dir(repo.path())
            .status()
            .expect("remote ref");
        assert!(resolve_effective_base_branch(repo.path(), Some("v1")).is_err());
        assert!(resolve_effective_base_branch(repo.path(), Some("origin/remote-only")).is_err());
        // 分离头指针对象 id 同样不可作为分支基线。
        let head = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo.path())
            .output()
            .expect("rev-parse");
        let sha = String::from_utf8_lossy(&head.stdout).trim().to_string();
        assert!(resolve_effective_base_branch(repo.path(), Some(&sha)).is_err());
    }

    #[test]
    fn resolve_none_follows_main_then_master_then_fail_closed() {
        let main_repo = git_repo_with_branches("main", &["master"]);
        assert_eq!(
            resolve_effective_base_branch(main_repo.path(), None).unwrap(),
            "main"
        );

        let master_repo = git_repo_with_branches("master", &[]);
        assert_eq!(
            resolve_effective_base_branch(master_repo.path(), None).unwrap(),
            "master"
        );

        let trunk_repo = git_repo_with_branches("trunk", &["release/1.0"]);
        let error = resolve_effective_base_branch(trunk_repo.path(), None).unwrap_err();
        assert_eq!(error, IssueBaselineError::NoDefaultBranch);
        assert!(error.diagnosis().contains("无法推断默认基准分支"));
    }

    #[test]
    fn resolve_blank_explicit_value_falls_back_to_default_chain() {
        let repo = git_repo_with_branches("main", &[]);
        assert_eq!(
            resolve_effective_base_branch(repo.path(), Some("   ")).unwrap(),
            "main"
        );
    }

    #[test]
    fn list_local_branches_is_heads_only_and_sorted() {
        let repo = git_repo_with_branches("main", &["feature/x", "release/1.0"]);
        std::process::Command::new("git")
            .args([
                "update-ref",
                "refs/remotes/origin/remote-only",
                "refs/heads/main",
            ])
            .current_dir(repo.path())
            .status()
            .expect("remote ref");
        std::process::Command::new("git")
            .args(["tag", "v1"])
            .current_dir(repo.path())
            .status()
            .expect("tag");

        let branches = list_local_branches(repo.path()).unwrap();
        assert_eq!(
            branches,
            vec![
                "feature/x".to_string(),
                "main".to_string(),
                "release/1.0".to_string()
            ]
        );
    }

    #[test]
    fn resolve_on_non_git_directory_reports_git_unavailable() {
        let dir = TempDir::new().expect("plain dir");
        let error = resolve_effective_base_branch(dir.path(), Some("main")).unwrap_err();
        assert!(matches!(error, IssueBaselineError::GitUnavailable { .. }));
        assert!(error.diagnosis().contains("git"));
    }
}
