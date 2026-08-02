use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::mpsc;

use super::{ISSUE_ID, MatrixStreamingProvider, PROJECT_ID};
use crate::cross_cutting::streaming_provider::{ProviderCommand, StreamingProviderAdapter};
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
use crate::product::models::{
    ProviderName, TrustedDraftVerificationCommand, WorkItemKind, WorkItemOutline,
    WorkItemOutlineSessionFit, WorkItemPlanDraftActiveIndex, WorkItemPlanOutline, WorkspaceType,
};
use crate::product::work_item_contract::CanonicalWorkItemContract;
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::product::work_item_split_engine::parse::parse_work_item_draft_output;
use crate::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};
use crate::web::test_controls::plan_repair::PlanRepairFixtureError;
use crate::web::test_controls::plan_repair::recovery::fixture_error;
use crate::web::test_controls::plan_repair::seed::fixture_paths;
use crate::web::workspace_ws_handler::parse_work_item_split_structured_output;
use crate::web::workspace_ws_types::{
    ArtifactPayload, WorkItemGenerationModeDto, WorkItemPlanOutlineCandidateDto,
    WorkItemPlanReviewVerdict,
};

pub(super) struct WorkspaceMatrixOutcome {
    pub author_contract_ids: Vec<String>,
    pub plan_review_passed: bool,
    pub author_draft_artifact_persisted: bool,
    pub plan_review_complete_event_observed: bool,
}

pub(super) async fn run_workspace_provider_roles(
    root: &Path,
    provider: ProviderName,
    adapter: Arc<MatrixStreamingProvider>,
    author_contract: &CanonicalWorkItemContract,
) -> Result<WorkspaceMatrixOutcome, PlanRepairFixtureError> {
    let paths = fixture_paths(root);
    let lifecycle = LifecycleStore::new(paths.clone());
    let session_id = format!(
        "workspace_session_provider_matrix_{}",
        provider_slug(&provider)
    );
    let record = lifecycle
        .create_workspace_session_with_id(
            CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: "work_item_plan_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: provider.clone(),
                reviewer_provider: provider.clone(),
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            },
            session_id.clone(),
        )
        .map_err(fixture_error)?;
    let checkpoint_store = Arc::new(CheckpointStore::new(
        paths.issue_lifecycle_root(PROJECT_ID, ISSUE_ID),
    ));
    let (event_tx, mut event_rx) = mpsc::channel(128);
    let mut session = WorkspaceSession::from_record(record);
    session.repository_path = Some(root.join("worktree"));
    let mut engine =
        WorkspaceEngine::new_persistent(checkpoint_store, lifecycle.clone(), event_tx, session);
    engine
        .update_artifact(outline_artifact(author_contract))
        .await;
    WorkItemPlanStore::new(paths)
        .save_active_index(&WorkItemPlanDraftActiveIndex {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_generation_round_id: "provider_matrix_round_0001".to_string(),
            outline_state: "confirmed".to_string(),
            active_outline_id: Some("outline_core".to_string()),
            outline_to_current_draft_id: BTreeMap::new(),
            draft_statuses: BTreeMap::new(),
            batches: Vec::new(),
            updated_at: "2026-07-20T00:01:00Z".to_string(),
        })
        .map_err(fixture_error)?;

    engine
        .start_serial_work_item_draft_run_for("outline_core")
        .await
        .map_err(fixture_error)?;
    let node_id = engine
        .active_timeline_node_id()
        .ok_or_else(|| fixture_error("work item author node is missing"))?;
    let author_input = engine
        .build_current_work_item_draft_streaming_input(None)
        .map_err(fixture_error)?;
    engine
        .emit_provider_prompt_event(
            &node_id,
            author_input.prompt.clone(),
            "Provider matrix Work Item Author prompt",
            Some(provider.clone()),
        )
        .await;
    let author_session = adapter.start(author_input, engine.cancel.clone()).await;
    let (_author_command_tx, mut author_command_rx) = mpsc::channel::<ProviderCommand>(1);
    let author_output = engine
        .drive_work_item_plan_provider_session_to_output(
            author_session,
            &mut author_command_rx,
            node_id,
            provider.clone(),
        )
        .await
        .map_err(fixture_error)?;
    let structured =
        parse_work_item_split_structured_output(&author_output).map_err(fixture_error)?;
    let candidate = parse_work_item_draft_output(structured).map_err(fixture_error)?;
    let author_contract_ids = candidate
        .canonical_contract_candidate
        .output_contracts
        .iter()
        .map(|contract| contract.contract_id.clone())
        .collect();
    engine
        .complete_work_item_draft_author(candidate, None)
        .await
        .map_err(fixture_error)?;

    let author_draft_artifact_persisted = lifecycle
        .list_artifact_versions(&session_id)
        .map_err(fixture_error)?
        .iter()
        .any(|version| {
            version.is_current
                && matches!(
                    version.payload,
                    ArtifactPayload::WorkItemDraftCandidate { .. }
                )
        });

    engine
        .begin_work_item_draft_review_run("outline_core")
        .await;
    let (_review_command_tx, review_command_rx) = mpsc::channel::<ProviderCommand>(1);
    engine
        .drive_review_session(adapter, review_command_rx)
        .await;

    let mut plan_review_passed = false;
    let mut plan_review_complete_event_observed = false;
    while let Ok(event) = event_rx.try_recv() {
        if let EngineEvent::ReviewComplete {
            work_item_plan_review: Some(review),
            ..
        } = event
        {
            plan_review_complete_event_observed = true;
            plan_review_passed = review.verdict == WorkItemPlanReviewVerdict::Pass;
        }
    }

    Ok(WorkspaceMatrixOutcome {
        author_contract_ids,
        plan_review_passed,
        author_draft_artifact_persisted,
        plan_review_complete_event_observed,
    })
}

