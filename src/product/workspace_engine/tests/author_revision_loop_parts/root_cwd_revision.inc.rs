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
/// 批2 Task 11：capture validated launch 冻结的执行合同事实
/// （None=合同显式缺失）。(duty, author_digest, profile, carrier)。
type CapturedExecutionContracts = std::sync::Mutex<Vec<Option<(String, bool, String, String)>>>;

struct RevisionCaptureProvider {
    starts: Arc<AtomicUsize>,
    inputs: mpsc::UnboundedSender<StreamingProviderInput>,
    output: &'static str,
    contracts: std::sync::Arc<CapturedExecutionContracts>,
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
        let snapshot = validated.frozen_execution().map(|contract| {
            (
                contract.duty().as_str().to_string(),
                contract.author_policy_digest().is_some(),
                contract.profile().as_str().to_string(),
                format!("{:?}", contract.carrier()),
            )
        });
        self.contracts
            .lock()
            .expect("contract capture lock")
            .push(snapshot);
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
    /// 批2 Task 11：gateway capture provider 观测到的执行合同快照。
    captured_contracts: std::sync::Arc<CapturedExecutionContracts>,
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
    logical_revision_fixture_with(authority, ProviderName::ClaudeCode, None)
}

/// T6 fix-loop r2：参数化变体——author provider 可指定（trust 面用
/// KimiCode），`trust_home` 注入隔离 home 只读 trust source（条目缺失
/// 场景）；其余拓扑与 [`logical_revision_fixture`] 同源。
#[allow(clippy::type_complexity)]
fn logical_revision_fixture_with(
    authority: LogicalRevisionAuthority,
    author_provider: ProviderName,
    trust_home: Option<std::path::PathBuf>,
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
                author_provider: author_provider.clone(),
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
    let contracts: std::sync::Arc<CapturedExecutionContracts> =
        Default::default();
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
                contracts: contracts.clone(),
            }),
        )
    };
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, capture);
    let audit = Arc::new(GatewayRunAudit::new());
    let gateway = LogicalCodebaseProviderGateway::with_audit(
        AggregatePolicyArtifactStore::new(paths.clone()),
        Arc::new(super::ReviewStaticCapabilitySource::default()),
        Arc::new(super::ReviewPassThroughTargetResolver),
        Arc::new(registry),
        Arc::new(super::ReviewStubSyncAdapter),
        super::review_always_available_gate(),
        audit.clone(),
        authority_root,
    );
    // T6 fix-loop r2：隔离 home 只读 trust source（真实 Codex/Kimi adapter
    // for_home，绝不触碰真实用户 home）。
    let gateway = match trust_home {
        Some(trust_home) => gateway.with_readonly_lc_facts(
            crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
                paths.clone(),
                &lc_id,
            ),
            Arc::new(
                crate::product::logical_codebase::provider_trust::ReadonlyProviderTrustSource::new(
                    paths.clone(),
                    Some(lc_id.clone()),
                    vec![
                        Arc::new(
                            crate::product::logical_codebase::provider_trust_adapters::CodexTrustAdapter::for_home(&trust_home),
                        ),
                        Arc::new(
                            crate::product::logical_codebase::provider_trust_adapters::KimiTrustAdapter::for_home(&trust_home),
                        ),
                    ],
                ),
            ),
            Some(lc_id.clone()),
        ),
        None => gateway,
    };
    let gateway = Arc::new(gateway);

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
            captured_contracts: contracts,
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
    // cwd 重绑 canonical root；input.working_dir 保持业务材料锚（成员
    // checkout，business 材料面不变）——E 修复后 envelope target 恒聚合根
    // 锚，两字段语义独立（cwd 恒 root）。
    assert_eq!(
        input.working_directory.as_deref(),
        Some(fixture.manifest_root.as_path()),
        "LC revision 的 provider cwd 必须是 manifest provider_context_root"
    );
    assert_eq!(
        input.working_dir, fixture.member_root,
        "input.working_dir 保持业务材料锚（成员 checkout 材料面不变）"
    );
    assert_ne!(
        input.working_dir,
        input
            .working_directory
            .clone()
            .expect("cwd must be rebound"),
        "cwd 与材料锚必须是两个独立维度"
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
    // A1-b1(Bug45 批2 环境回退):author root launch 停止注入环境——
    // env_vars 平台不构造(caller 未携带即空),宿主 PATH/HOME 由底座
    // (process_manager)继承到达子进程。
    assert!(
        input.env_vars.is_empty(),
        "author root launch 不得再构造白名单 env: {:?}",
        input.env_vars
    );
    // 批2 Task 11（Author 独立入口·revision 面）：LC revision launch 同样
    // 冻结 duty=Author 的执行合同（HostCoding + AuthorToolPolicy，经
    // Orchestrator 承载 audit 面仍 Author）——fresh 与 resume 同一口径，
    // 否则每次 resume 都因合同指纹漂移 supersede。
    let contract_snapshots = fixture
        .captured_contracts
        .lock()
        .expect("contract capture lock")
        .clone();
    assert_eq!(
        contract_snapshots.len(),
        1,
        "LC revision run must freeze the Author execution contract"
    );
    let Some((duty, author_digest, profile, carrier)) = &contract_snapshots[0] else {
        panic!("LC revision launch must freeze the Author execution contract");
    };
    assert_eq!(duty, "author");
    assert!(author_digest, "Author 合同必须冻结岗位策略摘要");
    assert_eq!(profile, "host_coding");
    assert_eq!(carrier, "Orchestrator", "Author 经 Orchestrator 承载");
    // 修订 run 完成回 AuthorConfirm（artifact gate 通过的完整产物）。
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::AuthorConfirm,
        "LC revision run 完成后必须回 AuthorConfirm"
    );

    // envelope 面（E 修复后）：launch 冻结 root cwd 与聚合根锚 target
    // （native_launch_anchor；与 Task 2.1 author 首轮、gateway_start 同口径）
    // ——成员/checkout 身份不入 target（scope 回写不迁移锚、指纹恒定）。
    let launch = engine
        .resolve_author_root_launch(&fixture.record)
        .expect("LC 会话必须解析出 root launch")
        .expect("gateway validate 必须通过");
    let envelope = launch.envelope();
    assert_eq!(envelope.working_directory, fixture.manifest_root);
    assert_eq!(
        envelope.target.worktree,
        fixture.manifest_root.canonicalize().expect("canonical root"),
        "target 是 E native anchor（聚合根），非成员 checkout"
    );
    assert!(
        envelope.target.logical_repository_id.is_empty()
            && envelope.target.checkout_id.is_empty(),
        "聚合根锚不携带成员/checkout 身份"
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
    // 批2 Task 11（硬约束2·resume 同口径）：续接轮与 fresh 轮绑定同一
    // duty=Author 执行合同——resume 判定以未装饰投影比对（T10），合同
    // 绑定不得引起 supersede；本轮合同事实与首轮完全一致。
    let contract_snapshots = fixture
        .captured_contracts
        .lock()
        .expect("contract capture lock")
        .clone();
    assert!(
        !contract_snapshots.is_empty(),
        "resume round must also freeze the Author execution contract"
    );
    for snapshot in &contract_snapshots {
        let Some((duty, author_digest, profile, carrier)) = snapshot else {
            panic!("resume round must freeze the Author execution contract");
        };
        assert_eq!(duty, "author");
        assert!(author_digest);
        assert_eq!(profile, "host_coding");
        assert_eq!(carrier, "Orchestrator");
    }

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

    // 第三轮（A2b / Task 2.6 契约更新）：waiting 已持久后，同一 revision
    // driver 的普通重驱**必须被拒**——显式 fresh 面归 B1b 的
    // `resume_recovery` 入参（A/B 未齐前无可启动半成品，旁路一律后端
    // 拒收），零 provider 启动、run 仍收口在等待形态。
    engine.session.stage = WorkspaceStage::Revision;
    engine.pending_revision_context = Some("补充异常场景".to_string());
    engine
        .drive_revision_session(direct, empty_provider_commands())
        .await;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), inputs.recv())
            .await
            .is_err(),
        "等待期间重驱不得捕获任何 provider input"
    );
    assert_eq!(
        direct_starts.load(Ordering::SeqCst),
        0,
        "全程不得绕过 gateway 直连 provider"
    );
    assert_eq!(
        fixture.provider_starts.load(Ordering::SeqCst),
        1,
        "等待期间重驱零新增 provider 启动(仅第一轮续接 1 次)"
    );
    let blocked_error = loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), engine_events.recv())
            .await
            .expect("blocked drive must surface a waiting error")
            .expect("engine event channel stays open");
        if let crate::product::workspace_engine::EngineEvent::Error { message } = event {
            break message;
        }
    };
    assert!(
        blocked_error.contains("blocked by lc resume recovery")
            && blocked_error.contains("start a new generation explicitly"),
        "等待守卫错误必须点名恢复等待与显式动作: {blocked_error}"
    );
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::PrepareContext,
        "重驱被拒后仍收口在 PrepareContext"
    );
    // durable waiting record 已在第二轮落盘(A2b 安全 record 先)。
    let reloaded = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload durable record");
    let recovery = reloaded
        .lc_resume_recovery
        .expect("既有 supersede 用例也必须持久 waiting record");
    assert_eq!(
        recovery.state,
        crate::product::models::LcResumeRecoveryState::Waiting
    );
    assert_eq!(recovery.reason_code, "resume_fingerprint_mismatch");
}

