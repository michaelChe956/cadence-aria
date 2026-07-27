fn seed_authoritative_group_plan_fixture(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = WorkItemPlanLineage {
        id: "work_item_plan_0001".to_string(),
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        story_spec_refs: Vec::new(),
        design_spec_refs: Vec::new(),
        active_revision_id: None,
        active_amendment_id: None,
        created_at: "2026-07-18T00:00:00Z".to_string(),
        updated_at: "2026-07-18T00:00:00Z".to_string(),
    };
    revision_store
        .put_plan_lineage(&lineage)
        .expect("plan lineage");
    let mut bindings = std::collections::BTreeMap::new();
    let mut work_item_projections = std::collections::BTreeMap::new();
    let mut projection_bundle_refs = Vec::new();
    for (logical_id, revision_id) in [
        ("work_item_0001", "work_item_revision_0001"),
        ("work_item_0002", "work_item_revision_0002"),
    ] {
        let logical = LogicalWorkItem {
            id: logical_id.to_string(),
            plan_id: lineage.id.clone(),
            title: logical_id.to_string(),
            active_revision_id: None,
            created_at: "2026-07-18T00:00:00Z".to_string(),
            updated_at: "2026-07-18T00:00:00Z".to_string(),
        };
        revision_store
            .put_logical_work_item(&lineage, &logical)
            .expect("logical work item");
        let contract = CanonicalWorkItemContract {
            schema_version: 1,
            identity: WorkItemContractIdentity {
                logical_work_item_id: logical.id.clone(),
                title: logical.title.clone(),
                kind: "implementation".to_string(),
            },
            goal: WorkItemGoal {
                summary: logical.title.clone(),
            },
            non_goals: Vec::new(),
            input_contracts: Vec::new(),
            output_contracts: Vec::new(),
            tasks: Vec::new(),
            write_policy: WorkItemWritePolicy {
                exclusive_scopes: Vec::new(),
                forbidden_scopes: Vec::new(),
            },
            acceptance_criteria: Vec::new(),
            verification_checks: Vec::new(),
            handoff_contract: HandoffContract {
                required_fields: Vec::new(),
                provided_contract_refs: Vec::new(),
                reviewer_check_refs: Vec::new(),
            },
            blocker_rules: authoritative_group_blocker_rules_fixture(),
            design_traceability: Vec::new(),
        };
        let revision = WorkItemRevision {
            id: revision_id.to_string(),
            logical_work_item_id: logical.id.clone(),
            source_draft_revision_id: format!("draft_{revision_id}"),
            canonical_contract_hash: canonical_contract_hash(&contract).expect("contract hash"),
            canonical_contract: contract,
            work_item_projection_bundle_id: format!("projection_{revision_id}"),
            verification_plan_revision_id: format!("verification_{revision_id}"),
            created_at: "2026-07-18T00:00:00Z".to_string(),
        };
        revision_store
            .put_verification_plan_revision(
                &lineage,
                &VerificationPlanRevision {
                    id: revision.verification_plan_revision_id.clone(),
                    logical_work_item_id: logical.id.clone(),
                    source_draft_revision_id: revision.source_draft_revision_id.clone(),
                    verification_checks: revision.canonical_contract.verification_checks.clone(),
                    created_at: "2026-07-18T00:00:00Z".to_string(),
                },
            )
            .expect("verification plan revision");
        revision_store
            .put_work_item_revision(&lineage, &revision)
            .expect("work item revision");
        let projections = WorkItemProjectionCompiler
            .compile(&revision.canonical_contract, &revision.id)
            .expect("compile work item projections");
        let hashes = projection_hashes(&projections).expect("projection hashes");
        revision_store
            .put_work_item_projection_bundle(
                &lineage,
                &WorkItemProjectionBundle {
                    id: revision.work_item_projection_bundle_id.clone(),
                    work_item_revision_id: revision.id.clone(),
                    canonical_contract_hash: revision.canonical_contract_hash.clone(),
                    projection_schema_version: 1,
                    compiler_version: "work-item-projection-compiler-v1".to_string(),
                    human_projection: projections.human.clone(),
                    coder_projection: projections.coder.clone(),
                    reviewer_projection: projections.reviewer.clone(),
                    human_projection_hash: hashes.human,
                    coder_projection_hash: hashes.coder,
                    reviewer_projection_hash: hashes.reviewer,
                    created_at: "2026-07-18T00:00:00Z".to_string(),
                },
            )
            .expect("work item projection bundle");
        revision_store
            .set_active_work_item_revision(&lineage, &logical, None, revision_id)
            .expect("active work item revision");
        projection_bundle_refs.push(revision.work_item_projection_bundle_id.clone());
        work_item_projections.insert(logical.id.clone(), projections);
        bindings.insert(logical.id, revision.id);
    }
    let graph = DependencyGraphRevision {
        id: "dependency_graph_revision_0001".to_string(),
        plan_id: lineage.id.clone(),
        edges: vec![cadence_aria::product::work_item_contract::DependencyContractEdge {
            from: "work_item_0001".to_string(),
            to: "work_item_0002".to_string(),
            required_contracts: Vec::new(),
        }],
        created_at: "2026-07-18T00:00:00Z".to_string(),
    };
    revision_store
        .put_dependency_graph_revision(&lineage, &graph)
        .expect("dependency graph");
    let ordered_logical_work_item_ids = bindings.keys().cloned().collect::<Vec<_>>();
    let plan_revision = WorkItemPlanRevision {
        id: "plan_revision_0001".to_string(),
        plan_id: lineage.id.clone(),
        revision_no: 1,
        supersedes: None,
        reason: PlanRevisionReason::InitialCompile,
        work_item_bindings: bindings,
        dependency_graph_revision_id: graph.id,
        validation_report_ref: "validation_report_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        created_at: "2026-07-18T00:00:00Z".to_string(),
    };
    let compiled_plan = CompiledPlanProjections {
        human: HumanGroupProjection {
            plan_id: lineage.id.clone(),
            goal: "authoritative group fixture".to_string(),
            split_reason: "fixture supplies every immutable Schema v2 revision".to_string(),
            work_items: ordered_logical_work_item_ids
                .iter()
                .map(|logical_id| {
                    let projection = &work_item_projections[logical_id].human;
                    HumanGroupWorkItemSummary {
                        logical_work_item_id: logical_id.clone(),
                        title: projection.title.clone(),
                        goal: projection.goal.clone(),
                        depends_on: graph
                            .edges
                            .iter()
                            .filter(|edge| edge.to == *logical_id)
                            .map(|edge| edge.from.clone())
                            .collect(),
                        provides: projection
                            .outputs
                            .iter()
                            .map(|output| output.contract_id.clone())
                            .collect(),
                        scope_summary: projection.scope_summary.clone(),
                    }
                })
                .collect(),
            contract_flow: Vec::new(),
            risks: Vec::new(),
            source_refs: Vec::new(),
            normative: false,
            used_by_provider: false,
        },
        coder: CoderGroupContext {
            plan_id: lineage.id.clone(),
            ordered_logical_work_item_ids: ordered_logical_work_item_ids.clone(),
            dependency_edges: graph.edges.clone(),
            group_write_scopes: ordered_logical_work_item_ids
                .iter()
                .map(|logical_id| {
                    (
                        logical_id.clone(),
                        work_item_projections[logical_id].coder.write_policy.clone(),
                    )
                })
                .collect(),
        },
        reviewer: ReviewerGroupMatrix {
            plan_id: lineage.id.clone(),
            work_items: ordered_logical_work_item_ids
                .iter()
                .map(|logical_id| ReviewerGroupMatrixEntry {
                    logical_work_item_id: logical_id.clone(),
                    criterion_refs: work_item_projections[logical_id]
                        .reviewer
                        .criterion_refs
                        .clone(),
                    input_contract_refs: Vec::new(),
                    output_contract_refs: Vec::new(),
                })
                .collect(),
            dependency_edges: graph.edges.clone(),
            design_traceability_refs: Vec::new(),
        },
    };
    revision_store
        .put_plan_projection_bundle(
            &lineage,
            &PlanProjectionBundle {
                id: plan_revision.plan_projection_bundle_id.clone(),
                plan_revision_id: plan_revision.id.clone(),
                dependency_graph_revision_id: plan_revision.dependency_graph_revision_id.clone(),
                work_item_projection_bundle_refs: projection_bundle_refs,
                human_group_projection: compiled_plan.human,
                coder_group_context: compiled_plan.coder,
                reviewer_group_matrix: compiled_plan.reviewer,
                human_group_projection_hash: "fixture-human-group-projection-hash".to_string(),
                coder_group_context_hash: "fixture-coder-group-context-hash".to_string(),
                reviewer_group_matrix_hash: "fixture-reviewer-group-matrix-hash".to_string(),
                compiler_version: "plan-projection-compiler-v1".to_string(),
                created_at: "2026-07-18T00:00:00Z".to_string(),
            },
        )
        .expect("plan projection bundle");
    revision_store
        .put_plan_revision(&lineage, &plan_revision)
        .expect("plan revision");
    revision_store
        .set_active_plan_revision(&lineage, &plan_revision.id)
        .expect("active plan revision");
    store
        .save_plan_binding(
            attempt,
            &CodingAttemptPlanBinding {
                attempt_id: attempt.id.clone(),
                plan_id: lineage.id,
                bound_plan_revision_id: plan_revision.id,
                applied_amendment_ids: Vec::new(),
                updated_at: "2026-07-18T00:00:00Z".to_string(),
            },
        )
        .expect("attempt plan binding");
}
