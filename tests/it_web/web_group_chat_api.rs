use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::issue_store::{CreateProductIssueInput, IssueStore};
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::events::EventHub;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

async fn request_json(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value = serde_json::from_slice(&bytes).expect("json response");
    (status, value)
}

#[tokio::test]
async fn group_chat_http_create_message_finalize_and_timeline() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let project = ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "群聊测试".into(),
            description: None,
        })
        .expect("project");
    let issue = IssueStore::new(paths)
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: None,
            title: "群聊 Story".into(),
            description: Some("原始需求".into()),
            change_id: None,
        })
        .expect("issue");
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        EventHub::new(),
    );
    let app = build_web_router(state);

    let (status, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/group-chat/sessions",
        json!({"project_id": project.id, "issue_id": issue.id}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let session_id = created["id"].as_str().expect("session id").to_owned();
    assert_eq!(created["roles"][0]["role_key"], "author");
    assert_eq!(
        created["artifact_lines"].as_array().expect("lines").len(),
        3
    );
    let (status, existing) = request_json(
        app.clone(),
        Method::POST,
        "/api/group-chat/sessions",
        json!({"project_id": project.id, "issue_id": issue.id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(existing["id"], session_id);

    let (status, updated) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/group-chat/sessions/{session_id}/roles"),
        json!({
            "role_key": "researcher",
            "provider": "fake",
            "display_name": "资料研究员"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "role response: {updated}");
    assert!(
        updated["roles"]
            .as_array()
            .expect("roles")
            .iter()
            .any(|role| {
                role["role_key"] == "researcher" && role["display_name"] == "资料研究员"
            })
    );

    let (status, invalid_message) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/group-chat/sessions/{session_id}/messages"),
        json!({"text": "invalid slot", "draft_slot": "unknown_slot"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_message["code"], "invalid_group_chat_draft_slot");

    let (status, message) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/group-chat/sessions/{session_id}/messages"),
        json!({
            "text": "请起草 Story Spec",
            "mentions": ["role_1"],
            "draft_slot": "story_full"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "message response: {message}");
    assert!(message["summary"]["appended_seqs"].as_array().is_some());
    assert!(
        message["session"]["artifact_lines"]
            .as_array()
            .expect("lines")
            .iter()
            .any(|line| line["kind"] == "story_spec"
                && line["drafts"]
                    .as_array()
                    .expect("drafts")
                    .iter()
                    .any(|draft| {
                        draft["slot_key"] == "story_full" && draft["current"].is_object()
                    }))
    );

    let (status, finalized) = request_json(
        app.clone(),
        Method::POST,
        &format!("/api/group-chat/sessions/{session_id}/finalize"),
        json!({"line_kind":"story_spec", "confirmed_by":"test-user"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "finalize response: {finalized}");
    assert_eq!(finalized["event"]["type"], "finalize_event");
    assert_eq!(finalized["event"]["artifact_line"], "story_spec");

    let (status, fetched) = request_json(
        app,
        Method::GET,
        &format!("/api/group-chat/sessions/{session_id}?after_seq=0"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let timeline = fetched["timeline"].as_array().expect("timeline");
    assert!(
        timeline
            .iter()
            .any(|entry| entry["event"]["type"] == "user_message")
    );
    assert!(
        timeline
            .iter()
            .any(|entry| entry["event"]["type"] == "agent_message")
    );
    assert!(
        timeline
            .iter()
            .any(|entry| entry["event"]["type"] == "finalize_event")
    );
}

#[tokio::test]
async fn group_chat_mode_and_triage_provider_settings_are_persisted() {
    let root = tempdir().expect("root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let project = ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "配置测试".into(),
            description: None,
        })
        .expect("project");
    let issue = IssueStore::new(paths)
        .create(CreateProductIssueInput {
            project_id: project.id.clone(),
            repo_id: None,
            title: "配置 Issue".into(),
            description: None,
            change_id: None,
        })
        .expect("issue");
    let state = WebAppState::with_events(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
        EventHub::new(),
    );
    let app = build_web_router(state);
    let (status, mode) = request_json(
        app.clone(),
        Method::GET,
        "/api/settings/spec-generation-mode",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mode, "pipeline");
    let (status, mode) = request_json(
        app.clone(),
        Method::PUT,
        "/api/settings/spec-generation-mode",
        json!("group_chat"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(mode, "group_chat");

    let (_, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/group-chat/sessions",
        json!({"project_id": project.id, "issue_id": issue.id}),
    )
    .await;
    let session_id = created["id"].as_str().expect("session id");
    let (status, triage) = request_json(
        app.clone(),
        Method::PUT,
        &format!("/api/group-chat/sessions/{session_id}/settings/triage-provider"),
        json!({"provider":"fake"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(triage["provider"], "fake");
    let (status, triage) = request_json(
        app,
        Method::GET,
        &format!("/api/group-chat/sessions/{session_id}/settings/triage-provider"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(triage["provider"], "fake");
}
