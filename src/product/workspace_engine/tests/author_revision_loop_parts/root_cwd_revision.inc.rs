// Task 2.3（lc-root-initialization）测试物理拆分（1200 行守卫）：经 include!
// 挂载，模块域与 author_revision_loop.rs 相同。
// ============================================================================
// Task 2.3（lc-root-initialization，REQ-PLN-03/PLN-07、REQ-ENV-04/ENV-10）：
// LC revision root-cwd 分流（映射 openspec tasks 2.3 Revision/ReviewOnly 面）。
//
// - `logical_revision_rebinds_root_cwd_and_preserves_revision_target`：LC
//   revision 必须经 gateway 启动，cwd 重绑 canonical root，target 保持独立
//   成员 checkout（不从 cwd 推导），author 工具策略（deny_file_write_builtins
//   + durable audit sink）不放宽，直连 sentinel 零启动。
// - `revision_resume_cwd_drift_supersedes_session`：resume 决策消费 cwd
//   inclusive launch fingerprint——root cwd 漂移时 supersede 旧 native
//   session 并以新 session 启动（resume id 丢弃）；指纹一致时 native session
//   identity 原样续接。
//
// 夹具登记链与 web 侧 `register_alias_logical_codebase` 同构（record/manifest/
// member/checkout/selection/policy + repos strict 投影 + story aggregate scope）。
// ============================================================================

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession, ProviderToolPolicy,
    StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::issue_store::CreateProductIssueInput;
use crate::product::lifecycle_store::{
    AggregateStorySpecScope, CreateStorySpecInput, CreateWorkspaceSessionInput, LifecycleStore,
};
use crate::product::logical_codebase::issue_selection::IssueCodebaseSelection;
use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::store::LogicalCodebaseRecord;
use crate::product::logical_codebase::types::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalRepositoryId, MemberStatus,
    RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
use crate::product::logical_codebase::{
    GatewayRunAudit, IssueCodebaseSelectionStore, LogicalCodebaseManifest,
    LogicalCodebaseProviderGateway, LogicalCodebaseStore,
};
use crate::product::models::{ProviderConversationRef, ProviderConversationRole};
use crate::product::project_store::CreateProjectInput;
use crate::product::repository_store::CreateRepositoryInput;

/// 记录启动 input 并以完整 Story 产物立即完成的 capture provider——注册进
/// gateway registry，是 LC revision 经 gateway 启动时唯一的真实 adapter。
struct RevisionCaptureProvider {
    starts: Arc<AtomicUsize>,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
    output: &'static str,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RevisionCaptureProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let _ = self.inputs.send(input);
        let output = self.output.to_string();
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    output, None,
                )))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    /// Task 7 分流收口:gateway 只经 validated trait 分发。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<
        mpsc::Receiver<crate::cross_cutting::streaming_provider::StreamChunk>,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        unreachable!("workspace revision tests use start")
    }
}

/// 直连 sentinel：Legacy 直连路径若被误走将计数（LC 会话下必须恒 0）。
struct RevisionDirectSentinel {
    starts: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RevisionDirectSentinel {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    "# sentinel".to_string(),
                    None,
                )))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    /// Task 7 分流收口:gateway 只经 validated trait 分发(sentinel 语义
    /// 保持——LC 会话误走直连仍计数)。
    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<
        mpsc::Receiver<crate::cross_cutting::streaming_provider::StreamChunk>,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        unreachable!("workspace revision tests use start")
    }
}

/// 修订产物必须通过 Completed 分支的 artifact gate（必需小节 + source id +
/// [REQ-*]/[AC-*]），否则 finish_failed_run 回 PrepareContext。
fn revision_story_output() -> &'static str {
    "# Story Spec\n\n\
        ## 范围\n来源 source id: Issue issue_0001；修订后产物：补充异常场景。\n\n\
        ## 用户故事\n作为用户，我希望异常场景有明确处理。\n\n\
        ## 功能需求\n- [REQ-001] 登录成功路径。\n- [REQ-002] 补充异常场景。\n\n\
        ## 成功标准\n- [AC-001] 覆盖异常场景。\n\n\
        ## 待确认项\n无。\n\n\
        ## 非功能需求\n无。\n\n\
        ## 改动摘要\n- 补充异常场景 [REQ-002]\n"
}

