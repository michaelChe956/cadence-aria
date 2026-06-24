use super::*;

pub async fn health() -> Json<serde_json::Value> {
    Json(json!({"status":"ok"}))
}

pub async fn runtime_info(State(state): State<WebAppState>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "package_version": env!("CARGO_PKG_VERSION"),
        "git_sha": option_env!("ARIA_GIT_SHA").unwrap_or("unknown"),
        "branch": option_env!("ARIA_GIT_BRANCH").unwrap_or("unknown"),
        "built_at_unix": option_env!("ARIA_BUILT_AT_UNIX").unwrap_or("unknown"),
        "workspace_root": state.workspace_root.display().to_string(),
        "features": {
            "testing_result_review_gate": true,
            "coding_choice_gate": true
        }
    }))
}