/// A2b（Task 2.6 / tasks.md 2.6，设计 §4.3/§4.4）：LC mismatch 的**真实等待
/// 收口**——旧 provider 节点幂等终态化（Active→Failed）、active node/run 清
/// 空、durable waiting record 先落盘（阻止旧 resume 与旁路 spawn）、纯投影
/// 只派生零写、显式 fresh 授权前（A/B 未齐）同 driver 重驱零启动且
/// allowed_actions 恒空。
#[tokio::test]
async fn anchor_t07_superseded_waiting_finishes_provider_node() {
    let (fixture, mut inputs) =
        logical_revision_fixture(LogicalRevisionAuthority::ParentOfManifestRoot);
    let direct = Arc::new(RevisionDirectSentinel {
        starts: Arc::new(AtomicUsize::new(0)),
    });
    let (mut engine, mut engine_events) = revision_stage_engine_with_events(&fixture);
    // 旧 native session + 其发行 launch 的 cwd-inclusive 指纹（同既有
    // supersede 用例口径）。
    engine.session.provider_conversations = vec![ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "native-revision-r7".to_string(),
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

    // 第一轮：指纹一致续接（恰一次 provider 启动）。
    engine
        .drive_revision_session(direct.clone(), empty_provider_commands())
        .await;
    let _resumed = next_captured_revision_input(&mut inputs).await;
    let starts_before_drift = fixture.provider_starts.load(Ordering::SeqCst);
    assert_eq!(starts_before_drift, 1);

    // 真实修订臂形态：mismatch 前 decisions 已创建 Active Revision 节点。
    engine.session.stage = WorkspaceStage::Revision;
    engine.pending_revision_context = Some("补充异常场景".to_string());
    let revision_node_id = engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::Revision,
            agent: Some(ProviderName::ClaudeCode),
            stage: WorkspaceStage::Revision,
            round: Some(2),
            title: "反馈修订".to_string(),
            summary: Some("补充异常场景".to_string()),
            status: TimelineNodeStatus::Active,
        })
        .await;

    // root cwd 漂移 → mismatch：真实等待收口。
    let drifted = LogicalCodebaseManifest {
        provider_context_root: fixture.drift_root.clone(),
        ..fixture.manifest.clone()
    };
    LogicalCodebaseStore::new(fixture.paths.clone())
        .save_manifest("project_0001", &drifted)
        .expect("save drifted manifest");
    engine
        .drive_revision_session(direct.clone(), empty_provider_commands())
        .await;

    // mismatch 零新 provider 启动、零捕获 input。
    assert_eq!(
        fixture.provider_starts.load(Ordering::SeqCst),
        starts_before_drift,
        "mismatch 收口不得有新 provider 启动"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), inputs.recv())
            .await
            .is_err(),
        "mismatch 后不得捕获任何 provider input"
    );
    // 旧 provider 节点终态化 Failed、active node/run 清空、legacy 对外回
    // PrepareContext。
    let old_node = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_id == revision_node_id)
        .expect("revision node present")
        .clone();
    assert_eq!(
        old_node.status,
        TimelineNodeStatus::Failed,
        "旧 provider 节点必须幂等终态化 Failed"
    );
    assert!(
        engine.active_timeline_node_id().is_none(),
        "active node 必须清空"
    );
    assert!(engine.active_run_id().is_none(), "active run 必须清空");
    assert_eq!(
        engine.current_stage(),
        WorkspaceStage::PrepareContext,
        "legacy 等待对外回 PrepareContext"
    );
    // durable waiting record（安全 record 先落盘）与被取代旧引用清理。
    let reloaded = fixture
        .lifecycle
        .get_workspace_session(&fixture.record.id)
        .expect("reload durable record");
    let recovery = reloaded
        .lc_resume_recovery
        .expect("superseded waiting record must be persisted");
    assert_eq!(
        recovery.state,
        crate::product::models::LcResumeRecoveryState::Waiting
    );
    assert_eq!(recovery.reason_code, "resume_fingerprint_mismatch");
    assert_eq!(recovery.failed_provider_node_id, revision_node_id);
    assert_eq!(
        recovery.superseded_native_session_id.as_deref(),
        Some("native-revision-r7")
    );
    assert_eq!(recovery.role, ProviderConversationRole::Author);
    assert_eq!(recovery.provider, ProviderName::ClaudeCode);
    assert!(
        engine
            .session
            .provider_conversations
            .iter()
            .all(|conversation| conversation.provider_session_id != "native-revision-r7")
    );
    // 稳定等待错误上浮（同既有文案契约）。
    let waiting_error = loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), engine_events.recv())
            .await
            .expect("mismatch must surface a stable waiting error")
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

    // 显式 fresh 前（A/B 未齐）同 driver 重驱：等待守卫拒收，零新增启动。
    engine.session.stage = WorkspaceStage::Revision;
    engine.pending_revision_context = Some("再次补充".to_string());
    engine
        .drive_revision_session(direct, empty_provider_commands())
        .await;
    assert_eq!(
        fixture.provider_starts.load(Ordering::SeqCst),
        starts_before_drift,
        "等待期间重驱必须零 provider 启动"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), inputs.recv())
            .await
            .is_err(),
        "等待期间重驱不得捕获 provider input"
    );
    assert_eq!(
        engine.current_stage(),
        WorkspaceStage::PrepareContext,
        "重驱被拒后仍收口在 PrepareContext"
    );

    // 纯投影零写：模拟「CAS 已落盘、节点未终态化」的中间态（manager 在
    // save 与 timeline 收口间崩溃）——节点回 Active 后 ProjectionOnly 只在
    // 内存派生终态/无 active/PrepareContext，磁盘不动。
    engine
        .update_timeline_node(&revision_node_id, TimelineNodeStatus::Active, None)
        .await;
    let mut session_for_projection = WorkspaceSession::from_record(
        fixture
            .lifecycle
            .get_workspace_session(&fixture.record.id)
            .expect("reload record for projection"),
    );
    session_for_projection.repository_path = Some(fixture.member_root.clone());
    let (projection_tx, _projection_rx) = mpsc::channel(8);
    let projection = WorkspaceEngine::new_persistent_for_projection(
        Arc::new(CheckpointStore::new(
            fixture.paths.root().join("checkpoints"),
        )),
        fixture.lifecycle.clone(),
        projection_tx,
        session_for_projection,
    )
    .expect("projection engine");
    assert_eq!(
        projection.current_stage(),
        WorkspaceStage::PrepareContext,
        "纯投影派生等待对外形态（PrepareContext）"
    );
    assert!(
        projection.active_timeline_node_id().is_none(),
        "纯投影不得派生活跃 provider 节点"
    );
    let projected_node = projection
        .timeline_nodes
        .iter()
        .find(|node| node.node_id == revision_node_id)
        .expect("projected revision node")
        .clone();
    assert_eq!(
        projected_node.status,
        TimelineNodeStatus::Failed,
        "纯投影在内存派生节点终态（不改磁盘）"
    );
    let disk_nodes = fixture
        .lifecycle
        .load_timeline_nodes_for_issue_session(
            &fixture.record.project_id,
            &fixture.record.issue_id,
            &fixture.record.id,
        )
        .expect("disk timeline nodes");
    let disk_node = disk_nodes
        .iter()
        .find(|node| node.node_id == revision_node_id)
        .expect("disk revision node")
        .clone();
    assert_eq!(
        disk_node.status,
        TimelineNodeStatus::Active,
        "纯投影零磁盘写（fallback 不得成为另一写方）"
    );
    // waiting DTO：B 波接齐前 allowed_actions 恒空（A/B 发布纪律）。
    let crate::web::workspace_ws_types::WsOutMessage::SessionState {
        lc_resume_recovery: Some(dto),
        ..
    } = projection.build_session_state()
    else {
        panic!("projection session state must carry the waiting recovery");
    };
    assert_eq!(
        dto.state,
        crate::product::models::LcResumeRecoveryState::Waiting
    );
    assert!(
        dto.allowed_actions.is_empty(),
        "B 齐前 waiting allowed_actions 必须为空"
    );
}