/// 与 web 侧 `init_ws_test_git_repo` 同构（跨模块不可见，本地复制）：main
/// 分支 + 初始空提交——基线解析（REQ-PIB-02）对裸目录 fail-closed 的夹具前提。
fn init_revision_git_repo(repo: &std::path::Path) {
    for args in [
        vec!["init", "--initial-branch", "main"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test User"],
        vec!["commit", "--allow-empty", "-m", "fixture baseline"],
    ] {
        let status = std::process::Command::new("git")
            .args(&args)
            .current_dir(repo)
            .status()
            .unwrap_or_else(|error| panic!("git {} failed to start: {error}", args.join(" ")));
        assert!(status.success(), "git {} failed", args.join(" "));
    }
}

struct LogicalRevisionFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    lifecycle: LifecycleStore,
    record: WorkspaceSessionRecord,
    gateway: Arc<LogicalCodebaseProviderGateway>,
    audit: Arc<GatewayRunAudit>,
    manifest: LogicalCodebaseManifest,
    member_id: LogicalRepositoryId,
    /// 成员 checkout（target）路径：与 manifest root（cwd）分离。
    member_root: std::path::PathBuf,
    /// 当前 manifest 冻结的 provider_context_root（cwd 来源）。
    manifest_root: std::path::PathBuf,
    /// 漂移备选 root（cwd 漂移测试用，位于 authority 允许范围内）。
    drift_root: std::path::PathBuf,
    provider_starts: Arc<AtomicUsize>,
}

/// gateway 冻结 authority root 的口径：`ManifestRoot`=canonical manifest root
///（生产形态）；`ParentOfManifestRoot`=canonical 父目录（cwd 漂移测试——同一
/// gateway 须放行漂移前后两个 root；生产 root 恒等断言属 it_web 双工厂
/// envelope 面，不在引擎层夹具重复）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogicalRevisionAuthority {
    ManifestRoot,
    ParentOfManifestRoot,
}

