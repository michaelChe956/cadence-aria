use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::coding_models::{PushStatus, RemoteKind};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::PointerPublication;

use super::locking::{canonical_path_identity, with_exclusive_lock};

const CODING_GIT_OPERATION_KIND: &str = "coding_git_operation";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingGitOperationKind {
    WorktreePrepare,
    ReviewRequest,
    PointerPublish,
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

// ---------------------------------------------------------------------------
// T19:平台 git mutation 写通道守卫(stage/commit/push/补偿共用单一收口)
//
// 约定:窗口 store 根 = `<aria>/reviewer-windows`(与
// coding_workspace_engine::code_review 的 `REVIEWER_WINDOW_STORE_DIR`
// 字面量同源一致——两处 mod 均私有,常量各持一份并互相锚定,改动须同步)。
// 闸门按 common-dir 键控:窗口冻结(Frozen/Draining)期间一律拒绝新 writer;
// enter 为非阻塞判定/登记,不在 attempt 仲裁锁内等待窗口释放(无死锁面)。
// 残留 writer(崩溃于 enter/exit 之间)由窗口排空按 PID-start 收割或停等,
// fail-closed。
// ---------------------------------------------------------------------------

const REVIEWER_WINDOW_STORE_DIR_NAME: &str = "reviewer-windows";

fn enter_review_write_channel(
    paths: &crate::product::app_paths::ProductAppPaths,
    worktree: &Path,
    channel: crate::product::reviewer_window_store::WriteChannelKind,
    attempt_id: &str,
) -> Result<(), ProductStoreError> {
    let store = crate::product::reviewer_window_store::ReviewerWindowStore::new(
        paths.root().join(REVIEWER_WINDOW_STORE_DIR_NAME),
    );
    // writer 身份与 engine 侧补偿守卫同方案(platform-git-{attempt_id} +
    // 本进程 PID-start),两侧 exit-all 自愈互认;改动须双侧同步。
    let writer = crate::product::reviewer_window_store::ChannelWriter {
        writer_id: format!("platform-git-{attempt_id}"),
        boot_identity: format!("aria-platform-{}", std::process::id()),
        pid: std::process::id(),
        start_time_ticks: crate::cross_cutting::process_manager::process_start_time_ticks(
            std::process::id(),
        )
        .unwrap_or(0),
        entered_at: Utc::now().to_rfc3339(),
    };
    crate::product::reviewer_window::enter_platform_write(&store, worktree, channel, writer)
        .map_err(|error| {
            ProductStoreError::Io(format!("reviewer_window_channel_blocked: {}", error))
        })
}

fn channel_exit_set() -> [crate::product::reviewer_window_store::WriteChannelKind; 4] {
    crate::product::reviewer_window_store::ALL_WRITE_CHANNELS
}

fn exit_review_write_channels_best_effort(
    paths: &crate::product::app_paths::ProductAppPaths,
    worktree: &Path,
    attempt_id: &str,
    channels: &[crate::product::reviewer_window_store::WriteChannelKind],
) {
    let store = crate::product::reviewer_window_store::ReviewerWindowStore::new(
        paths.root().join(REVIEWER_WINDOW_STORE_DIR_NAME),
    );
    let writer_id = format!("platform-git-{attempt_id}");
    for channel in channels {
        if let Err(error) = crate::product::reviewer_window::exit_platform_write(
            &store, worktree, *channel, &writer_id,
        ) {
            tracing::warn!(
                ?error,
                channel = channel.as_str(),
                "reviewer_window_channel_exit_failed"
            );
        }
    }
}

/// ReviewRequest 类 journal 相位 → 「该相位期间即将发生的平台 mutation」
/// 所属通道(None = 其间只有读操作/无 mutation)。enter 在相位落盘后、
/// mutation 发生前登记;下一相位推进时退出上一通道。
fn review_request_channel_for(
    phase: CodingGitOperationPhase,
) -> Option<crate::product::reviewer_window_store::WriteChannelKind> {
    use crate::product::reviewer_window_store::WriteChannelKind;
    match phase {
        // Before:git_add(Stage)。
        CodingGitOperationPhase::Before => Some(WriteChannelKind::Stage),
        // CommitStarted:git_commit(Commit)。
        CodingGitOperationPhase::CommitStarted => Some(WriteChannelKind::Commit),
        // CommitCreated→PushStarted 之间只有远端读探测;PushStarted:git_push。
        CodingGitOperationPhase::CommitCreated => None,
        CodingGitOperationPhase::PushStarted => Some(WriteChannelKind::Push),
        CodingGitOperationPhase::Compensated
        | CodingGitOperationPhase::Completed
        | CodingGitOperationPhase::BranchCreated
        | CodingGitOperationPhase::WorktreeCreated => None,
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

        // T19 通道守卫(自愈式):进入前清本 writer 全部残留(崩溃残留/
        // 上一相位;同 attempt 流程经 journal 锁串行,无误清并发),
        // durable 写意图落盘后、mutation 发生前登记 writer;窗口冻结
        // 期间拒绝(journal 停在原相位,重入幂等续接)。
        // ⚠4(修复轮 1):仅 ReviewRequest 走通道守卫——WorktreePrepare/
        // PointerPublish 不在 reviewer 窗口锁域,跳过闸门 IO。
        if candidate.kind == CodingGitOperationKind::ReviewRequest {
            let paths = self.paths();
            exit_review_write_channels_best_effort(
                &paths,
                &candidate.worktree_path,
                &current_attempt.id,
                &channel_exit_set(),
            );
        }
        let journal = with_exclusive_lock(&path, || {
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
        })?;
        if journal.kind == CodingGitOperationKind::ReviewRequest
            && let Some(channel) = review_request_channel_for(journal.phase)
        {
            enter_review_write_channel(
                &self.paths(),
                &journal.worktree_path,
                channel,
                &current_attempt.id,
            )?;
        }
        Ok(journal)
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
        // T19 通道守卫(自愈式,与 prepare 同款):清残留→durable 推进→
        // 登记新相位通道(冻结即拒;早退相位重入同样登记)。
        if expected.kind == CodingGitOperationKind::ReviewRequest {
            let paths = self.paths();
            exit_review_write_channels_best_effort(
                &paths,
                &expected.worktree_path,
                &current_attempt.id,
                &channel_exit_set(),
            );
        }
        let journal = with_exclusive_lock(&path, || {
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
        })?;
        if journal.kind == CodingGitOperationKind::ReviewRequest {
            let paths = self.paths();
            if let Some(next_channel) = review_request_channel_for(journal.phase) {
                enter_review_write_channel(
                    &paths,
                    &journal.worktree_path,
                    next_channel,
                    &current_attempt.id,
                )?;
            }
            // 终相位:全通道退出(含补偿)。
            if matches!(
                journal.phase,
                CodingGitOperationPhase::Completed | CodingGitOperationPhase::Compensated
            ) {
                exit_review_write_channels_best_effort(
                    &paths,
                    &journal.worktree_path,
                    &current_attempt.id,
                    &channel_exit_set(),
                );
            }
        }
        Ok(journal)
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

        let journal = with_exclusive_lock(&path, || {
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
        })?;
        // T19:Completed 终相位——全通道退出(幂等)。
        exit_review_write_channels_best_effort(
            &self.paths(),
            &journal.worktree_path,
            &current_attempt.id,
            &channel_exit_set(),
        );
        Ok(journal)
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

        let journal = with_exclusive_lock(&path, || {
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
        })?;
        // T19:RetryPush 重开为 PushStarted——清残留后登记 Push 通道
        // (冻结即拒,journal 已在 PushStarted,重入续接)。
        {
            let paths = self.paths();
            exit_review_write_channels_best_effort(
                &paths,
                &journal.worktree_path,
                &current_attempt.id,
                &channel_exit_set(),
            );
            enter_review_write_channel(
                &paths,
                &journal.worktree_path,
                crate::product::reviewer_window_store::WriteChannelKind::Push,
                &current_attempt.id,
            )?;
        }
        Ok(journal)
    }

    // 签名按 C-5 Task 8 简报逐字定义（9 参数），保留既定接口，不放宽参数上限。
    #[allow(clippy::too_many_arguments)]
    pub fn start_pointer_publish_git_operation(
        &self,
        publication: &PointerPublication,
        member_repo_id: &str,
        repo_path: &Path,
        worktree_path: &Path,
        branch_name: &str,
        base_branch: &str,
        remote: &str,
        commit_message: &str,
    ) -> Result<CodingGitOperationJournal, ProductStoreError> {
        validate_pointer_publication_ids(publication, member_repo_id)?;
        let id = pointer_journal_id(&publication.id, member_repo_id);
        let candidate = build_pointer_journal(
            publication,
            member_repo_id,
            repo_path,
            worktree_path,
            branch_name,
            base_branch,
            remote,
            commit_message,
        )?;
        let path = self.pointer_publication_git_operation_path(
            &publication.project_id,
            &publication.id,
            &id,
        );

        with_exclusive_lock(&path, || {
            if path.is_file() {
                let existing: CodingGitOperationJournal = read_json(&path)?;
                validate_pointer_journal(&existing, publication, member_repo_id)?;
                if same_identity(&existing, &candidate) {
                    return Ok(existing);
                }
                if !existing.is_terminal() {
                    return Err(identity_mismatch(&id));
                }
            }
            write_json(&path, &candidate)?;
            Ok(candidate)
        })
    }

    pub fn advance_pointer_publish_git_operation(
        &self,
        publication: &PointerPublication,
        member_repo_id: &str,
        expected: &CodingGitOperationJournal,
        phase: CodingGitOperationPhase,
        commit_sha: Option<String>,
    ) -> Result<CodingGitOperationJournal, ProductStoreError> {
        validate_pointer_publication_ids(publication, member_repo_id)?;
        let id = pointer_journal_id(&publication.id, member_repo_id);
        let path = self.pointer_publication_git_operation_path(
            &publication.project_id,
            &publication.id,
            &id,
        );

        with_exclusive_lock(&path, || {
            if !path.is_file() {
                return Err(identity_mismatch(&id));
            }
            let mut current: CodingGitOperationJournal = read_json(&path)?;
            validate_pointer_journal(&current, publication, member_repo_id)?;
            if !same_identity(&current, expected) {
                return Err(identity_mismatch(&id));
            }
            if let Some(commit_sha) = commit_sha {
                validate_non_empty("commit sha", &commit_sha, &id)?;
                if current.kind != CodingGitOperationKind::PointerPublish
                    || phase != CodingGitOperationPhase::CommitCreated
                    || current
                        .commit_sha
                        .as_deref()
                        .is_some_and(|existing| existing != commit_sha)
                {
                    return Err(identity_mismatch(&id));
                }
                current.commit_sha = Some(commit_sha);
            }
            if phase == current.phase {
                validate_phase_state(&current)?;
                return Ok(current);
            }
            if !is_allowed_transition(current.kind, current.phase, phase) {
                return Err(identity_mismatch(&id));
            }
            current.phase = phase;
            current.updated_at = Utc::now().to_rfc3339();
            validate_phase_state(&current)?;
            write_json(&path, &current)?;
            Ok(current)
        })
    }

    pub fn complete_review_pointer_publish_git_operation(
        &self,
        publication: &PointerPublication,
        member_repo_id: &str,
        expected: &CodingGitOperationJournal,
        input: CompleteReviewGitOperationInput,
    ) -> Result<CodingGitOperationJournal, ProductStoreError> {
        validate_relative_id(&input.review_request_id)?;
        validate_pointer_publication_ids(publication, member_repo_id)?;
        let id = pointer_journal_id(&publication.id, member_repo_id);
        let path = self.pointer_publication_git_operation_path(
            &publication.project_id,
            &publication.id,
            &id,
        );

        with_exclusive_lock(&path, || {
            if !path.is_file() {
                return Err(identity_mismatch(&id));
            }
            let mut current: CodingGitOperationJournal = read_json(&path)?;
            validate_pointer_journal(&current, publication, member_repo_id)?;
            if !same_identity(&current, expected)
                || current.kind != CodingGitOperationKind::PointerPublish
            {
                return Err(identity_mismatch(&id));
            }
            if current.phase == CodingGitOperationPhase::Completed {
                if current.push_status.as_ref() == Some(&input.push_status)
                    && current.remote_kind.as_ref() == Some(&input.remote_kind)
                    && current.review_request_id.as_deref()
                        == Some(input.review_request_id.as_str())
                {
                    return Ok(current);
                }
                return Err(identity_mismatch(&id));
            }
            if current.phase != CodingGitOperationPhase::PushStarted || current.commit_sha.is_none()
            {
                return Err(identity_mismatch(&id));
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

    /// 重推专用重开：仅当 journal 为 `Completed(Failed)` 时重开为 `PushStarted`
    /// （保留 `commit_sha`，清空 push 完成态字段）。红线同 attempt 版：
    /// `Completed(Pushed)` 与其他非 Completed 相位绝不重开。
    pub fn reopen_failed_pointer_publish_git_operation(
        &self,
        publication: &PointerPublication,
        member_repo_id: &str,
        expected: &CodingGitOperationJournal,
    ) -> Result<CodingGitOperationJournal, ProductStoreError> {
        validate_pointer_publication_ids(publication, member_repo_id)?;
        let id = pointer_journal_id(&publication.id, member_repo_id);
        let path = self.pointer_publication_git_operation_path(
            &publication.project_id,
            &publication.id,
            &id,
        );

        with_exclusive_lock(&path, || {
            if !path.is_file() {
                return Err(identity_mismatch(&id));
            }
            let mut current: CodingGitOperationJournal = read_json(&path)?;
            validate_pointer_journal(&current, publication, member_repo_id)?;
            if !same_identity(&current, expected)
                || current.kind != CodingGitOperationKind::PointerPublish
                || current.phase != CodingGitOperationPhase::Completed
                || current.push_status != Some(PushStatus::Failed)
                || current.commit_sha.is_none()
            {
                return Err(identity_mismatch(&id));
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
        CodingGitOperationKind::PointerPublish => {
            // PointerPublish 不绑定 attempt；attempt 方法族不得构造该 kind。
            return Err(identity_mismatch(&attempt.id));
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

/// PointerPublish journal 的合成稳定标识（与 ReviewRequest.attempt_id 命名空间一致），
/// 含成员仓维度：每仓一个独立 journal，避免单文件退化为 last-repo-wins。
fn pointer_journal_id(publication_id: &str, member_repo_id: &str) -> String {
    format!("pointer-pub-{publication_id}-{member_repo_id}")
}

fn validate_pointer_publication_ids(
    publication: &PointerPublication,
    member_repo_id: &str,
) -> Result<(), ProductStoreError> {
    validate_relative_id(&publication.id)?;
    validate_relative_id(&publication.project_id)?;
    validate_relative_id(member_repo_id)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_pointer_journal(
    publication: &PointerPublication,
    member_repo_id: &str,
    repo_path: &Path,
    worktree_path: &Path,
    branch_name: &str,
    base_branch: &str,
    remote: &str,
    commit_message: &str,
) -> Result<CodingGitOperationJournal, ProductStoreError> {
    let id = pointer_journal_id(&publication.id, member_repo_id);
    validate_non_empty("branch", branch_name, &id)?;
    validate_non_empty("base branch", base_branch, &id)?;
    validate_non_empty("remote", remote, &id)?;
    validate_non_empty("commit message", commit_message, &id)?;

    let now = Utc::now().to_rfc3339();
    let journal = CodingGitOperationJournal {
        project_id: publication.project_id.clone(),
        issue_id: String::new(),
        attempt_id: id,
        kind: CodingGitOperationKind::PointerPublish,
        repo_path: canonical_path_identity(repo_path)?,
        worktree_path: canonical_path_identity(worktree_path)?,
        branch_name: branch_name.to_string(),
        base_branch: base_branch.to_string(),
        before_head: String::new(),
        remote: Some(remote.to_string()),
        commit_message: Some(commit_message.to_string()),
        phase: CodingGitOperationPhase::Before,
        commit_sha: None,
        push_status: None,
        push_error: None,
        remote_kind: None,
        review_request_id: None,
        created_at: now.clone(),
        updated_at: now,
    };
    validate_pointer_journal(&journal, publication, member_repo_id)?;
    Ok(journal)
}

/// PointerPublish kind 的校验：按 publication 校验 project_id / attempt_id，
/// 要求 issue_id 为空、分支名符合 `aria-pointer/{member_repo_id}/{publication_id}`、
/// 该 member_repo_id 存在于 publication.entries、branch/base_branch/remote/commit_message
/// 非空、路径 canonical 一致；不要求 attempt 各字段相等。
fn validate_pointer_journal(
    journal: &CodingGitOperationJournal,
    publication: &PointerPublication,
    member_repo_id: &str,
) -> Result<(), ProductStoreError> {
    let id = pointer_journal_id(&publication.id, member_repo_id);
    let expected_branch = format!("aria-pointer/{member_repo_id}/{}", publication.id);
    let member_belongs = publication
        .entries
        .iter()
        .any(|entry| entry.member_repo_id == member_repo_id);
    if journal.project_id != publication.project_id
        || journal.attempt_id != id
        || journal.kind != CodingGitOperationKind::PointerPublish
        || !journal.issue_id.is_empty()
        || journal.branch_name != expected_branch
        || !member_belongs
        || canonical_path_identity(&journal.repo_path)? != journal.repo_path
        || canonical_path_identity(&journal.worktree_path)? != journal.worktree_path
    {
        return Err(identity_mismatch(&id));
    }
    validate_non_empty("branch", &journal.branch_name, &id)?;
    validate_non_empty("base branch", &journal.base_branch, &id)?;
    validate_non_empty("created at", &journal.created_at, &id)?;
    validate_non_empty("updated at", &journal.updated_at, &id)?;
    let Some(remote) = journal.remote.as_deref() else {
        return Err(identity_mismatch(&id));
    };
    let Some(message) = journal.commit_message.as_deref() else {
        return Err(identity_mismatch(&id));
    };
    validate_non_empty("remote", remote, &id)?;
    validate_non_empty("commit message", message, &id)?;
    if let Some(review_request_id) = journal.review_request_id.as_deref() {
        validate_relative_id(review_request_id)?;
    }
    validate_phase_state(journal)
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
        CodingGitOperationKind::PointerPublish => {
            // PointerPublish 不绑定 attempt；必须走 validate_pointer_journal。
            return Err(identity_mismatch(&attempt.id));
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
        CodingGitOperationKind::ReviewRequest | CodingGitOperationKind::PointerPublish => {
            review_like_phase_state_valid(journal)
        }
    };
    if valid {
        Ok(())
    } else {
        Err(identity_mismatch(&journal.attempt_id))
    }
}

fn review_like_phase_state_valid(journal: &CodingGitOperationJournal) -> bool {
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
        CodingGitOperationKind::ReviewRequest | CodingGitOperationKind::PointerPublish => {
            matches!(
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
            )
        }
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
