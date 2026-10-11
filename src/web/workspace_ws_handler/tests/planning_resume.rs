//! Task 11: 规划上下文 resume 一致性的 Web 层 resume 入口测试。
//!
//! `planning_resume_decision` 是 workspace_ws_handler 会话恢复处的 resume 校验入口：
//! 传统单仓路径（无 manifest/selection）返回 None 不受影响；逻辑代码库分支在 provider
//! 启动前校验 planning snapshot 指纹，SameContext 沿用、StaleContext 拒绝续跑。

use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{ProviderSession, StreamingProviderInput};
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
};
use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
    IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore,
    LogicalRepositoryId, MemberStatus, PlanningContextResolver, PlanningContextSnapshotStore,
    RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    ResumeDecision,
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
    physical_repository_id: Option<String>,
    api_checkout_id: Option<RepositoryCheckoutId>,
}

impl PlanningResumeFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "planning resume fixture".to_string(),
                description: None,
            })
            .expect("create project");
        Self {
            paths,
            temp,
            api_member_id: LogicalRepositoryId(API_MEMBER_UUID),
            physical_repository_id: None,
            api_checkout_id: None,
        }
    }

    fn aggregate_root(&self) -> PathBuf {
        self.temp.path().join("aggregate-root")
    }

    fn resolver(&self) -> PlanningContextResolver {
        PlanningContextResolver::new_without_freshness(self.paths.clone())
    }

    /// 写入单成员 selection + active aggregate index；物理仓库存在时复用迁移生成的
    /// 权威 manifest/member/checkout，纯 resolver fixture 则写入原有最小权威记录。
    fn write_logical_codebase(&self) {
        let store = LogicalCodebaseStore::new(self.paths.clone());
        let manifest = if self.physical_repository_id.is_some() {
            store.load_manifest("project_0001").unwrap().unwrap()
        } else {
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
            manifest
        };

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

    fn update_api_member_identity_from_authority(&mut self) {
        let store = LogicalCodebaseStore::new(self.paths.clone());
        let manifest = store.load_manifest("project_0001").unwrap().unwrap();
        let [api_member_id] = manifest.member_ids.as_slice() else {
            panic!("expected exactly one migrated logical member");
        };
        let member = store
            .load_member("project_0001", *api_member_id)
            .unwrap()
            .unwrap();
        let [checkout_id] = member.checkout_ids.as_slice() else {
            panic!("expected exactly one migrated repository checkout");
        };
        self.api_member_id = *api_member_id;
        self.physical_repository_id = Some(member.physical_repository_id);
        self.api_checkout_id = Some(*checkout_id);
    }

    fn physical_repository_id(&self) -> &str {
        self.physical_repository_id
            .as_deref()
            .unwrap_or("repository_api")
    }

    fn api_checkout_id(&self) -> RepositoryCheckoutId {
        self.api_checkout_id
            .unwrap_or(RepositoryCheckoutId(Uuid::nil()))
    }

    fn member_record(&self) -> CodebaseMemberRecord {
        let now = "2026-08-10T00:00:00Z".to_string();
        let checkout_path = self.aggregate_root().join("api");
        CodebaseMemberRecord {
            logical_repository_id: self.api_member_id,
            physical_repository_id: self.physical_repository_id().to_string(),
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
            checkout_ids: vec![self.api_checkout_id()],
            status: MemberStatus::Active,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    fn checkout_record(&self) -> RepositoryCheckoutRecord {
        let now = "2026-08-10T00:00:00Z".to_string();
        RepositoryCheckoutRecord {
            checkout_id: self.api_checkout_id(),
            logical_repository_id: self.api_member_id,
            physical_repository_id: self.physical_repository_id().to_string(),
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
                self.api_checkout_id(),
                "abc123".to_string(),
                false,
                now,
            )],
            "2026-08-10T00:00:00Z".to_string(),
        )
    }
}

/// 构造一个已成功启动（`Ok`）的 provider session，供延迟落盘 commit 测试使用。
fn started_provider_session() -> ProviderSession {
    let (_event_tx, event_rx) = mpsc::channel(8);
    let (command_tx, _command_rx) = mpsc::channel(8);
    ProviderSession {
        native_session_id: None,
        events: event_rx,
        commands: command_tx,
    }
}

