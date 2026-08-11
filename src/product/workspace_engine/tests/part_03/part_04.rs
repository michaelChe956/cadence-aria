use std::collections::BTreeMap;

use crate::product::logical_codebase::LogicalRepositoryId;
use uuid::Uuid;

#[test]
fn compile_rejects_draft_without_target_and_does_not_publish_partial_items() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_compile_target_missing");
    let previous_plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("load previous plan");
    let api = LogicalRepositoryId(Uuid::from_u128(1));
    let web = LogicalRepositoryId(Uuid::from_u128(2));
    let draft_a = target_draft(&plan_id, "outline_a", "draft_a", Some(api));
    let draft_b = target_draft(&plan_id, "outline_b", "draft_b", None);
    let targets = BTreeMap::from([
        (api, "repository_api".to_string()),
        (web, "repository_web".to_string()),
    ]);

    let persisted_before = lifecycle
        .count_work_items("project_0001", "issue_0001")
        .expect("count existing work item records");
    let error = engine
        .project_work_item_plan_drafts_for_compile(
            &previous_plan,
            &[draft_a, draft_b],
            compile_projection_context(&["outline_a", "outline_b"], Some(&targets)),
            &[],
        )
        .expect_err("missing target must block the whole compile projection");

    assert!(error.contains("work_item_target_missing"));
    assert!(error.contains("target_repository_id_missing"));
    assert_eq!(
        lifecycle
            .count_work_items("project_0001", "issue_0001")
            .expect("count work item records"),
        persisted_before,
        "compile blocker must not persist a partial work item record"
    );
}

#[test]
fn compile_allows_same_target_across_items_but_rejects_target_outside_selection() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_compile_target_outside_selection");
    let previous_plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("load previous plan");
    let api = LogicalRepositoryId(Uuid::from_u128(1));
    let web = LogicalRepositoryId(Uuid::from_u128(2));
    let removed = LogicalRepositoryId(Uuid::from_u128(3));
    let drafts = [
        target_draft(&plan_id, "outline_a", "draft_a", Some(api)),
        target_draft(&plan_id, "outline_b", "draft_b", Some(api)),
        target_draft(&plan_id, "outline_c", "draft_c", Some(removed)),
    ];
    let targets = BTreeMap::from([
        (api, "repository_api".to_string()),
        (web, "repository_web".to_string()),
    ]);

    let persisted_before = lifecycle
        .count_work_items("project_0001", "issue_0001")
        .expect("count existing work item records");
    let error = engine
        .project_work_item_plan_drafts_for_compile(
            &previous_plan,
            &drafts,
            compile_projection_context(
                &["outline_a", "outline_b", "outline_c"],
                Some(&targets),
            ),
            &[],
        )
        .expect_err("target outside effective selection must block compile");

    assert!(error.contains("work_item_target_missing"));
    assert!(error.contains("target_repository_id_not_effective"));
    assert_eq!(
        lifecycle
            .count_work_items("project_0001", "issue_0001")
            .expect("count work item records"),
        persisted_before,
        "invalid target must not publish a partial set"
    );
}

#[test]
fn compile_publishes_distinct_targets_in_single_transaction() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_compile_distinct_targets");
    let previous_plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("load previous plan");
    let api = LogicalRepositoryId(Uuid::from_u128(1));
    let web = LogicalRepositoryId(Uuid::from_u128(2));
    let drafts = [
        target_draft(&plan_id, "outline_a", "draft_a", Some(api)),
        target_draft(&plan_id, "outline_b", "draft_b", Some(web)),
    ];
    let targets = BTreeMap::from([
        (api, "repository_api".to_string()),
        (web, "repository_web".to_string()),
    ]);

    let (_plan, items, _verification_plans) = engine
        .project_work_item_plan_drafts_for_compile(
            &previous_plan,
            &drafts,
            compile_projection_context(&["outline_a", "outline_b"], Some(&targets)),
            &[],
        )
        .expect("distinct valid targets must compile together");

    assert_eq!(items.len(), 2);
    assert_eq!(items[0].target_repository_id, Some(api));
    assert_eq!(items[1].target_repository_id, Some(web));
    assert_ne!(items[0].repository_id, items[1].repository_id);
    assert!(items.iter().all(|item| item.target_repository_id.is_some()));
}

