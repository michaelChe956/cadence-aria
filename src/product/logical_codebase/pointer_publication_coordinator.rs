//! `PointerPublishCoordinator`：每仓发布流水编排 + 按仓重试 + revoke 补偿。
//!
//! 契约 REQ-ENV-07（独立 worktree/branch 受控发布、ReviewRequest 非自动 PR、可回滚、
//! partial 呈现）。每仓流水严格按设计 §5.3：
//!
//! 成员 checkout HEAD → 临时 worktree（publish 专用，不进 coding worktree 注册表）→
//! `classify_merge`（Append 则写指针文件；Skip/Conflict 直接落条目，无分支）→
//! journal start → branch/commit → git_push → complete_review → 写 ReviewRequest
//! （owner_kind=PointerPublication）→ 条目 `ReviewCreated`；单仓失败不阻断其他仓。
//!
//! revoke 按设计 §5.4 补偿矩阵：逐已推条目 `delete_remote_branch`（幂等）→
//! ReviewRequest 标记 Revoked → 条目 Revoked → publication Revoked；删除失败
//! `pointer_revoke_failed` 可重试；标记失败后重试只补标记（删除幂等）。

use std::path::PathBuf;

use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CodingGitOperationJournal, CodingGitOperationPhase,
    CompleteReviewGitOperationInput,
};
use crate::product::coding_models::{
    PushStatus, RemoteKind, ReviewRequest, ReviewRequestKind, ReviewRequestOwnerKind,
};
use crate::product::git_workspace_service::{GitWorkspaceError, GitWorkspaceService};
use crate::product::json_store::{ProductStoreError, read_json};
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest,
    LogicalCodebaseStore, LogicalRepositoryId, PointerBlockFields, PointerMergeVerdict,
    PointerPublication, PointerPublicationBatchKind, PointerPublicationEntry,
    PointerPublicationEntryState, PointerPublicationStatus, PointerPublicationStore, apply_append,
    classify_merge, render_pointer_block,
};

/// 指针文件在成员仓根目录的文件名（隐藏文件，命名空间隔离）。
pub const POINTER_FILE_NAME: &str = ".aria-pointer.md";

/// 指针块版本，与设计 §5.1 模板一致。
const POINTER_VERSION: u32 = 1;

/// 推送远端名。成员 checkout 必须配置 `origin` 才能 push/delete。
const REMOTE_NAME: &str = "origin";

/// 每仓 ReviewRequest 的合成稳定 id（每仓一条，可覆盖更新）。
fn pointer_review_request_id(publication_id: &str, member_repo_id: &str) -> String {
    format!("rr-{publication_id}-{member_repo_id}")
}

