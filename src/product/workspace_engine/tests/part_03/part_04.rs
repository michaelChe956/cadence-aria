#[test]
fn final_compile_projects_plan_dependency_graph_from_accepted_drafts() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_compile_draft_edges");
    engine.session.artifact = Some(ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: test_work_item_plan_outline(vec![
                WorkItemOutlineDependencyEdge {
                    from_outline_id: "outline_a".to_string(),
                    to_outline_id: "outline_b".to_string(),
                },
                WorkItemOutlineDependencyEdge {
                    from_outline_id: "outline_b".to_string(),
                    to_outline_id: "outline_c".to_string(),
                },
            ]),
            design_context_gaps: vec![],
            validator_findings: vec![],
            context_blockers: vec![],
            current_generation_round_id: Some("round_0001".to_string()),
            selected_generation_mode: Some(WorkItemGenerationModeDto::Serial),
        }),
    });
    let previous_plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("load previous plan");
    let draft_a = test_work_item_draft_record(
        &plan_id,
        "outline_a",
        "draft_a",
        WorkItemDraftStatus::Accepted,
        WorkItemGenerationMode::Serial,
        None,
    );
    let mut draft_b = test_work_item_draft_record(
        &plan_id,
        "outline_b",
        "draft_b",
        WorkItemDraftStatus::Accepted,
        WorkItemGenerationMode::Serial,
        None,
    );
    draft_b.candidate.depends_on_outline_ids = vec!["outline_a".to_string()];
    let mut draft_c = test_work_item_draft_record(
        &plan_id,
        "outline_c",
        "draft_c",
        WorkItemDraftStatus::Accepted,
        WorkItemGenerationMode::Serial,
        None,
    );
    draft_c.candidate.depends_on_outline_ids =
        vec!["outline_a".to_string(), "outline_b".to_string()];

    let (compiled_plan, work_items, _) = engine
        .project_work_item_plan_drafts_for_compile(
            &previous_plan,
            &[draft_a, draft_b, draft_c],
            WorkItemPlanCompileProjectionContext {
                outline_order: &[
                    "outline_a".to_string(),
                    "outline_b".to_string(),
                    "outline_c".to_string(),
                ],
                outline_to_work_item_id: &BTreeMap::from([
                    ("outline_a".to_string(), "work_item_a".to_string()),
                    ("outline_b".to_string(), "work_item_b".to_string()),
                    ("outline_c".to_string(), "work_item_c".to_string()),
                ]),
                outline_to_verification_plan_id: &BTreeMap::from([
                    ("outline_a".to_string(), "verification_plan_a".to_string()),
                    ("outline_b".to_string(), "verification_plan_b".to_string()),
                    ("outline_c".to_string(), "verification_plan_c".to_string()),
                ]),
                repository_id: "repo_0001",
                now: "2026-06-27T00:00:00Z",
            },
        )
        .expect("project compile records");

    let derived_edges = work_items
        .iter()
        .flat_map(|item| {
            item.depends_on
                .iter()
                .map(|dep| (dep.clone(), item.id.clone()))
                .collect::<Vec<_>>()
        })
        .collect::<HashSet<_>>();
    let plan_edges = compiled_plan
        .dependency_graph
        .iter()
        .map(|edge| (edge.from_work_item_id.clone(), edge.to_work_item_id.clone()))
        .collect::<HashSet<_>>();

    assert_eq!(
        plan_edges, derived_edges,
        "compiled plan dependency graph must match final work_item.depends_on"
    );
    assert!(plan_edges.contains(&("work_item_a".to_string(), "work_item_c".to_string())));
}

