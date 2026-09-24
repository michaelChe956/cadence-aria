// C1 Task 1 options 预检回灌链用例（REQ-WSC-02 场景 11-12；F-51）。
// 按大文件守卫(>1200 行)拆分至本文件，经 include! 内联进父模块。
mod preflight_options_loop {
    use super::contract_prerevision::{REP4_FIXTURE, author_round_record};
    use super::*;
    use crate::product::json_store::write_json;
    use crate::web::workspace_ws_types::TimelineNodeType;

    /// 显式改写 plan record 的存储 options（F-51：options 是创建意图事实，
    /// 模拟 UI 默认勾选 include_integration_tests=true 的真实形态）。
    fn set_plan_options(lifecycle: &LifecycleStore, integration: bool) {
        let mut plan = lifecycle
            .get_issue_work_item_plan("project_0001", "issue_0001", "issue_work_item_plan_0001")
            .expect("plan record");
        plan.options.include_integration_tests = integration;
        write_json(
            &lifecycle
                .issue_work_item_plans_root("project_0001", "issue_0001")
                .join("issue_work_item_plan_0001.json"),
            &plan,
        )
        .expect("persist plan options");
    }

    /// 单项 backend 候选（终端 WI 空 handoff）：无 integration item。
    fn no_integration_candidate(round: usize) -> String {
        REP4_FIXTURE
            .split("## Work Item WI-002:")
            .next()
            .expect("rep4 必须包含 WI-001")
            .trim_end()
            .to_string()
            .replace(
                "- provided_contract_refs: contract.levels-api",
                "- provided_contract_refs: []",
            )
            .replace(
                "Backend levels API",
                &format!("Backend levels API round-{round}"),
            )
            + "\n"
    }

    /// REQ-WSC-02 场景 11（generate/evaluate 期首轮即拦）：存储 options
    /// include_integration_tests=true 而候选无 integration WI——author 落盘
    /// 即产生机械返修 verdict（preflight 族不硬失败），policy 路由
    /// TriggerAggregateRepair：repairs_used 计 1、相位回 Generate 重驱 author。
    #[tokio::test]
    async fn options_gap_drives_mechanical_revision_round_via_stored_plan_options() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        set_plan_options(&lifecycle, true);
        author_round_record(&lifecycle, &mut engine);

        engine
            .complete_single_candidate_work_item_plan_author(
                no_integration_candidate(1),
                "repo_fixture".to_string(),
            )
            .await
            .expect("options gap candidate must compile, persist and route (not hard-fail)");

        // F5-A：机械 verdict 注入 latest_review_verdict，修订轮判定命中。
        let pending = engine
            .single_candidate_pending_revision_verdict()
            .expect("options gap must mark the next author round as revision");
        assert_eq!(pending.verdict, ReviewVerdictType::Revise);
        let finding = pending
            .findings
            .iter()
            .find(|finding| finding.message.contains("integration_work_item_required"))
            .expect("options gap finding must ride the verdict");
        assert!(
            finding.required_action.contains("kind=integration"),
            "required_action 与校验器口径同源：{}",
            finding.required_action
        );

        // 机械 verdict 持久化到 ReviewerRun 节点（跨轮指纹对比的数据源）。
        let reviewer_node = engine
            .timeline_nodes
            .iter()
            .rev()
            .find(|node| matches!(node.node_type, TimelineNodeType::ReviewerRun))
            .expect("options gap revision round must persist a ReviewerRun node");

        let detail = lifecycle
            .load_node_detail_for_issue_session(
                &engine.session().project_id,
                &engine.session().issue_id,
                &engine.session().session_id,
                &reviewer_node.node_id,
            )
            .expect("load reviewer node detail");
        assert!(detail.verdict.is_some(), "mechanical verdict must be durable");

