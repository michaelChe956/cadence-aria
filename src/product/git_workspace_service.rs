use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

use crate::product::coding_models::{PushStatus, RemoteKind};

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
}

impl GitWorkspaceService {
    pub fn new() -> Self {
        Self {
            command_timeout: Duration::from_secs(30),
        }
    }

    pub async fn create_branch(
        &self,
        repo_path: &Path,
        branch_name: &str,
        base_branch: &str,
    ) -> Result<(), GitWorkspaceError> {
        ensure_git_repo(repo_path).await?;
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
        ensure_git_repo(repo_path).await?;
        ensure_safe_worktree_path(repo_path, worktree_path)?;
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

    pub async fn git_status(
        &self,
        worktree_path: &Path,
    ) -> Result<Vec<FileStatus>, GitWorkspaceError> {
        ensure_git_repo(worktree_path).await?;
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
        ensure_git_repo(worktree_path).await?;
        self.run_git(worktree_path, &["add", "-A"])
            .await
            .map(|_| ())
    }

    pub async fn git_commit(
        &self,
        worktree_path: &Path,
        message: &str,
    ) -> Result<CommitResult, GitWorkspaceError> {
        ensure_git_repo(worktree_path).await?;
        self.run_git(worktree_path, &["commit", "-m", message])
            .await?;
        let rev = self.run_git(worktree_path, &["rev-parse", "HEAD"]).await?;
        Ok(CommitResult {
            commit_sha: rev.stdout.trim().to_string(),
        })
    }

    pub async fn git_push(
        &self,
        worktree_path: &Path,
        remote: &str,
        branch: &str,
    ) -> Result<PushResult, GitWorkspaceError> {
        ensure_git_repo(worktree_path).await?;
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
        ensure_git_repo(repo_path).await?;
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
        ensure_git_repo(worktree_path).await?;
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
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = timeout(self.command_timeout, command.output())
            .await
            .map_err(|_| GitWorkspaceError::Timeout {
                args: args.join(" "),
                cwd: cwd.display().to_string(),
            })?
            .map_err(|error| {
                GitWorkspaceError::Io(format!(
                    "git {} in {}: {error}",
                    args.join(" "),
                    cwd.display()
                ))
            })?;
        Ok(GitCommandOutput {
            status_success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
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

async fn ensure_git_repo(path: &Path) -> Result<(), GitWorkspaceError> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| {
            GitWorkspaceError::Io(format!("git rev-parse in {}: {error}", path.display()))
        })?;
    if !output.status.success() {
        return Err(GitWorkspaceError::CommandFailed {
            args: "rev-parse --show-toplevel".to_string(),
            cwd: path.display().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }
    Ok(())
}

fn ensure_safe_worktree_path(
    repo_path: &Path,
    worktree_path: &Path,
) -> Result<(), GitWorkspaceError> {
    let repo_root = repo_path.canonicalize().map_err(|error| {
        GitWorkspaceError::Io(format!("canonicalize {}: {error}", repo_path.display()))
    })?;
    let normalized = if worktree_path.is_absolute() {
        worktree_path.to_path_buf()
    } else {
        repo_root.join(worktree_path)
    };
    let expected = repo_root.join(".worktrees").join("aria-work-items");
    if !normalized.starts_with(&expected) {
        return Err(GitWorkspaceError::UnsafePath(format!(
            "{} is outside {}",
            worktree_path.display(),
            expected.display()
        )));
    }
    Ok(())
}

fn parse_status_line(line: &str) -> FileStatus {
    let code = line.get(0..2).unwrap_or("").trim().to_string();
    let path = line.get(3..).unwrap_or("").to_string();
    FileStatus { code, path }
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
