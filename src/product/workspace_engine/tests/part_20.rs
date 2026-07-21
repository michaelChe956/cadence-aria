use crate::product::models::HumanPresentationRevision;
#[cfg(unix)]
use std::sync::Barrier;

#[test]
fn build_work_item_plan_outline_review_input_includes_boundary_rules() {
    let (_tmp, _checkpoint_store, _lifecycle, _plan_id, engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_outline_review_boundary");
    let outline_payload = work_item_plan_outline_artifact();
    let ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate } = outline_payload else {
        panic!("expected outline candidate artifact");
    };

    let input = engine
        .build_work_item_plan_outline_review_input(&outline_candidate)
        .expect("outline review input");

    assert_work_item_plan_boundary_rules(&input.prompt);
    assert!(input.prompt.contains("estimated_context_tokens"));
    assert!(input.prompt.contains("session_fit"));
    for field in [
        "\"id\"",
        "\"project_id\"",
        "\"issue_id\"",
        "\"source_story_spec_ids\"",
        "\"source_design_spec_ids\"",
        "\"strategy_summary\"",
        "\"work_item_outlines\"",
        "\"dependency_graph\"",
        "\"risks\"",
        "\"handoff_strategy\"",
        "\"status\"",
        "\"outline_id\"",
        "\"title\"",
        "\"kind\"",
        "\"goal\"",
        "\"scope\"",
        "\"non_goals\"",
        "\"estimated_context_tokens\"",
        "\"session_fit\"",
        "\"exclusive_write_scopes\"",
        "\"forbidden_write_scopes\"",
        "\"depends_on\"",
        "\"verification_intent\"",
        "\"handoff_notes\"",
    ] {
        assert!(
            input.prompt.contains(field),
            "outline reviewer prompt must include complete candidate field {field}"
        );
    }
    for required in [
        "40k",
        "50k",
        "最大内聚",
        "最少拆分",
        "不必要拆分",
        "[outline_unnecessary_split]",
    ] {
        assert!(
            input.prompt.contains(required),
            "outline reviewer prompt must include `{required}`: {}",
            input.prompt
        );
    }
    assert!(input.prompt.contains("severity=must_fix"));
    assert!(input.prompt.contains("target_outline_id"));
    assert!(!input.prompt.contains("小于 20k"));
    for required_contract in [
        "不超过 40k 属正常范围",
        "40001..=50000",
        "超过 50k 必须返回 `revise` 并要求拆分",
        "发现不必要拆分时必须给出 severity=must_fix",
        "message 必须以 [outline_unnecessary_split] 开头",
        "target_outline_id 引用其中一个现有 outline",
        "evidence 列出全部可合并 outline ID",
        "required_action 明确要求合并",
    ] {
        assert!(
            input.prompt.contains(required_contract),
            "outline reviewer prompt must preserve contract `{required_contract}`: {}",
            input.prompt
        );
    }
    assert!(
        !input.prompt.contains("\"code\""),
        "outline review schema must reuse ReviewFinding without a code field: {}",
        input.prompt
    );
    assert!(input.prompt.contains(
        "\"generation_round_id\":\"generation_round_unknown\""
    ));
    assert!(input.prompt.contains("\"target_outline_id\":\"outline id\""));
    assert!(input.prompt.contains("从 findings[].target_outline_id 推导"));
    assert!(
        !input.prompt.contains("\"affects_items\""),
        "new outline review schema should not duplicate affected outline references"
    );
    assert_review_contract(&input, "work_item_plan_outline_review");
}

