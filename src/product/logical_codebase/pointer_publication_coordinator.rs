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
    CodingAttemptStore, CodingGitOperationPhase, CompleteReviewGitOperationInput,
};
use crate::product::coding_models::{
    PushStatus, RemoteKind, ReviewRequest, ReviewRequestKind, ReviewRequestOwnerKind,
};
use crate::product::git_workspace_service::{GitWorkspaceError, GitWorkspaceService};
use crate::product::json_store::ProductStoreError;
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
    #[error("pointer_push_failed: {0}")]
    PushFailed(String),
    #[error("pointer_revoke_failed: {0}")]
    RevokeFailed(String),
    #[error("pointer_validation: {0}")]
    Validation(String),
    #[error("pointer_store: {0}")]
    Store(#[from] ProductStoreError),
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

    /// 按仓重试。Conflict 未解决 → `pointer_conflict_unresolved`；Failed → 全量重跑；
    /// 其余状态不可重试。重试前把批次放回 InProgress（重新占用发布锁）并把条目
    /// 复位到 Pending，再删除该仓的旧 journal 强制全新流水。
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
            PointerPublicationEntryState::Failed => {}
            other => {
                return Err(PointerPublishError::Validation(format!(
                    "entry {member_repo_id} is {other:?} and cannot be retried"
                )));
            }
        }

        self.ensure_no_other_in_progress(
            project_id,
            &publication.logical_codebase_id,
            publication_id,
        )?;

        // 放回 InProgress 并复位条目到 Pending，再删除旧 journal 强制全新流水。
        let mut publication = publication;
        publication.status = PointerPublicationStatus::InProgress;
        publication.updated_at = chrono::Utc::now().to_rfc3339();
        self.publications.save_publication(&publication)?;
        let publication = self.publications.advance_entry_state(
            project_id,
            publication_id,
            member_repo_id,
            PointerPublicationEntryState::Pending,
        )?;

        let journal_path = self.git_ops.pointer_publication_git_operation_path(
            project_id,
            publication_id,
            &format!("pointer-pub-{publication_id}-{member_repo_id}"),
        );
        if journal_path.exists() {
            std::fs::remove_file(&journal_path).map_err(|error| {
                ProductStoreError::Io(format!("remove {}: {error}", journal_path.display()))
            })?;
        }

        let publication = self
            .publish_member(&publication, &manifest, member_repo_id)
            .await?;
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
        for entry in publication.entries.iter().filter(|entry| {
            matches!(
                entry.state,
                PointerPublicationEntryState::Pushed | PointerPublicationEntryState::ReviewCreated
            )
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

        // Phase 2：标记 ReviewRequest Revoked。删除幂等，重试只补标记。
        for entry in publication.entries.iter().filter(|entry| {
            matches!(
                entry.state,
                PointerPublicationEntryState::Pushed | PointerPublicationEntryState::ReviewCreated
            )
        }) {
            let request_id = pointer_review_request_id(publication_id, &entry.member_repo_id);
            let mut requests = self
                .git_ops
                .list_pointer_review_requests(project_id, publication_id)?;
            if let Some(request) = requests.iter_mut().find(|request| request.id == request_id) {
                request.revoked = true;
                request.updated_at = chrono::Utc::now().to_rfc3339();
                self.git_ops
                    .save_pointer_review_request(project_id, publication_id, request)?;
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
            let completed = match self.git_ops.complete_review_pointer_publish_git_operation(
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
                Ok(completed) => completed,
                Err(error) => {
                    self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
                        .await;
                    return self.record_failed(&publication, member_repo_id, error.to_string());
                }
            };
            let _ = completed;

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

            let now = chrono::Utc::now().to_rfc3339();
            let request = ReviewRequest {
                id: review_request_id,
                attempt_id: format!("pointer-pub-{}", publication.id),
                kind: ReviewRequestKind::GitBranchOnly,
                remote_kind,
                remote: REMOTE_NAME.to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: branch_name.clone(),
                commit_sha: commit.commit_sha.clone(),
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
                    PointerPublicationEntryState::ReviewCreated,
                    Some(branch_name),
                    Some(commit.commit_sha),
                    None,
                    None,
                )
                .map_err(PointerPublishError::from)
        } else {
            let completed = match self.git_ops.complete_review_pointer_publish_git_operation(
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
                Ok(completed) => completed,
                Err(error) => {
                    self.cleanup_worktree(&repo_path, &worktree_path, &branch_name)
                        .await;
                    return self.record_failed(&publication, member_repo_id, error.to_string());
                }
            };
            let _ = completed;

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
                // 比对既有 publication 条目集合（最新批次），只发新增成员。
                let already: std::collections::HashSet<String> = self
                    .publications
                    .list_publications(project_id)?
                    .into_iter()
                    .filter(|publication| publication.logical_codebase_id == logical_codebase_id)
                    .max_by_key(|publication| publication.created_at.clone())
                    .map(|publication| {
                        publication
                            .entries
                            .into_iter()
                            .map(|entry| entry.member_repo_id)
                            .collect()
                    })
                    .unwrap_or_default();
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest,
        LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord,
        RepositorySourceIdentity, RepositoryType,
    };
    use std::path::{Path, PathBuf};
    use std::process::Command as StdCommand;
    use tempfile::TempDir;

    const PROJECT_ID: &str = "project_0001";

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

    fn git_allow_failure(repo: &Path, args: &[&str]) -> (bool, String) {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git fixture command");
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).to_string(),
        )
    }

    struct MemberRepo {
        logical_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
        repo_path: PathBuf,
        bare_remote: Option<PathBuf>,
    }

    impl MemberRepo {
        fn member_record(&self) -> CodebaseMemberRecord {
            let now = "2026-08-14T00:00:00Z".to_string();
            CodebaseMemberRecord {
                logical_repository_id: self.logical_id,
                physical_repository_id: format!("repo_{}", self.logical_id.0),
                alias: format!("member_{}", self.logical_id.0.simple()),
                role: "service".to_string(),
                ordinal: 1,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &self.repo_path,
                    self.repo_path.join(".git"),
                    Some(format!(
                        "ssh://git@example.test/acme/{}.git",
                        self.logical_id.0
                    )),
                ),
                repo_type: RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![self.checkout_id],
                status: MemberStatus::Active,
                created_at: now.clone(),
                updated_at: now,
            }
        }

        fn checkout_record(&self) -> RepositoryCheckoutRecord {
            let now = "2026-08-14T00:00:00Z".to_string();
            RepositoryCheckoutRecord {
                checkout_id: self.checkout_id,
                logical_repository_id: self.logical_id,
                physical_repository_id: format!("repo_{}", self.logical_id.0),
                kind: CheckoutKind::Main,
                canonical_path: self.repo_path.clone(),
                checkout_path_hash: format!("sha256:{}", self.logical_id.0),
                git_dir_identity: format!("sha256:git-{}", self.logical_id.0),
                revision: None,
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            }
        }
    }

    struct Fixture {
        tmp: TempDir,
        coordinator: PointerPublishCoordinator,
        logical_codebase_id: String,
        members: Vec<MemberRepo>,
    }

    fn setup_member(tmp: &Path, name: &str, with_origin: bool) -> MemberRepo {
        let logical_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let repo_path = tmp.join(name);
        std::fs::create_dir_all(&repo_path).unwrap();
        git(&repo_path, &["init"]);
        git(&repo_path, &["config", "user.email", "test@example.com"]);
        git(&repo_path, &["config", "user.name", "Test User"]);
        std::fs::write(repo_path.join("README.md"), "base\n").unwrap();
        git(&repo_path, &["add", "README.md"]);
        git(&repo_path, &["commit", "-m", "base"]);

        let bare_remote = if with_origin {
            let remote_path = tmp.join(format!("{name}-origin.git"));
            std::fs::create_dir_all(&remote_path).unwrap();
            git(&remote_path, &["init", "--bare"]);
            git(
                &repo_path,
                &["remote", "add", "origin", remote_path.to_str().unwrap()],
            );
            git(&repo_path, &["push", "-u", "origin", "master"]);
            git(&repo_path, &["branch", "-m", "main"]);
            git(&repo_path, &["push", "-u", "origin", "main"]);
            Some(remote_path)
        } else {
            None
        };

        MemberRepo {
            logical_id,
            checkout_id,
            repo_path,
            bare_remote,
        }
    }

    fn setup(member_specs: &[(&str, bool)]) -> Fixture {
        let tmp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let members: Vec<MemberRepo> = member_specs
            .iter()
            .map(|(name, with_origin)| setup_member(tmp.path(), name, *with_origin))
            .collect();

        let aggregate_root = tmp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let manifest = LogicalCodebaseManifest::new(
            PROJECT_ID,
            aggregate_root,
            members.iter().map(|m| m.logical_id).collect(),
        );
        let logical_codebase_id = manifest.logical_codebase_id.to_string();
        let store = LogicalCodebaseStore::new(paths.clone());
        store.save_manifest(PROJECT_ID, &manifest).unwrap();
        for member in &members {
            store
                .save_member(PROJECT_ID, &member.member_record())
                .unwrap();
            store
                .save_checkout(PROJECT_ID, &member.checkout_record())
                .unwrap();
        }

        Fixture {
            tmp,
            coordinator: PointerPublishCoordinator::new(paths),
            logical_codebase_id,
            members,
        }
    }

    fn remote_has_branch(bare: &Path, branch: &str) -> bool {
        let (success, stdout) = git_allow_failure(
            bare,
            &["show-ref", "--verify", &format!("refs/heads/{branch}")],
        );
        success && !stdout.trim().is_empty()
    }

    fn entry<'a>(
        publication: &'a PointerPublication,
        member_repo_id: &str,
    ) -> &'a PointerPublicationEntry {
        publication
            .entries
            .iter()
            .find(|entry| entry.member_repo_id == member_repo_id)
            .expect("entry")
    }

    #[tokio::test]
    async fn publish_all_full_batch_pushes_all_members_and_writes_review_requests() {
        let fixture = setup(&[("api", true), ("worker", true)]);
        let publication = fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Full,
            )
            .await
            .expect("publish");

        assert_eq!(publication.status, PointerPublicationStatus::CompletedAll);
        assert_eq!(publication.entries.len(), 2);
        for member in &fixture.members {
            let member_repo_id = member.logical_id.0.to_string();
            let entry = entry(&publication, &member_repo_id);
            assert_eq!(entry.state, PointerPublicationEntryState::ReviewCreated);
            assert!(entry.branch_name.is_some());
            assert!(entry.commit_sha.is_some());

            let bare = member.bare_remote.as_ref().expect("bare");
            let branch = entry.branch_name.as_deref().unwrap();
            assert!(
                remote_has_branch(bare, branch),
                "remote branch {branch} must exist"
            );

            // 指针文件在主 checkout 未被污染（写入发生在临时 worktree）
            assert!(!member.repo_path.join(POINTER_FILE_NAME).exists());

            // ReviewRequest 落盘在 pointer-publications 分区
            let requests = fixture
                .coordinator
                .git_ops
                .list_pointer_review_requests(PROJECT_ID, &publication.id)
                .unwrap();
            assert_eq!(requests.len(), 2);
            let request = requests
                .iter()
                .find(|request| request.id == format!("rr-{}-{}", publication.id, member_repo_id))
                .expect("review request");
            assert_eq!(
                request.owner_kind,
                ReviewRequestOwnerKind::PointerPublication
            );
            assert_eq!(
                request.attempt_id,
                format!("pointer-pub-{}", publication.id)
            );
            assert!(!request.revoked);
        }
    }

    #[tokio::test]
    async fn publish_all_single_member_push_failure_yields_completed_partial() {
        let fixture = setup(&[("no-remote", false), ("with-remote", true)]);
        let publication = fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Full,
            )
            .await
            .expect("publish");

        assert_eq!(
            publication.status,
            PointerPublicationStatus::CompletedPartial
        );
        let failed_id = fixture.members[0].logical_id.0.to_string();
        let failed = entry(&publication, &failed_id);
        assert_eq!(failed.state, PointerPublicationEntryState::Failed);
        assert!(failed.push_error.is_some());

        let ok_id = fixture.members[1].logical_id.0.to_string();
        assert_eq!(
            entry(&publication, &ok_id).state,
            PointerPublicationEntryState::ReviewCreated
        );
    }

    #[tokio::test]
    async fn incremental_publish_only_creates_entries_for_new_members() {
        let fixture = setup(&[("api", true)]);
        fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Full,
            )
            .await
            .expect("full publish");

        // 新增第二个成员
        let new_member = setup_member(fixture.tmp.path(), "worker", true);
        let store =
            LogicalCodebaseStore::new(ProductAppPaths::new(fixture.tmp.path().join(".aria")));
        store
            .save_member(PROJECT_ID, &new_member.member_record())
            .unwrap();
        store
            .save_checkout(PROJECT_ID, &new_member.checkout_record())
            .unwrap();
        let mut manifest = store.load_manifest(PROJECT_ID).unwrap().unwrap();
        manifest.member_ids.push(new_member.logical_id);
        manifest.membership_revision += 1;
        store.save_manifest(PROJECT_ID, &manifest).unwrap();

        let publication = fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Incremental,
            )
            .await
            .expect("incremental publish");

        assert_eq!(publication.status, PointerPublicationStatus::CompletedAll);
        assert_eq!(publication.entries.len(), 1);
        let new_id = new_member.logical_id.0.to_string();
        assert_eq!(
            entry(&publication, &new_id).state,
            PointerPublicationEntryState::ReviewCreated
        );
        assert!(remote_has_branch(
            new_member.bare_remote.as_ref().unwrap(),
            entry(&publication, &new_id).branch_name.as_deref().unwrap()
        ));
    }

    #[tokio::test]
    async fn conflict_entry_then_retry_unresolved_blocks_until_fixed() {
        let fixture = setup(&[("api", true)]);
        // 预置冲突指针块（不同 logical_codebase_id）
        std::fs::write(
            fixture.members[0].repo_path.join(POINTER_FILE_NAME),
            "<!-- aria-logical-codebase-pointer:start\n  logical_codebase_id: other\n  repo_id: other\n  canonical_policy_locator: /other\n  声明：未加载集中政策前禁止写；本块仅用于发现，不作为政策正文\n  pointer_version: 1\naria-logical-codebase-pointer:end -->\n",
        )
        .unwrap();

        let publication = fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Full,
            )
            .await
            .expect("publish");
        assert_eq!(
            publication.status,
            PointerPublicationStatus::CompletedPartial
        );
        let member_repo_id = fixture.members[0].logical_id.0.to_string();
        assert_eq!(
            entry(&publication, &member_repo_id).state,
            PointerPublicationEntryState::Conflict
        );

        // 冲突未解决 → 409
        let error = fixture
            .coordinator
            .retry_member_repo(PROJECT_ID, &publication.id, &member_repo_id)
            .await
            .expect_err("conflict must block retry");
        assert!(matches!(error, PointerPublishError::ConflictUnresolved(_)));

        // 人工解决：删除冲突块
        std::fs::remove_file(fixture.members[0].repo_path.join(POINTER_FILE_NAME)).unwrap();
        let retried = fixture
            .coordinator
            .retry_member_repo(PROJECT_ID, &publication.id, &member_repo_id)
            .await
            .expect("retry after resolve");
        assert_eq!(retried.status, PointerPublicationStatus::CompletedAll);
        assert_eq!(
            entry(&retried, &member_repo_id).state,
            PointerPublicationEntryState::ReviewCreated
        );
    }

    #[tokio::test]
    async fn revoke_deletes_remote_branches_marks_requests_and_is_idempotent() {
        let fixture = setup(&[("api", true), ("worker", true)]);
        let publication = fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Full,
            )
            .await
            .expect("publish");

        let revoked = fixture
            .coordinator
            .revoke(PROJECT_ID, &publication.id)
            .await
            .expect("revoke");
        assert_eq!(revoked.status, PointerPublicationStatus::Revoked);
        for member in &fixture.members {
            let member_repo_id = member.logical_id.0.to_string();
            let entry = entry(&revoked, &member_repo_id);
            assert_eq!(entry.state, PointerPublicationEntryState::Revoked);
            let branch = entry.branch_name.as_deref().unwrap();
            assert!(
                !remote_has_branch(member.bare_remote.as_ref().unwrap(), branch),
                "remote branch {branch} must be deleted"
            );
        }

        let requests = fixture
            .coordinator
            .git_ops
            .list_pointer_review_requests(PROJECT_ID, &publication.id)
            .unwrap();
        assert!(requests.iter().all(|request| request.revoked));

        // 重复 revoke 幂等
        let again = fixture
            .coordinator
            .revoke(PROJECT_ID, &publication.id)
            .await
            .expect("repeat revoke");
        assert_eq!(again.status, PointerPublicationStatus::Revoked);
    }

    #[tokio::test]
    async fn revoke_delete_failure_returns_revoke_failed_and_keeps_entries() {
        let fixture = setup(&[("api", true)]);
        let publication = fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Full,
            )
            .await
            .expect("publish");

        // 移除 origin，使删除远端分支失败（origin 不存在 ≠ 远端 ref 不存在）
        git(
            &fixture.members[0].repo_path,
            &["remote", "remove", "origin"],
        );

        let error = fixture
            .coordinator
            .revoke(PROJECT_ID, &publication.id)
            .await
            .expect_err("revoke must fail");
        assert!(matches!(error, PointerPublishError::RevokeFailed(_)));

        let after = fixture
            .coordinator
            .publications
            .load_publication(PROJECT_ID, &publication.id)
            .unwrap();
        assert_eq!(after.status, PointerPublicationStatus::CompletedAll);
        let member_repo_id = fixture.members[0].logical_id.0.to_string();
        assert_eq!(
            entry(&after, &member_repo_id).state,
            PointerPublicationEntryState::ReviewCreated
        );
    }
}
