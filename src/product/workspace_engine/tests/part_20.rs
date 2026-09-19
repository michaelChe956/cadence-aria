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

// 退役留档（T5/REQ-RET-02）：`work_item_human_presentation_revision_changes_only_human_rendering` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_human_presentation_revision_requires_latest_supersedes_per_scope` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_human_presentation_revision_recovers_latest_overlay_in_session_state` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_human_presentation_concurrent_stores_commit_exactly_one_revision` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：`work_item_human_presentation_persistent_restart_recovers_each_latest_overlay` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：save_presentation_command 夹具随保存命令族退役（唯一调用方测试已退役）。

fn assert_non_plan_restart_has_no_human_presentations(
    checkpoint_root: &std::path::Path,
    lifecycle: &LifecycleStore,
    workspace_type: WorkspaceType,
    entity_id: &str,
) {
    let session_record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput { project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: entity_id.to_string(),
        workspace_type,
        author_provider: ProviderName::ClaudeCode,
        reviewer_provider: ProviderName::Codex,
        review_rounds: 1,
        superpowers_enabled: false, openspec_enabled: false, work_item_plan_options: None, })
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
