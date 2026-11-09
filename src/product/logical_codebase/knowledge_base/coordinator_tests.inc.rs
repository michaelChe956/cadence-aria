// `coordinator.rs` 的内联测试（按仓内惯例拆 `*_tests.inc.rs` + `include!`）。
// provider turn 一律走假 runner / 假 trust，不启动任何 provider 进程。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderCompletion, ProviderEvent, ProviderSession,
};
use crate::product::logical_codebase::provider_trust::{
    ProviderTrustPreparationResult, ProviderTrustScenario, ProviderTrustWaiting,
};

const PROJECT: &str = "project_0001";
const LC: &str = "lc_alpha";
const NOW: &str = "2026-11-09T00:00:00Z";
const PACKAGE: &str = "cadence/knowledge-base/user-input/updates/CHANGE-x";

// ───────────────────────── 假 runner / 假 trust ─────────────────────────

#[derive(Clone)]
enum Script {
    /// 在聚合根下写这些文件后正常完成。
    Writes(Vec<(&'static str, &'static str)>),
    /// 删除指定文件后正常完成（provider 删除平台配置的场景）。
    DeletesThenOk(&'static str),
    /// turn 失败。
    Fail { code: &'static str, retryable: bool },
    /// 先写这些文件再失败（provider 越界写后报错/超时）。
    WritesThenFail {
        files: Vec<(&'static str, &'static str)>,
        code: &'static str,
        retryable: bool,
    },
    /// 先写这些文件，再由取消信号触发 Cancelled（provider 越界写后被取消）。
    WritesThenCancel(Vec<(&'static str, &'static str)>),
    /// 先写这些文件再返回 CancelledStopUnconfirmed（Abort 后 provider 仍在跑）。
    WritesThenCancelUnconfirmed(Vec<(&'static str, &'static str)>),
    /// 等取消信号后返回 Cancelled。
    WaitForCancel,
    /// turn 开始即通知 `started`，等 `proceed` 放行后写文件并成功完成
    /// （测试需要在 turn 进行中做动作时使用）。
    GatedWrites {
        files: Vec<(&'static str, &'static str)>,
        started: Arc<tokio::sync::Notify>,
        proceed: Arc<tokio::sync::Notify>,
    },
    /// turn 开始先写文件并通知 `started`，等 `proceed` 放行后返回失败
    /// （模拟「取消 API 与失败 turn 并发」的交叉情形）。
    GatedWritesThenFail {
        files: Vec<(&'static str, &'static str)>,
        code: &'static str,
        retryable: bool,
        started: Arc<tokio::sync::Notify>,
        proceed: Arc<tokio::sync::Notify>,
    },
}

fn write_files(root: &Path, files: &[(&'static str, &'static str)]) {
    for (rel, content) in files {
        let target = root.join(rel);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedCall {
    command: String,
    provider: ProviderName,
    canonical_root: PathBuf,
}

struct FakeRunner {
    script: Script,
    calls: Mutex<Vec<RecordedCall>>,
}

impl FakeRunner {
    fn new(script: Script) -> Arc<Self> {
        Arc::new(Self {
            script,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<RecordedCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl KnowledgeBaseTurnRunner for FakeRunner {
    async fn run_turn(
        &self,
        request: KnowledgeBaseTurnRequest<'_>,
    ) -> Result<String, KnowledgeBaseTurnError> {
        self.calls.lock().unwrap().push(RecordedCall {
            command: request.command.to_string(),
            provider: request.provider.clone(),
            canonical_root: request.canonical_root.to_path_buf(),
        });
        match &self.script {
            Script::Writes(files) => {
                write_files(request.canonical_root, files);
                Ok("turn ok".to_string())
            }
            Script::DeletesThenOk(path) => {
                std::fs::remove_file(request.canonical_root.join(path)).unwrap();
                Ok("turn ok".to_string())
            }
            Script::Fail { code, retryable } => Err(KnowledgeBaseTurnError::Failed {
                reason_code: (*code).to_string(),
                summary: "provider exploded".to_string(),
                retryable: *retryable,
            }),
            Script::WritesThenFail {
                files,
                code,
                retryable,
            } => {
                write_files(request.canonical_root, files);
                Err(KnowledgeBaseTurnError::Failed {
                    reason_code: (*code).to_string(),
                    summary: "provider exploded".to_string(),
                    retryable: *retryable,
                })
            }
            Script::WritesThenCancel(files) => {
                write_files(request.canonical_root, files);
                request.cancellation.cancel();
                Err(KnowledgeBaseTurnError::Cancelled)
            }
            Script::WritesThenCancelUnconfirmed(files) => {
                write_files(request.canonical_root, files);
                request.cancellation.cancel();
                Err(KnowledgeBaseTurnError::CancelledStopUnconfirmed)
            }
            Script::WaitForCancel => {
                request.cancellation.cancelled().await;
                Err(KnowledgeBaseTurnError::Cancelled)
            }
            Script::GatedWrites {
                files,
                started,
                proceed,
            } => {
                started.notify_one();
                proceed.notified().await;
                write_files(request.canonical_root, files);
                Ok("turn ok".to_string())
            }
            Script::GatedWritesThenFail {
                files,
                code,
                retryable,
                started,
                proceed,
            } => {
                write_files(request.canonical_root, files);
                started.notify_one();
                proceed.notified().await;
                Err(KnowledgeBaseTurnError::Failed {
                    reason_code: (*code).to_string(),
                    summary: "provider exploded".to_string(),
                    retryable: *retryable,
                })
            }
        }
    }
}

struct FakeTrust {
    waiting: bool,
    calls: AtomicUsize,
    scenarios: Mutex<Vec<ProviderTrustScenario>>,
}

impl FakeTrust {
    fn new(waiting: bool) -> Arc<Self> {
        Arc::new(Self {
            waiting,
            calls: AtomicUsize::new(0),
            scenarios: Mutex::new(Vec::new()),
        })
    }
}

impl ProviderTrustPrecondition for FakeTrust {
    fn ensure_ready(
        &self,
        _project_id: &str,
        _operation_id: &str,
        _lc_id: &str,
        canonical_root: &Path,
        providers: &[ProviderName],
        scenario: ProviderTrustScenario,
    ) -> ProviderTrustPreparationResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.scenarios.lock().unwrap().push(scenario);
        if self.waiting {
            return ProviderTrustPreparationResult::Waiting {
                waiting: ProviderTrustWaiting {
                    provider: providers[0].clone(),
                    canonical_root: canonical_root.to_path_buf(),
                    trust_key: "trust-key".to_string(),
                    reason_code: "codex_trust_entry_missing".to_string(),
                    message: "trust not ready for this provider".to_string(),
                    retry_action: "complete the provider authorization and retry".to_string(),
                    manual_guidance: String::new(),
                    command_markdown: None,
                    recorded_at: NOW.to_string(),
                },
            };
        }
        ProviderTrustPreparationResult::Ready {
            registrations: Vec::new(),
        }
    }
}

// ───────────────────────── 夹具 ─────────────────────────

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    /// 平台状态根（测试里用于构造聚合索引锁路径等平台视角路径）。
    paths: ProductAppPaths,
    runner: Arc<FakeRunner>,
    trust: Arc<FakeTrust>,
    coordinator: KnowledgeBaseCoordinator,
    store: KnowledgeBaseOperationStore,
}

fn fixture(script: Script, trust_waiting: bool) -> Fixture {
    let temp = tempfile::tempdir().expect("创建临时目录失败");
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let root = temp.path().join("aggregate-root");
    for dir in [
        "alpha/src",
        "cadence/knowledge-base",
        "cadence/project-rules",
    ] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(root.join("alpha/src/main.rs"), "fn main() {}\n").unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    let runner = FakeRunner::new(script);
    let trust = FakeTrust::new(trust_waiting);
    let clock: Arc<Clock> = Arc::new(|| NOW.to_string());
    let coordinator =
        KnowledgeBaseCoordinator::new(paths.clone(), runner.clone(), trust.clone(), clock)
            .for_lc(LC);
    let store = KnowledgeBaseOperationStore::for_lc(paths.clone(), LC.to_string());
    Fixture {
        paths,
        _temp: temp,
        root,
        runner,
        trust,
        coordinator,
        store,
    }
}

fn selection() -> AggregateProviderSelection {
    AggregateProviderSelection {
        selected: "claude_code".to_string(),
        candidates: Vec::new(),
        rule_version: "kb-v1".to_string(),
        decided_at: NOW.to_string(),
    }
}

fn input(key: &str, provider: &str, package: Option<&str>) -> KnowledgeBaseOperationInput {
    KnowledgeBaseOperationInput {
        idempotency_key: key.to_string(),
        provider: provider.to_string(),
        package_path: package.map(str::to_string),
        package_id: package.map(|_| "CHANGE-x".to_string()),
    }
}

async fn begin_bootstrap(
    fixture: &Fixture,
    key: &str,
) -> Result<KnowledgeBaseOperation, KnowledgeBaseError> {
    fixture
        .coordinator
        .begin(
            derive_operation_id(PROJECT, LC, key),
            PROJECT,
            LC,
            KnowledgeBaseOperationKind::Bootstrap,
            input(key, "claude_code", None),
            selection(),
        )
        .await
}

async fn begin_update(
    fixture: &Fixture,
    key: &str,
    provider: &str,
) -> Result<KnowledgeBaseOperation, KnowledgeBaseError> {
    fixture
        .coordinator
        .begin(
            derive_operation_id(PROJECT, LC, key),
            PROJECT,
            LC,
            KnowledgeBaseOperationKind::Update,
            input(key, provider, Some(PACKAGE)),
            selection(),
        )
        .await
}

// ───────────────────────── 单飞 ─────────────────────────

#[tokio::test]
async fn other_running_detects_running_knowledge_base_operation() {
    let fixture = fixture(Script::Writes(Vec::new()), false);
    assert_eq!(fixture.coordinator.other_running(PROJECT).unwrap(), None);

    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    assert_eq!(operation.status, KnowledgeBaseOperationStatus::Running);
    assert_eq!(
        fixture.coordinator.other_running(PROJECT).unwrap(),
        Some(operation.operation_id.clone())
    );

    fixture
        .store
        .finish_completed(PROJECT, &operation.operation_id, NOW.to_string())
        .unwrap();
    assert_eq!(fixture.coordinator.other_running(PROJECT).unwrap(), None);
}

#[tokio::test]
async fn begin_rejects_second_operation_while_one_is_running() {
    let fixture = fixture(Script::Writes(Vec::new()), false);
    let first = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();

    // bootstrap 与 update 互斥：任一知识库 operation 在跑时拒绝新启。
    let rejected = begin_update(&fixture, "kb-key-0002", "claude_code").await;
    match rejected {
        Err(KnowledgeBaseError::Preflight(
            KnowledgeBasePreflightFailure::OtherKnowledgeBaseRunning { operation_id },
        )) => assert_eq!(operation_id, first.operation_id),
        other => panic!("expected OtherKnowledgeBaseRunning, got {other:?}"),
    }
    assert!(
        fixture
            .store
            .get(PROJECT, &derive_operation_id(PROJECT, LC, "kb-key-0002"))
            .is_err(),
        "被拒绝的 operation 不得落盘"
    );
}

#[tokio::test]
async fn begin_is_idempotent_for_the_same_operation_id() {
    let fixture = fixture(Script::Writes(Vec::new()), false);
    let first = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    let replay = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    assert_eq!(
        first, replay,
        "同 idempotency_key 重放返回既有记录而非被单飞拒绝"
    );
}

#[tokio::test]
async fn begin_allows_new_operation_after_previous_one_finished() {
    let fixture = fixture(Script::Writes(Vec::new()), false);
    let first = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    fixture
        .coordinator
        .execute(
            PROJECT,
            &first.operation_id,
            &fixture.root,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    begin_update(&fixture, "kb-key-0002", "claude_code")
        .await
        .expect("上一条已完成后允许新启");
}

#[tokio::test]
async fn begin_rejects_invalid_input_without_creating_an_operation() {
    let fixture = fixture(Script::Writes(Vec::new()), false);
    let missing_package = fixture
        .coordinator
        .begin(
            derive_operation_id(PROJECT, LC, "kb-key-0001"),
            PROJECT,
            LC,
            KnowledgeBaseOperationKind::Update,
            input("kb-key-0001", "claude_code", None),
            selection(),
        )
        .await;
    assert!(matches!(
        missing_package,
        Err(KnowledgeBaseError::InvalidInput(_))
    ));
    let fake_provider = begin_bootstrap_with_provider(&fixture, "kb-key-0002", "fake").await;
    assert!(matches!(
        fake_provider,
        Err(KnowledgeBaseError::InvalidInput(_))
    ));
    let unknown_provider = begin_bootstrap_with_provider(&fixture, "kb-key-0003", "nope").await;
    assert!(matches!(
        unknown_provider,
        Err(KnowledgeBaseError::InvalidInput(_))
    ));
    assert_eq!(fixture.coordinator.other_running(PROJECT).unwrap(), None);
}

async fn begin_bootstrap_with_provider(
    fixture: &Fixture,
    key: &str,
    provider: &str,
) -> Result<KnowledgeBaseOperation, KnowledgeBaseError> {
    fixture
        .coordinator
        .begin(
            derive_operation_id(PROJECT, LC, key),
            PROJECT,
            LC,
            KnowledgeBaseOperationKind::Bootstrap,
            input(key, provider, None),
            selection(),
        )
        .await
}

#[tokio::test]
async fn begin_rejects_lc_that_differs_from_the_bound_coordinator() {
    let fixture = fixture(Script::Writes(Vec::new()), false);
    let result = fixture
        .coordinator
        .begin(
            derive_operation_id(PROJECT, "lc_beta", "kb-key-0001"),
            PROJECT,
            "lc_beta",
            KnowledgeBaseOperationKind::Bootstrap,
            input("kb-key-0001", "claude_code", None),
            selection(),
        )
        .await;
    assert!(matches!(result, Err(KnowledgeBaseError::InvalidInput(_))));
}

// ───────────────────────── execute：成功 / 命令 ─────────────────────────

#[tokio::test]
async fn execute_runs_bootstrap_command_in_aggregate_root_and_completes() {
    let fixture = fixture(
        Script::Writes(vec![("cadence/knowledge-base/manifest.yaml", "x: 1\n")]),
        false,
    );
    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    fixture
        .coordinator
        .execute(
            PROJECT,
            &operation.operation_id,
            &fixture.root,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(
        fixture.runner.calls(),
        vec![RecordedCall {
            command: "/knowledge-base-bootstrap".to_string(),
            provider: ProviderName::ClaudeCode,
            canonical_root: fixture.root.clone(),
        }]
    );
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Completed);
    assert!(stored.completed_at.is_some());
    // 成功收口必须同时闭合步骤：状态、完成时间与输出摘要都落盘。
    assert_eq!(stored.steps.len(), 1);
    assert_eq!(stored.steps[0].step_id, KnowledgeBaseStepKind::Bootstrap);
    assert_eq!(stored.steps[0].status, KnowledgeBaseStepStatus::Completed);
    assert_eq!(stored.steps[0].completed_at.as_deref(), Some(NOW));
    assert_eq!(stored.steps[0].output_summary.as_deref(), Some("turn ok"));
    assert!(stored.error.is_none());
}

#[tokio::test]
async fn execute_update_passes_provider_prefixed_command_with_package_path() {
    let fixture = fixture(Script::Writes(Vec::new()), false);
    let operation = begin_update(&fixture, "kb-key-0001", "codex")
        .await
        .unwrap();
    fixture
        .coordinator
        .execute(
            PROJECT,
            &operation.operation_id,
            &fixture.root,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let calls = fixture.runner.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].command,
        "$cadence-init:knowledge-base-update cadence/knowledge-base/user-input/updates/CHANGE-x"
    );
    assert_eq!(calls[0].provider, ProviderName::Codex);
}

#[tokio::test]
async fn execute_allows_knowledge_base_rule_and_root_entry_writes() {
    let fixture = fixture(
        Script::Writes(vec![
            ("cadence/knowledge-base/x.md", "kb"),
            ("cadence/project-rules/knowledge-base-usage.md", "rule"),
            ("CLAUDE.md", "claude"),
            ("AGENTS.md", "agents"),
        ]),
        false,
    );
    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    fixture
        .coordinator
        .execute(
            PROJECT,
            &operation.operation_id,
            &fixture.root,
            CancellationToken::new(),
        )
        .await
        .expect("允许面内写入不得判越界");
}

// ───────────────────────── execute：失败 / 取消 / trust ─────────────────────────

#[tokio::test]
async fn execute_fails_with_write_scope_violation_for_member_repository_write() {
    let fixture = fixture(
        Script::Writes(vec![
            ("alpha/src/main.rs", "fn main() { /* touched */ }\n"),
            ("cadence/knowledge-base/x.md", "ok"),
        ]),
        false,
    );
    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    let error = fixture
        .coordinator
        .execute(
            PROJECT,
            &operation.operation_id,
            &fixture.root,
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    match &error {
        KnowledgeBaseError::WriteScopeViolation { paths } => {
            assert_eq!(paths, &vec!["alpha/src/main.rs".to_string()]);
        }
        other => panic!("expected WriteScopeViolation, got {other:?}"),
    }
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Failed);
    let record = stored.error.expect("失败记录");
    assert_eq!(record.reason_code, "knowledge_base_write_scope_violation");
    assert!(!record.retryable);
    assert!(
        record
            .stderr_summary
            .as_deref()
            .is_some_and(|text| text.contains("alpha/src/main.rs"))
    );
    assert_eq!(
        fixture.coordinator.other_running(PROJECT).unwrap(),
        None,
        "失败后单飞槽位必须释放"
    );
}

#[tokio::test]
async fn execute_records_provider_turn_failure() {
    let fixture = fixture(
        Script::Fail {
            code: "provider_turn_timed_out",
            retryable: true,
        },
        false,
    );
    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    let error = fixture
        .coordinator
        .execute(
            PROJECT,
            &operation.operation_id,
            &fixture.root,
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, KnowledgeBaseError::TurnFailed { .. }));
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Failed);
    let record = stored.error.expect("失败记录");
    assert_eq!(record.reason_code, "knowledge_base_provider_turn_failed");
    assert_eq!(record.cause.as_deref(), Some("provider_turn_timed_out"));
    assert!(record.retryable);
    assert_eq!(record.provider.as_deref(), Some("claude_code"));
    assert_eq!(stored.steps[0].status, KnowledgeBaseStepStatus::Failed);
}

#[tokio::test]
async fn execute_marks_operation_cancelled_when_turn_is_cancelled() {
    let fixture = fixture(Script::WaitForCancel, false);
    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = fixture
        .coordinator
        .execute(
            PROJECT,
            &operation.operation_id,
            &fixture.root,
            cancellation,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, KnowledgeBaseError::Cancelled));
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Cancelled);
    assert_eq!(fixture.coordinator.other_running(PROJECT).unwrap(), None);
}

#[tokio::test]
async fn execute_tolerates_operation_already_cancelled_by_cancel_api() {
    let fixture = fixture(Script::WaitForCancel, false);
    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    fixture
        .coordinator
        .cancel(PROJECT, &operation.operation_id)
        .unwrap();
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Cancelled);

    // 已被 cancel API 置为 Cancelled 的 operation 不再是 Running：execute 拒绝启动。
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = fixture
        .coordinator
        .execute(
            PROJECT,
            &operation.operation_id,
            &fixture.root,
            cancellation,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, KnowledgeBaseError::NotRunning { .. }));
    assert!(fixture.runner.calls().is_empty());
}

