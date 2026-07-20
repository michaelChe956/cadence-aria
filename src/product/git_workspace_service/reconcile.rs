use std::path::Path;

use super::{GitWorkspaceError, GitWorkspaceService};

impl GitWorkspaceService {
    pub async fn git_ref_head(
        &self,
        repo_path: &Path,
        reference: &str,
    ) -> Result<String, GitWorkspaceError> {
        if reference.trim().is_empty() {
            return Err(GitWorkspaceError::Parse(
                "git reference must not be empty".to_string(),
            ));
        }
        self.ensure_git_repo(repo_path).await?;
        let output = self
            .run_git(repo_path, &["rev-parse", "--verify", reference])
            .await?;
        let head = output.stdout.trim();
        if head.is_empty() {
            return Err(GitWorkspaceError::Parse(format!(
                "git reference has no object id: {reference}"
            )));
        }
        Ok(head.to_string())
    }

    pub async fn git_local_branch_head(
        &self,
        repo_path: &Path,
        branch: &str,
    ) -> Result<Option<String>, GitWorkspaceError> {
        self.ensure_git_repo(repo_path).await?;
        let reference = format!("refs/heads/{branch}");
        let output = self
            .run_git_allow_failure(repo_path, &["show-ref", "--hash", "--verify", &reference])
            .await?;
        if !output.status_success {
            return Ok(None);
        }
        let head = output.stdout.trim();
        if head.is_empty() {
            return Err(GitWorkspaceError::Parse(format!(
                "local branch has no object id: {branch}"
            )));
        }
        Ok(Some(head.to_string()))
    }

    pub async fn git_worktree_branch(
        &self,
        repo_path: &Path,
        worktree_path: &Path,
    ) -> Result<Option<String>, GitWorkspaceError> {
        self.ensure_git_repo(repo_path).await?;
        self.find_worktree_branch(repo_path, worktree_path).await
    }

    pub async fn git_reset_mixed(
        &self,
        worktree_path: &Path,
        target: &str,
    ) -> Result<(), GitWorkspaceError> {
        self.ensure_git_repo(worktree_path).await?;
        self.run_git(worktree_path, &["reset", "--mixed", target])
            .await
            .map(|_| ())
    }

    pub async fn git_commit_parent_and_message(
        &self,
        worktree_path: &Path,
        commit_sha: &str,
    ) -> Result<(String, String), GitWorkspaceError> {
        self.ensure_git_repo(worktree_path).await?;
        let parent_ref = format!("{commit_sha}^");
        let parent = self
            .run_git(worktree_path, &["rev-parse", "--verify", &parent_ref])
            .await?
            .stdout
            .trim()
            .to_string();
        let message = self
            .run_git(worktree_path, &["log", "-1", "--format=%B", commit_sha])
            .await?
            .stdout
            .trim_end()
            .to_string();
        Ok((parent, message))
    }

    pub async fn git_remote_branch_head(
        &self,
        worktree_path: &Path,
        remote: &str,
        branch: &str,
    ) -> Result<Option<String>, GitWorkspaceError> {
        self.ensure_git_repo(worktree_path).await?;
        let reference = format!("refs/heads/{branch}");
        let output = self
            .run_git(worktree_path, &["ls-remote", "--heads", remote, &reference])
            .await?;
        let line = output.stdout.trim();
        if line.is_empty() {
            return Ok(None);
        }
        let mut fields = line.split_whitespace();
        let head = fields.next().unwrap_or_default();
        let found_ref = fields.next().unwrap_or_default();
        if head.is_empty() || found_ref != reference || fields.next().is_some() {
            return Err(GitWorkspaceError::Parse(format!(
                "unexpected ls-remote output for {remote}/{branch}: {line}"
            )));
        }
        Ok(Some(head.to_string()))
    }
}
