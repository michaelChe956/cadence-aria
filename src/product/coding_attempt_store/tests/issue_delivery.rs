// issue_delivery 模块测试（自 issue_delivery.rs 内联 mod tests 拆出——1200 行
// 守护，WP6 关闸）：纯移动零语义变化；覆盖关系（list_attempts_covering_work_item）
// 与 per-target 投影一致性（WP4 REQ-COD-05/06）+ 既有聚合族。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::issue_delivery::IssueDeliveryOverall;
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CreateCodingAttemptInput, CreateCodingExecutionUnitInput,
    CreateGroupCodingAttemptInput,
};
use crate::product::coding_models::{
    AttemptTargetSnapshot, CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt,
    CodingExecutionUnitStatus, PushStatus, RemoteKind, ReviewRequest, ReviewRequestKind,
    ReviewRequestOwnerKind,
};
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::json_store::{read_json, write_json};
use crate::product::lifecycle_store::{CreateWorkItemInput, LifecycleStore};
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest,
    LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
    RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
use crate::product::models::{LifecycleWorkItemRecord, ProviderName, RepositoryRecord};
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";

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

fn seed_work_item(store: &CodingAttemptStore, work_item_id: &str, repository_id: &str) {
    LifecycleStore::new(store.paths())
        .create_work_item(CreateWorkItemInput {
            id: Some(work_item_id.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: repository_id.to_string(),
            title: format!("work item {work_item_id}"),
            plan_status: crate::product::models::WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .unwrap();
}

fn provider_snapshot() -> ProviderConfigSnapshot {
    ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::ClaudeCode),
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
    }
}

fn seed_completed_attempt(
    store: &CodingAttemptStore,
    work_item_id: &str,
    branch_name: &str,
    commit_sha: &str,
) -> CodingExecutionAttempt {
    let created = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: branch_name.to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .unwrap();
    let attempt = CodingExecutionAttempt {
        status: CodingAttemptStatus::Completed,
        head_commit: Some(commit_sha.to_string()),
        ..created
    };
    store.write_coding_attempt_for_test(&attempt).unwrap();
    attempt
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
        created_at: "2026-08-13T00:00:00Z".to_string(),
        updated_at: "2026-08-13T00:00:00Z".to_string(),
    };
    store.save_review_request(attempt, &request).unwrap();
}

fn set_work_item_target_repository(
    store: &CodingAttemptStore,
    work_item_id: &str,
    logical_id: LogicalRepositoryId,
) {
    let path = store
        .paths()
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("work-items")
        .join(format!("{work_item_id}.json"));
    let mut record: LifecycleWorkItemRecord = read_json(&path).unwrap();
    record.target_repository_id = Some(logical_id);
    write_json(&path, &record).unwrap();
}

/// 多 target 拆分增殖形态（REQ-MTG-02/WP4）夹具：按 target 分桶建
/// group attempt——api 桶 [w1, w2]、web 桶 [w3]。group attempt 仅在
/// `work_item_id` 承载桶内拓扑序首个 WI，其余 WI 的 attempt 归属经物化
/// coding units（`logical_work_item_id`）表达；per-(plan,target) 唯一性
/// 要求两 attempt 各持不同 target 冻结快照（OQ2 命名 branch）。
fn group_target_snapshot(tag: &str) -> AttemptTargetSnapshot {
    AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(Uuid::new_v4()),
        checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
        physical_repository_id: format!("repo_{tag}"),
        canonical_path: PathBuf::from(format!("/nonexistent/checkout-{tag}")),
        git_dir_identity: format!("git-dir-{tag}"),
        revision: Some(format!("rev-{tag}")),
        policy_digest: "policy-digest".to_string(),
        membership_revision: 1,
        captured_at: "2026-09-19T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    }
}

