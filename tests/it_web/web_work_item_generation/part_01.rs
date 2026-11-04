use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::streaming_provider::{
    FakeStreamingProvider, ProviderEvent, ProviderSession, StreamingProviderAdapter,
    StreamingProviderInput,
};
use cadence_aria::product::models::ProviderName;
use cadence_aria::protocol::contracts::{AdapterInput, AdapterOutput, AdapterRole, TimeoutStatus};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use cadence_aria::web::test_controls::TestControlledFakeStreamingProvider;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

#[derive(Debug, Clone)]
pub(crate) struct MockSplitProviderAdapter {
    pub(crate) output: Value,
    pub(crate) revision_output: Option<Value>,
}

impl ProviderAdapter for MockSplitProviderAdapter {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        let structured_output = if input.role == AdapterRole::WorkItemSplitter
            && self.revision_output.is_some()
            && (input.prompt.contains("局部重做（revision）")
                || input.prompt.contains("[revision_feedback]"))
        {
            self.revision_output.clone().unwrap()
        } else {
            self.output.clone()
        };
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            structured_output: Some(structured_output),
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

pub(crate) fn valid_split_output() -> Value {
    json!({
        "repository_profile": {
            "confidence": "high",
            "detected_layers": ["backend", "frontend"],
            "split_recommendation": "frontend_backend",
            "languages": ["rust"],
            "frameworks": [],
            "package_managers": ["cargo"],
            "test_frameworks": [],
            "build_systems": ["cargo"],
            "verification_capabilities": ["cargo test"],
            "uncertainties": []
        },
        "work_items": [
            {
                "title": "实现后端登录会话 API",
                "kind": "backend",
                "sequence_hint": 10,
                "depends_on": [],
                "exclusive_write_scopes": ["src/product/session.rs"],
                "forbidden_write_scopes": ["web/**"],
                "require_execution_plan_confirm": false
            },
            {
                "title": "实现前端会话过期提示",
                "kind": "frontend",
                "sequence_hint": 20,
                "depends_on": [0],
                "exclusive_write_scopes": ["web/src/session/**"],
                "forbidden_write_scopes": ["src/product/**"],
                "require_execution_plan_confirm": false
            },
            {
                "title": "集成测试：会话过期端到端",
                "kind": "integration",
                "sequence_hint": 30,
                "depends_on": [1],
                "exclusive_write_scopes": ["tests/session/**"],
                "forbidden_write_scopes": [],
                "require_execution_plan_confirm": false
            }
        ],
        "verification_plans": [
            {
                "scope": "unit",
                "commands": [
                    {
                        "label": "cargo test backend",
                        "command": "cargo test --lib session",
                        "cwd": "",
                        "purpose": "backend unit tests",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved"
                    }
                ],
                "manual_checks": [],
                "required_gates": [],
                "risk_notes": [],
                "confidence": "high",
                "fallback_policy": "manual_gate"
            },
            {
                "scope": "unit",
                "commands": [
                    {
                        "label": "cargo test frontend",
                        "command": "cargo test --lib frontend_session",
                        "cwd": "",
                        "purpose": "frontend unit tests",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved"
                    }
                ],
                "manual_checks": [],
                "required_gates": [],
                "risk_notes": [],
                "confidence": "high",
                "fallback_policy": "manual_gate"
            },
            {
                "scope": "integration",
                "commands": [
                    {
                        "label": "cargo test integration",
                        "command": "cargo test --test session_integration",
                        "cwd": "",
                        "purpose": "integration tests",
                        "required": true,
                        "timeout_seconds": 180,
                        "safety": "approved"
                    }
                ],
                "manual_checks": [],
                "required_gates": [],
                "risk_notes": [],
                "confidence": "high",
                "fallback_policy": "manual_gate"
            }
        ]
    })
}

// 退役留档（T5/REQ-RET-02）：valid_outline_output 夹具为 outline 族测试专用，随消息族
// 退役（下游消费面 app_with_confirmed_story_and_design_and_streaming_outputs 一并退役）。

// 退役留档（T5/REQ-RET-02）：context_blocker_outline_output 夹具（ContextBlocker 面已随 L2 删除）随消息族退役。

// 退役留档（T5/REQ-RET-02）：invalid_outline_output_duplicate_ids 夹具为 outline staged 校验重试测试专用，随消息族退役。

// 退役留档（T5/REQ-RET-02）：malformed_outline_structured_stdout 夹具为 outline structured 解析重试测试专用，随消息族退役。

// 退役留档（T5/REQ-RET-02）：valid_revision_redo_output 夹具为 legacy full-candidate revision 流测试专用，随消息族退役。

pub(crate) fn invalid_split_output_missing_e2e() -> Value {
    json!({
        "repository_profile": {
            "confidence": "high",
            "detected_layers": ["backend"],
            "split_recommendation": "backend_only",
            "languages": ["rust"],
            "frameworks": [],
            "package_managers": ["cargo"],
            "test_frameworks": [],
            "build_systems": ["cargo"],
            "verification_capabilities": ["cargo test"],
            "uncertainties": []
        },
        "work_items": [
            {
                "title": "实现后端登录会话 API",
                "kind": "backend",
                "sequence_hint": 10,
                "depends_on": [],
                "exclusive_write_scopes": ["src/product/session.rs"],
                "forbidden_write_scopes": ["web/**"],
                "require_execution_plan_confirm": false
            }
        ],
        "verification_plans": [
            {
                "scope": "unit",
                "commands": [
                    {
                        "label": "cargo test backend",
                        "command": "cargo test --lib session",
                        "cwd": "",
                        "purpose": "backend unit tests",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved"
                    }
                ],
                "manual_checks": [],
                "required_gates": [],
                "risk_notes": [],
                "confidence": "high",
                "fallback_policy": "manual_gate"
            }
        ]
    })
}

#[derive(Clone)]
pub(crate) struct QueuedSplitStreamingProvider {
    outputs: Arc<Mutex<VecDeque<Value>>>,
    captured_prompts: Option<Arc<Mutex<Vec<String>>>>,
}

impl QueuedSplitStreamingProvider {
    pub(crate) fn new(outputs: Vec<Value>) -> Self {
        Self {
            outputs: Arc::new(Mutex::new(VecDeque::from(outputs))),
            captured_prompts: None,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for QueuedSplitStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        if input.role != AdapterRole::WorkItemSplitter {
            return FakeStreamingProvider.start(input, cancel).await;
        }
        if let Some(captured_prompts) = &self.captured_prompts {
            captured_prompts
                .lock()
                .expect("captured prompts lock")
                .push(input.prompt.clone());
        }
        let mut output = self
            .outputs
            .lock()
            .expect("queued split outputs lock")
            .pop_front()
            .unwrap_or_else(valid_split_output);
        let nonce = input
            .structured_output_contract
            .as_ref()
            .map(|contract| contract.nonce.clone())
            .unwrap_or_else(|| {
                input
                    .prompt
                    .split_once("<ARIA_STRUCTURED_OUTPUT nonce=\"")
                    .and_then(|(_, tail)| tail.split_once('"'))
                    .map(|(nonce, _)| nonce.to_string())
                    .unwrap_or_else(|| "FAKE0001".to_string())
            });
        if let Some(object) = output.as_object_mut() {
            object.insert("nonce".to_string(), serde_json::json!(nonce));
        }
        let full_output = Some(format!(
            "Fake Work Item Plan streaming draft\n\n\
             <ARIA_STRUCTURED_OUTPUT nonce=\"{nonce}\">{output}</ARIA_STRUCTURED_OUTPUT>",
        ));
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);

        tokio::spawn(async move {
            let Some(full_output) = full_output else {
                cancel.cancelled().await;
                return;
            };
            if cancel.is_cancelled() {
                return;
            }
            if event_tx
                .send(ProviderEvent::TextDelta {
                    content: full_output.clone(),
                })
                .await
                .is_err()
            {
                return;
            }
            if cancel.is_cancelled() {
                return;
            }
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        full_output,
                        None,
                    ),
                ))
                .await;
        });

        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

