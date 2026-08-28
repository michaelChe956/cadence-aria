use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::workspace::PolicyRoutePersist;
use cadence_aria::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
};
use cadence_aria::product::models::{ProviderName, WorkspaceSessionStatus, WorkspaceType};
use cadence_aria::product::work_item_plan_policy::{
    ClassifiedFinding, FindingClass, FindingFingerprint, HumanGateSnapshot, HumanReason,
    ReviewFindingCategory, ReviewInvocationScope, RunPolicy, WorkItemPlanFlowKind,
};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

struct TakeoverFixture {
    _root: TempDir,
    app: axum::Router,
    store: LifecycleStore,
    parent_id: String,
}

fn parent_input(entity_id: &str) -> CreateWorkspaceSessionInput {
    CreateWorkspaceSessionInput {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: entity_id.to_string(),
        workspace_type: WorkspaceType::WorkItemPlan,
        author_provider: ProviderName::Codex,
        reviewer_provider: ProviderName::ClaudeCode,
        review_rounds: 2,
        superpowers_enabled: true,
        openspec_enabled: true,
        work_item_plan_options: Some(WorkItemPlanSessionOptions {
            flow_kind: WorkItemPlanFlowKind::SingleCandidate,
            run_policy: RunPolicy::AutoIfValid,
            rollout_snapshot: true,
        }),
    }
}

fn human_gate(resumable: bool) -> HumanGateSnapshot {
    let class = FindingClass::HumanRequired;
    HumanGateSnapshot {
        findings: vec![ClassifiedFinding {
            class,
            fingerprint: FindingFingerprint::for_finding(
                Some(ReviewFindingCategory::ScopeConflict),
                class,
                "需要人工决定范围",
                Some("scope"),
            ),
            category: Some(ReviewFindingCategory::ScopeConflict),
            severity: "blocking".to_string(),
            message: "需要人工决定范围".to_string(),
            evidence: Some("冲突证据".to_string()),
            required_action: Some("确认范围".to_string()),
            contract_field: Some("scope".to_string()),
        }],
        repeated_fingerprints: Vec::new(),
        attempts_used: 2,
        manual_repairs_remaining: 1,
        trigger: HumanReason::NativeHumanRequired,
        resumable,
    }
}

fn stopped_takeover_fixture() -> TakeoverFixture {
    stopped_takeover_fixture_with_resumable_gate(true)
}

fn stopped_takeover_fixture_with_resumable_gate(resumable: bool) -> TakeoverFixture {
    let root = tempfile::tempdir().expect("temporary app root");
    let app_root = root.path().to_path_buf();
    let store = LifecycleStore::new(ProductAppPaths::new(app_root.join(".aria")));
    let parent = store
        .create_workspace_session(parent_input("work_item_plan_takeover"))
        .expect("create stopped parent");
    let parent = store
        .compare_and_save_policy_route(
            &parent,
            PolicyRoutePersist {
                status: WorkspaceSessionStatus::StoppedNeedsHuman,
                single_candidate_phase: None,
                run_history: parent.run_history.clone(),
                scope: Some(ReviewInvocationScope::initial("outline:takeover")),
                gate: Some(human_gate(resumable)),
                diagnostics: Vec::new(),
                repair_reservation: None,
                provider_start_ledger: Vec::new(),
            },
        )
        .expect("persist resumable stopped parent");
    let state = WebAppState::new(app_root.clone(), WebRuntime::new_fake(app_root));
    TakeoverFixture {
        _root: root,
        app: build_web_router(state),
        store,
        parent_id: parent.id,
    }
}

fn open_parent_fixture() -> TakeoverFixture {
    let root = tempfile::tempdir().expect("temporary app root");
    let app_root = root.path().to_path_buf();
    let store = LifecycleStore::new(ProductAppPaths::new(app_root.join(".aria")));
    let parent = store
        .create_workspace_session(parent_input("work_item_plan_open"))
        .expect("create open parent");
    let state = WebAppState::new(app_root.clone(), WebRuntime::new_fake(app_root));
    TakeoverFixture {
        _root: root,
        app: build_web_router(state),
        store,
        parent_id: parent.id,
    }
}

