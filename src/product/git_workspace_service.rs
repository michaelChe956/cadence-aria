use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use command_group::AsyncCommandGroup;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::product::coding_models::{PushStatus, RemoteKind};

mod reconcile;
mod test_pause;

pub use test_pause::{GitCommandPause, pause_next_git_command_after_exit};

const SAFE_WORKTREE_PREFIXES: &[&str] = &["aria-work-items", "aria-issues"];
const SAFE_BRANCH_PREFIXES: &[&str] = &["aria/work-items/", "aria/issues/"];

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
        Ok(PushResult {
            status,
            remote: remote.to_string(),
            branch: branch.to_string(),
            stderr: (!output.stderr.trim().is_empty()).then_some(output.stderr),
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
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(cwd)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let args_display = args.join(" ");
        let cwd_display = cwd.display().to_string();
        let mut child = command.group_spawn().map_err(|error| {
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
                terminate_git_child(&mut child).await;
                let _ = join_pipe(stdout_task).await;
                let _ = join_pipe(stderr_task).await;
                return Err(GitWorkspaceError::Timeout {
                    args: args_display,
                    cwd: cwd_display,
                });
            }
            Completion::Cancelled => {
                terminate_git_child(&mut child).await;
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

async fn terminate_git_child(child: &mut command_group::AsyncGroupChild) {
    #[cfg(unix)]
    if let Some(pgid) = child.id() {
        // SAFETY: pgid is the process-group id returned for this spawned Git command.
        unsafe {
            let _ = libc::killpg(pgid as i32, libc::SIGKILL);
        }
    }
    let _ = child.start_kill();
    let _ = child.inner().start_kill();
    let _ = child.wait().await;
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
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Command as StdCommand;
    use std::time::Duration;

    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    use super::{GitWorkspaceError, GitWorkspaceService};

    fn git(repo: &Path, args: &[&str]) {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn cancelled_git_commit_is_killed_reaped_and_never_commits_late() {
        let tmp = tempdir().expect("tempdir");
        let repo = tmp.path();
        git(repo, &["init"]);
        git(repo, &["config", "user.email", "test@example.com"]);
        git(repo, &["config", "user.name", "Test User"]);
        fs::write(repo.join("README.md"), "base\n").expect("write base");
        git(repo, &["add", "README.md"]);
        git(repo, &["commit", "-m", "base"]);
        let before = StdCommand::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo)
            .output()
            .expect("read before head")
            .stdout;

        fs::write(repo.join("README.md"), "changed\n").expect("write change");
        git(repo, &["add", "README.md"]);
        let entered = repo.join("hook-entered");
        let release = repo.join("hook-release");
        let hook = repo.join(".git/hooks/pre-commit");
        fs::write(
            &hook,
            format!(
                "#!/bin/sh\ntouch '{}'\nwhile [ ! -f '{}' ]; do sleep 0.01; done\n",
                entered.display(),
                release.display()
            ),
        )
        .expect("write hook");
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("chmod hook");

        let cancellation = CancellationToken::new();
        let service = GitWorkspaceService::new().with_cancellation(cancellation.clone());
        let commit = tokio::spawn({
            let repo = repo.to_path_buf();
            async move { service.git_commit(&repo, "cancelled commit").await }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !entered.exists() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("pre-commit hook entered");
        cancellation.cancel();
        let result = tokio::time::timeout(Duration::from_millis(500), commit)
            .await
            .expect("cancelled git commit must be reaped")
            .expect("git commit task");
        assert!(matches!(result, Err(GitWorkspaceError::Cancelled { .. })));

        fs::write(&release, "release\n").expect("release hook if process survived");
        tokio::time::sleep(Duration::from_millis(100)).await;
        let after = StdCommand::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(repo)
            .output()
            .expect("read after head")
            .stdout;
        assert_eq!(
            before, after,
            "cancelled git process committed after return"
        );
    }
}