#[tokio::test]
async fn final_compile_failure_updates_artifact_with_failed_compile_report() {
    let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_failed_compile_artifact");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    let store = engine.work_item_plan_store().expect("work item plan store");
    let now = chrono::Utc::now().to_rfc3339();
    let mut draft_a = test_work_item_draft_record(
        &plan_id,
        "outline_a",
        "draft_outline_a",
        WorkItemDraftStatus::Accepted,
        WorkItemGenerationMode::Serial,
        None,
    );
    draft_a.candidate.verification_plan["commands"][0]["command"] = serde_json::json!("rm -rf /");
    let draft_b = test_work_item_draft_record(
        &plan_id,
        "outline_b",
        "draft_outline_b",
        WorkItemDraftStatus::Accepted,
        WorkItemGenerationMode::Serial,
        None,
    );
    let draft_c = test_work_item_draft_record(
        &plan_id,
        "outline_c",
        "draft_outline_c",
        WorkItemDraftStatus::Accepted,
        WorkItemGenerationMode::Serial,
        None,
    );
    for draft in [&draft_a, &draft_b, &draft_c] {
        store.put_draft_record(draft).expect("put accepted draft");
    }
    store
        .save_active_index(&WorkItemPlanDraftActiveIndex {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: plan_id.clone(),
            current_generation_round_id: "round_0001".to_string(),
            outline_state: "confirmed".to_string(),
            active_outline_id: None,
            outline_to_current_draft_id: BTreeMap::from([
                ("outline_a".to_string(), "draft_outline_a".to_string()),
                ("outline_b".to_string(), "draft_outline_b".to_string()),
                ("outline_c".to_string(), "draft_outline_c".to_string()),
            ]),
            draft_statuses: BTreeMap::from([
                (
                    "draft_outline_a".to_string(),
                    WorkItemDraftStatus::Accepted,
                ),
                (
                    "draft_outline_b".to_string(),
                    WorkItemDraftStatus::Accepted,
                ),
                (
                    "draft_outline_c".to_string(),
                    WorkItemDraftStatus::Accepted,
                ),
            ]),
            batches: vec![],
            updated_at: now,
        })
        .expect("save active index");
    engine.session.stage = WorkspaceStage::Running;

    engine.enter_work_item_plan_compile().await;

    let ArtifactPayload::WorkItemPlanCompileReport { compile_report } = engine
        .session
        .artifact
        .as_ref()
        .expect("failed compile should update artifact")
    else {
        panic!("expected compile report artifact");
    };
    assert_eq!(compile_report.status, WorkItemPlanCompileStatus::Failed);
    assert!(compile_report
        .validator_findings
        .iter()
        .any(|finding| finding.code == "verification_command_unsafe"));
}

#[tokio::test]
async fn work_item_plan_confirm_rejects_confirmed_plan_without_compiled_work_items() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_confirm_empty_plan");
    let mut empty_confirmed_plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("load plan");
    empty_confirmed_plan.status = IssueWorkItemPlanStatus::Confirmed;
    empty_confirmed_plan.work_item_ids.clear();
    empty_confirmed_plan.verification_plan_ids.clear();
    empty_confirmed_plan.dependency_graph.clear();
    lifecycle
        .restore_issue_work_item_plan_snapshot(
            "project_0001",
            "issue_0001",
            &plan_id,
            &empty_confirmed_plan,
        )
        .expect("restore empty confirmed plan");

    let error = engine
        .confirm_work_item_plan()
        .await
        .expect_err("empty confirmed plan must not be confirmable");

    assert!(error.contains("compiled WorkItem"));
}

#[tokio::test]
async fn outline_generation_metadata_updates_current_artifact_without_new_version() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_outline_metadata_no_version");
    prepare_work_item_plan_outline_artifact(&mut engine).await;
    let version_count_before = engine.artifact_versions.len();
    let current_version_before = engine
        .artifact_versions
        .iter()
        .find(|version| version.is_current)
        .map(|version| version.version)
        .expect("current outline version");

    engine
        .update_work_item_plan_outline_generation_metadata(
            Some("round_0002".to_string()),
            Some(WorkItemGenerationModeDto::Serial),
        )
        .await
        .expect("update outline metadata");

    assert_eq!(engine.artifact_versions.len(), version_count_before);
    let current_version = engine
        .artifact_versions
        .iter()
        .find(|version| version.is_current)
        .expect("current outline version after metadata update");
    assert_eq!(current_version.version, current_version_before);
    let ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate } =
        &current_version.payload
    else {
        panic!("expected outline artifact");
    };
    assert_eq!(
        outline_candidate.current_generation_round_id.as_deref(),
        Some("round_0002")
    );
    assert_eq!(
        outline_candidate.selected_generation_mode,
        Some(WorkItemGenerationModeDto::Serial)
    );
}