#[derive(Debug, thiserror::Error)]
pub enum PointerPublishError {
    #[error("pointer_publish_busy: {0}")]
    Busy(String),
    #[error("pointer_not_found: {0}")]
    NotFound(String),
    #[error("pointer_conflict_unresolved: {0}")]
    ConflictUnresolved(String),
    /// 防御性变体：coordinator 自身不直接产出 `PushFailed`（push 失败统一落
    /// `Failed` 条目 + `push_error`），保留该变体供 web 映射层与未来调用方使用。
    #[error("pointer_push_failed: {0}")]
    PushFailed(String),
    #[error("pointer_revoke_failed: {0}")]
    RevokeFailed(String),
    #[error("pointer_validation: {0}")]
    Validation(String),
    #[error("pointer_store: {0}")]
    Store(#[from] ProductStoreError),
    /// 防御性变体：底层 git 失败统一经 `record_failed` 落条目而非向上传播，
    /// 保留该变体供 `#[from]` 转换与 web 映射层（`pointer_push_failed`）使用。
    #[error("pointer_git: {0}")]
    Git(#[from] GitWorkspaceError),
}

pub struct PointerPublishCoordinator {
    publications: PointerPublicationStore,
    logical: LogicalCodebaseStore,
    git_ops: CodingAttemptStore,
    git: GitWorkspaceService,
}

impl PointerPublishCoordinator {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self {
            publications: PointerPublicationStore::new(paths.clone()),
            logical: LogicalCodebaseStore::new(paths.clone()),
            git_ops: CodingAttemptStore::new(paths),
            git: GitWorkspaceService::new(),
        }
    }

    /// Scopes publication records and manifest/member/checkout reads to one
    /// logical codebase subtree (the legacy alias keeps the legacy root).
    pub fn for_lc(paths: ProductAppPaths, lc_id: impl Into<String>) -> Self {
        let lc_id = lc_id.into();
        Self {
            publications: PointerPublicationStore::for_lc(paths.clone(), lc_id.clone()),
            logical: LogicalCodebaseStore::for_lc(paths.clone(), lc_id),
            git_ops: CodingAttemptStore::new(paths),
            git: GitWorkspaceService::new(),
        }
    }

    /// 全量/增量批次发布。`logical_codebase_id` 必须与 project 的 manifest 一致。
    /// 单仓失败不阻断其他仓；整体按 CompletedAll / CompletedPartial 呈现。
    pub async fn publish_all(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        batch_kind: PointerPublicationBatchKind,
    ) -> Result<PointerPublication, PointerPublishError> {
        let manifest = self.load_manifest(project_id, logical_codebase_id)?;
        let members = self.logical.list_members(project_id)?;
        let target_member_ids =
            self.select_target_members(project_id, logical_codebase_id, batch_kind, &members)?;

        let now = chrono::Utc::now().to_rfc3339();
        let publication = PointerPublication {
            id: Uuid::new_v4().to_string(),
            project_id: project_id.to_string(),
            logical_codebase_id: logical_codebase_id.to_string(),
            batch_kind,
            entries: target_member_ids
                .iter()
                .map(|member_repo_id| PointerPublicationEntry {
                    member_repo_id: member_repo_id.clone(),
                    state: PointerPublicationEntryState::Pending,
                    branch_name: None,
                    commit_sha: None,
                    push_error: None,
                    conflict_detail: None,
                })
                .collect(),
            status: PointerPublicationStatus::InProgress,
            created_at: now.clone(),
            updated_at: now,
        };
        let publication = self
            .publications
            .create_publication(publication)
            .map_err(|error| match error {
                ProductStoreError::Conflict { kind, id } => {
                    PointerPublishError::Busy(format!("{kind}:{id}"))
                }
                other => PointerPublishError::Store(other),
            })?;

        let mut publication = publication;
        for member_repo_id in &target_member_ids {
            publication = self
                .publish_member(&publication, &manifest, member_repo_id)
                .await?;
        }
        self.finalize(publication)
    }

    /// 按仓重试。接受全部非终态条目状态（Pending/Committed/Pushed/Failed/Conflict），
    /// 终态（Skipped/ReviewCreated/Revoked）不可重试。Conflict 未解决 →
    /// `pointer_conflict_unresolved`；远端已有分支（Pushed，或 push 成功后写
    /// ReviewRequest 失败的 Failed 带分支）→ 只补 ReviewRequest 不再 push；
    /// 其余非终态 → 复位 Pending 并清旧 journal/本地分支/worktree 后全量重跑。
    pub async fn retry_member_repo(
        &self,
        project_id: &str,
        publication_id: &str,
        member_repo_id: &str,
    ) -> Result<PointerPublication, PointerPublishError> {
        let publication = self
            .publications
            .load_publication(project_id, publication_id)
            .map_err(pointer_store_not_found)?;
        let manifest = self.load_manifest(project_id, &publication.logical_codebase_id)?;
        let entry = publication
            .entries
            .iter()
            .find(|entry| entry.member_repo_id == member_repo_id)
            .ok_or_else(|| {
                PointerPublishError::NotFound(format!(
                    "publication {publication_id} has no entry for {member_repo_id}"
                ))
            })?;

        match entry.state {
            PointerPublicationEntryState::Conflict => {
                // 人工是否已解决冲突：重新 classify 当前文件。
                let still_conflict = self
                    .classify_member(&manifest, member_repo_id)?
                    .is_conflict();
                if still_conflict {
                    return Err(PointerPublishError::ConflictUnresolved(format!(
                        "member {member_repo_id} pointer block conflict is not resolved"
                    )));
                }
            }
            PointerPublicationEntryState::Skipped
            | PointerPublicationEntryState::ReviewCreated
            | PointerPublicationEntryState::Revoked => {
                return Err(PointerPublishError::Validation(format!(
                    "entry {member_repo_id} is {:?} and cannot be retried",
                    entry.state
                )));
            }
            PointerPublicationEntryState::Pending
            | PointerPublicationEntryState::Committed
            | PointerPublicationEntryState::Pushed
            | PointerPublicationEntryState::Failed => {}
        }

        self.ensure_no_other_in_progress(
            project_id,
            &publication.logical_codebase_id,
            publication_id,
        )?;

        // 远端是否已有该分支：Pushed 条目，或 journal 已 Completed(Pushed) 的
        // Committed/Failed 条目 → 只补 ReviewRequest，不再 push。Committed 覆盖
        // journal 完成与 publication 条目落盘之间的崩溃窗口。
        let already_pushed = entry.state == PointerPublicationEntryState::Pushed
            || (matches!(
                entry.state,
                PointerPublicationEntryState::Committed | PointerPublicationEntryState::Failed
            ) && self.member_pushed_to_remote(&publication, member_repo_id));

        // 放回 InProgress（重新占用发布锁）。
        let mut publication = publication;
        publication.status = PointerPublicationStatus::InProgress;
        publication.updated_at = chrono::Utc::now().to_rfc3339();
        self.publications.save_publication(&publication)?;

        let publication = if already_pushed {
            let publication = self.publications.reset_entry_for_retry(
                project_id,
                publication_id,
                member_repo_id,
                PointerPublicationEntryState::Pushed,
            )?;
            self.finish_review_request_member(&publication, member_repo_id)
                .await?
        } else {
            // 干净重跑：复位 Pending + 清旧 journal/本地分支/worktree，保证流水幂等。
            let publication = self.publications.reset_entry_for_retry(
                project_id,
                publication_id,
                member_repo_id,
                PointerPublicationEntryState::Pending,
            )?;
            self.cleanup_member_artifacts(&publication, member_repo_id)
                .await?;
            self.publish_member(&publication, &manifest, member_repo_id)
                .await?
        };
        self.finalize(publication)
    }

    /// 撤回发布：删远端分支（每个已推条目）→ ReviewRequest 标记 Revoked →
    /// 条目 Revoked + publication Revoked。重复 revoke 幂等返回当前态。
    pub async fn revoke(
        &self,
        project_id: &str,
        publication_id: &str,
    ) -> Result<PointerPublication, PointerPublishError> {
        let publication = self
            .publications
            .load_publication(project_id, publication_id)
            .map_err(pointer_store_not_found)?;
        if publication.status == PointerPublicationStatus::Revoked {
            return Ok(publication);
        }

        // Phase 1：逐已推条目删除远端分支（幂等）。失败 → 可重试，不标记任何内容。
        // Failed 但已记录分支（push 成功后写 ReviewRequest 失败的孤儿分支）也一并回收。
        for entry in publication.entries.iter().filter(|entry| {
            matches!(
                entry.state,
                PointerPublicationEntryState::Pushed | PointerPublicationEntryState::ReviewCreated
            ) || (entry.state == PointerPublicationEntryState::Failed
                && entry.branch_name.is_some())
        }) {
            let branch_name = entry.branch_name.clone().ok_or_else(|| {
                PointerPublishError::RevokeFailed(format!(
                    "entry {} has no branch to delete",
                    entry.member_repo_id
                ))
            })?;
            let repo_path = self
                .resolve_member_repo_path(project_id, &entry.member_repo_id)
                .map_err(|error| PointerPublishError::RevokeFailed(error.to_string()))?;
            self.git
                .delete_remote_branch(&repo_path, REMOTE_NAME, &branch_name)
                .await
                .map_err(|error| {
                    PointerPublishError::RevokeFailed(format!(
                        "delete remote branch {branch_name} for {}: {error}",
                        entry.member_repo_id
                    ))
                })?;
        }

        // Phase 2：标记 ReviewRequest Revoked。删除幂等，重试只补标记；标记写失败
        // 统一归一为 `pointer_revoke_failed`（503），publication 保持可重试态。
        let mut requests = self
            .git_ops
            .list_pointer_review_requests(project_id, publication_id)
            .map_err(|error| {
                PointerPublishError::RevokeFailed(format!(
                    "list review requests for {publication_id}: {error}"
                ))
            })?;
        for entry in publication.entries.iter().filter(|entry| {
            matches!(
                entry.state,
                PointerPublicationEntryState::Pushed | PointerPublicationEntryState::ReviewCreated
            )
        }) {
            let request_id = pointer_review_request_id(publication_id, &entry.member_repo_id);
            match requests.iter_mut().find(|request| request.id == request_id) {
                Some(request) => {
                    request.revoked = true;
                    request.updated_at = chrono::Utc::now().to_rfc3339();
                    self.git_ops
                        .save_pointer_review_request(project_id, publication_id, request)
                        .map_err(|error| {
                            PointerPublishError::RevokeFailed(format!(
                                "mark review request {request_id} revoked: {error}"
                            ))
                        })?;
                }
                None => {
                    tracing::warn!(
                        publication_id,
                        member_repo_id = %entry.member_repo_id,
                        "review request missing during revoke; skipping mark"
                    );
                }
            }
        }

        // Phase 3：批次置 Revoked（全部条目 Revoked）。
        self.publications
            .mark_revoked(project_id, publication_id)
            .map_err(PointerPublishError::from)
    }

    /// 单成员流水。始终返回推进后的 publication；本仓失败只落 Failed 条目，不整体失败。
    async fn publish_member(
        &self,
        publication: &PointerPublication,
        manifest: &LogicalCodebaseManifest,
        member_repo_id: &str,
    ) -> Result<PointerPublication, PointerPublishError> {
        let repo_path = match self.resolve_member_repo_path(&publication.project_id, member_repo_id)
        {
            Ok(path) => path,
            Err(error) => {
                return self
                    .publications
                    .record_entry_outcome(
                        &publication.project_id,
                        &publication.id,
                        member_repo_id,
                        PointerPublicationEntryState::Failed,
                        None,
                        None,
                        Some(error.to_string()),
                        None,
                    )
                    .map_err(PointerPublishError::from);
            }
        };

        let block = self.render_block(manifest, member_repo_id);
        let existing =
            std::fs::read_to_string(repo_path.join(POINTER_FILE_NAME)).unwrap_or_default();
        match classify_merge(&existing, &block) {
            PointerMergeVerdict::Skip => {
                return self
                    .publications
                    .record_entry_outcome(
                        &publication.project_id,
                        &publication.id,
                        member_repo_id,
                        PointerPublicationEntryState::Skipped,
                        None,
                        None,
                        None,
                        None,
                    )
                    .map_err(PointerPublishError::from);
            }
            PointerMergeVerdict::Conflict { summary } => {
                return self
                    .publications
                    .record_entry_outcome(
                        &publication.project_id,
                        &publication.id,
                        member_repo_id,
                        PointerPublicationEntryState::Conflict,
                        None,
                        None,
                        None,
                        Some(summary),
                    )
                    .map_err(PointerPublishError::from);
            }
            PointerMergeVerdict::Append => {}
        }

        let new_content = apply_append(&existing, &block);
        let branch_name = format!("aria-pointer/{member_repo_id}/{}", publication.id);
        let worktree_path = repo_path
            .join(".worktrees/aria-pointer")
            .join(member_repo_id)
            .join(&publication.id);
        let commit_message =
            format!("feat(pointer): publish logical codebase pointer for {member_repo_id}");

        // journal start（幂等；崩溃恢复时以读回值为准，不沿用内存旧相位）。
        let journal = match self.git_ops.start_pointer_publish_git_operation(
            publication,
            member_repo_id,
            &repo_path,
            &worktree_path,
            &branch_name,
            "HEAD",
            REMOTE_NAME,
            &commit_message,
        ) {
            Ok(journal) => journal,
            Err(error) => {
                return self.record_failed(publication, member_repo_id, error.to_string());
            }
        };

        if let Err(error) = self
            .git
            .create_branch(&repo_path, &branch_name, "HEAD")
            .await
        {
            self.compensate_journal(publication, member_repo_id, &journal)
                .await;
            return self.record_failed(publication, member_repo_id, error.to_string());
        }
        if let Err(error) = self
            .git
            .create_worktree(&repo_path, &branch_name, &worktree_path)
            .await
        {
            self.compensate_journal(publication, member_repo_id, &journal)
                .await;
            return self.record_failed(publication, member_repo_id, error.to_string());
        }
        if let Err(error) = std::fs::write(worktree_path.join(POINTER_FILE_NAME), &new_content) {
            self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
                .await;
            self.compensate_journal(publication, member_repo_id, &journal)
                .await;
            return self.record_failed(publication, member_repo_id, error.to_string());
        }

        let journal = match self.git_ops.advance_pointer_publish_git_operation(
            publication,
            member_repo_id,
            &journal,
            CodingGitOperationPhase::CommitStarted,
            None,
        ) {
            Ok(journal) => journal,
            Err(error) => {
                self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
                    .await;
                return self.record_failed(publication, member_repo_id, error.to_string());
            }
        };

        if let Err(error) = self.git.git_add_all(&worktree_path).await {
            self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
                .await;
            self.compensate_journal(publication, member_repo_id, &journal)
                .await;
            return self.record_failed(publication, member_repo_id, error.to_string());
        }
        let commit = match self.git.git_commit(&worktree_path, &commit_message).await {
            Ok(commit) => commit,
            Err(error) => {
                self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
                    .await;
                self.compensate_journal(publication, member_repo_id, &journal)
                    .await;
                return self.record_failed(publication, member_repo_id, error.to_string());
            }
        };

        // 记录 Committed（带分支与 commit sha）。
        let publication = self
            .publications
            .record_entry_outcome(
                &publication.project_id,
                &publication.id,
                member_repo_id,
                PointerPublicationEntryState::Committed,
                Some(branch_name.clone()),
                Some(commit.commit_sha.clone()),
                None,
                None,
            )
            .map_err(PointerPublishError::from)?;

        let journal = match self.git_ops.advance_pointer_publish_git_operation(
            &publication,
            member_repo_id,
            &journal,
            CodingGitOperationPhase::CommitCreated,
            Some(commit.commit_sha.clone()),
        ) {
            Ok(journal) => journal,
            Err(error) => {
                self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
                    .await;
                return self.record_failed(&publication, member_repo_id, error.to_string());
            }
        };
        let journal = match self.git_ops.advance_pointer_publish_git_operation(
            &publication,
            member_repo_id,
            &journal,
            CodingGitOperationPhase::PushStarted,
            None,
        ) {
            Ok(journal) => journal,
            Err(error) => {
                self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
                    .await;
                return self.record_failed(&publication, member_repo_id, error.to_string());
            }
        };

        let push = match self
            .git
            .git_push(&worktree_path, REMOTE_NAME, &branch_name)
            .await
        {
            Ok(push) => push,
            Err(error) => {
                self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
                    .await;
                self.compensate_journal(&publication, member_repo_id, &journal)
                    .await;
                return self.record_failed(&publication, member_repo_id, error.to_string());
            }
        };

        let remote_kind = self
            .git
            .detect_remote_kind(&repo_path)
            .await
            .unwrap_or(RemoteKind::Unknown);
        let review_request_id = pointer_review_request_id(&publication.id, member_repo_id);

        if push.status == PushStatus::Pushed {
            if let Err(error) = self.git_ops.complete_review_pointer_publish_git_operation(
                &publication,
                member_repo_id,
                &journal,
                CompleteReviewGitOperationInput {
                    push_status: PushStatus::Pushed,
                    remote_kind: remote_kind.clone(),
                    review_request_id: review_request_id.clone(),
                    push_error: None,
                },
            ) {
                self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
                    .await;
                return self.record_failed(&publication, member_repo_id, error.to_string());
            }

            // 条目 Pushed（push 成功 + journal 完成），再写 ReviewRequest。
            let publication = self
                .publications
                .record_entry_outcome(
                    &publication.project_id,
                    &publication.id,
                    member_repo_id,
                    PointerPublicationEntryState::Pushed,
                    Some(branch_name.clone()),
                    Some(commit.commit_sha.clone()),
                    None,
                    None,
                )
                .map_err(PointerPublishError::from)?;

            self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
                .await;
            return self
                .finish_review_request_member(&publication, member_repo_id)
                .await;
        } else {
            if let Err(error) = self.git_ops.complete_review_pointer_publish_git_operation(
                &publication,
                member_repo_id,
                &journal,
                CompleteReviewGitOperationInput {
                    push_status: PushStatus::Failed,
                    remote_kind,
                    review_request_id: review_request_id.clone(),
                    push_error: push.stderr.clone(),
                },
            ) {
                self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
                    .await;
                return self.record_failed(&publication, member_repo_id, error.to_string());
            }

            self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
                .await;
            self.publications
                .record_entry_outcome(
                    &publication.project_id,
                    &publication.id,
                    member_repo_id,
                    PointerPublicationEntryState::Failed,
                    Some(branch_name),
                    Some(commit.commit_sha),
                    push.stderr,
                    None,
                )
                .map_err(PointerPublishError::from)
        }
    }

    /// 远端分支已存在（push 成功）：只补写 ReviewRequest 并把条目推进到
    /// `ReviewCreated`。写失败时条目落 `Failed` 且保留 branch/commit（不再清空），
    /// 供 revoke 回收远端孤儿分支与再次重试。
    async fn finish_review_request_member(
        &self,
        publication: &PointerPublication,
        member_repo_id: &str,
    ) -> Result<PointerPublication, PointerPublishError> {
        let entry = publication
            .entries
            .iter()
            .find(|entry| entry.member_repo_id == member_repo_id)
            .ok_or_else(|| {
                PointerPublishError::NotFound(format!(
                    "publication {} has no entry for {member_repo_id}",
                    publication.id
                ))
            })?;
        let branch_name = entry.branch_name.clone().ok_or_else(|| {
            PointerPublishError::Validation(format!(
                "entry {member_repo_id} has no branch to finish"
            ))
        })?;
        let commit_sha = entry.commit_sha.clone().ok_or_else(|| {
            PointerPublishError::Validation(format!(
                "entry {member_repo_id} has no commit to finish"
            ))
        })?;
        let repo_path = self.resolve_member_repo_path(&publication.project_id, member_repo_id)?;

        let remote_kind = self
            .git
            .detect_remote_kind(&repo_path)
            .await
            .unwrap_or(RemoteKind::Unknown);
        let review_request_id = pointer_review_request_id(&publication.id, member_repo_id);
        let now = chrono::Utc::now().to_rfc3339();
        let request = ReviewRequest {
            id: review_request_id,
            attempt_id: format!("pointer-pub-{}", publication.id),
            kind: ReviewRequestKind::GitBranchOnly,
            remote_kind,
            remote: REMOTE_NAME.to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: branch_name.clone(),
            commit_sha: commit_sha.clone(),
            push_status: PushStatus::Pushed,
            external_url: None,
            manual_instructions: vec![
                "指针标记块已推送远端分支；请在成员仓发起人工代码审查（非自动 PR）".to_string(),
            ],
            push_error: None,
            owner_kind: ReviewRequestOwnerKind::PointerPublication,
            pointer_publication_id: Some(publication.id.clone()),
            revoked: false,
            created_at: now.clone(),
            updated_at: now,
        };
        if let Err(error) = self.git_ops.save_pointer_review_request(
            &publication.project_id,
            &publication.id,
            &request,
        ) {
            // 保留分支/commit：条目 Failed 携带 branch/commit，revoke 可回收远端孤儿分支。
            return self
                .publications
                .record_entry_outcome(
                    &publication.project_id,
                    &publication.id,
                    member_repo_id,
                    PointerPublicationEntryState::Failed,
                    Some(branch_name),
                    Some(commit_sha),
                    Some(error.to_string()),
                    None,
                )
                .map_err(PointerPublishError::from);
        }
        self.publications
            .record_entry_outcome(
                &publication.project_id,
                &publication.id,
                member_repo_id,
                PointerPublicationEntryState::ReviewCreated,
                Some(branch_name),
                Some(commit_sha),
                None,
                None,
            )
            .map_err(PointerPublishError::from)
    }

    /// 判断远端是否已有该成员的分支：读 journal 的 `Completed(Pushed)`。
    /// 用于区分「push 成功后写 ReviewRequest 失败」（远端有分支）与「push 失败」
    /// （远端无分支），决定重试是补 ReviewRequest 还是全量重跑。
    fn member_pushed_to_remote(
        &self,
        publication: &PointerPublication,
        member_repo_id: &str,
    ) -> bool {
        let journal_id = format!("pointer-pub-{}-{}", publication.id, member_repo_id);
        let path = self.git_ops.pointer_publication_git_operation_path(
            &publication.project_id,
            &publication.id,
            &journal_id,
        );
        let Ok(journal) = read_json::<CodingGitOperationJournal>(&path) else {
            return false;
        };
        journal.phase == CodingGitOperationPhase::Completed
            && journal.push_status == Some(PushStatus::Pushed)
    }

    /// 删除该仓旧 journal 与本地分支/worktree，保证全量重跑时流水幂等（不残留
    /// 上次崩溃的中间产物）。仅用于远端无分支的非终态重试；未 push 的本地分支可安全删除。
    async fn cleanup_member_artifacts(
        &self,
        publication: &PointerPublication,
        member_repo_id: &str,
    ) -> Result<(), PointerPublishError> {
        let journal_path = self.git_ops.pointer_publication_git_operation_path(
            &publication.project_id,
            &publication.id,
            &format!("pointer-pub-{}-{}", publication.id, member_repo_id),
        );
        if journal_path.exists() {
            std::fs::remove_file(&journal_path).map_err(|error| {
                ProductStoreError::Io(format!("remove {}: {error}", journal_path.display()))
            })?;
        }
        let repo_path = self.resolve_member_repo_path(&publication.project_id, member_repo_id)?;
        let branch_name = format!("aria-pointer/{member_repo_id}/{}", publication.id);
        let worktree_path = repo_path
            .join(".worktrees/aria-pointer")
            .join(member_repo_id)
            .join(&publication.id);
        self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
            .await;
        Ok(())
    }

    fn record_failed(
        &self,
        publication: &PointerPublication,
        member_repo_id: &str,
        error: String,
    ) -> Result<PointerPublication, PointerPublishError> {
        self.publications
            .record_entry_outcome(
                &publication.project_id,
                &publication.id,
                member_repo_id,
                PointerPublicationEntryState::Failed,
                None,
                None,
                Some(error),
                None,
            )
            .map_err(PointerPublishError::from)
    }

    async fn compensate_journal(
        &self,
        publication: &PointerPublication,
        member_repo_id: &str,
        journal: &crate::product::coding_attempt_store::CodingGitOperationJournal,
    ) {
        let _ = self.git_ops.advance_pointer_publish_git_operation(
            publication,
            member_repo_id,
            journal,
            CodingGitOperationPhase::Compensated,
            None,
        );
    }

    async fn cleanup_worktree(
        &self,
        repo_path: &std::path::Path,
        worktree_path: &std::path::Path,
        branch_name: &str,
    ) {
        let _ = self.git.remove_worktree(repo_path, worktree_path).await;
        let _ = self.git.delete_local_branch(repo_path, branch_name).await;
    }

    fn finalize(
        &self,
        mut publication: PointerPublication,
    ) -> Result<PointerPublication, PointerPublishError> {
        let has_failure = publication.entries.iter().any(|entry| {
            matches!(
                entry.state,
                PointerPublicationEntryState::Failed | PointerPublicationEntryState::Conflict
            )
        });
        publication.status = if has_failure {
            PointerPublicationStatus::CompletedPartial
        } else {
            PointerPublicationStatus::CompletedAll
        };
        publication.updated_at = chrono::Utc::now().to_rfc3339();
        self.publications.save_publication(&publication)?;
        Ok(publication)
    }

    fn select_target_members(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        batch_kind: PointerPublicationBatchKind,
        members: &[CodebaseMemberRecord],
    ) -> Result<Vec<String>, PointerPublishError> {
        let all: Vec<String> = members
            .iter()
            .map(|member| member.logical_repository_id.0.to_string())
            .collect();
        match batch_kind {
            PointerPublicationBatchKind::Full => {
                if all.is_empty() {
                    return Err(PointerPublishError::Validation(
                        "logical codebase has no members to publish".to_string(),
                    ));
                }
                Ok(all)
            }
            PointerPublicationBatchKind::Incremental => {
                // 比对所有非 Revoked 历史批次条目并集（不是只比最新一条），只发未发布过的新成员。
                let already: std::collections::HashSet<String> = self
                    .publications
                    .list_publications(project_id)?
                    .into_iter()
                    .filter(|publication| {
                        publication.logical_codebase_id == logical_codebase_id
                            && publication.status != PointerPublicationStatus::Revoked
                    })
                    .flat_map(|publication| publication.entries)
                    .map(|entry| entry.member_repo_id)
                    .collect();
                let new_members: Vec<String> = all
                    .into_iter()
                    .filter(|member_repo_id| !already.contains(member_repo_id))
                    .collect();
                if new_members.is_empty() {
                    return Err(PointerPublishError::Validation(
                        "no new members to publish incrementally".to_string(),
                    ));
                }
                Ok(new_members)
            }
        }
    }

    fn load_manifest(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<LogicalCodebaseManifest, PointerPublishError> {
        let manifest = self.logical.load_manifest(project_id)?.ok_or_else(|| {
            PointerPublishError::NotFound(format!(
                "logical codebase manifest missing for {project_id}"
            ))
        })?;
        if manifest.logical_codebase_id.to_string() != logical_codebase_id {
            return Err(PointerPublishError::Validation(format!(
                "logical_codebase_id {logical_codebase_id} does not match manifest {}",
                manifest.logical_codebase_id
            )));
        }
        Ok(manifest)
    }

    fn resolve_member_repo_path(
        &self,
        project_id: &str,
        member_repo_id: &str,
    ) -> Result<PathBuf, PointerPublishError> {
        let id = Uuid::parse_str(member_repo_id).map_err(|error| {
            PointerPublishError::Validation(format!("invalid member repo id: {error}"))
        })?;
        let member = self
            .logical
            .load_member(project_id, LogicalRepositoryId(id))?
            .ok_or_else(|| {
                PointerPublishError::NotFound(format!("member {member_repo_id} missing"))
            })?;
        for checkout_id in &member.checkout_ids {
            if let Some(checkout) = self.logical.load_checkout(project_id, *checkout_id)?
                && checkout.kind == CheckoutKind::Main
                && checkout.availability == CheckoutAvailability::Available
            {
                return Ok(checkout.canonical_path);
            }
        }
        Err(PointerPublishError::Validation(format!(
            "member {member_repo_id} has no available main checkout"
        )))
    }

    fn render_block(&self, manifest: &LogicalCodebaseManifest, member_repo_id: &str) -> String {
        render_pointer_block(&PointerBlockFields {
            logical_codebase_id: manifest.logical_codebase_id.to_string(),
            repo_id: member_repo_id.to_string(),
            canonical_policy_locator: manifest
                .provider_context_root
                .to_string_lossy()
                .into_owned(),
            pointer_version: POINTER_VERSION,
        })
    }

    fn classify_member(
        &self,
        manifest: &LogicalCodebaseManifest,
        member_repo_id: &str,
    ) -> Result<PointerMergeVerdict, PointerPublishError> {
        let repo_path =
            self.resolve_member_repo_path(manifest.project_id.as_str(), member_repo_id)?;
        let existing =
            std::fs::read_to_string(repo_path.join(POINTER_FILE_NAME)).unwrap_or_default();
        Ok(classify_merge(
            &existing,
            &self.render_block(manifest, member_repo_id),
        ))
    }

    fn ensure_no_other_in_progress(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        exclude_publication_id: &str,
    ) -> Result<(), PointerPublishError> {
        for publication in self.publications.list_publications(project_id)? {
            if publication.id != exclude_publication_id
                && publication.logical_codebase_id == logical_codebase_id
                && publication.status == PointerPublicationStatus::InProgress
            {
                return Err(PointerPublishError::Busy(format!(
                    "another publication {} is InProgress for {logical_codebase_id}",
                    publication.id
                )));
            }
        }
        Ok(())
    }
}

trait PointerMergeVerdictExt {
    fn is_conflict(&self) -> bool;
}

impl PointerMergeVerdictExt for PointerMergeVerdict {
    fn is_conflict(&self) -> bool {
        matches!(self, PointerMergeVerdict::Conflict { .. })
    }
}

fn pointer_store_not_found(error: ProductStoreError) -> PointerPublishError {
    match error {
        ProductStoreError::NotFound { kind, id } => {
            PointerPublishError::NotFound(format!("{kind}:{id}"))
        }
        other => PointerPublishError::Store(other),
    }
}

// 测试模块按仓库惯例拆入 `.inc.rs`,保持主文件低于 large_file_guard 的 1200 行上限。
include!("pointer_publication_coordinator_tests.inc.rs");