fn logical_revision_fixture(
    authority: LogicalRevisionAuthority,
) -> (
    LogicalRevisionFixture,
    mpsc::UnboundedReceiver<StreamingProviderInput>,
) {
    let root = tempfile::tempdir().expect("temporary workspace root");
    // 成员 checkout 位于聚合根之下（CommonNonGitParent 布局）；漂移备选 root
    // 为兄弟目录（同属 authority 范围，成员/政策/能力不变——只有 cwd 漂移）。
    let manifest_root = root.path().join("lc-root");
    let member_root = manifest_root.join("member-checkout");
    let drift_root = root.path().join("lc-root-b");
    std::fs::create_dir_all(&member_root).expect("create member checkout");
    std::fs::create_dir_all(&drift_root).expect("create drift root");
    init_revision_git_repo(&member_root);
    let authority_root = match authority {
        LogicalRevisionAuthority::ManifestRoot => {
            std::fs::canonicalize(&manifest_root).expect("canonical manifest root")
        }
        LogicalRevisionAuthority::ParentOfManifestRoot => {
            std::fs::canonicalize(root.path()).expect("canonical authority parent")
        }
    };

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    crate::product::project_store::ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "logical revision fixture".to_string(),
            description: None,
        })
        .expect("create project");
    let repository = crate::product::repository_store::RepositoryStore::new(paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "logical revision fixture repository".to_string(),
            path: member_root.clone(),
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: "logical-revision-fixture".to_string(),
        })
        .expect("create repository");
    // issue 先落 issue_0001（selection 会占用 issues/ 顺序 id 目录名）。
    crate::product::issue_store::IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some(repository.id.clone()),
            logical_codebase_id: Some(
                crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001"),
            ),
            title: "logical revision fixture issue".to_string(),
            description: Some("task 2.3 revision root cwd".to_string()),
            change_id: None,
            base_branch: None,
        })
        .expect("create issue");

    // LC 权威记录（alias 位置）+ manifest/member/checkout + 显式单成员 selection。
    let lc_id = crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001");
    let record_root = paths.logical_codebase_record_root("project_0001", &lc_id);
    std::fs::create_dir_all(&record_root).expect("create lc record root");
    std::fs::write(
        record_root.join("record.json"),
        serde_json::to_vec_pretty(&LogicalCodebaseRecord {
            id: lc_id.clone(),
            name: "logical revision alias lc".to_string(),
            aggregate_root: manifest_root.clone(),
            created_at: "2026-10-02T00:00:00Z".to_string(),
        })
        .expect("serialize lc record"),
    )
    .expect("write lc record.json");

    let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
    let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
    let authority = LogicalCodebaseStore::new(paths.clone());
    let manifest =
        LogicalCodebaseManifest::new("project_0001", manifest_root.clone(), vec![member_id]);
    authority
        .save_manifest("project_0001", &manifest)
        .expect("save manifest");
    let now = "2026-10-02T00:00:00Z".to_string();
    authority
        .save_member(
            "project_0001",
            &CodebaseMemberRecord {
                logical_repository_id: member_id,
                physical_repository_id: repository.id.clone(),
                alias: "revision-member".to_string(),
                role: "member".to_string(),
                ordinal: 1,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &member_root,
                    member_root.join(".git"),
                    Some("ssh://git@example.test/revision/member.git".to_string()),
                ),
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
        .expect("save member");
    authority
        .save_checkout(
            "project_0001",
            &RepositoryCheckoutRecord {
                checkout_id,
                logical_repository_id: member_id,
                physical_repository_id: repository.id.clone(),
                kind: CheckoutKind::Main,
                canonical_path: member_root.clone(),
                checkout_path_hash: "sha256:logical-revision-checkout".to_string(),
                git_dir_identity: "sha256:logical-revision-git-dir".to_string(),
                revision: Some("abc123".to_string()),
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now.clone(),
            },
        )
        .expect("save checkout");
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![member_id],
            Vec::new(),
            Vec::new(),
            None,
        ))
        .expect("save issue selection");
    AggregatePolicyArtifactStore::new(paths.clone())
        .ensure_bootstrap(&manifest)
        .expect("bootstrap aggregate policy");

    // strict 三层身份投影：repos.json 升级为已登记形态（logical id/checkout
    // id/identity_schema_version 对齐）。
    let mut projection = repository.clone();
    projection.logical_repository_id = Some(member_id);
    projection.primary_checkout_id = Some(checkout_id);
    projection.identity_schema_version = 1;
    std::fs::write(
        paths.project_root("project_0001").join("repos.json"),
        serde_json::to_vec_pretty(&vec![projection]).expect("serialize repos projection"),
    )
    .expect("rewrite repos.json");

    // Story aggregate scope（focus=唯一成员）+ 持久会话记录。
    let lifecycle = LifecycleStore::new(paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: repository.id.clone(),
            title: "logical revision story".to_string(),
            aggregate_codebase: Some(AggregateStorySpecScope {
                logical_codebase_ref: manifest.logical_codebase_id,
                effective_member_ids: vec![member_id],
                involved_repository_ids: vec![member_id],
                focus_repository_id: Some(member_id),
            }),
        })
        .expect("create story");
    let record = lifecycle
        .create_workspace_session_with_id(
            CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: story.id,
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::ClaudeCode),
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
            format!(
                "workspace_session_logical_revision_{}",
                uuid::Uuid::new_v4().simple()
            ),
        )
        .expect("create workspace session");

    // gateway：capture provider 注册为 ClaudeCode 真实 adapter（LC revision 经
    // gateway 启动）；capability/target resolver 复用 part_32 测试 doubles。
    let (starts, inputs, capture) = {
        let starts = Arc::new(AtomicUsize::new(0));
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        (
            starts.clone(),
            input_rx,
            Arc::new(RevisionCaptureProvider {
                starts,
                inputs: input_tx,
                output: revision_story_output(),
            }),
        )
    };
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, capture);
    let audit = Arc::new(GatewayRunAudit::new());
    let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
        AggregatePolicyArtifactStore::new(paths.clone()),
        Arc::new(super::ReviewStaticCapabilitySource::default()),
        Arc::new(super::ReviewPassThroughTargetResolver),
        Arc::new(registry),
        Arc::new(super::ReviewStubSyncAdapter),
        super::review_always_available_gate(),
        audit.clone(),
        authority_root,
    ));

    (
        LogicalRevisionFixture {
            _root: root,
            paths,
            lifecycle,
            record,
            gateway,
            audit,
            manifest,
            member_id,
            member_root,
            manifest_root,
            drift_root,
            provider_starts: starts,
        },
        inputs,
    )
}

