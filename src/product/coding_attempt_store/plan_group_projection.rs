//! REQ-MTG-04（WP3）：plan 级 group 聚合只读投影。
//!
//! 三值终态判据（约束 9 定案，k3 F4 可测化）：
//! - **已达 provider 启动** = attempt 离开初始二元组（`StartCoding` 是唯一
//!   入口，离开必经 `admit_and_transition_attempt_to_executable`）；
//!   `(Aborted, PrepareContext)` **双形态消歧**：pre-start abort（无执行
//!   证据）未达（k3 fix round 1）；多 unit group attempt 在 unit 间推进时
//!   `advance_to_next_group_unit` 把 stage 回退 PrepareContext，其后 abort
//!   落盘同形态但已执行过 provider——取该 attempt 的 role_runs 非空或
//!   head_commit 在场为执行铁证，证据在场即算已达（k3 fix round 2）；
//! - **未启**（判定优先于「部分」）：无 target-attempt，或全部 target-attempt
//!   均未达 provider 启动（`(Created, PrepareContext)` 未触碰形态或无执行
//!   证据的 pre-start abort 形态）；
//! - **全部交付**：存在 target-attempt 且每 target 最新 attempt
//!   `Completed` 且最新 ReviewRequest `Pushed`——对齐
//!   `compute_issue_delivery_summary` 口径（issue_delivery.rs:110-113）；
//! - **部分**：其余一切（MUST NOT 伪装全局成功）。
//!
//! 只读派生（无第二状态机）：每次读取从 attempt/ReviewRequest durable 事实
//! 现算，不落任何持久化聚合记录。无快照存量 attempt 不进 per-target 投影
//! （D2.1 A4——按 issue 级 attempt 列表既有形态呈现，聚合视图不猜测归属）。

use std::collections::BTreeMap;

use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage, PushStatus, ReviewRequest,
};
use crate::product::issue_store::IssueStore;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;

/// plan 级 group 聚合终态三值。
///
/// 与 issue 级 `IssueDeliveryOverall` 的口径差异（k3 §2.3）：plan 级「未启」
/// ≠ issue 级 `None`（无条目）——plan 级未启=无 target-attempt 或全部未达
/// provider 启动（未启优先于「部分」），故 DTO 序列化不复用 `"none"`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanGroupOverall {
    /// 每 target 最新 attempt `Completed` 且最新 ReviewRequest `Pushed`。
    AllDelivered,
    /// 任一 target 已达 provider 启动且非全部交付（含「部分启动+部分未启」）。
    Partial,
    /// 无 target-attempt，或全部 target-attempt 均未离开 `(Created, PrepareContext)`。
    NotStarted,
}

/// 单个 target 的只读投影条目（按最新 target-attempt 派生）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanTargetEntry {
    pub target_repository_id: LogicalRepositoryId,
    /// 展示名（`resolve_repository_name` 同款解析：logical id 经 strict 解析取
    /// checkout 路径末段目录名，解析不出末段回落 id 字符串）。
    pub repository_name: String,
    pub attempt_id: Option<String>,
    pub attempt_status: Option<CodingAttemptStatus>,
    pub stage: Option<CodingExecutionStage>,
    pub branch_name: Option<String>,
    pub head_commit: Option<String>,
    /// 取自最新 attempt 的最新 ReviewRequest；`None` 表示无 ReviewRequest。
    pub push_status: Option<PushStatus>,
    pub review_request_id: Option<String>,
    /// 失败/阻塞原因（只呈现不判定，OQ3 定案派生规则）：
    /// `manual_recovery_reason` 优先 → `push_error` → 失败态 status 文本。
    pub blocked_reason: Option<String>,
}

/// plan 级 group 聚合只读投影结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanGroupProjection {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub entries: Vec<PlanTargetEntry>,
    pub overall: PlanGroupOverall,
}

/// `blocked_reason` 派生规则（OQ3 定案，只呈现不判定）：
/// `manual_recovery_reason` 优先 → 最新 ReviewRequest 的 `push_error` →
/// 失败态 status 文本（`Failed`/`Aborted`/`AmendmentApplyFailed`，与 DTO
/// status 文本一致的 snake_case）。
fn plan_target_blocked_reason(
    attempt: &CodingExecutionAttempt,
    latest_review: Option<&ReviewRequest>,
) -> Option<String> {
    if let Some(reason) = attempt.manual_recovery_reason.as_ref() {
        return Some(reason.clone());
    }
    if let Some(error) = latest_review.and_then(|review| review.push_error.as_ref()) {
        return Some(error.clone());
    }
    match attempt.status {
        CodingAttemptStatus::Failed => Some("failed".to_string()),
        CodingAttemptStatus::Aborted => Some("aborted".to_string()),
        CodingAttemptStatus::AmendmentApplyFailed => Some("amendment_apply_failed".to_string()),
        _ => None,
    }
}