pub(crate) async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json")
    };
    (status, value)
}

async fn bootstrap_project_repo_issue_and_specs(
    app: axum::Router,
    repo: &std::path::Path,
) -> axum::Router {
    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    assert!(status.is_success(), "bootstrap project failed: {response}");
    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "bootstrap repository initialization start failed: {response}"
    );
    let operation_id = response["operation_id"]
        .as_str()
        .expect("bootstrap repository initialization operation id");
    let operation_uri =
        format!("/api/projects/project_0001/repository-initializations/{operation_id}");
    let mut last_snapshot = response;
    let mut completed = None;
    for _ in 0..100 {
        let (status, snapshot) =
            request_json(app.clone(), Method::GET, &operation_uri, json!({})).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "bootstrap repository initialization poll failed: {snapshot}"
        );
        match snapshot["status"].as_str() {
            Some("completed") => {
                completed = Some(snapshot);
                break;
            }
            Some("failed") => panic!("bootstrap repository initialization failed: {snapshot}"),
            _ => last_snapshot = snapshot,
        }
        tokio::task::yield_now().await;
    }
    let completed = completed.unwrap_or_else(|| {
        panic!("bootstrap repository initialization did not complete: {last_snapshot}")
    });
    let repository_id = completed["result"]["repository"]["repository_id"]
        .as_str()
        .expect("bootstrap initialized repository id");
    // 块3（remove-single-repository-path）：issue 必须挂 logical_codebase_id 归属。
    // 按生产 v1.2→v1.3 身份迁移把 repository 升成 LC 权威，再经生产 rebuild
    // 端点建 active 聚合索引（generate 经 PlanningContextResolver 读取）；
    // story/design generate 显式钉 involved（REQ-PLN-04 确认门）。codegraph
    // sync 与并行夹具互踩 ⇒ 持 crate 级锁串行。
    let bootstrap_root = {
        let _codegraph_guard = crate::CODEGRAPH_FIXTURE_LOCK.lock().await;
        let (status, info) =
            request_json(app.clone(), Method::GET, "/api/runtime-info", json!({})).await;
        assert_eq!(status, StatusCode::OK, "{info}");
        let workspace_root =
            std::path::PathBuf::from(info["workspace_root"].as_str().expect("workspace_root"));
        let app_paths =
            cadence_aria::product::app_paths::ProductAppPaths::new(workspace_root.join(".aria"));
        cadence_aria::product::logical_codebase::IdentityMigrationExecutor::new(app_paths.clone())
            .ensure_identity_schema("project_0001")
            .expect("migrate fixture repository into logical codebase authority");
        let lc_id = cadence_aria::product::logical_codebase::store::legacy_logical_codebase_id(
            "project_0001",
        );
        let (status, index_rebuild) = request_json(
            app.clone(),
            Method::POST,
            &format!(
                "/api/projects/project_0001/logical-codebases/{lc_id}/aggregate-indexes/rebuild"
            ),
            json!({}),
        )
        .await;
        assert!(
            status.is_success(),
            "bootstrap index rebuild failed: {index_rebuild}"
        );
        let involved: Vec<String> =
            cadence_aria::product::logical_codebase::LogicalCodebaseStore::new(app_paths)
                .list_members("project_0001")
                .expect("list fixture logical members")
                .into_iter()
                .map(|member| member.logical_repository_id.0.to_string())
                .collect();
        let (status, response) = request_json(
            app.clone(),
            Method::POST,
            "/api/projects/project_0001/issues",
            json!({"title":"登录会话过期","description":"描述","repository_id":repository_id,"logical_codebase_id":lc_id}),
        )
        .await;
        assert!(status.is_success(), "bootstrap issue failed: {response}");
        involved
    };

    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title":"登录会话过期提示",
            "involved_repository_ids":bootstrap_root,
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":3,
            "superpowers_enabled":false,
            "openspec_enabled":true
        }),
    )
    .await;
    assert!(
        status.is_success(),
        "bootstrap story generation failed: {response}"
    );
    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    assert!(
        status.is_success(),
        "bootstrap story confirmation failed: {response}"
    );

    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"会话过期后端设计",
            "story_spec_ids":["story_spec_0001"],
            "involved_repository_ids":cadence_aria::product::logical_codebase::LogicalCodebaseStore::new(
                cadence_aria::product::app_paths::ProductAppPaths::new(
                    std::path::PathBuf::from(
                        request_json(app.clone(), Method::GET, "/api/runtime-info", json!({}))
                            .await
                            .1["workspace_root"]
                            .as_str()
                            .expect("workspace_root")
                            .to_string(),
                    )
                    .join(".aria"),
                ),
            )
            .list_members("project_0001")
            .expect("list fixture logical members")
            .into_iter()
            .map(|member| member.logical_repository_id.0.to_string())
            .collect::<Vec<String>>(),
            "author_provider":"codex",
            "reviewer_provider":"claude_code",
            "review_rounds":2,
            "superpowers_enabled":true,
            "openspec_enabled":true
        }),
    )
    .await;
    assert!(
        status.is_success(),
        "bootstrap design generation failed: {response}"
    );
    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0002/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;
    assert!(
        status.is_success(),
        "bootstrap design confirmation failed: {response}"
    );

    app
}

