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
        Self {
            paths: ProductAppPaths::new(temp.path()),
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
        PlanningContextResolver::new(self.paths.clone())
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
            events: event_rx,
            commands: command_tx,
        })
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
fn planning_resume_fail_closed_when_manifest_without_selection() {
    // 有 manifest、无 selection → 与 compile 一致 fail-closed（当前返回 Ok(None)）。
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

    let error = planning_resume_decision(&paths, "project_0001", "issue_0001").unwrap_err();
    assert!(error.contains("repository_routing_target_missing"));
    assert!(error.contains("work_item_target_missing"));
}

#[test]
fn planning_resume_legacy_when_none_none() {
    // 无 manifest、无 selection → Ok(None)，传统单仓不受影响。
    let root = tempfile::tempdir().unwrap();
    let result = planning_resume_decision(
        &ProductAppPaths::new(root.path().join(".aria")),
        "project_0001",
        "issue_0001",
    );
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
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

#[test]
fn planning_resume_run_kind_rebuilds_on_stale_context_and_resumes_on_same() {
    // B3 修复：StaleContext 时 Web 分支以 `WorkItemPlanOutlineRebuild` 启动全新 run，
    // 携带 rebuilt planning context（cwd/inventory/snapshot），不沿用可能复用旧 provider
    // 会话内容的 revision run kind；None（传统单仓）/ SameContext 沿用 fallback 续跑。
    let fixture = PlanningResumeFixture::new();
    fixture.write_logical_codebase();
    let resolver = fixture.resolver();
    let first = resolver.build("project_0001", "issue_0001", &[]).unwrap();

    let same = resolver.resume("project_0001", "issue_0001").unwrap();
    let kind = planning_resume_run_kind(
        &Some(same),
        ProviderRunKind::WorkItemPlanOutlineRevision { feedback: None },
    );
    assert!(matches!(
        kind,
        ProviderRunKind::WorkItemPlanOutlineRevision { .. }
    ));

    fixture.change_membership_revision();
    let stale = resolver.resume("project_0001", "issue_0001").unwrap();
    let kind = planning_resume_run_kind(
        &Some(stale),
        ProviderRunKind::WorkItemPlanOutlineRevision { feedback: None },
    );
    // B3：携带 rebuilt context —— cwd 与当前解析一致，指纹已重建（漂移），inventory 已注入。
    match kind {
        ProviderRunKind::WorkItemPlanOutlineRebuild { rebuilt } => {
            assert_eq!(rebuilt.cwd, first.cwd);
            assert_ne!(
                rebuilt.snapshot.access_fingerprint,
                first.snapshot.access_fingerprint
            );
            assert!(!rebuilt.inventory_injection.rendered.is_empty());
        }
        _ => panic!("expected WorkItemPlanOutlineRebuild carrying rebuilt context"),
    }

    let kind = planning_resume_run_kind(
        &None,
        ProviderRunKind::WorkItemPlanOutlineRevision { feedback: None },
    );
    assert!(matches!(
        kind,
        ProviderRunKind::WorkItemPlanOutlineRevision { .. }
    ));
}

#[test]
fn rebuilt_snapshot_committed_only_after_provider_start_success() {
    // 新 BLOCKER 修复：rebuilt snapshot 仅在 provider 成功启动后 commit。
    // provider 启动失败不落盘 —— 重连仍 StaleContext（避免再次 TOCTOU）。
    let fixture = PlanningResumeFixture::new();
    fixture.write_logical_codebase();
    let resolver = fixture.resolver();
    resolver.build("project_0001", "issue_0001", &[]).unwrap();

    fixture.change_membership_revision();
    let stale = resolver.resume("project_0001", "issue_0001").unwrap();
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
    let again = resolver.resume("project_0001", "issue_0001").unwrap();
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
    let resumed = resolver.resume("project_0001", "issue_0001").unwrap();
    assert!(matches!(resumed, ResumeDecision::SameContext(_)));
}

#[tokio::test]
async fn stale_context_rebuild_starts_new_outline_run_with_rebuilt_context() {
    // B3 修复：StaleContext 真正启动新 run —— 使用 rebuilt context 的 cwd（聚合根）作为
    // worktree、注入 inventory/effective members 到 prompt、新建 OutlineRun 节点（不沿用
    // 中断会话节点），并在 provider 成功启动后 commit rebuilt snapshot。
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::lifecycle_store::{
        CreateDesignSpecInput, CreateIssueWorkItemPlanInput, CreateStorySpecInput,
        CreateWorkspaceSessionInput,
    };
    use crate::product::models::{
        IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, WorkspaceType as ProductWorkspaceType,
    };
    use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
    use crate::web::workspace_ws_types::{
        ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType,
        WorkspaceStage as WsWorkspaceStage,
    };

    let mut fixture = PlanningResumeFixture::new();
    let app_paths = fixture.paths.clone();
    // 物理仓库 + issue：必须在 write_logical_codebase 之前创建，否则 selection 会先创建
    // `issues/issue_0001/` 目录，导致 IssueStore 自动 id 变为 issue_0002（而 session 的
    // issue_id 固定为 issue_0001）。
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
            idempotency_key: "planning-resume-rebuild-repository".to_string(),
        })
        .unwrap();
    crate::product::repository_store::RepositoryStore::with_logical_codebase_feature(
        app_paths.clone(),
        crate::product::logical_codebase::LogicalCodebaseFeature::enabled(),
    )
    .ensure_identity_schema("project_0001")
    .unwrap();
    fixture.update_api_member_identity_from_authority();
    let issue = IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some(repository.id.clone()),
            title: "Rebuild planning".to_string(),
            description: None,
            change_id: None,
        })
        .unwrap();
    assert_eq!(issue.id, "issue_0001");

    fixture.write_logical_codebase();
    let resolver = fixture.resolver();
    resolver.build("project_0001", "issue_0001", &[]).unwrap();
    fixture.change_membership_revision();
    let stale = resolver.resume("project_0001", "issue_0001").unwrap();
    let ResumeDecision::StaleContext { rebuilt, .. } = stale else {
        panic!("expected StaleContext");
    };

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
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .unwrap();

    // 中断的 OutlineRun 节点（reconnect 场景）：B3 必须新建节点而非沿用。
    let interrupted_node_id = "timeline_node_001".to_string();
    let mut session = WorkspaceSession::from_record(session_record.clone());
    session.stage = WorkspaceStage::Running;
    let checkpoint_store = Arc::new(CheckpointStore::new(
        fixture.temp.path().join("checkpoints"),
    ));
    let (engine_tx, mut engine_rx) = mpsc::channel::<EngineEvent>(64);
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle, engine_tx, session);
    engine.timeline_nodes = vec![TimelineNode {
        node_id: interrupted_node_id.clone(),
        node_type: TimelineNodeType::WorkItemPlanOutlineRun,
        agent: Some(ProviderName::ClaudeCode),
        stage: WsWorkspaceStage::Running,
        round: Some(1),
        status: TimelineNodeStatus::Active,
        title: interrupted_node_id.clone(),
        summary: Some("连接断开，运行已中止".to_string()),
        started_at: "2026-08-10T00:00:00Z".to_string(),
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
    let engine = Arc::new(Mutex::new(engine));

    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(RebuildRecordingProvider { input_tx }),
    );
    let current_run = Arc::new(Mutex::new(None));
    let workspace_runs = WorkspaceRunRegistry::default();
    let run_context = ProviderRunContext {
        provider_registry: Arc::new(registry),
        engine: engine.clone(),
        current_run: current_run.clone(),
        workspace_runs: workspace_runs.clone(),
        session_id: session_record.id.clone(),
        next_run_id: Arc::new(Mutex::new(0)),
        app_paths: app_paths.clone(),
        session_record: session_record.clone(),
    };
    let (outbound_tx, _outbound_rx) = mpsc::channel::<OutboundControl>(8);

    spawn_provider_run_from_handler(
        run_context,
        ProviderRunKind::WorkItemPlanOutlineRebuild {
            rebuilt: Box::new(rebuilt.clone()),
        },
        outbound_tx,
    )
    .await
    .expect("rebuild run should start");

    let input = tokio::time::timeout(std::time::Duration::from_secs(3), input_rx.recv())
        .await
        .expect("rebuild provider input should be sent")
        .expect("rebuild provider input");
    // B3：使用 rebuilt cwd（聚合根/provider_context_root）作为新 run worktree。
    assert_eq!(input.working_dir, rebuilt.cwd);
    // B3：注入 rebuilt inventory/effective members 到 prompt（不沿用旧内容）。
    assert!(input.prompt.contains("聚合代码库成员清单"));
    assert!(input.prompt.contains("target_repository_id"));
    // B3 round3：rebuild run 的 provider 全新启动 —— 不携带 provider_resume_session_id
    // （不 --resume 旧 Author provider 会话，真正隔离的全新规划会话）。
    assert_eq!(
        input.resume_provider_session_id, None,
        "rebuild run must not resume the old author provider session"
    );

    // 新 BLOCKER 修复：provider 成功启动后 rebuilt snapshot 已 commit。
    let store = PlanningContextSnapshotStore::new(app_paths.clone());
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let persisted = store.load("project_0001", "issue_0001").unwrap();
            if persisted.as_ref().map(|s| s.access_fingerprint.clone())
                == Some(rebuilt.snapshot.access_fingerprint.clone())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("rebuilt snapshot should be committed after provider start");

    // B3：不沿用中断 OutlineRun 节点 —— 重建 run 新建节点。通过 engine 事件通道断言
    // （run 在 provider 驱动阶段持有 engine 锁，不能直接 lock 断言）：
    // 首个 TimelineNodeCreated 必须是全新的 WorkItemPlanOutlineRun 节点（id ≠ 中断节点）。
    let fresh_node = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match engine_rx.recv().await {
                Some(EngineEvent::TimelineNodeCreated { node }) => {
                    if node.node_type == TimelineNodeType::WorkItemPlanOutlineRun
                        && node.node_id != interrupted_node_id
                    {
                        break node;
                    }
                }
                Some(_) => continue,
                None => panic!("engine event channel closed before fresh outline node"),
            }
        }
    })
    .await
    .expect("rebuild run must create a fresh WorkItemPlanOutlineRun node");
    assert_ne!(fresh_node.node_id, interrupted_node_id);
    assert_eq!(
        fresh_node.node_type,
        TimelineNodeType::WorkItemPlanOutlineRun
    );
}