/// T6 fix-loop r2：LC Author Revision（`drive_revision_session` 的 gateway
/// 启动口，review/drive.rs U2 位点）Kimi 条目缺失 → spawn trust 拒绝以
/// `provider_trust_missing` 分类＋同源指导＋生成场景 retry_action 经
/// `EngineEvent::Error` 上浮；provider 零启动（直连与 gateway registry
/// 双零启动）。
#[tokio::test]
async fn logical_revision_spawn_trust_missing_classifies_with_manual_guidance() {
    let trust_home = tempfile::tempdir().expect("isolated trust home");
    let (fixture, mut inputs) = logical_revision_fixture_with(
        LogicalRevisionAuthority::ManifestRoot,
        ProviderName::KimiCode,
        Some(trust_home.path().to_path_buf()),
    );
    let (mut engine, mut engine_rx) = revision_stage_engine_with_events(&fixture);
    // 直连 fallback provider：LC 下不得被触达（trust 拒绝先于一切启动）。
    let (direct_starts, _direct_inputs, direct) = {
        let starts = Arc::new(AtomicUsize::new(0));
        let (input_tx, _input_rx) = mpsc::unbounded_channel();
        (
            starts.clone(),
            _input_rx,
            Arc::new(RevisionCaptureProvider {
                starts,
                inputs: input_tx,
                output: revision_story_output(),
                contracts: Default::default(),
            }),
        )
    };

    engine
        .drive_revision_session(direct, empty_provider_commands())
        .await;

    let message = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match engine_rx.recv().await {
                Some(crate::product::workspace_engine::EngineEvent::Error { message }) => {
                    return message;
                }
                Some(_) => continue,
                None => panic!("engine event channel closed before spawn error surfaced"),
            }
        }
    })
    .await
    .expect("revision spawn trust failure must surface via EngineEvent::Error");

    assert!(
        message.contains("provider_trust_missing"),
        "revision gateway 启动臂 trust 拒绝必须提升 provider_trust_missing 分类，got: {message}"
    );
    assert!(
        message.contains("provider trust not established for KimiCode"),
        "原拒绝原文必须保留，got: {message}"
    );
    assert!(
        message.contains(
            &trust_home
                .path()
                .join(".kimi-code")
                .join("workspace-trust")
                .display()
                .to_string()
        ),
        "revision 臂指导必须同源（真实工件路径），got: {message}"
    );
    assert!(
        message.contains("人工登记") && message.contains("显式重新生成"),
        "revision 臂必须携带生成场景 retry_action，got: {message}"
    );
    // provider 零启动：gateway registry（fixture.provider_starts）与直连
    // fallback（direct_starts）都不得启动。
    assert_eq!(
        fixture.provider_starts.load(Ordering::SeqCst),
        0,
        "trust 拒绝必须发生在 gateway adapter 启动之前"
    );
    assert_eq!(
        direct_starts.load(Ordering::SeqCst),
        0,
        "LC revision 不得回落直连 provider"
    );
    assert!(
        !matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), inputs.recv()).await,
            Ok(Some(_))
        ),
        "trust 拒绝必须 reject before provider startup"
    );
}

