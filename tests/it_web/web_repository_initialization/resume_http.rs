// C5 Task 6（REQ-INIT-C5-RESUME）：project waiting-items GET 与 resume POST
// 的 HTTP 消费入口——Claude 网关不可用停等、恢复后继续、重放与前置校验。

use std::sync::atomic::{AtomicBool, Ordering};

/// 可翻转健康源：关闭时 Claude Code provider 不可用（503 语义），翻转后
/// 恢复可用——驱动“网关恢复后继续”的完整真实步骤链。
struct FlippableHealth {
    down: AtomicBool,
}

impl ProviderHealthSource for FlippableHealth {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
        let mut snapshot = HealthySource::new().snapshot().as_ref().clone();
        if self.down.load(Ordering::SeqCst) {
            snapshot.providers[0].available = false;
            snapshot.providers[0].reason = Some("claude gateway unavailable".to_string());
        }
        Arc::new(snapshot)
    }

    fn degraded(&self) -> bool {
        false
    }
}

fn resume_integration_state(
    root: &Path,
    provider: Arc<ScriptedClaude>,
    health: Arc<FlippableHealth>,
) -> WebAppState {
    let gate = Arc::new(ProviderAvailabilityGate::new(health));
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    let registry = Arc::new(registry);
    let runner: Arc<dyn BoundedCommandRunner> = Arc::new(TokioBoundedCommandRunner);
    let dependencies = RepositoryRegistrationDependencies::builder(
        ProductAppPaths::new(root.join(".aria")),
        root.join("home"),
        runner,
        gate,
        registry,
    )
    .with_cadence_skills(Arc::new(FixtureCadence {
        root: root.join("home"),
    }))
    .with_host_readiness(Arc::new(|| Ok(())))
    .with_clock(Arc::new(|| "2026-09-30T03:00:00Z".to_string()))
    .with_timeouts(Duration::from_secs(2), Duration::from_secs(5))
    .build()
    .expect("resume integration dependencies");
    WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()))
        .with_repository_registration_dependencies(dependencies)
}

async fn list_waiting_items(app: axum::Router, project_id: &str) -> (StatusCode, Value) {
    request_json(
        app,
        Method::GET,
        &format!("/api/projects/{project_id}/repository-initializations/waiting-items"),
        json!({}),
    )
    .await
}

async fn post_resume(
    app: axum::Router,
    project_id: &str,
    operation_id: &str,
    command_id: &str,
) -> (StatusCode, Value) {
    request_json(
        app,
        Method::POST,
        &format!("/api/projects/{project_id}/repository-initializations/{operation_id}/resume"),
        json!({"command_id": command_id}),
    )
    .await
}

#[tokio::test]
async fn repository_initialization_resume_http_roundtrip_recovers_after_gateway_outage() {
    let root = tempdir().unwrap();
    let repo = git_repo();
    let health = Arc::new(FlippableHealth {
        down: AtomicBool::new(true),
    });
    let provider = Arc::new(ScriptedClaude::new(vec![TurnScript::Complete; 4]));
    let app = build_web_router(resume_integration_state(root.path(), provider, health.clone()));
    create_project(app.clone()).await;

    // 网关不可用：初始化在 pre_check 以 provider_unavailable 失败停等，
    // 不创建 Repository 记录。
    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"].as_str().unwrap().to_string();
    let failed = get_operation_until_terminal(&app, "project_0001", &operation_id).await;
    assert_eq!(failed["status"], "failed");
    // project waiting-items：恰一条，含 operation_id、结构化诊断与动作，
    // issue_id 缺省。
    let (status, items) = list_waiting_items(app.clone(), "project_0001").await;
    assert_eq!(status, StatusCode::OK, "{items}");
    let items = items.as_array().expect("array body");
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["kind"], "repository_initialization_failed");
    assert_eq!(items[0]["operation_id"], operation_id);
    assert_eq!(items[0]["project_id"], "project_0001");
    assert_eq!(
        items[0]["id"],
        format!("c1:project:project_0001:repository_init:{operation_id}")
    );
    assert_eq!(items[0]["diagnostics"]["reason_code"], "provider_unavailable");
    assert_eq!(items[0]["diagnostics"]["failed_step"], "pre_check");
    assert_eq!(items[0]["actions"][0], "resume_repository_initialization");
    assert!(items[0].get("issue_id").is_none());

    // 恢复后继续：网关可用，resume 以冻结 input 走完整 Claude 步骤链至
    // completed。
    health.down.store(false, Ordering::SeqCst);
    let command_id = format!("cmd-repo-init-resume-{operation_id}");
    let (status, resumed) =
        post_resume(app.clone(), "project_0001", &operation_id, &command_id).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{resumed}");
    let successor_id = resumed["operation_id"].as_str().unwrap().to_string();
    assert_ne!(successor_id, operation_id);
    let completed = get_operation_until_terminal(&app, "project_0001", &successor_id).await;
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["result"]["repository"]["name"], "Repo");

    // 原 Failed 记录只读可查；等待项稳定消隐。
    let (status, original) = get_operation(app.clone(), "project_0001", &operation_id).await;
    assert_eq!(status, StatusCode::OK, "{original}");
    assert_eq!(original["status"], "failed");
    let (status, items) = list_waiting_items(app.clone(), "project_0001").await;
    assert_eq!(status, StatusCode::OK, "{items}");
    assert_eq!(items.as_array().expect("array body").len(), 0, "{items:?}");

    // 同 command 重放：只读返回同一 successor（completed），不重复执行。
    let (status, replay) =
        post_resume(app.clone(), "project_0001", &operation_id, &command_id).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["operation_id"], successor_id);
    assert_eq!(replay["status"], "completed");

    // 恢复前置校验：非 Failed 终态（completed successor）→ 422。
    let (status, error) = post_resume(
        app.clone(),
        "project_0001",
        &successor_id,
        "cmd-repo-init-resume-next",
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    assert_eq!(error["code"], "repository_initialization_resume_not_failed");

    // command_id 为空 → 422；未知 operation → 404。
    let (status, error) = post_resume(app.clone(), "project_0001", &operation_id, "").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
    assert_eq!(error["code"], "repository_initialization_resume_invalid_command");
    let (status, error) = post_resume(
        app.clone(),
        "project_0001",
        "repository_initialization_missing",
        "cmd-1",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{error}");
    assert_eq!(
        error["code"],
        "repository_initialization_operation_not_found"
    );

    // 空 project：200 空数组（无目录不报错）。
    let (status, empty) = list_waiting_items(app.clone(), "project_9999").await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(empty.as_array().expect("array body").len(), 0);
}
