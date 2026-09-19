// F5 契约机械校验前移（contract_prerevision）用例按大文件守卫(>1200 行)
// 拆分至本文件，经 include! 内联进父模块。

/// F5 回灌扩展：canonical 契约机械校验前移（3.6 矩阵 codex×重 根治）。
/// 契约缺口在本轮 author 落盘即产生机械 Revise verdict，经 complete_review
/// 既有 ingestion 驱动 F5-A 修订轮回灌；连续 2 轮同指纹由既有 policy
/// RepeatedFingerprint 人工门兜底；干净候选零变化。
mod contract_prerevision {
    use super::*;
    use crate::product::workspace_engine::WorkspaceStage;
    use crate::web::workspace_ws_types::TimelineNodeType;

    pub(super) const REP4_FIXTURE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
    ));

    /// campaign 同口径：终端 WI-003 handoff 提供行清空（否则 rep4 自带
    /// unconsumed_required_handoff Error，无法充当「干净」基线）。
    pub(super) fn clean_candidate() -> String {
        REP4_FIXTURE.replace(
            "- provided_contract_refs: contract.levels-integration",
            "- provided_contract_refs: []",
        )
    }

    /// 干净基线 + WI-001 反向消费 WI-002（contract.level-selector）制造依赖环
    /// ——非机械可修缺口（DEF-PVR-ALL 补齐器边界外），驱动机械 verdict 走
    /// 模型返修；capability 类缺口已由 contract_autorepair 确定性补齐，
    /// 不再抵达本通道。
    fn cycle_gap_candidate(round: usize) -> String {
        clean_candidate()
            .replace(
                "### Inputs\n\n### Outputs\n- contract_id: contract.levels-api",
                "### Inputs\n- contract_id: contract.level-selector\n- provider_logical_work_item_id: WI-002\n- required_capabilities: ui.level-selector.rendered\n- compatibility_policy: require_all\n\n### Outputs\n- contract_id: contract.levels-api",
            )
            .replace(
                "Backend levels API",
                &format!("Backend levels API round-{round}"),
            )
    }

    /// 驱动 complete_single_candidate_work_item_plan_author 需要的 durable 形态：
    /// Generate 相位、无候选 refs（首轮流）；single_candidate_record 的 Evaluate
    /// 形态供 review 完成路径，不适合 author 落盘 CAS。
    pub(super) fn author_round_record(lifecycle: &LifecycleStore, engine: &mut WorkspaceEngine) {
        let mut record = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load session");
        record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
        record.run_policy = RunPolicy::Interactive;
        record.single_candidate_phase = Some(SingleCandidatePhase::Generate);
        record.work_item_plan_source_revision_ref = None;
        record.plan_candidate_ir_ref = None;
        record.mechanical_report_ref = None;
        write_json(
            &lifecycle
                .app_paths()
                .issue_root(&record.project_id, &record.issue_id)
                .join("workspace-sessions")
                .join(format!("{}.json", record.id)),
            &record,
        )
        .expect("persist author round session");
        engine.session = WorkspaceSession::from_record(record);
    }

    /// E3-2/E3-5：IR 存在跨 item 契约缺口时，author 落盘即产生机械 verdict：
    /// - latest_review_verdict 被注入（F5-A 修订轮判定命中）；
    /// - verdict 持久化到本轮 ReviewerRun 节点（跨轮指纹对比数据源）；
    /// - policy 路由进 TriggerAggregateRepair：repairs_used 计 1（预算与
    ///   reviewer 返修轮同池），不落 Approval/人工门。
    #[tokio::test]
    async fn contract_gap_drives_mechanical_revision_round_via_complete_review() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);

        let item_count = engine
            .complete_single_candidate_work_item_plan_author(
                cycle_gap_candidate(1),
                "repo_fixture".to_string(),
            )
            .await
            .expect("gapped candidate must still compile and persist");

        assert_eq!(item_count, 3, "rep4 fixture compiles three work items");

        // F5-A：机械 verdict 注入 latest_review_verdict，修订轮判定命中。
        let pending = engine
            .single_candidate_pending_revision_verdict()
            .expect("mechanical contract gap must mark the next author round as revision");
        assert_eq!(pending.verdict, ReviewVerdictType::Revise);
        assert_eq!(pending.review_gate, ReviewGate::RequiresRevision);
        let gap_finding = pending
            .findings
            .iter()
            .find(|finding| finding.message.contains("dependency_cycle"))
            .expect("cycle gap finding must ride the verdict");
        assert!(
            gap_finding.required_action.contains("拆除依赖环"),
            "required_action must name the cycle teardown: {}",
            gap_finding.required_action
        );

        // 机械 verdict 持久化到 ReviewerRun 节点（跨轮指纹对比的数据源）。
        let reviewer_node = engine
            .timeline_nodes
            .iter()
            .rev()
            .find(|node| matches!(node.node_type, TimelineNodeType::ReviewerRun))
            .expect("mechanical revision round must persist a ReviewerRun node");
        let detail = lifecycle
            .load_node_detail_for_issue_session(
                &engine.session().project_id,
                &engine.session().issue_id,
                &engine.session().session_id,
                &reviewer_node.node_id,
            )
            .expect("load reviewer node detail");
        assert!(
            detail.verdict.is_some(),
            "mechanical verdict must be durable for cross-round fingerprint comparison"
        );

        // policy：TriggerAggregateRepair（预算计入），不进 Approval/人工门。
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load persisted session");
        assert_eq!(
            persisted.run_history.repairs_used, 1,
            "mechanical revise round must consume the shared repair budget"
        );
        assert!(
            !persisted.run_history.seen_fingerprints.is_empty(),
            "gap fingerprints must enter the durable seen set for the repeat gate"
        );
        assert_eq!(
            persisted.single_candidate_phase,
            Some(SingleCandidatePhase::Generate),
            "TriggerAggregateRepair must route back to Generate for the author rerun"
        );
        assert!(
            persisted.provider_start_ledger.iter().any(|entry| {
                entry.provider_start_idempotency_key
                    == format!("single_candidate_author:{}:0", persisted.id)
                    && entry.started
            }),
            "TriggerAggregateRepair must atomically preclaim the next author provider key"
        );
        assert!(
            persisted
                .repair_reservation
                .as_ref()
                .is_some_and(|reservation| {
                    reservation.provider_start_idempotency_key
                    == format!("single_candidate_author:{}:0", persisted.id)
                    && reservation.state
                        == crate::product::work_item_plan_policy::RepairReservationState::Reserved
                }),
            "the repair route must mark its preclaimed provider key consumable by the relay"
        );
    }

    /// E3-2：首轮无缺口路径零变化——干净候选不产生 verdict、不消费预算、
    /// 走既有 Evaluate 路由（有 reviewer 时进 CrossReview 等 reviewer run）。
    #[tokio::test]
    async fn clean_candidate_keeps_evaluate_routing_unchanged() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);

        engine
            .complete_single_candidate_work_item_plan_author(
                clean_candidate(),
                "repo_fixture".to_string(),
            )
            .await
            .expect("clean candidate must complete author round");

        assert!(
            engine.single_candidate_pending_revision_verdict().is_none(),
            "clean candidate must not be flagged as a revision round"
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load persisted session");
        assert_eq!(
            persisted.run_history.repairs_used, 0,
            "clean candidate must not consume any repair budget"
        );
        assert!(
            persisted.run_history.seen_fingerprints.is_empty(),
            "clean candidate must not seed any fingerprint"
        );
        // 既有路由形态：session 带 reviewer（Codex），Evaluate 后进 CrossReview
        // 等待 reviewer run；不因机械前移改变。
        assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);
    }

    /// E3-4/E3-5：连续 2 轮相同机械 findings——既有 policy RepeatedFingerprint
    /// 闸门强制人工门（Interactive → EnterHumanGate），不再自动重驱 author，
    /// 预算不再增长（第 2 轮不重复计入 repairs_used）。
    #[tokio::test]
    async fn repeated_contract_gap_lands_in_human_gate_without_further_auto_repair() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);

        engine
            .complete_single_candidate_work_item_plan_author(
                cycle_gap_candidate(1),
                "repo_fixture".to_string(),
            )
            .await
            .expect("first gapped round completes");
        assert_eq!(
            engine.session().stage,
            WorkspaceStage::CrossReview,
            "first mechanical round routes TriggerAggregateRepair and waits for the author rerun"
        );

        // 第二轮：author 重跑后仍给出实质相同的缺口（不同 source hash、
        // 同指纹——fingerprint 基于 category+contract_field，与措辞无关）。
        engine
            .complete_single_candidate_work_item_plan_author(
                cycle_gap_candidate(2),
                "repo_fixture".to_string(),
            )
            .await
            .expect("second gapped round completes");

        assert_eq!(
            engine.session().stage,
            WorkspaceStage::HumanConfirm,
            "identical mechanical fingerprints across two rounds must force the human gate"
        );
        assert_eq!(
            engine.session().session_status,
            WorkspaceSessionStatus::WaitingForHuman
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load persisted session");
        assert_eq!(
            persisted.single_candidate_phase,
            Some(SingleCandidatePhase::Approval),
            "repeated-fingerprint 人工门必须在开启时回到 Approval，避免前端可确认而引擎相位仍停在 Evaluate"
        );
        assert_eq!(
            engine.session().single_candidate_phase,
            Some(SingleCandidatePhase::Approval),
            "门开 CAS 后内存相位必须同步 durable Approval"
        );
        assert_eq!(
            persisted.run_history.repairs_used, 1,
            "the repeated round must not consume another repair budget"
        );
        let snapshot = persisted
            .human_gate_snapshot
            .as_ref()
            .expect("human gate must carry a durable snapshot");
        assert_eq!(
            snapshot.trigger,
            crate::product::work_item_plan_policy::HumanReason::RepeatedFingerprint,
            "gate trigger must name the repeated fingerprint reason"
        );
    }
}
