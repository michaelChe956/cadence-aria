// DEF-PVR-ALL 确定性 capability 补齐器（contract_autorepair）用例按大文件守卫
// (>1200 行) 拆分至本文件，经 include! 内联进父模块。

/// DEF-PVR-ALL 主方案：class_hint=repairable 的机械契约缺口（3.6 门重测
/// codex rep1/rep2 全败根因：required_capability_missing 指纹轮转不收敛）
/// 在 author 落盘前先行确定性修复——required_action 的「追加逐字行」/
/// 「删未消费引用」由服务端直接应用到 markdown source，重编译后校验干净
/// 即零返修轮（不产生 verdict、不消费预算、不播指纹）；残余非机械缺口
/// 才走 F5-A 模型返修通道（既有语义零变化）。
mod contract_autorepair {
    use super::contract_prerevision::{REP4_FIXTURE, author_round_record, clean_candidate};
    use super::*;
    use crate::product::work_item_plan_compiler::{
        WorkItemPlanSourceContext, compile_work_item_plan,
    };
    use crate::product::work_item_plan_source_store::WorkItemPlanSourceStore;

    /// 干净基线 + WI-002/WI-003 require_all 塞入 WI-001 未提供的 capability
    /// （与 contract_prerevision 测试同款缺口注入，两处 required 行都被改写）。
    fn capability_gap_candidate(round: usize) -> String {
        clean_candidate()
            .replace(
                "- required_capabilities: api.levels.read\n",
                "- required_capabilities: api.levels.read, api.levels.write\n",
            )
            .replace(
                "Backend levels API",
                &format!("Backend levels API round-{round}"),
            )
    }

    /// 非机械可修缺口（依赖环）：WI-001 反向消费 WI-002 制造环——补齐器
    /// 边界外的 Error 仍走既有机械 verdict → 模型返修通道。
    fn cycle_candidate(round: usize) -> String {
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

    /// 混合缺口：机械可修（capability）+ 非机械（依赖环）并存。
    fn mixed_gap_candidate(round: usize) -> String {
        cycle_candidate(round).replace(
            "- required_capabilities: api.levels.read\n",
            "- required_capabilities: api.levels.read, api.levels.write\n",
        )
    }
    fn persisted_source(lifecycle: &LifecycleStore, engine: &WorkspaceEngine) -> String {
        let scope = engine.session();
        let store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        let source_ref = scope
            .work_item_plan_source_revision_ref
            .as_deref()
            .expect("durable source ref");
        store
            .get_source_revision(
                &crate::product::work_item_plan_source_store::SourceStoreScope {
                    project_id: scope.project_id.clone(),
                    issue_id: scope.issue_id.clone(),
                    plan_id: scope.entity_id.clone(),
                },
                source_ref,
            )
            .expect("reload persisted source")
            .source
    }

    /// 红测主例：repairable capability 缺口进 author 落盘后产物必须已补齐——
    /// 补齐后的 source 重编译，canonical 契约投影校验必须干净，且全程零
    /// 返修轮（无 verdict、repairs_used=0、指纹集空、路由与干净候选一致）。
    #[tokio::test]
    async fn capability_gap_is_autorepaired_before_revision_routing() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);

        engine
            .complete_single_candidate_work_item_plan_author(
                capability_gap_candidate(1),
                "repo_fixture".to_string(),
            )
            .await
            .expect("gapped candidate must complete author round after autorepair");

        // 零返修轮：不产生 pending revision verdict、不消费预算、不播指纹。
        assert!(
            engine.single_candidate_pending_revision_verdict().is_none(),
            "capability gap must be deterministically repaired, not routed to a revision round"
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load persisted session");
        assert_eq!(
            persisted.run_history.repairs_used, 0,
            "autorepair must not consume the shared repair budget"
        );
        assert!(
            persisted.run_history.seen_fingerprints.is_empty(),
            "autorepair must not seed any fingerprint (repeat gate stays cold)"
        );
        // 路由与干净候选一致：有 reviewer 时 Evaluate 后进 CrossReview。
        assert_eq!(engine.session().stage, WorkspaceStage::CrossReview);

        // 产物已补齐：持久化 source 含逐字追加的 capability；重编译后
        // canonical 契约校验（同 contract_prerevision 口径）干净。
        let source = persisted_source(&lifecycle, &engine);
        assert!(
            source.contains("- capabilities: api.levels.read, api.levels.write\n"),
            "repaired source must merge the missing capability verbatim into the provider line:\n{source}"
        );
        let ir = compile_work_item_plan(
            &source,
            &WorkItemPlanSourceContext {
                target_repository_id: "repo_fixture".to_string(),
            },
        )
        .expect("repaired source must recompile");
        assert!(
            crate::product::workspace_engine::contract_prerevision::single_candidate_contract_prerevision_verdict(&ir)
                .is_none(),
            "repaired source must pass the mechanical contract validation"
        );
        // 持久化 IR 与补齐后 source 新鲜一致（freshness 链路不受影响）。
        assert_eq!(ir.items.len(), 3);
    }