impl super::CodingAttemptStore {
    /// 计算某个 plan 的 group 级聚合只读投影（REQ-MTG-04，无写入）。
    ///
    /// 判定语义（约束 9 定案）：每 target 取最新 attempt（按 `attempt_no`
    /// 升序末元素，与 `compute_issue_delivery_summary` 的最新口径一致）；
    /// `status == Completed` 且最新 ReviewRequest `push_status == Pushed`
    /// 才算该 target 已交付。三值判定见模块文档。
    pub fn compute_plan_group_projection(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<PlanGroupProjection, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        // 同 compute_issue_delivery_summary：先读 issue 确认存在，避免对不存在
        // 的 issue 返回空投影。
        IssueStore::new(self.paths()).get(project_id, issue_id)?;

        let attempts = self.list_attempts_for_work_item_group(project_id, issue_id, plan_id)?;

        // D2.1 A4：无快照 attempt 不入任何 target 桶——聚合投影不猜测归属。
        let mut by_target: BTreeMap<LogicalRepositoryId, Vec<&CodingExecutionAttempt>> =
            BTreeMap::new();
        for attempt in &attempts {
            if let Some(snapshot) = attempt.target_snapshot.as_ref() {
                by_target
                    .entry(snapshot.logical_repository_id)
                    .or_default()
                    .push(attempt);
            }
        }

        let mut entries = Vec::with_capacity(by_target.len());
        let mut all_delivered = true;
        let mut any_target_started = false;
        for (target, target_attempts) in &by_target {
            // list_attempts_for_work_item_group 已按 (attempt_no, id) 升序——末元素为最新。
            let latest = target_attempts
                .last()
                .expect("target bucket is never empty");
            // 已达 provider 启动 = 该 target 任一 attempt 已达（历史事实口径，
            // 判据见 attempt_reached_provider——k3 fix round 1/2 消歧版）。
            let mut target_started = false;
            for attempt in target_attempts {
                if self.attempt_reached_provider(project_id, issue_id, attempt)? {
                    target_started = true;
                    break;
                }
            }
            let latest_review = self
                .list_review_requests(project_id, issue_id, &latest.id)?
                .into_iter()
                .last();
            let delivered = latest.status == CodingAttemptStatus::Completed
                && latest_review
                    .as_ref()
                    .is_some_and(|review| review.push_status == PushStatus::Pushed);
            all_delivered = all_delivered && delivered;
            any_target_started = any_target_started || target_started;

            let repository_name = self.resolve_plan_target_repository_name(project_id, *target)?;
            entries.push(PlanTargetEntry {
                target_repository_id: *target,
                repository_name,
                attempt_id: Some(latest.id.clone()),
                attempt_status: Some(latest.status.clone()),
                stage: Some(latest.stage.clone()),
                branch_name: Some(latest.branch_name.clone()),
                head_commit: latest.head_commit.clone(),
                push_status: latest_review
                    .as_ref()
                    .map(|review| review.push_status.clone()),
                review_request_id: latest_review.as_ref().map(|review| review.id.clone()),
                blocked_reason: plan_target_blocked_reason(latest, latest_review.as_ref()),
            });
        }

        let overall = if entries.is_empty() {
            PlanGroupOverall::NotStarted
        } else if all_delivered {
            PlanGroupOverall::AllDelivered
        } else if !any_target_started {
            // 未启优先于部分（约束 9）：全部 target-attempt 均未离开初始二元组。
            PlanGroupOverall::NotStarted
        } else {
            PlanGroupOverall::Partial
        };

        Ok(PlanGroupProjection {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            plan_id: plan_id.to_string(),
            entries,
            overall,
        })
    }

    /// 该 attempt 是否已达 provider 启动（k3 fix round 1/2 消歧版判据）。
    ///
    /// - stage 已离开 `PrepareContext` → 已达（含启动后 abort）；
    /// - `(Created, PrepareContext)` → 未达（未触碰，`StartCoding` 唯一入口）；
    /// - 其余 PrepareContext 停留态（非 Created 非 Aborted）→ 已达：非 Created
    ///   状态均经 admission，PrepareContext 停留只可能来自 unit 间 stage 回退；
    /// - `(Aborted, PrepareContext)` **双形态消歧**（fix round 1 排除 pre-start
    ///   abort；fix round 2 修正误排）：pre-start abort（Created→Aborted 白名单
    ///   转换、abort 不改 stage、无执行证据）未达；多 unit group attempt 在
    ///   unit 间推进时 `advance_to_next_group_unit` 会把 stage 回退
    ///   PrepareContext（coding_workspace_engine/group.rs:315），其后 abort
    ///   落盘同形态但已执行过 provider——取 durable 执行证据消歧：该 attempt
    ///   的 role_runs 非空或 head_commit 在场即算已达（只读派生面，两者均在
    ///   attempt record/子目录可读）。
    fn attempt_reached_provider(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt: &CodingExecutionAttempt,
    ) -> Result<bool, ProductStoreError> {
        if attempt.stage != CodingExecutionStage::PrepareContext {
            return Ok(true);
        }
        match attempt.status {
            CodingAttemptStatus::Created => Ok(false),
            CodingAttemptStatus::Aborted => Ok(!self
                .list_role_runs(project_id, issue_id, &attempt.id)?
                .is_empty()
                || attempt.head_commit.is_some()),
            _ => Ok(true),
        }
    }

