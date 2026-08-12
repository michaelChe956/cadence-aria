#[tokio::test]
async fn coding_runtime_handoff_group_completion_resumes_and_starts_waiting_consumer_run() {
    let fixture = group_completion_fixture(true, true);
    let previous_run = create_authoritative_active_run(
        &fixture,
        "coding_unit_run_previous",
        1,
        CodingUnitRunStatus::Completed,
        Some(fixture.original_head.clone()),
        None,
    );
    create_authoritative_active_run(
        &fixture,
        "coding_unit_run_0001",
        2,
        CodingUnitRunStatus::Running,
        None,
        None,
    );
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("lineage");
    revision_store
        .put_handoff_revision(
            &lineage,
            &HandoffRevision {
                id: "handoff_revision_previous".to_string(),
                logical_work_item_id: "work_item_0001".to_string(),
                work_item_revision_id: previous_run.work_item_revision_id.clone(),
                coding_unit_run_id: previous_run.id.clone(),
                provided_contracts: vec!["contract_work_item_0001".to_string()],
                provided_capabilities: std::collections::BTreeMap::from([(
                    "contract_work_item_0001".to_string(),
                    vec!["capability_work_item_0001".to_string()],
                )]),
                contract_hash:
                    "5d1465e86ea2fbad8df040b5eac6ab52130ce6b06d2bd3b6403305c3b3e83b23"
                        .to_string(),
                commit_sha: fixture.original_head.clone(),
                created_at: "2026-07-18T00:00:00Z".to_string(),
            },
        )
        .expect("previous handoff");
    let source = fixture
        .store
        .get_active_coding_unit(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("active source lookup")
        .expect("active source");
    fixture
        .store
        .update_coding_unit_latest_handoff_revision_id(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &source.id,
            Some("handoff_revision_previous".to_string()),
        )
        .expect("previous source handoff pointer");
    let mut amended_plan = revision_store
        .get_plan_revision(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &lineage.id,
            "plan_revision_0001",
        )
        .expect("initial plan revision");
    amended_plan.id = "plan_revision_0002".to_string();
    amended_plan.revision_no = 2;
    amended_plan.supersedes = Some("plan_revision_0001".to_string());
    amended_plan.reason = crate::product::models::PlanRevisionReason::RepairUpstreamContract;
    amended_plan.created_at = "2026-07-18T00:00:01Z".to_string();
    revision_store
        .put_plan_revision(&lineage, &amended_plan)
        .expect("amended plan revision");
    revision_store
        .set_active_plan_revision(&lineage, &amended_plan.id)
        .expect("active amended plan");
    let amendment = crate::product::models::PlanAmendmentManifest {
        id: "plan_amendment_0001".to_string(),
        repair_request_id: "plan_repair_request_0001".to_string(),
        previous_plan_revision_id: "plan_revision_0001".to_string(),
        new_plan_revision_id: amended_plan.id.clone(),
        revised_work_items: std::collections::BTreeMap::new(),
        superseded_revisions: Vec::new(),
        dependency_graph_changes: Vec::new(),
        contract_deltas: Vec::new(),
        unaffected_units: vec!["work_item_0003".to_string()],
        revalidation_required_units: Vec::new(),
        stale_units: Vec::new(),
        replacement_units: std::collections::BTreeMap::new(),
        resume_target: crate::product::models::AmendmentResumeTarget {
            logical_work_item_id: "work_item_0002".to_string(),
            mode: crate::product::models::AmendmentResumeMode::AwaitHandoff,
        },
        created_at: "2026-07-18T00:00:01Z".to_string(),
    };
    revision_store
        .put_amendment_manifest(&lineage, &amendment)
        .expect("amendment");
    let mut binding = fixture
        .store
        .get_plan_binding(&fixture.attempt)
        .expect("plan binding");
    binding.bound_plan_revision_id = amended_plan.id;
    binding.applied_amendment_ids = vec![amendment.id.clone()];
    fixture
        .store
        .save_plan_binding(&fixture.attempt, &binding)
        .expect("applied amendment binding");

    let units = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("units");
    let consumer = units
        .iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0002")
        .expect("consumer")
        .clone();
    let revision = revision_store
        .get_work_item_revision(
            &lineage,
            &consumer.logical_work_item_id,
            &consumer.work_item_revision_id,
        )
        .expect("consumer revision");
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .expect("consumer bundle");
    let providers = fixture
        .store
        .get_role_provider_config_snapshot(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .expect("providers");
    fixture
        .store
        .create_coding_unit_run(
            &fixture.attempt,
            &CodingUnitRun {
                id: format!("coding_unit_run_{}_{}", consumer.id, amendment.id),
                unit_id: consumer.id.clone(),
                execution_no: 1,
                work_item_revision_id: revision.id.clone(),
                resolved_handoff_revision_ids: Vec::new(),
                canonical_contract_hash: revision.canonical_contract_hash.clone(),
                projection_bundle_id: bundle.id.clone(),
                projection_compiler_version: bundle.compiler_version.clone(),
                coder_provider_renderer_version: renderer_for(&providers.coder)
                    .renderer_version()
                    .to_string(),
                reviewer_provider_renderer_version: renderer_for(&providers.code_reviewer)
                    .renderer_version()
                    .to_string(),
                internal_reviewer_provider_renderer_version: None,
                coder_projection_hash: bundle.coder_projection_hash.clone(),
                reviewer_projection_hash: bundle.reviewer_projection_hash.clone(),
                coder_execution_context_hash: None,
                reviewer_execution_context_hash: None,
                internal_reviewer_execution_context_hash: None,
                status: CodingUnitRunStatus::AwaitingAmendment,
                unit_rework_count: 0,
                verification_retry_count: 0,
                operational_retry_count: 0,
                plan_repair_count: 1,
                start_commit: Some(fixture.original_head.clone()),
                completion_commit: None,
                created_at: "2026-07-18T00:00:01Z".to_string(),
                updated_at: "2026-07-18T00:00:01Z".to_string(),
            },
        )
        .expect("waiting consumer run");

    let updated = fixture
        .engine
        .complete_group_unit_after_code_review(&fixture.attempt)
        .await
        .expect("complete repaired source");

    assert_eq!(
        updated.current_work_item_id.as_deref(),
        Some("work_item_0002")
    );
    let consumer_run = fixture
        .store
        .list_unit_runs_by_logical_id(&updated, "work_item_0002")
        .expect("consumer runs")
        .pop()
        .expect("consumer run");
    assert_eq!(consumer_run.status, CodingUnitRunStatus::Running);
    assert_eq!(
        consumer_run.resolved_handoff_revision_ids,
        vec!["handoff_revision_coding_unit_run_0001"]
    );
    let source = fixture
        .store
        .list_coding_units(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
        )
        .expect("updated units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == "work_item_0001")
        .expect("updated source");
    assert_eq!(
        source.latest_handoff_revision_id.as_deref(),
        Some("handoff_revision_coding_unit_run_0001")
    );
}

#[tokio::test]
async fn coding_runtime_handoff_group_completion_rejects_non_authoritative_previous_pointer() {
    for case in ["missing", "cross_unit", "current_run_alias"] {
        let fixture = group_completion_fixture(true, true);
        create_authoritative_active_run(
            &fixture,
            "coding_unit_run_0001",
            1,
            CodingUnitRunStatus::Running,
            None,
            None,
        );
        let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
        let lineage = revision_store
            .get_plan_lineage(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                "work_item_plan_0001",
            )
            .expect("lineage");
        let pointer = "handoff_revision_non_authoritative";
        if case != "missing" {
            let logical_work_item_id = if case == "cross_unit" {
                "work_item_0002"
            } else {
                "work_item_0001"
            };
            let coding_unit_run_id = if case == "current_run_alias" {
                "coding_unit_run_0001"
            } else {
                "coding_unit_run_other"
            };
            revision_store
                .put_handoff_revision(
                    &lineage,
                    &HandoffRevision {
                        id: pointer.to_string(),
                        logical_work_item_id: logical_work_item_id.to_string(),
                        work_item_revision_id: format!(
                            "work_item_revision_000{}",
                            if case == "cross_unit" { 2 } else { 1 }
                        ),
                        coding_unit_run_id: coding_unit_run_id.to_string(),
                        provided_contracts: vec![format!(
                            "contract_work_item_000{}",
                            if case == "cross_unit" { 2 } else { 1 }
                        )],
                        provided_capabilities: std::collections::BTreeMap::new(),
                        contract_hash: "other_hash".to_string(),
                        commit_sha: fixture.original_head.clone(),
                        created_at: "2026-07-18T00:00:00Z".to_string(),
                    },
                )
                .expect("cross-unit handoff");
        }
        let source = fixture
            .store
            .get_active_coding_unit(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .expect("active source lookup")
            .expect("active source");
        fixture
            .store
            .update_coding_unit_latest_handoff_revision_id(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &source.id,
                Some(pointer.to_string()),
            )
            .expect("non-authoritative source pointer");

        assert_completion_preflight_is_zero_write(
            &fixture,
            "group_completion_handoff_revision_conflict",
        )
        .await;
    }
}

fn create_group_authority_run_for_logical_unit(
    fixture: &GroupCompletionFixture,
    logical_work_item_id: &str,
    id: &str,
    execution_no: u32,
    status: CodingUnitRunStatus,
    completion_commit: Option<String>,
) -> CodingUnitRun {
    let unit = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap()
        .into_iter()
        .find(|unit| unit.logical_work_item_id == logical_work_item_id)
        .unwrap();
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .unwrap();
    let revision = revision_store
        .get_work_item_revision(&lineage, logical_work_item_id, &unit.work_item_revision_id)
        .unwrap();
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .unwrap();
    let providers = fixture
        .store
        .get_role_provider_config_snapshot(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let run = CodingUnitRun {
        id: id.to_string(),
        unit_id: unit.id,
        execution_no,
        work_item_revision_id: revision.id,
        resolved_handoff_revision_ids: Vec::new(),
        canonical_contract_hash: revision.canonical_contract_hash,
        projection_bundle_id: bundle.id,
        projection_compiler_version: bundle.compiler_version,
        coder_provider_renderer_version: renderer_for(&providers.coder)
            .renderer_version()
            .to_string(),
        reviewer_provider_renderer_version: renderer_for(&providers.code_reviewer)
            .renderer_version()
            .to_string(),
        internal_reviewer_provider_renderer_version: None,
        coder_projection_hash: bundle.coder_projection_hash,
        reviewer_projection_hash: bundle.reviewer_projection_hash,
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        internal_reviewer_execution_context_hash: None,
        status,
        unit_rework_count: 0,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count: 0,
        start_commit: Some(fixture.original_head.clone()),
        completion_commit,
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    };
    fixture
        .store
        .create_coding_unit_run(&fixture.attempt, &run)
        .unwrap();
    run
}

fn create_group_authority_other_attempt_run(
    fixture: &GroupCompletionFixture,
    template: &CodingUnitRun,
) -> CodingUnitRun {
    let other = fixture
        .store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: fixture.attempt.project_id.clone(),
            issue_id: "issue_other".to_string(),
            plan_id: "work_item_plan_other".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_other".to_string(),
            worktree_path: None,
            provider_config_snapshot: fixture.attempt.provider_config_snapshot.clone(),
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .unwrap();
    let unit = fixture
        .store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: other.id.clone(),
            project_id: other.project_id.clone(),
            issue_id: other.issue_id.clone(),
            plan_id: "work_item_plan_other".to_string(),
            logical_work_item_id: "work_item_0001".to_string(),
            work_item_revision_id: template.work_item_revision_id.clone(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Completed,
        })
        .unwrap();
    let mut run = template.clone();
    run.id = "coding_unit_run_other_attempt".to_string();
    run.unit_id = unit.id;
    run.execution_no = 1;
    fixture.store.create_coding_unit_run(&other, &run).unwrap();
    run
}

#[tokio::test]
async fn coding_runtime_handoff_authority_group_previous_is_zero_write() {
    for case in [
        "unknown_run",
        "other_attempt",
        "wrong_unit",
        "wrong_revision",
        "non_completed",
        "commit_mismatch",
    ] {
        let fixture = group_completion_fixture(true, true);
        let active = create_authoritative_active_run(
            &fixture,
            "coding_unit_run_current",
            2,
            CodingUnitRunStatus::Running,
            None,
            None,
        );
        let prior = match case {
            "unknown_run" | "other_attempt" => None,
            "wrong_unit" => Some(create_group_authority_run_for_logical_unit(
                &fixture,
                "work_item_0002",
                "coding_unit_run_wrong_unit",
                1,
                CodingUnitRunStatus::Completed,
                Some(fixture.original_head.clone()),
            )),
            "non_completed" => Some(create_authoritative_active_run(
                &fixture,
                "coding_unit_run_previous_failed",
                1,
                CodingUnitRunStatus::Failed,
                None,
                None,
            )),
            _ => Some(create_authoritative_active_run(
                &fixture,
                "coding_unit_run_previous",
                1,
                CodingUnitRunStatus::Completed,
                Some(fixture.original_head.clone()),
                None,
            )),
        };
        let other_attempt = (case == "other_attempt")
            .then(|| {
                let mut template = active.clone();
                template.status = CodingUnitRunStatus::Completed;
                template.completion_commit = Some(fixture.original_head.clone());
                create_group_authority_other_attempt_run(&fixture, &template)
            });
        let run_id = match case {
            "unknown_run" => "coding_unit_run_unknown",
            "other_attempt" => &other_attempt.as_ref().unwrap().id,
            _ => &prior.as_ref().unwrap().id,
        };
        let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
        let lineage = revision_store
            .get_plan_lineage(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                "work_item_plan_0001",
            )
            .unwrap();
        let pointer = format!("handoff_revision_authority_{case}");
        revision_store
            .put_handoff_revision(
                &lineage,
                &HandoffRevision {
                    id: pointer.clone(),
                    logical_work_item_id: "work_item_0001".to_string(),
                    work_item_revision_id: if case == "wrong_revision" {
                        "work_item_revision_wrong".to_string()
                    } else {
                        prior
                            .as_ref()
                            .map(|run| run.work_item_revision_id.clone())
                            .or_else(|| {
                                other_attempt
                                    .as_ref()
                                    .map(|run| run.work_item_revision_id.clone())
                            })
                            .unwrap_or_else(|| active.work_item_revision_id.clone())
                    },
                    coding_unit_run_id: run_id.to_string(),
                    provided_contracts: vec!["contract_work_item_0001".to_string()],
                    provided_capabilities: std::collections::BTreeMap::new(),
                    contract_hash: "authority_hash".to_string(),
                    commit_sha: if case == "commit_mismatch" {
                        "commit_mismatch".to_string()
                    } else {
                        prior
                            .as_ref()
                            .and_then(|run| run.completion_commit.clone())
                            .or_else(|| {
                                other_attempt
                                    .as_ref()
                                    .and_then(|run| run.completion_commit.clone())
                            })
                            .unwrap_or_else(|| fixture.original_head.clone())
                    },
                    created_at: "2026-07-18T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        let source = fixture
            .store
            .get_active_coding_unit(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap()
            .unwrap();
        fixture
            .store
            .update_coding_unit_latest_handoff_revision_id(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
                &source.id,
                Some(pointer),
            )
            .unwrap();

        assert_completion_preflight_is_zero_write(
            &fixture,
            "group_completion_handoff_revision_conflict",
        )
        .await;
    }
}
