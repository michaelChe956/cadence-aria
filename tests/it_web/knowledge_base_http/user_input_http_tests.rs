// 用户输入面（GET/POST /user-input）与两段式分叉（REQ-KB-14/15）的集成测试：
// 经 `include!` 进入 `knowledge_base_http.rs` 同一模块，复用其夹具与助手。

/// REQ-KB-14 场景一：输入未准备（9 份全缺失）时点【知识库初始化】⇒ 分叉起
/// 起草 operation（kind=user_input、步骤 prepare_user_input），不再因输入
/// 缺失被预检③拒绝，也不直接执行 bootstrap。
#[tokio::test]
async fn create_bootstrap_operation_forks_to_user_input_draft_when_inputs_not_prepared() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    let _home = HomeEnvGuard::acquire(fixture._home.path());

    let (status, body) = request_json(
        &fixture.app,
        Method::POST,
        &fixture.kb_uri("/initializations"),
        json!({"kind":"bootstrap","idempotency_key":"kb-fork-draft-1"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["kind"], "user_input", "{body}");
    let steps = body["steps"].as_array().expect("steps");
    assert!(!steps.is_empty());
    assert_eq!(steps[0]["step_id"], "prepare_user_input", "{body}");
    let operation_id = body["operation_id"]
        .as_str()
        .expect("operation_id")
        .to_string();
    let settled = fixture.wait_operation_settled(&operation_id).await;
    assert!(
        ["completed", "failed", "cancelled"].contains(&settled.as_str()),
        "unexpected terminal status: {settled}"
    );
}

/// REQ-KB-14 场景三：输入已准备但未全部确认 ⇒ 拒绝并列出未确认文件，
/// 不产生任何 operation。`knowledge_base_user_input_not_confirmed` 的 HTTP
/// 状态映射已在 `src/web/error.rs` 登记为 422（前置不满足），此处钉住具体
/// 状态码、稳定码、中文文案与零 operation 语义。
#[tokio::test]
async fn create_bootstrap_operation_rejected_when_user_input_not_confirmed() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    fixture.seed_filled_user_inputs();
    let _home = HomeEnvGuard::acquire(fixture._home.path());

    let (status, body) = request_json(
        &fixture.app,
        Method::POST,
        &fixture.kb_uri("/initializations"),
        json!({"kind":"bootstrap","idempotency_key":"kb-unconfirmed-1"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(
        body["code"], "knowledge_base_user_input_not_confirmed",
        "{body}"
    );
    let message = body["message"].as_str().expect("message");
    assert!(
        message.contains("确认") && message.contains("输入"),
        "message must be actionable Chinese copy: {body}"
    );
    let unconfirmed = body["details"]["unconfirmed"]
        .as_array()
        .expect("unconfirmed files");
    assert_eq!(unconfirmed.len(), 9, "{body}");
    assert!(unconfirmed.iter().all(|path| {
        path.as_str()
            .is_some_and(|path| path.starts_with("user-input/"))
    }));
    assert_eq!(body["details"]["confirmed_count"], 0, "{body}");
    assert_eq!(body["details"]["total_count"], 9, "{body}");

    // 拒绝是请求期产物：不产生 operation。
    let (status, active) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri("/initializations/active"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{active}");
    assert_eq!(active["code"], "knowledge_base_operation_not_found");
}

/// GET /user-input 逐态投影（未填 / 待确认 / 已确认）+ 编辑后确认自动失效
/// （REQ-KB-15 场景三）。
#[tokio::test]
async fn user_input_endpoint_reports_file_states_and_invalidation() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    let _home = HomeEnvGuard::acquire(fixture._home.path());
    // 文件面写入要求父目录在场（与 file_roundtrip 用例同款前置）。
    std::fs::create_dir_all(fixture.kb_root().join("user-input")).expect("create user-input");
    let files_uri = fixture.kb_uri("/user-input");

    // 初始：9 份全部未填（filled=false、confirmed=false、pending=false）。
    let (status, body) = request_json(&fixture.app, Method::GET, &files_uri, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total_count"], 9, "{body}");
    assert_eq!(body["confirmed_count"], 0, "{body}");
    let files = body["files"].as_array().expect("files");
    assert_eq!(files.len(), 9, "{body}");
    assert!(
        files.iter().all(|file| file["filled"] == false
            && file["confirmed"] == false
            && file["pending"] == false),
        "{body}"
    );

    // 写入一份与模板逐字不同的内容 ⇒ filled + pending（待确认）。
    let (status, written) = request_json(
        &fixture.app,
        Method::PUT,
        &fixture.kb_uri("/file"),
        json!({"path":"user-input/base-info.md","content":"# 真实填写\n"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{written}");
    let (_, body) = request_json(&fixture.app, Method::GET, &files_uri, json!({})).await;
    let base_info = body["files"]
        .as_array()
        .expect("files")
        .iter()
        .find(|file| file["path"] == "user-input/base-info.md")
        .expect("base-info listed")
        .clone();
    assert_eq!(base_info["filled"], true, "{body}");
    assert_eq!(base_info["pending"], true, "{body}");
    assert_eq!(base_info["confirmed"], false, "{body}");
    assert_eq!(body["confirmed_count"], 0, "{body}");

    // 取服务端摘要 → 确认该份 ⇒ confirmed（pending 归零）。
    let (status, read) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri("/file?path=user-input/base-info.md"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read}");
    let digest = read["digest"].as_str().expect("digest").to_string();
    let (status, confirmed) = request_json(
        &fixture.app,
        Method::POST,
        &fixture.kb_uri("/user-input/confirm"),
        json!({"path":"user-input/base-info.md","digest":digest}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{confirmed}");
    let (_, body) = request_json(&fixture.app, Method::GET, &files_uri, json!({})).await;
    let base_info = body["files"]
        .as_array()
        .expect("files")
        .iter()
        .find(|file| file["path"] == "user-input/base-info.md")
        .expect("base-info listed")
        .clone();
    assert_eq!(base_info["confirmed"], true, "{body}");
    assert_eq!(base_info["pending"], false, "{body}");
    assert_eq!(body["confirmed_count"], 1, "{body}");

    // 确认记录落平台侧 .aria（per-LC），user-input/ 目录内没有任何新增文件。
    assert!(fixture.confirmations_path().exists());
    let mut names: Vec<String> = std::fs::read_dir(fixture.kb_root().join("user-input"))
        .expect("user-input dir")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["base-info.md".to_string()], "{names:?}");

    // 编辑已确认的文件 ⇒ 确认自动失效（重新待确认），初始化入口保持不可用。
    let (status, edited) = request_json(
        &fixture.app,
        Method::PUT,
        &fixture.kb_uri("/file"),
        json!({"path":"user-input/base-info.md","content":"# 已修改\n"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    let (_, body) = request_json(&fixture.app, Method::GET, &files_uri, json!({})).await;
    let base_info = body["files"]
        .as_array()
        .expect("files")
        .iter()
        .find(|file| file["path"] == "user-input/base-info.md")
        .expect("base-info listed")
        .clone();
    assert_eq!(base_info["filled"], true, "{body}");
    assert_eq!(base_info["confirmed"], false, "{body}");
    assert_eq!(base_info["pending"], true, "{body}");
    assert_eq!(body["confirmed_count"], 0, "{body}");
}

/// REQ-KB-15 场景二：过期摘要确认被拒（stale），不写入确认记录。
#[tokio::test]
async fn user_input_confirm_rejects_stale_digest_without_recording() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    fixture.seed_filled_user_inputs();
    let _home = HomeEnvGuard::acquire(fixture._home.path());

    let (status, body) = request_json(
        &fixture.app,
        Method::POST,
        &fixture.kb_uri("/user-input/confirm"),
        json!({"path":"user-input/base-info.md","digest":"deadbeef"}),
    )
    .await;
    // stale 码的 HTTP 状态映射已在 `src/web/error.rs` 登记为 409（客户端拿
    // 过期内容来确认），此处钉死状态码与中文可执行文案。
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["code"], "knowledge_base_user_input_confirmation_stale",
        "{body}"
    );
    let message = body["message"].as_str().expect("message");
    assert!(
        message.contains("确认已过期") && message.contains("重新确认"),
        "message must be actionable Chinese copy: {body}"
    );
    assert_eq!(body["details"]["path"], "user-input/base-info.md", "{body}");

    // 拒绝不落任何确认记录：平台侧无记录文件，状态仍全部未确认。
    assert!(!fixture.confirmations_path().exists());
    let (status, summary) = request_json(
        &fixture.app,
        Method::GET,
        &fixture.kb_uri("/user-input"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{summary}");
    assert_eq!(summary["confirmed_count"], 0, "{summary}");
}

/// REQ-KB-15 场景四（批量确认）+ REQ-KB-14 场景四（全部确认后执行
/// bootstrap）：批量确认 9 份 ⇒ 再点【知识库初始化】直接起 bootstrap。
#[tokio::test]
async fn user_input_confirm_batch_then_bootstrap_runs() {
    let fixture = KnowledgeBaseFixture::new().await;
    fixture.seed_aggregate_initialization();
    fixture.seed_filled_user_inputs();
    let _home = HomeEnvGuard::acquire(fixture._home.path());
    fixture.confirm_all_user_inputs().await;

    let (status, body) = request_json(
        &fixture.app,
        Method::POST,
        &fixture.kb_uri("/initializations"),
        json!({"kind":"bootstrap","idempotency_key":"kb-confirmed-bootstrap-1"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["kind"], "bootstrap", "{body}");
    let steps = body["steps"].as_array().expect("steps");
    assert_eq!(steps[0]["step_id"], "bootstrap", "{body}");
    let operation_id = body["operation_id"]
        .as_str()
        .expect("operation_id")
        .to_string();
    fixture.wait_operation_settled(&operation_id).await;
}

