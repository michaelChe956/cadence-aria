use super::*;

pub(super) fn seed_group_plan_facts(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt) {
    let plan_id = attempt
        .work_item_group_id
        .as_deref()
        .expect("group attempt plan id");
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = WorkItemPlanLineage {
        id: plan_id.to_string(),
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        story_spec_refs: Vec::new(),
        design_spec_refs: Vec::new(),
        active_revision_id: None,
        active_amendment_id: None,
        created_at: "2026-07-12T00:00:00Z".to_string(),
        updated_at: "2026-07-12T00:00:00Z".to_string(),
    };
    revision_store
        .put_plan_lineage(&lineage)
        .expect("group plan lineage");
    let logical_work_item = LogicalWorkItem {
        id: attempt.work_item_id.clone(),
        plan_id: lineage.id.clone(),
        title: "failed review recovery fixture".to_string(),
        active_revision_id: None,
        created_at: "2026-07-12T00:00:00Z".to_string(),
        updated_at: "2026-07-12T00:00:00Z".to_string(),
    };
    revision_store
        .put_logical_work_item(&lineage, &logical_work_item)
        .expect("logical work item");
    let contract = CanonicalWorkItemContract {
        schema_version: 1,
        identity: WorkItemContractIdentity {
            logical_work_item_id: logical_work_item.id.clone(),
            title: logical_work_item.title.clone(),
            kind: "implementation".to_string(),
        },
        goal: WorkItemGoal {
            summary: logical_work_item.title.clone(),
        },
        non_goals: Vec::new(),
        depends_on: Vec::new(),
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
        blocker_rules: vec![BlockerRule {
            reason_code: "verification_incomplete".to_string(),
            route: BlockerRoute::VerificationRetry,
            target_contract_refs: Vec::new(),
        }],
        design_traceability: Vec::new(),
    };
    let work_item_revision = WorkItemRevision {
        id: "work_item_revision_0001".to_string(),
        logical_work_item_id: logical_work_item.id.clone(),
        source_draft_revision_id: "draft_work_item_revision_0001".to_string(),
        canonical_contract_hash: canonical_contract_hash(&contract).expect("contract hash"),
        canonical_contract: contract,
        work_item_projection_bundle_id: "projection_bundle_0001".to_string(),
        verification_plan_revision_id: "verification_revision_0001".to_string(),
        created_at: "2026-07-12T00:00:00Z".to_string(),
    };
    revision_store
        .put_verification_plan_revision(
            &lineage,
            &VerificationPlanRevision {
                id: work_item_revision.verification_plan_revision_id.clone(),
                logical_work_item_id: logical_work_item.id.clone(),
                source_draft_revision_id: work_item_revision.source_draft_revision_id.clone(),
                verification_checks: work_item_revision
                    .canonical_contract
                    .verification_checks
                    .clone(),
                created_at: "2026-07-12T00:00:00Z".to_string(),
            },
        )
        .expect("verification plan revision");
    revision_store
        .put_work_item_revision(&lineage, &work_item_revision)
        .expect("work item revision");
    let projections = WorkItemProjectionCompiler
        .compile(
            &work_item_revision.canonical_contract,
            &work_item_revision.id,
        )
        .expect("work item projections");
    let projection_hashes = projection_hashes(&projections).expect("projection hashes");
    revision_store
        .put_work_item_projection_bundle(
            &lineage,
            &WorkItemProjectionBundle {
                id: work_item_revision.work_item_projection_bundle_id.clone(),
                work_item_revision_id: work_item_revision.id.clone(),
                canonical_contract_hash: work_item_revision.canonical_contract_hash.clone(),
                projection_schema_version: 1,
                compiler_version: "work-item-projection-compiler-v1".to_string(),
                human_projection: projections.human.clone(),
                coder_projection: projections.coder.clone(),
                reviewer_projection: projections.reviewer.clone(),
                human_projection_hash: projection_hashes.human,
                coder_projection_hash: projection_hashes.coder,
                reviewer_projection_hash: projection_hashes.reviewer,
                created_at: "2026-07-12T00:00:00Z".to_string(),
            },
        )
        .expect("work item projection bundle");
    revision_store
        .set_active_work_item_revision(&lineage, &logical_work_item, None, &work_item_revision.id)
        .expect("active work item revision");
    let dependency_graph = DependencyGraphRevision {
        id: "dependency_graph_revision_0001".to_string(),
        plan_id: lineage.id.clone(),
        edges: Vec::new(),
        created_at: "2026-07-12T00:00:00Z".to_string(),
    };
    revision_store
        .put_dependency_graph_revision(&lineage, &dependency_graph)
        .expect("dependency graph");
    let plan_revision = WorkItemPlanRevision {
        id: "plan_revision_0001".to_string(),
        plan_id: lineage.id.clone(),
        revision_no: 1,
        supersedes: None,
        reason: PlanRevisionReason::InitialCompile,
        work_item_bindings: BTreeMap::from([(
            logical_work_item.id.clone(),
            work_item_revision.id.clone(),
        )]),
        dependency_graph_revision_id: dependency_graph.id.clone(),
        validation_report_ref: "validation_report_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        created_at: "2026-07-12T00:00:00Z".to_string(),
    };
    let compiled_plan = CompiledPlanProjections {
        human: HumanGroupProjection {
            plan_id: lineage.id.clone(),
            goal: logical_work_item.title.clone(),
            split_reason: "fixture publishes complete Schema v2 revisions".to_string(),
            work_items: vec![HumanGroupWorkItemSummary {
                logical_work_item_id: logical_work_item.id.clone(),
                title: projections.human.title.clone(),
                goal: projections.human.goal.clone(),
                depends_on: Vec::new(),
                provides: Vec::new(),
                scope_summary: projections.human.scope_summary.clone(),
            }],
            contract_flow: Vec::new(),
            risks: Vec::new(),
            source_refs: Vec::new(),
            normative: false,
            used_by_provider: false,
        },
        coder: CoderGroupContext {
            plan_id: lineage.id.clone(),
            ordered_logical_work_item_ids: vec![logical_work_item.id.clone()],
            dependency_edges: Vec::new(),
            group_write_scopes: BTreeMap::from([(
                logical_work_item.id.clone(),
                projections.coder.write_policy.clone(),
            )]),
        },
        reviewer: ReviewerGroupMatrix {
            plan_id: lineage.id.clone(),
            work_items: vec![ReviewerGroupMatrixEntry {
                logical_work_item_id: logical_work_item.id.clone(),
                criterion_refs: projections.reviewer.criterion_refs.clone(),
                input_contract_refs: Vec::new(),
                output_contract_refs: Vec::new(),
            }],
            dependency_edges: Vec::new(),
            design_traceability_refs: Vec::new(),
        },
    };
    let plan_hashes = plan_projection_hashes(&compiled_plan).expect("plan projection hashes");
    revision_store
        .put_plan_projection_bundle(
            &lineage,
            &PlanProjectionBundle {
                id: plan_revision.plan_projection_bundle_id.clone(),
                plan_revision_id: plan_revision.id.clone(),
                dependency_graph_revision_id: dependency_graph.id.clone(),
                work_item_projection_bundle_refs: vec![
                    work_item_revision.work_item_projection_bundle_id.clone(),
                ],
                human_group_projection: compiled_plan.human,
                coder_group_context: compiled_plan.coder,
                reviewer_group_matrix: compiled_plan.reviewer,
                human_group_projection_hash: plan_hashes.human,
                coder_group_context_hash: plan_hashes.coder,
                reviewer_group_matrix_hash: plan_hashes.reviewer,
                compiler_version: "plan-projection-compiler-v1".to_string(),
                created_at: "2026-07-12T00:00:00Z".to_string(),
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
                plan_id: plan_id.to_string(),
                bound_plan_revision_id: "plan_revision_0001".to_string(),
                applied_amendment_ids: Vec::new(),
                updated_at: "2026-07-12T00:00:00Z".to_string(),
            },
        )
        .expect("group plan binding");
}