async fn takeover(app: axum::Router, session_id: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/workspace-sessions/{session_id}/takeover"))
                .body(Body::empty())
                .expect("takeover request"),
        )
        .await
        .expect("takeover response");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("takeover response body");
    let value = serde_json::from_slice(&body).expect("takeover response JSON");
    (status, value)
}

#[tokio::test]
async fn workspace_session_takeover_returns_interactive_child_and_associated_event() {
    let fixture = stopped_takeover_fixture();

    let (status, response) = takeover(fixture.app.clone(), &fixture.parent_id).await;

    assert_eq!(status, StatusCode::OK, "takeover response: {response}");
    let child_id = response["workspace_session_id"]
        .as_str()
        .expect("child workspace session id");
    assert_eq!(response["workspace_type"], "work_item_plan");
    assert_eq!(response["status"], "open");
    assert_eq!(response["parent_session_id"], fixture.parent_id);
    assert_eq!(
        response["takeover_event_id"],
        format!("human_gate_takeover_{}", fixture.parent_id)
    );
    let child = fixture
        .store
        .get_workspace_session(child_id)
        .expect("takeover child persisted");
    assert_eq!(child.status, WorkspaceSessionStatus::Open);
    assert_eq!(child.run_policy, RunPolicy::Interactive);
    assert!(child.human_gate_snapshot.is_none());
    assert_eq!(
        fixture
            .store
            .get_human_gate_takeover_event(&fixture.parent_id)
            .expect("associated takeover event read")
            .expect("associated takeover event exists")
            .child_session_id,
        child_id
    );
}

#[tokio::test]
async fn workspace_session_takeover_is_idempotent_for_repeated_http_calls() {
    let fixture = stopped_takeover_fixture();

    let (first_status, first) = takeover(fixture.app.clone(), &fixture.parent_id).await;
    let (second_status, second) = takeover(fixture.app.clone(), &fixture.parent_id).await;

    assert_eq!(first_status, StatusCode::OK, "first takeover: {first}");
    assert_eq!(second_status, StatusCode::OK, "second takeover: {second}");
    assert_eq!(
        first["workspace_session_id"], second["workspace_session_id"],
        "repeated takeover must return the first child"
    );
    assert_eq!(first["takeover_event_id"], second["takeover_event_id"]);
    assert_eq!(
        fixture
            .store
            .list_workspace_sessions("project_0001", "issue_0001")
            .expect("list parent and takeover child")
            .len(),
        2,
        "repeated takeover must not create another child"
    );
}

#[tokio::test]
async fn workspace_session_takeover_rejects_non_stopped_parent() {
    let fixture = open_parent_fixture();

    let (status, response) = takeover(fixture.app.clone(), &fixture.parent_id).await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "takeover response: {response}"
    );
    assert_eq!(response["code"], "workspace_session_takeover_not_allowed");
    assert!(
        fixture
            .store
            .get_human_gate_takeover_event(&fixture.parent_id)
            .expect("read rejected takeover event")
            .is_none(),
        "rejected parent must not get a takeover event"
    );
}

#[tokio::test]
async fn workspace_session_takeover_rejects_non_resumable_stopped_parent() {
    let fixture = stopped_takeover_fixture_with_resumable_gate(false);

    let (status, response) = takeover(fixture.app.clone(), &fixture.parent_id).await;

    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "takeover response: {response}"
    );
    assert_eq!(response["code"], "workspace_session_takeover_not_allowed");
    assert!(
        fixture
            .store
            .get_human_gate_takeover_event(&fixture.parent_id)
            .expect("read rejected takeover event")
            .is_none(),
        "non-resumable parent must not get a takeover event"
    );
}

#[tokio::test]
async fn workspace_session_takeover_returns_not_found_for_unknown_parent() {
    let fixture = stopped_takeover_fixture();

    let (status, response) = takeover(fixture.app, "workspace_session_missing").await;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "takeover response: {response}"
    );
    assert_eq!(response["code"], "workspace_session_not_found");
}
