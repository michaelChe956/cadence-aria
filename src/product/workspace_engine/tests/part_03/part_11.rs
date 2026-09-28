#[derive(Debug, Clone, Copy)]
enum ProjectionPayloadHashCorruption {
    CanonicalContract,
    WorkItemHuman,
    WorkItemCoder,
    WorkItemReviewer,
    PlanHuman,
    PlanCoder,
    PlanReviewer,
}

#[tokio::test]
async fn work_item_plan_reviewer_prompt_rejects_projection_payload_hash_corruption() {
    let mut accepted = Vec::new();
    for corruption in [
        ProjectionPayloadHashCorruption::CanonicalContract,
        ProjectionPayloadHashCorruption::WorkItemHuman,
        ProjectionPayloadHashCorruption::WorkItemCoder,
        ProjectionPayloadHashCorruption::WorkItemReviewer,
        ProjectionPayloadHashCorruption::PlanHuman,
        ProjectionPayloadHashCorruption::PlanCoder,
        ProjectionPayloadHashCorruption::PlanReviewer,
    ] {
        let (_tmp, lifecycle, plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        let outcome = engine.run_work_item_plan_compile().await.unwrap();
        let plan_root = persisted_plan_review_context_root(&lifecycle, &plan_id);
        engine.session.artifact = Some(ArtifactPayload::WorkItemPlanProjection {
            projection: Box::new(outcome.plan_projection_bundle.clone()),
        });

        match corruption {
            ProjectionPayloadHashCorruption::CanonicalContract => {
                let mut revision = outcome.work_items[0].work_item_revision.clone();
                revision
                    .canonical_contract
                    .goal
                    .summary
                    .push_str(" tampered without hash update");
                overwrite_persisted_review_context_json(
                    plan_root
                        .join("logical-work-items")
                        .join(&revision.logical_work_item_id)
                        .join("revisions")
                        .join(format!("{}.json", revision.id)),
                    &revision,
                );
            }
            ProjectionPayloadHashCorruption::WorkItemHuman
            | ProjectionPayloadHashCorruption::WorkItemCoder
            | ProjectionPayloadHashCorruption::WorkItemReviewer => {
                let mut bundle = outcome.work_items[0].projection_bundle.clone();
                match corruption {
                    ProjectionPayloadHashCorruption::WorkItemHuman => bundle
                        .human_projection
                        .title
                        .push_str(" tampered without hash update"),
                    ProjectionPayloadHashCorruption::WorkItemCoder => bundle
                        .coder_projection
                        .objective
                        .push_str(" tampered without hash update"),
                    ProjectionPayloadHashCorruption::WorkItemReviewer => bundle
                        .reviewer_projection
                        .criterion_refs
                        .push("criterion_tampered".to_string()),
                    _ => unreachable!(),
                }
                overwrite_persisted_review_context_json(
                    plan_root
                        .join("work-item-projection-bundles")
                        .join(format!("{}.json", bundle.id)),
                    &bundle,
                );
            }
            ProjectionPayloadHashCorruption::PlanHuman
            | ProjectionPayloadHashCorruption::PlanCoder
            | ProjectionPayloadHashCorruption::PlanReviewer => {
                let mut projection = outcome.plan_projection_bundle.clone();
                match corruption {
                    ProjectionPayloadHashCorruption::PlanHuman => projection
                        .human_group_projection
                        .goal
                        .push_str(" tampered without hash update"),
                    ProjectionPayloadHashCorruption::PlanCoder => projection
                        .coder_group_context
                        .ordered_logical_work_item_ids
                        .reverse(),
                    ProjectionPayloadHashCorruption::PlanReviewer => {
                        projection.reviewer_group_matrix.work_items.reverse()
                    }
                    _ => unreachable!(),
                }
                overwrite_persisted_review_context_json(
                    plan_root
                        .join("plan-projection-bundles")
                        .join(format!("{}.json", projection.id)),
                    &projection,
                );
                engine.session.artifact = Some(ArtifactPayload::WorkItemPlanProjection {
                    projection: Box::new(projection),
                });
            }
        }

        if engine.build_work_item_plan_review_input().is_ok() {
            accepted.push(format!("{corruption:?}"));
        }
    }

    assert!(
        accepted.is_empty(),
        "Plan Review Context accepted payloads with stale hashes: {accepted:?}"
    );
}

// ---------------------------------------------------------------------------
// C1 Task 8（REQ-C1-CHILD-01）：compile child/session 精确绑定与跨代新建。
// ---------------------------------------------------------------------------

fn c1_child_enrollment_target() -> crate::product::logical_codebase::EnrollmentTarget {
    crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
        repository_id: "repo_physical_c1_child".to_string(),
    }
}

