use std::sync::atomic::{AtomicU64, Ordering};

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::json;

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
use crate::web::state::WebAppState;
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ProviderConfigSnapshot, ReviewVerdictType, TimelineNode,
    TimelineNodeStatus, TimelineNodeType, WorkspaceStage, WsExecutionEventKind,
    WsExecutionEventStatus,
};

use super::git::init_git_repo;

pub async fn seed_large_workspace_fixture(
    State(state): State<WebAppState>,
) -> Json<serde_json::Value> {
    match create_large_workspace_fixture(ProductAppPaths::new(state.workspace_root.join(".aria"))) {
        Ok(session_id) => Json(json!({"session_id": session_id})),
        Err(error) => Json(json!({"error": error.to_string()})),
    }
}

pub(super) fn create_large_workspace_fixture(
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
            retry: None,
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
            payload: ArtifactPayload::Markdown {
                markdown: format!(
                    "{}\n# Large Artifact v{version}\n\n{}",
                    "artifact line\n".repeat(220),
                    "artifact line\n".repeat(8780)
                ),
                diff: None,
            },
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
            ..Default::default()
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
