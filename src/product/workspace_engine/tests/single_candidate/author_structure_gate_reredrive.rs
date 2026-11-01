// 作者阶段结构门拒收的「恰一次教学重驱」（REQ-SG-02/03、REQ-WSC-18/25）。
//
// 上一批（author_structure_gate_typed.rs）把拒收从硬失败改成 typed 失败路
// （durable + 尝试流水 + 人工门），owner 拿到了人话原因与两个修法，但仍要
// owner 自己动手。本文件钉住结构门重驱**自己的槽位**（2026-10-31 真机修订：
// 与 compile/IR 槽互不挤占，每类至多一次）下的三件事：
// 1. 槽位空闲（allow_structure_rerun=true）：不开人工门、不硬失败，返回
//    `Rejected { repair_prompt }`（环在哪 + 两个合法修法各带后果 + 保护项 +
//    完整修订版 markdown 输出口径）；
// 2. 槽位已占用（allow_structure_rerun=false）：保持既有落门语义（人话
//    摘要 + 两个带后果修法），会话不是 failed；
// 3. 非环拒收的修复指令不得套用「环」措辞（不虚构原因）。
//
// 每次尝试都进 `candidate-attempts.jsonl`（两次拒收 = 两行失败）在 1+2 中
// 一并钉住；`auto_rewrites_used` 不因作者阶段重驱而变、durable `auto_rewrite`
// 不留 running 记录同样在此钉住（重驱走 runner 的 provider 通道，不是引擎
// prerevision 路由——作者阶段候选 refs 未提升，重写通道不可达）。
mod author_structure_gate_reredrive {
    use super::contract_prerevision::{author_round_record, clean_candidate};
    use super::sg3a1_structure_gate::{cycle_candidate, unknown_provider_candidate};
    use super::*;
    use crate::product::workspace_engine::SingleCandidateAuthorCompletion;
    use crate::product::workspace_engine::single_candidate_structure_failure::STRUCTURE_STABLE_CODE;
    use crate::web::workspace_ws_types::TimelineNodeType;

    type Attempts =
        Vec<crate::product::lifecycle_store::candidate_attempts::CandidateAttemptRecord>;

