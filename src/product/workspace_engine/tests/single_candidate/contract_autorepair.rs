// O1（裁决二 §2.2/§2.4）：contract_autorepair 迁移为「机器指令交原授权
// author」后的引擎侧用例——原「自动补/删成功」断言按 8.4 改写为：source
// 不变（与 author 原始输出逐字节一致）、缺口沿既有机械 verdict 通道
// （F5-A）交原授权 author 返修。用例按大文件守卫 (>1200 行) 拆分至本
// 文件，经 include! 内联进父模块。
mod contract_autorepair {
    use super::contract_prerevision::{
        REP4_FIXTURE, author_round_record, clean_candidate, missing_contract_candidate,
    };
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

    // 非机械指令类缺口（required_contract_missing）见 contract_prerevision::
    // missing_contract_candidate——provider 未承诺被消费契约，指令域外。

    /// 混合缺口：指令类（capability）+ 非机械（required_contract_missing）并存。
    fn mixed_gap_candidate(round: usize) -> String {
        missing_contract_candidate(round).replace(
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

    /// 迁自 capability_gap_is_autorepaired_before_revision_routing（O1 改写）：
    /// capability 缺口不再被平台本地补齐——持久化 source 与 author 原始
    /// 输出逐字节一致；缺口沿既有机械 verdict 通道（F5-A）交原授权
    /// author 返修（repairs_used 计 1、缺口指纹入 durable 集）。
    #[tokio::test]
    async fn capability_gap_routes_author_revision_without_source_mutation() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);

        let author_source = capability_gap_candidate(1);
        engine
            .complete_single_candidate_work_item_plan_author(
                author_source.clone(),
                fixture_sc_target(&engine),
                fixture_sc_member_targets(&engine),
                false,
            )
            .await
            .expect("gapped candidate must complete with instructions, not mutation");

        // source 不变：持久化产物与 author 原始输出逐字节一致。
        let source = persisted_source(&lifecycle, &engine);
        assert_eq!(
            source, author_source,
            "completion must keep the author source byte-identical"
        );
        assert!(!source.contains("- capabilities: api.levels.read, api.levels.write\n"));

        // 指令完整（缺口未消失）：机械 verdict 携带 capability finding。
        let pending = engine
            .single_candidate_pending_revision_verdict()
            .expect("capability gap must route a mechanical revision round");
        assert_eq!(pending.verdict, ReviewVerdictType::Revise);
        assert!(
            pending
                .findings
                .iter()
                .any(|finding| finding.message.starts_with("required_capability_missing: ")),
            "capability finding must ride the verdict: {:?}",
            pending
                .findings
                .iter()
                .map(|finding| &finding.message)
                .collect::<Vec<_>>()
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load persisted session");
        assert_eq!(
            persisted.run_history.repairs_used, 1,
            "O1 收窄后机械缺口走真实 author 返修（不再零预算本地补齐）"
        );
        assert!(
            !persisted.run_history.seen_fingerprints.is_empty(),
            "gap fingerprints must enter the durable seen set for the repeat gate"
        );

        // author 新产物复验口径：重编译 author 原文，契约缺口仍在（未被暗修）。
        let ir = compile_work_item_plan(
            &source,
            &WorkItemPlanSourceContext {
                target_repository_id: "repo_fixture".to_string(),
                member_targets: None,
            },
        )
        .expect("author source must recompile");
        assert!(
            crate::product::workspace_engine::contract_prerevision::single_candidate_contract_prerevision_verdict(
                &ir
            )
            .is_some(),
            "the gap must still be present in the persisted author source"
        );
    }

    /// 迁自 unconsumed_handoff_ref_is_deterministically_removed（O1 改写）：
    /// 未消费引用不再被平台删除/改写为 `[]`——source 逐字节保持 author
    /// 原文，缺口走机械 verdict（裸 rep4 fixture 自带该缺口）。
    #[tokio::test]
    async fn unconsumed_handoff_ref_kept_for_author_decision() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);

        engine
            .complete_single_candidate_work_item_plan_author(
                REP4_FIXTURE.to_string(),
                fixture_sc_target(&engine),
                fixture_sc_member_targets(&engine),
                false,
            )
            .await
            .expect("unconsumed-handoff candidate must complete with instructions only");

        let source = persisted_source(&lifecycle, &engine);
        assert_eq!(
            source, REP4_FIXTURE,
            "completion must keep the author source byte-identical"
        );
        assert!(source.contains("- provided_contract_refs: contract.levels-integration\n"));
        let pending = engine
            .single_candidate_pending_revision_verdict()
            .expect("unconsumed handoff gap must route a mechanical revision round");
        assert!(
            pending
                .findings
                .iter()
                .any(|finding| finding.message.starts_with("unconsumed_required_handoff: ")),
            "handoff finding must ride the verdict: {:?}",
            pending
                .findings
                .iter()
                .map(|finding| &finding.message)
                .collect::<Vec<_>>()
        );
    }

    /// 边界（口径不变）：非指令类缺口（required_contract_missing——provider
    /// 未承诺被消费契约）仍走既有机械 verdict → 模型返修通道；残余 verdict
    /// 只含非指令类 finding。依赖环自 SG3a.1 起在候选结构门拒收（零提升/
    /// 零评审），不再走本通道。
    #[tokio::test]
    async fn non_mechanical_gap_still_routes_model_revision() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);

        engine
            .complete_single_candidate_work_item_plan_author(
                missing_contract_candidate(1),
                fixture_sc_target(&engine),
                fixture_sc_member_targets(&engine),
                false,
            )
            .await
            .expect("missing-contract candidate must still complete");

        let pending = engine.single_candidate_pending_revision_verdict().expect(
            "non-mechanical supply-demand gap must still drive a mechanical revision round",
        );
        assert_eq!(pending.verdict, ReviewVerdictType::Revise);
        assert!(
            pending
                .findings
                .iter()
                .all(|finding| finding.message.starts_with("required_contract_missing: ")),
            "residual verdict must carry only the non-autorepairable findings: {:?}",
            pending
                .findings
                .iter()
                .map(|finding| &finding.message)
                .collect::<Vec<_>>()
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("load persisted session");
        assert_eq!(persisted.run_history.repairs_used, 1);
    }

    /// 迁自 mixed_gap_autorepairs_mechanical_part_and_routes_residual（O1
    /// 改写）：指令类不再被先行补齐——source 逐字节保持，两类缺口同
    /// verdict 路由交 author。
    #[tokio::test]
    async fn mixed_gap_routes_all_findings_without_mutation() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);

        let author_source = mixed_gap_candidate(1);
        engine
            .complete_single_candidate_work_item_plan_author(
                author_source.clone(),
                fixture_sc_target(&engine),
                fixture_sc_member_targets(&engine),
                false,
            )
            .await
            .expect("mixed-gap candidate must still complete");

        let source = persisted_source(&lifecycle, &engine);
        assert_eq!(
            source, author_source,
            "completion must keep the author source byte-identical"
        );
        let pending = engine
            .single_candidate_pending_revision_verdict()
            .expect("mixed gap must still route a revision round");
        assert!(
            pending
                .findings
                .iter()
                .any(|finding| finding.message.starts_with("required_capability_missing: ")),
            "capability finding must ride the verdict alongside the residual: {:?}",
            pending
                .findings
                .iter()
                .map(|finding| &finding.message)
                .collect::<Vec<_>>()
        );
        assert!(
            pending
                .findings
                .iter()
                .any(|finding| finding.message.starts_with("required_contract_missing: ")),
            "residual non-mechanical finding must still ride the verdict"
        );
    }
}
