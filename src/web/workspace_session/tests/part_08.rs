// D2.1a（fix-cascade-deletion Task 3.1；REQ-DEL-06）：manager 与 REST 真实
// 运行 lease。真实入口（spawn_provider_run_with_start_mode 单一收口：author/
// review/revision/event-recovery）与 REST `ProviderWorkspaceRunner::run_next`
// 启动前 acquire_run，lease 随任务持有到最终 store writes/finish：
// - 运行持 lease 时删除准入 entity_busy 且零内容变动；
// - 删除赢后 provider 实际 start 计数 0、旧 Arc dispatch/attach 拒；
// - disconnect 迟到 lease/degraded 诊断不重建已删除 timeline；
// - 非目标 manager 不受牵连；正常 supersede/finish 不丢 lease 也不泄漏。
// include! 同模块纪律：WsOutMessage/CheckpointStore/CreateWorkspaceSessionInput/
// WorkspaceSessionRecord/WorkspaceType/WorkspaceEngine/WorkspaceSession 沿用
// part_07 既有导入，本文件不重复 use（E0252 去重）。
use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderSession, ProviderStatus, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::product::entity_deletion_store::{DeletionEntityKind, EntityDeletionStore, EntityScope};
use crate::product::entity_operation_store::{EntityOperationError, EntityOperationStore};
use crate::product::provider_workspace_runner::{
    ProviderWorkspaceRunner, WorkspaceProviderRunInput,
};
use crate::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};
use crate::web::state::WorkspaceRunRegistry;
use crate::web::workspace_ws_handler::{
    ProviderRunContext, spawn_provider_run_from_event, spawn_provider_run_from_handler,
};
use parking_lot::Mutex as SyncMutex;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use tokio::sync::Mutex;

/// 挂起式流 provider：start 即计数并保持会话存活（40ms keepalive 防零活动
/// 看门狗，ScAuthorStubProvider r29 先例）；永不完成。
struct D21aPendingProvider {
    starts: Arc<AtomicUsize>,
    held: Arc<SyncMutex<Vec<mpsc::Sender<ProviderEvent>>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for D21aPendingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let (event_tx, event_rx) = mpsc::channel(1);
        let keepalive_tx = event_tx.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(40)).await;
                if keepalive_tx
                    .send(ProviderEvent::StatusChanged(ProviderStatus::Running))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        self.held.lock().push(event_tx);
        let (command_tx, _command_rx) = mpsc::channel(1);
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn start_validated(
        &self,
        validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (input, _launch) = validated.into_parts();
        self.start(input, cancel).await
    }

    async fn run_streaming(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<
        mpsc::Receiver<crate::cross_cutting::streaming_provider::StreamChunk>,
        ProviderAdapterError,
    > {
        unreachable!("workspace runs use start")
    }
}