// ============================================================================
// E 锚点消费缺口（author launch 侧，2026-10-12 design 修订指纹漂移现场）：
// `build_root_launch_request` 的 target 仍从 `business_repository` 的成员
// checkout 派生（Some(checkout) 臂），scope 回写（involved/focus/
// change_order）即迁移 launch target → resume 指纹四漂移域（target
// logical id/checkout/worktree/git identity）随之漂移 → 修订轮 supersede。
// E 包（REQ-PLN-08）已交付 `native_launch_anchor`（恒聚合根）；本族测试
// 锁 author launch 消费面：两轮（生成无业务态 → 修订 involved=4+
// change_order）launch 指纹必须相等。
// ============================================================================

/// 四成员 LC + Design 会话夹具（登记链与 `logical_revision_fixture` 同构：
/// record/manifest/member/checkout/selection/policy + repos strict 投影 +
/// design aggregate scope），成员 checkout 均为聚合根直接子目录。
struct LogicalDesignRevisionFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    lifecycle: LifecycleStore,
    record: WorkspaceSessionRecord,
    gateway: Arc<LogicalCodebaseProviderGateway>,
    manifest: LogicalCodebaseManifest,
    /// 四成员逻辑身份（involved/change_order 现场形态：involved=4）。
    member_ids: Vec<LogicalRepositoryId>,
    /// 首成员 checkout（业务材料锚，与 manifest root 分离）。
    first_member_root: std::path::PathBuf,
    /// manifest 冻结的 provider_context_root（cwd 来源）。
    manifest_root: std::path::PathBuf,
}