/// B3 集成测试用 fake：记录 provider input 后保持 session 打开（不发 Completed），
/// 使 rebuild run 停在驱动阶段，便于稳定断言新 OutlineRun 节点与 rebuilt snapshot
/// commit（不受 AutoRevision 后续节点干扰）。
struct RebuildRecordingProvider {
    input_tx: mpsc::UnboundedSender<StreamingProviderInput>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RebuildRecordingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let _ = self.input_tx.send(input);
        // 保持 event sender 存活，使 receiver 永不关闭（run 阻塞在驱动阶段）。
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            std::future::pending::<()>().await;
            let _ = event_tx;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[tokio::test]
async fn planning_resume_decision_is_none_for_legacy_single_repo_path() {
    let fixture = PlanningResumeFixture::new();
    // 无 manifest/selection：传统单仓路径不校验，不受影响。
    let decision =
        planning_resume_decision_with_fresh_index(&fixture.paths, "project_0001", "issue_0001")
            .await
            .unwrap();
    assert!(decision.is_none());
}

#[tokio::test]
async fn planning_resume_fail_closed_when_manifest_without_selection() {
    // 有 manifest、无 selection → 与 compile 一致 fail-closed（返回稳定错误码 repository_routing_target_missing）。
    let root = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    LogicalCodebaseStore::new(paths.clone())
        .save_manifest(
            "project_0001",
            &LogicalCodebaseManifest::new(
                "project_0001",
                paths.root().join("aggregate-root"),
                Vec::new(),
            ),
        )
        .unwrap();

    let error = planning_resume_decision_with_fresh_index(&paths, "project_0001", "issue_0001")
        .await
        .unwrap_err();
    assert!(error.contains("repository_routing_target_missing"));
    assert!(error.contains("work_item_target_missing"));
}

#[tokio::test]
async fn planning_resume_legacy_when_none_none() {
    // 无 manifest、无 selection → Ok(None)，传统单仓不受影响。
    let root = tempfile::tempdir().unwrap();
    let result = planning_resume_decision_with_fresh_index(
        &ProductAppPaths::new(root.path().join(".aria")),
        "project_0001",
        "issue_0001",
    )
    .await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn planning_resume_decision_reuses_context_on_matching_fingerprint() {
    let fixture = PlanningResumeFixture::new();
    fixture.write_logical_codebase();
    fixture
        .resolver()
        .build("project_0001", "issue_0001", &[])
        .unwrap();

    let decision =
        planning_resume_decision_with_fresh_index(&fixture.paths, "project_0001", "issue_0001")
            .await
            .unwrap()
            .expect("logical codebase branch must produce a decision");
    assert!(matches!(decision, ResumeDecision::SameContext(_)));
}

#[tokio::test]
async fn planning_resume_decision_returns_stale_when_membership_drifted() {
    let fixture = PlanningResumeFixture::new();
    fixture.write_logical_codebase();
    fixture
        .resolver()
        .build("project_0001", "issue_0001", &[])
        .unwrap();

    fixture.change_membership_revision();

    let decision =
        planning_resume_decision_with_fresh_index(&fixture.paths, "project_0001", "issue_0001")
            .await
            .unwrap()
            .expect("logical codebase branch must produce a decision");
    assert!(matches!(decision, ResumeDecision::StaleContext { .. }));
}

#[tokio::test]
async fn anchor_t13_planning_resume_launch_decision_supersedes_waiting_on_stale() {
    // B2a（Task 5.1，tasks.md 5.1）：StaleContext 不再自动转
    // WorkItemPlanOutlineRebuild spawn——SupersededWaiting（0 自动 start，
    // 等待显式 fresh）；SameContext/None 沿用原 kind 续跑；SC author 透传
    //（B2c/5.5 接管其 stale 等待语义，漂移不得静默改写为 legacy rebuild）。
    let fixture = PlanningResumeFixture::new();
    fixture.write_logical_codebase();
    let resolver = fixture.resolver();
    resolver.build("project_0001", "issue_0001", &[]).unwrap();

    let same = resolver
        .resume_with_fresh_index("project_0001", "issue_0001")
        .await
        .unwrap();
    match planning_resume_launch_decision(
        &Some(same),
        ProviderRunKind::WorkItemPlanOutlineRevision { feedback: None },
    ) {
        PlanningResumeLaunchDecision::ResumeExisting(
            ProviderRunKind::WorkItemPlanOutlineRevision { .. },
        ) => {}
        other => panic!("SameContext 必须沿用原 kind 续跑，got {other:?}"),
    }

    fixture.change_membership_revision();
    let stale = resolver
        .resume_with_fresh_index("project_0001", "issue_0001")
        .await
        .unwrap();
    assert!(
        matches!(
            planning_resume_launch_decision(
                &Some(stale),
                ProviderRunKind::WorkItemPlanOutlineRevision { feedback: None },
            ),
            PlanningResumeLaunchDecision::SupersededWaiting
        ),
        "StaleContext 必须等待显式 fresh（SupersededWaiting），不得自动 spawn rebuild"
    );

    match planning_resume_launch_decision(
        &None,
        ProviderRunKind::WorkItemPlanOutlineRevision { feedback: None },
    ) {
        PlanningResumeLaunchDecision::ResumeExisting(
            ProviderRunKind::WorkItemPlanOutlineRevision { .. },
        ) => {}
        other => panic!("传统单仓 None 必须沿用原 kind，got {other:?}"),
    }

    let stale_again = resolver
        .resume_with_fresh_index("project_0001", "issue_0001")
        .await
        .unwrap();
    match planning_resume_launch_decision(
        &Some(stale_again),
        ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
    ) {
        PlanningResumeLaunchDecision::ResumeExisting(
            ProviderRunKind::WorkItemPlanSingleCandidateAuthor,
        ) => {}
        other => panic!("SC author 漂移透传（B2c 接管前原语义），got {other:?}"),
    }
}

#[tokio::test]
async fn rebuilt_snapshot_committed_only_after_provider_start_success() {
    // 新 BLOCKER 修复：rebuilt snapshot 仅在 provider 成功启动后 commit。
    // provider 启动失败不落盘 —— 重连仍 StaleContext（避免再次 TOCTOU）。
    let fixture = PlanningResumeFixture::new();
    fixture.write_logical_codebase();
    let resolver = fixture.resolver();
    resolver.build("project_0001", "issue_0001", &[]).unwrap();

    fixture.change_membership_revision();
    let stale = resolver
        .resume_with_fresh_index("project_0001", "issue_0001")
        .await
        .unwrap();
    let ResumeDecision::StaleContext { rebuilt, .. } = stale else {
        panic!("expected StaleContext");
    };

    // provider 启动失败：不 commit。
    let provider_failed =
        Err::<ProviderSession, _>(ProviderAdapterError::execution_failed(None, "", "", 0));
    assert!(!commit_rebuilt_snapshot_after_provider_start(
        &fixture.paths,
        &rebuilt,
        &provider_failed
    ));

    // 落盘快照仍是旧指纹，重连仍 StaleContext。
    let store = PlanningContextSnapshotStore::new(fixture.paths.clone());
    let persisted = store.load("project_0001", "issue_0001").unwrap().unwrap();
    assert_ne!(
        persisted.access_fingerprint,
        rebuilt.snapshot.access_fingerprint
    );
    let again = resolver
        .resume_with_fresh_index("project_0001", "issue_0001")
        .await
        .unwrap();
    assert!(matches!(again, ResumeDecision::StaleContext { .. }));

    // provider 成功启动：commit rebuilt 快照，新会话/后续重连恢复 SameContext。
    let provider_ok = Ok::<ProviderSession, _>(started_provider_session());
    assert!(commit_rebuilt_snapshot_after_provider_start(
        &fixture.paths,
        &rebuilt,
        &provider_ok
    ));
    let persisted = store.load("project_0001", "issue_0001").unwrap().unwrap();
    assert_eq!(
        persisted.access_fingerprint,
        rebuilt.snapshot.access_fingerprint
    );
    let resumed = resolver
        .resume_with_fresh_index("project_0001", "issue_0001")
        .await
        .unwrap();
    assert!(matches!(resumed, ResumeDecision::SameContext(_)));
}

// ------------------------------------------------------------------
// B2a（Task 5.1，tasks.md 5.1 / 设计 §4.3/§5.3）：Plan stale 等待与全 tag
// dispatch。Stale 重连 0 自动 start（planning_context_stale 等待，feedback
// 在任何 take 前捕获）；显式 fresh claim 后 tag 精确 dispatch——本波覆盖
// LegacyOutlineRebuild（经 start_work_item_plan_author Logical 分支唯一启动
// 口，B0 full prepare 消费；旧 audit 不回捞）；未接管 tag typed 拒
//（lc_resume_recovery_context_unsupported），不回落通用 plan_author。
// ------------------------------------------------------------------

use crate::product::lifecycle_store::LcFreshClaimOutcome;
use crate::product::models::{
    HumanGateTurnStatus, LcFreshOperationState, LcLegacyOutlineRevisionPayload,
    LcOutlineRebuildOrigin, LcRecoveryBusinessContext, LcRecoveryRunContext,
    LcResumeRecoveryRecord, LcResumeRecoveryState, ProviderConversationRole, SingleCandidatePhase,
};
use crate::web::workspace_session::WorkspaceSessionManager;

/// anchor_t13 共用基座：物理仓 + issue + story/design + plan + WorkItemPlan
/// 会话 + 漂移后的逻辑 codebase（重连判 StaleContext）。
struct AnchorT13PlanFixture {
    base: PlanningResumeFixture,
    repo_dir: tempfile::TempDir,
    app_paths: ProductAppPaths,
    lifecycle: LifecycleStore,
    session_record: WorkspaceSessionRecord,
}

impl AnchorT13PlanFixture {
    fn new() -> Self {
        use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
        use crate::product::lifecycle_store::{
            CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateStorySpecInput,
            CreateWorkspaceSessionInput,
        };
        use crate::product::models::{
            IssueWorkItemPlanOptions, IssueWorkItemPlanStatus,
            WorkspaceType as ProductWorkspaceType,
        };
        use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};

        let mut base = PlanningResumeFixture::new();
        let app_paths = base.paths.clone();
        // 物理 repo 必须先于 write_logical_codebase 创建（selection 目录次序，
        // 否则 IssueStore 自动 id 变 issue_0002，与固定 issue_0001 断言冲突）。
        let repo_dir = tempfile::tempdir().unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(repo_dir.path())
                .status()
                .expect("初始化测试物理仓库")
                .success(),
            "测试物理仓库初始化失败"
        );
        let repository = RepositoryStore::new(app_paths.clone())
            .create(CreateRepositoryInput {
                project_id: "project_0001".to_string(),
                name: "Repo".to_string(),
                path: repo_dir.path().to_path_buf(),
                default_policy_preset: None,
                default_provider_mode: None,
                idempotency_key: "anchor-t13-repository".to_string(),
            })
            .unwrap();
        crate::product::repository_store::RepositoryStore::with_logical_codebase_feature(
            app_paths.clone(),
            crate::product::logical_codebase::LogicalCodebaseFeature::enabled(),
        )
        .ensure_identity_schema("project_0001")
        .unwrap();
        base.update_api_member_identity_from_authority();
        let issue = IssueStore::new(app_paths.clone())
            .create(CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some(repository.id.clone()),
                logical_codebase_id: None,
                title: "Anchor T13 planning".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        assert_eq!(issue.id, "issue_0001");

        base.write_logical_codebase();
        let resolver = base.resolver();
        resolver.build("project_0001", "issue_0001", &[]).unwrap();
        // 成员漂移：后续重连 resume 判 StaleContext（等待显式 fresh）。
        base.change_membership_revision();

        let lifecycle = LifecycleStore::new(app_paths.clone());
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: repository.id.clone(),
                title: "Story".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "Design".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        let plan = lifecycle
            .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
                id: Some("issue_work_item_plan_0001".to_string()),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                source_story_spec_ids: vec![story.id],
                source_design_spec_ids: vec![design.id],
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Draft,
                work_item_ids: vec![],
                repository_profile_ref: None,
                verification_plan_ids: vec![],
                dependency_graph: vec![],
                created_from_provider_run: None,
                validator_findings: vec![],
            })
            .unwrap();
        let session_record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: plan.id,
                workspace_type: ProductWorkspaceType::WorkItemPlan,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),

                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .unwrap();
        Self {
            base,
            repo_dir,
            app_paths,
            lifecycle,
            session_record,
        }
    }

    fn latest_record(&self) -> WorkspaceSessionRecord {
        self.lifecycle
            .get_workspace_session(&self.session_record.id)
            .expect("latest record")
    }
}

fn anchor_t13_business(entity_id: &str) -> LcRecoveryBusinessContext {
    LcRecoveryBusinessContext {
        entity_id: entity_id.to_string(),
        plan_id: Some(entity_id.to_string()),
        workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
        business_repository_physical_id: None,
        business_repository_logical_id: None,
        business_checkout_id: None,
        business_repository_path: None,
        source_story_ids: Vec::new(),
        source_design_ids: Vec::new(),
        plan_options_ref: None,
        artifact_refs: Vec::new(),
        review_refs: Vec::new(),
        author_provider: ProviderName::ClaudeCode,
        effective_reviewer_provider: Some(ProviderName::Codex),
        original_node_id: Some("timeline_node_001".to_string()),
        original_role_run_seq: None,
    }
}