fn seed_group_attempt_with_units(
    store: &CodingAttemptStore,
    tag: &str,
    first_work_item_id: &str,
    bucket_work_items: &[&str],
) -> CodingExecutionAttempt {
    let snapshot = group_target_snapshot(tag);
    let created = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: first_work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: format!(
                "aria/issues/{ISSUE_ID}/{}",
                snapshot.logical_repository_id.0
            ),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            target_snapshot: Some(snapshot),
            max_auto_rework: 2,
        })
        .unwrap();
    for (index, work_item_id) in bucket_work_items.iter().enumerate() {
        store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: created.id.clone(),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                plan_id: "work_item_plan_0001".to_string(),
                logical_work_item_id: work_item_id.to_string(),
                work_item_revision_id: format!("revision_{work_item_id}"),
                dependency_logical_work_item_ids: Vec::new(),
                order_index: index as u32,
                status: CodingExecutionUnitStatus::Completed,
            })
            .unwrap();
    }
    let attempt = CodingExecutionAttempt {
        status: CodingAttemptStatus::Completed,
        head_commit: Some(format!("sha-{tag}")),
        ..created
    };
    store.write_coding_attempt_for_test(&attempt).unwrap();
    attempt
}

fn seed_multi_target_split_attempts(
    store: &CodingAttemptStore,
) -> (CodingExecutionAttempt, CodingExecutionAttempt) {
    seed_work_item(store, "work_item_0001", "repo_api");
    seed_work_item(store, "work_item_0002", "repo_api");
    seed_work_item(store, "work_item_0003", "repo_web");
    let api_attempt = seed_group_attempt_with_units(
        store,
        "api",
        "work_item_0001",
        &["work_item_0001", "work_item_0002"],
    );
    let web_attempt =
        seed_group_attempt_with_units(store, "web", "work_item_0003", &["work_item_0003"]);
    (api_attempt, web_attempt)
}

fn entry_for<'a>(
    summary: &'a super::IssueDeliverySummary,
    work_item_id: &str,
) -> &'a super::DeliveryEntry {
    summary
        .entries
        .iter()
        .find(|entry| entry.work_item_id == work_item_id)
        .unwrap_or_else(|| panic!("missing entry for {work_item_id}"))
}

/// REQ-COD-06 适配核查（WP4）：多 target 拆分增殖后，issue 级 per-WI 交付
/// 口径必须把 group attempt 的全部桶内 WI 视为被该 target-attempt 覆盖
/// （经物化 units），而非仅 `attempt.work_item_id` 直接承载的首个 WI——
/// 否则非首个 WI 永远 `attempt_status=None`、issue 永远 Partial、完成门
/// （`maybe_complete_issue_delivery`）永不触发。
#[test]
fn group_split_delivery_covers_bucket_work_items_via_units() {
    let (_tmp, store) = setup_store();
    let (api_attempt, web_attempt) = seed_multi_target_split_attempts(&store);
    seed_review_request(&store, &api_attempt, PushStatus::Pushed, None);
    seed_review_request(&store, &web_attempt, PushStatus::Pushed, None);

    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();

    assert_eq!(summary.overall, IssueDeliveryOverall::AllPushed);
    assert_eq!(summary.entries.len(), 3);
    for entry in &summary.entries {
        assert_eq!(entry.attempt_status, Some(CodingAttemptStatus::Completed));
        assert_eq!(entry.push_status, Some(PushStatus::Pushed));
    }
    // 每 WI 条目取其所属桶 target-attempt 的分支（per-target 交付链独立呈现）。
    assert_eq!(
        entry_for(&summary, "work_item_0001").branch_name.as_deref(),
        Some(api_attempt.branch_name.as_str())
    );
    assert_eq!(
        entry_for(&summary, "work_item_0002").branch_name.as_deref(),
        Some(api_attempt.branch_name.as_str())
    );
    assert_eq!(
        entry_for(&summary, "work_item_0003").branch_name.as_deref(),
        Some(web_attempt.branch_name.as_str())
    );
}