fn outline_artifact(contract: &CanonicalWorkItemContract) -> ArtifactPayload {
    ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: WorkItemPlanOutline {
                id: "provider_matrix_outline_0001".to_string(),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                source_story_spec_ids: vec!["story_spec_0001".to_string()],
                source_design_spec_ids: vec!["design_spec_0001".to_string()],
                strategy_summary: "validate production provider runners".to_string(),
                work_item_outlines: vec![WorkItemOutline {
                    outline_id: "outline_core".to_string(),
                    logical_work_item_id: contract.identity.logical_work_item_id.clone(),
                    title: contract.identity.title.clone(),
                    kind: WorkItemKind::Backend,
                    goal: contract.goal.summary.clone(),
                    scope: vec!["src/core.rs".to_string()],
                    non_goals: Vec::new(),
                    estimated_context_tokens: Some(8_000),
                    session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
                    source_story_spec_ids: vec!["story_spec_0001".to_string()],
                    source_design_spec_ids: vec!["design_spec_0001".to_string()],
                    exclusive_write_scopes: vec!["src/core.rs".to_string()],
                    forbidden_write_scopes: Vec::new(),
                    depends_on: Vec::new(),
                    verification_intent: vec!["cargo test --locked --lib core".to_string()],
                    trusted_verification_commands: vec![TrustedDraftVerificationCommand {
                        command: "cargo test --locked --lib core".to_string(),
                        cwd: ".".to_string(),
                        purpose: "验证核心工作流契约".to_string(),
                        source_ref: "provider_matrix#trusted_command".to_string(),
                    }],
                    handoff_notes: "publish finalization contract".to_string(),
                }],
                dependency_graph: Vec::new(),
                risks: Vec::new(),
                handoff_strategy: "typed contract handoff".to_string(),
                status: "confirmed".to_string(),
            },
            design_context_gaps: Vec::new(),
            validator_findings: Vec::new(),
            context_blockers: Vec::new(),
            current_generation_round_id: Some("provider_matrix_round_0001".to_string()),
            selected_generation_mode: Some(WorkItemGenerationModeDto::Serial),
        }),
    }
}

fn provider_slug(provider: &ProviderName) -> &'static str {
    match provider {
        ProviderName::Codex => "codex",
        ProviderName::ClaudeCode => "claude_code",
        ProviderName::Pi => "pi",
        ProviderName::Fake => "fake",
    }
}
