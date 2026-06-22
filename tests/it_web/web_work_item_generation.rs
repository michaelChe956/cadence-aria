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
                "required_handoff_from": [],
                "require_execution_plan_confirm": false
            },
            {
                "title": "实现前端会话过期提示",
                "kind": "frontend",
                "sequence_hint": 20,
                "depends_on": [0],
                "exclusive_write_scopes": ["web/src/session/**"],
                "forbidden_write_scopes": ["src/product/**"],
                "required_handoff_from": [],
                "require_execution_plan_confirm": false
            },
            {
                "title": "集成测试：会话过期端到端",
                "kind": "integration",
                "sequence_hint": 30,
                "depends_on": [1],
                "exclusive_write_scopes": ["tests/session/**"],
                "forbidden_write_scopes": [],
                "required_handoff_from": [],
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

pub(crate) fn valid_outline_output() -> Value {
    json!({
        "outline": {
            "id": "outline_0001",
            "project_id": "project_0001",
            "issue_id": "issue_0001",
            "source_story_spec_ids": ["story_spec_0001"],
            "source_design_spec_ids": ["design_spec_0001"],
            "strategy_summary": "先实现后端会话 API，再补前端过期提示，最后补集成测试。",
            "work_item_outlines": [
                {
                    "outline_id": "outline_backend_session",
                    "title": "实现后端登录会话 API",
                    "kind": "backend",
                    "goal": "提供登录会话过期检测与刷新相关 API。",
                    "scope": ["src/product/session.rs", "src/web/session_handlers.rs"],
                    "non_goals": ["不实现前端 UI"],
                    "source_story_spec_ids": ["story_spec_0001"],
                    "source_design_spec_ids": ["design_spec_0001"],
                    "exclusive_write_scopes": ["src/product/session.rs", "src/web/session_handlers.rs"],
                    "forbidden_write_scopes": ["web/**"],
                    "depends_on": [],
                    "verification_intent": ["cargo test --locked --lib session"],
                    "handoff_notes": "输出会话状态 DTO 与错误语义。"
                },
                {
                    "outline_id": "outline_frontend_expiry",
                    "title": "实现前端会话过期提示",
                    "kind": "frontend",
                    "goal": "在前端展示会话过期提示并触发重新登录入口。",
                    "scope": ["web/src/session/**"],
                    "non_goals": ["不修改后端 API"],
                    "source_story_spec_ids": ["story_spec_0001"],
                    "source_design_spec_ids": ["design_spec_0001"],
                    "exclusive_write_scopes": ["web/src/session/**"],
                    "forbidden_write_scopes": ["src/product/**"],
                    "depends_on": ["outline_backend_session"],
                    "verification_intent": ["pnpm -C web test"],
                    "handoff_notes": "消费后端会话状态 DTO。"
                },
                {
                    "outline_id": "outline_integration_session",
                    "title": "集成测试：会话过期端到端",
                    "kind": "integration",
                    "goal": "覆盖会话过期到前端提示的贯通路径。",
                    "scope": ["tests/session/**"],
                    "non_goals": ["不新增业务功能"],
                    "source_story_spec_ids": ["story_spec_0001"],
                    "source_design_spec_ids": ["design_spec_0001"],
                    "exclusive_write_scopes": ["tests/session/**"],
                    "forbidden_write_scopes": [],
                    "depends_on": ["outline_frontend_expiry"],
                    "verification_intent": ["cargo test --locked --test it_web session"],
                    "handoff_notes": "验证前后端协议一致性。"
                }
            ],
            "dependency_graph": [
                {
                    "from_outline_id": "outline_backend_session",
                    "to_outline_id": "outline_frontend_expiry"
                },
                {
                    "from_outline_id": "outline_frontend_expiry",
                    "to_outline_id": "outline_integration_session"
                }
            ],
            "risks": ["前后端错误码需要保持一致"],
            "handoff_strategy": "后端先定义稳定 DTO，前端和集成测试逐步消费。",
            "status": "draft"
        },
        "context_blockers": []
    })
}

pub(crate) fn context_blocker_outline_output() -> Value {
    json!({
        "context_blockers": [
            {
                "code": "missing_module_boundary",
                "message": "无法判断会话 API 应落在 product 还是 web 层。",
                "needed_context": ["请补充模块边界", "请说明测试策略"]
            }
        ]
    })
}

pub(crate) fn invalid_outline_output_duplicate_ids() -> Value {
    let mut output = valid_outline_output();
    let outlines = output["outline"]["work_item_outlines"]
        .as_array_mut()
        .expect("outline array");
    outlines[1]["outline_id"] = json!("outline_backend_session");
    output
}

pub(crate) fn valid_revision_redo_output() -> Value {
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
                "title": "实现后端登录会话 API（重做）",
                "kind": "backend",
                "sequence_hint": 10,
                "depends_on": [],
                "exclusive_write_scopes": ["src/product/session.rs"],
                "forbidden_write_scopes": ["web/**"],
                "required_handoff_from": [],
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
                "required_handoff_from": [],
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

    pub(crate) fn new_recording(
        outputs: Vec<Value>,
        captured_prompts: Arc<Mutex<Vec<String>>>,
    ) -> Self {
        Self {
            outputs: Arc::new(Mutex::new(VecDeque::from(outputs))),
            captured_prompts: Some(captured_prompts),
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
        let output = self
            .outputs
            .lock()
            .expect("queued split outputs lock")
            .pop_front()
            .unwrap_or_else(valid_split_output);
        let full_output = format!(
            "Fake Work Item Plan streaming draft\n\n\
             <ARIA_STRUCTURED_OUTPUT>{}</ARIA_STRUCTURED_OUTPUT>",
            output
        );
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);

        tokio::spawn(async move {
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
                .send(ProviderEvent::Completed {
                    full_output,
                    provider_session_id: None,
                })
                .await;
        });

        Ok(ProviderSession {
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
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Lifecycle","description":null}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo}),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({"title":"登录会话过期","description":"描述","repository_id":"repository_0001"}),
    )
    .await;

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/story-specs:generate",
        json!({
            "title":"登录会话过期提示",
            "author_provider":"fake",
            "reviewer_provider":"codex",
            "review_rounds":3,
            "superpowers_enabled":false,
            "openspec_enabled":true
        }),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0001/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title":"会话过期后端设计",
            "story_spec_ids":["story_spec_0001"],
            "author_provider":"codex",
            "reviewer_provider":"claude_code",
            "review_rounds":2,
            "superpowers_enabled":true,
            "openspec_enabled":true
        }),
    )
    .await;
    request_json(
        app.clone(),
        Method::POST,
        "/api/workspace-sessions/workspace_session_0002/confirm",
        json!({"confirmed_by":"human"}),
    )
    .await;

    app
}