/// REQ-COD-06 适配核查（WP4）：某 target 桶交付失败显式呈现（partial
/// failure 不伪装全局成功），且不影响他桶条目。
#[test]
fn group_split_partial_failure_is_explicit_per_bucket() {
    let (_tmp, store) = setup_store();
    let (api_attempt, web_attempt) = seed_multi_target_split_attempts(&store);
    seed_review_request(
        &store,
        &api_attempt,
        PushStatus::Failed,
        Some("api push rejected".to_string()),
    );
    seed_review_request(&store, &web_attempt, PushStatus::Pushed, None);

    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();

    assert_eq!(summary.overall, IssueDeliveryOverall::Partial);
    for work_item_id in ["work_item_0001", "work_item_0002"] {
        let entry = entry_for(&summary, work_item_id);
        assert_eq!(entry.attempt_status, Some(CodingAttemptStatus::Completed));
        assert_eq!(entry.push_status, Some(PushStatus::Failed));
        assert_eq!(entry.push_error.as_deref(), Some("api push rejected"));
    }
    let web_entry = entry_for(&summary, "work_item_0003");
    assert_eq!(
        web_entry.attempt_status,
        Some(CodingAttemptStatus::Completed)
    );
    assert_eq!(web_entry.push_status, Some(PushStatus::Pushed));
    assert_eq!(web_entry.push_error, None);
}

/// 覆盖关系扩展不改变「取最新 attempt」语义：同 WI 存在旧 WorkItem-scope
/// attempt（push 失败）与更新的 group attempt（Pushed）时，条目取最新。
#[test]
fn latest_covering_attempt_wins_across_direct_and_unit_association() {
    let (_tmp, store) = setup_store();
    seed_work_item(&store, "work_item_0001", "repo_api");
    let stale = seed_completed_attempt(&store, "work_item_0001", "aria/w1/attempt-1", "sha-old");
    seed_review_request(
        &store,
        &stale,
        PushStatus::Failed,
        Some("old push failed".to_string()),
    );
    let group = seed_group_attempt_with_units(&store, "api", "work_item_0001", &["work_item_0001"]);
    seed_review_request(&store, &group, PushStatus::Pushed, None);

    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();

    assert_eq!(summary.overall, IssueDeliveryOverall::AllPushed);
    let entry = entry_for(&summary, "work_item_0001");
    assert_eq!(
        entry.branch_name.as_deref(),
        Some(group.branch_name.as_str())
    );
    assert_eq!(entry.push_status, Some(PushStatus::Pushed));
    assert_eq!(entry.push_error, None);
}

/// 播种双 target logical codebase 权威记录（manifest+members+checkouts+
/// repos.json 兼容投影，双记录数组），使两个 logical id 均可经
/// `resolve_logical_repository_strict` 解析（投影展示名口径与 issue 级一致）。
fn seed_dual_target_authority(
    root: &Path,
    store: &CodingAttemptStore,
    api_id: LogicalRepositoryId,
    web_id: LogicalRepositoryId,
) {
    let now = "2026-09-19T00:00:00Z".to_string();
    let authority = LogicalCodebaseStore::new(store.paths());
    authority
        .save_manifest(
            PROJECT_ID,
            &LogicalCodebaseManifest::new(
                PROJECT_ID,
                root.join("aggregate-root"),
                vec![api_id, web_id],
            ),
        )
        .unwrap();
    let mut repositories = Vec::new();
    for (index, (logical_id, checkout_name)) in
        [(api_id, "checkout_repo_api"), (web_id, "checkout_repo_web")]
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
}

/// 建持显式 logical id 快照的 target-attempt（Created 初始态+units 物化）。
fn seed_split_attempt_with_target(
    store: &CodingAttemptStore,
    logical_id: LogicalRepositoryId,
    first_work_item_id: &str,
    bucket_work_items: &[&str],
) -> CodingExecutionAttempt {
    let logical_key = logical_id.0;
    let snapshot = AttemptTargetSnapshot {
        logical_repository_id: logical_id,
        checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
        physical_repository_id: format!("physical_{logical_key}"),
        canonical_path: PathBuf::from(format!("/nonexistent/checkout-{logical_key}")),
        git_dir_identity: format!("git-dir-{logical_key}"),
        revision: Some(format!("rev-{logical_key}")),
        policy_digest: "policy-digest".to_string(),
        membership_revision: 1,
        captured_at: "2026-09-19T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    };
    let created = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: first_work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: format!("aria/issues/{ISSUE_ID}/{}", logical_id.0),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            target_snapshot: Some(snapshot),
            max_auto_rework: 2,
        })
        .unwrap();
    for (index, work_item_id) in bucket_work_items.iter().enumerate() {
        store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: created.id.clone(),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                plan_id: "work_item_plan_0001".to_string(),
                logical_work_item_id: work_item_id.to_string(),
                work_item_revision_id: format!("revision_{work_item_id}"),
                dependency_logical_work_item_ids: Vec::new(),
                order_index: index as u32,
                status: CodingExecutionUnitStatus::Completed,
            })
            .unwrap();
    }
    created
}