#[tokio::test]
async fn execute_settles_cancellation_even_if_cancel_api_raced_the_turn() {
    // 取消信号到达后、execute 落盘 Cancelled 之前，cancel API 已先把记录置为
    // Cancelled：execute 收口时遇到非 Running 必须按已取消处理，不得报错。
    let fixture = fixture(Script::WaitForCancel, false);
    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    fixture
        .store
        .cancel(PROJECT, &operation.operation_id, NOW.to_string())
        .unwrap();
    let error = fixture.coordinator.settle_cancelled(
        PROJECT,
        &operation.operation_id,
        "claude_code",
        &[],
        false,
    );
    assert!(matches!(error, KnowledgeBaseError::Cancelled));
}

#[tokio::test]
async fn execute_stops_before_provider_turn_when_trust_is_waiting() {
    let fixture = fixture(Script::Writes(Vec::new()), true);
    let operation = begin_update(&fixture, "kb-key-0001", "codex")
        .await
        .unwrap();
    let error = fixture
        .coordinator
        .execute(
            PROJECT,
            &operation.operation_id,
            &fixture.root,
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, KnowledgeBaseError::TrustWaiting(_)));
    assert!(
        fixture.runner.calls().is_empty(),
        "trust 未就绪时 provider 零启动"
    );
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Failed);
    let record = stored.error.expect("失败记录");
    assert_eq!(record.reason_code, "aggregate_initialization_trust_waiting");
    assert!(record.retryable);
    assert_eq!(
        record.action,
        "complete the provider authorization and retry"
    );
    assert_eq!(
        fixture.trust.scenarios.lock().unwrap().as_slice(),
        &[ProviderTrustScenario::Recipe]
    );
}

