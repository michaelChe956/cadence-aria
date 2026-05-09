use axum::Router;
use axum::routing::{get, post};

use crate::web::handlers;
use crate::web::state::WebAppState;

pub fn build_web_router(state: WebAppState) -> Router {
    Router::new()
        .route("/api/health", get(handlers::health))
        .route("/api/events", get(handlers::events))
        .route("/api/projection", get(handlers::projection))
        .route("/api/tasks", post(handlers::create_task))
        .route("/api/tasks/{task_id}/advance", post(handlers::advance_task))
        .route("/api/tasks/{task_id}/confirm", post(handlers::confirm_task))
        .with_state(state)
}