/// Revision 态的持久 LC engine（gateway 注入；repository_path=成员 checkout，
/// 即 target worktree——与 cwd 来源分离）。
fn revision_stage_engine(fixture: &LogicalRevisionFixture) -> WorkspaceEngine {
    let mut session = WorkspaceSession::from_record(fixture.record.clone());
    session.repository_path = Some(fixture.member_root.clone());
    let (tx, _) = mpsc::channel(8);
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(
            fixture.paths.root().join("checkpoints"),
        )),
        fixture.lifecycle.clone(),
        tx,
        session,
    )
    .with_logical_provider_gateway(fixture.gateway.clone());
    engine.session.stage = WorkspaceStage::Revision;
    engine.session.artifact = Some(artifact_payload("# Story Spec\n\n修订前产物"));
    engine.pending_revision_context = Some("补充异常场景".to_string());
    engine
}

/// 同 `revision_stage_engine`,但保留事件接收端(Task 9c:StartNew 等待
/// 错误的上浮观测)。
fn revision_stage_engine_with_events(
    fixture: &LogicalRevisionFixture,
) -> (
    WorkspaceEngine,
    mpsc::Receiver<crate::product::workspace_engine::EngineEvent>,
) {
    let mut session = WorkspaceSession::from_record(fixture.record.clone());
    session.repository_path = Some(fixture.member_root.clone());
    let (tx, rx) = mpsc::channel(64);
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(
            fixture.paths.root().join("checkpoints"),
        )),
        fixture.lifecycle.clone(),
        tx,
        session,
    )
    .with_logical_provider_gateway(fixture.gateway.clone());
    engine.session.stage = WorkspaceStage::Revision;
    engine.session.artifact = Some(artifact_payload("# Story Spec\n\n修订前产物"));
    engine.pending_revision_context = Some("补充异常场景".to_string());
    (engine, rx)
}

async fn next_captured_revision_input(
    inputs: &mut mpsc::UnboundedReceiver<StreamingProviderInput>,
) -> StreamingProviderInput {
    tokio::time::timeout(std::time::Duration::from_secs(2), inputs.recv())
        .await
        .expect("captured provider input within timeout")
        .expect("capture channel stays open")
}