#[tokio::test]
async fn ensure_trust_ready_gates_before_any_operation_exists() {
    let waiting = fixture(Script::Writes(Vec::new()), true);
    let error = waiting
        .coordinator
        .ensure_trust_ready(PROJECT, "kb-pending", &waiting.root, &ProviderName::Codex)
        .await
        .unwrap_err();
    assert!(matches!(error, KnowledgeBaseError::TrustWaiting(_)));
    assert_eq!(waiting.coordinator.other_running(PROJECT).unwrap(), None);

    let ready = fixture(Script::Writes(Vec::new()), false);
    ready
        .coordinator
        .ensure_trust_ready(PROJECT, "kb-pending", &ready.root, &ProviderName::Codex)
        .await
        .unwrap();
    assert_eq!(ready.trust.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn execute_rejects_operation_that_is_not_running() {
    let fixture = fixture(Script::Writes(Vec::new()), false);
    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    fixture
        .store
        .finish_completed(PROJECT, &operation.operation_id, NOW.to_string())
        .unwrap();
    let error = fixture
        .coordinator
        .execute(
            PROJECT,
            &operation.operation_id,
            &fixture.root,
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, KnowledgeBaseError::NotRunning { .. }));
    assert!(fixture.runner.calls().is_empty());
}

#[tokio::test]
async fn cancel_marks_running_operation_cancelled() {
    let fixture = fixture(Script::Writes(Vec::new()), false);
    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    fixture
        .coordinator
        .cancel(PROJECT, &operation.operation_id)
        .unwrap();
    assert_eq!(fixture.coordinator.other_running(PROJECT).unwrap(), None);
    // 已终态的 operation 不可再取消。
    assert!(
        fixture
            .coordinator
            .cancel(PROJECT, &operation.operation_id)
            .is_err()
    );
}

// ───────────────────────── 写入面审计（纯函数） ─────────────────────────

fn audit_root() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("创建临时目录失败");
    let root = std::fs::canonicalize(temp.path()).unwrap();
    for dir in [
        "alpha/src",
        "cadence/knowledge-base",
        "cadence/project-rules",
    ] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(root.join("alpha/src/lib.rs"), "one").unwrap();
    std::fs::write(root.join("README.md"), "root readme").unwrap();
    (temp, root)
}

