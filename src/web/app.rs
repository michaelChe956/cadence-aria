use axum::Router;
use axum::routing::{get, post};
use std::net::SocketAddr;
use tokio::net::TcpListener;

use crate::web::handlers;
use crate::web::state::WebAppState;

pub fn build_web_router(state: WebAppState) -> Router {
    Router::new()
        .route("/api/health", get(handlers::health))
        .route("/api/events", get(handlers::events))
        .route("/api/projection", get(handlers::projection))
        .route(
            "/api/tasks",
            get(handlers::list_tasks).post(handlers::create_task),
        )
        .route("/api/tasks/{task_id}/advance", post(handlers::advance_task))
        .route("/api/tasks/{task_id}/confirm", post(handlers::confirm_task))
        .route("/api/tasks/{task_id}/stop", post(handlers::stop_task))
        .route(
            "/api/artifacts/{artifact_ref}",
            get(handlers::artifact_content),
        )
        .route("/api/files/content", get(handlers::file_content))
        .route("/api/files/diff", get(handlers::file_diff))
        .with_state(state)
}

pub async fn serve_web(
    workspace_root: std::path::PathBuf,
    host: String,
    port: Option<u16>,
) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", host, port.unwrap_or(0)).parse()?;
    let state = WebAppState::new(
        workspace_root.clone(),
        crate::web::runtime::WebRuntime::new_fake(workspace_root),
    );
    let app =
        build_web_router(state).fallback_service(crate::web::static_assets::static_dist_service());
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    eprintln!("aria web listening on http://{bound_addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
