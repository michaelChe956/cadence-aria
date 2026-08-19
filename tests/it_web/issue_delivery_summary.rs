//! REQ-COD-06 Issue 交付状态聚合的 it_web 验收测试。
//!
//! 驱动真实 web 交付流（HTTP 建 coding attempt + WS `StartCoding` 全链到
//! ReviewRequest push），而非直接播种 Completed attempt / ReviewRequest。断言
//! `IssueStore.get().status` 与 `GET /api/issues/{issue_id}/lifecycle` 的
//! `delivery_summary` DTO。
//!
//! 夹具模式仿照 `tests/it_web/web_coding_attempt_api/part_20.rs`（逻辑代码库
//! issue/work item/attempt 播种 + fake git push 服务注入）：两个物理仓 + 两个已确认
//! WorkItem，经 `IdentityMigrationExecutor::ensure_identity_schema` 迁移为逻辑代码库，
//! 再保存显式 codebase selection（两个成员），使多仓 attempt 的 push 路由通过
//! fail-closed 校验。fake push 通过真实 `git push` 到每仓各自 bare remote 注入：
//! 成功场景 remote 正常；partial failure 场景给其中一个 remote 注入拒绝 pre-receive hook。
//! Legacy 单仓场景保持未迁移，验证单仓全推回归红线。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use axum::http::{Method, StatusCode};
use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use cadence_aria::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession, StreamChunk, StreamingProviderAdapter,
    StreamingProviderInput,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::CodingAttemptStore;
use cadence_aria::product::coding_models::{
    CodingAttemptStatus, CodingExecutionStage, CodingGateKind,
};
use cadence_aria::product::issue_store::{CreateProductIssueInput, IssueStore};
use cadence_aria::product::lifecycle_store::{CreateWorkItemInput, LifecycleStore};
use cadence_aria::product::logical_codebase::{
    IdentityMigrationExecutor, IssueCodebaseSelection, IssueCodebaseSelectionStore,
    LogicalCodebaseStore, LogicalRepositoryId,
};
use cadence_aria::product::models::{IssueStatus, ProviderName, WorkItemPlanStatus};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
use cadence_aria::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, AdapterRole, TimeoutStatus};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::coding_ws_handler::{CodingWsInMessage, CodingWsOutMessage};
use cadence_aria::web::gateway_factory::LogicalCodebaseGatewayFactory;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";

/// 串行化 WS 全链测试：避免多个 `cargo generate-lockfile`/`git` 子进程并发争抢。
static DELIVERY_WS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// 报告 ClaudeCode/Codex 恒健康的 availability gate：多仓（Logical）路径经 provider
/// gateway 会在 spawn 前复验 provider 可用性，而测试环境无真实 provider 二进制；
/// 用该 gate 让 ClaudeCode 恒可用（Fake 在 gateway 中不会被映射为 Fake）。
fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
    struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

    impl ProviderHealthSource for AlwaysHealthy {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }

        fn degraded(&self) -> bool {
            false
        }
    }

    let checked_at = Utc::now();
    let snapshot = Arc::new(ProviderHealthSnapshot {
        schema_version: 1,
        generation: 1,
        checked_at,
        providers: [ProviderName::ClaudeCode, ProviderName::Codex]
            .into_iter()
            .map(|provider| ProviderHealthEntry {
                provider,
                command: "stub".to_string(),
                available: true,
                version: Some("1.0".to_string()),
                reason_code: None,
                reason: None,
                checked_at,
            })
            .collect(),
    });
    Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
        snapshot,
    ))))
}

/// 同步 adapter stub：gateway 工厂构造签名需要，但本测试只走 streaming 路径，
/// sync adapter 不会被调起。
struct StubSyncAdapter;