#[test]
fn write_scope_audit_flags_member_repository_write() {
    let (_temp, root) = audit_root();
    let before = snapshot_aggregate_root(&root).unwrap();
    // 修改（大小变化）+ 新增成员仓文件。
    std::fs::write(root.join("alpha/src/lib.rs"), "three!").unwrap();
    std::fs::write(root.join("alpha/src/new.rs"), "n").unwrap();
    let after = snapshot_aggregate_root(&root).unwrap();
    assert_eq!(
        write_scope_violations(&before, &after),
        vec![
            "alpha/src/lib.rs".to_string(),
            "alpha/src/new.rs".to_string()
        ]
    );
}

#[test]
fn write_scope_audit_allows_knowledge_base_and_rule_files() {
    let (_temp, root) = audit_root();
    let before = snapshot_aggregate_root(&root).unwrap();
    std::fs::create_dir_all(root.join("cadence/knowledge-base/api")).unwrap();
    std::fs::write(root.join("cadence/knowledge-base/x.md"), "kb").unwrap();
    std::fs::write(root.join("cadence/knowledge-base/api/y.md"), "kb").unwrap();
    std::fs::write(
        root.join("cadence/project-rules/knowledge-base-usage.md"),
        "rule",
    )
    .unwrap();
    std::fs::write(root.join("CLAUDE.md"), "claude").unwrap();
    std::fs::write(root.join("AGENTS.md"), "agents").unwrap();
    let after = snapshot_aggregate_root(&root).unwrap();
    assert_eq!(
        write_scope_violations(&before, &after),
        Vec::<String>::new()
    );
}

