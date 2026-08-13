use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::coding_models::{PushStatus, RemoteKind};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

use super::locking::{canonical_path_identity, with_exclusive_lock};

const CODING_GIT_OPERATION_KIND: &str = "coding_git_operation";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingGitOperationKind {
    WorktreePrepare,
    ReviewRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingGitOperationPhase {
    Before,
    BranchCreated,
    WorktreeCreated,
    CommitStarted,
    CommitCreated,
    PushStarted,
    Compensated,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareCodingGitOperationInput {
    pub kind: CodingGitOperationKind,
    pub repo_path: PathBuf,
    pub worktree_path: PathBuf,
    pub branch_name: String,
    pub base_branch: String,
    pub before_head: String,
    pub remote: Option<String>,
    pub commit_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteReviewGitOperationInput {
    pub push_status: PushStatus,
    pub remote_kind: RemoteKind,
    pub review_request_id: String,
    pub push_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingGitOperationJournal {
    pub project_id: String,
    pub issue_id: String,
    pub attempt_id: String,
    pub kind: CodingGitOperationKind,
    pub repo_path: PathBuf,
    pub worktree_path: PathBuf,
    pub branch_name: String,
    pub base_branch: String,
    pub before_head: String,
    pub remote: Option<String>,
    pub commit_message: Option<String>,
    pub phase: CodingGitOperationPhase,
    pub commit_sha: Option<String>,
    pub push_status: Option<PushStatus>,
    #[serde(default)]
    pub push_error: Option<String>,
    pub remote_kind: Option<RemoteKind>,
    pub review_request_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl CodingGitOperationJournal {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.phase,
            CodingGitOperationPhase::Compensated | CodingGitOperationPhase::Completed
        )
    }
}

impl super::CodingAttemptStore {
    pub fn prepare_coding_git_operation(
        &self,
        attempt: &CodingExecutionAttempt,
        input: PrepareCodingGitOperationInput,
    ) -> Result<CodingGitOperationJournal, ProductStoreError> {
        let current_attempt = self.validate_attempt_lineage(attempt)?;
        let candidate = build_journal(&current_attempt, input)?;
        let path = self.coding_git_operation_path(
            &current_attempt.project_id,
            &current_attempt.issue_id,
            &current_attempt.id,
        );

        with_exclusive_lock(&path, || {
            if path.is_file() {
                let existing: CodingGitOperationJournal = read_json(&path)?;
                validate_journal(&existing, &current_attempt)?;
                if same_identity(&existing, &candidate) {
                    return Ok(existing);
                }
                if !existing.is_terminal() {
                    return Err(identity_mismatch(&current_attempt.id));
                }
            }
            write_json(&path, &candidate)?;
            Ok(candidate)
        })
    }

    pub fn get_coding_git_operation(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Option<CodingGitOperationJournal>, ProductStoreError> {
        let current_attempt = self.validate_attempt_lineage(attempt)?;
        let path = self.coding_git_operation_path(
            &current_attempt.project_id,
            &current_attempt.issue_id,
            &current_attempt.id,
        );
        if !path.is_file() {
            return Ok(None);
        }
        let journal: CodingGitOperationJournal = read_json(&path)?;
        validate_journal(&journal, &current_attempt)?;
        Ok(Some(journal))
    }

    pub fn advance_coding_git_operation(
        &self,
        attempt: &CodingExecutionAttempt,
        expected: &CodingGitOperationJournal,
        phase: CodingGitOperationPhase,
        commit_sha: Option<String>,
    ) -> Result<CodingGitOperationJournal, ProductStoreError> {
        let current_attempt = self.validate_attempt_lineage(attempt)?;
        let path = self.coding_git_operation_path(
            &current_attempt.project_id,
            &current_attempt.issue_id,
            &current_attempt.id,
        );

        with_exclusive_lock(&path, || {
            if !path.is_file() {
                return Err(identity_mismatch(&current_attempt.id));
            }
            let mut current: CodingGitOperationJournal = read_json(&path)?;
            validate_journal(&current, &current_attempt)?;
            if !same_identity(&current, expected) {
                return Err(identity_mismatch(&current_attempt.id));
            }
            if let Some(commit_sha) = commit_sha {
                validate_non_empty("commit sha", &commit_sha, &current_attempt.id)?;
                if current.kind != CodingGitOperationKind::ReviewRequest
                    || phase != CodingGitOperationPhase::CommitCreated
                    || current
                        .commit_sha
                        .as_deref()
                        .is_some_and(|existing| existing != commit_sha)
                {
                    return Err(identity_mismatch(&current_attempt.id));
                }
                current.commit_sha = Some(commit_sha);
            }
            if phase == current.phase {
                validate_phase_state(&current)?;
                return Ok(current);
            }
            if !is_allowed_transition(current.kind, current.phase, phase) {
                return Err(identity_mismatch(&current_attempt.id));
            }
            current.phase = phase;
            current.updated_at = Utc::now().to_rfc3339();
            validate_phase_state(&current)?;
            write_json(&path, &current)?;
            Ok(current)
        })
    }

    pub fn complete_review_coding_git_operation(
        &self,
        attempt: &CodingExecutionAttempt,
        expected: &CodingGitOperationJournal,
        input: CompleteReviewGitOperationInput,
    ) -> Result<CodingGitOperationJournal, ProductStoreError> {
        validate_relative_id(&input.review_request_id)?;
        let current_attempt = self.validate_attempt_lineage(attempt)?;
        let path = self.coding_git_operation_path(
            &current_attempt.project_id,
            &current_attempt.issue_id,
            &current_attempt.id,
        );

        with_exclusive_lock(&path, || {
            if !path.is_file() {
                return Err(identity_mismatch(&current_attempt.id));
            }
            let mut current: CodingGitOperationJournal = read_json(&path)?;
            validate_journal(&current, &current_attempt)?;
            if !same_identity(&current, expected)
                || current.kind != CodingGitOperationKind::ReviewRequest
            {
                return Err(identity_mismatch(&current_attempt.id));
            }
            if current.phase == CodingGitOperationPhase::Completed {
                if current.push_status.as_ref() == Some(&input.push_status)
                    && current.remote_kind.as_ref() == Some(&input.remote_kind)
                    && current.review_request_id.as_deref()
                        == Some(input.review_request_id.as_str())
                {
                    return Ok(current);
                }
                return Err(identity_mismatch(&current_attempt.id));
            }
            if current.phase != CodingGitOperationPhase::PushStarted || current.commit_sha.is_none()
            {
                return Err(identity_mismatch(&current_attempt.id));
            }
            current.phase = CodingGitOperationPhase::Completed;
            current.push_status = Some(input.push_status);
            current.push_error = input.push_error;
            current.remote_kind = Some(input.remote_kind);
            current.review_request_id = Some(input.review_request_id);
            current.updated_at = Utc::now().to_rfc3339();
            validate_phase_state(&current)?;
            write_json(&path, &current)?;
            Ok(current)
        })
    }

    /// 重推（RetryPush）专用重开：仅当 journal 为 `Completed(Failed)` 时重开为
    /// `PushStarted`（保留 `commit_sha`/`before_head`，清空 push 完成态字段），供
    /// `execute_review_request` 幂等重入时重新 push。
    ///
    /// 红线：`Completed(Pushed)` 与其他非 Completed 相位绝不重开，一律返回
    /// `IdentityMismatch`（编程错误，不静默）。
    pub fn reopen_failed_review_git_operation(
        &self,
        attempt: &CodingExecutionAttempt,
        expected: &CodingGitOperationJournal,
    ) -> Result<CodingGitOperationJournal, ProductStoreError> {
        let current_attempt = self.validate_attempt_lineage(attempt)?;
        let path = self.coding_git_operation_path(
            &current_attempt.project_id,
            &current_attempt.issue_id,
            &current_attempt.id,
        );

        with_exclusive_lock(&path, || {
            if !path.is_file() {
                return Err(identity_mismatch(&current_attempt.id));
            }
            let mut current: CodingGitOperationJournal = read_json(&path)?;
            validate_journal(&current, &current_attempt)?;
            if !same_identity(&current, expected)
                || current.kind != CodingGitOperationKind::ReviewRequest
                || current.phase != CodingGitOperationPhase::Completed
                || current.push_status != Some(PushStatus::Failed)
                || current.commit_sha.is_none()
            {
                return Err(identity_mismatch(&current_attempt.id));
            }
            current.phase = CodingGitOperationPhase::PushStarted;
            current.push_status = None;
            current.push_error = None;
            current.remote_kind = None;
            current.review_request_id = None;
            current.updated_at = Utc::now().to_rfc3339();
            validate_phase_state(&current)?;
            write_json(&path, &current)?;
            Ok(current)
        })
    }
}

fn build_journal(
    attempt: &CodingExecutionAttempt,
    input: PrepareCodingGitOperationInput,
) -> Result<CodingGitOperationJournal, ProductStoreError> {
    validate_non_empty("branch", &input.branch_name, &attempt.id)?;
    validate_non_empty("base branch", &input.base_branch, &attempt.id)?;
    validate_non_empty("before head", &input.before_head, &attempt.id)?;
    if input.branch_name != attempt.branch_name || input.base_branch != attempt.base_branch {
        return Err(identity_mismatch(&attempt.id));
    }
    if let Some(remote) = input.remote.as_deref() {
        validate_non_empty("remote", remote, &attempt.id)?;
    }
    if let Some(message) = input.commit_message.as_deref() {
        validate_non_empty("commit message", message, &attempt.id)?;
    }
    match input.kind {
        CodingGitOperationKind::WorktreePrepare => {
            if input.remote.is_some() || input.commit_message.is_some() {
                return Err(identity_mismatch(&attempt.id));
            }
        }
        CodingGitOperationKind::ReviewRequest => {
            if input.remote.is_none()
                || input.commit_message.is_none()
                || attempt.worktree_path.as_ref() != Some(&input.worktree_path)
            {
                return Err(identity_mismatch(&attempt.id));
            }
        }
    }

    let now = Utc::now().to_rfc3339();
    let journal = CodingGitOperationJournal {
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        attempt_id: attempt.id.clone(),
        kind: input.kind,
        repo_path: canonical_path_identity(&input.repo_path)?,
        worktree_path: canonical_path_identity(&input.worktree_path)?,
        branch_name: input.branch_name,
        base_branch: input.base_branch,
        before_head: input.before_head,
        remote: input.remote,
        commit_message: input.commit_message,
        phase: CodingGitOperationPhase::Before,
        commit_sha: None,
        push_status: None,
        push_error: None,
        remote_kind: None,
        review_request_id: None,
        created_at: now.clone(),
        updated_at: now,
    };
    validate_journal(&journal, attempt)?;
    Ok(journal)
}

fn validate_journal(
    journal: &CodingGitOperationJournal,
    attempt: &CodingExecutionAttempt,
) -> Result<(), ProductStoreError> {
    if journal.project_id != attempt.project_id
        || journal.issue_id != attempt.issue_id
        || journal.attempt_id != attempt.id
        || journal.branch_name != attempt.branch_name
        || journal.base_branch != attempt.base_branch
        || canonical_path_identity(&journal.repo_path)? != journal.repo_path
        || canonical_path_identity(&journal.worktree_path)? != journal.worktree_path
    {
        return Err(identity_mismatch(&attempt.id));
    }
    validate_non_empty("branch", &journal.branch_name, &attempt.id)?;
    validate_non_empty("base branch", &journal.base_branch, &attempt.id)?;
    validate_non_empty("before head", &journal.before_head, &attempt.id)?;
    validate_non_empty("created at", &journal.created_at, &attempt.id)?;
    validate_non_empty("updated at", &journal.updated_at, &attempt.id)?;
    match journal.kind {
        CodingGitOperationKind::WorktreePrepare => {
            if journal.remote.is_some()
                || journal.commit_message.is_some()
                || journal.commit_sha.is_some()
                || journal.push_status.is_some()
                || journal.remote_kind.is_some()
                || journal.review_request_id.is_some()
            {
                return Err(identity_mismatch(&attempt.id));
            }
        }
        CodingGitOperationKind::ReviewRequest => {
            let Some(remote) = journal.remote.as_deref() else {
                return Err(identity_mismatch(&attempt.id));
            };
            let Some(message) = journal.commit_message.as_deref() else {
                return Err(identity_mismatch(&attempt.id));
            };
            validate_non_empty("remote", remote, &attempt.id)?;
            validate_non_empty("commit message", message, &attempt.id)?;
            if let Some(review_request_id) = journal.review_request_id.as_deref() {
                validate_relative_id(review_request_id)?;
            }
            if let Some(path) = attempt.worktree_path.as_ref()
                && canonical_path_identity(path)? != journal.worktree_path
            {
                return Err(identity_mismatch(&attempt.id));
            }
        }
    }
    validate_phase_state(journal)
}

fn validate_phase_state(journal: &CodingGitOperationJournal) -> Result<(), ProductStoreError> {
    let valid = match journal.kind {
        CodingGitOperationKind::WorktreePrepare => matches!(
            journal.phase,
            CodingGitOperationPhase::Before
                | CodingGitOperationPhase::BranchCreated
                | CodingGitOperationPhase::WorktreeCreated
                | CodingGitOperationPhase::Compensated
                | CodingGitOperationPhase::Completed
        ),
        CodingGitOperationKind::ReviewRequest => {
            let phase_valid = matches!(
                journal.phase,
                CodingGitOperationPhase::Before
                    | CodingGitOperationPhase::CommitStarted
                    | CodingGitOperationPhase::CommitCreated
                    | CodingGitOperationPhase::PushStarted
                    | CodingGitOperationPhase::Compensated
                    | CodingGitOperationPhase::Completed
            );
            let commit_valid = match journal.phase {
                CodingGitOperationPhase::CommitCreated
                | CodingGitOperationPhase::PushStarted
                | CodingGitOperationPhase::Completed => journal.commit_sha.is_some(),
                _ => true,
            };
            let completion_valid = if journal.phase == CodingGitOperationPhase::Completed {
                journal.push_status.is_some()
                    && journal.remote_kind.is_some()
                    && journal.review_request_id.is_some()
            } else {
                journal.push_status.is_none()
                    && journal.remote_kind.is_none()
                    && journal.review_request_id.is_none()
            };
            phase_valid && commit_valid && completion_valid
        }
    };
    if valid {
        Ok(())
    } else {
        Err(identity_mismatch(&journal.attempt_id))
    }
}

fn same_identity(left: &CodingGitOperationJournal, right: &CodingGitOperationJournal) -> bool {
    left.project_id == right.project_id
        && left.issue_id == right.issue_id
        && left.attempt_id == right.attempt_id
        && left.kind == right.kind
        && left.repo_path == right.repo_path
        && left.worktree_path == right.worktree_path
        && left.branch_name == right.branch_name
        && left.base_branch == right.base_branch
        && left.before_head == right.before_head
        && left.remote == right.remote
        && left.commit_message == right.commit_message
}

fn is_allowed_transition(
    kind: CodingGitOperationKind,
    current: CodingGitOperationPhase,
    next: CodingGitOperationPhase,
) -> bool {
    if next == CodingGitOperationPhase::Compensated {
        return !matches!(
            current,
            CodingGitOperationPhase::Completed | CodingGitOperationPhase::Compensated
        );
    }
    match kind {
        CodingGitOperationKind::WorktreePrepare => matches!(
            (current, next),
            (
                CodingGitOperationPhase::Before,
                CodingGitOperationPhase::BranchCreated
            ) | (
                CodingGitOperationPhase::BranchCreated,
                CodingGitOperationPhase::WorktreeCreated
            ) | (
                CodingGitOperationPhase::WorktreeCreated,
                CodingGitOperationPhase::Completed
            )
        ),
        CodingGitOperationKind::ReviewRequest => matches!(
            (current, next),
            (
                CodingGitOperationPhase::Before,
                CodingGitOperationPhase::CommitStarted
            ) | (
                CodingGitOperationPhase::CommitStarted,
                CodingGitOperationPhase::CommitCreated
            ) | (
                CodingGitOperationPhase::CommitCreated,
                CodingGitOperationPhase::PushStarted
            ) | (
                CodingGitOperationPhase::PushStarted,
                CodingGitOperationPhase::Completed
            )
        ),
    }
}

fn validate_non_empty(field: &str, value: &str, attempt_id: &str) -> Result<(), ProductStoreError> {
    if value.trim().is_empty() {
        return Err(ProductStoreError::IdentityMismatch {
            kind: CODING_GIT_OPERATION_KIND,
            id: format!("{attempt_id}:{field}"),
        });
    }
    Ok(())
}

fn identity_mismatch(attempt_id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: CODING_GIT_OPERATION_KIND,
        id: attempt_id.to_string(),
    }
}