        // policy：TriggerAggregateRepair（预算计入），不进 Approval/人工门。
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load persisted session");
        assert_eq!(
            persisted.run_history.repairs_used, 1,
            "options preflight revise round must consume the shared repair budget"
        );
        assert_eq!(
            persisted.single_candidate_phase,
            Some(SingleCandidatePhase::Generate),
            "TriggerAggregateRepair must route back to Generate for the author rerun"
        );
        assert!(
            !persisted.run_history.seen_fingerprints.is_empty(),
            "options gap fingerprints must enter the durable seen set"
        );
    }

    /// 回灌链收敛（Task 1.3）：第二轮 author 按 required_action 补上 integration
    /// item 后，候选通过预检——不再产生 verdict、不再消费预算，回到既有
    /// Evaluate 路由等待 reviewer。
    #[tokio::test]
    async fn options_gap_revision_converges_after_adding_integration_item() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        set_plan_options(&lifecycle, true);
        author_round_record(&lifecycle, &mut engine);

        engine
            .complete_single_candidate_work_item_plan_author(
                no_integration_candidate(1),
                "repo_fixture".to_string(),
            )
            .await
            .expect("first gapped round completes");

        // 第二轮：rep4 完整三 item（含 WI-003 integration）满足存储 options。
        engine
            .complete_single_candidate_work_item_plan_author(
                REP4_FIXTURE.replace(
                    "Integration levels API coverage",
                    "Integration levels API coverage round-2",
                ),
                "repo_fixture".to_string(),
            )
            .await
            .expect("fixed candidate must complete");

        // 第二轮干净候选不再注入机械 verdict（历史 Revise verdict 保留供
        // 审计）：带 durable verdict 的 ReviewerRun 节点数量保持 1，第二轮
        // 开的是真实 reviewer 评审轮（无 verdict，等待 reviewer run）。
        let durable_verdict_count = engine
            .timeline_nodes
            .iter()
            .filter(|node| matches!(node.node_type, TimelineNodeType::ReviewerRun))
            .filter(|node| {
                lifecycle
                    .load_node_detail_for_issue_session(
                        &engine.session().project_id,
                        &engine.session().issue_id,
                        &engine.session().session_id,
                        &node.node_id,
                    )
                    .map(|detail| detail.verdict.is_some())
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(
            durable_verdict_count, 1,
            "converged candidate must not inject a second mechanical verdict"
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load persisted session");
        assert_eq!(
            persisted.run_history.repairs_used, 1,
            "the converged round must not consume another repair budget"
        );
        assert_eq!(
            engine.session().stage,
            WorkspaceStage::CrossReview,
            "converged candidate routes the existing Evaluate path"
        );
    }

    /// 防环回归：options 缺口连续两轮重现（同身份不同字节）——既有
    /// RepeatedFingerprint 闸门强制人工门，不再自动重驱 author。
    #[tokio::test]
    async fn repeated_options_gap_lands_in_human_gate_without_further_auto_repair() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        set_plan_options(&lifecycle, true);
        author_round_record(&lifecycle, &mut engine);

        engine
            .complete_single_candidate_work_item_plan_author(
                no_integration_candidate(1),
                "repo_fixture".to_string(),
            )
            .await
            .expect("first gapped round completes");

        engine
            .complete_single_candidate_work_item_plan_author(
                no_integration_candidate(2),
                "repo_fixture".to_string(),
            )
            .await
            .expect("second gapped round completes");

        assert_eq!(
            engine.session().stage,
            WorkspaceStage::HumanConfirm,
            "identical options-gap fingerprints across two rounds must force the human gate"
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load persisted session");
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

    /// flag=false 基线（REQ-WSC-02 场景 12 反向）：无 options 意图时单项
    /// backend 候选零变化——不产生 verdict、不消费预算（既有 skipped-risk
    /// warning 路径不受影响）。
    #[tokio::test]
    async fn disabled_option_keeps_evaluate_routing_unchanged() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);

        engine
            .complete_single_candidate_work_item_plan_author(
                no_integration_candidate(1),
                "repo_fixture".to_string(),
            )
            .await
            .expect("candidate without options intent completes");

        assert!(
            engine.single_candidate_pending_revision_verdict().is_none(),
            "disabled option must not produce a revision verdict"
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load persisted session");
        assert_eq!(
            persisted.run_history.repairs_used, 0,
            "disabled option must not consume any repair budget"
        );
    }

    /// C1 Task 4（REQ-TOP-04 场景 6；F-51×F-52 交叉回放）：options 缺口
    /// 机械返修后，修复候选的复评落进锚定初评候选的同一 review cycle——
    /// scope=Verification（anchor=初评候选 ref、original_fingerprints=初评
    /// 机械缺口指纹集）、verification_count=1，终态后 cycle 关闭。
    #[tokio::test]
    async fn options_gap_repair_round_lands_in_verification_of_the_anchored_cycle() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        set_plan_options(&lifecycle, true);
        author_round_record(&lifecycle, &mut engine);

        // 第一轮：options 缺口 → 机械 verdict → TriggerAggregateRepair。
        engine
            .complete_single_candidate_work_item_plan_author(
                no_integration_candidate(1),
                "repo_fixture".to_string(),
            )
            .await
            .expect("first gapped round completes");
        let after_gap = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load gapped session");
        let anchor_ref = after_gap
            .plan_candidate_ir_ref
            .clone()
            .expect("anchor candidate ref");
        let anchor_key = format!(
            "sc:candidate:{}",
            anchor_ref.rsplit('-').next().unwrap_or(&anchor_ref)
        );
        assert_eq!(
            after_gap
                .run_history
                .review_cycles
                .get(&anchor_key)
                .map(|cycle| (cycle.initial_count, cycle.repairs_used)),
            Some((1, 1)),
            "the mechanical gap round must consume the anchored cycle's initial review"
        );
        assert!(
            matches!(
                after_gap.review_invocation_scope,
                Some(ReviewInvocationScope::Initial { .. })
            ),
            "TriggerAggregateRepair keeps the invocation chain scope Initial"
        );

        // 修复轮：补上 integration item 的收敛候选 → reviewer 复评。
        engine
            .complete_single_candidate_work_item_plan_author(
                REP4_FIXTURE.replace(
                    "Integration levels API coverage",
                    "Integration levels API coverage round-2",
                ),
                "repo_fixture".to_string(),
            )
            .await
            .expect("fixed candidate must complete");
        engine
            .ensure_review_invocation_scope()
            .await
            .expect("the replay review must materialize the anchored Verification scope");
        let scope = engine
            .session()
            .review_invocation_scope
            .clone()
            .expect("verification scope");
        let ReviewInvocationScope::Verification {
            original_fingerprints,
            repaired_revision_id,
            cycle_anchor_revision_id,
            ..
        } = &scope
        else {
            panic!("the replay scope must be Verification, got {scope:?}")
        };
        assert_eq!(
            cycle_anchor_revision_id.as_deref(),
            Some(anchor_ref.as_str()),
            "the verification scope must anchor the initial gap-round candidate"
        );
        assert_ne!(
            repaired_revision_id, &anchor_ref,
            "the repaired revision must be the fixed round-two candidate"
        );
        let anchored_cycle = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("reload session")
            .run_history
            .review_cycles
            .get(&anchor_key)
            .expect("anchored cycle")
            .clone();
        assert_eq!(
            original_fingerprints, &anchored_cycle.original_fingerprints,
            "the verification scope closes over the cycle's initial fingerprint set"
        );
        assert!(
            !anchored_cycle.original_fingerprints.is_empty(),
            "the options-gap fingerprints must be durable on the cycle"
        );

        complete_single_candidate_review(&mut engine, pass_verdict()).await;
        let verified = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("verification persisted");
        let cycle = verified
            .run_history
            .review_cycles
            .get(&anchor_key)
            .expect("anchored cycle after verification");
        assert_eq!(cycle.initial_count, 1);
        assert_eq!(cycle.verification_count, 1);
        assert!(
            verified.review_invocation_scope.is_none(),
            "the terminal action must close the anchored cycle"
        );
    }
}