#[test]
fn write_scope_audit_only_allows_root_level_entry_files_and_flags_deletions() {
    let (_temp, root) = audit_root();
    let before = snapshot_aggregate_root(&root).unwrap();
    // 子目录里的 CLAUDE.md 不属于允许面；根 README.md 被删除属于越界写。
    std::fs::write(root.join("alpha/CLAUDE.md"), "nested").unwrap();
    std::fs::remove_file(root.join("README.md")).unwrap();
    let after = snapshot_aggregate_root(&root).unwrap();
    assert_eq!(
        write_scope_violations(&before, &after),
        vec!["README.md".to_string(), "alpha/CLAUDE.md".to_string()]
    );
}

#[test]
fn write_scope_audit_ignores_git_index_refresh_and_unchanged_files() {
    let (_temp, root) = audit_root();
    std::fs::create_dir_all(root.join("alpha/.git")).unwrap();
    std::fs::write(root.join("alpha/.git/index"), "idx-1").unwrap();
    let before = snapshot_aggregate_root(&root).unwrap();
    // `git status` 之类的只读命令会刷新 .git/index：不算文件写入。
    std::fs::write(root.join("alpha/.git/index"), "idx-two-longer").unwrap();
    let after = snapshot_aggregate_root(&root).unwrap();
    assert_eq!(
        write_scope_violations(&before, &after),
        Vec::<String>::new()
    );
}