/// REST 阻塞式 provider：run() 即计数并等待放行信号（证明 lease 跨 provider
/// 运行与最终 store writes 全程持有）。
struct D21aBlockingRestProvider {
    starts: Arc<AtomicUsize>,
    release: SyncMutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl ProviderAdapter for D21aBlockingRestProvider {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        if let Some(receiver) = self.release.lock().take() {
            let _ = receiver.recv();
        }
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: "d21a rest lease markdown".to_string(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

/// REST 计数式 provider（删除赢后零 provider start 断言用）。
struct D21aCountingRestProvider {
    starts: Arc<AtomicUsize>,
}

impl ProviderAdapter for D21aCountingRestProvider {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: "d21a counting markdown".to_string(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

/// W0-3 N5-2(T20b 规划面接窗):`drive_review_session` 入口无条件开窗,
/// session.repository_path=None 时窗口目标回落测试进程 CWD(包根,
/// 含 target/ 数 GB)→ baseline 快照超限 → run 失败、provider 零启动。
/// 注入小体积真实 git 仓(init+一次 commit),与生产形态(评审目标=
/// 业务仓)一致,快照秒级完成。
fn d21a_business_git_repository(root: &Path, tag: &str) -> std::path::PathBuf {
    let repo = root.join(format!("business-repo-{tag}"));
    std::fs::create_dir_all(&repo).expect("d21a business repo dir");
    std::fs::write(repo.join("README.md"), "d2.1a business repository\n")
        .expect("d21a business repo readme");
    for arguments in [
        vec![
            "init".to_string(),
            "-q".to_string(),
            "--initial-branch=main".to_string(),
        ],
        vec!["add".to_string(), "-A".to_string()],
        vec![
            "commit".to_string(),
            "-q".to_string(),
            "-m".to_string(),
            "d21a fixture baseline".to_string(),
        ],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "-c",
                "user.email=d21a-fixture@example.invalid",
                "-c",
                "user.name=d21a fixture",
            ])
            .args(&arguments)
            .status()
            .expect("d21a business repo git command spawns");
        assert!(
            status.success(),
            "d21a business repo git {arguments:?} must succeed"
        );
    }
    repo
}

/// manager/WS 面真实 fixture：真 LifecycleStore 会话记录 + 持久 engine +
/// PendingStart 族 provider（ClaudeCode=author、Codex=reviewer 均挂起）。
struct D21aManagerFixture {
    app_paths: ProductAppPaths,
    scope: EntityScope,
    manager: Arc<WorkspaceSessionManager>,
    run_context: ProviderRunContext,
    starts: Arc<AtomicUsize>,
    _held: Arc<SyncMutex<Vec<mpsc::Sender<ProviderEvent>>>>,
    _outbound_rx: mpsc::Receiver<OutboundControl>,
    _engine_rx: mpsc::Receiver<EngineEvent>,
}

async fn d21a_manager_fixture(tag: &str) -> D21aManagerFixture {
    let root = tempfile::tempdir().expect("d21a fixture root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let (project_id, entity_id) = (format!("project_d21a_{tag}"), format!("story_d21a_{tag}"));
    // W0-3 N5-2:repository_path 注入后,issue 基线解析(resolve_issue_
    // baseline_tree)从"无仓库路径跳过"变为真实读取——须补 issue 记录
    //(store 生成 id,session 与 issue 链一致),否则 product_store_
    // not_found fail-closed、provider 零启动。
    let issue = IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: project_id.clone(),
            repo_id: None,
            logical_codebase_id: None,
            title: format!("d2.1a manager issue {tag}"),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("d21a manager issue");
    let issue_id = issue.id;
    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            entity_id: entity_id.clone(),
            workspace_type: WorkspaceType::Story,
            author_provider: crate::product::models::ProviderName::ClaudeCode,
            reviewer_provider: Some(crate::product::models::ProviderName::Codex),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("d21a workspace session");
    let (engine_tx, engine_rx) = mpsc::channel::<EngineEvent>(256);
    let mut session = WorkspaceSession::from_record(session_record.clone());
    // W0-3 N5-2:窗口目标锚定小体积真实 git 仓(生产形态=业务仓),
    // 避免 current_dir 回落包根导致 baseline 超限、provider 零启动。
    session.repository_path = Some(d21a_business_git_repository(root.path(), tag));
    let engine = Arc::new(Mutex::new(WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(
            app_paths.issue_lifecycle_root(&project_id, &issue_id),
        )),
        lifecycle,
        engine_tx.clone(),
        session,
    )));
    let starts = Arc::new(AtomicUsize::new(0));
    let held: Arc<SyncMutex<Vec<mpsc::Sender<ProviderEvent>>>> =
        Arc::new(SyncMutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry.register(
        crate::product::models::ProviderName::ClaudeCode,
        Arc::new(D21aPendingProvider {
            starts: starts.clone(),
            held: held.clone(),
        }),
    );
    registry.register(
        crate::product::models::ProviderName::Codex,
        Arc::new(D21aPendingProvider {
            starts: starts.clone(),
            held: held.clone(),
        }),
    );
    let (_outbound_tx, outbound_rx) = mpsc::channel(64);
    let run_context = ProviderRunContext::test_fixture(
        Arc::new(registry),
        engine,
        WorkspaceRunRegistry::default(),
        session_record.id.clone(),
        app_paths.clone(),
        session_record,
    );
    let manager = run_context.manager.clone();
    // root 必须活过 fixture（tempdir drop 即清理）：泄漏保活，测试进程内可接受。
    std::mem::forget(root);
    D21aManagerFixture {
        app_paths,
        scope: EntityScope::new(
            &project_id,
            &issue_id,
            DeletionEntityKind::Story,
            &entity_id,
        ),
        manager,
        run_context,
        starts,
        _held: held,
        _outbound_rx: outbound_rx,
        _engine_rx: engine_rx,
    }
}

/// REST 面真实 fixture（无 manager）：project/repository/issue/story 全链
///（`run_next` 的 `workspace_repository_for_session` 解析需要真实 story
/// 归属与 Legacy 仓库路由）+ 直接 LifecycleStore 会话记录。
fn d21a_rest_fixture(tag: &str) -> (ProductAppPaths, EntityScope, WorkspaceSessionRecord) {
    let root = tempfile::tempdir().expect("d21a rest root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let root_path = root.path().to_path_buf();
    std::mem::forget(root);
    let project = ProjectStore::new(app_paths.clone())
        .create(CreateProjectInput {
            name: format!("d2.1a rest project {tag}"),
            description: None,
        })
        .expect("d2.1a rest project");
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: project.id.clone(),
            name: format!("d2.1a rest repository {tag}"),
            path: root_path,
            default_policy_preset: None,
            default_provider_mode: Some("fake".to_string()),
            idempotency_key: format!("d2-1a-rest-repository-{tag}"),
        })
        .expect("d2.1a rest repository");
    let issue = IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: Some(repository.id.clone()),
            logical_codebase_id: None,
            title: format!("d2.1a rest issue {tag}"),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("d2.1a rest issue");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: project.id.clone(),
            issue_id: issue.id.clone(),
            repository_id: repository.id,
            title: format!("d2.1a rest story {tag}"),
            aggregate_codebase: None,
        })
        .expect("d2.1a rest story");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: project.id.clone(),
            issue_id: issue.id.clone(),
            entity_id: story.id.clone(),
            workspace_type: WorkspaceType::Story,
            author_provider: crate::product::models::ProviderName::Fake,
            reviewer_provider: Some(crate::product::models::ProviderName::Fake),
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("d21a rest workspace session");
    (
        app_paths,
        EntityScope::new(&project.id, &issue.id, DeletionEntityKind::Story, &story.id),
        session,
    )
}

