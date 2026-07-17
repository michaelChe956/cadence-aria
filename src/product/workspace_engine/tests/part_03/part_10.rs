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

#[tokio::test]
async fn work_item_plan_reviewer_prompt_contains_projection_validation_and_contract_flow() {
    let (_tmp, _lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let outcome = engine.run_work_item_plan_compile().await.unwrap();
    engine.session.artifact = Some(ArtifactPayload::WorkItemPlanProjection {
        projection: Box::new(outcome.plan_projection_bundle),
    });

    let input = engine.build_work_item_plan_review_input().unwrap();

    assert!(input.prompt.contains("## Plan Review Context"));
    assert!(input.prompt.contains("Story / Design Traceability"));
    assert!(input.prompt.contains("Canonical Contract Candidates"));
    assert!(input.prompt.contains("Dependency Contract Graph"));
    assert!(input.prompt.contains("PlanProjectionBundle Candidate"));
    assert!(input.prompt.contains("WorkItemProjectionBundle Candidates"));
    assert!(input.prompt.contains("Projection Validation Report"));
    assert!(input.prompt.contains("Contract Delta"));
    assert!(input.prompt.contains("Impact Analysis"));
    assert!(input.prompt.contains("Repair Evidence"));
    assert!(!input.prompt.contains("ReviewerExecutionEnvelope"));
    assert!(!input.prompt.contains("runtime Diff"));
}

#[tokio::test]
async fn work_item_plan_projection_artifact_updates_are_persisted_after_initial_compile() {
    use crate::web::workspace_ws_types::WorkItemHistoryEntryKind;

    let (_tmp, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let (event_tx, mut event_rx) = mpsc::channel(64);
    engine.event_tx = event_tx;

    let outcome = engine.run_work_item_plan_compile().await.unwrap();

    let events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
    let batches = events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::ArtifactBatchUpdate { updates } => Some(updates),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(batches.len(), 1);
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, EngineEvent::ArtifactUpdate { .. }))
    );
    let artifact_events = batches[0];
    assert_eq!(artifact_events.len(), outcome.work_items.len() + 4);
    assert!(matches!(
        artifact_events.first().map(|update| &update.payload),
        Some(ArtifactPayload::WorkItemPlanCompileReport { .. })
    ));
    assert!(
        artifact_events
            .windows(2)
            .all(|pair| pair[0].version < pair[1].version)
    );
    let structured_events = artifact_events
        .iter()
        .map(|update| &update.payload)
        .filter(|payload| {
            matches!(
                payload,
                ArtifactPayload::WorkItemPlanProjection { .. }
                    | ArtifactPayload::WorkItemProjection { .. }
                    | ArtifactPayload::WorkItemRevisionHistory { .. }
                    | ArtifactPayload::ProjectionValidation { .. }
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(structured_events.len(), outcome.work_items.len() + 3);
    assert!(matches!(
        structured_events.last(),
        Some(ArtifactPayload::WorkItemPlanProjection { .. })
    ));

    assert!(engine.artifact_versions.iter().any(|version| {
        matches!(
            &version.payload,
            ArtifactPayload::WorkItemPlanProjection { projection }
                if **projection == outcome.plan_projection_bundle
        )
    }));
    assert_eq!(
        engine
            .artifact_versions
            .iter()
            .filter(|version| matches!(version.payload, ArtifactPayload::WorkItemProjection { .. }))
            .count(),
        outcome.work_items.len()
    );
    assert!(engine.artifact_versions.iter().any(|version| {
        matches!(
            &version.payload,
            ArtifactPayload::ProjectionValidation { report }
                if **report == outcome.projection_validation
        )
    }));
    assert!(engine.artifact_versions.iter().any(|version| {
        matches!(version.payload, ArtifactPayload::WorkItemRevisionHistory { .. })
    }));
    let history = engine.artifact_versions.iter().find_map(|version| match &version.payload {
        ArtifactPayload::WorkItemRevisionHistory { history } => Some(history),
        _ => None,
    });
    let history = history.expect("initial revision history artifact");
    assert_eq!(history.entries.len(), outcome.work_items.len() * 2);
    for item in &outcome.work_items {
        assert!(history.entries.iter().any(|entry| {
            entry.kind == WorkItemHistoryEntryKind::DraftRevision
                && entry.id == item.draft_revision.id
                && entry.logical_work_item_id == item.draft_revision.logical_work_item_id
                && entry.related_revision_id.as_deref() == Some(item.work_item_revision.id.as_str())
        }));
        assert!(history.entries.iter().any(|entry| {
            entry.kind == WorkItemHistoryEntryKind::WorkItemRevision
                && entry.id == item.work_item_revision.id
                && entry.logical_work_item_id == item.work_item_revision.logical_work_item_id
                && entry.related_revision_id.as_deref()
                    == Some(item.work_item_revision.source_draft_revision_id.as_str())
        }));
    }
    assert!(matches!(
        engine.session.artifact,
        Some(ArtifactPayload::WorkItemPlanProjection { .. })
    ));
    assert!(engine.artifact_versions.last().is_some_and(|version| {
        version.is_current
            && matches!(version.payload, ArtifactPayload::WorkItemPlanProjection { .. })
    }));

    let session_record = lifecycle
        .get_workspace_session(&engine.session.session_id)
        .unwrap();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let recovered = WorkspaceEngine::new_persistent(
        engine.checkpoint_store.clone(),
        lifecycle,
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    let WsOutMessage::SessionState {
        artifact,
        artifact_versions,
        ..
    } = recovered.build_session_state()
    else {
        panic!("expected session state");
    };
    let Some(ArtifactPayload::WorkItemPlanProjection { projection }) = artifact else {
        panic!("expected current plan projection in session snapshot");
    };
    assert_eq!(projection.plan_revision_id, outcome.plan_revision.id);
    assert_eq!(
        projection.dependency_graph_revision_id,
        outcome.dependency_graph_revision.id
    );
    assert_eq!(
        projection.compiler_version,
        outcome.plan_projection_bundle.compiler_version
    );
    assert_eq!(artifact_versions.len(), outcome.work_items.len() + 3);
    for item in &outcome.work_items {
        let restored = artifact_versions.iter().find_map(|version| match &version.payload {
            ArtifactPayload::WorkItemProjection { projection }
                if projection.id == item.projection_bundle.id =>
            {
                Some(projection)
            }
            _ => None,
        });
        let restored = restored.expect("work item projection restored in snapshot");
        assert_eq!(
            restored.canonical_contract_hash,
            item.projection_bundle.canonical_contract_hash
        );
        assert_eq!(restored.compiler_version, item.projection_bundle.compiler_version);
    }
    assert!(artifact_versions.iter().any(|version| {
        matches!(version.payload, ArtifactPayload::ProjectionValidation { .. })
    }));
    assert!(artifact_versions.iter().any(|version| {
        matches!(version.payload, ArtifactPayload::WorkItemRevisionHistory { .. })
    }));
    let restored_plan = artifact_versions.iter().find_map(|version| match &version.payload {
        ArtifactPayload::WorkItemPlanProjection { projection } => Some(projection),
        _ => None,
    });
    let restored_plan = restored_plan.expect("plan projection restored in snapshot");
    assert_eq!(
        restored_plan.human_group_projection_hash,
        outcome.plan_projection_bundle.human_group_projection_hash
    );
    assert_eq!(
        restored_plan.coder_group_context_hash,
        outcome.plan_projection_bundle.coder_group_context_hash
    );
    assert_eq!(
        restored_plan.reviewer_group_matrix_hash,
        outcome.plan_projection_bundle.reviewer_group_matrix_hash
    );
}