/// REQ-COD-06×REQ-MTG-04 判定一致性（WP4 Step 2）：issue 级 per-WI 口径
/// （`compute_issue_delivery_summary`）与 plan 级 per-target 口径（WP3
/// `compute_plan_group_projection`）在同一多 target durable 事实上结论一致
/// ——同 target 的 WI 集内：target 交付 ⟺ 桶内全部 WI 条目满足
/// （Completed+Pushed）；未启/失败相位同构非成功。三相位钉死：未启 →
/// 全交付 → partial failure。
#[test]
fn issue_delivery_and_plan_projection_agree_on_multi_target_facts() {
    use crate::product::coding_attempt_store::PlanGroupOverall;

    let (tmp, store) = setup_store();
    let api_id = LogicalRepositoryId(Uuid::new_v4());
    let web_id = LogicalRepositoryId(Uuid::new_v4());
    seed_dual_target_authority(tmp.path(), &store, api_id, web_id);
    seed_work_item(&store, "work_item_0001", "physical_checkout_repo_api");
    seed_work_item(&store, "work_item_0002", "physical_checkout_repo_api");
    seed_work_item(&store, "work_item_0003", "physical_checkout_repo_web");
    set_work_item_target_repository(&store, "work_item_0001", api_id);
    set_work_item_target_repository(&store, "work_item_0002", api_id);
    set_work_item_target_repository(&store, "work_item_0003", web_id);

    let api_attempt = seed_split_attempt_with_target(
        &store,
        api_id,
        "work_item_0001",
        &["work_item_0001", "work_item_0002"],
    );
    let web_attempt =
        seed_split_attempt_with_target(&store, web_id, "work_item_0003", &["work_item_0003"]);

    // 相位一（未启）：两 target-attempt 均停初始二元组 (Created, PrepareContext)。
    let projection = store
        .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, "work_item_plan_0001")
        .unwrap();
    assert_eq!(projection.overall, PlanGroupOverall::NotStarted);
    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(summary.overall, IssueDeliveryOverall::Partial);
    // per-WI 条目经 units 覆盖到桶 attempt（Created 在案，非 None）。
    for entry in &summary.entries {
        assert_eq!(entry.attempt_status, Some(CodingAttemptStatus::Created));
        assert_eq!(entry.push_status, None);
    }

    // 相位二（全交付）：两 attempt Completed + 两 ReviewRequest Pushed。
    for attempt in [&api_attempt, &web_attempt] {
        let mut completed = attempt.clone();
        completed.status = CodingAttemptStatus::Completed;
        completed.head_commit = Some("sha-delivered".to_string());
        store.write_coding_attempt_for_test(&completed).unwrap();
    }
    seed_review_request(&store, &api_attempt, PushStatus::Pushed, None);
    seed_review_request(&store, &web_attempt, PushStatus::Pushed, None);

    let projection = store
        .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, "work_item_plan_0001")
        .unwrap();
    assert_eq!(projection.overall, PlanGroupOverall::AllDelivered);
    assert_eq!(projection.entries.len(), 2);
    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(summary.overall, IssueDeliveryOverall::AllPushed);
    // 一致性核心断言：每 target 投影条目交付 ⟺ 桶内全部 WI 条目满足，
    // 且 attempt 身份/分支/展示名两口径同源。
    let api_entry = projection
        .entries
        .iter()
        .find(|entry| entry.target_repository_id == api_id)
        .expect("api projection entry");
    assert_eq!(
        api_entry.attempt_id.as_deref(),
        Some(api_attempt.id.as_str())
    );
    assert_eq!(api_entry.push_status, Some(PushStatus::Pushed));
    assert_eq!(api_entry.repository_name, "checkout_repo_api");
    for work_item_id in ["work_item_0001", "work_item_0002"] {
        let entry = entry_for(&summary, work_item_id);
        assert_eq!(
            entry.branch_name.as_deref(),
            Some(api_attempt.branch_name.as_str())
        );
        assert_eq!(entry.attempt_status, Some(CodingAttemptStatus::Completed));
        assert_eq!(entry.push_status, Some(PushStatus::Pushed));
        assert_eq!(entry.repository_name, "checkout_repo_api");
    }
    let web_entry = projection
        .entries
        .iter()
        .find(|entry| entry.target_repository_id == web_id)
        .expect("web projection entry");
    assert_eq!(web_entry.repository_name, "checkout_repo_web");
    let web_summary_entry = entry_for(&summary, "work_item_0003");
    assert_eq!(
        web_summary_entry.branch_name.as_deref(),
        Some(web_attempt.branch_name.as_str())
    );
    assert_eq!(web_summary_entry.repository_name, "checkout_repo_web");

    // 相位三（partial failure）：api 桶重推失败——既有 ReviewRequest 同 id
    // 覆写为 Failed（save_review_request 覆盖更新，最新条目即失败态）。
    let api_review = store
        .list_review_requests(PROJECT_ID, ISSUE_ID, &api_attempt.id)
        .unwrap()
        .pop()
        .unwrap();
    let mut failed_review = api_review.clone();
    failed_review.push_status = PushStatus::Failed;
    failed_review.push_error = Some("api push rejected".to_string());
    store
        .save_review_request(&api_attempt, &failed_review)
        .unwrap();

    let projection = store
        .compute_plan_group_projection(PROJECT_ID, ISSUE_ID, "work_item_plan_0001")
        .unwrap();
    assert_eq!(projection.overall, PlanGroupOverall::Partial);
    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(summary.overall, IssueDeliveryOverall::Partial);
    let api_entry = projection
        .entries
        .iter()
        .find(|entry| entry.target_repository_id == api_id)
        .expect("api projection entry");
    assert_eq!(api_entry.push_status, Some(PushStatus::Failed));
    assert_eq!(
        api_entry.blocked_reason.as_deref(),
        Some("api push rejected")
    );
    for work_item_id in ["work_item_0001", "work_item_0002"] {
        let entry = entry_for(&summary, work_item_id);
        assert_eq!(entry.push_status, Some(PushStatus::Failed));
        assert_eq!(entry.push_error.as_deref(), Some("api push rejected"));
    }
    // web 桶不受 api 失败影响（两口径同判 delivered/满足）。
    let web_entry = projection
        .entries
        .iter()
        .find(|entry| entry.target_repository_id == web_id)
        .expect("web projection entry");
    assert_eq!(web_entry.push_status, Some(PushStatus::Pushed));
    let web_summary_entry = entry_for(&summary, "work_item_0003");
    assert_eq!(web_summary_entry.push_status, Some(PushStatus::Pushed));
}

