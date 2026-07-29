// change `fix-process-evidence-as-acceptance`：过程性验收标准检出为 Warning 级，
// 可见于候选的 validator_findings，但不阻断候选接受。
// 独立成文件以保持 part_02.rs 在 large_file_guard 的 800 行上限内。

#[tokio::test]
async fn drafting_with_process_evidence_warning_keeps_candidate_acceptable_and_visible() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_process_evidence_warning");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
    let store = engine.work_item_plan_store().expect("work item plan store");
    let mut index = store
        .load_active_index(
            &engine.session.project_id,
            &engine.session.issue_id,
            &engine.session.entity_id,
        )
        .expect("load active index")
        .expect("active index");
    index.outline_to_current_draft_id.clear();
    store.save_active_index(&index).expect("save active index");
    engine.session.stage = WorkspaceStage::Running;
    let run_node_id = engine
        .create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemDraftRun,
            agent: Some(ProviderName::Codex),
            stage: WorkspaceStage::Running,
            round: None,
            title: "Work Item Draft".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await;
    engine.active_node_id = Some(run_node_id);
    let mut candidate = test_work_item_draft_record(
        &plan_id,
        "outline_a",
        "draft_process_evidence",
        WorkItemDraftStatus::Draft,
        WorkItemGenerationMode::Serial,
        None,
    )
    .candidate;
    candidate.canonical_contract_candidate.identity.title = "A".to_string();
    candidate.canonical_contract_candidate.write_policy.exclusive_scopes = vec!["src/a.rs".to_string()];
    candidate.canonical_contract_candidate.verification_checks[0].command = None;
    candidate.canonical_contract_candidate.verification_checks[0].required = false;
    candidate.canonical_contract_candidate.verification_checks[0].non_zero_test_execution_required = false;
    candidate.canonical_contract_candidate.blocker_rules[0].route =
        crate::product::work_item_contract::BlockerRoute::OperationalGate;
    candidate.canonical_contract_candidate.acceptance_criteria[0].criterion_id =
        "ac_tdd_red_evidence".to_string();
    candidate.canonical_contract_candidate.acceptance_criteria[0].statement =
        "先失败的测试提交必须存在".to_string();
    candidate.canonical_contract_candidate.tasks[0].done_when_refs =
        vec!["ac_tdd_red_evidence".to_string()];
    candidate
        .canonical_contract_candidate
        .handoff_contract
        .reviewer_check_refs = vec!["ac_tdd_red_evidence".to_string()];
    candidate.verification_plan.checks = candidate
        .canonical_contract_candidate
        .verification_checks
        .clone();

    let outcome = engine
        .complete_work_item_draft_author(candidate, None)
        .await
        .expect("warning-only draft must complete authoring");

    assert_eq!(outcome, WorkItemDraftAuthorOutcome::AwaitConfirmation);
    let ArtifactPayload::WorkItemDraftCandidate { draft_candidate } = engine
        .session()
        .artifact
        .as_ref()
        .expect("draft artifact")
    else {
        panic!("expected work item draft artifact");
    };
    assert!(draft_candidate.can_accept);
    let finding = draft_candidate
        .validator_findings
        .iter()
        .find(|finding| finding.code == "process_evidence_acceptance_criterion")
        .expect("process-evidence warning must remain visible in the draft payload");
    assert_eq!(finding.severity, "warning");
    assert!(
        finding
            .work_item_ids
            .iter()
            .any(|work_item_id| work_item_id == "ac_tdd_red_evidence"),
        "visible warning must preserve the acceptance criterion reference"
    );
}