pub(crate) async fn app_with_confirmed_story_and_design(
    output: Value,
) -> (axum::Router, tempfile::TempDir) {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(&repo)
        .status()
        .expect("git init");
    assert!(status.success());

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

pub(crate) async fn app_with_confirmed_story_and_design_and_revision_output(
    output: Value,
    revision_output: Value,
) -> (axum::Router, tempfile::TempDir) {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(&repo)
        .status()
        .expect("git init");
    assert!(status.success());

    let runtime = WebRuntime::new_fake(root.path().to_path_buf());
    let state = WebAppState::new(root.path().to_path_buf(), runtime).with_provider_adapter(
        Arc::new(MockSplitProviderAdapter {
            output,
            revision_output: Some(revision_output),
        }),
    );
    let app = build_web_router(state);
    let app = bootstrap_project_repo_issue_and_specs(app, &repo).await;

    (app, root)
}

pub(crate) async fn app_with_confirmed_story_and_design_and_streaming_revision_output(
    output: Value,
    revision_output: Value,
) -> (axum::Router, tempfile::TempDir) {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(&repo)
        .status()
        .expect("git init");
    assert!(status.success());

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

pub(crate) async fn app_with_confirmed_story_and_design_and_streaming_outputs(
    outputs: Vec<Value>,
) -> (axum::Router, tempfile::TempDir, Arc<Mutex<Vec<String>>>) {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(&repo)
        .status()
        .expect("git init");
    assert!(status.success());

    let runtime = WebRuntime::new_fake(root.path().to_path_buf());
    let mut state = WebAppState::new(root.path().to_path_buf(), runtime).with_provider_adapter(
        Arc::new(MockSplitProviderAdapter {
            output: outputs.first().cloned().unwrap_or_else(valid_split_output),
            revision_output: None,
        }),
    );

    let captured_prompts = Arc::new(Mutex::new(Vec::new()));
    let test_controls = cadence_aria::web::test_controls::TestControls::default();
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Fake,
        Arc::new(QueuedSplitStreamingProvider::new_recording(
            outputs,
            captured_prompts.clone(),
        )),
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

    (app, root, captured_prompts)
}

/// 与 `app_with_confirmed_story_and_design` 相同，但额外把 codex/claude_code 也注册为
/// TestControlledFakeStreamingProvider，以便在 review 阶段通过 review fixture 注入固定 verdict。
pub(crate) async fn app_with_confirmed_story_and_design_and_test_providers(
    output: Value,
) -> (axum::Router, tempfile::TempDir) {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(&repo)
        .status()
        .expect("git init");
    assert!(status.success());

    let runtime = WebRuntime::new_fake(root.path().to_path_buf());
    let mut state = WebAppState::new(root.path().to_path_buf(), runtime).with_provider_adapter(
        Arc::new(MockSplitProviderAdapter {
            output,
            revision_output: None,
        }),
    );

    let mut registry = ProviderRegistry::new();
    let test_controls = cadence_aria::web::test_controls::TestControls::default();
    registry.register(
        ProviderName::Fake,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
        )),
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

/// 与 `app_with_confirmed_story_and_design_and_test_providers` 相同，但额外提供 revision 输出，
/// 用于验证包含 review / revision 的完整 WorkItemPlan 流程。
pub(crate) async fn app_with_confirmed_story_and_design_revision_and_test_providers(
    output: Value,
    revision_output: Value,
) -> (axum::Router, tempfile::TempDir) {
    unsafe {
        std::env::set_var("ARIA_E2E_TEST_CONTROLS", "1");
    }
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    let status = Command::new("git")
        .args(["init"])
        .current_dir(&repo)
        .status()
        .expect("git init");
    assert!(status.success());

    let runtime = WebRuntime::new_fake(root.path().to_path_buf());
    let mut state = WebAppState::new(root.path().to_path_buf(), runtime).with_provider_adapter(
        Arc::new(MockSplitProviderAdapter {
            output,
            revision_output: Some(revision_output),
        }),
    );

    let mut registry = ProviderRegistry::new();
    let test_controls = cadence_aria::web::test_controls::TestControls::default();
    registry.register(
        ProviderName::Fake,
        Arc::new(TestControlledFakeStreamingProvider::new(
            test_controls.clone(),
        )),
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

#[tokio::test]
async fn prepare_work_item_plan_creates_draft_plan_and_session_without_generating() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "爬楼梯问题 Work Item Plan",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "author_provider": "fake",
            "reviewer_provider": "codex",
            "review_rounds": 1,
            "superpowers_enabled": true,
            "openspec_enabled": true,
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["work_item_plan"]["status"], "draft");
    assert_eq!(
        response["work_item_plan"]["options"]["include_integration_tests"],
        true
    );
    assert_eq!(
        response["work_item_plan"]["options"]["include_e2e_tests"],
        false
    );
    assert_eq!(
        response["work_item_plan"]["options"]["force_frontend_backend_split"],
        true
    );
    assert_eq!(
        response["work_item_plan"]["options"]["require_execution_plan_confirm"],
        false
    );
    assert!(
        response["work_item_plan"]["work_item_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        response["work_item_plan"]["verification_plan_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        response["work_item_plan"]["dependency_graph"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        response["workspace_session"]["workspace_type"],
        "work_item_plan"
    );
    assert_eq!(
        response["workspace_session"]["entity_id"],
        response["work_item_plan"]["id"]
    );

    let lifecycle = cadence_aria::product::lifecycle_store::LifecycleStore::new(
        cadence_aria::product::app_paths::ProductAppPaths::new(_repo.path().join(".aria")),
    );
    assert!(
        lifecycle
            .list_work_items("project_0001", "issue_0001")
            .unwrap()
            .is_empty()
    );

    let first_message = &response["workspace_session"]["messages"][0]["content"];
    assert!(
        first_message
            .as_str()
            .unwrap()
            .contains("候选 work item plan 生成器")
    );
}