/// 直改 attempt 的 attempt_no/created_at 落盘（构造跨计数空间的账簿形态；
/// k3 fix round 1 夹具——per-WI 与 group attempt 的 attempt_no 来自不同
/// 计数空间，group 锚桶内首个 WI，混排不可比）。
fn force_attempt_bookkeeping(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    attempt_no: u32,
    created_at: &str,
) -> CodingExecutionAttempt {
    let mut shaped = attempt.clone();
    shaped.attempt_no = attempt_no;
    shaped.created_at = created_at.to_string();
    shaped.updated_at = created_at.to_string();
    store
        .write_coding_attempt_for_test(&shaped)
        .expect("shape attempt bookkeeping");
    shaped
}

/// k3 fix round 1（P2）用例①：非桶首 WI 持 2 个陈旧 per-WI attempt
/// （w2 计数空间编号 1/2、created_at 更早、交付失败）+ 新 group attempt
/// （锚 w1、w1 计数空间编号 1、created_at 更新、交付成功）——「最新」须以
/// created_at 判：取 group（修前按 (attempt_no,id) 混排被陈旧编号 2 压过，
/// w2 条目永远失败态、完成门永不触发）。
#[test]
fn latest_covering_prefers_newer_group_over_stale_per_wi_attempts() {
    let (_tmp, store) = setup_store();
    seed_work_item(&store, "work_item_0001", "repo_api");
    seed_work_item(&store, "work_item_0002", "repo_api");

    let stale1 = seed_completed_attempt(&store, "work_item_0002", "aria/w2/a1", "sha-old1");
    let stale1 = force_attempt_bookkeeping(&store, &stale1, 1, "2026-09-18T00:00:00Z");
    seed_review_request(
        &store,
        &stale1,
        PushStatus::Failed,
        Some("stale push 1".to_string()),
    );
    let stale2 = seed_completed_attempt(&store, "work_item_0002", "aria/w2/a2", "sha-old2");
    let stale2 = force_attempt_bookkeeping(&store, &stale2, 2, "2026-09-18T01:00:00Z");
    seed_review_request(
        &store,
        &stale2,
        PushStatus::Failed,
        Some("stale push 2".to_string()),
    );

    let group = seed_group_attempt_with_units(
        &store,
        "api",
        "work_item_0001",
        &["work_item_0001", "work_item_0002"],
    );
    let group = force_attempt_bookkeeping(&store, &group, 1, "2026-09-19T00:00:00Z");
    seed_review_request(&store, &group, PushStatus::Pushed, None);

    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();

    // 陈旧 per-WI 失败不得压过更新的 group 交付（否则 issue 永远 Partial）。
    assert_eq!(summary.overall, IssueDeliveryOverall::AllPushed);
    for work_item_id in ["work_item_0001", "work_item_0002"] {
        assert_eq!(
            entry_for(&summary, work_item_id).branch_name.as_deref(),
            Some(group.branch_name.as_str())
        );
    }
}