#[tokio::test]
async fn work_item_human_presentation_revision_changes_only_human_rendering() {
    let (_tmp, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let outcome = engine.run_work_item_plan_compile().await.unwrap();
    let store = engine.revision_store();
    let plan = store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    let before_plan_revision = plan.active_revision_id.clone();
    let before_plan_provider_hashes = (
        outcome.plan_projection_bundle.coder_group_context_hash.clone(),
        outcome
            .plan_projection_bundle
            .reviewer_group_matrix_hash
            .clone(),
    );
    let before_work_items = outcome
        .work_items
        .iter()
        .map(|item| {
            (
                item.work_item_revision.logical_work_item_id.clone(),
                item.work_item_revision.id.clone(),
                item.projection_bundle.id.clone(),
                item.projection_bundle.coder_projection_hash.clone(),
                item.projection_bundle.reviewer_projection_hash.clone(),
            )
        })
        .collect::<Vec<_>>();

    let saved = save_human_presentation_revision(
        &store,
        &plan,
        HumanPresentationRevision {
            id: "human_presentation_revision_0001".to_string(),
            source_plan_projection_bundle_id: Some(outcome.plan_projection_bundle.id.clone()),
            source_work_item_projection_bundle_id: None,
            supersedes: None,
            human_summary: "先稳定核心状态机，再接 API".to_string(),
            why_split: Some("按契约边界解释拆分".to_string()),
            dependency_explanation: vec!["先完成上游状态模型".to_string()],
            risk_explanation: vec!["避免并发修改同一写入范围".to_string()],
            source_refs: outcome
                .plan_projection_bundle
                .human_group_projection
                .source_refs
                .clone(),
            normative: true,
            used_by_provider: true,
            created_at: "2026-07-18T12:00:00Z".to_string(),
        },
    )
    .unwrap();

    assert_eq!(saved.supersedes, None);
    assert!(!saved.normative);
    assert!(!saved.used_by_provider);
    let reloaded_plan = store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(reloaded_plan.active_revision_id, before_plan_revision);
    let reloaded_plan_revision = store
        .get_plan_revision(
            "project_0001",
            "issue_0001",
            &plan_id,
            reloaded_plan.active_revision_id.as_deref().unwrap(),
        )
        .unwrap();
    let reloaded_plan_projection = store
        .get_plan_projection_bundle(&reloaded_plan, &reloaded_plan_revision.plan_projection_bundle_id)
        .unwrap();
    assert_eq!(
        (
            reloaded_plan_projection.coder_group_context_hash,
            reloaded_plan_projection.reviewer_group_matrix_hash,
        ),
        before_plan_provider_hashes
    );
    for (logical_id, revision_id, bundle_id, coder_hash, reviewer_hash) in before_work_items {
        let logical = store
            .get_logical_work_item(&reloaded_plan, &logical_id)
            .unwrap();
        assert_eq!(logical.active_revision_id.as_deref(), Some(revision_id.as_str()));
        let bundle = store
            .get_work_item_projection_bundle(&reloaded_plan, &bundle_id)
            .unwrap();
        assert_eq!(bundle.coder_projection_hash, coder_hash);
        assert_eq!(bundle.reviewer_projection_hash, reviewer_hash);
    }
}

#[tokio::test]
async fn work_item_human_presentation_revision_requires_latest_supersedes_per_scope() {
    let (_tmp, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let outcome = engine.run_work_item_plan_compile().await.unwrap();
    let store = engine.revision_store();
    let plan = store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    let base = &outcome.work_items[0].projection_bundle;
    let revision = |id: &str, supersedes: Option<&str>, summary: &str| HumanPresentationRevision {
        id: id.to_string(),
        source_plan_projection_bundle_id: None,
        source_work_item_projection_bundle_id: Some(base.id.clone()),
        supersedes: supersedes.map(str::to_string),
        human_summary: summary.to_string(),
        why_split: None,
        dependency_explanation: base.human_projection.dependencies.clone(),
        risk_explanation: vec![],
        source_refs: base.human_projection.source_refs.clone(),
        normative: false,
        used_by_provider: false,
        created_at: "2026-07-18T12:00:00Z".to_string(),
    };

    let first = save_human_presentation_revision(
        &store,
        &plan,
        revision("human_presentation_revision_0001", None, "first"),
    )
    .unwrap();
    let stale = save_human_presentation_revision(
        &store,
        &plan,
        revision("human_presentation_revision_0002", None, "stale"),
    )
    .unwrap_err();
    assert!(stale.to_string().contains("supersedes"));
    assert_eq!(
        store
            .get_latest_human_presentation_revision(&plan, &base.id)
            .unwrap(),
        Some(first.clone())
    );

    let second = save_human_presentation_revision(
        &store,
        &plan,
        revision(
            "human_presentation_revision_0003",
            Some(&first.id),
            "second",
        ),
    )
    .unwrap();
    assert_eq!(second.supersedes.as_deref(), Some(first.id.as_str()));
    assert_eq!(
        store
            .get_latest_human_presentation_revision(&plan, &base.id)
            .unwrap(),
        Some(second)
    );
}

#[tokio::test]
async fn work_item_human_presentation_revision_recovers_latest_overlay_in_session_state() {
    let (_tmp, _lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let outcome = engine.run_work_item_plan_compile().await.unwrap();
    let store = engine.revision_store();
    let plan = store
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    let saved = save_human_presentation_revision(
        &store,
        &plan,
        HumanPresentationRevision {
            id: "human_presentation_revision_0001".to_string(),
            source_plan_projection_bundle_id: Some(outcome.plan_projection_bundle.id.clone()),
            source_work_item_projection_bundle_id: None,
            supersedes: None,
            human_summary: "恢复后的解释".to_string(),
            why_split: None,
            dependency_explanation: vec![],
            risk_explanation: vec![],
            source_refs: vec![],
            normative: false,
            used_by_provider: false,
            created_at: "2026-07-18T12:00:00Z".to_string(),
        },
    )
    .unwrap();

    let WsOutMessage::SessionState {
        human_presentation_revisions,
        ..
    } = engine.build_session_state()
    else {
        panic!("expected session state");
    };
    assert_eq!(human_presentation_revisions, vec![saved]);
}

#[cfg(unix)]
#[tokio::test]
async fn work_item_human_presentation_concurrent_stores_commit_exactly_one_revision() {
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let outcome = engine.run_work_item_plan_compile().await.unwrap();
    let paths = lifecycle.app_paths();
    let plan = engine
        .revision_store()
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    let source_bundle_id = outcome.plan_projection_bundle.id.clone();
    let barrier = Arc::new(Barrier::new(2));

    let results = std::thread::scope(|scope| {
        let handles = ["first", "second"].map(|summary| {
            let store = crate::product::work_item_revision_store::WorkItemRevisionStore::new(
                paths.clone(),
            );
            let plan = plan.clone();
            let source_bundle_id = source_bundle_id.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                save_human_presentation_revision(
                    &store,
                    &plan,
                    HumanPresentationRevision {
                        id: String::new(),
                        source_plan_projection_bundle_id: Some(source_bundle_id),
                        source_work_item_projection_bundle_id: None,
                        supersedes: None,
                        human_summary: summary.to_string(),
                        why_split: None,
                        dependency_explanation: vec![],
                        risk_explanation: vec![],
                        source_refs: vec![],
                        normative: false,
                        used_by_provider: false,
                        created_at: "2026-07-18T12:00:00Z".to_string(),
                    },
                )
            })
        });
        handles.map(|handle| handle.join().unwrap())
    });

    let successes = results
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .collect::<Vec<_>>();
    let conflicts = results
        .iter()
        .filter_map(|result| result.as_ref().err())
        .collect::<Vec<_>>();
    assert_eq!(successes.len(), 1);
    assert_eq!(conflicts.len(), 1);
    assert!(conflicts[0].to_string().contains("supersedes"));

    let revision_root = paths
        .issue_root("project_0001", "issue_0001")
        .join("work-item-revisions")
        .join(&plan_id)
        .join("human-presentation-revisions");
    let persisted_revision_files = std::fs::read_dir(&revision_root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    assert_eq!(persisted_revision_files.len(), 1);

    let verification_store =
        crate::product::work_item_revision_store::WorkItemRevisionStore::new(paths);
    let latest = verification_store
        .get_latest_human_presentation_revision(&plan, &source_bundle_id)
        .unwrap()
        .unwrap();
    assert_eq!(latest, *successes[0]);
    assert_eq!(latest.id, "human_presentation_revision_0001");
    assert_eq!(
        persisted_revision_files[0]
            .path()
            .file_stem()
            .and_then(|value| value.to_str()),
        Some(latest.id.as_str())
    );
}

#[tokio::test]
async fn work_item_human_presentation_persistent_restart_recovers_each_latest_overlay() {
    let (tmp, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let outcome = engine.run_work_item_plan_compile().await.unwrap();

    let plan_first = save_presentation_command(
        &engine,
        outcome.plan_projection_bundle.id.clone(),
        HumanPresentationScope::Plan,
        None,
        "plan first",
    );
    let plan_latest = save_presentation_command(
        &engine,
        outcome.plan_projection_bundle.id.clone(),
        HumanPresentationScope::Plan,
        Some(plan_first.id.clone()),
        "plan latest",
    );

    let mut expected = std::collections::BTreeMap::from([(
        outcome.plan_projection_bundle.id.clone(),
        plan_latest,
    )]);
    for (index, item) in outcome.work_items.iter().take(2).enumerate() {
        let first = save_presentation_command(
            &engine,
            item.projection_bundle.id.clone(),
            HumanPresentationScope::WorkItem,
            None,
            &format!("work item {index} first"),
        );
        let latest = save_presentation_command(
            &engine,
            item.projection_bundle.id.clone(),
            HumanPresentationScope::WorkItem,
            Some(first.id),
            &format!("work item {index} latest"),
        );
        expected.insert(item.projection_bundle.id.clone(), latest);
    }
    assert_eq!(expected.len(), 3);

    let session_record = lifecycle
        .get_workspace_session(&engine.session.session_id)
        .unwrap();
    drop(engine);
    let (event_tx, _event_rx) = mpsc::channel(8);
    let recovered = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(tmp.path().to_path_buf())),
        lifecycle.clone(),
        event_tx,
        WorkspaceSession::from_record(session_record),
    );
    let WsOutMessage::SessionState {
        human_presentation_revisions,
        ..
    } = recovered.build_session_state()
    else {
        panic!("expected session state");
    };
    let recovered = human_presentation_revisions
        .into_iter()
        .map(|revision| {
            let source = revision
                .source_plan_projection_bundle_id
                .clone()
                .or(revision.source_work_item_projection_bundle_id.clone())
                .unwrap();
            (source, revision)
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(recovered, expected);

    for (workspace_type, entity_id) in [
        (WorkspaceType::Story, "story_spec_0001"),
        (WorkspaceType::Design, "design_spec_0001"),
        (WorkspaceType::WorkItem, "work_item_0001"),
    ] {
        assert_non_plan_restart_has_no_human_presentations(
            tmp.path(),
            &lifecycle,
            workspace_type,
            entity_id,
        );
    }
}

fn save_presentation_command(
    engine: &WorkspaceEngine,
    source_projection_bundle_id: String,
    scope: HumanPresentationScope,
    supersedes: Option<String>,
    human_summary: &str,
) -> HumanPresentationRevision {
    engine
        .save_human_presentation_revision_command(SaveHumanPresentationRevision {
            source_projection_bundle_id,
            scope,
            supersedes,
            human_summary: human_summary.to_string(),
            why_split: None,
            dependency_explanation: vec![],
            risk_explanation: vec![],
            source_refs: vec![],
        })
        .unwrap()
}

fn assert_non_plan_restart_has_no_human_presentations(
    checkpoint_root: &std::path::Path,
    lifecycle: &LifecycleStore,
    workspace_type: WorkspaceType,
    entity_id: &str,
) {
    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: entity_id.to_string(),
            workspace_type,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
        })
        .unwrap();
    let session_id = session_record.id.clone();
    let (initial_tx, _initial_rx) = mpsc::channel(8);
    let initial = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(checkpoint_root.to_path_buf())),
        lifecycle.clone(),
        initial_tx,
        WorkspaceSession::from_record(session_record),
    );
    drop(initial);

    let persisted = lifecycle.get_workspace_session(&session_id).unwrap();
    let (restart_tx, _restart_rx) = mpsc::channel(8);
    let restarted = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(checkpoint_root.to_path_buf())),
        lifecycle.clone(),
        restart_tx,
        WorkspaceSession::from_record(persisted),
    );
    let WsOutMessage::SessionState {
        human_presentation_revisions,
        ..
    } = restarted.build_session_state()
    else {
        panic!("expected session state");
    };
    assert!(human_presentation_revisions.is_empty());
}
