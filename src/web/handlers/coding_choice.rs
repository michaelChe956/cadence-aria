//! P0 1.3（REQ-WIGA-05）：无 driver 的 coding choice REST——与 coding WS 共用
//! `CodingRunRegistry` claim 门面。状态映射与 Task 8 workspace 侧一致：
//! Delivered→200，Submitting/Resolving→202，Unknown→404，Conflict→409，
//! Expired/Rejected→410；deadline 到期仅返回最新状态，不启动新 command。
//! P0 不触发 coding 首启：只作答已存在的 open choice gate。

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use crate::cross_cutting::choice_delivery::ChoiceReplyState;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::web::choice_reply::{ChoiceReplyStatus, ChoiceResponseRequest};
use crate::web::error::{ApiError, ApiResult};
use crate::web::handlers::support::{product_app_paths, product_store_api_error};
use crate::web::state::{CodingAttemptRunKey, WebAppState};

/// HTTP 等待 provider 等待者接收（Delivered）的预算；到期返回当前状态。
const CODING_CHOICE_RECEIPT_HTTP_DEADLINE: Duration = Duration::from_millis(350);

pub async fn post_coding_choice_response(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, attempt_id, choice_id)): Path<(String, String, String, String)>,
    Json(request): Json<ChoiceResponseRequest>,
) -> ApiResult<(StatusCode, Json<ChoiceReplyStatus>)> {
    let attempt_key = resolve_attempt_key(&state, &project_id, &issue_id, &attempt_id).await?;
    let (_, won) = state
        .coding_runs
        .claim_choice(&attempt_key, &choice_id, &request)
        .map_err(coding_choice_api_error)?;
    if won {
        state
            .coding_runs
            .submit_claimed_choice(&attempt_key, &choice_id, &request)
            .await
            .map_err(coding_choice_api_error)?;
    }
    let current = state
        .coding_runs
        .wait_choice_receipt(
            &attempt_key,
            &choice_id,
            &request.command_id,
            CODING_CHOICE_RECEIPT_HTTP_DEADLINE,
        )
        .await
        .map_err(coding_choice_api_error)?;
    let code = match current.state {
        ChoiceReplyState::Delivered => StatusCode::OK,
        ChoiceReplyState::Submitting | ChoiceReplyState::Resolving => StatusCode::ACCEPTED,
        ChoiceReplyState::Rejected | ChoiceReplyState::Expired => StatusCode::GONE,
    };
    Ok((code, Json(current)))
}

pub async fn get_coding_choice_response_status(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, attempt_id, choice_id, command_id)): Path<(
        String,
        String,
        String,
        String,
        String,
    )>,
) -> ApiResult<Json<ChoiceReplyStatus>> {
    let attempt_key = resolve_attempt_key(&state, &project_id, &issue_id, &attempt_id).await?;
    let status = state
        .coding_runs
        .choice_status(&attempt_key, &choice_id, &command_id)
        .map_err(coding_choice_api_error)?;
    Ok(Json(status))
}

