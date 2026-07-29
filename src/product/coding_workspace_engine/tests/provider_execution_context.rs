use super::*;
use crate::product::coding_models::{CodingUnitRun, CodingUnitRunStatus};
use crate::product::models::HandoffRevision;
use crate::product::work_item_projection::{
    CoderExecutionEnvelope, ReviewerExecutionEnvelope, renderer_for,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(super) struct CapturingProjectionProvider {
    output: String,
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
}

impl CapturingProjectionProvider {
    pub(super) fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            inputs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn input(&self) -> StreamingProviderInput {
        self.inputs.lock().unwrap()[0].clone()
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CapturingProjectionProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().unwrap().push(input);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        let output = self.output.clone();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        output, None,
                    ),
                ))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[tokio::test]
async fn coding_unit_run_provider_execution_context_binds_authoritative_coder_and_reviewer_envelopes()
 {
    let root = tempdir().unwrap();
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).unwrap();
    init_test_git_repo(&worktree);
    let head = git_stdout(&worktree, &["rev-parse", "HEAD"]);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: head.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .unwrap();
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let mut attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .unwrap();
    attempt.head_commit = Some(head.clone());
    attempt.stage = CodingExecutionStage::Coding;
    store.save_coding_attempt(&attempt).unwrap();
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let coder = CapturingProjectionProvider::new(current_plan_defect_output());

    let coded = engine
        .execute_coding(&attempt, &coder, &CodingExecutionContext::default())
        .await
        .unwrap();
    let run = store
        .get_active_unit_run(&coded)
        .expect("materialized unit run");
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &run.projection_bundle_id)
        .unwrap();
    let expected_coder = renderer_for(&ProviderName::Codex)
        .render_coder(
            &bundle.coder_projection,
            &CoderExecutionEnvelope {
                repository_state_ref: head.clone(),
                resolved_handoff_revision_ids: Vec::new(),
                unit_run_id: run.id.clone(),
                previous_actionable_review: None,
                start_commit: Some(head.clone()),
            },
        )
        .unwrap();
    assert_eq!(coder.input().prompt, expected_coder.text);
    assert_eq!(
        run.coder_provider_renderer_version,
        expected_coder.renderer_version
    );
    assert_eq!(
        run.coder_execution_context_hash.as_deref(),
        Some(expected_coder.content_hash.as_str())
    );
    let coder_entry = store
        .list_chat_entries(&coded.project_id, &coded.issue_id, &coded.id)
        .unwrap()
        .into_iter()
        .find(|entry| entry.role == CodingAgentRole::Author)
        .expect("coder chat entry");
    assert_eq!(
        coder_entry.metadata.as_ref().unwrap()["plan_defect_route"],
        "start_plan_repair"
    );
    assert_eq!(coded.rework_count, 0);
    assert_eq!(run.unit_rework_count, 0);
    assert_eq!(run.verification_retry_count, 0);
    assert_eq!(run.operational_retry_count, 0);
    assert_eq!(run.plan_repair_count, 0);

    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").unwrap();
    let reviewer = CapturingProjectionProvider::new(review_plan_defect_output());
    let report = engine.execute_code_review(&coded, &reviewer).await.unwrap();
    assert_eq!(report.verdict, ReviewVerdict::Blocked);
    assert_eq!(
        report.findings[0].plan_defect_evidence[0].kind,
        "test_execution"
    );
    assert_eq!(
        report.findings[0].plan_defect_evidence[0].source_ref,
        "provider-managed-unit.log"
    );
    assert_eq!(
        code_review_flow_decision(&report, &bundle.reviewer_projection),
        CodeReviewFlowDecision::StartPlanRepair
    );
    let review_entry = store
        .list_chat_entries(&coded.project_id, &coded.issue_id, &coded.id)
        .unwrap()
        .into_iter()
        .find(|entry| {
            entry.metadata.as_ref().is_some_and(|metadata| {
                metadata.get("source").and_then(|value| value.as_str()) == Some("code_review")
            })
        })
        .expect("code review chat entry");
    assert_eq!(
        review_entry.metadata.as_ref().unwrap()["plan_defect_source"],
        "code_reviewer"
    );
    assert_eq!(
        review_entry.metadata.as_ref().unwrap()["plan_defect_route"],
        "start_plan_repair"
    );
    let rebound = store.get_active_unit_run(&coded).unwrap();
    let expected_reviewer = renderer_for(&ProviderName::ClaudeCode)
        .render_reviewer(
            &bundle.reviewer_projection,
            &ReviewerExecutionEnvelope {
                unit_run_id: rebound.id.clone(),
                diff_ref: format!("{}..worktree", head),
                handoff_revision_ids: Vec::new(),
                contract_delta_refs: Vec::new(),
                completion_commit: head,
            },
        )
        .unwrap();
    assert_eq!(reviewer.input().prompt, expected_reviewer.text);
    assert_eq!(
        rebound.reviewer_provider_renderer_version,
        expected_reviewer.renderer_version
    );
    assert_eq!(
        rebound.reviewer_execution_context_hash.as_deref(),
        Some(expected_reviewer.content_hash.as_str())
    );
}

