// SG3a.1 作者阶段结构门拒收的 typed 失败（REQ-SG-02/03、REQ-WSC-18/21/25）。
//
// 现场（2026-10-31 真机，kimi author + pi reviewer，3 成员仓 LC）：author 产出
// 候选后被候选结构门拒收（`single-candidate structure gate rejected:
// dependency_cycle[WI-003->WI-004]`），作者阶段把它当 `Err(String)` **硬失败**
// ——会话 `status=failed`、`human_gate_snapshot.findings` 为空、`repairs_used=0`、
// 完全没有 `candidate-attempts.jsonl`。owner 拿到的是「生成失败」，而不是两个
// 合法修法选一个。
//
// 本文件钉住修好后的四件事（与 Final Compile 同一条 typed 失败路）：
// 1. durable `candidate-compile-status.json` 落盘，`failure.message` 走共享环模板；
// 2. `candidate-attempts.jsonl` 真的追加了一行（此前完全空白）；
// 3. 人工门打开（`human_gate_snapshot` 在场、会话不 failed）；
// 4. 候选**零提升**（current refs 语义零变化，REQ-SG-03 不放宽）。
mod author_structure_gate_typed {
    use super::contract_prerevision::{author_round_record, clean_candidate};
    use super::sg3a1_structure_gate::{cycle_candidate, unknown_provider_candidate};
    use super::*;
    use crate::product::workspace_engine::SingleCandidateAuthorCompletion;
    use crate::product::workspace_engine::compile_failure_surface::{
        CYCLE_FINGERPRINT_MARKER, CYCLE_IDS_MARKER,
    };
    use crate::product::workspace_engine::single_candidate_structure_failure::{
        STRUCTURE_STABLE_CODE,
    };
    use crate::web::workspace_ws_types::TimelineNodeType;