impl ProviderAdapter for StubSyncAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: "ok".to_string(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

struct DeliveryFixture {
    _root: TempDir,
    app: axum::Router,
    store: CodingAttemptStore,
    attempt_ids: Vec<String>,
}

/// 交付全链 fixture 构建器。
///
/// `repo_dirs` 与 `work_item_ids` 一一对应；按序注册为 `repository_0001`、`repository_0002`…
/// `migrate=true` 时把 issue 迁移为逻辑代码库并保存显式 selection（多仓）；
/// `reject_push_repo_dir` 指定某个 repo dir 的 bare remote 注入拒绝 push 的 hook。
async fn build_delivery_fixture(
    repo_dirs: &[&str],
    work_item_ids: &[&str],
    migrate: bool,
    reject_push_repo_dir: Option<&str>,
) -> DeliveryFixture {
    assert_eq!(repo_dirs.len(), work_item_ids.len());
    let root = tempdir().expect("root");
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));

    ProjectStore::new(app_paths.clone())
        .create(CreateProjectInput {
            name: "Delivery".to_string(),
            description: None,
        })
        .expect("create project");

    let mut repository_ids = Vec::new();
    for (index, repo_dir) in repo_dirs.iter().enumerate() {
        let repo_path = root.path().join(repo_dir);
        let remote_path = root.path().join(format!("{repo_dir}-remote.git"));
        init_cargo_repo_with_remote(&repo_path, &remote_path);
        let repository = RepositoryStore::new(app_paths.clone())
            .create(CreateRepositoryInput {
                project_id: PROJECT_ID.to_string(),
                name: repo_dir.to_string(),
                path: repo_path,
                default_policy_preset: Some("manual-write".to_string()),
                // 用 claude_code 而非 fake：Logical（多仓）路径经 provider gateway，
                // `provider_ref_for_name` 仅支持 ClaudeCode/Codex，Fake 会被映射为
                // claude_code；这里用 claude_code 搭配注入的 always-available gate。
                default_provider_mode: Some("claude_code".to_string()),
                idempotency_key: format!("delivery-{repo_dir}"),
            })
            .expect("create repository");
        assert_eq!(repository.id, format!("repository_{:04}", index + 1));
        repository_ids.push(repository.id);
    }

    IssueStore::new(app_paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some(repository_ids[0].clone()),
            title: "交付状态聚合验收".to_string(),
            description: None,
            change_id: None,
        })
        .expect("create issue");

    let lifecycle = LifecycleStore::new(app_paths.clone());
    for (index, work_item_id) in work_item_ids.iter().enumerate() {
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: repository_ids[index].clone(),
                title: format!("工作项 {work_item_id}"),
                plan_status: WorkItemPlanStatus::Confirmed,
                ..Default::default()
            })
            .expect("create work item");
    }

    if migrate {
        IdentityMigrationExecutor::new(app_paths.clone())
            .ensure_identity_schema(PROJECT_ID)
            .expect("migrate fixture to logical codebase");
        let members = LogicalCodebaseStore::new(app_paths.clone())
            .list_members(PROJECT_ID)
            .expect("logical members");
        assert_eq!(members.len(), repo_dirs.len());
        let logical_ids: Vec<LogicalRepositoryId> = members
            .iter()
            .map(|member| member.logical_repository_id)
            .collect();
        IssueCodebaseSelectionStore::new(app_paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                PROJECT_ID,
                ISSUE_ID,
                logical_ids.clone(),
                Vec::new(),
                logical_ids,
                None,
            ))
            .expect("issue codebase selection includes all members");
    }

    if let Some(reject_dir) = reject_push_repo_dir {
        inject_rejecting_hook(&root.path().join(format!("{reject_dir}-remote.git")));
    }

    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(FullChainStreamingProvider) as Arc<dyn StreamingProviderAdapter>,
    );
    registry.register(
        ProviderName::Fake,
        Arc::new(FullChainStreamingProvider) as Arc<dyn StreamingProviderAdapter>,
    );
    let registry = Arc::new(registry);
    let state = WebAppState::with_events_and_provider_registry(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        cadence_aria::web::events::EventHub::new(),
        registry.clone(),
    )
    .with_gateway_factory(Arc::new(LogicalCodebaseGatewayFactory::new(
        app_paths.clone(),
        registry,
        Arc::new(StubSyncAdapter),
        always_available_gate(),
    )));
    let app = build_web_router(state);

    let mut attempt_ids = Vec::new();
    for work_item_id in work_item_ids {
        let (status, body) = crate::web_coding_attempt_api::request_json(
            app.clone(),
            Method::POST,
            &format!(
                "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/work-items/{work_item_id}/coding-attempts"
            ),
            json!({}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "create coding attempt for {work_item_id}: {body}"
        );
        attempt_ids.push(body["attempt_id"].as_str().expect("attempt id").to_string());
    }

    DeliveryFixture {
        _root: root,
        app,
        store: CodingAttemptStore::new(app_paths),
        attempt_ids,
    }
}