/// Task 2.3 主用例：LC revision 的 cwd 重绑 canonical root、target 保持独立
/// 成员 checkout（revision target 不从 cwd 推导）、author 工具策略不放宽、
/// 直连零启动、真实启动唯一经 gateway 并留 audit。
#[tokio::test]
async fn logical_revision_rebinds_root_cwd_and_preserves_revision_target() {
    let (fixture, mut inputs) = logical_revision_fixture(LogicalRevisionAuthority::ManifestRoot);
    let direct = Arc::new(RevisionDirectSentinel {
        starts: Arc::new(AtomicUsize::new(0)),
    });
    let direct_starts = direct.starts.clone();
    let mut engine = revision_stage_engine(&fixture);

    engine
        .drive_revision_session(direct, empty_provider_commands())
        .await;

    let input = next_captured_revision_input(&mut inputs).await;
    assert_eq!(
        direct_starts.load(Ordering::SeqCst),
        0,
        "LC revision 不得绕过 gateway 直连 provider"
    );
    assert_eq!(
        fixture.provider_starts.load(Ordering::SeqCst),
        1,
        "LC revision 必须经 gateway 启动一次"
    );
    assert_eq!(
        fixture.audit.stream_launches(),
        1,
        "gateway 必须记录一次 stream launch"
    );
    // cwd 重绑 canonical root；target worktree 保持成员 checkout——两个字段
    // 显式分离（revision target 不从 cwd 推导）。
    assert_eq!(
        input.working_directory.as_deref(),
        Some(fixture.manifest_root.as_path()),
        "LC revision 的 provider cwd 必须是 manifest provider_context_root"
    );
    assert_eq!(
        input.working_dir, fixture.member_root,
        "target worktree 保持成员 checkout（cwd/target 分离）"
    );
    assert_ne!(
        input.working_dir,
        input
            .working_directory
            .clone()
            .expect("cwd must be rebound"),
        "cwd 与 target 必须是两个独立维度"
    );
    // author 工具策略不放宽：deny_file_write_builtins + durable audit sink。
    assert_eq!(
        input.tool_policy,
        Some(ProviderToolPolicy::deny_file_write_builtins()),
        "LC revision 的 author 工具策略不得放宽"
    );
    assert!(
        input.audit_sink.is_some(),
        "策略 provider run 必须绑定 durable tool-policy audit sink"
    );
    // 修订 run 完成回 AuthorConfirm（artifact gate 通过的完整产物）。
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::AuthorConfirm,
        "LC revision run 完成后必须回 AuthorConfirm"
    );

    // envelope 面：launch 冻结 root cwd 与显式 member/checkout target（与
    // Task 2.1 author 首轮同一 resolver，envelope/resume 面一致）。
    let launch = engine
        .resolve_author_root_launch(&fixture.record)
        .expect("LC 会话必须解析出 root launch")
        .expect("gateway validate 必须通过");
    let envelope = launch.envelope();
    assert_eq!(envelope.working_directory, fixture.manifest_root);
    assert_eq!(envelope.target.worktree, fixture.member_root);
    assert_eq!(
        envelope.target.logical_repository_id,
        fixture.member_id.0.to_string(),
        "target 是显式 member（不从 cwd 推导）"
    );
    assert!(
        !envelope.target.checkout_id.is_empty(),
        "target 是显式 checkout"
    );
}