/// k3 fix round 1（P2）用例②（反向）：旧 group attempt（编号 2、created_at
/// 更早、交付失败）+ 新 per-WI attempt（w2 计数空间编号 1、created_at 更新、
/// 交付成功）——取新 per-WI（修前编号 2 的 group 压过编号 1，误持旧失败态）。
#[test]
fn latest_covering_prefers_newer_per_wi_over_stale_group_attempt() {
    let (_tmp, store) = setup_store();
    seed_work_item(&store, "work_item_0001", "repo_api");
    seed_work_item(&store, "work_item_0002", "repo_api");

    let group = seed_group_attempt_with_units(
        &store,
        "api",
        "work_item_0001",
        &["work_item_0001", "work_item_0002"],
    );
    let group = force_attempt_bookkeeping(&store, &group, 2, "2026-09-18T00:00:00Z");
    seed_review_request(
        &store,
        &group,
        PushStatus::Failed,
        Some("group push failed".to_string()),
    );

    let fresh = seed_completed_attempt(&store, "work_item_0002", "aria/w2/fresh", "sha-new");
    let fresh = force_attempt_bookkeeping(&store, &fresh, 1, "2026-09-19T00:00:00Z");
    seed_review_request(&store, &fresh, PushStatus::Pushed, None);

    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();

    let entry = entry_for(&summary, "work_item_0002");
    assert_eq!(
        entry.branch_name.as_deref(),
        Some(fresh.branch_name.as_str())
    );
    assert_eq!(entry.push_status, Some(PushStatus::Pushed));
    assert_eq!(entry.push_error, None);
    // w1 仍由 group 承载（失败态）——整体显式 Partial。
    assert_eq!(summary.overall, IssueDeliveryOverall::Partial);
}

