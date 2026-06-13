use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    FakeStreamingProvider, PermissionRequestData, ProviderCommand, ProviderEvent, ProviderSession,
    RiskLevel, StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateBlockedGateInput};
use crate::product::coding_models::{
    CodingAgentRole, CodingAttemptStatus, CodingChatEntry, CodingEntryType, CodingExecutionAttempt,
    CodingExecutionStage as FixtureStage, CodingGateAction, CodingGateActionType,
    CodingProviderRole, CodingRoleRunEventType, CodingRoleRunStatus, CodingRoleRunTrigger,
    CodingTimelineNode, CodingTimelineNodeStatus, PushStatus, RemoteKind, ReviewRequest,
    ReviewRequestKind,
};
use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
use crate::product::lifecycle_store::{
    CreateStorySpecInput, CreateWorkspaceSessionInput, LifecycleStore,
};
use crate::product::models::{
    AgentRole, NodeDetail, ProviderName, ProviderSnapshot, WorkItemPlanStatus, WorkspaceType,
};
use crate::product::project_store::{CreateProjectInput, ProjectStore};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::protocol::contracts::{AdapterInput, AdapterRole};
use crate::web::state::WebAppState;
use crate::web::workspace_ws_types::{
    ArtifactVersion, ProviderConfigSnapshot, ReviewVerdictType, TimelineNode, TimelineNodeStatus,
    TimelineNodeType, WorkspaceStage, WsExecutionEventKind, WsExecutionEventStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceSocketControl {
    CloseForTestDrop,
}

#[derive(Clone, Default)]
pub struct TestControls {
    inner: Arc<TestControlsInner>,
}

#[derive(Default)]
struct TestControlsInner {
    workspace_sockets: Mutex<HashMap<String, Vec<mpsc::Sender<WorkspaceSocketControl>>>>,
    workspace_socket_rejects: Mutex<HashMap<String, u32>>,
    permission_fixture_sessions: Mutex<HashSet<String>>,
    testing_fixture_sessions: Mutex<HashMap<String, TestingFixtureState>>,
    review_fixture_sessions: Mutex<HashMap<String, VecDeque<ReviewFixture>>>,
    permission_timeout: Mutex<Option<Duration>>,
    server_idle_timeout: Mutex<Option<Duration>>,
}

pub fn test_controls_enabled() -> bool {
    std::env::var("ARIA_E2E_TEST_CONTROLS").as_deref() == Ok("1")
}

#[derive(Debug, Deserialize)]
pub struct PermissionFixtureRequest {
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewFixture {
    pub verdict: String,
    pub summary: String,
    pub comments: String,
    #[serde(default)]
    pub raw_json: Option<Value>,
    #[serde(default)]
    pub raw_text: Option<String>,
    #[serde(default)]
    pub findings: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestingFixture {
    pub plan_output: Value,
    #[serde(default)]
    pub step_results: Vec<Value>,
    #[serde(default)]
    pub malformed_plan_output: Option<String>,
    #[serde(default)]
    pub provider_failure: Option<String>,
}

#[derive(Debug, Clone)]
struct TestingFixtureState {
    fixture: TestingFixture,
    plan_consumed: bool,
}

#[derive(Debug, Clone)]
enum TestingFixtureRun {
    Output(String),
    Failure(String),
}

#[derive(Debug, Deserialize)]
pub struct PermissionTimeoutRequest {
    pub timeout_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct WsTimeoutRequest {
    pub server_idle_timeout_ms: Option<u64>,
    pub client_idle_timeout_ms: Option<u64>,
    pub suppress_server_messages: Option<bool>,
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WsRejectRequest {
    pub count: u32,
}

pub async fn drop_workspace_socket(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
) -> Json<serde_json::Value> {
    let dropped = state
        .test_controls
        .drop_workspace_socket_when_registered(&session_id, Duration::from_secs(2))
        .await;
    Json(json!({"status": "ok", "dropped": dropped}))
}

pub async fn reject_next_workspace_sockets(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<WsRejectRequest>,
) -> Json<serde_json::Value> {
    state
        .test_controls
        .reject_next_workspace_sockets(session_id, request.count)
        .await;
    Json(json!({"status": "ok"}))
}

pub async fn enable_permission_fixture(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<PermissionFixtureRequest>,
) -> Json<serde_json::Value> {
    let _mode = request.mode.as_deref().unwrap_or("single-request");
    state
        .test_controls
        .enable_permission_fixture(session_id)
        .await;
    Json(json!({"status": "ok"}))
}

pub async fn enable_review_fixture(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<ReviewFixture>,
) -> Json<serde_json::Value> {
    state
        .test_controls
        .enable_review_fixture(session_id, request)
        .await;
    Json(json!({"status": "ok"}))
}

pub async fn enable_testing_fixture(
    State(state): State<WebAppState>,
    Path(attempt_id): Path<String>,
    Json(request): Json<TestingFixture>,
) -> Json<serde_json::Value> {
    state
        .test_controls
        .enable_testing_fixture(attempt_id, request)
        .await;
    Json(json!({"status": "ok"}))
}

pub async fn set_permission_timeout(
    State(state): State<WebAppState>,
    Json(request): Json<PermissionTimeoutRequest>,
) -> Json<serde_json::Value> {
    state
        .test_controls
        .set_permission_timeout(Duration::from_millis(request.timeout_ms))
        .await;
    Json(json!({"status": "ok"}))
}

pub async fn set_ws_timeout(
    State(state): State<WebAppState>,
    Json(request): Json<WsTimeoutRequest>,
) -> Json<serde_json::Value> {
    if let Some(timeout_ms) = request.server_idle_timeout_ms {
        state
            .test_controls
            .set_server_idle_timeout(Duration::from_millis(timeout_ms))
            .await;
    }
    let _ = (
        request.client_idle_timeout_ms,
        request.suppress_server_messages,
        request.session_id,
    );
    Json(json!({"status": "ok"}))
}

pub async fn seed_large_workspace_fixture(
    State(state): State<WebAppState>,
) -> Json<serde_json::Value> {
    match create_large_workspace_fixture(ProductAppPaths::new(state.workspace_root.join(".aria"))) {
        Ok(session_id) => Json(json!({"session_id": session_id})),
        Err(error) => Json(json!({"error": error.to_string()})),
    }
}

fn create_large_workspace_fixture(
    app_paths: ProductAppPaths,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let project = ProjectStore::new(app_paths.clone()).create(CreateProjectInput {
        name: "Large Workspace Memory E2E".to_string(),
        description: Some("大型 Workspace 内存治理 E2E fixture".to_string()),
    })?;
    let repository = RepositoryStore::new(app_paths.clone()).create(CreateRepositoryInput {
        project_id: project.id.clone(),
        name: "Large Fixture Repo".to_string(),
        path: app_paths.root().to_path_buf(),
        default_policy_preset: Some("manual-write".to_string()),
        default_provider_mode: Some("fake".to_string()),
    })?;
    let issue = IssueStore::new(app_paths.clone()).create(CreateProductIssueInput {
        project_id: project.id.clone(),
        repo_id: Some(repository.id.clone()),
        title: "Large Workspace Memory Issue".to_string(),
        description: Some("验证大型 workspace 的按需内容加载".to_string()),
        change_id: None,
    })?;
    let lifecycle = LifecycleStore::new(app_paths);
    let story = lifecycle.create_story_spec(CreateStorySpecInput {
        project_id: project.id.clone(),
        issue_id: issue.id.clone(),
        repository_id: repository.id,
        title: "Large Workspace Memory Story".to_string(),
    })?;
    let session = lifecycle.create_workspace_session(CreateWorkspaceSessionInput {
        project_id: project.id,
        issue_id: issue.id,
        entity_id: story.id,
        workspace_type: WorkspaceType::Story,
        author_provider: ProviderName::Codex,
        reviewer_provider: ProviderName::ClaudeCode,
        review_rounds: 5,
        superpowers_enabled: false,
        openspec_enabled: true,
    })?;

    let session_id = session.id;
    let now = chrono::Utc::now().to_rfc3339();
    let provider_snapshot = ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::ClaudeCode),
        review_rounds: 5,
    };
    let mut nodes = Vec::new();
    for index in 0..45 {
        let node_id = format!("timeline_node_{:03}", index + 1);
        let is_provider_node = index >= 33;
        let provider_index = index - 33;
        let node_type = if is_provider_node {
            if provider_index % 2 == 0 {
                TimelineNodeType::AuthorRun
            } else {
                TimelineNodeType::ReviewerRun
            }
        } else {
            TimelineNodeType::ContextNote
        };
        let stage = match node_type {
            TimelineNodeType::ReviewerRun => WorkspaceStage::CrossReview,
            TimelineNodeType::HumanConfirm => WorkspaceStage::HumanConfirm,
            TimelineNodeType::ContextNote => WorkspaceStage::PrepareContext,
            _ => WorkspaceStage::Running,
        };
        let agent = match node_type {
            TimelineNodeType::AuthorRun => Some(ProviderName::Codex),
            TimelineNodeType::ReviewerRun => Some(ProviderName::ClaudeCode),
            _ => None,
        };
        let source_artifact = index >= 40;
        nodes.push(TimelineNode {
            node_id: node_id.clone(),
            node_type: node_type.clone(),
            agent: agent.clone(),
            stage,
            round: if is_provider_node {
                Some((provider_index / 2 + 1) as u32)
            } else {
                None
            },
            status: TimelineNodeStatus::Completed,
            title: if is_provider_node {
                format!("Large Provider Stream {}", index)
            } else {
                format!("Large Timeline Node {}", index)
            },
            summary: if is_provider_node {
                Some(format!(
                    "Provider Prompt / Execution Output summary large-prompt-{provider_index} large-output-{provider_index}"
                ))
            } else {
                Some(format!("large fixture summary {}", index))
            },
            started_at: now.clone(),
            completed_at: Some(now.clone()),
            duration_ms: Some(100 + index as u64),
            artifact_ref: if source_artifact {
                Some("artifact_current".to_string())
            } else {
                None
            },
            provider_config_snapshot: provider_snapshot.clone(),
        });
        if is_provider_node {
            let prompt_index = provider_index as usize;
            let output_index = provider_index as usize;
            let prompt = large_text("完整提示词", "large-prompt", prompt_index);
            let output = large_text("完整输出", "large-output", output_index);
            lifecycle.save_node_detail(
                &session_id,
                &node_id,
                &NodeDetail {
                    node_id: node_id.clone(),
                    session_id: session_id.clone(),
                    node_type,
                    status: TimelineNodeStatus::Completed,
                    agent_role: if provider_index % 2 == 0 {
                        Some(AgentRole::Author)
                    } else {
                        Some(AgentRole::Reviewer)
                    },
                    provider: agent.map(|provider| ProviderSnapshot {
                        name: provider_name(&provider).to_string(),
                        model: provider_name(&provider).to_string(),
                    }),
                    prompt: Some(prompt),
                    messages: Vec::new(),
                    streaming_content: format!("stream summary large-output-{output_index}"),
                    execution_events: vec![json!({
                        "event_id": format!("{node_id}_output"),
                        "node_id": node_id,
                        "agent": if provider_index % 2 == 0 { "codex" } else { "claude_code" },
                        "kind": WsExecutionEventKind::Output,
                        "status": WsExecutionEventStatus::Completed,
                        "title": "Execution Output",
                        "detail": "Large output loaded on demand",
                        "command": null,
                        "cwd": null,
                        "output": output,
                        "exit_code": 0
                    })],
                    permission_events: Vec::new(),
                    verdict: None,
                    artifact_ref: None,
                    is_revision: false,
                    base_artifact_ref: None,
                    started_at: now.clone(),
                    ended_at: Some(now.clone()),
                },
            )?;
        }
    }
    lifecycle.save_timeline_nodes(&session_id, &nodes)?;

    let artifact_versions = (1..=5)
        .map(|version| ArtifactVersion {
            version,
            markdown: format!(
                "{}\n# Large Artifact v{version}\n\n{}",
                "artifact line\n".repeat(220),
                "artifact line\n".repeat(8780)
            ),
            generated_by: ProviderName::Codex,
            reviewed_by: Some(ProviderName::ClaudeCode),
            review_verdict: Some(ReviewVerdictType::Pass),
            confirmed_by: if version == 5 {
                Some("e2e".to_string())
            } else {
                None
            },
            is_current: false,
            created_at: now.clone(),
            source_node_id: format!("timeline_node_{:03}", 40 + version),
        })
        .collect::<Vec<_>>();
    lifecycle.save_artifact_versions(&session_id, &artifact_versions)?;

    Ok(session_id)
}

fn large_text(label: &str, token: &str, index: usize) -> String {
    format!(
        "{}\n{label} {token}-{index}\n{}",
        format!("{token}-{index} payload line\n").repeat(120),
        format!("{token}-{index} payload line\n").repeat(5880)
    )
}

fn provider_name(provider: &ProviderName) -> &'static str {
    match provider {
        ProviderName::ClaudeCode => "claude_code",
        ProviderName::Codex => "codex",
        ProviderName::Fake => "fake",
    }
}

#[derive(Debug, Deserialize)]
pub struct CodingRoleRunFixtureRequest {
    #[serde(default = "default_blocked_stage")]
    pub blocked_stage: String,
}

fn default_blocked_stage() -> String {
    "rework".to_string()
}

static CODING_FIXTURE_ATTEMPT_COUNTER: AtomicU64 = AtomicU64::new(1);

pub async fn seed_coding_role_run_fixture(
    State(state): State<WebAppState>,
    Json(request): Json<CodingRoleRunFixtureRequest>,
) -> Json<serde_json::Value> {
    match create_coding_role_run_fixture(
        ProductAppPaths::new(state.workspace_root.join(".aria")),
        &state.workspace_root,
        &request.blocked_stage,
    ) {
        Ok(value) => Json(value),
        Err(error) => Json(json!({"error": error.to_string()})),
    }
}

fn create_coding_role_run_fixture(
    app_paths: ProductAppPaths,
    workspace_root: &std::path::Path,
    blocked_stage: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let repo_path = workspace_root.join("coding-role-run-fixture-repo");
    init_git_repo(&repo_path)?;

    let project = ProjectStore::new(app_paths.clone()).create(CreateProjectInput {
        name: "Coding Role Run Fixture".to_string(),
        description: Some("Role run history E2E fixture".to_string()),
    })?;
    let repository = RepositoryStore::new(app_paths.clone()).create(CreateRepositoryInput {
        project_id: project.id.clone(),
        name: "Fixture Repo".to_string(),
        path: repo_path.clone(),
        default_policy_preset: Some("manual-write".to_string()),
        default_provider_mode: Some("fake".to_string()),
    })?;
    let issue = IssueStore::new(app_paths.clone()).create(CreateProductIssueInput {
        project_id: project.id.clone(),
        repo_id: Some(repository.id.clone()),
        title: "Coding Role Run Issue".to_string(),
        description: Some("Issue for role run history E2E".to_string()),
        change_id: None,
    })?;

    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle.create_story_spec(CreateStorySpecInput {
        project_id: project.id.clone(),
        issue_id: issue.id.clone(),
        repository_id: repository.id.clone(),
        title: "Fixture Story".to_string(),
    })?;
    let work_item =
        lifecycle.create_work_item(crate::product::lifecycle_store::CreateWorkItemInput {
            project_id: project.id.clone(),
            issue_id: issue.id.clone(),
            repository_id: repository.id.clone(),
            story_spec_ids: vec![story.id],
            design_spec_ids: Vec::new(),
            title: "Fixture Work Item".to_string(),
        })?;
    lifecycle.update_work_item_plan_status(
        &project.id,
        &issue.id,
        &work_item.id,
        WorkItemPlanStatus::Confirmed,
    )?;

    let store = CodingAttemptStore::new(app_paths);
    let provider_snapshot = ProviderConfigSnapshot {
        author: ProviderName::Fake,
        reviewer: Some(ProviderName::Fake),
        review_rounds: 1,
    };
    let attempt_index = CODING_FIXTURE_ATTEMPT_COUNTER.fetch_add(1, Ordering::SeqCst);
    let attempt_id = format!("coding_attempt_{attempt_index:04}");
    let now = chrono::Utc::now().to_rfc3339();
    let blocked_stage_internal = blocked_stage == "internal_pr_review";
    let attempt = CodingExecutionAttempt {
        id: attempt_id,
        project_id: project.id.clone(),
        issue_id: issue.id.clone(),
        work_item_id: work_item.id.clone(),
        attempt_no: 1,
        status: CodingAttemptStatus::Blocked,
        stage: if blocked_stage_internal {
            FixtureStage::InternalPrReview
        } else {
            FixtureStage::Rework
        },
        base_branch: "HEAD".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: Some(repo_path.clone()),
        provider_config_snapshot: provider_snapshot.clone(),
        rework_count: if blocked_stage_internal { 0 } else { 1 },
        max_auto_rework: 2,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        provider_conversations: Vec::new(),
        created_at: now.clone(),
        updated_at: now.clone(),
        completed_at: None,
    };
    store.save_coding_attempt(&attempt)?;

    let now = chrono::Utc::now().to_rfc3339();
    let testing_node = CodingTimelineNode {
        id: "coding_node_0001".to_string(),
        attempt_id: attempt.id.clone(),
        stage: FixtureStage::Testing,
        title: "执行测试".to_string(),
        status: CodingTimelineNodeStatus::Completed,
        agent_role: Some(CodingAgentRole::Tester),
        summary: Some("测试完成".to_string()),
        started_at: now.clone(),
        completed_at: Some(now.clone()),
        artifact_refs: Vec::new(),
    };
    store.save_timeline_node(testing_node.clone())?;

    let tester_raw = store.save_provider_raw_output(
        &attempt.id,
        FixtureStage::Testing,
        "plan_tests",
        "fixture tester raw output",
    )?;
    let tester_run = store.create_role_run(
        &attempt,
        FixtureStage::Testing,
        CodingProviderRole::Tester,
        CodingRoleRunTrigger::Initial,
        Some("coding_node_0001".to_string()),
    )?;
    store.append_role_run_event(
        &attempt,
        &tester_run,
        CodingRoleRunEventType::ExecutionEvent,
        json!({
            "title": "Tester task update",
            "status": "running",
            "detail": "No tasks found"
        }),
    )?;
    store.update_role_run_refs(
        &project.id,
        &issue.id,
        &attempt.id,
        &tester_run.id,
        vec![tester_raw],
        Vec::new(),
    )?;
    store.update_role_run_status(
        &project.id,
        &issue.id,
        &attempt.id,
        &tester_run.id,
        CodingRoleRunStatus::Completed,
        None,
    )?;

    store.save_chat_entry(&CodingChatEntry {
        id: "coding_node_0001_tester_report".to_string(),
        attempt_id: attempt.id.clone(),
        node_id: Some("coding_node_0001".to_string()),
        role: CodingAgentRole::Tester,
        entry_type: CodingEntryType::AssistantMessage,
        content: Some("fixture tester output".to_string()),
        metadata: Some(json!({
            "source": "testing_result",
            "role_run_id": tester_run.id,
            "run_no": tester_run.run_no,
        })),
        created_at: now.clone(),
    })?;

    if blocked_stage == "internal_pr_review" {
        let review_request = ReviewRequest {
            id: "review_request_0001".to_string(),
            attempt_id: attempt.id.clone(),
            kind: ReviewRequestKind::GitBranchOnly,
            remote_kind: RemoteKind::GenericGit,
            remote: "origin".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            commit_sha: "e2e-fixture-commit".to_string(),
            push_status: PushStatus::Pushed,
            external_url: None,
            manual_instructions: vec!["E2E fixture review request".to_string()],
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        store.save_review_request(&review_request)?;

        let internal_node = CodingTimelineNode {
            id: "coding_node_0002".to_string(),
            attempt_id: attempt.id.clone(),
            stage: FixtureStage::InternalPrReview,
            title: "内部 PR 审查".to_string(),
            status: CodingTimelineNodeStatus::Blocked,
            agent_role: Some(CodingAgentRole::Reviewer),
            summary: Some("内部审查阻塞".to_string()),
            started_at: now.clone(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        store.save_timeline_node(internal_node.clone())?;

        let internal_raw = store.save_provider_raw_output(
            &attempt.id,
            FixtureStage::InternalPrReview,
            "internal_pr_review",
            "fixture internal review raw output",
        )?;
        let internal_run = store.create_role_run(
            &attempt,
            FixtureStage::InternalPrReview,
            CodingProviderRole::InternalReviewer,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0002".to_string()),
        )?;
        store.append_role_run_event(
            &attempt,
            &internal_run,
            CodingRoleRunEventType::ExecutionEvent,
            json!({
                "title": "Internal reviewer task update",
                "status": "blocked",
                "detail": "Inspecting pushed review request"
            }),
        )?;
        store.update_role_run_refs(
            &project.id,
            &issue.id,
            &attempt.id,
            &internal_run.id,
            vec![internal_raw],
            Vec::new(),
        )?;
        store.update_role_run_status(
            &project.id,
            &issue.id,
            &attempt.id,
            &internal_run.id,
            CodingRoleRunStatus::Blocked,
            Some("internal_review_blocked".to_string()),
        )?;

        store.save_chat_entry(&CodingChatEntry {
            id: "coding_node_0002_internal_review".to_string(),
            attempt_id: attempt.id.clone(),
            node_id: Some("coding_node_0002".to_string()),
            role: CodingAgentRole::Reviewer,
            entry_type: CodingEntryType::AssistantMessage,
            content: Some("fixture internal review blocked".to_string()),
            metadata: Some(json!({
                "source": "internal_pr_review",
                "role_run_id": internal_run.id,
                "run_no": internal_run.run_no,
            })),
            created_at: now.clone(),
        })?;

        store.create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: FixtureStage::InternalPrReview,
            node_id: Some("coding_node_0002".to_string()),
            role: Some(CodingProviderRole::InternalReviewer),
            title: "内部 PR 审查阻塞".to_string(),
            description: "需要重试内部审查".to_string(),
            reason_code: Some("internal_review_blocked".to_string()),
            evidence_refs: Vec::new(),
            raw_provider_output_ref: None,
            available_actions: vec![
                CodingGateAction {
                    action_id: "retry_internal_review".to_string(),
                    label: "重试审查".to_string(),
                    action_type: CodingGateActionType::RetryInternalReview,
                },
                CodingGateAction {
                    action_id: "manual_continue".to_string(),
                    label: "人工继续".to_string(),
                    action_type: CodingGateActionType::ManualContinue,
                },
                CodingGateAction {
                    action_id: "abort".to_string(),
                    label: "终止".to_string(),
                    action_type: CodingGateActionType::Abort,
                },
            ],
        })?;
    } else {
        let analyst_node = CodingTimelineNode {
            id: "coding_node_0002".to_string(),
            attempt_id: attempt.id.clone(),
            stage: FixtureStage::Rework,
            title: "Analyst 路由决策".to_string(),
            status: CodingTimelineNodeStatus::Blocked,
            agent_role: Some(CodingAgentRole::System),
            summary: Some("需要人工处理".to_string()),
            started_at: now.clone(),
            completed_at: None,
            artifact_refs: Vec::new(),
        };
        store.save_timeline_node(analyst_node.clone())?;

        let analyst_evidence = store.save_provider_raw_output(
            &attempt.id,
            FixtureStage::Rework,
            "analyst_evidence",
            "fixture analyst evidence",
        )?;
        let analyst_run = store.create_role_run(
            &attempt,
            FixtureStage::Rework,
            CodingProviderRole::Analyst,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0002".to_string()),
        )?;
        store.append_role_run_event(
            &attempt,
            &analyst_run,
            CodingRoleRunEventType::ExecutionEvent,
            json!({
                "title": "Analyst task update",
                "status": "blocked",
                "detail": "Inspecting previous testing evidence"
            }),
        )?;
        store.update_role_run_refs(
            &project.id,
            &issue.id,
            &attempt.id,
            &analyst_run.id,
            Vec::new(),
            vec![analyst_evidence],
        )?;
        store.update_role_run_status(
            &project.id,
            &issue.id,
            &attempt.id,
            &analyst_run.id,
            CodingRoleRunStatus::Blocked,
            Some("analyst_human_gate".to_string()),
        )?;

        store.save_chat_entry(&CodingChatEntry {
            id: "coding_node_0002_analyst_verdict".to_string(),
            attempt_id: attempt.id.clone(),
            node_id: Some("coding_node_0002".to_string()),
            role: CodingAgentRole::System,
            entry_type: CodingEntryType::AnalystVerdict {
                verdict: crate::product::coding_models::AnalystVerdict::NeedsFix,
            },
            content: Some("fixture analyst verdict".to_string()),
            metadata: Some(json!({
                "source": "rework",
                "role_run_id": analyst_run.id,
                "run_no": analyst_run.run_no,
            })),
            created_at: now.clone(),
        })?;

        store.create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: FixtureStage::Rework,
            node_id: Some("coding_node_0002".to_string()),
            role: Some(CodingProviderRole::Analyst),
            title: "Analyst human gate".to_string(),
            description: "需要重跑 Analyst".to_string(),
            reason_code: Some("analyst_human_gate".to_string()),
            evidence_refs: vec!["provider-raw/rework/analyst_evidence_0001.txt".to_string()],
            raw_provider_output_ref: None,
            available_actions: vec![
                CodingGateAction {
                    action_id: "retry_analyst".to_string(),
                    label: "重试 Analyst".to_string(),
                    action_type: CodingGateActionType::RetryAnalyst,
                },
                CodingGateAction {
                    action_id: "manual_continue".to_string(),
                    label: "人工继续".to_string(),
                    action_type: CodingGateActionType::ManualContinue,
                },
                CodingGateAction {
                    action_id: "abort".to_string(),
                    label: "终止".to_string(),
                    action_type: CodingGateActionType::Abort,
                },
            ],
        })?;
    }

    Ok(json!({
        "status": "ok",
        "project_id": project.id,
        "issue_id": issue.id,
        "attempt_id": attempt.id
    }))
}

fn init_git_repo(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    let has_commits = run_git_command_silent(path, &["rev-parse", "HEAD"]).is_ok();
    if has_commits {
        return Ok(());
    }
    run_git_command(path, &["init"])?;
    run_git_command(path, &["config", "user.email", "fixture@example.com"])?;
    run_git_command(path, &["config", "user.name", "Fixture"])?;
    std::fs::write(path.join("README.md"), "# fixture\n")?;
    run_git_command(path, &["add", "."])?;
    run_git_command(path, &["commit", "-m", "initial"])?;
    Ok(())
}

fn run_git_command_silent(
    cwd: &std::path::Path,
    args: &[&str],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err("git command failed".into());
    }
    Ok(())
}

