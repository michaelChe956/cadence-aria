#[tokio::test]
async fn compile_recovery_abort_rejects_prepared_publication_and_preserves_continue_path() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let outline_payload = engine.session.artifact.clone().unwrap();
    engine.update_artifact(outline_payload).await;
    let (compile_tx, accepted_drafts) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_abort_prepared_publication",
        "2026-07-17T00:02:00Z",
    );
    let plan_store = engine.work_item_plan_store().unwrap();
    plan_store.put_compile_transaction(&compile_tx).unwrap();
    let revision_store = engine.revision_store();
    let _failpoint = revision_store.register_initial_plan_publication_failpoint(
        "project_0001",
        "issue_0001",
        &plan_id,
        &compile_tx.compile_id,
        crate::product::work_item_revision_store::InitialPlanPublicationCheckpoint::LineageWritten,
    );

    let publication_error = engine
        .compile_initial_plan_revision(&accepted_drafts)
        .unwrap_err();
    assert!(publication_error.to_string().contains("LineageWritten"));
    assert!(engine.mark_latest_compile_transaction_recovery_required(
        &publication_error.to_string()
    ));
    engine
        .enter_work_item_plan_compile_recovery(Some(publication_error.to_string()))
        .await;

    let abort_error = engine
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::AbortAndRollback,
            Some("try unsafe rollback".to_string()),
        )
        .await
        .unwrap_err();

    assert!(abort_error.contains("abort_and_rollback is not allowed"));
    assert!(abort_error.contains("Continue or HumanTriage"));
    let recovery_tx = plan_store
        .get_compile_transaction(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_tx.compile_id,
        )
        .unwrap();
    assert_eq!(recovery_tx.status, WorkItemPlanCompileStatus::RecoveryRequired);
    assert_eq!(recovery_tx.step_cursor, "committing");
    assert_eq!(
        revision_store
            .get_plan_lineage("project_0001", "issue_0001", &plan_id)
            .unwrap()
            .active_revision_id,
        None
    );
    assert_eq!(
        revision_store
            .get_initial_plan_publication_journal(
                "project_0001",
                "issue_0001",
                &plan_id,
                &compile_tx.compile_id,
            )
            .unwrap()
            .phase,
        crate::product::work_item_revision_store::InitialPlanPublicationPhase::Prepared
    );

    let continued = engine
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .unwrap();

    assert_eq!(continued, WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
    let committed_tx = plan_store
        .get_compile_transaction(
            "project_0001",
            "issue_0001",
            &plan_id,
            &compile_tx.compile_id,
        )
        .unwrap();
    assert_eq!(committed_tx.status, WorkItemPlanCompileStatus::Committed);
    assert_eq!(
        committed_tx.plan_commit_state,
        WorkItemPlanCommitState::Committed
    );
    assert!(
        revision_store
            .get_plan_lineage("project_0001", "issue_0001", &plan_id)
            .unwrap()
            .active_revision_id
            .is_some()
    );
}

#[tokio::test]
async fn work_item_plan_initial_compile_non_topological_outline_finds_tx_and_publishes_topological_projection()
{
    let (_tmp, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let Some(ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate }) =
        engine.session.artifact.as_mut()
    else {
        panic!("expected outline candidate");
    };
    outline_candidate.outline.work_item_outlines.rotate_right(1);
    assert_eq!(
        outline_candidate
            .outline
            .work_item_outlines
            .iter()
            .map(|item| item.outline_id.as_str())
            .collect::<Vec<_>>(),
        vec!["outline_b", "outline_a"]
    );

    let outcome = engine.run_work_item_plan_compile().await.unwrap();

    assert_eq!(
        outcome
            .plan_projection_bundle
            .coder_group_context
            .ordered_logical_work_item_ids,
        vec!["wi_a".to_string(), "wi_b".to_string()]
    );
    let tx = engine
        .work_item_plan_store()
        .unwrap()
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        tx.active_draft_ids,
        vec![
            "draft_outline_a".to_string(),
            "draft_outline_b".to_string()
        ]
    );
    assert_eq!(tx.status, WorkItemPlanCompileStatus::Committed);
}

#[test]
fn work_item_plan_initial_compile_rejects_ambiguous_matching_transactions() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let (first_tx, accepted_drafts) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_ambiguous_first",
        "2026-07-17T00:02:20Z",
    );
    let (second_tx, _) = prepare_initial_compile_transaction(
        &engine,
        &lifecycle,
        &plan_id,
        "compile_ambiguous_second",
        "2026-07-17T00:02:21Z",
    );
    let store = engine.work_item_plan_store().unwrap();
    store.put_compile_transaction(&first_tx).unwrap();
    store.put_compile_transaction(&second_tx).unwrap();

    let error = engine
        .compile_initial_plan_revision(&accepted_drafts)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("current initial plan compile transaction is ambiguous")
    );
    assert!(matches!(
        engine.revision_store().get_plan_lineage(
            "project_0001",
            "issue_0001",
            &plan_id,
        ),
        Err(ProductStoreError::NotFound { .. })
    ));
}