    /// unconsumed_required_handoff 同机械修复：终端 WI 的未消费 handoff 引用
    /// 直接从 provided_contract_refs 删除（裸 rep4 fixture 自带该缺口）。
    #[tokio::test]
    async fn unconsumed_handoff_ref_is_deterministically_removed() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);

        engine
            .complete_single_candidate_work_item_plan_author(
                REP4_FIXTURE.to_string(),
                "repo_fixture".to_string(),
            )
            .await
            .expect("unconsumed-handoff candidate must complete after autorepair");

        assert!(
            engine.single_candidate_pending_revision_verdict().is_none(),
            "unconsumed handoff ref must be removed deterministically, not routed to a revision round"
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load persisted session");
        assert_eq!(persisted.run_history.repairs_used, 0);
        let source = persisted_source(&lifecycle, &engine);
        assert!(
            source.contains(
                "### Handoff Schema\n- required_fields: commit_sha\n- provided_contract_refs: []\n"
            ),
            "the unconsumed provided ref must be rewritten to the legal empty list:\n{source}"
        );
        let ir = compile_work_item_plan(
            &source,
            &WorkItemPlanSourceContext {
                target_repository_id: "repo_fixture".to_string(),
            },
        )
        .expect("repaired source must recompile");
        assert!(
            crate::product::workspace_engine::contract_prerevision::single_candidate_contract_prerevision_verdict(&ir)
                .is_none()
        );
    }

    /// 边界：非机械可修缺口（依赖环）不被补齐器拦截，
    /// 仍走既有机械 verdict → 模型返修通道；残余 verdict 只含非机械 finding。
    #[tokio::test]
    async fn non_mechanical_gap_still_routes_model_revision() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);

        engine
            .complete_single_candidate_work_item_plan_author(
                cycle_candidate(1),
                "repo_fixture".to_string(),
            )
            .await
            .expect("cycle candidate must still complete");

        let pending = engine
            .single_candidate_pending_revision_verdict()
            .expect("non-mechanical cycle gap must still drive a mechanical revision round");
        assert_eq!(pending.verdict, ReviewVerdictType::Revise);
        assert!(
            pending
                .findings
                .iter()
                .all(|finding| finding.message.starts_with("dependency_cycle: ")),
            "residual verdict must carry only the non-autorepairable findings: {:?}",
            pending
                .findings
                .iter()
                .map(|f| &f.message)
                .collect::<Vec<_>>()
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load persisted session");
        assert_eq!(persisted.run_history.repairs_used, 1);
    }

    /// 混合缺口：机械项被先行补齐，残余（非机械）走模型返修，且残余 verdict
    /// 不再包含已补齐的 capability finding。
    #[tokio::test]
    async fn mixed_gap_autorepairs_mechanical_part_and_routes_residual() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);

        engine
            .complete_single_candidate_work_item_plan_author(
                mixed_gap_candidate(1),
                "repo_fixture".to_string(),
            )
            .await
            .expect("mixed-gap candidate must still complete");

        let source = persisted_source(&lifecycle, &engine);
        assert!(
            source.contains("- capabilities: api.levels.read, api.levels.write\n"),
            "mechanical part must be repaired even when residual findings remain:\n{source}"
        );
        let pending = engine
            .single_candidate_pending_revision_verdict()
            .expect("residual non-mechanical gap must still route a revision round");
        assert!(
            pending
                .findings
                .iter()
                .all(|finding| finding.message.starts_with("dependency_cycle: ")),
            "repaired capability findings must not ride the residual verdict: {:?}",
            pending
                .findings
                .iter()
                .map(|f| &f.message)
                .collect::<Vec<_>>()
        );
    }
}