pub(crate) async fn app_with_confirmed_story_and_design(
    output: Value,
) -> (axum::Router, tempfile::TempDir) {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    // REQ-PIB-02：夹具对齐生产不变量（main 分支+初始提交，裸 init 无分支
    // 会令基线解析 fail-closed）。
    // 聚合索引布局规则：每成员至少 1 个可索引源码文件
    // （aggregate_index_member_coverage_failed），空提交不算。
    std::fs::create_dir_all(repo.join("src")).expect("repo src dir");
    std::fs::write(repo.join("src/main.rs"), "fn main() {}\n").expect("repo source");
    let status = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&repo)
        .status()
        .expect("git init");
    assert!(status.success());
    for args in [
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test User"],
        vec!["add", "."],
        vec!["commit", "-m", "fixture baseline"],
    ] {
        let status = Command::new("git")
            .args(&args)
            .current_dir(&repo)
            .status()
            .expect("git fixture");
        assert!(status.success());
    }

    let runtime = WebRuntime::new_fake(root.path().to_path_buf());
    let state = WebAppState::new(root.path().to_path_buf(), runtime).with_provider_adapter(
        Arc::new(MockSplitProviderAdapter {
            output,
            revision_output: None,
        }),
    );
    let app = build_web_router(state);
    let app = bootstrap_project_repo_issue_and_specs(app, &repo).await;

    (app, root)
}