#[tokio::test]
async fn coding_plan_repair_group_final_reviewer_uses_all_authoritative_unit_contexts_in_stable_order()
 {
    let root = tempdir().unwrap();
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).unwrap();
    init_test_git_repo(&worktree);
    let head = git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: head.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .unwrap();
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .unwrap();
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::ReviewRequest,
        )
        .unwrap();
    let attempt = store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some(head.clone()),
        )
        .unwrap();
    store
        .save_review_request(
            &attempt,
            &ReviewRequest {
                id: "review_request_0001".to_string(),
                attempt_id: attempt.id.clone(),
                kind: ReviewRequestKind::GitBranchOnly,
                remote_kind: RemoteKind::GenericGit,
                remote: "origin".to_string(),
                base_branch: attempt.base_branch.clone(),
                branch_name: attempt.branch_name.clone(),
                commit_sha: head.clone(),
                push_status: PushStatus::Pushed,
                external_url: None,
                manual_instructions: Vec::new(),
                created_at: "2026-07-19T00:00:00Z".to_string(),
                updated_at: "2026-07-19T00:00:00Z".to_string(),
            },
        )
        .unwrap();

    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_plan_0001",
        )
        .unwrap();
    let renderer_version = renderer_for(&ProviderName::Fake)
        .renderer_version()
        .to_string();
    let mut units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    units.sort_by_key(|unit| unit.order_index);
    for (index, unit) in units.iter().enumerate() {
        store
            .update_coding_unit_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                CodingExecutionUnitStatus::Completed,
                Some("completed".to_string()),
            )
            .unwrap();
        store
            .update_coding_unit_completion_commit(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                Some(head.clone()),
            )
            .unwrap();
        let revision = revision_store
            .get_work_item_revision(
                &lineage,
                &unit.logical_work_item_id,
                &unit.work_item_revision_id,
            )
            .unwrap();
        let bundle = revision_store
            .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
            .unwrap();
        store
            .create_coding_unit_run(
                &attempt,
                &CodingUnitRun {
                    id: format!("coding_unit_run_{:04}", units.len() - index),
                    unit_id: unit.id.clone(),
                    execution_no: 1,
                    work_item_revision_id: revision.id,
                    resolved_handoff_revision_ids: Vec::new(),
                    canonical_contract_hash: bundle.canonical_contract_hash,
                    projection_bundle_id: bundle.id,
                    projection_compiler_version: bundle.compiler_version,
                    coder_provider_renderer_version: renderer_version.clone(),
                    reviewer_provider_renderer_version: renderer_version.clone(),
                    internal_reviewer_provider_renderer_version: None,
                    coder_projection_hash: bundle.coder_projection_hash,
                    reviewer_projection_hash: bundle.reviewer_projection_hash,
                    coder_execution_context_hash: None,
                    reviewer_execution_context_hash: None,
                    internal_reviewer_execution_context_hash: None,
                    status: CodingUnitRunStatus::Completed,
                    unit_rework_count: 0,
                    verification_retry_count: 0,
                    operational_retry_count: 0,
                    plan_repair_count: 0,
                    start_commit: Some(head.clone()),
                    completion_commit: Some(head.clone()),
                    created_at: "2026-07-19T00:00:00Z".to_string(),
                    updated_at: "2026-07-19T00:00:00Z".to_string(),
                },
            )
            .unwrap();
    }

    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = CapturingProjectionProvider::new(
        serde_json::json!({
            "verdict": "approve",
            "summary": "group review ok",
            "findings": [],
            "impact_scope": [],
            "pr_description": "group changes",
            "commit_message_suggestion": "feat: group changes"
        })
        .to_string(),
    );

    engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .unwrap();

    let prompt = provider.input().prompt;
    assert!(prompt.contains("[cadence_project_rules]"));
    assert!(prompt.contains("AGENTS.md"));
    assert!(prompt.contains("CLAUDE.md"));
    assert!(!prompt.contains(&["Cadence-", "skills/"].concat()));
    assert!(prompt.contains("requesting-code-review"));
    assert!(prompt.contains("source_stage=group_final_review"));
    assert!(prompt.contains("\"schema_v2_group_final_review\": true"));
    assert!(prompt.contains("只输出 JSON"));
    let runs_before_retry = units
        .iter()
        .map(|unit| {
            store
                .list_coding_unit_runs(&attempt, &unit.id)
                .unwrap()
                .into_iter()
                .next()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let retry_context = engine
        .prepare_group_reviewer_context(&attempt, &ProviderName::Fake)
        .unwrap();
    let runs_after_retry = units
        .iter()
        .map(|unit| {
            store
                .list_coding_unit_runs(&attempt, &unit.id)
                .unwrap()
                .into_iter()
                .next()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(prompt.starts_with(&retry_context.prompt_section));
    assert_eq!(runs_after_retry, runs_before_retry);

    let mut previous_position = None;
    for unit in &units {
        let run = store
            .list_coding_unit_runs(&attempt, &unit.id)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let revision = revision_store
            .get_work_item_revision(
                &lineage,
                &unit.logical_work_item_id,
                &run.work_item_revision_id,
            )
            .unwrap();
        let bundle = revision_store
            .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
            .unwrap();
        let expected = renderer_for(&ProviderName::Fake)
            .render_reviewer(
                &bundle.reviewer_projection,
                &ReviewerExecutionEnvelope {
                    unit_run_id: run.id.clone(),
                    diff_ref: format!("{head}..{head}"),
                    handoff_revision_ids: Vec::new(),
                    contract_delta_refs: Vec::new(),
                    completion_commit: head.clone(),
                },
            )
            .unwrap();
        let position = prompt
            .find(&expected.text)
            .expect("rendered reviewer context");
        assert!(previous_position.is_none_or(|previous| previous < position));
        previous_position = Some(position);
        assert_eq!(
            run.internal_reviewer_provider_renderer_version.as_deref(),
            Some(expected.renderer_version.as_str())
        );
        assert_eq!(
            run.internal_reviewer_execution_context_hash.as_deref(),
            Some(expected.content_hash.as_str())
        );
    }
}

pub(super) fn current_plan_defect_finding() -> serde_json::Value {
    serde_json::json!({
        "finding_id": "tester_finding_0001",
        "severity": "error",
        "defect_class": "current_work_item_invalid",
        "reason_code": "current_work_item_contract_invalid",
        "message": "the current work item contract is not implementable",
        "contract_refs": [],
        "capability_refs": [],
        "repair_target": {
            "kind": "current_work_item",
            "logical_work_item_ids": ["work_item_0001"],
            "work_item_revision_ids": ["work_item_revision_0001"]
        },
        "recommended_route": "plan_repair",
        "confidence": "high",
        "evidence": [{
            "kind": "test_execution",
            "source_ref": "provider-managed-unit.log",
            "message": "the current contract cannot be exercised"
        }]
    })
}

fn current_plan_defect_output() -> String {
    serde_json::json!({
        "plan_defect_findings": [current_plan_defect_finding()]
    })
    .to_string()
}

pub(super) fn review_plan_defect_output() -> String {
    serde_json::json!({
        "verdict": "blocked",
        "summary": "current work item invalid",
        "findings": [current_plan_defect_finding()]
    })
    .to_string()
}

#[tokio::test]
async fn coding_unit_run_provider_execution_context_dependency_handoff_mismatch_fails_closed() {
    let root = tempdir().unwrap();
    let worktree = root.path().join("worktree");
    fs::create_dir_all(&worktree).unwrap();
    init_test_git_repo(&worktree);
    let head = git_stdout(&worktree, &["rev-parse", "HEAD"]);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: head.clone(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .unwrap();
    seed_group_attempt_fixture(&store, &attempt, true, true);
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    let first = &units[0];
    let second = &units[1];
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    let handoff = HandoffRevision {
        id: "handoff_revision_0001".to_string(),
        logical_work_item_id: first.logical_work_item_id.clone(),
        work_item_revision_id: first.work_item_revision_id.clone(),
        coding_unit_run_id: "coding_unit_run_missing".to_string(),
        provided_contracts: Vec::new(),
        provided_capabilities: BTreeMap::new(),
        contract_hash: "contract_hash".to_string(),
        commit_sha: head.clone(),
        created_at: "2026-07-18T00:00:00Z".to_string(),
    };
    revision_store
        .put_handoff_revision(&lineage, &handoff)
        .unwrap();
    store
        .update_coding_unit_latest_handoff_revision_id(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &first.id,
            Some(handoff.id),
        )
        .unwrap();
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &first.id,
            CodingExecutionUnitStatus::Completed,
            None,
        )
        .unwrap();
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &second.id,
            CodingExecutionUnitStatus::Running,
            None,
        )
        .unwrap();
    let mut attempt = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .unwrap();
    attempt.status = CodingAttemptStatus::Running;
    attempt.stage = CodingExecutionStage::Coding;
    attempt.current_work_item_id = Some(second.logical_work_item_id.clone());
    attempt.active_unit_id = Some(second.id.clone());
    attempt.head_commit = Some(head);
    store.save_coding_attempt(&attempt).unwrap();
    let (tx, _rx) = mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let provider = CapturingProjectionProvider::new("must not run");

    let error = engine
        .execute_coding(&attempt, &provider, &CodingExecutionContext::default())
        .await
        .expect_err("forged dependency handoff must fail closed");

    assert!(
        error
            .to_string()
            .contains("unit_run_handoff_binding_mismatch")
    );
    assert!(provider.inputs.lock().unwrap().is_empty());
}