fn target_draft(
    plan_id: &str,
    outline_id: &str,
    draft_id: &str,
    target_repository_id: Option<LogicalRepositoryId>,
) -> WorkItemDraftRecord {
    let mut draft = test_work_item_draft_record(
        plan_id,
        outline_id,
        draft_id,
        WorkItemDraftStatus::Accepted,
        WorkItemGenerationMode::Serial,
        None,
    );
    draft.candidate.target_repository_id = target_repository_id;
    draft
}

fn compile_projection_context<'a>(
    outline_ids: &[&str],
    logical_targets: Option<&'a BTreeMap<LogicalRepositoryId, String>>,
) -> WorkItemPlanCompileProjectionContext<'a> {
    let outline_order = Box::leak(
        outline_ids
            .iter()
            .map(|outline_id| (*outline_id).to_string())
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let outline_to_work_item_id = Box::leak(Box::new(
        outline_ids
            .iter()
            .enumerate()
            .map(|(index, outline_id)| {
                (
                    (*outline_id).to_string(),
                    format!("work_item_{:03}", index + 1),
                )
            })
            .collect::<BTreeMap<_, _>>(),
    ));
    let outline_to_verification_plan_id = Box::leak(Box::new(
        outline_ids
            .iter()
            .enumerate()
            .map(|(index, outline_id)| {
                (
                    (*outline_id).to_string(),
                    format!("verification_plan_{:03}", index + 1),
                )
            })
            .collect::<BTreeMap<_, _>>(),
    ));
    WorkItemPlanCompileProjectionContext {
        outline_order,
        outline_to_work_item_id,
        outline_to_verification_plan_id,
        repository_id: "repository_legacy",
        logical_targets,
        now: "2026-08-10T00:00:00Z",
    }
}

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
    let mut input_a = crate::product::work_item_contract::canonical_contract_fixture("wi_temp")
        .input_contracts
        .remove(0);
    input_a.provider_logical_work_item_id = "wi_a".to_string();
    draft_b
        .candidate
        .canonical_contract_candidate
        .input_contracts
        .push(input_a.clone());
    let mut draft_c = test_work_item_draft_record(
        &plan_id,
        "outline_c",
        "draft_c",
        WorkItemDraftStatus::Accepted,
        WorkItemGenerationMode::Serial,
        None,
    );
    draft_c
        .candidate
        .canonical_contract_candidate
        .input_contracts
        .push(input_a);
    let mut input_b = crate::product::work_item_contract::canonical_contract_fixture("wi_temp")
        .input_contracts
        .remove(0);
    input_b.contract_id = "contract.source.b".to_string();
    input_b.provider_logical_work_item_id = "wi_b".to_string();
    draft_c
        .candidate
        .canonical_contract_candidate
        .input_contracts
        .push(input_b);

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
                logical_targets: None,
                now: "2026-06-27T00:00:00Z",
            },
            &[],
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

#[test]
fn final_compile_projects_source_draft_context_into_work_items() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_compile_source_context");
    engine.session.artifact = Some(ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: test_work_item_plan_outline(vec![]),
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

    let (_compiled_plan, work_items, _verification_plans) = engine
        .project_work_item_plan_drafts_for_compile(
            &previous_plan,
            &[draft_a],
            WorkItemPlanCompileProjectionContext {
                outline_order: &["outline_a".to_string()],
                outline_to_work_item_id: &BTreeMap::from([(
                    "outline_a".to_string(),
                    "work_item_a".to_string(),
                )]),
                outline_to_verification_plan_id: &BTreeMap::from([(
                    "outline_a".to_string(),
                    "verification_plan_a".to_string(),
                )]),
                repository_id: "repo_0001",
                logical_targets: None,
                now: "2026-06-30T00:00:00Z",
            },
            &[],
        )
        .expect("project compile records");

    let work_item = work_items.first().expect("work item");
    assert_eq!(
        work_item.source_work_item_plan_id.as_deref(),
        Some(plan_id.as_str())
    );
    assert_eq!(work_item.source_outline_id.as_deref(), Some("outline_a"));
    assert_eq!(work_item.source_draft_id.as_deref(), Some("draft_a"));
    assert_eq!(work_item.planned_implementation_context, None);
}

#[tokio::test]
async fn work_item_plan_compile_failure_updates_artifact_with_failed_compile_report() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
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
    draft_a.candidate.verification_plan.checks[0].command = Some("rm -rf /".to_string());
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
    let revision_store = crate::product::work_item_revision_store::WorkItemRevisionStore::new(
        lifecycle.app_paths(),
    );
    assert!(matches!(
        revision_store.get_plan_lineage("project_0001", "issue_0001", &plan_id),
        Err(ProductStoreError::NotFound { .. })
    ));
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