fn logical_design_revision_fixture() -> LogicalDesignRevisionFixture {
    let root = tempfile::tempdir().expect("temporary workspace root");
    let manifest_root = root.path().join("lc-root");
    let member_roots: Vec<std::path::PathBuf> = (0..4)
        .map(|index| manifest_root.join(format!("design-member-{index}")))
        .collect();
    for member_root in &member_roots {
        std::fs::create_dir_all(member_root).expect("create design member checkout");
        init_revision_git_repo(member_root);
    }

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    crate::product::project_store::ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "logical design revision fixture".to_string(),
            description: None,
        })
        .expect("create project");
    let repositories: Vec<_> = member_roots
        .iter()
        .enumerate()
        .map(|(index, member_root)| {
            crate::product::repository_store::RepositoryStore::new(paths.clone())
                .create(CreateRepositoryInput {
                    project_id: "project_0001".to_string(),
                    name: format!("logical design revision member {index}"),
                    path: member_root.clone(),
                    default_policy_preset: None,
                    default_provider_mode: None,
                    idempotency_key: format!("logical-design-revision-{index}"),
                })
                .expect("create member repository")
        })
        .collect();
    crate::product::issue_store::IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some(repositories[0].id.clone()),
            logical_codebase_id: Some(
                crate::product::logical_codebase::store::legacy_logical_codebase_id(
                    "project_0001",
                ),
            ),
            title: "logical design revision fixture issue".to_string(),
            description: Some("design revision anchor".to_string()),
            change_id: None,
            base_branch: None,
        })
        .expect("create issue");

    // LC 权威记录（alias 位置）+ manifest/四成员/四 checkout + 显式全成员
    // selection（登记链与 logical_revision_fixture 同构，×4）。
    let lc_id = crate::product::logical_codebase::store::legacy_logical_codebase_id("project_0001");
    let record_root = paths.logical_codebase_record_root("project_0001", &lc_id);
    std::fs::create_dir_all(&record_root).expect("create lc record root");
    std::fs::write(
        record_root.join("record.json"),
        serde_json::to_vec_pretty(&LogicalCodebaseRecord {
            id: lc_id.clone(),
            name: "logical design revision alias lc".to_string(),
            aggregate_root: manifest_root.clone(),
            created_at: "2026-10-12T00:00:00Z".to_string(),
        })
        .expect("serialize lc record"),
    )
    .expect("write lc record.json");

    let member_ids: Vec<LogicalRepositoryId> = (0..4)
        .map(|_| LogicalRepositoryId(uuid::Uuid::new_v4()))
        .collect();
    let checkout_ids: Vec<RepositoryCheckoutId> = (0..4)
        .map(|_| RepositoryCheckoutId(uuid::Uuid::new_v4()))
        .collect();
    let authority = LogicalCodebaseStore::new(paths.clone());
    let manifest =
        LogicalCodebaseManifest::new("project_0001", manifest_root.clone(), member_ids.clone());
    authority
        .save_manifest("project_0001", &manifest)
        .expect("save manifest");
    let now = "2026-10-12T00:00:00Z".to_string();
    for index in 0..4 {
        authority
            .save_member(
                "project_0001",
                &CodebaseMemberRecord {
                    logical_repository_id: member_ids[index],
                    physical_repository_id: repositories[index].id.clone(),
                    alias: format!("design-member-{index}"),
                    role: "member".to_string(),
                    ordinal: (index + 1) as u32,
                    source_identity: RepositorySourceIdentity::from_git_parts(
                        &member_roots[index],
                        member_roots[index].join(".git"),
                        Some(format!("ssh://git@example.test/design/member-{index}.git")),
                    ),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_ids[index]],
                    status: MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .expect("save design member");
        authority
            .save_checkout(
                "project_0001",
                &RepositoryCheckoutRecord {
                    checkout_id: checkout_ids[index],
                    logical_repository_id: member_ids[index],
                    physical_repository_id: repositories[index].id.clone(),
                    kind: CheckoutKind::Main,
                    canonical_path: member_roots[index].clone(),
                    checkout_path_hash: format!("sha256:logical-design-revision-checkout-{index}"),
                    git_dir_identity: format!("sha256:logical-design-revision-git-dir-{index}"),
                    revision: Some("abc123".to_string()),
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .expect("save design checkout");
    }
    IssueCodebaseSelectionStore::new(paths.clone())
        .save(&IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            member_ids.clone(),
            Vec::new(),
            Vec::new(),
            None,
        ))
        .expect("save issue selection");
    AggregatePolicyArtifactStore::new(paths.clone())
        .ensure_bootstrap(&manifest)
        .expect("bootstrap aggregate policy");

    // strict 三层身份投影：repos.json 升级为已登记形态（×4）。
    let projections: Vec<_> = repositories
        .iter()
        .enumerate()
        .map(|(index, repository)| {
            let mut projection = repository.clone();
            projection.logical_repository_id = Some(member_ids[index]);
            projection.primary_checkout_id = Some(checkout_ids[index]);
            projection.identity_schema_version = 1;
            projection
        })
        .collect();
    std::fs::write(
        paths.project_root("project_0001").join("repos.json"),
        serde_json::to_vec_pretty(&projections).expect("serialize repos projection"),
    )
    .expect("rewrite repos.json");

    // Design aggregate scope（生成形态：无业务态 involved/change_order 均空）
    // + 持久会话记录。
    let lifecycle = LifecycleStore::new(paths.clone());
    let design = lifecycle
        .create_design_spec(crate::product::lifecycle_store::CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: Vec::new(),
            title: "logical design revision design".to_string(),
            aggregate_codebase: Some(crate::product::lifecycle_store::AggregateDesignSpecScope {
                logical_codebase_ref: manifest.logical_codebase_id.clone(),
                effective_member_ids: member_ids.clone(),
                involved_repository_ids: Vec::new(),
                change_order: Vec::new(),
            }),
        })
        .expect("create design spec");
    let record = lifecycle
        .create_workspace_session_with_id(
            CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: design.id,
                workspace_type: WorkspaceType::Design,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::ClaudeCode),
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
            format!(
                "workspace_session_logical_design_revision_{}",
                uuid::Uuid::new_v4().simple()
            ),
        )
        .expect("create workspace session");

    // gateway：capture provider 注册为 ClaudeCode 真实 adapter（validate 面
    // 与生产同构；resolver/doubles 复用 part_32 测试件）。
    let starts = Arc::new(AtomicUsize::new(0));
    let (input_tx, _input_rx) = mpsc::unbounded_channel();
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(RevisionCaptureProvider {
            starts,
            inputs: input_tx,
            output: revision_story_output(),
            contracts: Default::default(),
        }),
    );
    let audit = Arc::new(GatewayRunAudit::new());
    let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
        AggregatePolicyArtifactStore::new(paths.clone()),
        Arc::new(super::ReviewStaticCapabilitySource::default()),
        Arc::new(super::ReviewPassThroughTargetResolver),
        Arc::new(registry),
        Arc::new(super::ReviewStubSyncAdapter),
        super::review_always_available_gate(),
        audit,
        std::fs::canonicalize(&manifest_root).expect("canonical manifest root"),
    ));

    LogicalDesignRevisionFixture {
        _root: root,
        paths,
        lifecycle,
        record,
        gateway,
        manifest,
        member_ids,
        first_member_root: member_roots[0].clone(),
        manifest_root,
    }
}