/// 先验 attempt 作用域（404 语义），再取 run key——不创建 run、不抢 lease。
async fn resolve_attempt_key(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> ApiResult<CodingAttemptRunKey> {
    let attempt = CodingAttemptStore::new(product_app_paths(state))
        .get_attempt(project_id, issue_id, attempt_id)
        .map_err(product_store_api_error)?;
    Ok(CodingAttemptRunKey::from_attempt(&attempt))
}

fn coding_choice_api_error(error: crate::web::workspace_session::ChoiceReplyError) -> ApiError {
    match error {
        crate::web::workspace_session::ChoiceReplyError::Unknown => ApiError::runtime(
            "coding_choice_unknown",
            "choice response command is unknown for this attempt",
            serde_json::Value::Null,
        ),
        crate::web::workspace_session::ChoiceReplyError::Conflict => ApiError::runtime(
            "coding_choice_conflict",
            "choice response conflicts with an in-flight command",
            serde_json::Value::Null,
        ),
        crate::web::workspace_session::ChoiceReplyError::Expired => ApiError::runtime(
            "coding_choice_expired",
            "choice response targets a finished or missing coding run",
            serde_json::Value::Null,
        ),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    use super::*;
    use crate::cross_cutting::streaming_provider::ChoiceAnswerData;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::{
        CodingAttemptStore, CreateChoiceGateInput, CreateCodingAttemptInput,
    };
    use crate::product::coding_models::{
        CodingChoiceOption, CodingChoiceQuestion, CodingChoiceGateStatus, CodingExecutionStage,
        CodingProviderRole,
    };
    use crate::product::coding_workspace_runner::CodingRunnerCommand;
    use crate::product::models::ProviderName;
    use crate::web::app::build_web_router;
    use crate::web::runtime::WebRuntime;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    struct CodingChoiceHttpFixture {
        router: axum::Router,
        state: WebAppState,
        #[allow(dead_code)]
        store: CodingAttemptStore,
        attempt_key: CodingAttemptRunKey,
        attempt_id: String,
        command_rx: mpsc::Receiver<CodingRunnerCommand>,
        run_id: u64,
        incarnation: String,
    }

    async fn coding_choice_http_fixture() -> CodingChoiceHttpFixture {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().to_path_buf();
        static ROOTS: std::sync::Mutex<Vec<tempfile::TempDir>> = std::sync::Mutex::new(Vec::new());
        ROOTS.lock().unwrap().push(root);

        let state = WebAppState::new(root_path.clone(), WebRuntime::new_fake(root_path.clone()));
        let store = CodingAttemptStore::new(ProductAppPaths::new(root_path.join(".aria")));
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/work-items/work_item_0001/attempt-choice".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Fake,
                    reviewer: Some(ProviderName::Fake),
                    review_rounds: 1,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 1,
            })
            .unwrap();
        let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
        let (command_tx, command_rx) = mpsc::channel(8);
        let registration = state
            .coding_runs
            .insert_cancellable(&attempt_key, command_tx)
            .unwrap();
        let incarnation = state
            .coding_runs
            .active_run_incarnation(&attempt_key)
            .unwrap();
        // durable 多问题 gate（两题）：REST 作答对象。
        store
            .create_choice_gate(
                &attempt,
                CreateChoiceGateInput {
                    attempt_id: attempt.id.clone(),
                    choice_id: "choice-http-1".to_string(),
                    stage: CodingExecutionStage::Coding,
                    node_id: None,
                    role: CodingProviderRole::Coder,
                    provider: ProviderName::Fake,
                    source: "provider".to_string(),
                    prompt: "拆分方案确认".to_string(),
                    options: vec![
                        choice_option("a", "方案 A"),
                        choice_option("b", "方案 B"),
                    ],
                    allow_multiple: false,
                    allow_free_text: false,
                    questions: vec![
                        CodingChoiceQuestion {
                            id: "q-1".to_string(),
                            prompt: "是否包含集成测试".to_string(),
                            options: vec![
                                choice_option("yes", "包含"),
                                choice_option("no", "不包含"),
                            ],
                            allow_multiple: false,
                            allow_free_text: false,
                        },
                        CodingChoiceQuestion {
                            id: "q-2".to_string(),
                            prompt: "评审轮数".to_string(),
                            options: vec![
                                choice_option("one", "一轮"),
                                choice_option("two", "两轮"),
                            ],
                            allow_multiple: false,
                            allow_free_text: true,
                        },
                    ],
                },
            )
            .unwrap();
        let attempt_id = attempt.id.clone();
        CodingChoiceHttpFixture {
            router: build_web_router(state.clone()),
            state,
            store,
            attempt_key,
            attempt_id,
            command_rx,
            run_id: registration.run_id(),
            incarnation,
        }
    }

    fn choice_option(id: &str, label: &str) -> CodingChoiceOption {
        CodingChoiceOption {
            id: id.to_string(),
            label: label.to_string(),
            description: None,
        }
    }

    fn request_body(command_id: &str, incarnation: &str) -> serde_json::Value {
        serde_json::json!({
            "command_id": command_id,
            "expected_run_id": incarnation,
            "answers": [
                { "question_id": "q-1", "selected_option_ids": ["yes"], "free_text": null },
                { "question_id": "q-2", "selected_option_ids": ["one"], "free_text": "按一轮即可" },
            ],
        })
    }

    fn answers_payload() -> Vec<ChoiceAnswerData> {
        vec![
            ChoiceAnswerData {
                question_id: "q-1".to_string(),
                selected_option_ids: vec!["yes".to_string()],
                free_text: None,
            },
            ChoiceAnswerData {
                question_id: "q-2".to_string(),
                selected_option_ids: vec!["one".to_string()],
                free_text: Some("按一轮即可".to_string()),
            },
        ]
    }

    async fn post_choice(
        router: &axum::Router,
        attempt_id: &str,
        choice_id: &str,
        body: &serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let uri = format!(
            "/api/projects/project_0001/issues/issue_0001/coding-attempts/{attempt_id}/choices/{choice_id}/response"
        );
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    async fn get_status(
        router: &axum::Router,
        attempt_id: &str,
        choice_id: &str,
        command_id: &str,
    ) -> (StatusCode, serde_json::Value) {
        let uri = format!(
            "/api/projects/project_0001/issues/issue_0001/coding-attempts/{attempt_id}/choices/{choice_id}/responses/{command_id}"
        );
        let response = router
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    /// 受控 provider 等待者：领取 runner command 后按栅栏暂停，释放时 Delivered。
    /// 同时捕获最近一条命令的完整 answers 供断言。
    #[allow(clippy::type_complexity)]
    fn paused_coding_runner(
        mut command_rx: mpsc::Receiver<CodingRunnerCommand>,
    ) -> (
        Arc<tokio::sync::Notify>,
        Arc<std::sync::atomic::AtomicUsize>,
        Arc<std::sync::Mutex<Option<Vec<ChoiceAnswerData>>>>,
    ) {
        let release = Arc::new(tokio::sync::Notify::new());
        let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let last_answers = Arc::new(std::sync::Mutex::new(None));
        let counter = Arc::clone(&seen);
        let answers_slot = Arc::clone(&last_answers);
        let release_handle = Arc::clone(&release);
        tokio::spawn(async move {
            while let Some(command) = command_rx.recv().await {
                if let CodingRunnerCommand::ChoiceResponse {
                    id,
                    answers,
                    receipt: Some(receipt),
                    ..
                } = command
                {
                    assert_eq!(id, "choice-http-1");
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    *answers_slot.lock().unwrap() = Some(answers);
                    receipt.mark_resolving();
                    release_handle.notified().await;
                    receipt.deliver();
                }
            }
        });
        (release, seen, last_answers)
    }

    #[tokio::test]
    async fn coding_choice_reply_http_202_then_retry_200_with_full_answers() {
        let fixture = coding_choice_http_fixture().await;
        let (release, seen, last_answers) = paused_coding_runner(fixture.command_rx);

        let (status, body) = post_choice(
            &fixture.router,
            fixture.attempt_id.as_str(),
            "choice-http-1",
            &request_body("cmd-one", &fixture.incarnation),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "body: {body}");
        assert_eq!(body["state"], "resolving");
        assert_eq!(body["command_id"], "cmd-one");
        assert_eq!(
            seen.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "mpsc 入队 ≠ Delivered：等待者未解析时只能 202"
        );
        // runner command 的完整 answers 原样透传（两题、含自由文本）。
        assert_eq!(
            last_answers.lock().unwrap().as_ref().unwrap(),
            &answers_payload()
        );

        // 释放等待者 → Delivered；同 command retry 200 且不二发。
        release.notify_one();
        let (retry_status, retry_body) = post_choice(
            &fixture.router,
            fixture.attempt_id.as_str(),
            "choice-http-1",
            &request_body("cmd-one", &fixture.incarnation),
        )
        .await;
        assert_eq!(retry_status, StatusCode::OK, "body: {retry_body}");
        assert_eq!(retry_body["state"], "delivered");
        assert_eq!(
            seen.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "同 command 同 payload 不得二发"
        );

        // durable gate resolve 由真实 provider_stream 在 receipt Delivered 后执行
        // （answers 落盘断言见 coding_attempt_store gate 测试）；REST 假等待者
        // fixture 不复刻 engine 循环，此处不伪造该断言。

        // GET 状态查询 Delivered → 200。
        let (get_code, get_body) = get_status(&fixture.router, fixture.attempt_id.as_str(), "choice-http-1", "cmd-one").await;
        assert_eq!(get_code, StatusCode::OK);
        assert_eq!(get_body["state"], "delivered");
    }

    #[tokio::test]
    async fn coding_choice_reply_http_maps_unknown_conflict_expired() {
        let fixture = coding_choice_http_fixture().await;
        let (_release, _seen, _answers) = paused_coding_runner(fixture.command_rx);

        // unknown attempt → 404（durable 作用域校验）。
        let uri = "/api/projects/project_0001/issues/issue_0001/coding-attempts/attempt_4242/choices/any/response";
        let response = fixture
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(request_body("cmd-x", &fixture.incarnation).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // unknown command 查询 → 404。
        let (status, _) = get_status(&fixture.router, fixture.attempt_id.as_str(), "choice-http-1", "cmd-unknown").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // 首个 claim 202；同 command 异 payload → 409；另一 command → 409。
        let (status, _) = post_choice(
            &fixture.router,
            fixture.attempt_id.as_str(),
            "choice-http-1",
            &request_body("cmd-first", &fixture.incarnation),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let mut divergent = request_body("cmd-first", &fixture.incarnation);
        divergent["answers"][0]["selected_option_ids"] = serde_json::json!(["no"]);
        let (status, body) = post_choice(&fixture.router, fixture.attempt_id.as_str(), "choice-http-1", &divergent).await;
        assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
        let (status, _) = post_choice(
            &fixture.router,
            fixture.attempt_id.as_str(),
            "choice-http-1",
            &request_body("cmd-second", &fixture.incarnation),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);

        // run 结束后同 command → 410（旧 run 许可无效，不可送新 run）。
        fixture.state.coding_runs.remove(&fixture.attempt_key, fixture.run_id);
        let (status, body) = post_choice(
            &fixture.router,
            fixture.attempt_id.as_str(),
            "choice-http-1",
            &request_body("cmd-first", &fixture.incarnation),
        )
        .await;
        assert_eq!(status, StatusCode::GONE, "body: {body}");
        // 状态登记面：同 command 仍可见 Expired。
        let (status, body) = get_status(&fixture.router, fixture.attempt_id.as_str(), "choice-http-1", "cmd-first").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["state"], "expired");
    }

    async fn get_attempt_snapshot(
        router: &axum::Router,
        attempt_id: &str,
    ) -> serde_json::Value {
        let uri = format!(
            "/api/projects/project_0001/issues/issue_0001/coding-attempts/{attempt_id}"
        );
        let response = router
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// P0 1.3（REQ-WIGA-05）Task 11：冷驾驶舱（无 coding socket）作答依赖
    /// GET attempt snapshot 携带应答须绑定的 run 化身——claim 要求
    /// `expected_run_id === active_run_incarnation`，快照不暴露则 REST 永远 410。
    #[tokio::test]
    async fn attempt_snapshot_stamps_pending_choice_expected_run_id() {
        let fixture = coding_choice_http_fixture().await;

        let body = get_attempt_snapshot(&fixture.router, fixture.attempt_id.as_str()).await;
        let choices = body["pending_choices"].as_array().unwrap();
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0]["choice_id"], "choice-http-1");
        assert_eq!(
            choices[0]["expected_run_id"].as_str().unwrap(),
            fixture.incarnation,
            "快照 open choice 必须带 active run incarnation（REST 作答绑定）"
        );

        // run 结束后快照不再携带化身（无活跃 run 不猜、不送新 run）。
        fixture.state.coding_runs.remove(&fixture.attempt_key, fixture.run_id);
        let body = get_attempt_snapshot(&fixture.router, fixture.attempt_id.as_str()).await;
        assert!(
            body["pending_choices"][0].get("expected_run_id").is_none(),
            "无活跃 run 时不得伪造化身"
        );
    }
}