/// 播种 logical codebase 权威记录 + 兼容投影，使
/// `resolve_logical_repository_strict` 能把 `target_repository_id` 解析到
/// checkout 路径末段目录名 `checkout_repo_alpha`。
fn seed_logical_codebase_for_repository(
    root: &Path,
    store: &CodingAttemptStore,
    physical_repository_id: &str,
) -> LogicalRepositoryId {
    let logical_id = LogicalRepositoryId(Uuid::new_v4());
    let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
    let repository_path = root.join("checkout_repo_alpha");
    let source_identity = RepositorySourceIdentity::from_git_parts(
        &repository_path,
        repository_path.join(".git"),
        None,
    );

    let authority = LogicalCodebaseStore::new(store.paths());
    authority
        .save_manifest(
            PROJECT_ID,
            &LogicalCodebaseManifest::new(
                PROJECT_ID,
                root.join("aggregate-root"),
                vec![logical_id],
            ),
        )
        .unwrap();
    authority
        .save_member(
            PROJECT_ID,
            &CodebaseMemberRecord {
                logical_repository_id: logical_id,
                physical_repository_id: physical_repository_id.to_string(),
                alias: "checkout_repo_alpha".to_string(),
                role: "repository".to_string(),
                ordinal: 1,
                source_identity: source_identity.clone(),
                repo_type: RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![checkout_id],
                status: MemberStatus::Active,
                created_at: "2026-08-13T00:00:00Z".to_string(),
                updated_at: "2026-08-13T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    authority
        .save_checkout(
            PROJECT_ID,
            &RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: logical_id,
                physical_repository_id: physical_repository_id.to_string(),
                kind: CheckoutKind::Main,
                canonical_path: repository_path.clone(),
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: source_identity.git_dir_identity(),
                revision: Some("abcdef".to_string()),
                availability: CheckoutAvailability::Available,
                observed_at: "2026-08-13T00:00:00Z".to_string(),
                created_at: "2026-08-13T00:00:00Z".to_string(),
                updated_at: "2026-08-13T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    write_json(
        &store.paths().project_root(PROJECT_ID).join("repos.json"),
        &[RepositoryRecord {
            id: physical_repository_id.to_string(),
            project_id: PROJECT_ID.to_string(),
            name: physical_repository_id.to_string(),
            path: repository_path.clone(),
            repo_hash: "sha256:repository".to_string(),
            runtime_root: repository_path.join(".aria/runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: "2026-08-13T00:00:00Z".to_string(),
            logical_repository_id: Some(logical_id),
            primary_checkout_id: Some(checkout_id),
            identity_schema_version: 1,
            updated_at: "2026-08-13T00:00:00Z".to_string(),
        }],
    )
    .unwrap();

    logical_id
}

#[test]
fn all_pushed_returns_all_pushed() {
    let (_tmp, store) = setup_store();
    seed_work_item(&store, "work_item_0001", "repo_alpha");
    seed_work_item(&store, "work_item_0002", "repo_beta");

    let attempt1 = seed_completed_attempt(&store, "work_item_0001", "aria/w1/attempt-1", "sha111");
    seed_review_request(&store, &attempt1, PushStatus::Pushed, None);
    let attempt2 = seed_completed_attempt(&store, "work_item_0002", "aria/w2/attempt-1", "sha222");
    seed_review_request(&store, &attempt2, PushStatus::Pushed, None);

    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();

    assert_eq!(summary.overall, IssueDeliveryOverall::AllPushed);
    assert_eq!(summary.entries.len(), 2);
    for entry in &summary.entries {
        assert_eq!(entry.attempt_status, Some(CodingAttemptStatus::Completed));
        assert_eq!(entry.push_status, Some(PushStatus::Pushed));
    }
    // 无 target_repository_id 时回退 repository_id 字符串。
    let repository_names: Vec<&str> = summary
        .entries
        .iter()
        .map(|entry| entry.repository_name.as_str())
        .collect();
    assert!(repository_names.contains(&"repo_alpha"));
    assert!(repository_names.contains(&"repo_beta"));
}

#[test]
fn partial_when_one_push_failed() {
    let (_tmp, store) = setup_store();
    seed_work_item(&store, "work_item_0001", "repo_alpha");
    seed_work_item(&store, "work_item_0002", "repo_beta");

    let attempt1 = seed_completed_attempt(&store, "work_item_0001", "aria/w1/attempt-1", "sha111");
    seed_review_request(&store, &attempt1, PushStatus::Pushed, None);
    let attempt2 = seed_completed_attempt(&store, "work_item_0002", "aria/w2/attempt-1", "sha222");
    seed_review_request(
        &store,
        &attempt2,
        PushStatus::Failed,
        Some("push rejected".to_string()),
    );

    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();

    assert_eq!(summary.overall, IssueDeliveryOverall::Partial);
    let failed = summary
        .entries
        .iter()
        .find(|entry| entry.work_item_id == "work_item_0002")
        .expect("failed work item entry");
    assert_eq!(failed.attempt_status, Some(CodingAttemptStatus::Completed));
    assert_eq!(failed.push_status, Some(PushStatus::Failed));
    assert_eq!(failed.push_error.as_deref(), Some("push rejected"));
}

#[test]
fn partial_when_attempt_missing_or_not_completed() {
    let (_tmp, store) = setup_store();
    seed_work_item(&store, "work_item_0001", "repo_alpha");
    seed_work_item(&store, "work_item_0002", "repo_beta");

    // work_item_0001 无 attempt；work_item_0002 有 Running（非 Completed）attempt。
    let running = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: "work_item_0002".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0002/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .unwrap();
    store
        .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &running.id)
        .unwrap();

    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();

    assert_eq!(summary.overall, IssueDeliveryOverall::Partial);
    assert_eq!(summary.entries.len(), 2);

    let missing = summary
        .entries
        .iter()
        .find(|entry| entry.work_item_id == "work_item_0001")
        .expect("missing attempt entry");
    assert_eq!(missing.attempt_status, None);
    assert_eq!(missing.branch_name, None);
    assert_eq!(missing.commit_sha, None);
    assert_eq!(missing.push_status, None);

    let not_completed = summary
        .entries
        .iter()
        .find(|entry| entry.work_item_id == "work_item_0002")
        .expect("not completed entry");
    assert_eq!(
        not_completed.attempt_status,
        Some(CodingAttemptStatus::Running)
    );
    assert_eq!(
        not_completed.branch_name.as_deref(),
        Some("aria/work-items/work_item_0002/attempt-1")
    );
    assert_eq!(not_completed.push_status, None);
}

#[test]
fn none_when_issue_has_no_work_items() {
    let (_tmp, store) = setup_store();

    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();

    assert_eq!(summary.overall, IssueDeliveryOverall::None);
    assert!(summary.entries.is_empty());
}

#[test]
fn idempotent_recompute() {
    let (_tmp, store) = setup_store();
    seed_work_item(&store, "work_item_0001", "repo_alpha");
    let attempt = seed_completed_attempt(&store, "work_item_0001", "aria/w1/attempt-1", "sha111");
    seed_review_request(&store, &attempt, PushStatus::Pushed, None);

    let first = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();
    let second = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();

    assert_eq!(first, second);
}

#[test]
fn repository_name_prefers_target_repository_checkout_path() {
    let (tmp, store) = setup_store();
    seed_work_item(&store, "work_item_0001", "repo_alpha");
    let logical_id = seed_logical_codebase_for_repository(tmp.path(), &store, "repository_0001");
    set_work_item_target_repository(&store, "work_item_0001", logical_id);

    let attempt = seed_completed_attempt(&store, "work_item_0001", "aria/w1/attempt-1", "sha111");
    seed_review_request(&store, &attempt, PushStatus::Pushed, None);

    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();

    assert_eq!(summary.overall, IssueDeliveryOverall::AllPushed);
    assert_eq!(summary.entries.len(), 1);
    assert_eq!(summary.entries[0].repository_name, "checkout_repo_alpha");
}

#[test]
fn ten_work_items_compute_within_one_second() {
    // 性能基线：10 个 Work Item（各含 Completed attempt + Pushed ReviewRequest）的
    // 聚合计算必须在 1s 内完成，防止 O(n²) 或逐文件全量扫描回归。
    let (_tmp, store) = setup_store();
    for index in 0..10 {
        let work_item_id = format!("work_item_{index:04}");
        let repository_id = format!("repo_{index:04}");
        seed_work_item(&store, &work_item_id, &repository_id);
        let attempt = seed_completed_attempt(
            &store,
            &work_item_id,
            &format!("aria/w{index}/attempt-1"),
            &format!("sha{index:03}"),
        );
        seed_review_request(&store, &attempt, PushStatus::Pushed, None);
    }

    let started = Instant::now();
    let summary = store
        .compute_issue_delivery_summary(PROJECT_ID, ISSUE_ID)
        .unwrap();
    let elapsed = started.elapsed();

    assert_eq!(summary.overall, IssueDeliveryOverall::AllPushed);
    assert_eq!(summary.entries.len(), 10);
    assert!(
        elapsed < Duration::from_secs(1),
        "aggregation took {elapsed:?}"
    );
}