// 退役留档（T5/REQ-RET-02）：app_with_confirmed_story_and_design_and_revision_output 夹具
// （revision 输出注入）为 legacy full-candidate revision 流测试专用，随消息族退役。

pub(crate) async fn app_with_confirmed_story_and_design_and_streaming_revision_output(
    output: Value,
    revision_output: Value,
) -> (axum::Router, tempfile::TempDir) {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    // REQ-PIB-02：夹具对齐生产不变量（main 分支+初始提交，裸 init 无分支
    // 会令基线解析 fail-closed）。
    // 聚合索引布局规则：每成员至少 1 个可索引源码文件
    // （aggregate_index_member_coverage_failed），空提交不算。
    std::fs::create_dir_all(repo.join("src")).expect("repo src dir");
    std::fs::write(repo.join("src/main.rs"), "fn main() {}\n").expect("repo source");
    let status = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(&repo)
        .status()
        .expect("git init");
    assert!(status.success());
    for args in [
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test User"],
        vec!["add", "."],
        vec!["commit", "-m", "fixture baseline"],
    ] {
        let status = Command::new("git")
            .args(&args)
            .current_dir(&repo)
            .status()
            .expect("git fixture");
        assert!(status.success());
    }

    let runtime = WebRuntime::new_fake(root.path().to_path_buf());
    let mut state = WebAppState::new(root.path().to_path_buf(), runtime).with_provider_adapter(
        Arc::new(MockSplitProviderAdapter {
            output: output.clone(),
            revision_output: Some(revision_output.clone()),
        }),
    );

    let test_controls = cadence_aria::web::test_controls::TestControls::default();
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(QueuedSplitStreamingProvider::new(vec![
            output,
            revision_output,
        ])),
    );
    registry.register(
        ProviderName::Codex,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
        )),
    );
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
        )),
    );
    state.test_controls = test_controls;
    state.provider_registry = Arc::new(registry);

    let app = build_web_router(state);
    let app = bootstrap_project_repo_issue_and_specs(app, &repo).await;

    (app, root)
}