fn anchor_t13_revision_node_detail(
    session_id: &str,
    node_id: &str,
    revision_feedback: &str,
) -> crate::product::models::NodeDetail {
    crate::product::models::NodeDetail {
        node_id: node_id.to_string(),
        session_id: session_id.to_string(),
        node_type: crate::web::workspace_ws_types::TimelineNodeType::WorkItemPlanOutlineRun,
        status: crate::web::workspace_ws_types::TimelineNodeStatus::Active,
        agent_role: None,
        provider: None,
        prompt: None,
        messages: Vec::new(),
        streaming_content: String::new(),
        execution_events: Vec::new(),
        permission_events: Vec::new(),
        verdict: None,
        artifact_ref: None,
        is_revision: true,
        revision_feedback: Some(revision_feedback.to_string()),
        base_artifact_ref: None,
        started_at: "2026-10-12T00:00:00Z".to_string(),
        ended_at: None,
    }
}

#[tokio::test]
async fn anchor_t13_plan_stale_waits_for_explicit_fresh() {
    use crate::web::workspace_ws_types::{
        ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType,
        WorkspaceStage as WsWorkspaceStage,
    };

    let fixture = AnchorT13PlanFixture::new();
    let revision_feedback = "原始 outline 返修反馈（take 前固化）".to_string();

    // 中断的 outline revision 节点：durable detail 已固化 revision feedback
    //（任何 take 之前的冻结事实）。
    let interrupted_node_id = "timeline_node_001".to_string();
    fixture
        .lifecycle
        .save_node_detail(
            &fixture.session_record.id,
            &interrupted_node_id,
            &anchor_t13_revision_node_detail(
                &fixture.session_record.id,
                &interrupted_node_id,
                &revision_feedback,
            ),
        )
        .expect("seed revision node detail");

    // 重连形态 engine：stage Running + 活动 OutlineRun 节点 + 无活 run。
    let record = fixture.latest_record();
    let checkpoint_store = Arc::new(CheckpointStore::new(
        fixture.base.temp.path().join("checkpoints-t13-stale"),
    ));
    let (engine_tx, _engine_rx) = mpsc::channel::<EngineEvent>(64);
    let mut session = WorkspaceSession::from_record(record.clone());
    session.stage = WorkspaceStage::Running;
    let mut engine = WorkspaceEngine::new_persistent(
        checkpoint_store,
        fixture.lifecycle.clone(),
        engine_tx,
        session,
    );
    engine.timeline_nodes = vec![TimelineNode {
        node_id: interrupted_node_id.clone(),
        node_type: TimelineNodeType::WorkItemPlanOutlineRun,
        agent: Some(ProviderName::ClaudeCode),
        stage: WsWorkspaceStage::Running,
        round: Some(1),
        status: TimelineNodeStatus::Active,
        title: interrupted_node_id.clone(),
        summary: Some("连接断开，运行已中止".to_string()),
        started_at: "2026-10-12T00:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: Some("artifact_current".to_string()),
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 1,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        },
        retry: None,
    }];
    engine.active_node_id = Some(interrupted_node_id.clone());
    let engine_arc = Arc::new(Mutex::new(engine));

    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(RebuildRecordingProvider {
            input_tx: input_tx.clone(),
        }),
    );
    let manager = WorkspaceSessionManager::test_fixture_with_parts(
        &record.id,
        engine_arc.clone(),
        Arc::new(registry),
        fixture.app_paths.clone(),
        record.clone(),
    );
    manager.recover_on_creation().await;

    // ① stale 重连 0 自动 start：不 spawn OutlineRebuild（B3 旧行为删除）。
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), input_rx.recv())
            .await
            .is_err(),
        "stale 重连不得自动 spawn，必须等待显式 fresh 授权"
    );

    // ② durable 等待：planning_context_stale + run_context 捕获（feedback
    //    在任何 take 前固化）。
    let after = fixture.latest_record();
    let recovery = after.lc_resume_recovery.clone().expect("等待记录必须落盘");
    assert_eq!(recovery.state, LcResumeRecoveryState::Waiting);
    assert_eq!(recovery.reason_code, "planning_context_stale");
    match &recovery.run_context {
        LcRecoveryRunContext::LegacyOutlineRevision {
            normalized_feedback,
            ..
        } => {
            assert_eq!(
                normalized_feedback, &revision_feedback,
                "captured run_context 必须保留原 revision feedback（take 前）"
            );
        }
        other => panic!("期望捕获 LegacyOutlineRevision run_context，got {other:?}"),
    }

    // ③ 收口：旧节点 Failed/active 清空/run 阶段回 PrepareContext；等待
    //    active 阻断后续自动链（outline/manual turn 不自动 resume）。
    {
        let engine = engine_arc.lock().await;
        assert!(
            engine.active_node_id.is_none(),
            "等待收口后活动节点必须清空"
        );
        assert_eq!(engine.session().stage, WorkspaceStage::PrepareContext);
        assert!(engine.lc_resume_recovery_active());
    }

    // ④ 重连幂等：同存活 manager 再次恢复仍 0 start、等待记录原样。
    manager.recover_on_creation().await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), input_rx.recv())
            .await
            .is_err(),
        "等待 active 期间不得有任何自动接力 spawn"
    );
    let replay_record = fixture.latest_record();
    let replay_recovery = replay_record.lc_resume_recovery.expect("recovery kept");
    assert_eq!(replay_recovery.state, LcResumeRecoveryState::Waiting);
    assert!(
        replay_recovery.fresh_operations.is_empty(),
        "未显式 fresh 前零受理 operation"
    );

    // ⑤ 显式 fresh：claim 受理（FreshReserved）后 dispatch——本波未接管的
    //    tag typed 拒（context_unsupported），operation failed、recovery 回
    //    Waiting、0 spawn（不回落通用 plan_author）。
    assert!(matches!(
        fixture.lifecycle.claim_lc_fresh(
            &fixture.session_record.id,
            &recovery.recovery_id,
            "op-t13-wait"
        ),
        Ok(LcFreshClaimOutcome::Claimed { .. })
    ));
    let mut dispatch_registry = ProviderRegistry::new();
    dispatch_registry.register(
        ProviderName::ClaudeCode,
        Arc::new(RebuildRecordingProvider { input_tx }),
    );
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(dispatch_registry),
        engine_arc.clone(),
        WorkspaceRunRegistry::default(),
        record.id.clone(),
        fixture.app_paths.clone(),
        fixture.latest_record(),
    );
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<OutboundControl>(8);
    dispatch_lc_fresh_recovery_run(run_context, outbound_tx).await;
    let frame = tokio::time::timeout(std::time::Duration::from_secs(1), outbound_rx.recv())
        .await
        .expect("未接管 tag 必须收到 typed 拒帧")
        .expect("outbound frame");
    match frame {
        OutboundControl::Text(text) => {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["type"], "protocol_error");
            assert_eq!(value["code"], "lc_resume_recovery_context_unsupported");
        }
        other => panic!("期望 Text 帧，got {other:?}"),
    }
    let final_record = fixture.latest_record();
    let final_recovery = final_record.lc_resume_recovery.expect("recovery kept");
    assert_eq!(final_recovery.state, LcResumeRecoveryState::Waiting);
    assert_eq!(final_recovery.fresh_operations.len(), 1);
    assert_eq!(
        final_recovery.fresh_operations[0].state,
        LcFreshOperationState::Failed
    );
    assert_eq!(
        final_recovery.fresh_operations[0].reason_code.as_deref(),
        Some("lc_resume_recovery_context_unsupported")
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), input_rx.recv())
            .await
            .is_err(),
        "typed 拒收必须零 spawn"
    );
}

