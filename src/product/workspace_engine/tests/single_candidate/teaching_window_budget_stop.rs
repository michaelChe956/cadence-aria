// Task 7（tasks 7.1，REQ-SG-05）：SC 人工修订扣减/结算的 SG-05 接线回归。
// bound-sc-generation-teaching 把通道 C 的预算归属收口为「窗口 attempt 上限
// 表达、不扣 A/B 池」，该收口不得波及 SC 人工修订路径的既有计费、上限与
// 结算语义（本 change 对该路径零代码改动）。用例驱动一次完整的门内修订
// turn（feedback reserve → running → Accepted），钉住扣减/结算对照值；
// 既有测试族（conversational_gate_revision / amendment_real_chain）另行
// 定向复跑，两者共同构成 7.1 的「不受影响」验收面。
mod teaching_window_budget_stop {
    use super::*;
    use crate::product::json_store::write_json;
    use crate::product::models::{
        HumanGateTurnStatus, UserBudgetReceiptKind, WorkspaceSessionStatus,
    };
    use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason};
    use crate::product::workspace_engine::{HumanGateCommandOutcome, HumanGateFeedbackInput};

    const REVISION_CANDIDATE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
    ));

    /// 自含夹具（同 conversational_gate_revision::durable_revision_fixture 的
    /// 形状；跨测试树不取私有依赖）：SingleCandidate 会话 + 预算 3 的门快照
    /// + rep4 候选 artifact，引擎镜像同步到 HumanConfirm。
    fn budget_stop_revision_fixture() -> (
        tempfile::TempDir,
        LifecycleStore,
        WorkspaceEngine,
    ) {
        let (tmp, _checkpoint_store, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_draft_candidate("budget_stop_sg05");
        let mut record = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("durable session");
        record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
        record.review_rounds = 0;
        record.status = WorkspaceSessionStatus::WaitingForHuman;
        record.human_gate_snapshot = Some(HumanGateSnapshot {
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            attempts_used: 0,
            manual_repairs_remaining: 3,
            trigger: HumanReason::NativeHumanRequired,
            resumable: false,
            accepted_feedback_turns: Some(0),
            candidate_recovery: None,
        });
        write_json(
            &lifecycle
                .app_paths()
                .issue_root(&record.project_id, &record.issue_id)
                .join("workspace-sessions")
                .join(format!("{}.json", record.id)),
            &record,
        )
        .expect("persist human gate session");
        engine.session.flow_kind = record.flow_kind;
        engine.session.stage = crate::product::workspace_engine::WorkspaceStage::HumanConfirm;
        engine.session.session_status = WorkspaceSessionStatus::WaitingForHuman;
        engine.session.human_gate_snapshot = record.human_gate_snapshot.clone();
        engine.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
            markdown: REVISION_CANDIDATE.to_string(),
            diff: None,
        });
        (tmp, lifecycle, engine)
    }

    async fn open_running_revision_turn(
        engine: &mut WorkspaceEngine,
        command_id: &str,
    ) -> String {
        let outcome = engine
            .handle_human_gate_feedback(HumanGateFeedbackInput {
                command_id: command_id.to_string(),
                feedback: "修订当前候选标题".to_string(),
            })
            .await
            .expect("open human gate turn");
        let turn_id = match outcome {
            HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
            other => panic!("expected opened turn, got {other:?}"),
        };
        engine
            .mark_human_gate_turn_running(&turn_id)
            .expect("mark turn running");
        turn_id
    }

    /// 7.1（REQ-SG-05）：SC 人工修订的扣减与结算语义不受本 change 影响。
    /// 一次成功修订 turn 的既有合同：reserve 扣门快照预算恰 1 且与
    /// accepted_feedback_turns 同 CAS 递增；交付后以 DeliveredCharge 结算
    /// 恰一次、活动 hold 清零；policy 计数（repairs_used /
    /// manual_repairs_used）不双计；不经通道 C 窗口/预领记账。
    #[tokio::test]
    async fn sc_manual_revision_billing_settlement_unchanged() {
        let (_root, lifecycle, mut engine) = budget_stop_revision_fixture();
        let before = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("session before revision turn");
        assert_eq!(
            (
                before.run_history.repairs_used,
                before.run_history.manual_repairs_used
            ),
            (0, 0),
            "前提：policy 两池计数起点为 0"
        );
        let ledger_before = before.provider_start_ledger.len();

        let turn_id =
            open_running_revision_turn(&mut engine, "budget_stop_sg05_command").await;
        // reserve 扣减：门快照预算 3 → 2，反馈轮次计数同 CAS 递增。
        let reserved = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("session after reservation");
        let reserved_snapshot = reserved.human_gate_snapshot.as_ref().expect("snapshot");
        assert_eq!(
            reserved_snapshot.manual_repairs_remaining, 2,
            "reserve 扣门快照预算恰 1（3 → 2）"
        );
        assert_eq!(
            reserved_snapshot.accepted_feedback_turns,
            Some(1),
            "accepted_feedback_turns 与预算预留同一 CAS 原子递增"
        );

        let result = engine
            .run_sc_manual_revision_turn(
                &turn_id,
                format!("provider preamble\n{REVISION_CANDIDATE}"),
            )
            .await
            .expect("valid provider output should complete revision");
        assert!(
            matches!(
                result,
                crate::product::workspace_engine::ScManualRevisionResult::Accepted { .. }
            ),
            "合法修订输出必须被接受"
        );
        let after = lifecycle
            .get_workspace_session(&engine.session().session_id)
            .expect("session after settlement");
        let turn = lifecycle
            .get_human_gate_turn(engine.session().session_id.as_str(), &turn_id)
            .expect("completed turn");
        assert_eq!(
            turn.status,
            HumanGateTurnStatus::Completed,
            "修订 turn 以 Completed 收口"
        );
        assert!(turn.result_artifact_ref.is_some());
        let after_snapshot = after.human_gate_snapshot.as_ref().expect("snapshot");
        assert_eq!(
            after_snapshot.manual_repairs_remaining, 2,
            "交付不恢复（也不重复扣）门快照预算"
        );
        assert_eq!(
            after_snapshot.accepted_feedback_turns,
            Some(1),
            "同 turn 幂等：计数不因结算再动"
        );
        let governance = after
            .structural_governance_state
            .as_ref()
            .expect("structural governance state after settlement");
        assert_eq!(
            governance
                .user_receipts
                .iter()
                .filter(|receipt| {
                    receipt.turn_id == turn_id
                        && receipt.kind == UserBudgetReceiptKind::DeliveredCharge
                })
                .count(),
            1,
            "DeliveredCharge 结算恰一次（按 turn 幂等）"
        );
        assert_eq!(
            governance.active_hold_count(),
            0,
            "结算后无残留活动 hold"
        );
        assert_eq!(
            governance.available_user_budget(),
            2,
            "可用预算 = 总额 3 − 已交付计费 1"
        );
        assert_eq!(
            (
                after.run_history.repairs_used,
                after.run_history.manual_repairs_used
            ),
            (0, 0),
            "人工修订轮次不双计 policy 的 repairs_used / manual_repairs_used"
        );
        // SG-05 既有面：修订 turn 预领自己的 human_gate 家族 provider start
        //（恰 1 条，携带 attempt 关联）；不得借用通道 C 的记账面——无
        // single_candidate_author 教学键、无教学 attempt 键关联。
        assert_eq!(
            after.provider_start_ledger.len() - ledger_before,
            1,
            "修订 turn 恰预领 1 条自身 provider start"
        );
        let revision_entry = after
            .provider_start_ledger
            .iter()
            .find(|entry| {
                entry.provider_start_idempotency_key.starts_with("human_gate:")
            })
            .expect("revision provider start uses the human-gate key family");
        assert!(
            revision_entry.provider_attempt_key.is_none(),
            "修订预领不携带教学 attempt 键关联（不经通道 C 窗口）"
        );
        assert!(
            !after.provider_start_ledger.iter().any(|entry| entry
                .provider_start_idempotency_key
                .starts_with("single_candidate_author:")),
            "修订轮不产生 SC author 教学启动键"
        );
        assert!(
            after.sc_generation_teaching_windows.is_empty(),
            "人工修订不得触发任何教学窗口"
        );
        assert!(
            after.repair_reservation.is_none(),
            "人工修订 turn 不落 A 通道预领"
        );
    }
}