    fn attempt_outcomes(attempts: &Attempts) -> Vec<&'static str> {
        attempts
            .iter()
            .map(|record| match record.outcome {
                crate::product::lifecycle_store::candidate_attempts::CandidateAttemptOutcome::Passed => "passed",
                crate::product::lifecycle_store::candidate_attempts::CandidateAttemptOutcome::Failed => "failed",
            })
            .collect()
    }

    /// ① 槽位空闲：拒收返回 `Rejected`，不开人工门、不硬失败、零提升，
    ///    流水恰好一行失败，预算不虚记。
    #[tokio::test]
    async fn structure_rejection_with_rerun_slot_free_returns_repair_prompt_without_gate() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);
        let completion = engine
            .complete_single_candidate_work_item_plan_author(
                cycle_candidate(),
                "repo_fixture".to_string(),
                None,
                true,
            )
            .await
            .expect("槽位空闲时的拒收必须交还修复指令，不得 Err");
        let SingleCandidateAuthorCompletion::Rejected { repair_prompt } = completion else {
            panic!("槽位空闲时必须返回 Rejected（修复指令），得到 {completion:?}");
        };

        // 修复指令 = 环在哪（人话）+ 消环要点 + 保护项 + 完整修订版
        // markdown 输出口径（REVISION_TEACHING 的口径）。本夹具的环是纯
        // 真实边（无逆仓序自动边）→ witness 不完整分支：给通用消环要点，
        // 不虚构位次与选项（witness 完整的两修法形态由下方纯函数用例钉）。
        for required in [
            "依赖环",
            "消环要点",
            "不得删除真实业务数据流",
            "不得改动仓序本身",
            "必须输出完整修订版 markdown",
            "不得输出 diff、patch",
        ] {
            assert!(
                repair_prompt.contains(required),
                "结构拒收修复指令必须包含 {required}：{repair_prompt}"
            );
        }
        assert!(
            !repair_prompt.contains("两个合法修法"),
            "witness 不完整不得虚构两个修法：{repair_prompt}"
        );

        let session_id = engine.session().session_id.clone();
        let after = lifecycle
            .get_workspace_session(&session_id)
            .expect("reload session after rerun-eligible rejection");
        // 不落人工门、不硬失败：无门快照、相位留在 Generate。
        assert!(
            after.human_gate_snapshot.is_none(),
            "重驱机会还在时不得打开人工门（等作者重驱后再说）"
        );
        assert_eq!(
            after.single_candidate_phase,
            Some(crate::product::models::SingleCandidatePhase::Generate),
            "重驱机会还在时相位必须留在 Generate，不开确认面"
        );
        assert_ne!(
            after.status,
            crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
            "重驱机会还在时会话不得停在人工门"
        );
        // 候选零提升（REQ-SG-03 不放宽）。
        assert!(
            after.work_item_plan_source_revision_ref.is_none()
                && after.plan_candidate_ir_ref.is_none(),
            "被拒候选不得提升 current refs"
        );
        let confirm_nodes = engine
            .timeline_nodes
            .iter()
            .filter(|node| matches!(node.node_type, TimelineNodeType::HumanConfirm))
            .count();
        assert_eq!(
            confirm_nodes, 0,
            "重驱机会还在时不得出现 HumanConfirm 节点"
        );

        // 每次尝试都进流水：恰好一行失败（通过率读数靠它）。
        let attempts: Attempts = lifecycle
            .list_candidate_attempts(&session_id)
            .expect("read candidate attempt log");
        assert_eq!(attempt_outcomes(&attempts), vec!["failed"]);
        assert_eq!(
            attempts[0].failure.as_ref().expect("failure facts").code,
            STRUCTURE_STABLE_CODE
        );
        // 阶段单源派生：作者阶段结构拒收码走 operational 家族 →
        // `candidate_attempt_record_from_failure` 派生 Final（与上一批
        // 真机流水 stage=final 一致，不从文案反推）。
        assert_eq!(
            attempts[0].stage,
            crate::product::lifecycle_store::candidate_attempts::CandidateAttemptStage::Final
        );

        // 不虚记自动重写预算：durable 不留 running，auto_rewrites_used 不变。
        let status = lifecycle
            .load_candidate_compile_status(&session_id)
            .expect("load candidate compile status")
            .expect("durable candidate compile status must exist");
        assert!(status.auto_rewrite.is_none(), "不得留下 auto_rewrite running 记录");
        assert_eq!(status.auto_rewrites_used, 0, "作者阶段重驱不扣自动重写预算");
    }

    /// ② 槽位已占用：第二次拒收落人工门（人话摘要 + 两个带后果修法），
    ///    会话不是 failed；两次拒收在流水中留下两行失败。
    #[tokio::test]
    async fn second_structure_rejection_after_slot_used_lands_human_gate_with_two_rows() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);
        let first = engine
            .complete_single_candidate_work_item_plan_author(
                cycle_candidate(),
                "repo_fixture".to_string(),
                None,
                true,
            )
            .await
            .expect("首轮拒收不得 Err");
        assert!(matches!(
            first,
            SingleCandidateAuthorCompletion::Rejected { .. }
        ));
        // 槽位已被（模拟的）compile/IR 重驱占用 → allow_structure_rerun=false。
        // 第二轮换一份内容不同（改标题）的同构环候选：attempt_key 内容寻址
        // 幂等——同源候选重放＝同一次尝试（零膨胀），真实重驱轮的 source
        // 必然不同，故此处以改标题体现「第二次真实尝试」。
        let second = engine
            .complete_single_candidate_work_item_plan_author(
                cycle_candidate().replace("Backend levels API", "Backend levels API round-2"),
                "repo_fixture".to_string(),
                None,
                false,
            )
            .await
            .expect("槽位占用后的拒收同样不得 Err");
        assert_eq!(
            second,
            SingleCandidateAuthorCompletion::Routed,
            "槽位占用后必须回到既有落门收口（Routed）"
        );

        let session_id = engine.session().session_id.clone();
        let after = lifecycle
            .get_workspace_session(&session_id)
            .expect("reload session after second rejection");
        assert_eq!(
            after.status,
            crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
            "重驱后仍被拒必须落人工门"
        );
        assert!(
            after.human_gate_snapshot.is_some(),
            "人工门快照必须在场（owner 拿到人话摘要）"
        );
        assert_ne!(
            after.single_candidate_phase,
            Some(crate::product::models::SingleCandidatePhase::Failed),
            "落门不是终态失败"
        );
        // 门卡摘要（durable HumanConfirm 节点）带 owner 可读的话 + 两个修法。
        let summary = engine
            .timeline_nodes
            .iter()
            .rev()
            .find(|node| matches!(node.node_type, TimelineNodeType::HumanConfirm))
            .and_then(|node| node.summary.clone())
            .expect("人工门节点必须带摘要");
        // 本夹具环无逆仓序自动边 → witness 不完整：摘要给真实环链与
        // 「不给出位次与选项」的诚实呈现（witness 完整时的两个修法由
        // structure_parity 的 owner 面用例钉住，不在此重复）。
        for required in ["候选没有通过候选结构门", "依赖环", "不给出位次与选项"] {
            assert!(
                summary.contains(required),
                "人工门摘要必须包含 {required}：{summary}"
            );
        }

        // 两次拒收 = 两行失败（每次尝试都进流水）。
        let attempts: Attempts = lifecycle
            .list_candidate_attempts(&session_id)
            .expect("read candidate attempt log");
        assert_eq!(attempt_outcomes(&attempts), vec!["failed", "failed"]);
        assert!(attempts.iter().all(|record| record
            .failure
            .as_ref()
            .is_some_and(|failure| failure.code == STRUCTURE_STABLE_CODE)));
        let status = lifecycle
            .load_candidate_compile_status(&session_id)
            .expect("load candidate compile status")
            .expect("durable candidate compile status must exist");
        assert_eq!(
            status.auto_rewrites_used, 0,
            "两次作者阶段拒收都不扣自动重写预算"
        );
        assert!(status.auto_rewrite.is_none(), "不得留下 running 记录");
    }

    /// ③ 非环拒收（unknown_provider）：修复指令给通用结构修正指令，不虚构
    ///    「环」措辞；槽位空闲同样返回 Rejected。
    #[tokio::test]
    async fn unknown_provider_rejection_repair_prompt_avoids_cycle_wording() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);
        let completion = engine
            .complete_single_candidate_work_item_plan_author(
                unknown_provider_candidate(),
                "repo_fixture".to_string(),
                None,
                true,
            )
            .await
            .expect("非环拒收同样不得 Err");
        let SingleCandidateAuthorCompletion::Rejected { repair_prompt } = completion else {
            panic!("非环拒收在槽位空闲时同样返回 Rejected：{completion:?}");
        };
        assert!(
            repair_prompt.contains("unknown_provider"),
            "修复指令必须携带稳定失败事实：{repair_prompt}"
        );
        for forbidden in ["依赖环", "消环", "整体迁到第"] {
            assert!(
                !repair_prompt.contains(forbidden),
                "非环拒收不得虚构「{forbidden}」措辞：{repair_prompt}"
            );
        }
        for required in [
            "修正 Work Item 的依赖与契约声明",
            "不得删除真实业务数据流",
            "必须输出完整修订版 markdown",
        ] {
            assert!(
                repair_prompt.contains(required),
                "非环修复指令必须包含 {required}：{repair_prompt}"
            );
        }
        // 干净候选零变化：同槽位语义下合法候选仍返回 Candidate。
        let clean = engine
            .complete_single_candidate_work_item_plan_author(
                clean_candidate(),
                "repo_fixture".to_string(),
                None,
                true,
            )
            .await
            .expect("clean candidate must complete");
        assert_eq!(
            clean,
            SingleCandidateAuthorCompletion::Candidate { item_count: 3 },
            "合法候选不受结构拒收重驱语义影响"
        );
    }

    /// ④ witness 完整的逆仓序环（2026-10-31 现场形态）：修复指令必须给出
    ///    环在哪（人话）+ **两个合法修法各带后果** + 保护项 + 完整修订版
    ///    输出口径——这是作者阶段教学重驱要模型自己改对的主形态。
    #[test]
    fn witness_complete_cycle_repair_prompt_carries_two_options_with_consequences() {
        use super::structure_parity::{Repos, ir_of, item, structure_context};
        use crate::product::workspace_engine::plan_projection::assess_single_candidate_structure;
        use crate::product::workspace_engine::types::StructuralAssessment;
        use crate::product::workspace_engine::single_candidate_structure_failure::{
            author_structure_gate_failure, author_structure_repair_prompt,
        };

        let repos = Repos::new();
        // 现场形态：WI-004 托管仓序第 2（repository_b），却消费仓序第 3 的产出。
        let ir = ir_of(vec![
            item("WI-001", "repository_a", None),
            item("WI-002", "repository_b", Some("WI-001")),
            item("WI-003", "repository_c", Some("WI-002")),
            item("WI-004", "repository_b", Some("WI-003")),
        ]);
        let context = structure_context(repos.table(), repos.change_order());
        let StructuralAssessment::Rejected(failures, _) =
            assess_single_candidate_structure(String::new(), &ir, &context)
        else {
            panic!("逆仓序的跨仓消费必须在候选门被拦下");
        };
        let typed = author_structure_gate_failure(&ir, &context, &failures);
        let prompt = author_structure_repair_prompt(&typed);
        for required in [
            "依赖环",
            "两个合法修法",
            "后果：",
            "去掉「",
            "整体迁到第",
            "不得删除真实业务数据流",
            "不得改动仓序本身",
            "必须输出完整修订版 markdown",
            "不得输出 diff、patch",
        ] {
            assert!(
                prompt.contains(required),
                "witness 完整的修复指令必须包含 {required}：{prompt}"
            );
        }
    }
}