fn c1_child_enrollment_source() -> crate::product::models::automation::EnrollmentSource {
    crate::product::models::automation::EnrollmentSource {
        stories: vec![],
        designs: vec![],
    }
}

fn c1_child_enrollment_options() -> crate::product::models::automation::EnrollmentOptions {
    crate::product::models::automation::EnrollmentOptions {
        author_provider: ProviderName::Fake,
        reviewer_provider: ProviderName::Fake,
        review_rounds: 1,
        superpowers_enabled: false,
        openspec_enabled: false,
        plan_options: crate::product::models::lifecycle::IssueWorkItemPlanOptions {
            include_integration_tests: true,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: false,
        },
    }
}

/// enable（显式 target → binding v1）+ bind_plan → durable binding v1 精确
/// 指向 fixture plan/session；返回 enrollment store 供 rebind。
fn enable_c1_child_enrollment(
    lifecycle: &LifecycleStore,
    plan_id: &str,
    session_id: &str,
) -> crate::product::issue_automation_store::IssueAutomationStore {
    let store = crate::product::issue_automation_store::IssueAutomationStore::new(
        lifecycle.app_paths(),
    );
    store
        .compare_and_set(
            "project_0001",
            "issue_0001",
            None,
            crate::product::models::automation::EnrollmentWriteCommand::Enable {
                selection_key: "c1_child_binding_fixture".to_string(),
                source: c1_child_enrollment_source(),
                options: c1_child_enrollment_options(),
                logical_repository_id: crate::product::logical_codebase::LogicalRepositoryId(
                    uuid::Uuid::nil(),
                ),
                target: Some(c1_child_enrollment_target()),
            },
        )
        .unwrap();
    store
        .bind_plan("project_0001", "issue_0001", 1, plan_id, session_id)
        .unwrap();
    store
}

fn c1_child_session_path(lifecycle: &LifecycleStore, session_id: &str) -> PathBuf {
    lifecycle
        .app_paths()
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("workspace-sessions")
        .join(format!("{session_id}.json"))
}

async fn run_c1_child_finalize(
    engine: &mut WorkspaceEngine,
    compile_tx: &mut WorkItemPlanCompileTransaction,
) -> Result<(), String> {
    let plan_store = engine.work_item_plan_store().unwrap();
    compile_tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
    compile_tx.failure_reason = Some("simulated crash before child sessions".to_string());
    plan_store
        .put_compile_transaction(compile_tx)
        .unwrap();
    engine
        .enter_work_item_plan_compile_recovery(Some("simulated crash".to_string()))
 .await;
    engine
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .map(|_| ())
}

