//! Task 11: 规划上下文 resume 一致性的 Web 层 resume 入口测试。
//!
//! `planning_resume_decision` 是 workspace_ws_handler 会话恢复处的 resume 校验入口：
//! 传统单仓路径（无 manifest/selection）返回 None 不受影响；逻辑代码库分支在 provider
//! 启动前校验 planning snapshot 指纹，SameContext 沿用、StaleContext 拒绝续跑。

use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
};
use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
    IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore,
    LogicalRepositoryId, MemberStatus, PlanningContextResolver, RepositoryCheckoutId,
    RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType, ResumeDecision,
};
use std::path::PathBuf;
use uuid::Uuid;

/// 稳定 UUID：禁止运行时随机，保证测试可复现。
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

struct PlanningResumeFixture {
    // 保留 temp 以持有临时目录生命周期；paths 派生自 temp.path()。
    #[allow(dead_code)]
    temp: tempfile::TempDir,
    paths: ProductAppPaths,
    api_member_id: LogicalRepositoryId,
}

impl PlanningResumeFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        Self {
            paths: ProductAppPaths::new(temp.path()),
            temp,
            api_member_id: LogicalRepositoryId(API_MEMBER_UUID),
        }
    }

    fn aggregate_root(&self) -> PathBuf {
        self.temp.path().join("aggregate-root")
    }

    fn resolver(&self) -> PlanningContextResolver {
        PlanningContextResolver::new(self.paths.clone())
    }

    /// 写入单成员 manifest + selection + active aggregate index + policy bootstrap，
    /// 与 planning_context_resolver 测试 fixture 同构，覆盖 resolver 的所有必读依赖。
    fn write_logical_codebase(&self) {
        let store = LogicalCodebaseStore::new(self.paths.clone());
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            self.aggregate_root(),
            vec![self.api_member_id],
        );
        store.save_manifest("project_0001", &manifest).unwrap();
        store
            .save_member("project_0001", &self.member_record())
            .unwrap();
        store
            .save_checkout("project_0001", &self.checkout_record())
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

        let index = self.active_index_record();
        AggregateIndexStore::new(self.paths.clone())
            .create("project_0001", index.clone())
            .unwrap();
        let mut activated = index.clone();
        activated.status = AggregateIndexStatus::Active;
        AggregateIndexStore::new(self.paths.clone())
            .replace_active("project_0001", activated)
            .unwrap();

        AggregatePolicyArtifactStore::new(self.paths.clone())
            .ensure_bootstrap(&manifest)
            .unwrap();
    }

    /// 模拟成员变更：manifest membership_revision 1 → 2 并同步推进 active aggregate
    /// index 的 membership_revision 与成员 checkout revision，使 planning snapshot
    /// 指纹漂移。
    fn change_membership_revision(&self) {
        let store = LogicalCodebaseStore::new(self.paths.clone());
        let mut manifest = store.load_manifest("project_0001").unwrap().unwrap();
        manifest.membership_revision = 2;
        store.save_manifest("project_0001", &manifest).unwrap();

        let index_store = AggregateIndexStore::new(self.paths.clone());
        let mut index = index_store.active("project_0001").unwrap().unwrap();
        index.membership_revision = 2;
        for snapshot in &mut index.member_snapshots {
            snapshot.revision = "def456".to_string();
        }
        index.updated_at = "2026-08-10T01:00:00Z".to_string();
        index_store.replace_active("project_0001", index).unwrap();
    }

    fn member_record(&self) -> CodebaseMemberRecord {
        let now = "2026-08-10T00:00:00Z".to_string();
        let checkout_path = self.aggregate_root().join("api");
        CodebaseMemberRecord {
            logical_repository_id: self.api_member_id,
            physical_repository_id: "repository_api".to_string(),
            alias: "api".to_string(),
            role: "service".to_string(),
            ordinal: 1,
            source_identity: RepositorySourceIdentity::from_git_parts(
                &checkout_path,
                checkout_path.join(".git"),
                Some("ssh://git@example.test/acme/api.git".to_string()),
            ),
            repo_type: RepositoryType::Backend,
            tech_stack: vec!["rust".to_string()],
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![RepositoryCheckoutId(Uuid::nil())],
            status: MemberStatus::Active,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    fn checkout_record(&self) -> RepositoryCheckoutRecord {
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

    fn active_index_record(&self) -> AggregateIndexRecord {
        let now = "2026-08-10T00:00:00Z".to_string();
        AggregateIndexRecord::building(
            "aggregate_index_0001".to_string(),
            "project_0001".to_string(),
            1,
            vec![AggregateIndexMemberSnapshot::indexed(
                self.api_member_id,
                RepositoryCheckoutId(Uuid::nil()),
                "abc123".to_string(),
                false,
                now,
            )],
            "2026-08-10T00:00:00Z".to_string(),
        )
    }
}

#[test]
fn planning_resume_decision_is_none_for_legacy_single_repo_path() {
    let fixture = PlanningResumeFixture::new();
    // 无 manifest/selection：传统单仓路径不校验，不受影响。
    let decision = planning_resume_decision(&fixture.paths, "project_0001", "issue_0001").unwrap();
    assert!(decision.is_none());
}

#[test]
fn planning_resume_decision_reuses_context_on_matching_fingerprint() {
    let fixture = PlanningResumeFixture::new();
    fixture.write_logical_codebase();
    fixture
        .resolver()
        .build("project_0001", "issue_0001", &[])
        .unwrap();

    let decision = planning_resume_decision(&fixture.paths, "project_0001", "issue_0001")
        .unwrap()
        .expect("logical codebase branch must produce a decision");
    assert!(matches!(decision, ResumeDecision::SameContext(_)));
}

#[test]
fn planning_resume_decision_returns_stale_when_membership_drifted() {
    let fixture = PlanningResumeFixture::new();
    fixture.write_logical_codebase();
    fixture
        .resolver()
        .build("project_0001", "issue_0001", &[])
        .unwrap();

    fixture.change_membership_revision();

    let decision = planning_resume_decision(&fixture.paths, "project_0001", "issue_0001")
        .unwrap()
        .expect("logical codebase branch must produce a decision");
    assert!(matches!(decision, ResumeDecision::StaleContext { .. }));
}
