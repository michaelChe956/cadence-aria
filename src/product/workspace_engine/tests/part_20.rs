use crate::product::models::HumanPresentationRevision;

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
