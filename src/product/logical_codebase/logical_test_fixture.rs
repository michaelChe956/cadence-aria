//! `#[cfg(test)]` 共享夹具：在任意 `ProductAppPaths` 下建出
//! 「逻辑代码库权威（manifest/member/checkout/selection/policy/repos.json）」
//! 与「冻结 `AttemptTargetSnapshot`」的最小可准入现场。
//!
//! REQ-ROUTE-01 两态化（去单仓入口）后，admission 的唯一凭证是
//! 「LC 权威 + 冻结快照」：无归属/不成对一律 `FailClosed(TargetMissing)`，
//! 不再有 `legacy_snapshot_digest` 之类按尝试记录自身生成快照摘要的准入路径。
//! 因此任何以 admission 行为（签票、CAS 转换、marker 清理）为不变量的用例，
//! 都必须先经本模块建出真实归属，否则测到的是 fail-closed 而不是原不变量。

use std::path::{Path, PathBuf};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::json_store::write_json;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
};
use crate::product::logical_codebase::store::LogicalCodebaseRecord;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, CheckoutAvailability, CheckoutKind, CodebaseMemberRecord,
    IssueCodebaseSelection, IssueCodebaseSelectionStore, LogicalCodebaseManifest,
    LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
    RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
use crate::product::models::RepositoryRecord;
use crate::product::project_store::{CreateProjectInput, ProjectStore};

/// 夹具内唯一成员/物理仓的 id（与 `repos.json`、checkout 记录一致）。
pub(crate) const FIXTURE_PHYSICAL_REPOSITORY_ID: &str = "repository_0001";

