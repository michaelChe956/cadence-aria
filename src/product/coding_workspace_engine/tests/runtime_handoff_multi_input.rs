fn runtime_multi_input_fixture() -> (RuntimeHandoffFixture, HandoffRevision) {
    let fixture = runtime_handoff_fixture(
        RuntimeContractChange::Unchanged,
        CodingUnitRunStatus::AwaitingAmendment,
    );
    let revision_store = WorkItemRevisionStore::new(fixture.store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            "work_item_plan_0001",
        )
        .unwrap();
    let mut aux_contract = runtime_contract("wi_aux", Vec::new(), vec!["aux_ready"]);
    aux_contract.output_contracts[0].contract_id = "aux_contract".to_string();
    aux_contract.handoff_contract.provided_contract_refs = vec!["aux_contract".to_string()];
    ensure_runtime_logical_item(&revision_store, &lineage, "wi_aux");
    put_runtime_revision(
        &revision_store,
        &lineage,
        "wi_aux",
        "work_item_revision_aux_v1",
        &aux_contract,
    );
    let mut graph = revision_store
        .get_dependency_graph_revision(&lineage, "dependency_graph_revision_0002")
        .unwrap();
    graph.id = "dependency_graph_revision_0003".to_string();
    graph
        .edges
        .iter_mut()
        .find(|edge| edge.from == "wi_core" && edge.to == "wi_registration")
        .unwrap()
        .required_contracts[0]
        .required_capabilities = vec!["registration_ready".to_string()];
    graph.edges.push(
        crate::product::work_item_contract::DependencyContractEdge {
            from: "wi_aux".to_string(),
            to: "wi_registration".to_string(),
            required_contracts: vec![RequiredDependencyContract {
                contract_id: "aux_contract".to_string(),
                required_capabilities: vec!["aux_ready".to_string()],
                compatibility_policy: ContractCompatibilityPolicy::RequireAll,
            }],
        },
    );
    revision_store
        .put_dependency_graph_revision(&lineage, &graph)
        .unwrap();
    let mut plan = revision_store
        .get_plan_revision(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &lineage.id,
            "plan_revision_0002",
        )
        .unwrap();
    plan.id = "plan_revision_0003".to_string();
    plan.revision_no = 3;
    plan.supersedes = Some("plan_revision_0002".to_string());
    plan.dependency_graph_revision_id = graph.id;
    plan.work_item_bindings.insert(
        "wi_aux".to_string(),
        "work_item_revision_aux_v1".to_string(),
    );
    revision_store.put_plan_revision(&lineage, &plan).unwrap();
    revision_store
        .set_active_plan_revision(&lineage, &plan.id)
        .unwrap();
    let mut manifest = revision_store
        .get_amendment_manifest(&lineage, "plan_amendment_0001")
        .unwrap();
    manifest.id = "plan_amendment_0002".to_string();
    manifest.repair_request_id = "plan_repair_request_0002".to_string();
    manifest.previous_plan_revision_id = "plan_revision_0002".to_string();
    manifest.new_plan_revision_id = plan.id.clone();
    manifest.revalidation_required_units.clear();
    revision_store
        .put_amendment_manifest(&lineage, &manifest)
        .unwrap();
    let mut binding = fixture.store.get_plan_binding(&fixture.attempt).unwrap();
    binding.bound_plan_revision_id = plan.id;
    binding.applied_amendment_ids.push(manifest.id);
    fixture
        .store
        .save_plan_binding(&fixture.attempt, &binding)
        .unwrap();
    let aux_unit = fixture
        .store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: fixture.attempt.id.clone(),
            project_id: fixture.attempt.project_id.clone(),
            issue_id: fixture.attempt.issue_id.clone(),
            plan_id: lineage.id.clone(),
            logical_work_item_id: "wi_aux".to_string(),
            work_item_revision_id: "work_item_revision_aux_v1".to_string(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 4,
            status: CodingExecutionUnitStatus::Completed,
        })
        .unwrap();
    let aux_revision = revision_store
        .get_work_item_revision(&lineage, "wi_aux", "work_item_revision_aux_v1")
        .unwrap();
    let aux_bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &aux_revision.work_item_projection_bundle_id)
        .unwrap();
    let mut aux_run = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "wi_core")
        .unwrap()
        .remove(0);
    aux_run.id = "coding_unit_run_aux_0001".to_string();
    aux_run.unit_id = aux_unit.id.clone();
    aux_run.execution_no = 1;
    aux_run.work_item_revision_id = aux_revision.id.clone();
    aux_run.canonical_contract_hash = aux_revision.canonical_contract_hash;
    aux_run.projection_bundle_id = aux_bundle.id;
    aux_run.projection_compiler_version = aux_bundle.compiler_version;
    aux_run.coder_projection_hash = aux_bundle.coder_projection_hash;
    aux_run.reviewer_projection_hash = aux_bundle.reviewer_projection_hash;
    aux_run.completion_commit = Some("commit_aux_0001".to_string());
    fixture
        .store
        .create_coding_unit_run(&fixture.attempt, &aux_run)
        .unwrap();
    let mut satisfying = handoff(
        "handoff_revision_aux_satisfying",
        &["aux_contract"],
        &[("aux_contract", &["aux_ready"])],
        "aux_hash_satisfying",
    );
    satisfying.logical_work_item_id = "wi_aux".to_string();
    satisfying.work_item_revision_id = aux_revision.id;
    satisfying.coding_unit_run_id = aux_run.id;
    satisfying.commit_sha = aux_run.completion_commit.unwrap();
    let mut insufficient = satisfying.clone();
    insufficient.id = "handoff_revision_aux_insufficient".to_string();
    insufficient.provided_capabilities.clear();
    insufficient.contract_hash = "aux_hash_insufficient".to_string();
    revision_store
        .put_handoff_revision(&lineage, &satisfying)
        .unwrap();
    revision_store
        .put_handoff_revision(&lineage, &insufficient)
        .unwrap();
    fixture
        .store
        .update_coding_unit_latest_handoff_revision_id(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &aux_unit.id,
            Some(insufficient.id),
        )
        .unwrap();
    (fixture, satisfying)
}

#[tokio::test]
async fn coding_runtime_handoff_unchanged_multi_input_requires_all_capabilities() {
    let (fixture, satisfying) = runtime_multi_input_fixture();
    let runs_before = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "wi_registration")
        .unwrap();

    let blocked = fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &fixture.next_handoff)
        .await
        .unwrap();

    assert!(blocked.resumed_units.is_empty());
    assert_eq!(blocked.propagation_stopped_at, Some("wi_core".to_string()));
    assert_eq!(
        fixture
            .store
            .list_unit_runs_by_logical_id(&fixture.attempt, "wi_registration")
            .unwrap(),
        runs_before
    );
    let aux = fixture
        .store
        .list_coding_units(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap()
        .into_iter()
        .find(|unit| unit.logical_work_item_id == "wi_aux")
        .unwrap();
    fixture
        .store
        .update_coding_unit_latest_handoff_revision_id(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
            &aux.id,
            Some(satisfying.id.clone()),
        )
        .unwrap();

    let resumed = fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &fixture.next_handoff)
        .await
        .unwrap();

    assert_eq!(resumed.resumed_units, vec!["wi_registration"]);
    let run = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "wi_registration")
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(run.status, CodingUnitRunStatus::Pending);
    assert_eq!(
        run.resolved_handoff_revision_ids,
        vec!["handoff_revision_0002", "handoff_revision_aux_satisfying"]
    );
}
