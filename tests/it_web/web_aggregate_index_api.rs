//! 聚合索引端点集成覆盖（假 CodeGraph CLI）。
//!
//! 端点行为覆盖与夹具前置解耦：`web_coding_attempt_api` 的夹具不再真跑
//! `codegraph sync .`（本机重活、并行互踩 ⇒ 30s 增量预算耗尽 ⇒ 422
//! `codegraph_timeout`），改由本模块用同一假 CLI 覆盖 rebuild / active 端点。
//!
//! 覆盖要点：
//! - `POST /api/projects/{p}/logical-codebases/{lc}/aggregate-indexes/rebuild`
//!   → 200，且发布一条 active 世代（durable 落盘、版本取假 CLI 实测值）。
//! - `GET  /api/projects/{p}/logical-codebases/{lc}/aggregate-indexes/active`
//!   → 200，投影为 active（与 rebuild 同一世代）。

use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::IdentityMigrationExecutor;
use cadence_aria::product::logical_codebase::aggregate_index::{
    AggregateIndexStore, CODEGRAPH_MIN_VERSION,
};
use cadence_aria::product::logical_codebase::store::legacy_logical_codebase_id;
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::json;
use tempfile::tempdir;

use crate::aggregate_index_fixture::fake_index_operation;

const PROJECT_ID: &str = "project_0001";

#[tokio::test]
async fn rebuild_endpoint_publishes_active_generation_and_active_projection_reads_it() {
    let root = tempdir().expect("root");
    let repo = crate::web_coding_attempt_api::git_repo();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lc_id = legacy_logical_codebase_id(PROJECT_ID);

    // 假 CLI 驱动的聚合索引操作（真实 git 快照 + 真实 durable 落盘）。
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    )
    .with_aggregate_index_operation(fake_index_operation(&paths, PROJECT_ID, &lc_id));
    let app = build_web_router(state);

    // project + repository 注册（真实 HTTP 路由）。
    crate::web_coding_attempt_api::request_json(
        app.clone(),
        Method::POST,
        "/api/projects",
        json!({"name":"Coding","description":null}),
    )
    .await;
    let repository = crate::web_coding_attempt_api::register_repository_and_wait(
        app.clone(),
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(repository["repository_id"], "repository_0001");

    // 身份迁移：repository_0001 → LC 权威（manifest/member/checkout 落盘）。
    IdentityMigrationExecutor::new(paths.clone())
        .ensure_identity_schema(PROJECT_ID)
        .expect("migrate fixture repository into logical codebase authority");

    // rebuild 端点：真实路由 + 生产 handler + 假 CLI ⇒ 200 + active 世代。
    let (status, rebuilt) = crate::web_coding_attempt_api::request_json(
        app.clone(),
        Method::POST,
        &format!("/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/aggregate-indexes/rebuild"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rebuilt}");
    assert_eq!(rebuilt["state"], "active", "{rebuilt}");

    // durable 世代：active 且版本取假 CLI 的实测应答（证明走的是假 CLI）。
    let record = AggregateIndexStore::for_lc(paths.clone(), lc_id.clone())
        .active_required(PROJECT_ID)
        .expect("durable active aggregate-index generation");
    assert_eq!(
        record.codegraph_version,
        format!(
            "{}.{}.{}",
            CODEGRAPH_MIN_VERSION.0, CODEGRAPH_MIN_VERSION.1, CODEGRAPH_MIN_VERSION.2
        ),
        "active generation must carry the codegraph version measured by the CLI"
    );

    // active 投影端点：与 rebuild 同一世代可读。
    let (status, projection) = crate::web_coding_attempt_api::request_json(
        app.clone(),
        Method::GET,
        &format!("/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/aggregate-indexes/active"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{projection}");
    assert_eq!(projection["state"], "active", "{projection}");
    assert_eq!(
        projection["revision"], record.membership_revision,
        "{projection}"
    );
}