/// 建 project 记录 + legacy 布局的逻辑代码库权威。
///
/// 夹具沿用「项目级（legacy）布局」写 manifest/member/checkout/selection：
/// `LogicalCodebaseStore::new` 与 `for_lc(legacy_logical_codebase_id(project))`
/// 指向同一根（`store::lc_scope_root` 明文契约），而 `ProjectStore::get` 的
/// 幂等 `migrate_legacy` 会把该布局落成 legacy 别名 LC record。
///
/// 返回 `(成员 logical id, 主 checkout id)`，可直接交给 [`snapshot_for`]。
pub(crate) fn seed_logical_authority(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    root: &Path,
) -> (LogicalRepositoryId, RepositoryCheckoutId) {
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "project".to_string(),
            description: None,
        })
        .unwrap();
    let logical_id = LogicalRepositoryId(uuid::Uuid::new_v4());
    let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
    // 真实拓扑：成员 checkout 位于聚合根**之下**（`scope_member_prefix` 要求
    // `snapshot.canonical_path` 能 strip 掉 `provider_context_root`，否则
    // `WorkItemScopeDomainUnresolved` fail-closed）。聚合根留给 `aggregate-root`，
    // 与 manifest 的 `CommonNonGitParent` 布局一致。
    let aggregate_root = root.join("aggregate-root");
    let repository_path = aggregate_root.join(FIXTURE_PHYSICAL_REPOSITORY_ID);
    // 成员主 checkout 必须是**真实 git 仓**：cross-target 基线采样（`git_head` /
    // `git_status`）与 `build_attempt_target_snapshot` 都按真仓读 HEAD；裸目录会
    // 直接 fail-closed 成 `CrossTargetStoreFailure`。
    init_member_git_repo(&repository_path);
    let source_identity = RepositorySourceIdentity::from_git_parts(
        &repository_path,
        repository_path.join(".git"),
        None,
    );
    let authority = LogicalCodebaseStore::new(paths.clone());
    let manifest = LogicalCodebaseManifest::new(project_id, aggregate_root, vec![logical_id]);
    authority.save_manifest(project_id, &manifest).unwrap();
    authority
        .save_member(
            project_id,
            &CodebaseMemberRecord {
                logical_repository_id: logical_id,
                physical_repository_id: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
                alias: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
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
                created_at: "2026-08-11T00:00:00Z".to_string(),
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    authority
        .save_checkout(
            project_id,
            &RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: logical_id,
                physical_repository_id: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
                kind: CheckoutKind::Main,
                canonical_path: repository_path.clone(),
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: source_identity.git_dir_identity(),
                revision: Some("abcdef".to_string()),
                availability: CheckoutAvailability::Available,
                observed_at: "2026-08-11T00:00:00Z".to_string(),
                created_at: "2026-08-11T00:00:00Z".to_string(),
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            project_id,
            issue_id,
            vec![logical_id],
            Vec::new(),
            vec![logical_id],
            None,
        ))
        .unwrap();
    write_json(
        &paths.project_root(project_id).join("repos.json"),
        &[RepositoryRecord {
            id: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
            project_id: project_id.to_string(),
            name: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
            path: repository_path,
            repo_hash: "sha256:repository".to_string(),
            runtime_root: PathBuf::from("runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: "2026-08-11T00:00:00Z".to_string(),
            updated_at: "2026-08-11T00:00:00Z".to_string(),
            logical_repository_id: Some(logical_id),
            primary_checkout_id: Some(checkout_id),
            identity_schema_version: 1,
        }],
    )
    .unwrap();
    AggregatePolicyArtifactStore::new(paths.clone())
        .ensure_bootstrap(&manifest)
        .unwrap();
    (logical_id, checkout_id)
}

/// 真多仓（非 legacy 别名 lc_id）per-LC 子树权威播种。
///
/// REQ-ROUTE-01 两态化后路由/评估链只读 issue 持久归属的 **for_lc 子树**
/// （`lc_scope_root`：`logical_codebases/<lc_id>/`），project 级（legacy）布局
/// 的任何记录都不会成为该 issue 的路由事实——所以归属真 LC 的用例必须在
/// per-LC 子树落 manifest/member/checkout/selection/policy 与 repos.json
/// 投影，`RepositoryRouting::load_for_issue` 及
/// `resolve_logical_repository_strict` 三层身份复验（要求投影
/// `identity_schema_version=1` 与权威 member/checkout 配对一致）才能通过。
///
/// ⚠️ 调用方须**先建 issue**（归属 `lc_id`）再调本函数：selection 落在
/// `issues/<issue_id>/` 之内，不改变 `issues/` 顶层目录数，但保持与
/// [`seed_complete`] 相同的顺序纪律。返回 `(成员 logical id, 主 checkout id)`。
pub(crate) fn seed_logical_authority_for_lc(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    root: &Path,
    lc_id: &str,
) -> (LogicalRepositoryId, RepositoryCheckoutId) {
    assert_ne!(
        lc_id,
        &crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id),
        "seed_logical_authority_for_lc requires a non-legacy lc id; \
         use seed_logical_authority for the project-level legacy layout"
    );
    let logical_id = LogicalRepositoryId(uuid::Uuid::new_v4());
    let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
    // 与 legacy 布局同构的真实拓扑：成员 checkout 位于本 LC 聚合根之下
    // （`scope_member_prefix` 的 strip 前提），且必须是真实 git 仓。
    let aggregate_root = root.join("lc-aggregate-root");
    let repository_path = aggregate_root.join(FIXTURE_PHYSICAL_REPOSITORY_ID);
    init_member_git_repo(&repository_path);
    let source_identity = RepositorySourceIdentity::from_git_parts(
        &repository_path,
        repository_path.join(".git"),
        None,
    );
    let authority = LogicalCodebaseStore::for_lc(paths.clone(), lc_id);
    let manifest =
        LogicalCodebaseManifest::new(project_id, aggregate_root.clone(), vec![logical_id]);
    authority.save_manifest(project_id, &manifest).unwrap();
    // authority resolver 的 LC 存在性凭证：`resolve_logical` **直读**
    // `logical-codebases/{lc_id}/record.json`（不经 migrate_legacy）⇒ 本子树
    // 必须自带 record（legacy 别名布局由 `migrate_legacy` 懒生成，真 per-LC
    // 子树没有那条捷径）。
    let record_root = paths.logical_codebase_record_root(project_id, lc_id);
    std::fs::create_dir_all(&record_root).expect("fixture lc record root");
    write_json(
        &record_root.join("record.json"),
        &LogicalCodebaseRecord {
            id: lc_id.to_string(),
            name: lc_id.to_string(),
            aggregate_root,
            created_at: "2026-08-11T00:00:00Z".to_string(),
        },
    )
    .expect("fixture lc record.json");
    authority
        .save_member(
            project_id,
            &CodebaseMemberRecord {
                logical_repository_id: logical_id,
                physical_repository_id: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
                alias: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
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
                created_at: "2026-08-11T00:00:00Z".to_string(),
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    authority
        .save_checkout(
            project_id,
            &RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: logical_id,
                physical_repository_id: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
                kind: CheckoutKind::Main,
                canonical_path: repository_path.clone(),
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: source_identity.git_dir_identity(),
                revision: Some("abcdef".to_string()),
                availability: CheckoutAvailability::Available,
                observed_at: "2026-08-11T00:00:00Z".to_string(),
                created_at: "2026-08-11T00:00:00Z".to_string(),
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    IssueCodebaseSelectionStore::for_lc(paths.clone(), lc_id)
        .save(
            &IssueCodebaseSelection::all_members(project_id, issue_id, None)
                .for_logical_codebase(lc_id),
        )
        .unwrap();
    write_json(
        &paths.project_root(project_id).join("repos.json"),
        &[RepositoryRecord {
            id: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
            project_id: project_id.to_string(),
            name: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
            path: repository_path,
            repo_hash: "sha256:repository".to_string(),
            runtime_root: PathBuf::from("runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: "2026-08-11T00:00:00Z".to_string(),
            updated_at: "2026-08-11T00:00:00Z".to_string(),
            logical_repository_id: Some(logical_id),
            primary_checkout_id: Some(checkout_id),
            identity_schema_version: 1,
        }],
    )
    .unwrap();
    AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id)
        .ensure_bootstrap(&manifest)
        .unwrap();
    (logical_id, checkout_id)
}

/// 补规划链前置：**active(→Degraded) 聚合索引 + 聚合 policy artifact**。
///
/// 基座夹具（`workspace_engine::tests::make_work_item_plan_engine_with_accepted_contract_drafts`
/// 一族）只建 manifest/member/checkout/selection 权威，**不**建聚合索引与聚合
/// policy artifact；而规划链（workspace context / gateway / 计划聚合视野）与
/// 冻结快照构造都以二者为 fail-closed 前置：
/// - `AggregateIndexStore::active_required` 缺索引 ⇒ `aggregate_index_unavailable`；
/// - `AggregatePolicyArtifactStore::get` 缺 artifact ⇒ 快照无 `policy_digest`
///   可取（`snapshot_for` 直接读该 artifact）。
///
/// 本函数**只补这两段规划链前置**，不重写权威记录与兼容投影；index 标
/// Degraded（last-known-good）以短路 freshness 的真实 codegraph 探测。
/// `lc_id = None` 走项目级 legacy 布局，`Some(lc_id)` 走真多仓 per-LC 子树。
/// 幂等：索引已存在则不重建（只补 policy artifact）。
pub(crate) fn ensure_planning_prerequisites(
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: Option<&str>,
) {
    let (authority, policy_store, index_store) = match lc_id {
        Some(lc_id) => (
            LogicalCodebaseStore::for_lc(paths.clone(), lc_id),
            AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id),
            AggregateIndexStore::for_lc(paths.clone(), lc_id),
        ),
        None => (
            LogicalCodebaseStore::new(paths.clone()),
            AggregatePolicyArtifactStore::new(paths.clone()),
            AggregateIndexStore::new(paths.clone()),
        ),
    };
    let manifest = authority
        .load_manifest(project_id)
        .expect("load fixture manifest")
        .expect("fixture manifest must exist");
    let members = authority
        .list_members(project_id)
        .expect("list fixture members");
    let checkouts = authority
        .list_checkouts(project_id)
        .expect("list fixture checkouts");
    if index_store
        .get(project_id, FIXTURE_AGGREGATE_INDEX_ID)
        .expect("read fixture aggregate index")
        .is_none()
    {
        let now = chrono::Utc::now().to_rfc3339();
        let snapshots = members
            .iter()
            .filter(|member| member.status == MemberStatus::Active)
            .map(|member| {
                let checkout = checkouts
                    .iter()
                    .find(|checkout| member.checkout_ids.contains(&checkout.checkout_id))
                    .unwrap_or_else(|| panic!("member {} lacks a recorded checkout", member.alias));
                AggregateIndexMemberSnapshot::indexed(
                    member.logical_repository_id,
                    checkout.checkout_id,
                    checkout
                        .revision
                        .clone()
                        .unwrap_or_else(|| "abc123".to_string()),
                    false,
                    now.clone(),
                )
            })
            .collect::<Vec<_>>();
        let index = AggregateIndexRecord::building(
            FIXTURE_AGGREGATE_INDEX_ID.to_string(),
            project_id.to_string(),
            manifest.membership_revision,
            snapshots,
            now.clone(),
        );
        index_store
            .create(project_id, index.clone())
            .expect("create fixture aggregate index");
        let mut activated = index;
        activated.status = AggregateIndexStatus::Active;
        index_store
            .replace_active(project_id, activated)
            .expect("activate fixture aggregate index");
        index_store
            .mark_status(
                project_id,
                FIXTURE_AGGREGATE_INDEX_ID,
                AggregateIndexStatus::Degraded,
                Some("test fixture degraded last-known-good index".to_string()),
            )
            .expect("mark fixture aggregate index degraded");
    }
    policy_store
        .ensure_bootstrap(&manifest)
        .expect("bootstrap fixture aggregate policy");
}

/// 夹具内聚合索引的固定 id（与 web 侧 `lc_fixture_support` 同口径）。
pub(crate) const FIXTURE_AGGREGATE_INDEX_ID: &str = "aggregate_index_0001";

/// 按当前 LC 权威事实构造一份**与权威一致**的冻结快照（policy digest /
/// membership revision 逐字取自 store，故 `validate_snapshot_fields` 必然通过）。
pub(crate) fn snapshot_for(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_id: LogicalRepositoryId,
    checkout_id: RepositoryCheckoutId,
) -> AttemptTargetSnapshot {
    snapshot_for_in_scope(paths, project_id, logical_id, checkout_id, None)
}

/// [`snapshot_for`] 的 per-LC 子树变体：权威事实按 `for_lc(lc_id)` 读取
/// （真 per-LC 子树的项目级 store 读不到 checkout/manifest/policy ⇒ 必须
/// 显式给 lc_id）。
pub(crate) fn snapshot_for_in_scope(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_id: LogicalRepositoryId,
    checkout_id: RepositoryCheckoutId,
    lc_id: Option<&str>,
) -> AttemptTargetSnapshot {
    let (logical, policy_store) = match lc_id {
        Some(lc_id) => (
            LogicalCodebaseStore::for_lc(paths.clone(), lc_id),
            AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id),
        ),
        None => (
            LogicalCodebaseStore::new(paths.clone()),
            AggregatePolicyArtifactStore::new(paths.clone()),
        ),
    };
    let checkout = logical
        .load_checkout(project_id, checkout_id)
        .unwrap()
        .unwrap();
    let manifest = logical.load_manifest(project_id).unwrap().unwrap();
    let policy = policy_store.get(project_id).unwrap().unwrap();
    AttemptTargetSnapshot {
        logical_repository_id: logical_id,
        checkout_id,
        physical_repository_id: FIXTURE_PHYSICAL_REPOSITORY_ID.to_string(),
        canonical_path: checkout.canonical_path,
        git_dir_identity: checkout.git_dir_identity,
        revision: checkout.revision,
        policy_digest: policy.digest,
        membership_revision: manifest.membership_revision,
        captured_at: "2026-08-11T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    }
}

/// 一条 issue 的完整可准入现场（供各处夹具共用）。
pub(crate) struct LogicalFixture {
    pub(crate) project_id: String,
    pub(crate) issue_id: String,
    /// issue 持久归属的 LC id（legacy 别名 id：其子树 = 项目级 legacy 布局）。
    pub(crate) logical_codebase_id: String,
    pub(crate) member_id: LogicalRepositoryId,
    pub(crate) checkout_id: RepositoryCheckoutId,
    pub(crate) member_path: PathBuf,
}

/// 建「project + 完整 LC（manifest/member/checkout/selection/policy/repos.json）
/// + 归属该 LC 的 issue」。
///
/// ⚠️ issue **必须先于** 任何 `IssueCodebaseSelectionStore::save` 创建：项目级
/// selection store 会生成 `issues/<issue_id>/` 目录，而 `IssueStore::create` 的 id
/// 由 `next_sequential_id_in_directory` 按目录数分配——顺序颠倒会让 issue id 漂移，
/// 与硬编码 id 的夹具全部错位。
pub(crate) fn seed_complete(
    paths: &ProductAppPaths,
    root: &Path,
    project_id: &str,
    issue_id: &str,
) -> LogicalFixture {
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "logical fixture".to_string(),
            description: None,
        })
        .unwrap();
    // 先建 issue（此时 issues/ 为空 ⇒ id 恰为 `issue_0001`），再建 LC 权威。
    crate::product::issue_store::IssueStore::new(paths.clone())
        .create(crate::product::issue_store::CreateProductIssueInput {
            project_id: project_id.to_string(),
            repo_id: Some(FIXTURE_PHYSICAL_REPOSITORY_ID.to_string()),
            logical_codebase_id:
                crate::product::logical_codebase::store::legacy_logical_codebase_id(project_id),
            title: "logical fixture issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .unwrap();
    let (member_id, checkout_id) = seed_logical_authority(paths, project_id, issue_id, root);
    LogicalFixture {
        project_id: project_id.to_string(),
        issue_id: issue_id.to_string(),
        logical_codebase_id: crate::product::logical_codebase::store::legacy_logical_codebase_id(
            project_id,
        ),
        member_id,
        checkout_id,
        member_path: root
            .join("aggregate-root")
            .join(FIXTURE_PHYSICAL_REPOSITORY_ID),
    }
}

impl LogicalFixture {
    /// 该现场下与权威一致、必然通过 `validate_snapshot_fields` 的冻结快照。
    pub(crate) fn snapshot(&self, paths: &ProductAppPaths) -> AttemptTargetSnapshot {
        snapshot_for(paths, &self.project_id, self.member_id, self.checkout_id)
    }
}

/// 建**仓维** shared-worktree 记录。
///
/// 有 `target_snapshot` 的 attempt 只按仓维分类租约（`run_exclusion`），issue 维
/// 记录会被判成 `legacy_shared_worktree_present` 而 fail-closed——所以迁移到
/// 快照语义的夹具必须改用本函数，而不是 `upsert_issue_shared_worktree`。
pub(crate) fn upsert_repo_worktree(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    project_id: &str,
    issue_id: &str,
    logical_id: LogicalRepositoryId,
    worktree_path: &Path,
) {
    lifecycle
        .upsert_repo_shared_worktree(
            crate::product::lifecycle_store::UpsertRepoSharedWorktreeInput {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                repository_id: logical_id,
                branch_name: "main".to_string(),
                worktree_path: worktree_path.to_path_buf(),
                base_branch: "main".to_string(),
            },
        )
        .expect("fixture repo shared worktree");
}

/// 仓维锁「获取 + 绑定到 attempt」（advance 预绑定语义）。
pub(crate) fn bind_repo_lock_to_attempt(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    project_id: &str,
    issue_id: &str,
    logical_id: LogicalRepositoryId,
    work_item_id: &str,
    attempt_id: &str,
) {
    let lease = lifecycle
        .try_acquire_repo_worktree_lock(
            project_id,
            issue_id,
            logical_id,
            work_item_id,
            &format!("repo_worktree_lease_{attempt_id}"),
        )
        .expect("fixture repo lease acquire");
    assert!(lease.acquired, "fixture repo lease must acquire");
    lifecycle
        .bind_repo_worktree_lock_to_attempt(
            project_id,
            issue_id,
            logical_id,
            work_item_id,
            attempt_id,
        )
        .expect("fixture repo lease bind");
}

/// 在夹具成员 checkout 上初始化一个带初始提交的真实 git 仓（幂等）。
fn init_member_git_repo(path: &Path) {
    if path.join(".git").exists() {
        return;
    }
    std::fs::create_dir_all(path).expect("fixture member dir");
    for args in [
        ["init", "--quiet"].as_slice(),
        ["config", "user.email", "fixture@example.test"].as_slice(),
        ["config", "user.name", "Fixture"].as_slice(),
    ] {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(path)
            .status()
            .expect("fixture git command");
        assert!(status.success(), "git {args:?} failed");
    }
    std::fs::write(path.join("README.md"), "fixture\n").expect("fixture readme");
    for args in [
        ["add", "README.md"].as_slice(),
        ["commit", "--quiet", "-m", "fixture"].as_slice(),
    ] {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(path)
            .status()
            .expect("fixture git command");
        assert!(status.success(), "git {args:?} failed");
    }
}
