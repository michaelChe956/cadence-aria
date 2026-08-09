//! 唯一 `PlanningContextResolver`：使 Story/Design/WorkItemPlan 的 context、
//! cwd、prompt、session audit 全部来自同一 `PlanningContextSnapshot`。
//!
//! 禁止任何 `issue.repo_id` / first Story fallback（REQ-PLN-07）：有效成员为空即
//! blocker，不塞 primary；`active_required` / 政策 artifact 缺失时 fail-closed，
//! 不回退任何单仓路径。`cwd` 来自 manifest 的 `provider_context_root`（聚合根），
//! 不硬编码 `/aggregate`。

use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::aggregate_index::{AggregateIndexError, AggregateIndexStore};
use crate::product::logical_codebase::planning_context::{
    MemberCheckoutFingerprint, PlanningContextSnapshot, PlanningContextSnapshotStore,
};
use crate::product::logical_codebase::planning_context_set::{
    InventoryInjection, PlanningContextSetResolver, RepositoryContextResolution,
    render_compact_inventory,
};
use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::{LogicalCodebaseStore, LogicalRepositoryId};

/// 规划只读 best-effort 状态。当前唯一取值为 `BestEffortConfigured`：已配置目标 +
/// cwd + pre/post 检测，但未达 `production_verified_readonly`，因此不宣称「物理上
/// 无法写入」。
///
/// 后续 task（PreToolUse deny / 生产级只读）可在本枚举新增 `ProductionVerifiedReadonly`，
/// 但本 task 仅产出 `BestEffortConfigured`，调用方必须据此上报 best-effort 语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BestEffortReadonlyStatus {
    /// best_effort_configured：配置目标 + cwd + pre/post 检测，未达 production_verified_readonly。
    BestEffortConfigured,
}

/// `PlanningContextResolver::build` 的返回值。Story/Design/WorkItemPlan 的
/// context/cwd/prompt/session audit 全部由该返回值派生，禁止旁路重建 context。
#[derive(Debug, Clone)]
pub struct ResolvedPlanningContext {
    /// 冻结后的唯一上下文快照；context/prompt/session audit 的唯一事实来源。
    pub snapshot: PlanningContextSnapshot,
    /// 聚合根 cwd，等于 manifest 的 `provider_context_root`。
    pub cwd: PathBuf,
    /// 紧凑成员 inventory 注入（已按预算截断），供 prompt 注入。
    pub inventory_injection: InventoryInjection,
    /// 读到的 active aggregate index id（冗余于 snapshot，便于 gateway 直接消费）。
    pub aggregate_index_id: String,
    /// 读到的 policy digest（冗余于 snapshot，便于 gateway 直接消费）。
    pub policy_digest: String,
    /// best-effort 只读状态；本 task 恒为 `BestEffortConfigured`。
    pub best_effort_readonly_status: BestEffortReadonlyStatus,
    /// 参与仓库集合解析结果（含 invalid 成员与 manifest 成员修订号）。
    pub context_resolution: RepositoryContextResolution,
}

/// 唯一规划上下文 resolver。组合 `PlanningContextSetResolver`（参与仓库集合）、
/// `AggregateIndexStore::active_required`（索引快照）、
/// `AggregatePolicyArtifactStore`（政策 digest）与
/// `PlanningContextSnapshotStore`（快照持久化），产出单一 `ResolvedPlanningContext`。
pub struct PlanningContextResolver {
    paths: ProductAppPaths,
    logical: LogicalCodebaseStore,
    sets: PlanningContextSetResolver,
    snapshots: PlanningContextSnapshotStore,
}

