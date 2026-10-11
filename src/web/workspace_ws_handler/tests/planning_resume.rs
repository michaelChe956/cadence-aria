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
