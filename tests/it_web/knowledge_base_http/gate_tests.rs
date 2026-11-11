// REQ-KB-13（知识库全域聚合初始化前置门）集成测试：门未满足时全部端点
// 422 拒绝、零写副作用；门满足后端点恢复服务。本片段经 `include!` 进入
// `knowledge_base_http.rs`，复用其夹具与 `request_json` 助手。

/// 门未满足（冷启动投影「规则/政策」步非 Completed）时，14 个知识库端点
/// 全部拒绝：422 + `knowledge_base_requires_aggregate_initialization` +
/// 中文可执行文案 + `details.bootstrap_step == "rules_policy"`；且磁盘
/// 零写副作用（文件字节不变、模板/变更包/输入模板端点不落任何新文件、
/// 确认端点不落平台侧确认记录）。
#[tokio::test]
async fn gate_rejects_every_endpoint_until_aggregate_initialization_completed() {
    let fixture = KnowledgeBaseFixture::new().await;
    // 预置既有输入文件：写端点被门拦截时磁盘字节必须原样保留。
    let base_info = fixture.kb_root().join("user-input").join("base-info.md");
    std::fs::create_dir_all(base_info.parent().expect("user-input dir")).expect("seed dir");
    std::fs::write(&base_info, "# 既有内容\n").expect("seed base-info");

    let endpoints: [(Method, &str, Value); 14] = [
        (Method::GET, "/status", json!({})),
        (Method::GET, "/files", json!({})),
        (
            Method::GET,
            "/file?path=user-input/base-info.md",
            json!({}),
        ),
        (
            Method::PUT,
            "/file",
            json!({"path":"user-input/base-info.md","content":"# 覆盖尝试\n"}),
        ),
        (Method::GET, "/user-input", json!({})),
        (
            Method::POST,
            "/user-input/confirm",
            json!({"path":"user-input/base-info.md","digest":"deadbeef"}),
        ),
        (Method::GET, "/change-packages", json!({})),
        (
            Method::GET,
            "/change-packages/lint?path=user-input/updates/CHANGE-x",
            json!({}),
        ),
        (
            Method::POST,
            "/change-packages",
            json!({"identifier":"gate"}),
        ),
        (Method::POST, "/bootstrap-inputs", json!({})),
        (
            Method::POST,
            "/initializations",
            json!({"kind":"bootstrap","idempotency_key":"gate-1"}),
        ),
        (Method::GET, "/initializations/active", json!({})),
        (Method::GET, "/initializations/op_gate_probe_001", json!({})),
        (
            Method::POST,
            "/initializations/op_gate_probe_001/cancel",
            json!({}),
        ),
    ];
    for (method, suffix, body) in endpoints {
        let (status, error) =
            request_json(&fixture.app, method.clone(), &fixture.kb_uri(suffix), body).await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{method} {suffix} must be gated: {error}"
        );
        assert_eq!(
            error["code"], "knowledge_base_requires_aggregate_initialization",
            "{method} {suffix}: {error}"
        );
        let message = error["message"].as_str().expect("message");
        assert!(
            message.contains("聚合初始化"),
            "{method} {suffix} message must be actionable Chinese copy: {error}"
        );
        assert_eq!(
            error["details"]["bootstrap_step"], "rules_policy",
            "{method} {suffix}: {error}"
        );
    }

    // 零写副作用①：PUT /file 被拦截 ⇒ 既有文件字节不变。
    assert_eq!(
        std::fs::read_to_string(&base_info).expect("base-info survives"),
        "# 既有内容\n"
    );
    // 零写副作用②：POST /change-packages 被拦截 ⇒ 不落任何变更包目录。
    assert!(!fixture.kb_root().join("user-input").join("updates").exists());
    // 零写副作用③：POST /bootstrap-inputs 被拦截 ⇒ user-input/ 下只有预置的
    // base-info.md，没有任何模板文件被补齐。
    let mut remaining: Vec<String> = std::fs::read_dir(base_info.parent().expect("user-input"))
        .expect("user-input dir")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    remaining.sort();
    assert_eq!(remaining, vec!["base-info.md".to_string()]);
    // 零写副作用④：POST /user-input/confirm 被拦截 ⇒ 平台侧不落确认记录。
    assert!(!fixture.confirmations_path().exists());
}

/// REQ-KB-13 场景三：聚合初始化完成后知识库端点正常服务，不再返回前置
/// 失败码（status 200 投影 / files 200 / file 读 200）。
#[tokio::test]
async fn gate_opens_endpoints_once_aggregate_initialization_completed() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();

    let (status, body) = request_json(&fixture.app, Method::GET, &fixture.kb_uri("/status"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["exists"], false);

    let (status, body) = request_json(&fixture.app, Method::GET, &fixture.kb_uri("/files"), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, _) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri("/initializations/active"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