#[tokio::test]
async fn anchor_t13_plan_fresh_authorized_rebuild_starts_after_claim() {
    let fixture = AnchorT13PlanFixture::new();

    // 直接落 LegacyOutlineRebuild 等待（origin=outline_revision，不丢
    // feedback）——stale 捕获面由上一用例覆盖，本用例验证显式 fresh 授权
    // 后的正路：claim → dispatch → 唯一启动口 native 启动。
    let expected = fixture.latest_record();
    let recovery = LcResumeRecoveryRecord {
        schema_version: 1,
        recovery_id: "recovery_anchor_t13_rebuild".to_string(),
        failed_provider_node_id: "timeline_node_001".to_string(),
        reason_code: "planning_context_stale".to_string(),
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        superseded_native_session_id: Some("native-t13-old".to_string()),
        previous_fingerprint_digest: Some("sha256:t13-old".to_string()),
        previous_session_status: expected.status.clone(),
        previous_stage: "running".to_string(),
        run_context: LcRecoveryRunContext::LegacyOutlineRebuild {
            business: anchor_t13_business(&expected.entity_id),
            origin: LcOutlineRebuildOrigin::OutlineRevision(LcLegacyOutlineRevisionPayload {
                normalized_feedback: "重建前返修反馈".to_string(),
                outline_ref: None,
                outline_version: None,
                outline_hash: None,
            }),
            previous_snapshot_access_fingerprint: Some("sha256:t13-snap".to_string()),
        },
        state: LcResumeRecoveryState::Waiting,
        operation_id: None,
        fresh_node_id: None,
        fresh_native_session_id: None,
        fresh_role_run_seq: None,
        fresh_operations: Vec::new(),
        detected_at: "2026-10-12T00:00:00Z".to_string(),
        updated_at: "2026-10-12T00:00:00Z".to_string(),
    };
    fixture
        .lifecycle
        .compare_and_save_lc_resume_state(&expected, recovery)
        .expect("seed rebuild waiting recovery");

    // 显式 fresh：socket 门通过后的权威 CAS claim（FreshReserved/Reserved）。
    assert!(matches!(
        fixture.lifecycle.claim_lc_fresh(
            &fixture.session_record.id,
            "recovery_anchor_t13_rebuild",
            "op-t13-fresh"
        ),
        Ok(LcFreshClaimOutcome::Claimed { .. })
    ));

    let record = fixture.latest_record();
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(RebuildRecordingProvider { input_tx }),
    );
    let checkpoint_store = Arc::new(CheckpointStore::new(
        fixture.base.temp.path().join("checkpoints-t13-rebuild"),
    ));
    let (engine_tx, _engine_rx) = mpsc::channel::<EngineEvent>(64);
    let engine = WorkspaceEngine::new_persistent(
        checkpoint_store,
        fixture.lifecycle.clone(),
        engine_tx,
        WorkspaceSession::from_record(record.clone()),
    );
    let engine_arc = Arc::new(Mutex::new(engine));
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine_arc,
        WorkspaceRunRegistry::default(),
        record.id.clone(),
        fixture.app_paths.clone(),
        record.clone(),
    );
    let (outbound_tx, _outbound_rx) = mpsc::channel::<OutboundControl>(8);
    dispatch_lc_fresh_recovery_run(run_context.clone(), outbound_tx).await;

    let input = tokio::time::timeout(std::time::Duration::from_secs(3), input_rx.recv())
        .await
        .expect("显式 fresh 授权后 rebuild 必须经唯一启动口启动 provider")
        .expect("provider input");
    // 唯一启动口/native 语义：worktree=dispatch 时新解析的 resolved cwd
    //（manifest provider_context_root；与 dispatch 同源解析断言）、全新启动
    // 不回捞旧 audit（resume None）、prompt 注入新 inventory。
    let resolved_root = match planning_resume_decision_with_fresh_index(
        &fixture.app_paths,
        "project_0001",
        "issue_0001",
    )
    .await
    .expect("fresh 解析 planning 决策")
    .expect("逻辑路由决策")
    {
        ResumeDecision::StaleContext { rebuilt, .. } => rebuilt.cwd,
        ResumeDecision::SameContext(resolved) => resolved.cwd,
    };
    assert_eq!(input.working_dir, resolved_root);
    assert_eq!(
        input.resume_provider_session_id, None,
        "fresh 不回捞旧 audit/native id"
    );
    assert!(input.prompt.contains("聚合代码库成员清单"));

    let after = fixture.latest_record();
    let recovery = after.lc_resume_recovery.expect("recovery kept");
    assert_eq!(recovery.state, LcResumeRecoveryState::FreshRunning);
    assert_eq!(recovery.fresh_operations.len(), 1);
    assert_eq!(recovery.fresh_operations[0].operation_id, "op-t13-fresh");
    assert_eq!(
        recovery.fresh_operations[0].state,
        LcFreshOperationState::Running
    );

    // 幂等：dispatch 重放（FreshRunning）不二次 spawn、状态不回退。双窗口
    // 断言：首轮后无残留第二 input（try_recv 即刻空）＋重放后 300ms 无新增。
    assert!(
        input_rx.try_recv().is_err(),
        "首轮 dispatch 后不得残留第二 provider input"
    );
    let (outbound_tx_2, _outbound_rx_2) = mpsc::channel::<OutboundControl>(8);
    dispatch_lc_fresh_recovery_run(run_context, outbound_tx_2).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(input_rx.try_recv().is_err(), "FreshRunning 重放不得二次 spawn");
    let idempotent = fixture.latest_record();
    assert_eq!(
        idempotent.lc_resume_recovery.expect("recovery kept").state,
        LcResumeRecoveryState::FreshRunning
    );
}

#[test]
fn anchor_t13_plan_run_context_dispatch_never_falls_back() {
    // WorkItemPlanFlowKind 经 tests.rs 顶层 glob(super::*)可用。

    let business = anchor_t13_business("plan_entity_t13");

    // typed tag → 精确 kind（§5.3 对照列）；feedback 原样透传、不重算。
    match lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::Author {
        business: business.clone(),
        original_content: Some("原始 author 内容".to_string()),
    })
    .expect("Author tag 必须可映射")
    {
        LcFreshRunKind::Kind(ProviderRunKind::Author { content }) => {
            assert_eq!(content, "原始 author 内容");
        }
        other => panic!("Author 精确 dispatch，got {other:?}"),
    }
    match lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::AuthorChoiceFollowup {
        business: business.clone(),
        original_content: None,
    })
    .expect("AuthorChoiceFollowup tag 必须可映射")
    {
        LcFreshRunKind::Kind(ProviderRunKind::AuthorChoiceFollowup { content }) => {
            assert_eq!(content, "");
        }
        other => panic!("AuthorChoiceFollowup 精确 dispatch，got {other:?}"),
    }
    match lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::AuthorFeedback {
        business: business.clone(),
        original_feedback: Some("author 反馈".to_string()),
    })
    .expect("AuthorFeedback tag 必须可映射")
    {
        LcFreshRunKind::Kind(ProviderRunKind::Revision) => {}
        other => panic!("AuthorFeedback 精确 dispatch，got {other:?}"),
    }
    match lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::ReviewerRevision {
        business: business.clone(),
        original_feedback: Some("reviewer 反馈".to_string()),
    })
    .expect("ReviewerRevision tag 必须可映射")
    {
        LcFreshRunKind::Kind(ProviderRunKind::Revision) => {}
        other => panic!("ReviewerRevision 精确 dispatch，got {other:?}"),
    }
    match lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::LegacyOutlineAuthor {
        business: business.clone(),
        flow: WorkItemPlanFlowKind::Legacy,
        original_provider_node_id: Some("node_outline".to_string()),
        outline_ref: None,
    })
    .expect("LegacyOutlineAuthor tag 必须可映射")
    {
        LcFreshRunKind::Kind(ProviderRunKind::WorkItemPlanLegacyAuthor) => {}
        other => panic!("LegacyOutlineAuthor 精确 dispatch，got {other:?}"),
    }
    match lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::LegacyOutlineRevision {
        business: business.clone(),
        normalized_feedback: "outline 返修反馈".to_string(),
        outline_ref: None,
        outline_version: None,
        outline_hash: None,
    })
    .expect("LegacyOutlineRevision tag 必须可映射")
    {
        LcFreshRunKind::Kind(ProviderRunKind::WorkItemPlanOutlineRevision { feedback }) => {
            assert_eq!(feedback.as_deref(), Some("outline 返修反馈"));
        }
        other => panic!("LegacyOutlineRevision 精确 dispatch，got {other:?}"),
    }
    match lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::LegacyOutlineRevision {
        business: business.clone(),
        normalized_feedback: String::new(),
        outline_ref: None,
        outline_version: None,
        outline_hash: None,
    })
    .expect("空 feedback 的 LegacyOutlineRevision 必须可映射")
    {
        LcFreshRunKind::Kind(ProviderRunKind::WorkItemPlanOutlineRevision { feedback }) => {
            assert_eq!(feedback, None);
        }
        other => panic!("空 feedback 默认 None，got {other:?}"),
    }
    match lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::LegacyOutlineRebuild {
        business: business.clone(),
        origin: LcOutlineRebuildOrigin::OutlineRevision(LcLegacyOutlineRevisionPayload {
            normalized_feedback: "重建前返修反馈".to_string(),
            outline_ref: None,
            outline_version: None,
            outline_hash: None,
        }),
        previous_snapshot_access_fingerprint: Some("sha256:old".to_string()),
    })
    .expect("LegacyOutlineRebuild tag 必须可映射")
    {
        LcFreshRunKind::NeedsFreshPlanningContext => {}
        other => panic!("rebuild 需要 dispatch 时新解析 planning context，got {other:?}"),
    }
    match lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::LegacyDraft {
        business: business.clone(),
        generation_round_id: "round_0001".to_string(),
        active_outline_id: "outline_0001".to_string(),
        draft_refs: Vec::new(),
        pending_feedback: Some("draft 反馈".to_string()),
        outline_ref: None,
        candidate_ref: None,
    })
    .expect("LegacyDraft tag 必须可映射")
    {
        LcFreshRunKind::Kind(ProviderRunKind::WorkItemPlanDraft { feedback }) => {
            assert_eq!(feedback.as_deref(), Some("draft 反馈"));
        }
        other => panic!("LegacyDraft 精确 dispatch，got {other:?}"),
    }
    match lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::LegacyBatch {
        business: business.clone(),
        generation_round_id: "round_0001".to_string(),
        active_outline_id: "outline_0001".to_string(),
        draft_refs: Vec::new(),
        pending_feedback: None,
        outline_ref: None,
        candidate_ref: None,
        batch_id: "batch_0001".to_string(),
        batch_item_draft_refs: Vec::new(),
    })
    .expect("LegacyBatch tag 必须可映射")
    {
        LcFreshRunKind::Kind(ProviderRunKind::WorkItemPlanBatch) => {}
        other => panic!("LegacyBatch 精确 dispatch，got {other:?}"),
    }
    match lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::LegacyPlanRevision {
        business: business.clone(),
        candidate_ref: None,
        candidate_version: None,
        candidate_hash: None,
        retained_items: Vec::new(),
        redo_specs: Vec::new(),
        overall_feedback: Some("plan 反馈".to_string()),
    })
    .expect("LegacyPlanRevision tag 必须可映射")
    {
        LcFreshRunKind::Kind(ProviderRunKind::WorkItemPlanRevision { feedback }) => {
            assert_eq!(feedback.as_deref(), Some("plan 反馈"));
        }
        other => panic!("LegacyPlanRevision 精确 dispatch，got {other:?}"),
    }
    match lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::ScAuthor {
        business: business.clone(),
        flow: WorkItemPlanFlowKind::SingleCandidate,
        single_candidate_phase: SingleCandidatePhase::Generate,
        provider_start_reservation_key: None,
        source_revision_ref: None,
        ir_ref: None,
        report_ref: None,
        candidate_source_hash: None,
        business_material_identity: None,
    })
    .expect("ScAuthor tag 必须可映射")
    {
        LcFreshRunKind::Kind(ProviderRunKind::WorkItemPlanSingleCandidateAuthor) => {}
        other => panic!("ScAuthor 精确 dispatch，got {other:?}"),
    }
    match lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::ScManualRevision {
        business: business.clone(),
        turn_id: "turn_0001".to_string(),
        command_id: "cmd_0001".to_string(),
        source_hash: "sha256:src".to_string(),
        turn_status: HumanGateTurnStatus::Running,
        attempt_no: 1,
        budget_reserved: 1,
        human_gate_reservation_key: None,
        gate_snapshot_ref: None,
        source_revision_ref: None,
        feedback_text: Some("manual 反馈".to_string()),
        assembly_digest: None,
        assembly_ref: None,
    })
    .expect("ScManualRevision tag 必须可映射")
    {
        LcFreshRunKind::Kind(ProviderRunKind::HumanGateScManualRevision { turn_id, prompt }) => {
            assert_eq!(turn_id, "turn_0001");
            assert!(prompt.is_empty(), "prompt 由臂内按 durable turn 重建");
        }
        other => panic!("ScManualRevision 精确 dispatch，got {other:?}"),
    }

    // 非法 flow：typed 拒（context_unsupported），不回落通用 plan_author。
    let illegal =
        lc_fresh_run_kind_for_recovery_context(&LcRecoveryRunContext::LegacyOutlineAuthor {
            business,
            flow: WorkItemPlanFlowKind::SingleCandidate,
            original_provider_node_id: None,
            outline_ref: None,
        });
    let rejection = illegal.expect_err("非法 flow 必须拒收");
    assert_eq!(rejection.code, "lc_resume_recovery_context_unsupported");
}