/// 驱动一个 WorkItem attempt 从 `StartCoding` 全链到 ReviewRequest 完成。
///
/// 无论 push 成功或失败（失败被记录为 `push_status=Failed`，不阻断主流程），单 WorkItem
/// attempt 都在 ReviewRequest 阶段以 `Completed` 收尾。
async fn drive_work_item_attempt_to_completion(app: axum::Router, attempt_id: &str) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/{attempt_id}");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;

    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    let mut confirmed_gates = HashSet::new();
    let mut observed = Vec::new();
    for _ in 0..240 {
        match timeout(Duration::from_secs(3), recv_json(&mut ws)).await {
            Ok(CodingWsOutMessage::CodingGateRequired { gate }) => {
                observed.push(format!(
                    "gate:{:?}:{:?}:reason={:?}",
                    gate.kind,
                    gate.stage.as_ref(),
                    gate.reason_code.as_deref()
                ));
                if gate.kind == CodingGateKind::StageGate
                    && let Some(stage) = gate.stage.clone()
                    && confirmed_gates.insert(gate.gate_id)
                {
                    send_json(&mut ws, &CodingWsInMessage::StageGateConfirm { stage }).await;
                }
            }
            Ok(CodingWsOutMessage::CodingSessionState { status, stage, .. }) => {
                observed.push(format!("state:{status:?}:{stage:?}"));
                if status == CodingAttemptStatus::Completed
                    && stage == CodingExecutionStage::ReviewRequest
                {
                    ws.close(None).await.expect("close ws");
                    server.abort();
                    return;
                }
            }
            Ok(CodingWsOutMessage::CodingProtocolError { code, message }) => {
                panic!("unexpected coding protocol error {code}: {message}");
            }
            Ok(_) => {}
            Err(_) => {
                panic!("work item attempt {attempt_id} did not complete; observed={observed:?}");
            }
        }
    }
    panic!(
        "work item attempt {attempt_id} did not complete within message budget; observed={observed:?}"
    );
}