    /// 解析 plan target 的仓展示名（issue_delivery.rs `resolve_repository_name`
    /// 同款）：logical id 经 `resolve_logical_repository_strict` 取 checkout 路径
    /// 末段目录名；解析不出末段时回落 logical id 字符串本身。
    fn resolve_plan_target_repository_name(
        &self,
        project_id: &str,
        target: LogicalRepositoryId,
    ) -> Result<String, ProductStoreError> {
        let paths = self.paths();
        let project = ProjectStore::new(paths.clone()).get(project_id)?;
        let (_, checkout, _) = RepositoryStore::for_project(paths, &project)
            .resolve_logical_repository_strict(project_id, target)?;
        if let Some(name) = checkout
            .canonical_path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .filter(|value| !value.is_empty())
        {
            return Ok(name);
        }
        Ok(target.0.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;
    use uuid::Uuid;

    use super::{PlanGroupOverall, PlanGroupProjection};
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::{CodingAttemptStore, CreateGroupCodingAttemptInput};
    use crate::product::coding_models::{
        AttemptTargetSnapshot, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
        CodingProviderRole, CodingRoleRunTrigger, PushStatus, RemoteKind, ReviewRequest,
        ReviewRequestKind, ReviewRequestOwnerKind,
    };
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::json_store::write_json;
    use crate::product::lifecycle_store::{CreateWorkItemInput, LifecycleStore};
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest,
        LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
        RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::models::{ProviderName, RepositoryRecord};
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const WORK_ITEM_ID: &str = "work_item_0001";
    const PLAN_ID: &str = "work_item_plan_0001";

    fn setup_store() -> (TempDir, CodingAttemptStore) {
        let tmp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: PROJECT_ID.to_string(),
                repo_id: Some("repository_0001".to_string()),
                logical_codebase_id: None,
                title: "issue".to_string(),
                description: None,
                change_id: None,
            })
            .unwrap();
        let store = CodingAttemptStore::new(paths);
        (tmp, store)
    }