// ============================================================================
// B2b3（Task 5.4，tasks.md 5.4 / 设计 §5.3 legacy_draft·legacy_batch·
// legacy_plan_revision 行）：Draft/Batch/PlanRevision 三 tag 的 full fresh
// 臂接管——显式 fresh 后同业务代恢复：feedback 为 durable 冻结值（take 前）、
// provider 全新启动（resume=None）、generation/outline/batch 不重建、
// PlanRevision 重算 retained/redo 与 payload 一致、原完成器/门保持、
// 未接管 tag typed 拒不回落。
// ============================================================================

use crate::product::lifecycle_store::CreateWorkItemInput;
use crate::product::models::{
    LcRecoveryDraftRef, LcRecoveryRedoSpec, LcRetainedPlanItemRef, ProviderConversationRef,
    WorkItemBatchRecord, WorkItemBatchStatus, WorkItemContextBudget, WorkItemDraftRecord,
    WorkItemDraftStatus, WorkItemGenerationMode, WorkItemKind, WorkItemPlanDraftActiveIndex,
    WorkItemPlanOutline, WorkItemPlanStatus, WorkItemOutline,
};
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::web::workspace_ws_types::{
    WorkItemCandidateDto, WorkItemCandidateMetaDto, WorkItemPlanCandidateDto, WorkItemPlanDto,
    WorkItemPlanOutlineCandidateDto, WorkItemSplitOptionsDto,
};
use std::collections::BTreeMap;

/// anchor_t16 业务夹具：T13 基座（物理仓+issue+story/design+plan+WorkItemPlan
/// 会话）+ draft/batch/revision 业务事实（outline artifact 由各用例注入 engine
/// session；WorkItemPlanStore index/draft 记录按场景落盘）。
struct AnchorT16PlanFixture {
    t13: AnchorT13PlanFixture,
}

impl AnchorT16PlanFixture {
    fn new() -> Self {
        Self {
            t13: AnchorT13PlanFixture::new(),
        }
    }

    fn lifecycle(&self) -> &LifecycleStore {
        &self.t13.lifecycle
    }

    fn app_paths(&self) -> &ProductAppPaths {
        &self.t13.app_paths
    }

    fn session_id(&self) -> &str {
        &self.t13.session_record.id
    }

    fn latest_record(&self) -> WorkspaceSessionRecord {
        self.t13.latest_record()
    }

    fn plan_store(&self) -> WorkItemPlanStore {
        WorkItemPlanStore::new(self.t13.app_paths.clone())
    }

    /// 单 outline（outline_a）的 draft 场景 index + accepted draft 记录。
    fn seed_serial_draft_business(&self) {
        let store = self.plan_store();
        let record = anchor_t16_draft_record("outline_a", "draft_outline_a", None);
        store.put_draft_record(&record).expect("put draft record");
        store
            .save_active_index(&anchor_t16_active_index(
                Some("outline_a".to_string()),
                BTreeMap::from([(
                    "outline_a".to_string(),
                    "draft_outline_a".to_string(),
                )]),
                BTreeMap::from([(
                    "draft_outline_a".to_string(),
                    WorkItemDraftStatus::Accepted,
                )]),
                Vec::new(),
            ))
            .expect("save active index");
    }

    /// batch 场景 index（batch_0001 Generating，active_outline 生产形态）+
    /// 三个 accepted batch draft 记录。
    fn seed_batch_business(&self) {
        let store = self.plan_store();
        let mut statuses = BTreeMap::new();
        let mut mapping = BTreeMap::new();
        for outline_id in ["outline_a", "outline_b", "outline_c"] {
            let draft_id = format!("draft_{outline_id}");
            store
                .put_draft_record(&anchor_t16_draft_record(
                    outline_id,
                    &draft_id,
                    Some("batch_0001"),
                ))
                .expect("put batch draft record");
            mapping.insert(outline_id.to_string(), draft_id.clone());
            statuses.insert(draft_id, WorkItemDraftStatus::Accepted);
        }
        store
            .save_active_index(&anchor_t16_active_index(
                Some("outline_a".to_string()),
                mapping,
                statuses,
                vec![WorkItemBatchRecord {
                    batch_id: "batch_0001".to_string(),
                    generation_round_id: "round_0001".to_string(),
                    mode: WorkItemGenerationMode::Batch,
                    item_draft_ids: vec![
                        "draft_outline_a".to_string(),
                        "draft_outline_b".to_string(),
                        "draft_outline_c".to_string(),
                    ],
                    status: WorkItemBatchStatus::Generating,
                    validation_failed_ids: Vec::new(),
                    created_at: "2026-10-12T00:00:00Z".to_string(),
                }],
            ))
            .expect("save active index");
    }

