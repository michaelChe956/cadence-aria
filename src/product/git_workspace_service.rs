use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::process_manager::ManagedProcessChild;
use crate::product::coding_models::{PushStatus, RemoteKind};

mod reconcile;
mod test_pause;

pub use test_pause::{GitCommandPause, pause_next_git_command_after_exit};

const SAFE_WORKTREE_PREFIXES: &[&str] = &["aria-work-items", "aria-issues", "aria-pointer"];
const SAFE_BRANCH_PREFIXES: &[&str] = &["aria/work-items/", "aria/issues/", "aria-pointer/"];

#[derive(Debug, thiserror::Error)]
pub enum GitWorkspaceError {
    #[error("git_workspace_io: {0}")]
    Io(String),
    #[error("git_workspace_command_failed: git {args} in {cwd}: {stderr}")]
    CommandFailed {
        args: String,
        cwd: String,
        stderr: String,
    },
    #[error("git_workspace_timeout: git {args} in {cwd}")]
    Timeout { args: String, cwd: String },
    #[error("git_workspace_cancelled: git {args} in {cwd}")]
    Cancelled { args: String, cwd: String },
    #[error("git_workspace_unsafe_path: {0}")]
    UnsafePath(String),
    #[error("git_workspace_parse: {0}")]
    Parse(String),
    #[error("git_push_indeterminate: {remote}/{branch} at {commit_sha}")]
    PushIndeterminate {
        remote: String,
        branch: String,
        commit_sha: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    pub code: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitResult {
    pub commit_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushResult {
    pub status: PushStatus,
    pub remote: String,
    pub branch: String,
    pub stderr: Option<String>,
    pub remote_rejected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFileStat {
    pub path: String,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffStat {
    pub files: Vec<DiffFileStat>,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone)]
pub struct GitWorkspaceService {
    command_timeout: Duration,
    cancellation: CancellationToken,
}

impl GitWorkspaceService {
    pub fn new() -> Self {
        Self {
            command_timeout: Duration::from_secs(30),
            cancellation: CancellationToken::new(),
        }
    }

    pub(crate) fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub async fn create_branch(
        &self,
        repo_path: &Path,
        branch_name: &str,
        base_branch: &str,
    ) -> Result<(), GitWorkspaceError> {
        self.ensure_git_repo(repo_path).await?;
        ensure_safe_aria_branch_name(branch_name)?;
        let ref_name = format!("refs/heads/{branch_name}");
        let exists = self
            .run_git_allow_failure(repo_path, &["show-ref", "--verify", "--quiet", &ref_name])
            .await?;
        if exists.status_success {
            return Ok(());
        }
        self.run_git(repo_path, &["branch", branch_name, base_branch])
            .await
            .map(|_| ())
    }

    pub async fn create_worktree(
        &self,
        repo_path: &Path,
        branch_name: &str,
        worktree_path: &Path,
    ) -> Result<(), GitWorkspaceError> {
        self.ensure_git_repo(repo_path).await?;
        ensure_safe_aria_branch_name(branch_name)?;
        ensure_safe_worktree_path(repo_path, worktree_path)?;

        if let Some(existing_branch) = self.find_worktree_branch(repo_path, worktree_path).await? {
            if existing_branch == branch_name {
                return Ok(());
            }
            return Err(GitWorkspaceError::UnsafePath(format!(
                "worktree {} already bound to branch {} not {}",
                worktree_path.display(),
                existing_branch,
                branch_name
            )));
        }

        if let Some(parent) = worktree_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                GitWorkspaceError::Io(format!("create {}: {error}", parent.display()))
            })?;
        }
        let worktree = worktree_path.to_string_lossy().to_string();
        self.run_git(repo_path, &["worktree", "add", &worktree, branch_name])
            .await
            .map(|_| ())
    }

    pub async fn remove_worktree(
        &self,
        repo_path: &Path,
        worktree_path: &Path,
    ) -> Result<(), GitWorkspaceError> {
        self.ensure_git_repo(repo_path).await?;
        ensure_safe_worktree_path(repo_path, worktree_path)?;
        if !worktree_path.exists() {
            return Ok(());
        }
        let worktree = worktree_path.to_string_lossy().to_string();
        self.run_git(repo_path, &["worktree", "remove", "--force", &worktree])
            .await
            .map(|_| ())
    }

    pub async fn prune_worktrees(&self, repo_path: &Path) -> Result<(), GitWorkspaceError> {
        self.ensure_git_repo(repo_path).await?;
        self.run_git(repo_path, &["worktree", "prune"])
            .await
            .map(|_| ())
    }

    pub async fn delete_local_branch(
        &self,
        repo_path: &Path,
        branch_name: &str,
    ) -> Result<(), GitWorkspaceError> {
        self.ensure_git_repo(repo_path).await?;
        ensure_safe_aria_branch_name(branch_name)?;
        let ref_name = format!("refs/heads/{branch_name}");
        let exists = self
            .run_git_allow_failure(repo_path, &["show-ref", "--verify", "--quiet", &ref_name])
            .await?;
        if !exists.status_success {
            return Ok(());
        }
        self.run_git(repo_path, &["branch", "-D", branch_name])
            .await
            .map(|_| ())
    }

    pub async fn delete_remote_branch(
        &self,
        repo_path: &Path,
        remote: &str,
        branch_name: &str,
    ) -> Result<(), GitWorkspaceError> {
        self.ensure_git_repo(repo_path).await?;
        ensure_safe_aria_branch_name(branch_name)?;
        let output = self
            .run_git_allow_failure(repo_path, &["push", remote, "--delete", branch_name])
            .await?;
        if output.status_success {
            return Ok(());
        }
        // 远端分支本就不存在：git 输出含 "remote ref does not exist" 且 exit != 0，
        // 与 delete_local_branch 的 show-ref 幂等语义对齐，视为成功。
        if remote_delete_output_is_missing_ref(&output.stderr) {
            return Ok(());
        }
        Err(GitWorkspaceError::CommandFailed {
            args: format!("push {remote} --delete {branch_name}"),
            cwd: repo_path.display().to_string(),
            stderr: output.stderr,
        })
    }

    pub async fn git_status(
        &self,
        worktree_path: &Path,
    ) -> Result<Vec<FileStatus>, GitWorkspaceError> {
        self.ensure_git_repo(worktree_path).await?;
        let output = self
            .run_git(worktree_path, &["status", "--porcelain"])
            .await?;
        Ok(output
            .stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(parse_status_line)
            .collect())
    }

    pub async fn git_add_all(&self, worktree_path: &Path) -> Result<(), GitWorkspaceError> {
        self.ensure_git_repo(worktree_path).await?;
        self.run_git(worktree_path, &["add", "-A"])
            .await
            .map(|_| ())
    }

    pub async fn git_add_work_item_changes(
        &self,
        worktree_path: &Path,
    ) -> Result<(), GitWorkspaceError> {
        self.ensure_git_repo(worktree_path).await?;
        self.run_git(worktree_path, &["add", "-A"]).await?;
        let output = self
            .run_git(worktree_path, &["diff", "--cached", "--name-only", "-z"])
            .await?;
        for path in output.stdout.split('\0').filter(|path| !path.is_empty()) {
            if should_exclude_from_work_item_commit(path) {
                self.run_git(worktree_path, &["restore", "--staged", "--", path])
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn git_has_staged_changes(
        &self,
        worktree_path: &Path,
    ) -> Result<bool, GitWorkspaceError> {
        self.ensure_git_repo(worktree_path).await?;
        let output = self
            .run_git(worktree_path, &["diff", "--cached", "--name-only", "-z"])
            .await?;
        Ok(!output.stdout.is_empty())
    }

    pub async fn git_commit(
        &self,
        worktree_path: &Path,
        message: &str,
    ) -> Result<CommitResult, GitWorkspaceError> {
        self.ensure_git_repo(worktree_path).await?;
        self.run_git(worktree_path, &["commit", "-m", message])
            .await?;
        let rev = self.run_git(worktree_path, &["rev-parse", "HEAD"]).await?;
        Ok(CommitResult {
            commit_sha: rev.stdout.trim().to_string(),
        })
    }

    pub async fn git_current_head(
        &self,
        worktree_path: &Path,
    ) -> Result<String, GitWorkspaceError> {
        self.ensure_git_repo(worktree_path).await?;
        let rev = self.run_git(worktree_path, &["rev-parse", "HEAD"]).await?;
        Ok(rev.stdout.trim().to_string())
    }

    pub async fn git_push(
        &self,
        worktree_path: &Path,
        remote: &str,
        branch: &str,
    ) -> Result<PushResult, GitWorkspaceError> {
        self.ensure_git_repo(worktree_path).await?;
        let output = self
            .run_git_allow_failure(worktree_path, &["push", remote, branch])
            .await?;
        let status = if output.status_success {
            PushStatus::Pushed
        } else {
            PushStatus::Failed
        };
        let stderr = (!output.stderr.trim().is_empty()).then_some(output.stderr);
        let remote_rejected = !output.status_success
            && stderr
                .as_deref()
                .is_some_and(push_output_is_explicit_remote_rejection);
        Ok(PushResult {
            status,
            remote: remote.to_string(),
            branch: branch.to_string(),
            stderr,
            remote_rejected,
        })
    }

    pub async fn detect_remote_kind(
        &self,
        repo_path: &Path,
    ) -> Result<RemoteKind, GitWorkspaceError> {
        self.ensure_git_repo(repo_path).await?;
        let output = self
            .run_git_allow_failure(repo_path, &["remote", "get-url", "origin"])
            .await?;
        if !output.status_success {
            return Ok(RemoteKind::Unknown);
        }
        let url = output.stdout.trim().to_ascii_lowercase();
        if url.contains("github.com") {
            Ok(RemoteKind::Github)
        } else if url.contains("gitlab.com") {
            Ok(RemoteKind::Gitlab)
        } else if url.is_empty() {
            Ok(RemoteKind::Unknown)
        } else {
            Ok(RemoteKind::GenericGit)
        }
    }

    pub async fn git_diff_stat(
        &self,
        worktree_path: &Path,
        base_branch: &str,
    ) -> Result<DiffStat, GitWorkspaceError> {
        self.ensure_git_repo(worktree_path).await?;
        let output = self
            .run_git(worktree_path, &["diff", "--numstat", base_branch])
            .await?;
        let mut files = Vec::new();
        let mut total_insertions = 0_u32;
        let mut total_deletions = 0_u32;
        for line in output.stdout.lines().filter(|line| !line.trim().is_empty()) {
            let file = parse_numstat_line(line)?;
            total_insertions = total_insertions.saturating_add(file.insertions);
            total_deletions = total_deletions.saturating_add(file.deletions);
            files.push(file);
        }
        Ok(DiffStat {
            files,
            insertions: total_insertions,
            deletions: total_deletions,
        })
    }

    /// `start_commit..completion_commit` 区间内改动过的去重文件路径。
    ///
    /// 当两个提交相同时，没有观察到该 unit 的新提交，因此返回空集合，且不会
    /// 回退为读取单提交或其父提交的 diff。
    pub async fn git_commit_range_changed_files(
        &self,
        worktree_path: &Path,
        start_commit: &str,
        completion_commit: &str,
    ) -> Result<Vec<String>, GitWorkspaceError> {
        if start_commit == completion_commit {
            return Ok(Vec::new());
        }
        self.ensure_git_repo(worktree_path).await?;
        let commit_range = format!("{start_commit}..{completion_commit}");
        let output = self
            .run_git(
                worktree_path,
                &["diff", "--name-only", "--no-renames", &commit_range],
            )
            .await?;
        Ok(output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }

    /// `start_commit..completion_commit` 区间内按提交拓扑顺序排列的提交 SHA。
    ///
    /// 当两个提交相同时，没有观察到该 unit 的新提交，返回空集合。
    pub async fn git_commit_range_commits(
        &self,
        worktree_path: &Path,
        start_commit: &str,
        completion_commit: &str,
    ) -> Result<Vec<String>, GitWorkspaceError> {
        if start_commit == completion_commit {
            return Ok(Vec::new());
        }
        self.ensure_git_repo(worktree_path).await?;
        let commit_range = format!("{start_commit}..{completion_commit}");
        let output = self
            .run_git(
                worktree_path,
                &["rev-list", "--topo-order", "--reverse", &commit_range],
            )
            .await?;
        Ok(output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// 某个 commit 相对其第一父提交所改动的文件路径清单。
    ///
    /// 用于组完成写入范围门禁：每个已完成 unit 的 `completion_commit` 决定该 unit
    /// 实际改了哪些文件，从而保住 per-unit 的归属判定。根提交（无父）返回该提交
    /// 引入的全部文件。
    pub async fn git_commit_changed_files(
        &self,
        worktree_path: &Path,
        commit_sha: &str,
    ) -> Result<Vec<String>, GitWorkspaceError> {
        self.ensure_git_repo(worktree_path).await?;
        let output = self
            .run_git(
                worktree_path,
                &[
                    "show",
                    "--name-only",
                    "--pretty=format:",
                    "--no-renames",
                    commit_sha,
                ],
            )
            .await?;
        Ok(output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect())
    }

    pub async fn git_diff(
        &self,
        worktree_path: &Path,
        base_branch: &str,
    ) -> Result<String, GitWorkspaceError> {
        self.ensure_git_repo(worktree_path).await?;
        let output = self.run_git(worktree_path, &["diff", base_branch]).await?;
        let mut diff = output.stdout;
        let untracked = self
            .run_git(
                worktree_path,
                &["ls-files", "--others", "--exclude-standard", "-z"],
            )
            .await?;
        for path in untracked.stdout.split('\0').filter(|path| !path.is_empty()) {
            let output = self
                .run_git_allow_failure(
                    worktree_path,
                    &["diff", "--no-index", "--", "/dev/null", path],
                )
                .await?;
            if !output.status_success && output.stdout.is_empty() {
                return Err(GitWorkspaceError::CommandFailed {
                    args: format!("diff --no-index -- /dev/null {path}"),
                    cwd: worktree_path.display().to_string(),
                    stderr: output.stderr,
                });
            }
            if !diff.is_empty() && !diff.ends_with('\n') {
                diff.push('\n');
            }
            diff.push_str(&output.stdout);
        }
        Ok(diff)
    }

    async fn find_worktree_branch(
        &self,
        repo_path: &Path,
        worktree_path: &Path,
    ) -> Result<Option<String>, GitWorkspaceError> {
        let output = self
            .run_git(repo_path, &["worktree", "list", "--porcelain"])
            .await?;
        let target = worktree_path
            .canonicalize()
            .unwrap_or_else(|_| worktree_path.to_path_buf());
        let mut current_path: Option<String> = None;
        for line in output.stdout.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                current_path = Some(path.to_string());
            } else if let Some(branch) = line.strip_prefix("branch ") {
                if let Some(path) = current_path.take() {
                    let path_buf = PathBuf::from(&path);
                    let normalized = path_buf.canonicalize().unwrap_or_else(|_| path_buf.clone());
                    if normalized == target {
                        return Ok(Some(
                            branch
                                .strip_prefix("refs/heads/")
                                .unwrap_or(branch)
                                .to_string(),
                        ));
                    }
                }
            } else if line.is_empty() {
                current_path = None;
            }
        }
        Ok(None)
    }

    async fn run_git(
        &self,
        cwd: &Path,
        args: &[&str],
    ) -> Result<GitCommandOutput, GitWorkspaceError> {
        let output = self.run_git_allow_failure(cwd, args).await?;
        if !output.status_success {
            return Err(GitWorkspaceError::CommandFailed {
                args: args.join(" "),
                cwd: cwd.display().to_string(),
                stderr: output.stderr,
            });
        }
        Ok(output)
    }

    async fn run_git_allow_failure(
        &self,
        cwd: &Path,
        args: &[&str],
    ) -> Result<GitCommandOutput, GitWorkspaceError> {
        let mut command = git_command(cwd, args);
        let args_display = args.join(" ");
        let cwd_display = cwd.display().to_string();
        let mut child = ManagedProcessChild::spawn(&mut command).map_err(|error| {
            GitWorkspaceError::Io(format!("git {args_display} in {cwd_display}: {error}"))
        })?;
        let stdout = child.inner().stdout.take().ok_or_else(|| {
            GitWorkspaceError::Io(format!(
                "git {args_display} in {cwd_display}: stdout pipe missing"
            ))
        })?;
        let stderr = child.inner().stderr.take().ok_or_else(|| {
            GitWorkspaceError::Io(format!(
                "git {args_display} in {cwd_display}: stderr pipe missing"
            ))
        })?;
        let stdout_task = tokio::spawn(read_pipe(stdout));
        let stderr_task = tokio::spawn(read_pipe(stderr));

        enum Completion {
            Exited(std::io::Result<std::process::ExitStatus>),
            TimedOut,
            Cancelled,
        }

        let completion = tokio::select! {
            biased;
            status = child.wait() => Completion::Exited(status),
            _ = tokio::time::sleep(self.command_timeout) => Completion::TimedOut,
            _ = self.cancellation.cancelled() => Completion::Cancelled,
        };
        let status = match completion {
            Completion::Exited(status) => status.map_err(|error| {
                GitWorkspaceError::Io(format!("wait git {args_display} in {cwd_display}: {error}"))
            })?,
            Completion::TimedOut => {
                child.terminate().await;
                let _ = join_pipe(stdout_task).await;
                let _ = join_pipe(stderr_task).await;
                return Err(GitWorkspaceError::Timeout {
                    args: args_display,
                    cwd: cwd_display,
                });
            }
            Completion::Cancelled => {
                child.terminate().await;
                let _ = join_pipe(stdout_task).await;
                let _ = join_pipe(stderr_task).await;
                return Err(GitWorkspaceError::Cancelled {
                    args: args_display,
                    cwd: cwd_display,
                });
            }
        };
        let stdout = join_pipe(stdout_task).await?;
        let stderr = join_pipe(stderr_task).await?;
        test_pause::pause_git_command_after_exit_if_configured(cwd, &args_display).await;
        if self.cancellation.is_cancelled() {
            return Err(GitWorkspaceError::Cancelled {
                args: args_display,
                cwd: cwd_display,
            });
        }
        Ok(GitCommandOutput {
            status_success: status.success(),
            stdout: String::from_utf8_lossy(&stdout).to_string(),
            stderr: String::from_utf8_lossy(&stderr).to_string(),
        })
    }

    async fn ensure_git_repo(&self, path: &Path) -> Result<(), GitWorkspaceError> {
        self.run_git(path, &["rev-parse", "--show-toplevel"])
            .await
            .map(|_| ())
    }
}

impl Default for GitWorkspaceService {
    fn default() -> Self {
        Self::new()
    }
}

/// 构造 git 子进程命令。固定 `LC_ALL=C`（评审 I1）：git 的诊断短语
/// （如 `push --delete` 的 `remote ref does not exist`）会随 locale 本地化，
/// 而 `remote_delete_output_is_missing_ref` 仅匹配英文短语，因此必须强制英文输出，
/// 否则非 C locale 下 `delete_remote_branch` 的幂等判定会误报真实失败。
fn git_command(cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(cwd)
        .kill_on_drop(true)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

struct GitCommandOutput {
    status_success: bool,
    stdout: String,
    stderr: String,
}

async fn read_pipe<R>(mut pipe: R) -> std::io::Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes).await?;
    Ok(bytes)
}

async fn join_pipe(
    task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<Vec<u8>, GitWorkspaceError> {
    task.await
        .map_err(|error| GitWorkspaceError::Io(format!("join git output reader: {error}")))?
        .map_err(|error| GitWorkspaceError::Io(format!("read git output: {error}")))
}

fn ensure_safe_worktree_path(
    repo_path: &Path,
    worktree_path: &Path,
) -> Result<(), GitWorkspaceError> {
    let repo_root = repo_path.canonicalize().map_err(|error| {
        GitWorkspaceError::Io(format!("canonicalize {}: {error}", repo_path.display()))
    })?;
    reject_parent_dir_components(worktree_path)?;
    let absolute = if worktree_path.is_absolute() {
        worktree_path.to_path_buf()
    } else {
        repo_root.join(worktree_path)
    };
    let normalized = normalize_existing_prefix(&absolute)?;
    let worktrees_root = normalize_existing_prefix(&repo_root.join(".worktrees"))?;
    if !normalized.starts_with(&worktrees_root) {
        return Err(GitWorkspaceError::UnsafePath(format!(
            "{} is outside {}",
            worktree_path.display(),
            worktrees_root.display()
        )));
    }
    let relative = normalized
        .strip_prefix(&worktrees_root)
        .expect("normalized starts with worktrees_root");
    let first_component = relative
        .components()
        .next()
        .and_then(|component| component.as_os_str().to_str())
        .ok_or_else(|| {
            GitWorkspaceError::UnsafePath(format!(
                "{} has no worktree prefix",
                worktree_path.display()
            ))
        })?;
    if !SAFE_WORKTREE_PREFIXES.contains(&first_component) {
        return Err(GitWorkspaceError::UnsafePath(format!(
            "{} is outside allowed aria worktree prefixes",
            worktree_path.display()
        )));
    }
    Ok(())
}

fn reject_parent_dir_components(path: &Path) -> Result<(), GitWorkspaceError> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(GitWorkspaceError::UnsafePath(format!(
            "{} contains parent directory traversal",
            path.display()
        )));
    }
    Ok(())
}

fn ensure_safe_aria_branch_name(branch_name: &str) -> Result<(), GitWorkspaceError> {
    if branch_name.starts_with('/')
        || branch_name.contains("..")
        || !SAFE_BRANCH_PREFIXES
            .iter()
            .any(|prefix| branch_name.starts_with(*prefix))
    {
        return Err(GitWorkspaceError::UnsafePath(format!(
            "{branch_name} is outside allowed aria branch prefixes"
        )));
    }
    Ok(())
}

fn normalize_existing_prefix(path: &Path) -> Result<PathBuf, GitWorkspaceError> {
    if path.exists() {
        return path.canonicalize().map_err(|error| {
            GitWorkspaceError::Io(format!("canonicalize {}: {error}", path.display()))
        });
    }

    let mut existing = path;
    let mut missing = Vec::<OsString>::new();
    loop {
        if existing.exists() {
            let mut normalized = existing.canonicalize().map_err(|error| {
                GitWorkspaceError::Io(format!("canonicalize {}: {error}", existing.display()))
            })?;
            for component in missing.iter().rev() {
                normalized.push(component);
            }
            return Ok(normalized);
        }

        let Some(name) = existing.file_name() else {
            return Err(GitWorkspaceError::Io(format!(
                "no existing parent for {}",
                path.display()
            )));
        };
        missing.push(name.to_os_string());
        existing = existing
            .parent()
            .ok_or_else(|| GitWorkspaceError::Io(format!("no parent for {}", path.display())))?;
    }
}

fn parse_status_line(line: &str) -> FileStatus {
    let code = line.get(0..2).unwrap_or("").trim().to_string();
    let path = line.get(3..).unwrap_or("").to_string();
    FileStatus { code, path }
}

fn should_exclude_from_work_item_commit(path: &str) -> bool {
    path == ".aria"
        || path.starts_with(".aria/coding-artifacts/")
        || path == "__pycache__"
        || path.starts_with("__pycache__/")
        || path.contains("/__pycache__/")
        || path.ends_with(".pyc")
}

fn push_output_is_explicit_remote_rejection(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("[remote rejected]")
        || stderr.contains("[rejected]")
        || stderr.contains("remote: error:")
}

fn remote_delete_output_is_missing_ref(stderr: &str) -> bool {
    stderr
        .to_ascii_lowercase()
        .contains("remote ref does not exist")
}

fn parse_numstat_line(line: &str) -> Result<DiffFileStat, GitWorkspaceError> {
    let mut parts = line.split('\t');
    let insertions = parse_numstat_count(parts.next(), line)?;
    let deletions = parse_numstat_count(parts.next(), line)?;
    let path = parts
        .next()
        .ok_or_else(|| GitWorkspaceError::Parse(format!("invalid numstat line: {line}")))?
        .to_string();
    Ok(DiffFileStat {
        path,
        insertions,
        deletions,
    })
}

fn parse_numstat_count(value: Option<&str>, line: &str) -> Result<u32, GitWorkspaceError> {
    let value =
        value.ok_or_else(|| GitWorkspaceError::Parse(format!("invalid numstat line: {line}")))?;
    if value == "-" {
        return Ok(0);
    }
    value
        .parse::<u32>()
        .map_err(|error| GitWorkspaceError::Parse(format!("{value}: {error}")))
}

#[cfg(test)]
mod push_tests;
#[cfg(all(test, unix))]
mod tests;