fn run_git_command(
    cwd: &std::path::Path,
    args: &[&str],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let output = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git {:?} failed: {stderr}", args).into());
    }
    Ok(())
}

impl TestControls {
    pub async fn register_workspace_socket(
        &self,
        session_id: String,
        sender: mpsc::Sender<WorkspaceSocketControl>,
    ) {
        self.inner
            .workspace_sockets
            .lock()
            .expect("test controls workspace socket lock")
            .entry(session_id)
            .or_default()
            .push(sender);
    }

    pub async fn reject_next_workspace_sockets(&self, session_id: String, count: u32) {
        if count == 0 {
            self.inner
                .workspace_socket_rejects
                .lock()
                .expect("test controls workspace socket rejects lock")
                .remove(&session_id);
            return;
        }
        self.inner
            .workspace_socket_rejects
            .lock()
            .expect("test controls workspace socket rejects lock")
            .insert(session_id, count);
    }

    pub async fn consume_workspace_socket_reject(&self, session_id: &str) -> bool {
        let mut rejects = self
            .inner
            .workspace_socket_rejects
            .lock()
            .expect("test controls workspace socket rejects lock");
        let Some(count) = rejects.get_mut(session_id) else {
            return false;
        };
        if *count <= 1 {
            rejects.remove(session_id);
        } else {
            *count -= 1;
        }
        true
    }