    /// revision 场景：两个 lifecycle work item（保留+重做），engine artifact
    /// 由用例注入 candidate（一个 reverted 带反馈）。
    fn seed_revision_work_items(&self, repository_id: &str) {
        for (id, title) in [
            ("wi_t16_keep", "保留项"),
            ("wi_t16_redo", "重做项"),
        ] {
            self.t13
                .lifecycle
                .create_work_item(CreateWorkItemInput {
                    id: Some(id.to_string()),
                    project_id: "project_0001".to_string(),
                    issue_id: "issue_0001".to_string(),
                    repository_id: repository_id.to_string(),
                    story_spec_ids: Vec::new(),
                    design_spec_ids: Vec::new(),
                    title: title.to_string(),
                    work_item_set_id: None,
                    source_work_item_plan_id: None,
                    source_outline_id: None,
                    source_draft_id: None,
                    planned_implementation_context: None,
                    kind: WorkItemKind::Backend,
                    sequence_hint: None,
                    depends_on: Vec::new(),
                    exclusive_write_scopes: Vec::new(),
                    forbidden_write_scopes: Vec::new(),
                    context_budget: WorkItemContextBudget::default(),
                    verification_plan_ref: None,
                    require_execution_plan_confirm: false,
                    plan_status: WorkItemPlanStatus::Draft,
                })
                .expect("create work item");
        }
    }

    /// 直接落 Waiting 恢复记录（捕获面归 5.1/manager；本包验证 fresh 消费）。
    fn seed_recovery_waiting(
        &self,
        recovery_id: &str,
        run_context: LcRecoveryRunContext,
    ) -> LcResumeRecoveryRecord {
        let expected = self.latest_record();
        let recovery = LcResumeRecoveryRecord {
            schema_version: 1,
            recovery_id: recovery_id.to_string(),
            failed_provider_node_id: "timeline_node_001".to_string(),
            reason_code: "planning_context_stale".to_string(),
            role: ProviderConversationRole::Author,
            provider: ProviderName::ClaudeCode,
            superseded_native_session_id: Some("native-t16-old".to_string()),
            previous_fingerprint_digest: Some("sha256:t16-old".to_string()),
            previous_session_status: expected.status.clone(),
            previous_stage: "running".to_string(),
            run_context,
            state: LcResumeRecoveryState::Waiting,
            operation_id: None,
            fresh_node_id: None,
            fresh_native_session_id: None,
            fresh_role_run_seq: None,
            fresh_operations: Vec::new(),
            detected_at: "2026-10-12T00:00:00Z".to_string(),
            updated_at: "2026-10-12T00:00:00Z".to_string(),
        };
        self.t13
            .lifecycle
            .compare_and_save_lc_resume_state(&expected, recovery.clone())
            .expect("seed t16 waiting recovery");
        recovery
    }

    /// stale 收口后的 engine 形态：stage=PrepareContext、无活动节点、无
    /// timeline 节点、artifact/candidate 由用例注入；注入旧 Author native
    /// conversation 引用（fresh 必须不回捞，resume=None 的对账锚点）。
    fn engine_after_stale_closure(
        &self,
        artifact: Option<crate::web::workspace_ws_types::ArtifactPayload>,
        checkpoint_dir: &str,
    ) -> Arc<Mutex<WorkspaceEngine>> {
        let record = self.latest_record();
        let mut session = WorkspaceSession::from_record(record.clone());
        session.stage = WorkspaceStage::PrepareContext;
        session.artifact = artifact;
        session.provider_conversations = vec![ProviderConversationRef {
            role: ProviderConversationRole::Author,
            provider: ProviderName::ClaudeCode,
            provider_session_id: "native-t16-old".to_string(),
            updated_at: "2026-10-12T00:00:00Z".to_string(),
            last_node_id: Some("timeline_node_001".to_string()),
        }];
        let checkpoint_store = Arc::new(CheckpointStore::new(
            self.t13.base.temp.path().join(checkpoint_dir),
        ));
        let (engine_tx, _engine_rx) = mpsc::channel::<EngineEvent>(64);
        let engine = WorkspaceEngine::new_persistent(
            checkpoint_store,
            self.t13.lifecycle.clone(),
            engine_tx,
            session,
        );
        Arc::new(Mutex::new(engine))
    }
}

fn anchor_t16_active_index(
    active_outline_id: Option<String>,
    outline_to_current_draft_id: BTreeMap<String, String>,
    draft_statuses: BTreeMap<String, WorkItemDraftStatus>,
    batches: Vec<WorkItemBatchRecord>,
) -> WorkItemPlanDraftActiveIndex {
    WorkItemPlanDraftActiveIndex {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: "issue_work_item_plan_0001".to_string(),
        current_generation_round_id: "round_0001".to_string(),
        outline_state: "confirmed".to_string(),
        active_outline_id,
        outline_to_current_draft_id,
        draft_statuses,
        batches,
        updated_at: "2026-10-12T00:00:00Z".to_string(),
    }
}

fn anchor_t16_work_item_plan_outline() -> WorkItemPlanOutline {
    WorkItemPlanOutline {
        id: "outline_001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        source_story_spec_ids: Vec::new(),
        source_design_spec_ids: Vec::new(),
        strategy_summary: "t16 strategy".to_string(),
        work_item_outlines: vec![
            WorkItemOutline {
                target_repository_id: None,
                outline_id: "outline_a".to_string(),
                logical_work_item_id: "wi_a".to_string(),
                title: "T16-A".to_string(),
                kind: WorkItemKind::Backend,
                goal: "T16-A goal".to_string(),
                scope: vec!["src/a.rs".to_string()],
                non_goals: Vec::new(),
                estimated_context_tokens: Some(12_000),
                session_fit: Some(
                    crate::product::models::WorkItemOutlineSessionFit::FitsSingleAgentSession,
                ),
                source_story_spec_ids: Vec::new(),
                source_design_spec_ids: Vec::new(),
                exclusive_write_scopes: vec!["src/a.rs".to_string()],
                forbidden_write_scopes: Vec::new(),
                depends_on: Vec::new(),
                verification_intent: vec!["cargo test --locked --lib t16".to_string()],
                trusted_verification_commands: Vec::new(),
                handoff_notes: "handoff A".to_string(),
            },
            WorkItemOutline {
                target_repository_id: None,
                outline_id: "outline_b".to_string(),
                logical_work_item_id: "wi_b".to_string(),
                title: "T16-B".to_string(),
                kind: WorkItemKind::Frontend,
                goal: "T16-B goal".to_string(),
                scope: vec!["web/b.ts".to_string()],
                non_goals: Vec::new(),
                estimated_context_tokens: Some(10_000),
                session_fit: Some(
                    crate::product::models::WorkItemOutlineSessionFit::FitsSingleAgentSession,
                ),
                source_story_spec_ids: Vec::new(),
                source_design_spec_ids: Vec::new(),
                exclusive_write_scopes: vec!["web/b.ts".to_string()],
                forbidden_write_scopes: Vec::new(),
                depends_on: Vec::new(),
                verification_intent: vec!["cargo test --locked --lib t16b".to_string()],
                trusted_verification_commands: Vec::new(),
                handoff_notes: "handoff B".to_string(),
            },
        ],
        dependency_graph: Vec::new(),
        risks: Vec::new(),
        handoff_strategy: String::new(),
        status: "confirmed".to_string(),
    }
}

fn anchor_t16_outline_artifact() -> crate::web::workspace_ws_types::ArtifactPayload {
    crate::web::workspace_ws_types::ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: anchor_t16_work_item_plan_outline(),
            design_context_gaps: Vec::new(),
            validator_findings: Vec::new(),
            context_blockers: Vec::new(),
            current_generation_round_id: Some("round_0001".to_string()),
            selected_generation_mode: None,
        }),
    }
}

fn anchor_t16_draft_record(
    outline_id: &str,
    draft_id: &str,
    batch_id: Option<&str>,
) -> WorkItemDraftRecord {
    let now = "2026-10-12T00:00:00Z";
    let logical_work_item_id = format!(
        "wi_{}",
        outline_id.strip_prefix("outline_").unwrap_or(outline_id)
    );
    let mut contract =
        crate::product::work_item_contract::canonical_contract_fixture(&logical_work_item_id);
    contract.identity.title = format!("{outline_id} draft");
    contract.identity.kind = "backend".to_string();
    contract.goal.summary = format!("实现 {outline_id}");
    WorkItemDraftRecord {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: "issue_work_item_plan_0001".to_string(),
        draft_id: draft_id.to_string(),
        outline_id: outline_id.to_string(),
        generation_round_id: "round_0001".to_string(),
        batch_id: batch_id.map(str::to_string),
        attempt_index: 1,
        outline_version_ref: "outline_001".to_string(),
        generation_mode: if batch_id.is_some() {
            WorkItemGenerationMode::Batch
        } else {
            WorkItemGenerationMode::Serial
        },
        generation_diagnostics: None,
        candidate: crate::product::models::WorkItemDraftCandidate {
            target_repository_id: None,
            outline_id: outline_id.to_string(),
            logical_work_item_id,
            canonical_contract_candidate: contract.clone(),
            verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                checks: contract.verification_checks,
            },
        },
        status: WorkItemDraftStatus::Accepted,
        active: true,
        superseded_by_draft_id: None,
        supersede_reason: None,
        copied_from_draft_id: None,
        review_node_id: None,
        review_verdict_ref: None,
        generated_from_node_id: "timeline_node_draft".to_string(),
        accepted_at: Some(now.to_string()),
        superseded_at: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    }
}