impl PlanningContextResolver {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self {
            logical: LogicalCodebaseStore::new(paths.clone()),
            sets: PlanningContextSetResolver::new(paths.clone()),
            snapshots: PlanningContextSnapshotStore::new(paths.clone()),
            paths,
        }
    }

    /// 构建 `ResolvedPlanningContext`。流程：解析参与仓库集合 → fail-closed 拒绝空有效
    /// 成员（REQ-PLN-07）→ 读 active 索引 + 政策 artifact（缺失即 blocker）→ 渲染紧凑
    /// inventory → 组装并持久化快照 → 返回唯一上下文。`cwd` 来自 manifest 的
    /// `provider_context_root`。
    pub fn build(
        &self,
        project_id: &str,
        issue_id: &str,
        targets: &[LogicalRepositoryId],
    ) -> Result<ResolvedPlanningContext, ProductStoreError> {
        let resolution = self.sets.resolve(project_id, issue_id)?;
        if resolution.set.is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "effective_member_empty",
                reason: format!(
                    "{project_id}/{issue_id}: no effective member; primary fallback forbidden"
                ),
            });
        }

        let index = AggregateIndexStore::new(self.paths.clone())
            .active_required(project_id)
            .map_err(map_index_error)?;
        let policy = AggregatePolicyArtifactStore::new(self.paths.clone())
            .get(project_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "aggregate_policy_artifact",
                id: project_id.to_string(),
            })?;

        // cwd 来自 manifest 的 provider_context_root（聚合根），不硬编码。
        let manifest =
            self.logical
                .load_manifest(project_id)?
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "logical_codebase_manifest",
                    id: project_id.to_string(),
                })?;
        let cwd = manifest.provider_context_root.clone();

        let injection = render_compact_inventory(&resolution, targets)?;

        let member_fingerprints = build_member_fingerprints(&index.member_snapshots, &resolution);

        let snapshot = PlanningContextSnapshot {
            schema_version: 1,
            project_id: project_id.into(),
            issue_id: issue_id.into(),
            membership_revision: resolution.membership_revision,
            effective_member_ids: resolution
                .set
                .iter()
                .map(|member| member.member_id)
                .collect(),
            member_fingerprints,
            aggregate_index_id: index.aggregate_index_id.clone(),
            index_revision: index.membership_revision,
            policy_digest: policy.digest.clone(),
            access_fingerprint: String::new(),
            captured_at: chrono::Utc::now().to_rfc3339(),
        };
        self.snapshots.save(&snapshot)?;

        Ok(ResolvedPlanningContext {
            cwd,
            inventory_injection: injection,
            aggregate_index_id: index.aggregate_index_id,
            policy_digest: policy.digest,
            best_effort_readonly_status: BestEffortReadonlyStatus::BestEffortConfigured,
            context_resolution: resolution,
            snapshot,
        })
    }
}

/// 将 active aggregate-index 成员快照投影为快照所需 `MemberCheckoutFingerprint`，
/// 只保留参与成员集合内的成员；可用性直接取 index 的 `included` 标志。
fn build_member_fingerprints(
    member_snapshots: &[crate::product::logical_codebase::aggregate_index::AggregateIndexMemberSnapshot],
    resolution: &RepositoryContextResolution,
) -> Vec<MemberCheckoutFingerprint> {
    member_snapshots
        .iter()
        .filter(|snapshot| {
            resolution
                .set
                .iter()
                .any(|member| member.member_id == snapshot.logical_repository_id)
        })
        .map(|snapshot| MemberCheckoutFingerprint {
            logical_repository_id: snapshot.logical_repository_id,
            checkout_id: snapshot.checkout_id,
            revision: snapshot.revision.clone(),
            dirty: snapshot.dirty,
            available: snapshot.included,
        })
        .collect()
}