/// 读取 issue lifecycle 的 delivery_summary DTO。
async fn delivery_summary(app: &axum::Router) -> Value {
    let (status, body) = crate::web_coding_attempt_api::request_json(
        app.clone(),
        Method::GET,
        &format!("/api/issues/{ISSUE_ID}/lifecycle?project_id={PROJECT_ID}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["delivery_summary"].clone()
}

/// 场景 1（多仓各仓交付）：逻辑 issue + 2 仓 work items 各自 coding 完成 + push 成功，
/// issue 落盘 `Completed` 且 DTO `overall == "all_pushed"`。
#[tokio::test]
async fn multi_repo_all_work_items_pushed_marks_issue_completed_all_pushed() {
    let _guard = DELIVERY_WS_TEST_LOCK.lock().await;
    let fixture = build_delivery_fixture(
        &["repo_alpha", "repo_beta"],
        &["work_item_0001", "work_item_0002"],
        true,
        None,
    )
    .await;

    for attempt_id in &fixture.attempt_ids {
        drive_work_item_attempt_to_completion(fixture.app.clone(), attempt_id).await;
    }

    let issue = IssueStore::new(fixture.store.paths())
        .get(PROJECT_ID, ISSUE_ID)
        .expect("issue");
    assert_eq!(issue.status, IssueStatus::Completed);

    let summary = delivery_summary(&fixture.app).await;
    assert_eq!(summary["overall"].as_str(), Some("all_pushed"));
    let entries = summary["entries"].as_array().expect("delivery entries");
    assert_eq!(entries.len(), 2);
    for entry in entries {
        assert_eq!(entry["attempt_status"].as_str(), Some("completed"));
        assert_eq!(entry["push_status"].as_str(), Some("pushed"));
        assert!(entry["push_error"].is_null());
    }
}

/// 场景 2（partial failure）：一仓 push 失败注入 → issue 状态未 Completed、DTO
/// `overall == "partial"`、失败行 `push_error` 非空。
#[cfg(unix)]
#[tokio::test]
async fn multi_repo_partial_push_failure_keeps_issue_open_and_reports_partial() {
    let _guard = DELIVERY_WS_TEST_LOCK.lock().await;
    let fixture = build_delivery_fixture(
        &["repo_alpha", "repo_beta"],
        &["work_item_0001", "work_item_0002"],
        true,
        Some("repo_beta"),
    )
    .await;

    for attempt_id in &fixture.attempt_ids {
        drive_work_item_attempt_to_completion(fixture.app.clone(), attempt_id).await;
    }

    let issue = IssueStore::new(fixture.store.paths())
        .get(PROJECT_ID, ISSUE_ID)
        .expect("issue");
    assert_ne!(
        issue.status,
        IssueStatus::Completed,
        "partial delivery must not complete the issue"
    );

    let summary = delivery_summary(&fixture.app).await;
    assert_eq!(summary["overall"].as_str(), Some("partial"));
    let entries = summary["entries"].as_array().expect("delivery entries");
    assert_eq!(entries.len(), 2);

    let pushed = entries
        .iter()
        .find(|entry| entry["push_status"].as_str() == Some("pushed"))
        .expect("one pushed entry");
    assert!(pushed["push_error"].is_null());

    let failed = entries
        .iter()
        .find(|entry| entry["push_status"].as_str() == Some("failed"))
        .expect("one failed entry");
    assert_eq!(failed["attempt_status"].as_str(), Some("completed"));
    let push_error = failed["push_error"].as_str().expect("push_error present");
    assert!(!push_error.is_empty(), "push_error must be non-empty");
}

/// 场景 3（Legacy 单仓全推）：单仓 issue 全推 → issue `Completed`（单仓红线回归）。
#[tokio::test]
async fn legacy_single_repo_all_pushed_marks_issue_completed() {
    let _guard = DELIVERY_WS_TEST_LOCK.lock().await;
    let fixture = build_delivery_fixture(&["repo_alpha"], &["work_item_0001"], false, None).await;

    for attempt_id in &fixture.attempt_ids {
        drive_work_item_attempt_to_completion(fixture.app.clone(), attempt_id).await;
    }

    let issue = IssueStore::new(fixture.store.paths())
        .get(PROJECT_ID, ISSUE_ID)
        .expect("issue");
    assert_eq!(issue.status, IssueStatus::Completed);

    let summary = delivery_summary(&fixture.app).await;
    assert_eq!(summary["overall"].as_str(), Some("all_pushed"));
    let entries = summary["entries"].as_array().expect("delivery entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["attempt_status"].as_str(), Some("completed"));
    assert_eq!(entries[0]["push_status"].as_str(), Some("pushed"));
}

fn init_cargo_repo_with_remote(repo: &Path, remote: &Path) {
    fs::create_dir_all(repo.join("src")).expect("create src");
    fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"delivery-summary\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write cargo manifest");
    fs::write(
        repo.join("src/lib.rs"),
        "pub fn climb_stairs(_n: u32) -> u32 { 0 }\n",
    )
    .expect("write lib");
    run_command(repo, "cargo", &["generate-lockfile"]);
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "aria@example.com"]);
    run_git(repo, &["config", "user.name", "Aria Test"]);
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);

    run_git(
        repo.parent().expect("repo parent"),
        &["init", "--bare", remote.to_str().expect("remote path")],
    );
    run_git(
        repo,
        &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("remote path"),
        ],
    );
}

/// 注入拒绝 push 的 pre-receive hook，使 review_request push 必然失败（同 T6 模式）。
#[cfg(unix)]
fn inject_rejecting_hook(remote: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let hook = remote.join("hooks/pre-receive");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").expect("rejecting hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("chmod hook");
}

fn run_git(cwd: &Path, args: &[&str]) {
    run_command(cwd, "git", args);
}