fn anchor_t16_plan_candidate_artifact() -> crate::web::workspace_ws_types::ArtifactPayload {
    crate::web::workspace_ws_types::ArtifactPayload::WorkItemPlanCandidate {
        candidate: Box::new(WorkItemPlanCandidateDto {
            plan: WorkItemPlanDto {
                id: "issue_work_item_plan_0001".to_string(),
                status: "draft".to_string(),
                options: WorkItemSplitOptionsDto {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                dependency_graph: Vec::new(),
            },
            work_items: vec![
                WorkItemCandidateDto {
                    id: "wi_t16_keep".to_string(),
                    kind: "backend".to_string(),
                    title: "保留项".to_string(),
                    depends_on: Vec::new(),
                    exclusive_write_scopes: Vec::new(),
                    verification_plan_ref: None,
                    meta: WorkItemCandidateMetaDto {
                        reverted: false,
                        revert_feedback: None,
                    },
                },
                WorkItemCandidateDto {
                    id: "wi_t16_redo".to_string(),
                    kind: "backend".to_string(),
                    title: "重做项".to_string(),
                    depends_on: Vec::new(),
                    exclusive_write_scopes: Vec::new(),
                    verification_plan_ref: None,
                    meta: WorkItemCandidateMetaDto {
                        reverted: true,
                        revert_feedback: Some("返修该组件".to_string()),
                    },
                },
            ],
            verification_plans: Vec::new(),
            repository_profile: None,
            validator_findings: Vec::new(),
        }),
    }
}

fn anchor_t16_business() -> LcRecoveryBusinessContext {
    anchor_t13_business("issue_work_item_plan_0001")
}

/// dispatch 快捷封装：seed Waiting → claim → 组 run_context → dispatch。
#[allow(clippy::too_many_arguments)]
async fn anchor_t16_claim_and_dispatch(
    fixture: &AnchorT16PlanFixture,
    recovery: &LcResumeRecoveryRecord,
    operation_id: &str,
    engine_arc: Arc<Mutex<WorkspaceEngine>>,
    input_tx: mpsc::UnboundedSender<StreamingProviderInput>,
) -> mpsc::Receiver<OutboundControl> {
    assert!(matches!(
        fixture
            .lifecycle()
            .claim_lc_fresh(fixture.session_id(), &recovery.recovery_id, operation_id),
        Ok(LcFreshClaimOutcome::Claimed { .. })
    ));
    let record = fixture.latest_record();
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(RebuildRecordingProvider { input_tx }),
    );
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine_arc,
        WorkspaceRunRegistry::default(),
        record.id.clone(),
        fixture.app_paths().clone(),
        fixture.latest_record(),
    );
    let (outbound_tx, outbound_rx) = mpsc::channel::<OutboundControl>(8);
    dispatch_lc_fresh_recovery_run(run_context, outbound_tx).await;
    outbound_rx
}

/// 读 frame 的 (type, code)（typed 拒/等待帧断言用）。
async fn anchor_t16_next_frame_code(
    outbound_rx: &mut mpsc::Receiver<OutboundControl>,
) -> (serde_json::Value, String) {
    let frame = tokio::time::timeout(std::time::Duration::from_secs(1), outbound_rx.recv())
        .await
        .expect("必须收到 typed 帧")
        .expect("outbound frame");
    match frame {
        OutboundControl::Text(text) => {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            let code = value["code"].as_str().unwrap_or_default().to_string();
            (value.clone(), code)
        }
        other => panic!("期望 Text 帧，got {other:?}"),
    }
}

#[tokio::test]
async fn anchor_t16_legacy_draft_batch_revision_fresh_preserves_business_generation() {
    let fixture = AnchorT16PlanFixture::new();
    fixture.seed_serial_draft_business();
    let feedback = "draft 冻结返修反馈（t16，take 前 durable）".to_string();
    let recovery = fixture.seed_recovery_waiting(
        "recovery_anchor_t16_draft",
        LcRecoveryRunContext::LegacyDraft {
            business: anchor_t16_business(),
            generation_round_id: "round_0001".to_string(),
            active_outline_id: "outline_a".to_string(),
            draft_refs: vec![LcRecoveryDraftRef {
                draft_id: "draft_outline_a".to_string(),
                status: WorkItemDraftStatus::Accepted,
                version: None,
                hash: None,
            }],
            pending_feedback: Some(feedback.clone()),
            outline_ref: Some("outline_001".to_string()),
            candidate_ref: None,
        },
    );
    let engine_arc = fixture.engine_after_stale_closure(
        Some(anchor_t16_outline_artifact()),
        "checkpoints-t16-draft",
    );
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let outbound_rx = anchor_t16_claim_and_dispatch(
        &fixture,
        &recovery,
        "op-t16-draft",
        engine_arc,
        input_tx,
    )
    .await;

    let input = tokio::time::timeout(std::time::Duration::from_secs(3), input_rx.recv())
        .await
        .expect("显式 fresh 授权后 Draft 臂必须启动 provider（resume=None 全新会话）")
        .expect("provider input");
    assert_eq!(
        input.resume_provider_session_id, None,
        "fresh 不回捞旧 native 会话（native-t16-old 在 conversation 引用中在场）"
    );
    assert!(
        input.prompt.contains(&feedback),
        "prompt 必须携带 durable 冻结 feedback（take 前）"
    );
    assert!(
        input.prompt.contains("T16-A"),
        "prompt 必须携带当前 outline 全量材料"
    );

    // 业务代同原：generation/outline/批次零重建（before==after）。
    let index = fixture
        .plan_store()
        .load_active_index("project_0001", "issue_0001", "issue_work_item_plan_0001")
        .expect("load index")
        .expect("index present");
    assert_eq!(index.current_generation_round_id, "round_0001");
    assert_eq!(index.active_outline_id.as_deref(), Some("outline_a"));
    assert!(index.batches.is_empty(), "draft fresh 不得新建 batch");
    assert_eq!(
        index.draft_statuses.get("draft_outline_a"),
        Some(&WorkItemDraftStatus::Accepted)
    );

    let after = fixture.latest_record();
    let recovery_after = after.lc_resume_recovery.expect("recovery kept");
    assert_eq!(recovery_after.state, LcResumeRecoveryState::FreshRunning);
    assert_eq!(recovery_after.fresh_operations.len(), 1);
    assert_eq!(
        recovery_after.fresh_operations[0].state,
        LcFreshOperationState::Running
    );
    // dispatch 后无第二 input（无重复 spawn）。
    assert!(input_rx.try_recv().is_err());
    drop(outbound_rx);
}

#[tokio::test]
async fn anchor_t16_legacy_batch_fresh_keeps_batch_and_budget() {
    let fixture = AnchorT16PlanFixture::new();
    fixture.seed_batch_business();
    let feedback = "batch 冻结返修反馈（t16）".to_string();
    let recovery = fixture.seed_recovery_waiting(
        "recovery_anchor_t16_batch",
        LcRecoveryRunContext::LegacyBatch {
            business: anchor_t16_business(),
            generation_round_id: "round_0001".to_string(),
            active_outline_id: "outline_a".to_string(),
            draft_refs: Vec::new(),
            pending_feedback: Some(feedback.clone()),
            outline_ref: Some("outline_001".to_string()),
            candidate_ref: None,
            batch_id: "batch_0001".to_string(),
            batch_item_draft_refs: vec![
                LcRecoveryDraftRef {
                    draft_id: "draft_outline_a".to_string(),
                    status: WorkItemDraftStatus::Accepted,
                    version: None,
                    hash: None,
                },
            ],
        },
    );
    let engine_arc = fixture.engine_after_stale_closure(
        Some(anchor_t16_outline_artifact()),
        "checkpoints-t16-batch",
    );
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let outbound_rx = anchor_t16_claim_and_dispatch(
        &fixture,
        &recovery,
        "op-t16-batch",
        engine_arc,
        input_tx,
    )
    .await;

    let input = tokio::time::timeout(std::time::Duration::from_secs(3), input_rx.recv())
        .await
        .expect("显式 fresh 授权后 Batch 臂必须启动 provider")
        .expect("provider input");
    assert_eq!(input.resume_provider_session_id, None);
    assert!(input.prompt.contains(&feedback));

    // 同 batch：不另建 batch、generation 不重建、draft 状态同原。
    let index = fixture
        .plan_store()
        .load_active_index("project_0001", "issue_0001", "issue_work_item_plan_0001")
        .expect("load index")
        .expect("index present");
    assert_eq!(index.current_generation_round_id, "round_0001");
    assert_eq!(index.batches.len(), 1, "batch fresh 不得另建 batch");
    assert_eq!(index.batches[0].batch_id, "batch_0001");
    assert_eq!(index.batches[0].item_draft_ids.len(), 3);
    assert_eq!(
        index.draft_statuses.get("draft_outline_b"),
        Some(&WorkItemDraftStatus::Accepted)
    );

    let after = fixture.latest_record();
    assert_eq!(
        after.lc_resume_recovery.expect("recovery kept").state,
        LcResumeRecoveryState::FreshRunning
    );
    assert!(input_rx.try_recv().is_err());
    drop(outbound_rx);
}