// ───────────────────────── gateway runner：turn 启动形状 ─────────────────────────

#[test]
fn gateway_runner_launch_request_is_coding_target_write_on_aggregate_root() {
    use crate::product::logical_codebase::{PolicyTarget, ProviderRef, SessionPolicyAction};
    let root = PathBuf::from("/tmp/kb-aggregate-root");
    let provider_ref = ProviderRef::pi("cap_managed_snapshot");
    let request = GatewayKnowledgeBaseTurnRunner::launch_request(&provider_ref, PROJECT, &root);
    assert_eq!(request.project_id, PROJECT);
    assert_eq!(request.provider, provider_ref);
    assert!(matches!(
        request.action,
        SessionPolicyAction::CodingTargetWrite
    ));
    assert_eq!(request.target, PolicyTarget::aggregate_root(root.clone()));
    assert_eq!(request.working_directory, root);
    assert_eq!(request.readable_roots, vec![root.clone()]);
    assert_eq!(request.writable_roots, vec![root]);
    assert_eq!(
        request.config_artifact_ref,
        "sha256:aggregate-initialization-managed-config"
    );
}

#[test]
fn gateway_runner_streaming_input_is_executor_auto_with_bounded_timeout() {
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use crate::product::logical_codebase::ProviderRef;
    use crate::protocol::contracts::{AdapterRole, ProviderType};
    let root = PathBuf::from("/tmp/kb-aggregate-root");
    for (provider_ref, expected) in [
        (ProviderRef::claude_code("s"), ProviderType::ClaudeCode),
        (ProviderRef::codex("s"), ProviderType::Codex),
        (ProviderRef::pi("s"), ProviderType::Pi),
        (ProviderRef::kimi_code("s"), ProviderType::KimiCode),
    ] {
        let input = GatewayKnowledgeBaseTurnRunner::streaming_input(
            &provider_ref,
            &root,
            "/knowledge-base-bootstrap",
            std::time::Duration::from_secs(1800),
        );
        assert_eq!(input.provider_type, expected);
        assert!(matches!(input.role, AdapterRole::Executor));
        assert!(matches!(
            input.permission_mode,
            ProviderPermissionMode::Auto
        ));
        assert_eq!(input.timeout_secs, 1800);
        assert_eq!(input.working_dir, root);
        assert_eq!(input.prompt, "/knowledge-base-bootstrap");
        assert!(
            input.tool_policy.is_none(),
            "Normal 相位不携带 bootstrap marker"
        );
        assert!(input.resume_provider_session_id.is_none());
    }
}