fn run_command(cwd: &Path, program: &str, args: &[&str]) {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run command");
    assert!(
        output.status.success(),
        "{program} {args:?} failed\nstdout:{}\nstderr:{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn send_json(ws: &mut WsStream, message: &CodingWsInMessage) {
    ws.send(Message::Text(
        serde_json::to_string(message).unwrap().into(),
    ))
    .await
    .expect("send ws message");
}

async fn recv_json(ws: &mut WsStream) -> CodingWsOutMessage {
    let message = timeout(Duration::from_secs(10), ws.next())
        .await
        .expect("ws message timeout")
        .expect("ws message")
        .expect("valid ws message");
    match message {
        Message::Text(text) => serde_json::from_str(&text).expect("ws json"),
        other => panic!("expected text ws message, got {other:?}"),
    }
}

const CLIMB_STAIRS_LIB: &str = r#"pub fn climb_stairs(n: u32) -> u32 {
    if n <= 2 {
        return n;
    }
    let mut prev = 1;
    let mut curr = 2;
    for _ in 3..=n {
        let next = prev + curr;
        prev = curr;
        curr = next;
    }
    curr
}

#[cfg(test)]
mod tests {
    use super::climb_stairs;

    #[test]
    fn computes_climb_stairs_examples() {
        assert_eq!(climb_stairs(1), 1);
        assert_eq!(climb_stairs(2), 2);
        assert_eq!(climb_stairs(3), 3);
        assert_eq!(climb_stairs(5), 8);
        assert_eq!(climb_stairs(10), 89);
    }
}
"#;

struct FullChainStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for FullChainStreamingProvider {
    /// 逻辑代码库（Logical）路径经 gateway 调用 `start`；Legacy 路径调用 `run_streaming`。
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(8);
        let (command_tx, _command_rx) = tokio::sync::mpsc::channel(8);
        let role = input.role.clone();
        let working_dir = input.working_dir.clone();
        let contract = input.structured_output_contract.clone();
        tokio::spawn(async move {
            match role {
                AdapterRole::Executor => {
                    if let Err(error) = fs::write(working_dir.join("src/lib.rs"), CLIMB_STAIRS_LIB)
                    {
                        let _ = event_tx
                            .send(ProviderEvent::Failed {
                                message: error.to_string(),
                            })
                            .await;
                        return;
                    }
                    let _ = event_tx
                        .send(ProviderEvent::TextDelta {
                            content: "implemented climb_stairs".to_string(),
                        })
                        .await;
                    let _ = event_tx
                        .send(ProviderEvent::Completed(ProviderCompletion::from_output(
                            "implemented climb_stairs".to_string(),
                            contract.as_ref(),
                            None,
                        )))
                        .await;
                }
                AdapterRole::Reviewer => {
                    let _ = event_tx
                        .send(ProviderEvent::TextDelta {
                            content: "review approved".to_string(),
                        })
                        .await;
                    let _ = event_tx
                        .send(ProviderEvent::Completed(ProviderCompletion::from_output(
                            r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
                                .to_string(),
                            contract.as_ref(),
                            None,
                        )))
                        .await;
                }
                _ => {
                    let _ = event_tx
                        .send(ProviderEvent::Completed(ProviderCompletion::plain(
                            "ok".to_string(),
                            None,
                        )))
                        .await;
                }
            }
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        match input.role {
            AdapterRole::Executor => {
                let worktree = input
                    .worktree_path
                    .as_ref()
                    .map(PathBuf::from)
                    .expect("worktree path");
                fs::write(worktree.join("src/lib.rs"), CLIMB_STAIRS_LIB).map_err(|error| {
                    ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                })?;
                tx.try_send(StreamChunk::Text("implemented climb_stairs".to_string()))
                    .expect("send coding chunk");
                tx.try_send(StreamChunk::Done {
                    full_output: "implemented climb_stairs".to_string(),
                })
                .expect("send coding done");
            }
            AdapterRole::Reviewer => {
                tx.try_send(StreamChunk::Text("review approved".to_string()))
                    .expect("send review chunk");
                tx.try_send(StreamChunk::Done {
                    full_output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
                        .to_string(),
                })
                .expect("send review done");
            }
            _ => {
                tx.try_send(StreamChunk::Done {
                    full_output: "ok".to_string(),
                })
                .expect("send done");
            }
        }
        Ok(rx)
    }
}