/// 旧 child 仅 entity_id 相同但 binding v1：当前 v2 compile 不复用，创建
/// 一个 v2 child；旧 child/session binding JSON 逐字段不变（A13/REQ-C1-CHILD-01）。
#[tokio::test]
async fn finalizer_rejects_old_binding_child() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let session_id = engine.session.session_id.clone();
    let enrollment_store = enable_c1_child_enrollment(&lifecycle, &plan_id, &session_id);
    let enrollment_id = enrollment_store
        .get("project_0001", "issue_0001")
        .unwrap()
        .expect("enrollment fixture enabled")
        .enrollment_id;

    let compile_id = "compile_c1_child_binding";
    let (mut compile_tx, accepted_drafts) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        &compile_id,
        "2026-09-29T00:00:00Z",
    );
    engine
        .work_item_plan_store()
        .unwrap()
        .put_compile_transaction(&compile_tx)
        .unwrap();
    let published = engine
        .compile_initial_plan_revision(&accepted_drafts)
        .unwrap();
    let first_logical_id = published.work_items[0]
        .work_item_revision
        .logical_work_item_id
        .clone();

    // 预置旧代 v1 child：仅 entity_id 与当前 compile 派生 id 相同。
    let stale_child = lifecycle
        .create_workspace_child_session(CreateWorkItemChildSessionInput {
            session: CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: first_logical_id.clone(),
                workspace_type: WorkspaceType::WorkItem,
                author_provider: ProviderName::Fake,
                reviewer_provider: ProviderName::Fake,
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
            child_binding: crate::product::models::ChildBindingIdentity {
                plan_id: plan_id.clone(),
                plan_revision_id: "plan_revision_stale_v1".to_string(),
                logical_work_item_id: first_logical_id.clone(),
                work_item_revision_id: "work_item_revision_stale_v1".to_string(),
                binding_version: 1,
                enrollment_id: enrollment_id.clone(),
                target: c1_child_enrollment_target(),
            },
        })
        .unwrap();
    let stale_child_path = c1_child_session_path(&lifecycle, &stale_child.id);
    let stale_child_json_before = std::fs::read_to_string(&stale_child_path).unwrap();

    // 显式换代：同 plan/session 换 provider，binding v1 → v2。
    enrollment_store
        .rebind(
            "project_0001",
            "issue_0001",
            crate::product::models::automation::EnrollmentRebindRequest {
                command_id: "cmd_rebind_c1_child_v2".to_string(),
                expected_policy_revision: 2,
                expected_binding_version: 1,
                binding: crate::product::models::automation::EnrollmentBindingIdentityInput {
                    plan_id: plan_id.clone(),
                    session_id: session_id.clone(),
                    source: c1_child_enrollment_source(),
                    target: c1_child_enrollment_target(),
                    author_provider: ProviderName::Fake,
                    reviewer_provider: ProviderName::Fake,
                },
                reason: "c1 child binding fixture rebind".to_string(),
            },
        )
        .unwrap();

    // 从原 compile transaction 恢复 finalization（同 compile_id → 同 logical ids）。
    run_c1_child_finalize(&mut engine, &mut compile_tx)
        .await
        .unwrap();

    // 旧 child/session binding JSON 逐字段不变（不迁移、不覆盖、不删除）。
    let stale_child_json_after = std::fs::read_to_string(&stale_child_path).unwrap();
    assert_eq!(stale_child_json_before, stale_child_json_after);

    // 当前 v2 compile 不复用旧 child：为同一 logical id 新建 v2 child。
    let sessions_for_first = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .unwrap()
        .into_iter()
        .filter(|session| {
            session.workspace_type == WorkspaceType::WorkItem
                && session.entity_id == first_logical_id
        })
        .collect::<Vec<_>>();
    assert_eq!(
        sessions_for_first.len(),
        2,
        "cross-generation compile must create a fresh child"
    );
    let v2_binding = sessions_for_first
        .iter()
        .filter_map(|session| session.work_item_child_binding.as_ref())
        .find(|binding| binding.binding_version == 2)
        .expect("current generation child carries binding v2 identity");
    assert_eq!(v2_binding.plan_id, plan_id);
    assert_eq!(v2_binding.plan_revision_id, published.plan_revision.id);
    assert_eq!(
        v2_binding.work_item_revision_id,
        published.work_items[0].work_item_revision.id
    );
    assert_eq!(v2_binding.logical_work_item_id, first_logical_id);
    assert_eq!(v2_binding.enrollment_id, enrollment_id);
    assert_eq!(v2_binding.target, c1_child_enrollment_target());
}