/// Task 2.3 resume 面 + Task 9c StartNew 语义:cwd-inclusive launch
/// fingerprint 决定续接/supersede——指纹一致(cwd 未漂移)时 native
/// session identity 原样续接;root cwd 漂移时 supersede 旧 native
/// session(gateway 审计),StartNew 后**同一 revision driver 不得继续
/// start_streaming**(零 provider 启动、稳定等待错误上浮、run 收口回
/// PrepareContext、被 supersede 的旧 native 引用清理);用户显式动作
/// (StartGeneration 重驱)后重新 prepare/revalidate 以全新会话启动。
#[tokio::test]
async fn revision_resume_cwd_drift_supersedes_session() {
    let (fixture, mut inputs) =
        logical_revision_fixture(LogicalRevisionAuthority::ParentOfManifestRoot);
    let direct = Arc::new(RevisionDirectSentinel {
        starts: Arc::new(AtomicUsize::new(0)),
    });
    let direct_starts = direct.starts.clone();
    let (mut engine, mut engine_events) = revision_stage_engine_with_events(&fixture);
    // 旧 native session（Author 对话）+ 其发行 launch 的 cwd-inclusive 指纹
    // （Task 2.1 author 路径在 launch 时记忆；此处按同一口径预置）。
    engine.session.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "native-revision-r1".to_string(),
        updated_at: "2026-10-02T00:00:00Z".to_string(),
        last_node_id: None,
    }];
    let first_launch = engine
        .resolve_author_root_launch(&fixture.record)
        .expect("LC 会话必须解析出 root launch")
        .expect("gateway validate 必须通过");
    engine
        .logical_launch_fingerprints
        .insert(ProviderName::ClaudeCode, first_launch.fingerprint().clone());

    // 第一轮：cwd 未漂移（指纹一致）→ native session identity 原样续接。
    engine
        .drive_revision_session(direct.clone(), empty_provider_commands())
        .await;
    let resumed = next_captured_revision_input(&mut inputs).await;
    assert_eq!(
        resumed.resume_provider_session_id.as_deref(),
        Some("native-revision-r1"),
        "指纹一致时必须原样续接 native session（identity 不变）"
    );
    assert_eq!(
        resumed.working_directory.as_deref(),
        Some(fixture.manifest_root.as_path()),
        "续接轮的 cwd 仍是 manifest root"
    );
    assert_eq!(fixture.audit.supersede_count(), 0, "未漂移不得 supersede");

    // root cwd 漂移：manifest 指向另一 root（成员/政策/能力均不变）。
    let drifted = LogicalCodebaseManifest {
        provider_context_root: fixture.drift_root.clone(),
        ..fixture.manifest.clone()
    };
    LogicalCodebaseStore::new(fixture.paths.clone())
        .save_manifest("project_0001", &drifted)
        .expect("save drifted manifest");

    // 第二轮（Task 9c）：漂移 → StartNew(supersede)——同一 revision
    // driver 不得继续 start_streaming:零 provider 启动、零捕获 input。
    engine.session.stage = WorkspaceStage::Revision;
    engine.pending_revision_context = Some("补充异常场景".to_string());
    engine
        .drive_revision_session(direct.clone(), empty_provider_commands())
        .await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), inputs.recv())
            .await
            .is_err(),
        "StartNew 后同一 revision driver 不得启动 provider(零捕获 input)"
    );
    assert_eq!(
        fixture.provider_starts.load(Ordering::SeqCst),
        1,
        "StartNew 等待显式 StartGeneration:revision driver 的 provider 启动计数为 0(第一轮 1 次)"
    );
    // supersede 审计落旧 run(durable),等待错误稳定上浮。
    assert_eq!(
        fixture.audit.supersede_count(),
        1,
        "cwd 漂移 supersede 必须写 gateway 审计"
    );
    assert_eq!(
        fixture.audit.last_supersede_reason().as_deref(),
        Some("resume_fingerprint_mismatch"),
        "supersede 原因必须是 resume fingerprint 漂移"
    );
    // 第一轮的运行事件仍在通道里:有界排空直到 StartNew 的等待错误。
    let waiting_error = loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), engine_events.recv())
            .await
            .expect("StartNew must surface a stable waiting error")
            .expect("engine event channel stays open");
        if let crate::product::workspace_engine::EngineEvent::Error { message } = event {
            break message;
        }
    };
    assert!(
        waiting_error.contains("superseded")
            && waiting_error.contains("start a new generation explicitly"),
        "waiting error must name superseded and the explicit fresh action: {waiting_error}"
    );
    // run 收口回 PrepareContext(waiting 不是隐式续跑,用户显式动作后重新
    // prepare/revalidate)。
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::PrepareContext,
        "StartNew 等待收口回 PrepareContext"
    );
    // 被 supersede 的旧 native 引用清理:后续显式 fresh 不再携带旧 resume id。
    assert!(
        engine
            .session
            .provider_conversations
            .iter()
            .all(|conversation| conversation.provider_session_id != "native-revision-r1"),
        "superseded native session reference must be cleared"
    );

    // 第三轮（Task 9c）：用户显式动作(现有 StartGeneration 面重驱)→ 重新
    // prepare/revalidate 后 fresh 启动:经 gateway 全新会话(resume id 无),
    // cwd=漂移后的新 root。
    engine.session.stage = WorkspaceStage::Revision;
    engine.pending_revision_context = Some("补充异常场景".to_string());
    engine
        .drive_revision_session(direct, empty_provider_commands())
        .await;
    let fresh = next_captured_revision_input(&mut inputs).await;

    assert_eq!(
        direct_starts.load(Ordering::SeqCst),
        0,
        "全程不得绕过 gateway 直连 provider"
    );
    assert_eq!(
        fixture.provider_starts.load(Ordering::SeqCst),
        2,
        "显式 fresh 与第一轮续接各经 gateway 启动一次"
    );
    assert_eq!(
        fresh.resume_provider_session_id, None,
        "显式 fresh 是全新会话,不再携带被 supersede 的旧 resume id"
    );
    assert_eq!(
        fresh.working_directory.as_deref(),
        Some(fixture.drift_root.as_path()),
        "显式 fresh 启动的 cwd 必须是重新 validate 后的新 root"
    );
}