/// `AggregateIndexError` → `ProductStoreError`：聚合索引读取失败一律 fail-closed，
/// 不回退单仓路径。
fn map_index_error(error: AggregateIndexError) -> ProductStoreError {
    ProductStoreError::Io(format!("aggregate_index_unavailable: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus,
    };
    use crate::product::logical_codebase::planning_context_set::InventoryInjectionBudget;
    use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
        IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore, MemberStatus,
        RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    };
    use std::path::PathBuf;
    use tempfile::TempDir;
    use uuid::Uuid;

    /// 稳定 UUID：禁止运行时随机，保证测试可复现；ID 组成磁盘路径前受
    /// `validate_relative_id` 约束（本测试使用 `project_0001` / `issue_0001` 等稳定 id）。
    const API_MEMBER_UUID: Uuid = stable_uuid(0x0001);

    const fn stable_uuid(seed: u16) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[14] = (seed >> 8) as u8;
        bytes[15] = seed as u8;
        // version 7 + variant 10xx，满足 Uuid::from_bytes 的合法构造。
        bytes[6] = 0x70;
        bytes[8] = 0x80;
        Uuid::from_bytes(bytes)
    }

    struct ResolverFixture {
        // 保留 temp 以持有临时目录生命周期；paths 派生自 temp.path()。
        #[allow(dead_code)]
        temp: TempDir,
        paths: ProductAppPaths,
        api_member_id: LogicalRepositoryId,
        cached_policy_digest: Option<String>,
    }

    impl ResolverFixture {
        fn resolver(&self) -> PlanningContextResolver {
            PlanningContextResolver::new(self.paths.clone())
        }

        fn aggregate_root(&self) -> PathBuf {
            self.temp.path().join("aggregate-root")
        }

        fn membership_revision(&self) -> u64 {
            1
        }

        fn policy_digest(&self) -> String {
            self.cached_policy_digest
                .clone()
                .expect("write_active_manifest_index_and_policy must run first")
        }

        /// 写入单成员 manifest（api，active）+ 显式 selection(issue_0001 → api) +
        /// active aggregate index（membership_revision 与 manifest 对齐）+ 政策
        /// bootstrap artifact，覆盖 resolver 的所有必读依赖。
        fn write_active_manifest_index_and_policy(&mut self) {
            let store = LogicalCodebaseStore::new(self.paths.clone());
            let manifest = LogicalCodebaseManifest::new(
                "project_0001",
                self.aggregate_root(),
                vec![self.api_member_id],
            );
            store.save_manifest("project_0001", &manifest).unwrap();
            store
                .save_member(
                    "project_0001",
                    &self.member_record(self.api_member_id, "api", MemberStatus::Active),
                )
                .unwrap();
            store
                .save_checkout("project_0001", &self.api_checkout())
                .unwrap();

            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![self.api_member_id],
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.paths.clone())
                .save(&selection)
                .unwrap();

            // active aggregate index：成员快照与 api member 对齐。
            let index = active_index_record("project_0001", self.api_member_id);
            AggregateIndexStore::new(self.paths.clone())
                .create("project_0001", index.clone())
                .unwrap();
            let mut activated = index.clone();
            activated.status = AggregateIndexStatus::Active;
            AggregateIndexStore::new(self.paths.clone())
                .replace_active("project_0001", activated)
                .unwrap();

            // 政策 bootstrap artifact。
            let policy = AggregatePolicyArtifactStore::new(self.paths.clone())
                .ensure_bootstrap(&manifest)
                .unwrap();
            self.cached_policy_digest = Some(policy.digest);
        }

        /// 写入一个 issue（issue_empty）的显式空 selection，使 resolver 对该 issue
        /// 解析出空有效成员集合，触发 fail-closed blocker。
        fn write_selection_with_no_effective_members(&self) {
            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_empty",
                Vec::new(),
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.paths.clone())
                .save(&selection)
                .unwrap();
        }

        fn member_record(
            &self,
            id: LogicalRepositoryId,
            alias: &str,
            status: MemberStatus,
        ) -> CodebaseMemberRecord {
            let now = "2026-08-10T00:00:00Z".to_string();
            let checkout_path = self.aggregate_root().join(alias);
            CodebaseMemberRecord {
                logical_repository_id: id,
                physical_repository_id: format!("repository_{alias}"),
                alias: alias.to_string(),
                role: "service".to_string(),
                ordinal: 1,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &checkout_path,
                    checkout_path.join(".git"),
                    Some(format!("ssh://git@example.test/acme/{alias}.git")),
                ),
                repo_type: RepositoryType::Backend,
                tech_stack: vec!["rust".to_string()],
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![RepositoryCheckoutId(Uuid::nil())],
                status,
                created_at: now.clone(),
                updated_at: now,
            }
        }

        fn api_checkout(&self) -> RepositoryCheckoutRecord {
            let now = "2026-08-10T00:00:00Z".to_string();
            RepositoryCheckoutRecord {
                checkout_id: RepositoryCheckoutId(Uuid::nil()),
                logical_repository_id: self.api_member_id,
                physical_repository_id: "repository_api".to_string(),
                kind: CheckoutKind::Main,
                canonical_path: self.aggregate_root().join("api"),
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: "sha256:git-dir".to_string(),
                revision: Some("abc123".to_string()),
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            }
        }
    }

    fn active_index_record(
        project_id: &str,
        member_id: LogicalRepositoryId,
    ) -> AggregateIndexRecord {
        let now = "2026-08-10T00:00:00Z".to_string();
        AggregateIndexRecord::building(
            "aggregate_index_0001".to_string(),
            project_id.to_string(),
            1,
            vec![AggregateIndexMemberSnapshot::indexed(
                member_id,
                RepositoryCheckoutId(Uuid::nil()),
                "abc123".to_string(),
                false,
                now,
            )],
            "2026-08-10T00:00:00Z".to_string(),
        )
    }

    fn resolver_fixture() -> ResolverFixture {
        let temp = tempfile::tempdir().unwrap();
        ResolverFixture {
            paths: ProductAppPaths::new(temp.path()),
            temp,
            api_member_id: LogicalRepositoryId(API_MEMBER_UUID),
            cached_policy_digest: None,
        }
    }

    #[test]
    fn resolver_produces_single_snapshot_cwd_and_inventory_for_all_artifacts() {
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();

        let resolved = fixture
            .resolver()
            .build("project_0001", "issue_0001", &[])
            .unwrap();
        assert_eq!(resolved.cwd, fixture.aggregate_root());
        assert_eq!(
            resolved.snapshot.membership_revision,
            fixture.membership_revision()
        );
        assert_eq!(resolved.snapshot.policy_digest, fixture.policy_digest());
        assert!(
            resolved.inventory_injection.rendered.len()
                <= InventoryInjectionBudget::DEFAULT.hard_bytes
        );
        assert_eq!(
            resolved.best_effort_readonly_status,
            BestEffortReadonlyStatus::BestEffortConfigured
        );
    }

    #[test]
    fn resolver_rejects_primary_fallback_when_selection_empty() {
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();
        fixture.write_selection_with_no_effective_members();

        let error = fixture
            .resolver()
            .build("project_0001", "issue_empty", &[])
            .unwrap_err();
        assert!(error.to_string().contains("effective_member_empty"));
    }
}