/// Design 修订态的持久 LC engine（gateway 注入；repository_path=首成员
/// checkout 业务材料锚，与 cwd 来源分离）。
fn design_stage_engine(fixture: &LogicalDesignRevisionFixture) -> WorkspaceEngine {
    let mut session = WorkspaceSession::from_record(fixture.record.clone());
    session.repository_path = Some(fixture.first_member_root.clone());
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
    engine.session.artifact = Some(artifact_payload("# Design Spec\n\n修订前产物"));
    engine.pending_revision_context = Some("补充异常场景".to_string());
    engine
}

/// E 锚点消费缺口红→绿主锁（author launch 侧）：Design 带 involved=4+
/// change_order 的两轮 launch——生成轮（无业务态，business=聚合视图）与
/// 修订轮（scope 回写后 business=首 change_order 成员）的 launch 指纹必须
/// 相等：native launch target 恒取 E `native_launch_anchor`（聚合根），
/// scope 回写不迁移锚（今晨 design 修订指纹漂移现场：聚合根=388779…
/// 生成轮 vs busi 成员=dda858… 修订轮，四漂移域全动）。
#[test]
fn anchor_t12_design_revision_launch_fingerprint_stable_across_scope_writeback() {
    let fixture = logical_design_revision_fixture();
    let engine = design_stage_engine(&fixture);

    // 生成轮：design 无业务态（involved 空）→ business 派生聚合根视图。
    let generation_request = engine
        .build_root_launch_request(
            &fixture.record,
            &ProviderName::ClaudeCode,
            crate::product::logical_codebase::SessionPolicyAction::PlanningReadOnly,
        )
        .expect("LC 会话必须解析出 launch request")
        .expect("generation round must resolve");
    let generation_target = generation_request.target.clone();
    let generation_fingerprint = fixture
        .gateway
        .validate(generation_request)
        .expect("generation round gateway validate")
        .fingerprint()
        .clone();

    // 修订轮：scope 回写 involved=4 + change_order=4（REQ-PLN-05 合法确认
    // 态）→ business 恢复首成员解析；成员 checkout 不得进入 native launch。
    fixture
        .lifecycle
        .update_design_spec_aggregate(
            "project_0001",
            "issue_0001",
            &fixture.record.entity_id,
            &crate::product::lifecycle_store::AggregateDesignSpecScope {
                logical_codebase_ref: fixture.manifest.logical_codebase_id.clone(),
                effective_member_ids: fixture.member_ids.clone(),
                involved_repository_ids: fixture.member_ids.clone(),
                change_order: fixture.member_ids.clone(),
            },
        )
        .expect("scope writeback to multi-repo revision form");
    let revision_request = engine
        .build_root_launch_request(
            &fixture.record,
            &ProviderName::ClaudeCode,
            crate::product::logical_codebase::SessionPolicyAction::PlanningReadOnly,
        )
        .expect("LC 会话必须解析出 launch request")
        .expect("revision round must resolve");
    let revision_target = revision_request.target.clone();
    let revision_fingerprint = fixture
        .gateway
        .validate(revision_request)
        .expect("revision round gateway validate")
        .fingerprint()
        .clone();

    assert_eq!(
        generation_fingerprint, revision_fingerprint,
        "Design 两轮 launch 指纹必须相等（native target 恒聚合根锚，scope 回写不迁移）"
    );
    assert!(
        generation_target.logical_repository_id.is_empty()
            && generation_target.checkout_id.is_empty(),
        "生成轮 target 是 aggregate_root 锚"
    );
    assert!(
        revision_target.logical_repository_id.is_empty()
            && revision_target.checkout_id.is_empty(),
        "修订轮 native launch target 必须是聚合根锚（成员 checkout 不入 target）"
    );
    assert_eq!(
        revision_target.worktree,
        fixture
            .manifest_root
            .canonicalize()
            .expect("canonical root"),
        "修订轮 native launch target 锚定 canonical 聚合根"
    );
}