    pub async fn drop_workspace_socket(&self, session_id: &str) -> bool {
        let senders = self
            .inner
            .workspace_sockets
            .lock()
            .expect("test controls workspace socket lock")
            .remove(session_id)
            .unwrap_or_default();

        let mut dropped = false;
        for sender in senders {
            if sender
                .send(WorkspaceSocketControl::CloseForTestDrop)
                .await
                .is_ok()
            {
                dropped = true;
            }
        }
        dropped
    }

    pub async fn drop_workspace_socket_when_registered(
        &self,
        session_id: &str,
        timeout: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.drop_workspace_socket(session_id).await {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    pub async fn enable_permission_fixture(&self, session_id: String) {
        self.inner
            .permission_fixture_sessions
            .lock()
            .expect("test controls permission fixture lock")
            .insert(session_id);
    }

    pub async fn enable_review_fixture(&self, session_id: String, fixture: ReviewFixture) {
        self.inner
            .review_fixture_sessions
            .lock()
            .expect("test controls review fixture lock")
            .entry(session_id)
            .or_default()
            .push_back(fixture);
    }

    pub async fn enable_testing_fixture(&self, session_id: String, fixture: TestingFixture) {
        self.inner
            .testing_fixture_sessions
            .lock()
            .expect("test controls testing fixture lock")
            .insert(
                session_id,
                TestingFixtureState {
                    fixture,
                    plan_consumed: false,
                },
            );
    }

    pub async fn consume_permission_fixture(&self, session_id: &str) -> bool {
        self.inner
            .permission_fixture_sessions
            .lock()
            .expect("test controls permission fixture lock")
            .remove(session_id)
    }

    pub async fn consume_review_fixture(&self, session_id: &str) -> Option<ReviewFixture> {
        let mut fixtures = self
            .inner
            .review_fixture_sessions
            .lock()
            .expect("test controls review fixture lock");
        let queue = fixtures.get_mut(session_id)?;
        let fixture = queue.pop_front();
        if queue.is_empty() {
            fixtures.remove(session_id);
        }
        fixture
    }

    async fn testing_fixture_run(
        &self,
        session_id: &str,
        prompt: &str,
    ) -> Option<TestingFixtureRun> {
        let mut fixtures = self
            .inner
            .testing_fixture_sessions
            .lock()
            .expect("test controls testing fixture lock");
        let state = fixtures.get_mut(session_id)?;
        if prompt.contains("execute_test_plan") && state.plan_consumed {
            let state = fixtures.remove(session_id)?;
            if let Some(message) = state.fixture.provider_failure {
                return Some(TestingFixtureRun::Failure(message));
            }
            return Some(TestingFixtureRun::Output(
                json!({ "step_results": state.fixture.step_results }).to_string(),
            ));
        }
        if prompt.contains("plan_tests") {
            state.plan_consumed = true;
            if let Some(message) = state.fixture.provider_failure.clone() {
                return Some(TestingFixtureRun::Failure(message));
            }
            let output = state
                .fixture
                .malformed_plan_output
                .clone()
                .unwrap_or_else(|| state.fixture.plan_output.to_string());
            return Some(TestingFixtureRun::Output(output));
        }
        None
    }

    pub fn permission_timeout(&self) -> Duration {
        self.inner
            .permission_timeout
            .lock()
            .expect("test controls permission timeout lock")
            .unwrap_or(Duration::from_secs(900))
    }

    pub async fn set_permission_timeout(&self, timeout: Duration) {
        *self
            .inner
            .permission_timeout
            .lock()
            .expect("test controls permission timeout lock") = Some(timeout);
    }

    pub fn server_idle_timeout(&self) -> Duration {
        self.inner
            .server_idle_timeout
            .lock()
            .expect("test controls server idle timeout lock")
            .unwrap_or(Duration::from_secs(90))
    }

    pub async fn set_server_idle_timeout(&self, timeout: Duration) {
        *self
            .inner
            .server_idle_timeout
            .lock()
            .expect("test controls server idle timeout lock") = Some(timeout);
    }
}

pub struct TestControlledFakeStreamingProvider {
    controls: TestControls,
    fallback: FakeStreamingProvider,
}

impl TestControlledFakeStreamingProvider {
    pub fn new(controls: TestControls) -> Self {
        Self {
            controls,
            fallback: FakeStreamingProvider,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for TestControlledFakeStreamingProvider {
    fn supports_tool_calls(&self) -> bool {
        true
    }

    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        if input.role == AdapterRole::Reviewer
            && let Some(session_id) = input.workspace_session_id.as_deref()
            && let Some(run) = self
                .controls
                .testing_fixture_run(session_id, &input.prompt)
                .await
        {
            return Ok(start_testing_fixture_session(run, cancel));
        }

        if input.role == AdapterRole::Reviewer
            && let Some(session_id) = input.workspace_session_id.as_deref()
            && let Some(fixture) = self.controls.consume_review_fixture(session_id).await
        {
            return Ok(start_review_fixture_session(fixture, cancel));
        }

        let use_fixture = match input.workspace_session_id.as_deref() {
            Some(session_id) => self.controls.consume_permission_fixture(session_id).await,
            None => false,
        };

        if !use_fixture {
            return self.fallback.start(input, cancel).await;
        }

        Ok(start_permission_fixture_session(
            self.controls.permission_timeout(),
            cancel,
        ))
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.fallback.run_streaming(input, cancel).await
    }
}

fn start_review_fixture_session(
    fixture: ReviewFixture,
    cancel: tokio_util::sync::CancellationToken,
) -> ProviderSession {
    let (event_tx, event_rx) = mpsc::channel(8);
    let (command_tx, _command_rx) = mpsc::channel(8);

    tokio::spawn(async move {
        let output = if let Some(raw_text) = fixture.raw_text {
            raw_text
        } else if let Some(raw_json) = fixture.raw_json {
            raw_json.to_string()
        } else {
            let contract = json!({
                "verdict": fixture.verdict,
                "summary": fixture.summary,
                "findings": fixture.findings,
            });
            format!("{}\n\n```json\n{}\n```", fixture.comments, contract)
        };
        if cancel.is_cancelled() {
            return;
        }
        if event_tx
            .send(ProviderEvent::TextDelta {
                content: output.clone(),
            })
            .await
            .is_err()
        {
            return;
        }
        if cancel.is_cancelled() {
            return;
        }
        let _ = event_tx
            .send(ProviderEvent::Completed {
                full_output: output,
                provider_session_id: None,
            })
            .await;
    });

    ProviderSession {
        events: event_rx,
        commands: command_tx,
    }
}

fn start_testing_fixture_session(
    run: TestingFixtureRun,
    cancel: tokio_util::sync::CancellationToken,
) -> ProviderSession {
    let (event_tx, event_rx) = mpsc::channel(8);
    let (command_tx, _command_rx) = mpsc::channel(8);

    tokio::spawn(async move {
        match run {
            TestingFixtureRun::Failure(message) => {
                let _ = event_tx.send(ProviderEvent::Failed { message }).await;
            }
            TestingFixtureRun::Output(output) => {
                if cancel.is_cancelled() {
                    return;
                }
                if event_tx
                    .send(ProviderEvent::TextDelta {
                        content: output.clone(),
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                if cancel.is_cancelled() {
                    return;
                }
                let _ = event_tx
                    .send(ProviderEvent::Completed {
                        full_output: output,
                        provider_session_id: None,
                    })
                    .await;
            }
        }
    });

    ProviderSession {
        events: event_rx,
        commands: command_tx,
    }
}

fn start_permission_fixture_session(
    timeout_after: Duration,
    cancel: tokio_util::sync::CancellationToken,
) -> ProviderSession {
    let (event_tx, event_rx) = mpsc::channel(16);
    let (command_tx, mut command_rx) = mpsc::channel(8);

    tokio::spawn(async move {
        let permission_id = "e2e_permission_1".to_string();
        if event_tx
            .send(ProviderEvent::TextDelta {
                content: "E2E permission fixture stream\n".to_string(),
            })
            .await
            .is_err()
        {
            return;
        }

        if event_tx
            .send(ProviderEvent::PermissionRequest(PermissionRequestData {
                id: permission_id.clone(),
                tool_name: "Bash".to_string(),
                description: "E2E permission fixture request".to_string(),
                risk_level: RiskLevel::Medium,
            }))
            .await
            .is_err()
        {
            return;
        }

        let timeout = tokio::time::sleep(timeout_after);
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = &mut timeout => {
                    let _ = event_tx
                        .send(ProviderEvent::PermissionTimeout {
                            permission_id: permission_id.clone(),
                        })
                        .await;
                    return;
                }
                command = command_rx.recv() => {
                    match command {
                        Some(ProviderCommand::PermissionResponse { id, approved, .. }) if id == permission_id => {
                            if approved {
                                let output = "# Permission Fixture\n\nApproved request completed.\n".to_string();
                                let _ = event_tx
                                    .send(ProviderEvent::TextDelta {
                                        content: output.clone(),
                                    })
                                    .await;
                                let _ = event_tx
                                    .send(ProviderEvent::Completed {
                                        full_output: output,
                                        provider_session_id: None,
                                    })
                                    .await;
                            } else {
                                let _ = event_tx
                                    .send(ProviderEvent::Failed {
                                        message: "permission denied".to_string(),
                                    })
                                    .await;
                            }
                            return;
                        }
                        Some(ProviderCommand::PermissionResponse { id, .. }) => {
                            let _ = event_tx
                                .send(ProviderEvent::ProtocolError {
                                    code: "PERMISSION_ID_UNMATCHED".to_string(),
                                    message: format!("PermissionResponse id={id} not found in pending"),
                                    context: Some(json!({ "permission_id": id })),
                                })
                                .await;
                        }
                        Some(ProviderCommand::ChoiceResponse { .. }) => {}
                        Some(ProviderCommand::ToolResult(_)) => {}
                        Some(ProviderCommand::Abort) | None => return,
                    }
                }
            }
        }
    });

    ProviderSession {
        events: event_rx,
        commands: command_tx,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use serde_json::json;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use crate::cross_cutting::streaming_provider::{
        ProviderCommand, ProviderEvent, ProviderPermissionMode, StreamingProviderAdapter,
        StreamingProviderInput,
    };
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::protocol::contracts::{AdapterRole, ProviderType};

    use super::{
        ReviewFixture, TestControlledFakeStreamingProvider, TestControls, TestingFixture,
        WorkspaceSocketControl, create_large_workspace_fixture, test_controls_enabled,
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_controls_are_disabled_without_e2e_env() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        unsafe {
            std::env::remove_var("ARIA_E2E_TEST_CONTROLS");
        }

        assert!(!test_controls_enabled());
    }

    #[test]
    fn test_controls_are_enabled_in_e2e_env() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        unsafe {
            std::env::set_var("ARIA_E2E_TEST_CONTROLS", "1");
        }

        assert!(test_controls_enabled());

        unsafe {
            std::env::remove_var("ARIA_E2E_TEST_CONTROLS");
        }
    }

    #[test]
    fn large_workspace_fixture_contains_large_lazy_loaded_content() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let app_paths = ProductAppPaths::new(temp_dir.path());

        let session_id = create_large_workspace_fixture(app_paths.clone()).expect("large fixture");

        let lifecycle = LifecycleStore::new(app_paths);
        let nodes = lifecycle
            .load_timeline_nodes(&session_id)
            .expect("timeline nodes");
        let provider_nodes = nodes
            .iter()
            .filter(|node| {
                matches!(
                    node.node_type,
                    crate::web::workspace_ws_types::TimelineNodeType::AuthorRun
                        | crate::web::workspace_ws_types::TimelineNodeType::ReviewerRun
                )
            })
            .count();
        let detail = lifecycle
            .load_node_detail(&session_id, "timeline_node_034")
            .expect("first node detail");
        let output = detail.execution_events[0]
            .get("output")
            .and_then(|value| value.as_str())
            .expect("large output");
        let artifacts = lifecycle
            .list_artifact_versions(&session_id)
            .expect("artifact versions");

        assert_eq!(nodes.len(), 45);
        assert!(provider_nodes >= 10);
        assert!(detail.prompt.expect("large prompt").len() > 100_000);
        assert!(output.len() > 100_000);
        assert_eq!(artifacts.len(), 5);
        assert!(artifacts[4].markdown.contains("# Large Artifact v5"));
    }

    #[tokio::test]
    async fn workspace_socket_drop_notifies_registered_session_connection() {
        let controls = TestControls::default();
        let (tx, mut rx) = mpsc::channel(1);
        controls
            .register_workspace_socket("workspace_session_1".to_string(), tx)
            .await;

        let dropped = controls.drop_workspace_socket("workspace_session_1").await;

        assert!(dropped);
        assert_eq!(
            rx.recv().await,
            Some(WorkspaceSocketControl::CloseForTestDrop)
        );
    }

    #[tokio::test]
    async fn workspace_socket_drop_waits_for_late_session_registration() {
        let controls = TestControls::default();
        let delayed_controls = controls.clone();
        let (tx, mut rx) = mpsc::channel(1);

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            delayed_controls
                .register_workspace_socket("workspace_session_1".to_string(), tx)
                .await;
        });

        let dropped = controls
            .drop_workspace_socket_when_registered(
                "workspace_session_1",
                Duration::from_millis(200),
            )
            .await;

        assert!(dropped);
        assert_eq!(
            rx.recv().await,
            Some(WorkspaceSocketControl::CloseForTestDrop)
        );
    }

    #[tokio::test]
    async fn workspace_socket_rejects_are_consumed_per_session() {
        let controls = TestControls::default();
        controls
            .reject_next_workspace_sockets("workspace_session_1".to_string(), 2)
            .await;

        assert!(
            controls
                .consume_workspace_socket_reject("workspace_session_1")
                .await
        );
        assert!(
            controls
                .consume_workspace_socket_reject("workspace_session_1")
                .await
        );
        assert!(
            !controls
                .consume_workspace_socket_reject("workspace_session_1")
                .await
        );
        assert!(
            !controls
                .consume_workspace_socket_reject("workspace_session_2")
                .await
        );
    }

    #[tokio::test]
    async fn permission_fixture_is_session_scoped_and_consumed_once() {
        let controls = TestControls::default();

        assert!(
            !controls
                .consume_permission_fixture("workspace_session_1")
                .await
        );

        controls
            .enable_permission_fixture("workspace_session_1".to_string())
            .await;

        assert!(
            controls
                .consume_permission_fixture("workspace_session_1")
                .await
        );
        assert!(
            !controls
                .consume_permission_fixture("workspace_session_1")
                .await
        );
        assert!(
            !controls
                .consume_permission_fixture("workspace_session_2")
                .await
        );
    }

    #[tokio::test]
    async fn review_fixture_is_session_scoped_and_consumed_once() {
        let controls = TestControls::default();

        assert!(
            controls
                .consume_review_fixture("workspace_session_1")
                .await
                .is_none()
        );

        controls
            .enable_review_fixture(
                "workspace_session_1".to_string(),
                ReviewFixture {
                    verdict: "revise".to_string(),
                    summary: "补充异常路径".to_string(),
                    comments: "需要补充失败路径。".to_string(),
                    raw_json: None,
                    raw_text: None,
                    findings: Vec::new(),
                },
            )
            .await;

        let fixture = controls
            .consume_review_fixture("workspace_session_1")
            .await
            .expect("review fixture");

        assert_eq!(fixture.verdict, "revise");
        assert_eq!(fixture.summary, "补充异常路径");
        assert!(
            controls
                .consume_review_fixture("workspace_session_1")
                .await
                .is_none()
        );
        assert!(
            controls
                .consume_review_fixture("workspace_session_2")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn permission_timeout_override_defaults_and_updates() {
        let controls = TestControls::default();

        assert_eq!(controls.permission_timeout(), Duration::from_secs(900));

        controls
            .set_permission_timeout(Duration::from_millis(500))
            .await;

        assert_eq!(controls.permission_timeout(), Duration::from_millis(500));
    }

    #[tokio::test]
    async fn server_idle_timeout_override_defaults_and_updates() {
        let controls = TestControls::default();

        assert_eq!(controls.server_idle_timeout(), Duration::from_secs(90));

        controls
            .set_server_idle_timeout(Duration::from_millis(750))
            .await;

        assert_eq!(controls.server_idle_timeout(), Duration::from_millis(750));
    }

    #[tokio::test]
    async fn permission_fixture_fake_provider_waits_for_approval_and_completes() {
        let controls = TestControls::default();
        controls
            .enable_permission_fixture("workspace_session_1".to_string())
            .await;
        let provider = TestControlledFakeStreamingProvider::new(controls);
        let mut session = provider
            .start(
                streaming_input("workspace_session_1"),
                CancellationToken::new(),
            )
            .await
            .expect("provider session");

        match tokio::time::timeout(Duration::from_secs(1), session.events.recv())
            .await
            .expect("stream event")
            .expect("text delta")
        {
            ProviderEvent::TextDelta { content } => {
                assert!(content.contains("E2E permission fixture stream"));
            }
            other => panic!("unexpected provider event: {other:?}"),
        }

        let request_id = match tokio::time::timeout(Duration::from_secs(1), session.events.recv())
            .await
            .expect("permission event")
            .expect("permission request")
        {
            ProviderEvent::PermissionRequest(request) => request.id,
            other => panic!("unexpected provider event: {other:?}"),
        };
        session
            .commands
            .send(ProviderCommand::PermissionResponse {
                id: request_id,
                approved: true,
                reason: None,
            })
            .await
            .expect("send approval");

        for _ in 0..3 {
            match tokio::time::timeout(Duration::from_secs(1), session.events.recv())
                .await
                .expect("completed event")
                .expect("completed")
            {
                ProviderEvent::Completed { full_output, .. } => {
                    assert!(full_output.contains("Permission Fixture"));
                    return;
                }
                ProviderEvent::TextDelta { .. } => {}
                other => panic!("unexpected provider event: {other:?}"),
            }
        }
        panic!("permission fixture did not complete");
    }

    #[tokio::test]
    async fn review_fixture_fake_provider_emits_json_contract_for_reviewer() {
        let controls = TestControls::default();
        controls
            .enable_review_fixture(
                "workspace_session_1".to_string(),
                ReviewFixture {
                    verdict: "revise".to_string(),
                    summary: "补充异常路径".to_string(),
                    comments: "需要补充失败路径。".to_string(),
                    raw_json: None,
                    raw_text: None,
                    findings: Vec::new(),
                },
            )
            .await;
        let provider = TestControlledFakeStreamingProvider::new(controls);
        let mut session = provider
            .start(
                StreamingProviderInput {
                    provider_type: ProviderType::Codex,
                    role: AdapterRole::Reviewer,
                    prompt: "请作为 reviewer 审核当前 Workspace 产物。".to_string(),
                    working_dir: std::env::current_dir().expect("current dir"),
                    workspace_session_id: Some("workspace_session_1".to_string()),
                    resume_provider_session_id: None,
                    permission_mode: ProviderPermissionMode::Supervised,
                    env_vars: Default::default(),
                    timeout_secs: 60,
                },
                CancellationToken::new(),
            )
            .await
            .expect("review fixture provider session");

        let mut output = String::new();
        while let Some(event) = session.events.recv().await {
            match event {
                ProviderEvent::TextDelta { content } => output.push_str(&content),
                ProviderEvent::Completed { full_output, .. } => {
                    output.push_str(&full_output);
                    break;
                }
                _ => {}
            }
        }

        assert!(output.contains("需要补充失败路径。"));
        assert!(output.contains("\"verdict\":\"revise\""));
        assert!(output.contains("\"summary\":\"补充异常路径\""));
    }

    #[tokio::test]
    async fn testing_fixture_fake_provider_emits_plan_and_step_results() {
        let controls = TestControls::default();
        controls
            .enable_testing_fixture(
                "coding_attempt_0001".to_string(),
                TestingFixture {
                    plan_output: json!({
                        "summary": "controlled QA plan",
                        "steps": [
                            {
                                "id": "unit",
                                "title": "Unit tests",
                                "intent": "prove unit behavior",
                                "required": true,
                                "tool": "run_command",
                                "risk_level": "low",
                                "command_or_tool_input": {"command": ["true"]},
                                "evidence_expectation": "exit 0"
                            },
                            {
                                "id": "security",
                                "title": "Security check",
                                "intent": "prove security checklist",
                                "required": true,
                                "tool": "provider_managed",
                                "risk_level": "medium",
                                "command_or_tool_input": {"note": "controlled missing step"},
                                "evidence_expectation": "provider evidence"
                            }
                        ]
                    }),
                    step_results: vec![json!({"step_id": "unit", "status": "passed"})],
                    malformed_plan_output: None,
                    provider_failure: None,
                },
            )
            .await;
        let provider = TestControlledFakeStreamingProvider::new(controls);
        assert!(provider.supports_tool_calls());
        let mut plan_session = provider
            .start(
                StreamingProviderInput {
                    provider_type: ProviderType::Codex,
                    role: AdapterRole::Reviewer,
                    prompt: "plan_tests".to_string(),
                    working_dir: std::env::current_dir().expect("current dir"),
                    workspace_session_id: Some("coding_attempt_0001".to_string()),
                    resume_provider_session_id: None,
                    permission_mode: ProviderPermissionMode::Supervised,
                    env_vars: Default::default(),
                    timeout_secs: 60,
                },
                CancellationToken::new(),
            )
            .await
            .expect("plan fixture provider session");
        let plan_output = completed_output(&mut plan_session).await;
        assert!(plan_output.contains("controlled QA plan"));

        let mut execute_session = provider
            .start(
                StreamingProviderInput {
                    provider_type: ProviderType::Codex,
                    role: AdapterRole::Reviewer,
                    prompt: "Phase: plan_tests -> execute_test_plan\nexecute_test_plan".to_string(),
                    working_dir: std::env::current_dir().expect("current dir"),
                    workspace_session_id: Some("coding_attempt_0001".to_string()),
                    resume_provider_session_id: None,
                    permission_mode: ProviderPermissionMode::Supervised,
                    env_vars: Default::default(),
                    timeout_secs: 60,
                },
                CancellationToken::new(),
            )
            .await
            .expect("execute fixture provider session");
        let execute_output = completed_output(&mut execute_session).await;
        assert!(execute_output.contains("\"step_id\":\"unit\""));
        assert!(execute_output.contains("\"status\":\"passed\""));
    }

    #[tokio::test]
    async fn review_fixture_can_emit_alias_findings_and_malformed_json() {
        let controls = TestControls::default();
        controls
            .enable_review_fixture(
                "coding_attempt_0001".to_string(),
                ReviewFixture {
                    verdict: "request_changes".to_string(),
                    summary: "alias finding".to_string(),
                    comments: String::new(),
                    raw_json: Some(json!({
                        "verdict": "request_changes",
                        "summary": "alias finding",
                        "findings": [{
                            "file": "src/lib.rs",
                            "description": "missing validation",
                            "recommendation": "add validation"
                        }]
                    })),
                    raw_text: None,
                    findings: Vec::new(),
                },
            )
            .await;
        let provider = TestControlledFakeStreamingProvider::new(controls.clone());
        let mut session = provider
            .start(
                StreamingProviderInput {
                    provider_type: ProviderType::Codex,
                    role: AdapterRole::Reviewer,
                    prompt: "code review".to_string(),
                    working_dir: std::env::current_dir().expect("current dir"),
                    workspace_session_id: Some("coding_attempt_0001".to_string()),
                    resume_provider_session_id: None,
                    permission_mode: ProviderPermissionMode::Supervised,
                    env_vars: Default::default(),
                    timeout_secs: 60,
                },
                CancellationToken::new(),
            )
            .await
            .expect("review fixture provider session");
        let output = completed_output(&mut session).await;
        assert!(output.contains("\"file\":\"src/lib.rs\""));
        assert!(output.contains("\"description\":\"missing validation\""));
        assert!(output.contains("\"recommendation\":\"add validation\""));

        controls
            .enable_review_fixture(
                "coding_attempt_0001".to_string(),
                ReviewFixture {
                    verdict: "blocked".to_string(),
                    summary: "malformed".to_string(),
                    comments: String::new(),
                    raw_json: None,
                    raw_text: Some("not json at all".to_string()),
                    findings: Vec::new(),
                },
            )
            .await;
        let provider = TestControlledFakeStreamingProvider::new(controls);
        let mut malformed_session = provider
            .start(
                StreamingProviderInput {
                    provider_type: ProviderType::Codex,
                    role: AdapterRole::Reviewer,
                    prompt: "code review".to_string(),
                    working_dir: std::env::current_dir().expect("current dir"),
                    workspace_session_id: Some("coding_attempt_0001".to_string()),
                    resume_provider_session_id: None,
                    permission_mode: ProviderPermissionMode::Supervised,
                    env_vars: Default::default(),
                    timeout_secs: 60,
                },
                CancellationToken::new(),
            )
            .await
            .expect("malformed review fixture provider session");
        assert_eq!(
            completed_output(&mut malformed_session).await,
            "not json at all"
        );
    }

    #[tokio::test]
    async fn review_fixture_provider_consumes_queued_outputs_in_order() {
        let controls = TestControls::default();
        controls
            .enable_review_fixture(
                "coding_attempt_0001".to_string(),
                ReviewFixture {
                    verdict: "no_issue".to_string(),
                    summary: "analyst pass".to_string(),
                    comments: String::new(),
                    raw_json: Some(json!({
                        "verdict": "no_issue",
                        "summary": "analyst pass"
                    })),
                    raw_text: None,
                    findings: Vec::new(),
                },
            )
            .await;
        controls
            .enable_review_fixture(
                "coding_attempt_0001".to_string(),
                ReviewFixture {
                    verdict: "request_changes".to_string(),
                    summary: "review alias".to_string(),
                    comments: String::new(),
                    raw_json: Some(json!({
                        "verdict": "request_changes",
                        "summary": "review alias",
                        "findings": [{
                            "file": "src/lib.rs",
                            "description": "missing validation",
                            "recommendation": "add validation"
                        }]
                    })),
                    raw_text: None,
                    findings: Vec::new(),
                },
            )
            .await;

        let provider = TestControlledFakeStreamingProvider::new(controls);
        let mut analyst_session = provider
            .start(
                StreamingProviderInput {
                    provider_type: ProviderType::Codex,
                    role: AdapterRole::Reviewer,
                    prompt: "analyst".to_string(),
                    working_dir: std::env::current_dir().expect("current dir"),
                    workspace_session_id: Some("coding_attempt_0001".to_string()),
                    resume_provider_session_id: None,
                    permission_mode: ProviderPermissionMode::Supervised,
                    env_vars: Default::default(),
                    timeout_secs: 60,
                },
                CancellationToken::new(),
            )
            .await
            .expect("analyst fixture provider session");
        assert!(
            completed_output(&mut analyst_session)
                .await
                .contains("\"verdict\":\"no_issue\"")
        );

        let mut review_session = provider
            .start(
                StreamingProviderInput {
                    provider_type: ProviderType::Codex,
                    role: AdapterRole::Reviewer,
                    prompt: "code review".to_string(),
                    working_dir: std::env::current_dir().expect("current dir"),
                    workspace_session_id: Some("coding_attempt_0001".to_string()),
                    resume_provider_session_id: None,
                    permission_mode: ProviderPermissionMode::Supervised,
                    env_vars: Default::default(),
                    timeout_secs: 60,
                },
                CancellationToken::new(),
            )
            .await
            .expect("review fixture provider session");
        let review_output = completed_output(&mut review_session).await;
        assert!(review_output.contains("\"verdict\":\"request_changes\""));
        assert!(review_output.contains("\"file\":\"src/lib.rs\""));
    }

    async fn completed_output(
        session: &mut crate::cross_cutting::streaming_provider::ProviderSession,
    ) -> String {
        while let Some(event) = session.events.recv().await {
            match event {
                ProviderEvent::Completed { full_output, .. } => return full_output,
                ProviderEvent::TextDelta { .. } => {}
                other => panic!("unexpected provider event: {other:?}"),
            }
        }
        panic!("provider session ended without completion")
    }

    #[tokio::test]
    async fn permission_fixture_fake_provider_emits_timeout_when_unanswered() {
        let controls = TestControls::default();
        controls
            .enable_permission_fixture("workspace_session_1".to_string())
            .await;
        controls
            .set_permission_timeout(Duration::from_millis(20))
            .await;
        let provider = TestControlledFakeStreamingProvider::new(controls);
        let mut session = provider
            .start(
                streaming_input("workspace_session_1"),
                CancellationToken::new(),
            )
            .await
            .expect("provider session");

        match tokio::time::timeout(Duration::from_secs(1), session.events.recv())
            .await
            .expect("stream event")
            .expect("text delta")
        {
            ProviderEvent::TextDelta { content } => {
                assert!(content.contains("E2E permission fixture stream"));
            }
            other => panic!("unexpected provider event: {other:?}"),
        }

        let request_id = match tokio::time::timeout(Duration::from_secs(1), session.events.recv())
            .await
            .expect("permission event")
            .expect("permission request")
        {
            ProviderEvent::PermissionRequest(request) => request.id,
            other => panic!("unexpected provider event: {other:?}"),
        };

        match tokio::time::timeout(Duration::from_secs(1), session.events.recv())
            .await
            .expect("timeout event")
            .expect("permission timeout")
        {
            ProviderEvent::PermissionTimeout { permission_id } => {
                assert_eq!(permission_id, request_id)
            }
            other => panic!("unexpected provider event: {other:?}"),
        }
    }

    fn streaming_input(session_id: &str) -> StreamingProviderInput {
        StreamingProviderInput {
            provider_type: ProviderType::Fake,
            role: AdapterRole::Orchestrator,
            prompt: "生成测试产物".to_string(),
            working_dir: std::env::current_dir().expect("current dir"),
            workspace_session_id: Some(session_id.to_string()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: Default::default(),
            timeout_secs: 60,
        }
    }
}