// ───────────────────────── gateway runner：事件消费语义 ─────────────────────────

fn scripted_session(
    events: Vec<ProviderEvent>,
) -> (
    ProviderSession,
    tokio::sync::mpsc::Sender<ProviderEvent>,
    tokio::sync::mpsc::Receiver<ProviderCommand>,
) {
    let (event_tx, events_rx) = tokio::sync::mpsc::channel(32);
    let (commands, command_rx) = tokio::sync::mpsc::channel(4);
    for event in events {
        event_tx.try_send(event).unwrap();
    }
    (
        ProviderSession {
            events: events_rx,
            commands,
            native_session_id: Some("kb-fake-session".to_string()),
        },
        event_tx,
        command_rx,
    )
}

#[tokio::test]
async fn consume_turn_returns_bounded_summary_on_completion() {
    let long = "x".repeat(KNOWLEDGE_BASE_TURN_OUTPUT_LIMIT * 2);
    let (session, _tx, _rx) = scripted_session(vec![
        ProviderEvent::TextDelta {
            content: "hello ".to_string(),
        },
        ProviderEvent::TextDelta { content: long },
        ProviderEvent::Completed(ProviderCompletion::plain("ignored", None)),
    ]);
    let summary = GatewayKnowledgeBaseTurnRunner::consume_turn(
        session,
        std::time::Duration::from_secs(5),
        std::time::Duration::from_secs(5),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(summary.starts_with("hello "));
    assert_eq!(summary.len(), KNOWLEDGE_BASE_TURN_OUTPUT_LIMIT);
}

#[tokio::test]
async fn consume_turn_maps_cancellation_to_cancelled_and_aborts_the_session() {
    // Abort 后事件流关闭 ⇒ provider 停止已确认 ⇒ 普通 Cancelled。
    let (session, tx, mut command_rx) = scripted_session(Vec::new());
    drop(tx);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let result = GatewayKnowledgeBaseTurnRunner::consume_turn(
        session,
        std::time::Duration::from_secs(5),
        std::time::Duration::from_secs(5),
        cancellation,
    )
    .await;
    assert!(matches!(result, Err(KnowledgeBaseTurnError::Cancelled)));
    assert!(matches!(command_rx.try_recv(), Ok(ProviderCommand::Abort)));
}

#[tokio::test]
async fn consume_turn_maps_cancellation_to_stop_unconfirmed_when_stream_stays_open() {
    // Abort 后宽限期内事件流未关闭 ⇒ provider 可能仍在写 ⇒ 停止未确认。
    let (session, _tx, mut command_rx) = scripted_session(Vec::new());
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let result = GatewayKnowledgeBaseTurnRunner::consume_turn(
        session,
        std::time::Duration::from_secs(5),
        std::time::Duration::from_millis(30),
        cancellation,
    )
    .await;
    assert!(matches!(
        result,
        Err(KnowledgeBaseTurnError::CancelledStopUnconfirmed)
    ));
    assert!(matches!(command_rx.try_recv(), Ok(ProviderCommand::Abort)));
}

#[tokio::test]
async fn consume_turn_times_out_as_retryable_failure() {
    let (session, _tx, mut command_rx) = scripted_session(Vec::new());
    let result = GatewayKnowledgeBaseTurnRunner::consume_turn(
        session,
        std::time::Duration::from_millis(30),
        std::time::Duration::from_millis(30),
        CancellationToken::new(),
    )
    .await;
    match result {
        Err(KnowledgeBaseTurnError::Failed {
            reason_code,
            retryable,
            ..
        }) => {
            assert_eq!(reason_code, "provider_turn_timed_out");
            assert!(retryable);
        }
        other => panic!("expected timeout failure, got {other:?}"),
    }
    assert!(matches!(command_rx.try_recv(), Ok(ProviderCommand::Abort)));
}

#[tokio::test]
async fn consume_turn_maps_provider_failure_stream_close_and_interaction() {
    let (session, _tx, _rx) = scripted_session(vec![ProviderEvent::Failed {
        message: "boom".to_string(),
    }]);
    let failed = GatewayKnowledgeBaseTurnRunner::consume_turn(
        session,
        std::time::Duration::from_secs(5),
        std::time::Duration::from_secs(5),
        CancellationToken::new(),
    )
    .await;
    match failed {
        Err(KnowledgeBaseTurnError::Failed {
            reason_code,
            summary,
            retryable,
        }) => {
            assert_eq!(reason_code, "provider_reported_failure");
            assert!(summary.contains("boom"));
            assert!(retryable);
        }
        other => panic!("expected provider failure, got {other:?}"),
    }

    // 事件流在 Completed 前关闭。
    let (session, tx, _rx) = scripted_session(Vec::new());
    drop(tx);
    let closed = GatewayKnowledgeBaseTurnRunner::consume_turn(
        session,
        std::time::Duration::from_secs(5),
        std::time::Duration::from_secs(5),
        CancellationToken::new(),
    )
    .await;
    assert!(matches!(
        closed,
        Err(KnowledgeBaseTurnError::Failed { ref reason_code, retryable: true, .. })
            if reason_code == "provider_stream_closed"
    ));
}
