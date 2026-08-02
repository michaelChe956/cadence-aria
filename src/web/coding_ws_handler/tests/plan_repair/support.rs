use super::*;

pub(super) fn plan_repair_fixture() -> PlanRepairFixture {
    plan_repair_fixture_with_dependency(true)
}

pub(super) fn plan_repair_fixture_with_dependency(with_dependency: bool) -> PlanRepairFixture {
    let tmp = TempDir::new().expect("temp dir");
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let repository_path = tmp.path().join("worktree");
    std::fs::create_dir_all(&repository_path).expect("repository directory");
    let repository = RepositoryStore::new(paths.clone())
        .create(CreateRepositoryInput {
            project_id: "project_0001".to_string(),
            name: "plan repair fixture".to_string(),
            path: repository_path,
            default_policy_preset: None,
            default_provider_mode: Some("fake".to_string()),
        })
        .expect("repository");
    let issue = IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: "project_0001".to_string(),
            repo_id: Some(repository.id),
            title: "plan repair fixture".to_string(),
            description: None,
            change_id: None,
        })
        .expect("issue");
    assert_eq!(issue.id, "issue_0001");
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "wi_current".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(tmp.path().join("worktree")),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            max_auto_rework: 2,
        })
        .unwrap();
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let plan = WorkItemPlanLineage {
        id: "work_item_plan_0001".to_string(),
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        story_spec_refs: Vec::new(),
        design_spec_refs: Vec::new(),
        active_revision_id: None,
        active_amendment_id: None,
        created_at: "2026-07-19T00:00:00Z".to_string(),
        updated_at: "2026-07-19T00:00:00Z".to_string(),
    };
    revision_store.put_plan_lineage(&plan).unwrap();
    let upstream = logical_item("wi_upstream", &plan.id);
    let current = logical_item("wi_current", &plan.id);
    revision_store
        .put_logical_work_item(&plan, &upstream)
        .unwrap();
    revision_store
        .put_logical_work_item(&plan, &current)
        .unwrap();
    let upstream_revision = work_item_revision(
        &upstream,
        "work_item_revision_upstream",
        CanonicalWorkItemContract {
            output_contracts: vec![PromisedOutputContract {
                contract_id: "contract_upstream".to_string(),
                capabilities: vec!["capability_existing".to_string()],
            }],
            ..contract(&upstream.id)
        },
        "projection_bundle_upstream",
    );
    revision_store
        .put_verification_plan_revision(
            &plan,
            &VerificationPlanRevision {
                id: upstream_revision.verification_plan_revision_id.clone(),
                logical_work_item_id: upstream.id.clone(),
                source_draft_revision_id: upstream_revision.source_draft_revision_id.clone(),
                verification_checks: upstream_revision
                    .canonical_contract
                    .verification_checks
                    .clone(),
                created_at: "2026-07-19T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    revision_store
        .put_work_item_revision(&plan, &upstream_revision)
        .unwrap();
    let upstream_compiled = WorkItemProjectionCompiler
        .compile(&upstream_revision.canonical_contract, &upstream_revision.id)
        .unwrap();
    let upstream_hashes = projection_hashes(&upstream_compiled).unwrap();
    revision_store
        .put_work_item_projection_bundle(
            &plan,
            &WorkItemProjectionBundle {
                id: upstream_revision.work_item_projection_bundle_id.clone(),
                work_item_revision_id: upstream_revision.id.clone(),
                canonical_contract_hash: upstream_revision.canonical_contract_hash.clone(),
                projection_schema_version: 1,
                compiler_version: "work-item-projection-compiler-v1".to_string(),
                human_projection: upstream_compiled.human.clone(),
                coder_projection: upstream_compiled.coder.clone(),
                reviewer_projection: upstream_compiled.reviewer.clone(),
                human_projection_hash: upstream_hashes.human,
                coder_projection_hash: upstream_hashes.coder,
                reviewer_projection_hash: upstream_hashes.reviewer,
                created_at: "2026-07-19T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    revision_store
        .set_active_work_item_revision(&plan, &upstream, None, &upstream_revision.id)
        .unwrap();
    let current_revision = work_item_revision(
        &current,
        "work_item_revision_current",
        CanonicalWorkItemContract {
            input_contracts: vec![RequiredInputContract {
                contract_id: "contract_upstream".to_string(),
                provider_logical_work_item_id: upstream.id.clone(),
                required_capabilities: vec!["capability_missing".to_string()],
                compatibility_policy: ContractCompatibilityPolicy::RequireAll,
            }],
            blocker_rules: vec![BlockerRule {
                reason_code: "upstream_contract_capability_missing".to_string(),
                route: BlockerRoute::PlanRepairUpstream,
                target_contract_refs: vec!["contract_upstream".to_string()],
            }],
            ..contract(&current.id)
        },
        "projection_bundle_current",
    );
    revision_store
        .put_verification_plan_revision(
            &plan,
            &VerificationPlanRevision {
                id: current_revision.verification_plan_revision_id.clone(),
                logical_work_item_id: current.id.clone(),
                source_draft_revision_id: current_revision.source_draft_revision_id.clone(),
                verification_checks: current_revision
                    .canonical_contract
                    .verification_checks
                    .clone(),
                created_at: "2026-07-19T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    revision_store
        .put_work_item_revision(&plan, &current_revision)
        .unwrap();
    revision_store
        .set_active_work_item_revision(&plan, &current, None, &current_revision.id)
        .unwrap();
    let compiled = WorkItemProjectionCompiler
        .compile(&current_revision.canonical_contract, &current_revision.id)
        .unwrap();
    let hashes = projection_hashes(&compiled).unwrap();
    let projection = compiled.reviewer.clone();
    let bundle = WorkItemProjectionBundle {
        id: current_revision.work_item_projection_bundle_id.clone(),
        work_item_revision_id: current_revision.id.clone(),
        canonical_contract_hash: current_revision.canonical_contract_hash.clone(),
        projection_schema_version: 1,
        compiler_version: "work-item-projection-compiler-v1".to_string(),
        human_projection: compiled.human.clone(),
        coder_projection: compiled.coder.clone(),
        reviewer_projection: compiled.reviewer.clone(),
        human_projection_hash: hashes.human,
        coder_projection_hash: hashes.coder.clone(),
        reviewer_projection_hash: hashes.reviewer.clone(),
        created_at: "2026-07-19T00:00:00Z".to_string(),
    };
    revision_store
        .put_work_item_projection_bundle(&plan, &bundle)
        .unwrap();
    let dependency_graph = DependencyGraphRevision {
        id: "dependency_graph_revision_0001".to_string(),
        plan_id: plan.id.clone(),
        edges: Vec::new(),
        created_at: "2026-07-19T00:00:00Z".to_string(),
    };
    revision_store
        .put_dependency_graph_revision(&plan, &dependency_graph)
        .unwrap();
    let plan_revision = WorkItemPlanRevision {
        id: "plan_revision_0001".to_string(),
        plan_id: plan.id.clone(),
        revision_no: 1,
        supersedes: None,
        reason: PlanRevisionReason::InitialCompile,
        work_item_bindings: BTreeMap::from([
            (upstream.id.clone(), upstream_revision.id.clone()),
            (current.id.clone(), current_revision.id.clone()),
        ]),
        dependency_graph_revision_id: dependency_graph.id.clone(),
        validation_report_ref: "validation_report_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        created_at: "2026-07-19T00:00:00Z".to_string(),
    };
    let ordered_logical_work_item_ids = vec![upstream.id.clone(), current.id.clone()];
    let work_item_projections = BTreeMap::from([
        (upstream.id.clone(), upstream_compiled),
        (current.id.clone(), compiled),
    ]);
    let compiled_plan = CompiledPlanProjections {
        human: HumanGroupProjection {
            plan_id: plan.id.clone(),
            goal: "Plan repair fixture".to_string(),
            split_reason: "Fixture publishes complete Schema v2 revisions".to_string(),
            work_items: ordered_logical_work_item_ids
                .iter()
                .map(|logical_id| {
                    let projection = &work_item_projections[logical_id].human;
                    HumanGroupWorkItemSummary {
                        logical_work_item_id: logical_id.clone(),
                        title: projection.title.clone(),
                        goal: projection.goal.clone(),
                        depends_on: dependency_graph
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
            plan_id: plan.id.clone(),
            ordered_logical_work_item_ids: ordered_logical_work_item_ids.clone(),
            dependency_edges: dependency_graph.edges.clone(),
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
            plan_id: plan.id.clone(),
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
            dependency_edges: dependency_graph.edges.clone(),
            design_traceability_refs: Vec::new(),
        },
    };
    let plan_hashes = plan_projection_hashes(&compiled_plan).unwrap();
    revision_store
        .put_plan_projection_bundle(
            &plan,
            &PlanProjectionBundle {
                id: plan_revision.plan_projection_bundle_id.clone(),
                plan_revision_id: plan_revision.id.clone(),
                dependency_graph_revision_id: plan_revision.dependency_graph_revision_id.clone(),
                work_item_projection_bundle_refs: vec![
                    upstream_revision.work_item_projection_bundle_id.clone(),
                    current_revision.work_item_projection_bundle_id.clone(),
                ],
                human_group_projection: compiled_plan.human,
                coder_group_context: compiled_plan.coder,
                reviewer_group_matrix: compiled_plan.reviewer,
                human_group_projection_hash: plan_hashes.human,
                coder_group_context_hash: plan_hashes.coder,
                reviewer_group_matrix_hash: plan_hashes.reviewer,
                compiler_version: "plan-projection-compiler-v1".to_string(),
                created_at: "2026-07-19T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    revision_store
        .put_plan_revision(&plan, &plan_revision)
        .unwrap();
    let plan = revision_store
        .set_active_plan_revision(&plan, "plan_revision_0001")
        .unwrap();
    store
        .save_plan_binding(
            &attempt,
            &CodingAttemptPlanBinding {
                attempt_id: attempt.id.clone(),
                plan_id: plan.id.clone(),
                bound_plan_revision_id: "plan_revision_0001".to_string(),
                applied_amendment_ids: Vec::new(),
                updated_at: "2026-07-19T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    let unit = store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: attempt.id.clone(),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            plan_id: plan.id.clone(),
            logical_work_item_id: current.id.clone(),
            work_item_revision_id: current_revision.id.clone(),
            dependency_logical_work_item_ids: if with_dependency {
                vec![upstream.id.clone()]
            } else {
                Vec::new()
            },
            order_index: 1,
            status: CodingExecutionUnitStatus::Running,
        })
        .unwrap();
    let mut attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    attempt.status = CodingAttemptStatus::Running;
    attempt.stage = CodingExecutionStage::CodeReview;
    attempt.active_unit_id = Some(unit.id.clone());
    attempt.current_work_item_id = Some(current.id.clone());
    store.save_coding_attempt(&attempt).unwrap();
    store
        .create_coding_unit_run(
            &attempt,
            &CodingUnitRun {
                id: "coding_unit_run_0001".to_string(),
                unit_id: unit.id,
                execution_no: 1,
                work_item_revision_id: current_revision.id,
                resolved_handoff_revision_ids: Vec::new(),
                canonical_contract_hash: bundle.canonical_contract_hash,
                projection_bundle_id: bundle.id,
                projection_compiler_version: bundle.compiler_version,
                coder_provider_renderer_version:
                    crate::product::work_item_projection::renderer_for(&ProviderName::Codex)
                        .renderer_version()
                        .to_string(),
                reviewer_provider_renderer_version:
                    crate::product::work_item_projection::renderer_for(&ProviderName::ClaudeCode)
                        .renderer_version()
                        .to_string(),
                internal_reviewer_provider_renderer_version: None,
                coder_projection_hash: hashes.coder,
                reviewer_projection_hash: hashes.reviewer,
                coder_execution_context_hash: None,
                reviewer_execution_context_hash: None,
                internal_reviewer_execution_context_hash: None,
                status: CodingUnitRunStatus::Running,
                unit_rework_count: 0,
                verification_retry_count: 0,
                operational_retry_count: 0,
                plan_repair_count: 0,
                start_commit: Some("commit_0001".to_string()),
                completion_commit: None,
                created_at: "2026-07-19T00:00:00Z".to_string(),
                updated_at: "2026-07-19T00:00:00Z".to_string(),
            },
        )
        .unwrap();
    let lifecycle = LifecycleStore::new(paths.clone());
    let plan_session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            entity_id: plan.id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .unwrap();
    lifecycle
        .save_artifact_versions(
            &plan_session.id,
            &[crate::web::workspace_ws_types::ArtifactVersion {
                version: 1,
                payload:
                    crate::web::workspace_ws_types::ArtifactPayload::WorkItemRevisionHistory {
                        history: Box::new(
                            crate::web::workspace_ws_types::WorkItemRevisionHistoryDto {
                                entries: vec![
                                    crate::web::workspace_ws_types::WorkItemHistoryEntryDto {
                                        kind: crate::web::workspace_ws_types::WorkItemHistoryEntryKind::WorkItemRevision,
                                        id: upstream_revision.id,
                                        logical_work_item_id: upstream.id,
                                        related_revision_id: None,
                                        summary: "Compiled upstream WorkItem revision".to_string(),
                                        created_at: "2026-07-19T00:00:00Z".to_string(),
                                    },
                                    crate::web::workspace_ws_types::WorkItemHistoryEntryDto {
                                        kind: crate::web::workspace_ws_types::WorkItemHistoryEntryKind::WorkItemRevision,
                                        id: "work_item_revision_current".to_string(),
                                        logical_work_item_id: current.id,
                                        related_revision_id: None,
                                        summary: "Compiled current WorkItem revision".to_string(),
                                        created_at: "2026-07-19T00:00:00Z".to_string(),
                                    },
                                ],
                            },
                        ),
                    },
                generated_by: ProviderName::Codex,
                reviewed_by: None,
                review_verdict: None,
                confirmed_by: None,
                is_current: false,
                created_at: "2026-07-19T00:00:00Z".to_string(),
                source_node_id: "timeline_node_compile".to_string(),
            }],
        )
        .unwrap();
    let (event_tx, event_rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    PlanRepairFixture {
        _tmp: tmp,
        store,
        revision_store,
        attempt,
        plan,
        projection,
        event_rx,
        engine,
    }
}

pub(super) fn internal_plan_defect_review(
    attempt: &CodingExecutionAttempt,
    mut finding: ReviewFinding,
) -> InternalPrReview {
    finding.source_stage = CodingExecutionStage::InternalPrReview;
    InternalPrReview {
        id: "internal_pr_review_0001".to_string(),
        attempt_id: attempt.id.clone(),
        review_request_id: "review_request_0001".to_string(),
        verdict: ReviewVerdict::Blocked,
        findings: vec![finding],
        impact_scope: Vec::new(),
        pr_description: String::new(),
        commit_message_suggestion: String::new(),
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "group final review found plan defect".to_string(),
        created_at: "2026-07-19T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
    }
}

pub(super) fn seed_completed_upstream_binding(fixture: &PlanRepairFixture) {
    let revision = fixture
        .revision_store
        .get_work_item_revision(&fixture.plan, "wi_upstream", "work_item_revision_upstream")
        .unwrap();
    let bundle = fixture
        .revision_store
        .get_work_item_projection_bundle(&fixture.plan, &revision.work_item_projection_bundle_id)
        .unwrap();
    let unit = fixture
        .store
        .create_coding_unit(CreateCodingExecutionUnitInput {
            attempt_id: fixture.attempt.id.clone(),
            project_id: fixture.attempt.project_id.clone(),
            issue_id: fixture.attempt.issue_id.clone(),
            plan_id: fixture.plan.id.clone(),
            logical_work_item_id: "wi_upstream".to_string(),
            work_item_revision_id: revision.id.clone(),
            dependency_logical_work_item_ids: Vec::new(),
            order_index: 0,
            status: CodingExecutionUnitStatus::Completed,
        })
        .unwrap();
    fixture
        .store
        .create_coding_unit_run(
            &fixture.attempt,
            &CodingUnitRun {
                id: "coding_unit_run_upstream".to_string(),
                unit_id: unit.id,
                execution_no: 1,
                work_item_revision_id: revision.id,
                resolved_handoff_revision_ids: Vec::new(),
                canonical_contract_hash: bundle.canonical_contract_hash,
                projection_bundle_id: bundle.id,
                projection_compiler_version: bundle.compiler_version,
                coder_provider_renderer_version: "coder-v1".to_string(),
                reviewer_provider_renderer_version: "reviewer-v1".to_string(),
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
                start_commit: Some("commit_0000".to_string()),
                completion_commit: Some("commit_0001".to_string()),
                created_at: "2026-07-19T00:00:00Z".to_string(),
                updated_at: "2026-07-19T00:00:00Z".to_string(),
            },
        )
        .unwrap();
}

fn logical_item(id: &str, plan_id: &str) -> LogicalWorkItem {
    LogicalWorkItem {
        id: id.to_string(),
        plan_id: plan_id.to_string(),
        title: id.to_string(),
        active_revision_id: None,
        created_at: "2026-07-19T00:00:00Z".to_string(),
        updated_at: "2026-07-19T00:00:00Z".to_string(),
    }
}

fn contract(logical_id: &str) -> CanonicalWorkItemContract {
    CanonicalWorkItemContract {
        schema_version: 1,
        identity: WorkItemContractIdentity {
            logical_work_item_id: logical_id.to_string(),
            title: logical_id.to_string(),
            kind: "implementation".to_string(),
        },
        goal: WorkItemGoal {
            summary: logical_id.to_string(),
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
        blocker_rules: Vec::new(),
        design_traceability: Vec::new(),
    }
}

fn work_item_revision(
    item: &LogicalWorkItem,
    id: &str,
    contract: CanonicalWorkItemContract,
    bundle_id: &str,
) -> WorkItemRevision {
    WorkItemRevision {
        id: id.to_string(),
        logical_work_item_id: item.id.clone(),
        source_draft_revision_id: format!("draft_{id}"),
        canonical_contract_hash: canonical_contract_hash(&contract).unwrap(),
        canonical_contract: contract,
        work_item_projection_bundle_id: bundle_id.to_string(),
        verification_plan_revision_id: format!("verification_{id}"),
        created_at: "2026-07-19T00:00:00Z".to_string(),
    }
}

pub(super) fn plan_defect_finding(evidence_ref: &str) -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Error,
        file_path: None,
        line: None,
        message: "upstream contract lacks required capability".to_string(),
        required_action: None,
        source_stage: CodingExecutionStage::CodeReview,
        evidence: vec![evidence_ref.to_string()],
        plan_defect_evidence: vec![PlanDefectEvidence {
            kind: "review".to_string(),
            source_ref: evidence_ref.to_string(),
            message: "missing capability".to_string(),
        }],
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
        defect_class: PlanDefectClass::UpstreamContractInvalid,
        reason_code: Some("upstream_contract_capability_missing".to_string()),
        contract_refs: vec!["contract_upstream".to_string()],
        capability_refs: vec!["capability_missing".to_string()],
        repair_target: Some(RepairTarget {
            kind: RepairTargetKind::UpstreamWorkItem,
            logical_work_item_ids: vec!["wi_upstream".to_string()],
            work_item_revision_ids: vec!["work_item_revision_upstream".to_string()],
        }),
        recommended_route: PlanDefectRoute::PlanRepair,
        confidence: Some(PlanDefectConfidence::High),
    }
}

pub(super) fn plan_defect_report(finding: ReviewFinding) -> CodeReviewReport {
    CodeReviewReport {
        id: "code_review_report_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        round: 1,
        verdict: ReviewVerdict::Blocked,
        findings: vec![finding],
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "plan defect".to_string(),
        created_at: "2026-07-19T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
    }
}