    fn provider_snapshot() -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        }
    }

    /// 播种双 target logical codebase 权威记录（manifest+members+checkouts+
    /// repos.json 兼容投影），使 `resolve_logical_repository_strict` 解析到
    /// `checkout_repo_alpha` / `checkout_repo_beta` 展示名。
    fn seed_two_targets(
        root: &Path,
        store: &CodingAttemptStore,
    ) -> (LogicalRepositoryId, LogicalRepositoryId) {
        let alpha = LogicalRepositoryId(Uuid::new_v4());
        let beta = LogicalRepositoryId(Uuid::new_v4());
        let now = "2026-09-19T00:00:00Z".to_string();
        let mut repositories = Vec::new();
        let authority = LogicalCodebaseStore::new(store.paths());
        authority
            .save_manifest(
                PROJECT_ID,
                &LogicalCodebaseManifest::new(
                    PROJECT_ID,
                    root.join("aggregate-root"),
                    vec![alpha, beta],
                ),
            )
            .unwrap();
        for (index, (logical_id, checkout_name)) in
            [(alpha, "checkout_repo_alpha"), (beta, "checkout_repo_beta")]
                .into_iter()
                .enumerate()
        {
            let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
            let repository_path = root.join(checkout_name);
            let source_identity = RepositorySourceIdentity::from_git_parts(
                &repository_path,
                repository_path.join(".git"),
                None,
            );
            authority
                .save_member(
                    PROJECT_ID,
                    &CodebaseMemberRecord {
                        logical_repository_id: logical_id,
                        physical_repository_id: format!("physical_{checkout_name}"),
                        alias: checkout_name.to_string(),
                        role: "repository".to_string(),
                        ordinal: (index + 1) as u32,
                        source_identity: source_identity.clone(),
                        repo_type: RepositoryType::Unknown,
                        tech_stack: Vec::new(),
                        owner: None,
                        tags: Vec::new(),
                        default_ref: None,
                        checkout_ids: vec![checkout_id],
                        status: MemberStatus::Active,
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .unwrap();
            authority
                .save_checkout(
                    PROJECT_ID,
                    &RepositoryCheckoutRecord {
                        checkout_id,
                        logical_repository_id: logical_id,
                        physical_repository_id: format!("physical_{checkout_name}"),
                        kind: CheckoutKind::Main,
                        canonical_path: repository_path.clone(),
                        checkout_path_hash: format!("sha256:{checkout_name}"),
                        git_dir_identity: source_identity.git_dir_identity(),
                        revision: Some("abcdef".to_string()),
                        availability: CheckoutAvailability::Available,
                        observed_at: now.clone(),
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .unwrap();
            repositories.push(RepositoryRecord {
                id: format!("physical_{checkout_name}"),
                project_id: PROJECT_ID.to_string(),
                name: checkout_name.to_string(),
                path: repository_path,
                repo_hash: format!("sha256:{checkout_name}"),
                runtime_root: root.join(checkout_name).join(".aria/runtime"),
                default_policy_preset: "manual-write".to_string(),
                default_provider_mode: "fake".to_string(),
                created_at: now.clone(),
                logical_repository_id: Some(logical_id),
                primary_checkout_id: Some(checkout_id),
                identity_schema_version: 1,
                updated_at: now.clone(),
            });
        }
        write_json(
            &store.paths().project_root(PROJECT_ID).join("repos.json"),
            &repositories,
        )
        .unwrap();
        (alpha, beta)
    }

    fn target_snapshot(logical_id: LogicalRepositoryId) -> AttemptTargetSnapshot {
        AttemptTargetSnapshot {
            logical_repository_id: logical_id,
            checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
            physical_repository_id: "physical_split".to_string(),
            canonical_path: std::path::PathBuf::from("/split/repository"),
            git_dir_identity: "split-git-dir".to_string(),
            revision: Some("split-revision".to_string()),
            policy_digest: "split-policy".to_string(),
            membership_revision: 1,
            captured_at: "2026-09-19T00:00:00Z".to_string(),
            capture_source: "test".to_string(),
        }
    }

    fn group_input(
        plan_id: &str,
        target: Option<LogicalRepositoryId>,
        branch_name: &str,
    ) -> CreateGroupCodingAttemptInput {
        CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: plan_id.to_string(),
            current_work_item_id: WORK_ITEM_ID.to_string(),
            base_branch: "main".to_string(),
            branch_name: branch_name.to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            target_snapshot: target.map(target_snapshot),
            max_auto_rework: 2,
        }
    }

    fn seed_group_attempt(
        store: &CodingAttemptStore,
        target: Option<LogicalRepositoryId>,
        branch_name: &str,
    ) -> CodingExecutionAttempt {
        store
            .create_group_attempt(group_input(PLAN_ID, target, branch_name))
            .expect("seed target attempt")
    }

    /// 直改 attempt 状态/stage 落盘（播种非 Created 形态；测试不经守卫）。
    fn force_attempt_state(
        store: &CodingAttemptStore,
        attempt: &CodingExecutionAttempt,
        status: CodingAttemptStatus,
        stage: CodingExecutionStage,
        manual_recovery_reason: Option<String>,
    ) -> CodingExecutionAttempt {
        let mut updated = attempt.clone();
        let is_completed = status == CodingAttemptStatus::Completed;
        updated.status = status;
        updated.stage = stage;
        updated.manual_recovery_reason = manual_recovery_reason;
        updated.updated_at = "2026-09-19T00:01:00Z".to_string();
        if is_completed {
            updated.completed_at = Some("2026-09-19T00:01:00Z".to_string());
            updated.head_commit = Some("sha999".to_string());
        }
        store.write_coding_attempt_for_test(&updated).unwrap();
        updated
    }

    /// 同 target 追加第二个 attempt（attempt_no 更大，绕过建组守卫直接落盘）——
    /// 模拟 per-(plan,target) 增殖后的 durable 事实，验证「取最新」口径。
    fn extra_attempt_for_target(
        store: &CodingAttemptStore,
        base: &CodingExecutionAttempt,
        attempt_no: u32,
        status: CodingAttemptStatus,
        stage: CodingExecutionStage,
    ) -> CodingExecutionAttempt {
        let mut extra = base.clone();
        extra.id = format!("coding_attempt_extra_{attempt_no}");
        extra.attempt_no = attempt_no;
        let is_completed = status == CodingAttemptStatus::Completed;
        extra.status = status;
        extra.stage = stage;
        extra.created_at = "2026-09-19T00:02:00Z".to_string();
        extra.updated_at = "2026-09-19T00:02:00Z".to_string();
        if is_completed {
            extra.completed_at = Some("2026-09-19T00:02:00Z".to_string());
            extra.head_commit = Some("sha888".to_string());
        }
        store.write_coding_attempt_for_test(&extra).unwrap();
        extra
    }

    fn seed_review_request(
        store: &CodingAttemptStore,
        attempt: &CodingExecutionAttempt,
        push_status: PushStatus,
        push_error: Option<String>,
    ) {
        let request = ReviewRequest {
            id: format!("review_request_{}", Uuid::new_v4().simple()),
            attempt_id: attempt.id.clone(),
            kind: ReviewRequestKind::GitBranchOnly,
            remote_kind: RemoteKind::GenericGit,
            remote: "origin".to_string(),
            base_branch: "main".to_string(),
            branch_name: attempt.branch_name.clone(),
            commit_sha: attempt.head_commit.clone().unwrap_or_default(),
            push_status,
            external_url: None,
            manual_instructions: Vec::new(),
            push_error,
            owner_kind: ReviewRequestOwnerKind::Attempt,
            pointer_publication_id: None,
            revoked: false,
            created_at: "2026-09-19T00:00:00Z".to_string(),
            updated_at: "2026-09-19T00:00:00Z".to_string(),
        };
        store.save_review_request(attempt, &request).unwrap();
    }

    fn durable_inventory(root: &Path) -> BTreeMap<PathBuf, (u128, u64)> {
        let mut inventory = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(directory) = stack.pop() {
            for entry in std::fs::read_dir(&directory).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let metadata = entry.metadata().unwrap();
                if metadata.is_dir() {
                    stack.push(path);
                } else {
                    let modified = metadata.modified().unwrap();
                    let since_epoch = modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos();
                    inventory.insert(path, (since_epoch, metadata.len()));
                }
            }
        }
        inventory
    }

    fn entry_for(
        projection: &PlanGroupProjection,
        target: LogicalRepositoryId,
    ) -> &super::PlanTargetEntry {
        projection
            .entries
            .iter()
            .find(|entry| entry.target_repository_id == target)
            .unwrap_or_else(|| panic!("missing projection entry for target {target:?}"))
    }

    // ── 1) 全部交付：两 target 均 Completed+Pushed+ReviewRequest 在案 ──

    #[test]
    fn all_delivered_when_every_target_latest_attempt_completed_and_pushed() {
        let (tmp, store) = setup_store();
        let (alpha, beta) = seed_two_targets(tmp.path(), &store);
        let attempt_a = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let attempt_b = seed_group_attempt(&store, Some(beta), "aria/issues/i/web");
        let attempt_a = force_attempt_state(
            &store,
            &attempt_a,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
            None,
        );
        let attempt_b = force_attempt_state(
            &store,
            &attempt_b,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
            None,
        );
        seed_review_request(&store, &attempt_a, PushStatus::Pushed, None);
        seed_review_request(&store, &attempt_b, PushStatus::Pushed, None);

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(projection.overall, PlanGroupOverall::AllDelivered);
        assert_eq!(projection.entries.len(), 2);
        assert_eq!(projection.plan_id, PLAN_ID);
        for (target, attempt, name) in [
            (alpha, &attempt_a, "checkout_repo_alpha"),
            (beta, &attempt_b, "checkout_repo_beta"),
        ] {
            let entry = entry_for(&projection, target);
            assert_eq!(entry.repository_name, name);
            assert_eq!(entry.attempt_id.as_deref(), Some(attempt.id.as_str()));
            assert_eq!(entry.attempt_status, Some(CodingAttemptStatus::Completed));
            assert_eq!(entry.push_status, Some(PushStatus::Pushed));
            assert!(entry.review_request_id.is_some(), "ReviewRequest 在案");
            assert_eq!(entry.blocked_reason, None);
        }
    }

    // ── 2) partial failure 三负向：不伪装全局成功 + 未满足项显式 ──

    #[test]
    fn partial_failure_unpushed_completed_attempt_is_partial_with_explicit_state() {
        let (tmp, store) = setup_store();
        let (alpha, beta) = seed_two_targets(tmp.path(), &store);
        let attempt_a = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let attempt_b = seed_group_attempt(&store, Some(beta), "aria/issues/i/web");
        let attempt_a = force_attempt_state(
            &store,
            &attempt_a,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
            None,
        );
        let attempt_b = force_attempt_state(
            &store,
            &attempt_b,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
            None,
        );
        seed_review_request(&store, &attempt_a, PushStatus::Pushed, None);
        seed_review_request(&store, &attempt_b, PushStatus::NotPushed, None);

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(
            projection.overall,
            PlanGroupOverall::Partial,
            "未推送 target 显式落 Partial——不伪装全局成功"
        );
        let unmet = entry_for(&projection, beta);
        assert_eq!(unmet.attempt_status, Some(CodingAttemptStatus::Completed));
        assert_eq!(
            unmet.push_status,
            Some(PushStatus::NotPushed),
            "未满足项显式：未推送"
        );
    }

    #[test]
    fn partial_failure_push_error_surfaces_in_blocked_reason() {
        let (tmp, store) = setup_store();
        let (alpha, beta) = seed_two_targets(tmp.path(), &store);
        let attempt_a = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let attempt_b = seed_group_attempt(&store, Some(beta), "aria/issues/i/web");
        let attempt_a = force_attempt_state(
            &store,
            &attempt_a,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
            None,
        );
        let attempt_b = force_attempt_state(
            &store,
            &attempt_b,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
            None,
        );
        seed_review_request(&store, &attempt_a, PushStatus::Pushed, None);
        seed_review_request(
            &store,
            &attempt_b,
            PushStatus::Failed,
            Some("push rejected".to_string()),
        );

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(projection.overall, PlanGroupOverall::Partial);
        let unmet = entry_for(&projection, beta);
        assert_eq!(unmet.push_status, Some(PushStatus::Failed));
        assert_eq!(
            unmet.blocked_reason.as_deref(),
            Some("push rejected"),
            "push_error 显式进入 blocked_reason"
        );
    }

    #[test]
    fn partial_failure_failed_and_manual_recovery_statuses_surface_blocked_reason() {
        let (tmp, store) = setup_store();
        let (alpha, beta) = seed_two_targets(tmp.path(), &store);
        let attempt_a = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let attempt_b = seed_group_attempt(&store, Some(beta), "aria/issues/i/web");
        let attempt_a = force_attempt_state(
            &store,
            &attempt_a,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
            None,
        );
        seed_review_request(&store, &attempt_a, PushStatus::Pushed, None);

        // 失败态（无 manual reason / push_error）→ 回落失败态 status 文本。
        let attempt_b = force_attempt_state(
            &store,
            &attempt_b,
            CodingAttemptStatus::Failed,
            CodingExecutionStage::Coding,
            None,
        );
        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(projection.overall, PlanGroupOverall::Partial);
        let unmet = entry_for(&projection, beta);
        assert_eq!(unmet.attempt_status, Some(CodingAttemptStatus::Failed));
        assert_eq!(unmet.blocked_reason.as_deref(), Some("failed"));

        // manual_recovery_reason 在场 → 优先进入 blocked_reason。
        let _attempt_b = force_attempt_state(
            &store,
            &attempt_b,
            CodingAttemptStatus::AwaitingManualRecovery,
            CodingExecutionStage::CodeReview,
            Some("需要人工恢复".to_string()),
        );
        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(projection.overall, PlanGroupOverall::Partial);
        let unmet = entry_for(&projection, beta);
        assert_eq!(
            unmet.attempt_status,
            Some(CodingAttemptStatus::AwaitingManualRecovery)
        );
        assert_eq!(
            unmet.blocked_reason.as_deref(),
            Some("需要人工恢复"),
            "manual_recovery_reason 优先进入 blocked_reason"
        );
    }

    #[test]
    fn partial_failure_missing_review_request_is_partial() {
        let (tmp, store) = setup_store();
        let (alpha, beta) = seed_two_targets(tmp.path(), &store);
        let attempt_a = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let attempt_b = seed_group_attempt(&store, Some(beta), "aria/issues/i/web");
        let attempt_a = force_attempt_state(
            &store,
            &attempt_a,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
            None,
        );
        let _attempt_b = force_attempt_state(
            &store,
            &attempt_b,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
            None,
        );
        seed_review_request(&store, &attempt_a, PushStatus::Pushed, None);

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(projection.overall, PlanGroupOverall::Partial);
        let unmet = entry_for(&projection, beta);
        assert_eq!(
            unmet.push_status, None,
            "无 ReviewRequest——未满足项显式（无推送事实）"
        );
        assert_eq!(unmet.review_request_id, None);
    }

    // ── 3) 未启三态：未启优先于部分 ──

    #[test]
    fn not_started_when_no_target_attempts() {
        let (tmp, store) = setup_store();
        let _ = seed_two_targets(tmp.path(), &store);

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(projection.overall, PlanGroupOverall::NotStarted);
        assert!(projection.entries.is_empty());
    }

    #[test]
    fn not_started_when_all_attempts_stay_in_initial_binary_group() {
        let (tmp, store) = setup_store();
        let (alpha, beta) = seed_two_targets(tmp.path(), &store);
        let _attempt_a = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let _attempt_b = seed_group_attempt(&store, Some(beta), "aria/issues/i/web");
        // 默认 create_group_attempt 落 (Created, PrepareContext)——全部未离开初始二元组。

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(
            projection.overall,
            PlanGroupOverall::NotStarted,
            "全部 (Created, PrepareContext) → 未启优先消歧——即使零交付也不落 Partial"
        );
        assert_eq!(projection.entries.len(), 2, "条目仍显式呈现（未启非缺席）");
        for entry in &projection.entries {
            assert_eq!(entry.attempt_status, Some(CodingAttemptStatus::Created));
            assert_eq!(entry.stage, Some(CodingExecutionStage::PrepareContext));
            assert_eq!(entry.blocked_reason, None);
        }
    }

    #[test]
    fn partial_when_some_targets_started_and_others_not() {
        let (tmp, store) = setup_store();
        let (alpha, beta) = seed_two_targets(tmp.path(), &store);
        let attempt_a = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let _attempt_b = seed_group_attempt(&store, Some(beta), "aria/issues/i/web");
        let _attempt_a = force_attempt_state(
            &store,
            &attempt_a,
            CodingAttemptStatus::Running,
            CodingExecutionStage::Coding,
            None,
        );

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(
            projection.overall,
            PlanGroupOverall::Partial,
            "部分启动+部分未启 → Partial（未启优先只适用于全部未启）"
        );
    }

    /// k3 fix round 1：PrepareContext 期 AbortAttempt 合法落盘
    /// (Aborted, PrepareContext)（Created→Aborted 为 attempt.rs 状态白名单转换，
    /// abort 不改 stage）——未达 provider 启动，不算离开初始二元组。
    #[test]
    fn not_started_when_pre_start_abort_never_reached_provider() {
        let (tmp, store) = setup_store();
        let (alpha, beta) = seed_two_targets(tmp.path(), &store);
        let attempt_a = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let _attempt_b = seed_group_attempt(&store, Some(beta), "aria/issues/i/web");
        let _attempt_a = force_attempt_state(
            &store,
            &attempt_a,
            CodingAttemptStatus::Aborted,
            CodingExecutionStage::PrepareContext,
            None,
        );

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(
            projection.overall,
            PlanGroupOverall::NotStarted,
            "pre-start abort（(Aborted, PrepareContext)）不算已达 provider 启动——另一 target 未动时整体未启（未启优先于部分）"
        );
        assert_eq!(projection.entries.len(), 2, "条目仍显式呈现");
        let aborted_entry = entry_for(&projection, alpha);
        assert_eq!(
            aborted_entry.attempt_status,
            Some(CodingAttemptStatus::Aborted)
        );
        assert_eq!(
            aborted_entry.stage,
            Some(CodingExecutionStage::PrepareContext)
        );
        assert_eq!(aborted_entry.blocked_reason.as_deref(), Some("aborted"));
    }

    /// k3 fix round 1 对偶：pre-start abort 与真正启动并存 → Partial 语义不变。
    #[test]
    fn partial_when_pre_start_abort_alongside_real_start() {
        let (tmp, store) = setup_store();
        let (alpha, beta) = seed_two_targets(tmp.path(), &store);
        let attempt_a = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let attempt_b = seed_group_attempt(&store, Some(beta), "aria/issues/i/web");
        let _attempt_a = force_attempt_state(
            &store,
            &attempt_a,
            CodingAttemptStatus::Aborted,
            CodingExecutionStage::PrepareContext,
            None,
        );
        let _attempt_b = force_attempt_state(
            &store,
            &attempt_b,
            CodingAttemptStatus::Running,
            CodingExecutionStage::Coding,
            None,
        );

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(
            projection.overall,
            PlanGroupOverall::Partial,
            "pre-start abort 不拉低已启动 target——任一 target 真正启动即 Partial"
        );
    }

    /// k3 fix round 2：多 unit group attempt 在 unit 间推进时
    /// `advance_to_next_group_unit` 把 stage 回退 PrepareContext
    /// （coding_workspace_engine/group.rs:315），其后 abort 落盘
    /// (Aborted, PrepareContext) 与 pre-start abort 同形态——但该 attempt
    /// 已执行过 provider（role_runs 在案）。判据取 durable 执行证据消歧：
    /// 证据在场=已达 provider 启动 → Partial（修前误 NotStarted）。
    #[test]
    fn partial_when_stage_reset_abort_has_provider_execution_evidence() {
        let (tmp, store) = setup_store();
        let (alpha, beta) = seed_two_targets(tmp.path(), &store);
        let attempt_a = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let _attempt_b = seed_group_attempt(&store, Some(beta), "aria/issues/i/web");
        // 前 unit 已执行：coder role run 在案（provider 执行铁证）。
        store
            .create_role_run(
                &attempt_a,
                CodingExecutionStage::Coding,
                CodingProviderRole::Coder,
                CodingRoleRunTrigger::Initial,
                None,
            )
            .unwrap();
        // unit 间推进回退 stage 后 abort：(Aborted, PrepareContext)+role_runs 非空。
        let _attempt_a = force_attempt_state(
            &store,
            &attempt_a,
            CodingAttemptStatus::Aborted,
            CodingExecutionStage::PrepareContext,
            None,
        );

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(
            projection.overall,
            PlanGroupOverall::Partial,
            "已执行过 provider 的 (Aborted, PrepareContext)（role_runs 在案）不算未启——整体不误翻 NotStarted"
        );
    }

    /// k3 fix round 2 证据通道二：head_commit 在场（无 role run）同为执行铁证。
    #[test]
    fn partial_when_stage_reset_abort_has_head_commit_evidence() {
        let (tmp, store) = setup_store();
        let (alpha, beta) = seed_two_targets(tmp.path(), &store);
        let attempt_a = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let _attempt_b = seed_group_attempt(&store, Some(beta), "aria/issues/i/web");
        let mut aborted = force_attempt_state(
            &store,
            &attempt_a,
            CodingAttemptStatus::Aborted,
            CodingExecutionStage::PrepareContext,
            None,
        );
        aborted.head_commit = Some("sha-evidence".to_string());
        store.write_coding_attempt_for_test(&aborted).unwrap();

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(
            projection.overall,
            PlanGroupOverall::Partial,
            "head_commit 在场=执行铁证——(Aborted, PrepareContext) 判已达 provider 启动"
        );
    }

    // ── 4) 每 target 最新 attempt 口径 ──

    #[test]
    fn latest_attempt_per_target_wins_the_verdict() {
        let (tmp, store) = setup_store();
        let (alpha, beta) = seed_two_targets(tmp.path(), &store);
        let attempt_a = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let attempt_b = seed_group_attempt(&store, Some(beta), "aria/issues/i/web");
        // target alpha：前失败（attempt_no=1）后完成（attempt_no=2）→ 取最新。
        let failed = force_attempt_state(
            &store,
            &attempt_a,
            CodingAttemptStatus::Failed,
            CodingExecutionStage::Coding,
            None,
        );
        let completed = extra_attempt_for_target(
            &store,
            &failed,
            2,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
        );
        seed_review_request(&store, &completed, PushStatus::Pushed, None);
        let attempt_b = force_attempt_state(
            &store,
            &attempt_b,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
            None,
        );
        seed_review_request(&store, &attempt_b, PushStatus::Pushed, None);

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(
            projection.overall,
            PlanGroupOverall::AllDelivered,
            "前失败后完成——按最新 attempt 判定"
        );
        let alpha_entry = entry_for(&projection, alpha);
        assert_eq!(
            alpha_entry.attempt_id.as_deref(),
            Some(completed.id.as_str())
        );
        assert_eq!(alpha_entry.blocked_reason, None);

        // 反向：前完成（attempt_no=1）后失败（attempt_no=3）→ 仍取最新（Partial）。
        let regressed = extra_attempt_for_target(
            &store,
            &completed,
            3,
            CodingAttemptStatus::Failed,
            CodingExecutionStage::Coding,
        );
        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(projection.overall, PlanGroupOverall::Partial);
        let alpha_entry = entry_for(&projection, alpha);
        assert_eq!(
            alpha_entry.attempt_id.as_deref(),
            Some(regressed.id.as_str())
        );
        assert_eq!(alpha_entry.blocked_reason.as_deref(), Some("failed"));
    }

    // ── 5) 只读派生：无第二状态机（调用前后 durable 目录零新增/修改） ──

    #[test]
    fn projection_is_read_only_derivation_no_durable_writes() {
        let (tmp, store) = setup_store();
        let (alpha, beta) = seed_two_targets(tmp.path(), &store);
        let attempt_a = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let _attempt_b = seed_group_attempt(&store, Some(beta), "aria/issues/i/web");
        let attempt_a = force_attempt_state(
            &store,
            &attempt_a,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
            None,
        );
        seed_review_request(&store, &attempt_a, PushStatus::Pushed, None);

        // 预热 store 级一次性 legacy logical-codebase 懒迁移（strict 解析路径的
        // 既有幂等行为，issue 级 delivery 面同款）——本断言的对象是「聚合派生
        // 不写任何文件」，须在迁移后的稳态上比较。
        LogicalCodebaseStore::new(store.paths())
            .migrate_legacy(PROJECT_ID)
            .unwrap();
        let before = durable_inventory(&tmp.path().join(".aria"));
        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        let after = durable_inventory(&tmp.path().join(".aria"));
        let mut delta: Vec<String> = before
            .keys()
            .chain(after.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter(|path| before.get(*path) != after.get(*path))
            .map(|path| {
                format!(
                    "{:?}: {:?} -> {:?}",
                    path.file_name(),
                    before.get(path),
                    after.get(path)
                )
            })
            .collect();
        delta.sort();
        assert!(
            delta.is_empty(),
            "聚合投影只读派生——不得新增/修改任何 durable 文件（无第二状态机），delta: {delta:#?}"
        );
        assert_eq!(projection.overall, PlanGroupOverall::Partial);
    }

    // ── 6) D2.1-A4：无快照存量 attempt 不进 per-target 投影 ──

    #[test]
    fn snapshotless_attempt_excluded_from_per_target_projection() {
        let (tmp, store) = setup_store();
        let (alpha, _beta) = seed_two_targets(tmp.path(), &store);
        let _legacy = seed_group_attempt(&store, None, "aria/issues/legacy");
        let target_attempt = seed_group_attempt(&store, Some(alpha), "aria/issues/i/api");
        let target_attempt = force_attempt_state(
            &store,
            &target_attempt,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
            None,
        );
        seed_review_request(&store, &target_attempt, PushStatus::Pushed, None);

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(
            projection.entries.len(),
            1,
            "无快照 attempt 不出现在 per-target 投影（不猜测归属）"
        );
        assert_eq!(projection.overall, PlanGroupOverall::AllDelivered);
    }

    // ── 7) 单 target 零变化旁路：与 issue 级 delivery 口径一致 ──

    #[test]
    fn single_target_projection_matches_issue_delivery_verdict() {
        let (tmp, store) = setup_store();
        let (alpha, _beta) = seed_two_targets(tmp.path(), &store);
        LifecycleStore::new(store.paths())
            .create_work_item(CreateWorkItemInput {
                id: Some(WORK_ITEM_ID.to_string()),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: "repository_0001".to_string(),
                title: "single target work item".to_string(),
                plan_status: crate::product::models::WorkItemPlanStatus::Confirmed,
                ..Default::default()
            })
            .unwrap();
        let attempt = seed_group_attempt(&store, Some(alpha), "aria/issues/issue_0001");
        let attempt = force_attempt_state(
            &store,
            &attempt,
            CodingAttemptStatus::Completed,
            CodingExecutionStage::FinalConfirm,
            None,
        );
        seed_review_request(&store, &attempt, PushStatus::Pushed, None);

        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(projection.entries.len(), 1, "单 target plan 投影=单条目");
        assert_eq!(projection.overall, PlanGroupOverall::AllDelivered);

        let issue_summary = store
            .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
            .unwrap();
        assert_eq!(
            issue_summary.overall,
            crate::product::coding_attempt_store::IssueDeliveryOverall::AllPushed,
            "单 target 场景两口径结论一致（plan AllDelivered ⟺ issue AllPushed）"
        );

        // 负向一致：attempt 失败 → 两口径同为未全交付。
        let _attempt = force_attempt_state(
            &store,
            &attempt,
            CodingAttemptStatus::Failed,
            CodingExecutionStage::Coding,
            None,
        );
        let projection = store
            .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        assert_eq!(projection.overall, PlanGroupOverall::Partial);
        let issue_summary = store
            .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
            .unwrap();
        assert_eq!(
            issue_summary.overall,
            crate::product::coding_attempt_store::IssueDeliveryOverall::Partial
        );
    }
}