/// E 锚点消费缺口（max-task 定案补验①）：Story focus 回写用例——story
/// 生成轮（AI 自决前草稿态，focus=None → business 聚合视图）→ 完成后回写
/// involved/focus（business 恢复成员解析）→ 修订轮：两轮 launch 指纹必须
/// 相等、native target 恒聚合根锚（今晨 story「侥幸通过」变必然——修复前
/// 成员 target 恰好两轮同成员才未漂，design 无业务态生成轮即漂）。
#[test]
fn anchor_t13_story_focus_writeback_launch_fingerprint_stays_aggregate() {
    let (fixture, _inputs) = logical_revision_fixture(LogicalRevisionAuthority::ManifestRoot);
    // 生成轮形态：story focus=None（anchor_t11 同款 JSON 改写）。
    let story_spec_path = fixture
        .paths
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("story-specs")
        .join(format!("{}.json", fixture.record.entity_id));
    let mut story_spec: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&story_spec_path).expect("read story spec"))
            .expect("parse story spec");
    story_spec["focus_repository_id"] = serde_json::Value::Null;
    std::fs::write(
        &story_spec_path,
        serde_json::to_vec_pretty(&story_spec).expect("serialize story spec"),
    )
    .expect("rewrite story spec without focus");

    let engine = revision_stage_engine(&fixture);
    let generation_request = engine
        .build_root_launch_request(
            &fixture.record,
            &ProviderName::ClaudeCode,
            crate::product::logical_codebase::SessionPolicyAction::PlanningReadOnly,
        )
        .expect("LC 会话必须解析出 launch request")
        .expect("generation round must resolve");
    let generation_fingerprint = fixture
        .gateway
        .validate(generation_request)
        .expect("generation round gateway validate")
        .fingerprint()
        .clone();

    // 完成后回写 involved/focus（r23 生产口径：AI 显式回写 focus 走成员
    // 锚 business 臂）→ 修订轮。
    fixture
        .lifecycle
        .update_story_spec_aggregate(
            "project_0001",
            "issue_0001",
            &fixture.record.entity_id,
            &crate::product::lifecycle_store::AggregateStorySpecScope {
                logical_codebase_ref: fixture.manifest.logical_codebase_id.clone(),
                effective_member_ids: vec![fixture.member_id],
                involved_repository_ids: vec![fixture.member_id],
                focus_repository_id: Some(fixture.member_id),
            },
        )
        .expect("focus writeback to revision form");
    let revision_request = engine
        .build_root_launch_request(
            &fixture.record,
            &ProviderName::ClaudeCode,
            crate::product::logical_codebase::SessionPolicyAction::PlanningReadOnly,
        )
        .expect("LC 会话必须解析出 launch request")
        .expect("revision round must resolve");
    let revision_target = revision_request.target.clone();
    let revision_fingerprint = fixture
        .gateway
        .validate(revision_request)
        .expect("revision round gateway validate")
        .fingerprint()
        .clone();

    assert_eq!(
        generation_fingerprint, revision_fingerprint,
        "Story focus 回写前后两轮 launch 指纹必须相等（native target 恒聚合根锚）"
    );
    assert!(
        revision_target.logical_repository_id.is_empty()
            && revision_target.checkout_id.is_empty(),
        "回写后修订轮 native launch target 仍是聚合根锚（成员不入 target）"
    );
    assert_eq!(
        revision_target.worktree,
        fixture
            .manifest_root
            .canonicalize()
            .expect("canonical root"),
        "回写后 native launch target 仍锚定 canonical 聚合根"
    );
}