    /// 走一遍作者阶段并返回（引擎、durable 会话、候选编译事实、尝试流水）。
    async fn reject(
        candidate: String,
    ) -> (
        tempfile::TempDir,
        LifecycleStore,
        WorkspaceEngine,
        crate::product::models::WorkspaceSessionRecord,
        crate::product::models::CandidateCompileStatus,
        Vec<crate::product::lifecycle_store::candidate_attempts::CandidateAttemptRecord>,
    ) {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);
        let completion = engine
            .complete_single_candidate_work_item_plan_author(
                candidate,
                fixture_sc_target(&engine),
                fixture_sc_member_targets(&engine),
                false,
            )
            .await
            .expect("SG3a.1 拒收不得再硬失败（Err）——已按 typed 失败收口");
        assert_eq!(
            completion,
            SingleCandidateAuthorCompletion::Routed,
            "拒收必须表达「已路由」，而不是「生成了 N 个 item」"
        );
        let session_id = engine.session().session_id.clone();
        let persisted = lifecycle
            .get_workspace_session(&session_id)
            .expect("reload rejected session");
        let status = lifecycle
            .load_candidate_compile_status(&session_id)
            .expect("load candidate compile status")
            .expect("durable candidate compile status must exist after a gate rejection");
        let attempts = lifecycle
            .list_candidate_attempts(&session_id)
            .expect("read candidate attempt log");
        (_tmp, lifecycle, engine, persisted, status, attempts)
    }

    /// ① 环拒收：durable 诊断 + 尝试流水 + 人工门，且候选零提升。
    #[tokio::test]
    async fn cycle_rejection_lands_durable_failure_attempt_log_and_owner_gate() {
        let (_tmp, _lifecycle, engine, persisted, status, attempts) =
            reject(cycle_candidate()).await;

        // (a) durable 失败事实：稳定码 + 走共享环模板的人话段 + 指纹/机器 id 行。
        let failure = status.failure.clone().expect("durable failure present");
        assert_eq!(
            failure.code, STRUCTURE_STABLE_CODE,
            "作者阶段拒收一律用结构拒收码：owner 文案不得指向没跑过的终编译，\
             失败码统计也要能区分阶段（作者阶段 vs 终编译阶段）"
        );
        assert_eq!(
            status.auto_rewrites_used, 0,
            "作者阶段没有真正发生自动重写（候选 refs 未提升、通道不可达）→ \
             自动重写预算不得虚记（终编译 prepare 码会把这里记成 +1）"
        );
        assert!(
            status.auto_rewrite.is_none(),
            "没有委托重写时 durable 不得留下 auto_rewrite running 状态"
        );
        assert!(
            failure.message.contains("构成依赖环"),
            "message 必须是共享环模板的人话段（owner 可读）：{}",
            failure.message
        );
        assert!(
            failure.message.contains("依赖环")
                && failure.detail.contains(CYCLE_FINGERPRINT_MARKER)
                && failure.detail.contains(CYCLE_IDS_MARKER),
            "detail 必须携带结构指纹行与环上 Work Item id 行：{}",
            failure.detail
        );
        let fingerprint = failure
            .detail
            .split(CYCLE_FINGERPRINT_MARKER)
            .nth(1)
            .and_then(|rest| rest.lines().next())
            .expect("fingerprint segment");
        assert_eq!(
            fingerprint.len(),
            64,
            "结构指纹必须是 sha256 hex（跨尝试稳定，不靠文案）：{fingerprint}"
        );
        assert!(
            failure.detail.contains("WI-001") && failure.detail.contains("WI-002"),
            "环上 Work Item id 必须逐字保留（机器 id 段）：{}",
            failure.detail
        );

        // (b) 尝试流水：此前完全空白，现在必须有且只有这一行。
        assert_eq!(
            attempts.len(),
            1,
            "拒收必须追加一行 candidate-attempts.jsonl（REQ-WSC-25）：{attempts:?}"
        );
        assert_eq!(
            attempts[0].outcome,
            crate::product::lifecycle_store::candidate_attempts::CandidateAttemptOutcome::Failed,
            "本次尝试的结局是失败"
        );
        assert!(
            attempts[0].failure.is_some(),
            "尝试流水必须携带失败事实：{:?}",
            attempts[0]
        );
        // 未真正委托 author 重写时，durable 不得留下任何「正在自动重写」的
        // 痕迹（REQ-WSC-25：不得对 owner 谎报「正在自动重写」）。
        //
        // 作者阶段用结构拒收码（Gate 路由）→ 平台**从未安排**自动重写，
        // 因此 durable 里**不应有** auto_rewrite 记录（比「安排过又停了」
        // 更如实：连预算都不该扣）。
        assert!(
            status.auto_rewrite.is_none(),
            "未安排重写时 durable 不得留 auto_rewrite 记录：{:?}",
            status.auto_rewrite
        );

        // (c) 人工门打开：会话不是 failed，相位不进 Failed。
        assert_ne!(
            persisted.status,
            WorkspaceSessionStatus::Failed,
            "SG3a.1 拒收不得把会话判成 failed（现场缺口）"
        );
        assert_eq!(
            persisted.status,
            WorkspaceSessionStatus::WaitingForHuman,
            "必须停在人工门（owner 侧要能读到修法并做选择）"
        );
        assert!(
            persisted.human_gate_snapshot.is_some(),
            "人工门快照必须打开（现场为缺席/空 findings 的硬失败）"
        );
        assert_ne!(
            persisted.single_candidate_phase,
            Some(SingleCandidatePhase::Failed),
            "候选相位不得落 Failed"
        );
        assert_eq!(
            persisted.single_candidate_phase,
            Some(SingleCandidatePhase::Generate),
            "相位保持 Generate（下一轮 author 由此重跑）"
        );

        // (d) 零提升：坏候选不得成为 current candidate（既有门禁不放宽）。
        assert!(
            persisted.work_item_plan_source_revision_ref.is_none()
                && persisted.plan_candidate_ir_ref.is_none()
                && persisted.mechanical_report_ref.is_none(),
            "被拒候选的 current refs 必须保持 None（零提升）"
        );
        assert!(
            persisted.review_invocation_scope.is_none(),
            "被拒候选不得实现评审 invocation scope"
        );
        assert!(
            persisted.provider_start_ledger.is_empty(),
            "被拒候选不得启动任何 provider run（不新造 attempt 通道）"
        );

        // 门卡摘要（durable HumanConfirm 节点）带 owner 可读的话 + 两个修法。
        let gate_summary = engine
            .timeline_nodes
            .iter()
            .rev()
            .find(|node| matches!(node.node_type, TimelineNodeType::HumanConfirm))
            .and_then(|node| node.summary.clone())
            .expect("人工门节点必须带摘要");
        assert!(
            gate_summary.contains("候选结构门") || gate_summary.contains("依赖环"),
            "门摘要必须说清发生了什么：{gate_summary}"
        );
    }

    /// ② 非环拒收（batch 外 provider）：同一 typed 路、同一结构拒收码，
    ///    仍然不硬失败、仍然落流水与门，且同样不虚记自动重写预算。
    #[tokio::test]
    async fn non_cycle_rejection_still_lands_gate_without_budget_charge() {
        let (_tmp, _lifecycle, _engine, persisted, status, attempts) =
            reject(unknown_provider_candidate()).await;

        let failure = status.failure.clone().expect("durable failure present");
        assert_eq!(
            failure.code, STRUCTURE_STABLE_CODE,
            "非环结构缺口与环拒收同用结构拒收稳定码（都不进自动重写族）"
        );
        assert!(
            failure.message.contains("结构校验") || failure.message.contains("structure gate"),
            "非环拒收的人话段必须说清「结构校验没过」：{}",
            failure.message
        );
        assert!(
            failure.detail.contains("unknown_provider"),
            "既有 typed evidence 不得丢失：{}",
            failure.detail
        );
        assert_eq!(attempts.len(), 1, "非环拒收同样必须落尝试流水");
        assert_eq!(
            persisted.status,
            WorkspaceSessionStatus::WaitingForHuman,
            "非环拒收同样停在人工门，不 failed"
        );
    }

    /// ③ 合法候选不受影响：仍返回 `Candidate { item_count }` 并提升 current refs。
    #[tokio::test]
    async fn clean_candidate_still_completes_as_candidate() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);
        let completion = engine
            .complete_single_candidate_work_item_plan_author(
                clean_candidate(),
                fixture_sc_target(&engine),
                fixture_sc_member_targets(&engine),
                false,
            )
            .await
            .expect("clean candidate must pass the structure gate");
        assert_eq!(
            completion,
            SingleCandidateAuthorCompletion::Candidate { item_count: 3 },
            "合法候选必须照常进入评审/确认链"
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("reload promoted session");
        assert!(
            persisted.plan_candidate_ir_ref.is_some(),
            "合法候选必须提升 current refs"
        );
        assert!(
            lifecycle
                .load_candidate_compile_status(&engine.session().session_id)
                .expect("load status")
                .is_none(),
            "合法候选不应留下候选编译失败事实"
        );
    }

    /// ④ 尝试流水写失败只降级为**可观测静默**（REQ-WSC-25 场景 2）：把 issue
    ///    级流水落点占成目录 → 追加必失败 → 候选生成/确认**不得**因此失败、
    ///    不得 panic；失败被记入运行日志（policy diagnostics），会话仍落人工门
    ///    （不是 failed）。
    #[tokio::test]
    async fn attempt_log_write_failure_degrades_to_silent_without_failing_candidate() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);
        let ledger_path = lifecycle
            .app_paths()
            .issue_lifecycle_root(&engine.session().project_id, &engine.session().issue_id)
            .join("candidate-attempts.jsonl");
        std::fs::create_dir_all(&ledger_path).expect("occupy ledger path with a directory");
        let completion = engine
            .complete_single_candidate_work_item_plan_author(
                cycle_candidate(),
                fixture_sc_target(&engine),
                fixture_sc_member_targets(&engine),
                false,
            )
            .await
            .expect("流水写失败不得让候选生成/确认失败");
        assert_eq!(
            completion,
            SingleCandidateAuthorCompletion::Routed,
            "流水写失败不改变既有落门收口语义"
        );
        let persisted = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("reload session after ledger write failure");
        assert_eq!(
            persisted.status,
            WorkspaceSessionStatus::WaitingForHuman,
            "流水写失败不得把会话打成 failed"
        );
        assert!(
            persisted
                .policy_diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "candidate_attempt_log_write_failed"),
            "流水写失败必须降级为可观测静默（记入运行日志）：{:?}",
            persisted.policy_diagnostics
        );
    }
}