#[tokio::test]
async fn anchor_t16_legacy_plan_revision_fresh_recomputes_matching_payload() {
    let fixture = AnchorT16PlanFixture::new();
    fixture.seed_revision_work_items("repository_0001");
    let feedback = "plan 整体返修反馈（t16）".to_string();
    let recovery = fixture.seed_recovery_waiting(
        "recovery_anchor_t16_revision",
        LcRecoveryRunContext::LegacyPlanRevision {
            business: anchor_t16_business(),
            candidate_ref: None,
            candidate_version: None,
            candidate_hash: None,
            retained_items: vec![LcRetainedPlanItemRef {
                item_id: "wi_t16_keep".to_string(),
                revision: 0,
            }],
            redo_specs: vec![LcRecoveryRedoSpec {
                old_item_id: "wi_t16_redo".to_string(),
                feedback: Some(
                    "返修该组件\n\n整体反馈: ".to_string() + &feedback,
                ),
            }],
            overall_feedback: Some(feedback.clone()),
        },
    );
    let engine_arc = fixture.engine_after_stale_closure(
        Some(anchor_t16_plan_candidate_artifact()),
        "checkpoints-t16-revision",
    );
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let mut outbound_rx = anchor_t16_claim_and_dispatch(
        &fixture,
        &recovery,
        "op-t16-revision",
        engine_arc,
        input_tx,
    )
    .await;

    let input = tokio::time::timeout(std::time::Duration::from_secs(3), input_rx.recv())
        .await
        .expect(
            "显式 fresh 授权后 PlanRevision 臂必须在 retained/redo 重算与 payload 一致时启动 provider",
        )
        .expect("provider input");
    assert_eq!(input.resume_provider_session_id, None);
    assert!(input.prompt.contains(&feedback));

    let after = fixture.latest_record();
    assert_eq!(
        after.lc_resume_recovery.expect("recovery kept").state,
        LcResumeRecoveryState::FreshRunning
    );
    assert!(input_rx.try_recv().is_err());
    drop(outbound_rx);
}

#[tokio::test]
async fn anchor_t16_fresh_draft_business_mismatch_waits_typed_stale() {
    let fixture = AnchorT16PlanFixture::new();
    fixture.seed_serial_draft_business();
    // payload 携带旧业务代（round_9999）——与 durable index（round_0001）不一致。
    let recovery = fixture.seed_recovery_waiting(
        "recovery_anchor_t16_draft_mismatch",
        LcRecoveryRunContext::LegacyDraft {
            business: anchor_t16_business(),
            generation_round_id: "round_9999".to_string(),
            active_outline_id: "outline_a".to_string(),
            draft_refs: Vec::new(),
            pending_feedback: Some("过期反馈".to_string()),
            outline_ref: None,
            candidate_ref: None,
        },
    );
    let engine_arc = fixture.engine_after_stale_closure(
        Some(anchor_t16_outline_artifact()),
        "checkpoints-t16-draft-mismatch",
    );
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let mut outbound_rx = anchor_t16_claim_and_dispatch(
        &fixture,
        &recovery,
        "op-t16-draft-mismatch",
        engine_arc,
        input_tx,
    )
    .await;

    let (_frame, code) = anchor_t16_next_frame_code(&mut outbound_rx).await;
    assert_eq!(code, "lc_resume_recovery_stale");
    let after = fixture.latest_record();
    let recovery = after.lc_resume_recovery.expect("recovery kept");
    assert_eq!(recovery.state, LcResumeRecoveryState::Waiting);
    assert_eq!(
        recovery.fresh_operations[0].state,
        LcFreshOperationState::Failed
    );
    // 零 spawn 判定：窗口内无输入，或通道在无输入情况下关闭（registry 随
    // run_context 在 dispatch 返回时释放，recv 得 None 同样证明零输入）。
    match tokio::time::timeout(std::time::Duration::from_millis(300), input_rx.recv()).await {
        Err(_) | Ok(None) => {}
        Ok(Some(_)) => panic!("业务代不一致必须零 spawn"),
    }
}

#[tokio::test]
async fn anchor_t16_fresh_plan_revision_payload_mismatch_waits_typed_stale() {
    let fixture = AnchorT16PlanFixture::new();
    fixture.seed_revision_work_items("repository_0001");
    // payload redo feedback 与 candidate 重算（revert_feedback＋整体反馈组合）不一致。
    let recovery = fixture.seed_recovery_waiting(
        "recovery_anchor_t16_revision_mismatch",
        LcRecoveryRunContext::LegacyPlanRevision {
            business: anchor_t16_business(),
            candidate_ref: None,
            candidate_version: None,
            candidate_hash: None,
            retained_items: vec![LcRetainedPlanItemRef {
                item_id: "wi_t16_keep".to_string(),
                revision: 0,
            }],
            redo_specs: vec![LcRecoveryRedoSpec {
                old_item_id: "wi_t16_redo".to_string(),
                feedback: Some("过期 redo 反馈".to_string()),
            }],
            overall_feedback: Some("plan 整体返修反馈（t16）".to_string()),
        },
    );
    let engine_arc = fixture.engine_after_stale_closure(
        Some(anchor_t16_plan_candidate_artifact()),
        "checkpoints-t16-revision-mismatch",
    );
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let mut outbound_rx = anchor_t16_claim_and_dispatch(
        &fixture,
        &recovery,
        "op-t16-revision-mismatch",
        engine_arc,
        input_tx,
    )
    .await;

    let (_frame, code) = anchor_t16_next_frame_code(&mut outbound_rx).await;
    assert_eq!(
        code, "lc_resume_recovery_stale",
        "retained/redo 重算与 payload 不一致必须 typed Stale 拒收（不用旧权限强行完成）"
    );
    let after = fixture.latest_record();
    let recovery = after.lc_resume_recovery.expect("recovery kept");
    assert_eq!(recovery.state, LcResumeRecoveryState::Waiting);
    assert_eq!(
        recovery.fresh_operations[0].state,
        LcFreshOperationState::Failed
    );
    // 零 spawn 判定：窗口内无输入，或通道在无输入情况下关闭（registry 随
    // run_context 在 dispatch 返回时释放，recv 得 None 同样证明零输入）。
    match tokio::time::timeout(std::time::Duration::from_millis(300), input_rx.recv()).await {
        Err(_) | Ok(None) => {}
        Ok(Some(_)) => panic!("payload 对照失败必须零 provider start"),
    }
}

#[tokio::test]
async fn anchor_t16_untaken_tag_still_typed_rejected_no_fallback() {
    let fixture = AnchorT16PlanFixture::new();
    // 未接管 tag（sc_author 归 B2c/5.5）：typed 拒，不回落通用 plan_author。
    let recovery = fixture.seed_recovery_waiting(
        "recovery_anchor_t16_sc_author",
        LcRecoveryRunContext::ScAuthor {
            business: anchor_t16_business(),
            flow: WorkItemPlanFlowKind::SingleCandidate,
            single_candidate_phase: SingleCandidatePhase::Generate,
            provider_start_reservation_key: None,
            source_revision_ref: None,
            ir_ref: None,
            report_ref: None,
            candidate_source_hash: None,
            business_material_identity: None,
        },
    );
    let engine_arc =
        fixture.engine_after_stale_closure(None, "checkpoints-t16-sc-author");
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let mut outbound_rx = anchor_t16_claim_and_dispatch(
        &fixture,
        &recovery,
        "op-t16-sc-author",
        engine_arc,
        input_tx,
    )
    .await;

    let (_frame, code) = anchor_t16_next_frame_code(&mut outbound_rx).await;
    assert_eq!(code, "lc_resume_recovery_context_unsupported");
    let after = fixture.latest_record();
    let recovery = after.lc_resume_recovery.expect("recovery kept");
    assert_eq!(recovery.state, LcResumeRecoveryState::Waiting);
    assert_eq!(
        recovery.fresh_operations[0].state,
        LcFreshOperationState::Failed
    );
    // 零 spawn 判定：窗口内无输入，或通道在无输入情况下关闭（registry 随
    // run_context 在 dispatch 返回时释放，recv 得 None 同样证明零输入）。
    match tokio::time::timeout(std::time::Duration::from_millis(300), input_rx.recv()).await {
        Err(_) | Ok(None) => {}
        Ok(Some(_)) => panic!("未接管 tag 必须零 spawn"),
    }
}