/// 等待条件成立（20ms 轮询，5s 上限）。
async fn d21a_wait_until(description: &str, mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("d21a wait timeout: {description}"));
}

/// 删除准入最终成功（run 释放后无 lease 泄漏的证据）。
async fn d21a_delete_admission_succeeds(
    operations: &EntityOperationStore,
    scope: &EntityScope,
) -> crate::product::entity_operation_store::EntityDeletionLease {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match operations.try_acquire_delete(scope) {
                Ok(lease) => return lease,
                Err(EntityOperationError::Busy) => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(other) => panic!("d21a unexpected deletion admission error: {other:?}"),
            }
        }
    })
    .await
    .expect("d21a deletion admission must succeed after run release (no lease leak)")
}

/// 业务树字节快照（排除删除机制自身元数据：entity-operations/、
/// entity-deletions/——D2.2 同口径）。
fn d21a_tree_snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == "entity-operations" || name == "entity-deletions" {
                    continue;
                }
                walk(&path, base, out);
            } else if let Ok(bytes) = std::fs::read(&path) {
                let relative = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(relative, bytes);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// fixture 的 .aria 根（ProductAppPaths::state_root 的父目录）。
fn d21a_fixture_root(app_paths: &ProductAppPaths) -> std::path::PathBuf {
    app_paths
        .state_root()
        .parent()
        .expect("d21a fixture root")
        .to_path_buf()
}

/// 主锚（plan Step 1 命名用例）：manager 真实入口与 REST run_next 的运行
/// lease——运行持 lease 时删除 busy/零内容变动；supersede 不丢 lease；
/// 释放后删除准入成功（不泄漏）；删除赢后 manager spawn 与 REST run_next
/// 均稳定拒且零 provider start。
#[tokio::test]
async fn cascade_d2_1a_real_manager_rest_lease() {
    let fixture = d21a_manager_fixture("umbrella").await;
    let operations = EntityOperationStore::new(fixture.app_paths.clone());

    // ① manager author 真实入口持 lease：删除 busy + 零内容变动。
    spawn_provider_run_from_handler(
        fixture.run_context.clone(),
        ProviderRunKind::Author {
            content: "d21a author run".to_string(),
        },
        mpsc::channel(1).0,
    )
    .await
    .expect("d2.1a author run must start");
    let starts = fixture.starts.clone();
    let manager = fixture.manager.clone();
    d21a_wait_until("author provider started and run registered", || {
        starts.load(Ordering::SeqCst) >= 1 && manager.is_active_run()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let root = d21a_fixture_root(&fixture.app_paths);
    let before = d21a_tree_snapshot(&root);
    match operations.try_acquire_delete(&fixture.scope) {
        Err(EntityOperationError::Busy) => {}
        Err(other) => panic!("d2.1a unexpected admission error while run holds lease: {other:?}"),
        Ok(_lease) => {
            panic!("d2.1a deletion must be entity_busy while manager run holds the lease")
        }
    }
    let after = d21a_tree_snapshot(&root);
    assert_eq!(
        before, after,
        "d2.1a: 运行持 lease 时删除准入失败必须零内容变动"
    );

    // ② supersede 接力不丢 lease：新 run 持有期间删除仍 busy。
    spawn_provider_run_from_handler(
        fixture.run_context.clone(),
        ProviderRunKind::Author {
            content: "d21a superseding run".to_string(),
        },
        mpsc::channel(1).0,
    )
    .await
    .expect("d2.1a superseding run must start");
    let starts = fixture.starts.clone();
    d21a_wait_until("superseding provider started", || {
        starts.load(Ordering::SeqCst) >= 2
    })
    .await;
    assert!(
        matches!(
            operations.try_acquire_delete(&fixture.scope),
            Err(EntityOperationError::Busy)
        ),
        "d2.1a: supersede 接力 run 必须继续持有 lease（不丢）"
    );

    // ③ abort 收口后删除准入成功（无 lease 泄漏），落 purging 墓碑。
    let aborted = fixture.manager.abort_active_run().await;
    assert!(aborted, "d2.1a active run must abort");
    let lease = d21a_delete_admission_succeeds(&operations, &fixture.scope).await;
    EntityDeletionStore::new(fixture.app_paths.clone())
        .begin(&lease)
        .expect("d2.1a purging tombstone begin");

    // ④ 删除赢后：manager 真实入口稳定拒、零新 provider start。
    let starts_before_reject = fixture.starts.load(Ordering::SeqCst);
    let rejected = spawn_provider_run_from_handler(
        fixture.run_context.clone(),
        ProviderRunKind::Author {
            content: "d2.1a post delete run".to_string(),
        },
        mpsc::channel(1).0,
    )
    .await
    .expect_err("d2.1a spawn after deletion must be rejected");
    assert!(
        rejected.contains("entity_deleting"),
        "d2.1a spawn rejection must carry stable entity_deleting code: {rejected}"
    );
    assert_eq!(
        fixture.starts.load(Ordering::SeqCst),
        starts_before_reject,
        "d2.1a: 删除赢后 provider 实际 start 计数必须为 0 增量"
    );
    drop(lease);

    // ⑤ REST run_next 同样：运行持 lease 时删除 busy；删除赢后稳定拒。
    let (rest_paths, rest_scope, rest_session) = d21a_rest_fixture("umbrella");
    let rest_operations = EntityOperationStore::new(rest_paths.clone());
    let rest_started = Arc::new(AtomicUsize::new(0));
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let runner = ProviderWorkspaceRunner::new(rest_paths.clone());
    let blocking_adapter = Arc::new(D21aBlockingRestProvider {
        starts: rest_started.clone(),
        release: SyncMutex::new(Some(release_rx)),
    });
    let rest_session_id = rest_session.id.clone();
    let run_handle = std::thread::spawn(move || {
        runner.run_next(
            WorkspaceProviderRunInput {
                session_id: rest_session_id,
                user_prompt: "d2.1a rest run".to_string(),
            },
            blocking_adapter.as_ref(),
        )
    });
    let rest_started_flag = rest_started.clone();
    d21a_wait_until("rest provider started", || {
        rest_started_flag.load(Ordering::SeqCst) == 1
    })
    .await;
    assert!(
        matches!(
            rest_operations.try_acquire_delete(&rest_scope),
            Err(EntityOperationError::Busy)
        ),
        "d2.1a: REST run_next 运行持 lease 时删除必须 entity_busy"
    );
    release_tx.send(()).expect("d2.1a release rest provider");
    let rest_output = run_handle
        .join()
        .expect("d2.1a rest run must complete while holding the lease")
        .expect("d2.1a rest run output");
    assert_eq!(rest_output.version.markdown, "d21a rest lease markdown");
    let rest_lease = d21a_delete_admission_succeeds(&rest_operations, &rest_scope).await;
    EntityDeletionStore::new(rest_paths.clone())
        .begin(&rest_lease)
        .expect("d2.1a rest purging tombstone begin");
    let counting_starts = Arc::new(AtomicUsize::new(0));
    let counting = D21aCountingRestProvider {
        starts: counting_starts.clone(),
    };
    let rejected_rest = ProviderWorkspaceRunner::new(rest_paths.clone())
        .run_next(
            WorkspaceProviderRunInput {
                session_id: rest_session.id.clone(),
                user_prompt: "d2.1a post delete rest run".to_string(),
            },
            &counting,
        )
        .expect_err("d2.1a REST run after deletion must be rejected");
    assert!(
        format!("{rejected_rest:?}").contains("entity_deleting"),
        "d2.1a REST rejection must carry stable entity_deleting code: {rejected_rest:?}"
    );
    assert_eq!(
        counting_starts.load(Ordering::SeqCst),
        0,
        "d2.1a: 删除赢后 REST provider 实际 start 计数必须为 0"
    );
    drop(rest_lease);
}

/// review（event-recovery 面经 spawn_provider_run_from_event）与 revision
/// （handler 面经 spawn_provider_run_from_handler）真实入口同样持 lease。
#[tokio::test]
async fn cascade_d2_1a_review_revision_event_entries_hold_lease() {
    let fixture = d21a_manager_fixture("entries").await;
    let operations = EntityOperationStore::new(fixture.app_paths.clone());

    // review：router 事件面（event-recovery 真实入口）。
    spawn_provider_run_from_event(
        fixture.run_context.clone(),
        ProviderRunKind::ReviewOnly,
        None,
        mpsc::channel(1).0,
    )
    .await
    .expect("d2.1a review run must start via event entry");
    let starts = fixture.starts.clone();
    let manager = fixture.manager.clone();
    d21a_wait_until("review provider started and run registered", || {
        starts.load(Ordering::SeqCst) >= 1 && manager.is_active_run()
    })
    .await;
    assert!(
        matches!(
            operations.try_acquire_delete(&fixture.scope),
            Err(EntityOperationError::Busy)
        ),
        "d2.1a: review run 持 lease 时删除必须 entity_busy"
    );
    assert!(fixture.manager.abort_active_run().await);

    // revision：handler 面（该 kind 在本 fixture 无待驱修订会话、立即收口，
    // 不适合 busy 方向观测——以删除赢方向证明同一 lease 收口点）。
    let lease = d21a_delete_admission_succeeds(&operations, &fixture.scope).await;
    EntityDeletionStore::new(fixture.app_paths.clone())
        .begin(&lease)
        .expect("d2.1a purging tombstone for revision entry");
    let revision_rejection = spawn_provider_run_from_handler(
        fixture.run_context.clone(),
        ProviderRunKind::Revision,
        mpsc::channel(1).0,
    )
    .await
    .expect_err("d2.1a revision spawn under purging tombstone must reject");
    assert!(
        revision_rejection.contains("entity_deleting"),
        "d2.1a revision 入口拒绝必须携带稳定 entity_deleting 码: {revision_rejection}"
    );
}

/// 删除先赢（purging 墓碑在位）：manager handler/event 两入口均稳定拒、
/// 零 provider start；非目标 manager（他实体）不受牵连继续运行。
#[tokio::test]
async fn cascade_d2_1a_delete_wins_zero_provider_start_and_bystander_keeps_running() {
    let target = d21a_manager_fixture("deletewins").await;
    let bystander = d21a_manager_fixture("bystander").await;
    let operations = EntityOperationStore::new(target.app_paths.clone());

    let lease = operations
        .try_acquire_delete(&target.scope)
        .expect("d2.1a delete admission before any run");
    EntityDeletionStore::new(target.app_paths.clone())
        .begin(&lease)
        .expect("d2.1a purging tombstone");

    let handler_rejection = spawn_provider_run_from_handler(
        target.run_context.clone(),
        ProviderRunKind::Author {
            content: "d2.1a deleted author".to_string(),
        },
        mpsc::channel(1).0,
    )
    .await
    .expect_err("d2.1a handler spawn under purging tombstone must reject");
    assert!(handler_rejection.contains("entity_deleting"));

    let event_rejection = spawn_provider_run_from_event(
        target.run_context.clone(),
        ProviderRunKind::Author {
            content: "d2.1a deleted event author".to_string(),
        },
        None,
        mpsc::channel(1).0,
    )
    .await
    .expect_err("d2.1a event spawn under purging tombstone must reject");
    assert!(event_rejection.contains("entity_deleting"));
    assert_eq!(
        target.starts.load(Ordering::SeqCst),
        0,
        "d2.1a: 删除赢后目标 manager provider start 计数必须为 0"
    );

    // 非目标 manager：同刻继续正常 spawn。
    spawn_provider_run_from_handler(
        bystander.run_context.clone(),
        ProviderRunKind::Author {
            content: "d2.1a bystander author".to_string(),
        },
        mpsc::channel(1).0,
    )
    .await
    .expect("d2.1a bystander manager must keep running");
    let starts = bystander.starts.clone();
    let manager = bystander.manager.clone();
    d21a_wait_until("bystander provider started", || {
        starts.load(Ordering::SeqCst) >= 1 && manager.is_active_run()
    })
    .await;
    drop(lease);
}

/// invalidate_for_entity_deletion：旧 Arc dispatch（start_run 族）与 attach
/// 检查后稳定拒；非目标 manager 不受牵连。
#[tokio::test]
async fn cascade_d2_1a_invalidate_blocks_old_arc_dispatch_and_attach() {
    let manager = WorkspaceSessionManager::test_fixture("session_d21a_invalidated");
    manager.invalidate_for_entity_deletion();

    let dispatch_rejection = manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect_err("d2.1a invalidated manager dispatch must reject");
    assert!(
        dispatch_rejection.contains("entity_deleted"),
        "d2.1a dispatch rejection must carry stable entity_deleted code: {dispatch_rejection}"
    );

    let (outbound_tx, mut outbound_rx) = mpsc::channel::<OutboundControl>(8);
    manager.register_attachment("d21a-stale-connection", outbound_tx);
    match tokio::time::timeout(Duration::from_secs(2), outbound_rx.recv()).await {
        Ok(Some(OutboundControl::Text(frame))) => {
            assert!(
                frame.contains("entity_deleted"),
                "d2.1a invalidated attach must receive stable entity_deleted error frame: {frame}"
            );
        }
        other => panic!("d2.1a invalidated attach must be rejected with an error frame: {other:?}"),
    }
    // 拒绝登记：invalidated manager 不再向该连接投递直播帧。
    manager.broadcast_test_keyframe(WsOutMessage::ProviderStatus {
        status: crate::web::workspace_ws_types::WsProviderStatus::Running,
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        outbound_rx.try_recv().is_err(),
        "d2.1a invalidated manager must not register attachments for live delivery"
    );

    // 非目标 manager 不受牵连。
    let bystander = WorkspaceSessionManager::test_fixture("session_d21a_bystander");
    bystander
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("d2.1a bystander manager dispatch must keep working");
}

/// disconnect 迟到 lease 打点与 degraded 诊断：purging/deleted 会话不重建
/// 已删除 timeline 分区；未删除实体原诊断落盘语义不变。
#[tokio::test]
async fn cascade_d2_1a_disconnect_and_degraded_diagnostics_skip_deleted_timeline() {
    let fixture = d21a_manager_fixture("diagnostics").await;
    let operations = EntityOperationStore::new(fixture.app_paths.clone());

    // holder 连接显式接管（bind_role Driver hello），随后进入删除。
    let (holder_tx, _holder_rx) = mpsc::channel::<OutboundControl>(8);
    fixture
        .manager
        .register_attachment("d21a-holder", holder_tx.clone());
    fixture.manager.bind_role(
        &holder_tx,
        crate::web::workspace_session::ConnectionRole::Driver,
        None,
    );

    // fixture 诊断分区路径在各自 app_paths state root 下（随 fixture 唯一），
    // 文件名与生产 lease/degraded diagnostics 契约一致。bind 的 `hold` 事件
    // 在删除开始前合法落盘——以此为基线，断言迟到的 release/degraded 打点
    // 零新增行、不重建分区。
    let diagnostics_root = fixture
        .app_paths
        .state_root()
        .join(&fixture.manager.session_id);
    let lease_diagnostics = diagnostics_root.join("lease-diagnostics.jsonl");
    let degraded_diagnostics = diagnostics_root.join("degraded-diagnostics.jsonl");
    d21a_wait_until("hold event written before deletion", || {
        lease_diagnostics.exists()
    })
    .await;
    let hold_baseline = std::fs::read(&lease_diagnostics).expect("hold baseline bytes");
    assert!(
        !degraded_diagnostics.exists(),
        "d2.1a: 删除前无 degraded 事件，分区不应存在"
    );

    let lease = operations
        .try_acquire_delete(&fixture.scope)
        .expect("d2.1a no run: delete admission must succeed");
    EntityDeletionStore::new(fixture.app_paths.clone())
        .begin(&lease)
        .expect("d2.1a purging tombstone for diagnostics");

    fixture
        .manager
        .handle_connection_closed("d21a-holder")
        .await; // record_release（disconnect 迟到 lease 打点）
    fixture.manager.record_degraded_enter("d2-1a-late-conn", 7);
    fixture.manager.record_degraded_exit("d2-1a-late-conn", 9);
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        std::fs::read(&lease_diagnostics).expect("lease diagnostics bytes after disconnect"),
        hold_baseline,
        "d2.1a: purging 会话 disconnect 迟到 lease 打点不得新增行/重建分区"
    );
    assert!(
        !degraded_diagnostics.exists(),
        "d2.1a: purging 会话 degraded 诊断不得重建分区"
    );
    drop(lease);

    // 未删除实体：原诊断落盘语义不变（降级打点仍写 durable）。
    let live = d21a_manager_fixture("diagnostics_live").await;
    live.manager.record_degraded_enter("d2-1a-live-conn", 3);
    let live_degraded = live
        .app_paths
        .state_root()
        .join(&live.manager.session_id)
        .join("degraded-diagnostics.jsonl");
    d21a_wait_until("live degraded diagnostics written", || {
        live_degraded.exists()
    })
    .await;
}

/// registry 重建面（旧 Arc 驱逐后的复活口）：purging/deleted 会话不得重建
/// manager——create 稳定拒；正常会话 create 不受影响。
#[tokio::test]
async fn cascade_d2_1a_registry_recreate_refuses_purging_session() {
    let root = tempfile::tempdir().expect("d2.1a recreate root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let project = ProjectStore::new(app_paths.clone())
        .create(CreateProjectInput {
            name: "d2.1a recreate project".to_string(),
            description: None,
        })
        .expect("project");
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: project.id.clone(),
            name: "d2.1a recreate repository".to_string(),
            path: root.path().to_path_buf(),
            default_policy_preset: None,
            default_provider_mode: Some("fake".to_string()),
            idempotency_key: "d2-1a-recreate-repository".to_string(),
        })
        .expect("repository");
    let issue = IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: Some(repository.id.clone()),
            logical_codebase_id: None,
            title: "d2.1a recreate issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("issue");
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: project.id.clone(),
            issue_id: issue.id.clone(),
            repository_id: repository.id,
            title: "d2.1a recreate story".to_string(),
            aggregate_codebase: None,
        })
        .expect("story");
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: project.id.clone(),
            issue_id: issue.id.clone(),
            entity_id: story.id.clone(),
            workspace_type: WorkspaceType::Story,
            author_provider: crate::product::models::ProviderName::ClaudeCode,
            reviewer_provider: Some(crate::product::models::ProviderName::Codex),
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("session");

    let state = WebAppState::with_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        ProviderRegistry::new(),
    );
    let healthy = WorkspaceSessionManager::create(&state, &session.id).await;
    assert!(
        healthy.is_ok(),
        "d2.1a 正常会话 create 不得受影响: {:?}",
        healthy.err()
    );

    let scope = EntityScope::new(&project.id, &issue.id, DeletionEntityKind::Story, &story.id);
    let lease = EntityOperationStore::new(app_paths.clone())
        .try_acquire_delete(&scope)
        .expect("delete admission");
    EntityDeletionStore::new(app_paths.clone())
        .begin(&lease)
        .expect("purging tombstone");

    let refused = WorkspaceSessionManager::create(&state, &session.id).await;
    assert!(
        refused.is_err(),
        "d2.1a purging 会话不得重建 manager（旧 Arc 不可复活）"
    );
    let message = refused.err().unwrap_or_default();
    assert!(
        message.contains("entity_deleting"),
        "d2.1a create 拒绝必须携带稳定 entity_deleting 码: {message}"
    );
}
