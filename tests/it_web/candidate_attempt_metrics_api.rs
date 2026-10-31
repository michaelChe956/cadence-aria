//! REQ-WSC-25 度量读数的只读 HTTP 出口（B）：
//! `GET /api/projects/{project_id}/issues/{issue_id}/candidate-attempt-metrics`。
//!
//! `compute_candidate_attempt_metrics` / `list_issue_candidate_attempts` 此前
//! 已实现但没有任何 HTTP/CLI 调用方——owner 打不开。本用例钉住：
//! 1. 端点返回该 issue 的尝试序列 + 读数（首次通过率、失败码 Top N、同 issue
//!    连续失败次数、lowering 终态失败计数），字段与既有结构同形；
//! 2. 端点**只读**：调用前后 durable `candidate-attempts.jsonl` 逐字节不变。

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::lifecycle_store::candidate_attempts::{
    CandidateAttemptFailure, CandidateAttemptOutcome, CandidateAttemptRecord, CandidateAttemptStage,
};
use cadence_aria::product::models::{ProviderName, WorkspaceType};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use std::{fs, process::Command};
use tempfile::tempdir;
use tower::ServiceExt;

async fn request_json(
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
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

fn git_repo() -> tempfile::TempDir {
    let dir = tempdir().expect("repo");
    let status = Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
    assert!(status.success());
    for args in [
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test"],
        vec!["commit", "--allow-empty", "-m", "fixture baseline"],
    ] {
        let status = Command::new("git")
            .args(&args)
            .current_dir(dir.path())
            .status()
            .expect("git fixture");
        assert!(status.success());
    }
    dir
}

fn attempt_record(
    attempt_key: &str,
    recorded_at: &str,
    stage: CandidateAttemptStage,
    outcome: CandidateAttemptOutcome,
    failure: Option<CandidateAttemptFailure>,
) -> CandidateAttemptRecord {
    CandidateAttemptRecord {
        attempt_seq: 0,
        attempt_key: attempt_key.to_string(),
        session_id: String::new(),
        issue_id: String::new(),
        recorded_at: recorded_at.to_string(),
        stage,
        outcome,
        source_revision_ref: None,
        plan_candidate_ir_ref: None,
        failure,
        auto_rewrite: None,
        auto_repairs_applied: Vec::new(),
        equivalence_canonicalizations: Vec::new(),
    }
}

#[tokio::test]
async fn candidate_attempt_metrics_endpoint_returns_series_and_readonly_metrics() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));

    request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Metrics","description":null}),
    )
    .await;
    crate::create_repository_and_wait(
        app.clone(),
        "project_0001",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    let (status, _issue) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues",
        json!({
            "title":"候选尝试度量",
            "description":"验证尝试流水读数的只读出口",
            "repository_id":"repository_0001"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 种一个 WorkItemPlan 会话 + 两行尝试（同 issue：1 次结构拒收失败 + 1 次
    // 终编译通过）——度量输入与真实落盘路径完全一致（store 盖印序号/身份）。
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let session = lifecycle
        .create_workspace_session(
            cadence_aria::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "plan_metrics_fixture".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: None,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
        )
        .expect("create work item plan session");
    let failed_row = attempt_record(
        "pass/structure-rejected-0001",
        "2026-10-31T00:00:00Z",
        CandidateAttemptStage::Structure,
        CandidateAttemptOutcome::Failed,
        Some(CandidateAttemptFailure {
            failure_id: "planning_compile_failure_metrics_fixture".to_string(),
            code: "final_compile_structure_rejected".to_string(),
            typed_kinds: Vec::new(),
            cycle_fingerprint: None,
        }),
    );
    assert!(
        lifecycle
            .append_candidate_attempt(&session.id, &failed_row)
            .expect("append failure row")
    );
    let passed_row = attempt_record(
        "pass/final-passed-0002",
        "2026-10-31T00:01:00Z",
        CandidateAttemptStage::Final,
        CandidateAttemptOutcome::Passed,
        None,
    );
    assert!(
        lifecycle
            .append_candidate_attempt(&session.id, &passed_row)
            .expect("append passed row")
    );
    let attempts_file = root
        .path()
        .join(".aria")
        .join("projects/project_0001/issues/issue_0001/workspace-sessions")
        .join(&session.id)
        .join("candidate-attempts.jsonl");
    let before = fs::read(&attempts_file).expect("attempts file must exist after seeding");

    let (status, body) = request_json(
        app.clone(),
        Method::GET,
        "/api/projects/project_0001/issues/issue_0001/candidate-attempt-metrics",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    // 尝试序列：同 issue 跨行全量、按时间升序。
    let attempts = body["attempts"].as_array().expect("attempts array");
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0]["outcome"], "failed");
    assert_eq!(attempts[0]["stage"], "structure");
    assert_eq!(
        attempts[0]["failure"]["code"],
        "final_compile_structure_rejected"
    );
    assert_eq!(attempts[1]["outcome"], "passed");
    assert_eq!(attempts[1]["stage"], "final");

    // 读数：与 CandidateAttemptMetrics 既有字段同形。
    let metrics = &body["metrics"];
    assert_eq!(metrics["issue_count"], 1);
    assert_eq!(metrics["first_pass_issue_count"], 0);
    assert_eq!(metrics["first_pass_rate"], 0.0);
    assert_eq!(metrics["consecutive_failure_p50"], 1.0);
    assert_eq!(metrics["consecutive_failure_p90"], 1.0);
    assert_eq!(
        metrics["failure_code_ranking"][0]["code"],
        "final_compile_structure_rejected"
    );
    assert_eq!(metrics["failure_code_ranking"][0]["count"], 1);
    assert_eq!(metrics["lowering_terminal_failure_count"], 0);

    // 只读：调用前后 durable 流水逐字节不变，重复调用结果一致。
    let after = fs::read(&attempts_file).expect("attempts file after GET");
    assert_eq!(
        before, after,
        "度量端点必须只读，不得改写 append-only 尝试流水"
    );
    let (status_again, body_again) = request_json(
        app,
        Method::GET,
        "/api/projects/project_0001/issues/issue_0001/candidate-attempt-metrics",
        json!({}),
    )
    .await;
    assert_eq!(status_again, StatusCode::OK);
    assert_eq!(body, body_again, "只读端点重复调用必须幂等同形");
}

#[tokio::test]
async fn candidate_attempt_metrics_endpoint_rejects_invalid_ids() {
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    // 非法相对 id（`%2F` 解码后含路径分隔符）→ 400 稳定码，不触盘；裸
    // 路径穿越（`..`）由 axum 路由层先行 404（不在本端点重复实现）。
    let (status, body) = request_json(
        app,
        Method::GET,
        "/api/projects/project%2Fsecrets/issues/issue_0001/candidate-attempt-metrics",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert_eq!(body["code"], "invalid_project_id");
}
