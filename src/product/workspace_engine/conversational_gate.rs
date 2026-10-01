use chrono::Utc;

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::PlanAmendmentContext;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    HumanGateReservation, HumanGateTurn, HumanGateTurnStatus, SingleCandidatePhase,
    WorkItemPlanCompileStatus, WorkItemPlanCompileTransaction, WorkItemSplitFinding,
    WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_plan_compiler::grammar;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;

mod story_terminate;

pub(crate) enum ScManualRevisionResult {
    Accepted { artifact_ref: String },
    ValidationRejected { diagnostics: Vec<String> },
}
pub(crate) const HUMAN_GATE_COMMAND_ID_MAX_BYTES: usize = 256;
pub(crate) const HUMAN_GATE_BUDGET_EXHAUSTED_CODE: &str = "HUMAN_GATE_BUDGET_EXHAUSTED";
/// Fixed upper bound for real provider starts belonging to one logical turn.
/// A turn is reserved as attempt 1 and may be resumed once as attempt 2.
pub(crate) const HUMAN_GATE_PROVIDER_MAX_ATTEMPTS: u32 = 2;

#[derive(Debug, Clone)]
pub(crate) struct HumanGateFeedbackInput {
    pub command_id: String,
    pub feedback: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HumanGateCommandOutcome {
    TurnOpened {
        turn: HumanGateTurn,
        remaining_budget: u32,
        prompt: String,
    },
    Busy {
        turn_id: String,
    },
    Replayed {
        turn: HumanGateTurn,
    },
    Rejected {
        code: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HumanGateCloseOutcome {
    Confirmed,
    Abandoned,
    Busy {
        turn_id: String,
    },
    /// 地雷 2（F3 run1d 实证）：另一 worker 已先行关门（durable Running/Confirmed），
    /// 迟到 confirm 的幂等 no-op 结果——不 abort 会话，可见提示事件由 engine 发出。
    AlreadyClosed {
        status: WorkspaceSessionStatus,
    },
}

/// SC 门关门决策（L0 typed 重承载，REQ-RET-02/REQ-CG-04）：approve=既有
/// `Confirm` 入站变体；abandon=显式 `AbandonHumanGate` 入站命令。与 legacy
/// human-confirm 决策枚举零共用（该旧枚举随 L2 退役删除）；legacy
/// RequestChange 在此类型面上不可表达（SC 门结构性拒绝，wire 面由 stage
/// 白名单直接拒绝）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HumanGateCloseDecision {
    Approve,
    Abandon,
}

/// confirm 后 compile 失败的结构化详情：engine 层 Err 文本与 web 层
/// ProtocolError.context 共用同一 durable 来源（最近一次 Failed compile
/// transaction 的 failure_reason + validator findings）。
#[derive(Debug, Clone)]
pub(crate) struct HumanGateCloseCompileFailure {
    pub(crate) failure_reason: Option<String>,
    pub(crate) findings: Vec<WorkItemSplitFinding>,
}

fn rejected(code: &str, reason: impl Into<String>) -> HumanGateCommandOutcome {
    HumanGateCommandOutcome::Rejected {
        code: code.to_string(),
        reason: reason.into(),
    }
}

#[cfg(test)]
pub(crate) use story_terminate::register_story_terminate_drift_hook;

fn non_terminal(turn: &HumanGateTurn) -> bool {
    matches!(
        turn.status,
        HumanGateTurnStatus::Reserved | HumanGateTurnStatus::Running
    )
}

pub(crate) fn validate_command_id(command_id: &str) -> Result<(), String> {
    if command_id.trim().is_empty() {
        return Err("INVALID_COMMAND_ID: command_id must not be blank".to_string());
    }
    if command_id.len() > HUMAN_GATE_COMMAND_ID_MAX_BYTES {
        return Err(format!(
            "HUMAN_GATE_COMMAND_ID_TOO_LARGE: command_id exceeds {} bytes",
            HUMAN_GATE_COMMAND_ID_MAX_BYTES
        ));
    }
    Ok(())
}

fn validate_feedback(feedback: &str) -> Result<(), String> {
    super::prompts::validate_sc_manual_revision_feedback(feedback)
}
/// C2 Task 11：SC 修订组装记录文件（session 分区；"组装 digest 记录"）。
pub(crate) const SC_REVISION_ASSEMBLY_FILE: &str = "sc-revision-assembly.json";
/// C2 Task 11：大候选停等等待事实文件（session 分区；Task 12 投影
/// kind `large_candidate_blocked`）。
pub(crate) const SC_REVISION_BLOCKED_FILE: &str = "sc-revision-blocked.json";

// C2 Task 11：分块交付与静态预算表供 WS/REST 层与测试消费（同模块再导出）。
pub(crate) use super::prompts::{render_sc_revision_delivery, sc_provider_input_budget};

/// C2 Task 11：SC 修订组装记录（inline 交付不落账——零新增持久化）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ScRevisionAssemblyRecord {
    pub session_id: String,
    pub command_id: String,
    pub total_bytes: usize,
    pub transport: super::prompts::ScCandidateTransport,
    pub created_at: String,
}

/// C2 Task 11：大候选停等等待事实（CAS 前拒绝时落账；操作面
/// segmented_revision／retry 由用户点击后才开新回合）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ScRevisionBlockedRecord {
    pub session_id: String,
    pub command_id: String,
    pub reason_code: String,
    pub detail: String,
    pub total_bytes: usize,
    pub hard_limit_bytes: usize,
    pub actions: Vec<String>,
    pub created_at: String,
}

/// "CODE: reason" 稳定码拆分（无前缀时整串作 code）。
fn split_stable_error(error: &str) -> (String, &str) {
    match error.split_once(':') {
        Some((code, reason)) => (code.to_string(), reason.trim()),
        None => (error.to_string(), ""),
    }
}
include!("conversational_gate_parts/sc_revision_delivery.inc.rs");

impl super::WorkspaceEngine {
    /// F-49/A6：门内人工修订轮必须与普通 SC 修订同构地落在一个 author 节点上。
    ///
    /// 修订轮此前沿用 `active_timeline_node_id()`——门内活动节点就是 HumanConfirm
    /// 门节点，于是 revision prompt / streaming_content / output 事件 / artifact_ref
    /// 与「产物版本 source_node_id」全部写进门节点 detail。门节点
    /// `node_type=human_confirm` 在前端对话流 rebuild 的 `chatRoleForTimelineNode`
    /// 判为 `null`，整节点零条目（实测 issue_0002/workspace_session_0009：artifact v3
    /// 挂 timeline_node_006，74 条 rebuild 条目里既无 node_006 也无 v3），用户侧
    /// 「author 修订步不可见」。
    ///
    /// 本入口给出与普通 SC 修订（`single_candidate.rs` 的 SC author Run）同构的载体：
    /// 活动节点已是 AuthorRun 时复用——provider 在修订 run 内中断、恢复以同一 turn
    /// 重跑时不得重复建节点（与 SC author 的选节点约定一致）；否则新建 AuthorRun
    /// 节点（agent=author provider；stage 取会话阶段 human_confirm，理由见方法内注释）。
    /// 修订事实随之落在该节点 detail 上，前端既有 rebuild 分支（`author_run` →
    /// author 气泡）与 live `TimelineNodeCreated` 帧即可渲染「author 正在修订 /
    /// 修订完成」；本入口不新造事件类型，`human_gate_turn_open`/
    /// `human_gate_turn_completed` 契约（REQ-CG-01/03）零变化。
    pub(crate) async fn begin_work_item_plan_human_gate_revision_run(&mut self) -> String {
        use crate::product::workspace_engine::TimelineNodeDraft;
        use crate::web::workspace_ws_types::{TimelineNodeStatus, TimelineNodeType};

        if self.active_node_type() == Some(TimelineNodeType::AuthorRun)
            && let Some(node_id) = self.active_node_id.clone()
        {
            return node_id;
        }
        // stage 必须与会话阶段一致（human_confirm）：`WorkspaceEngine::new_persistent`
        // 对非空 durable 时间线以「活动节点的 stage」重建 `session.stage`
        // （lifecycle.rs 的 `workspace_stage_from_ws_stage`），而门内修订期间会话阶段
        // 恒为 human_confirm——门命令语义（REQ-CG-02 单飞：在飞 turn 期间 feedback/
        // approve/abandon 返回 gate_busy）以该阶段为前提。若本节点写成 running，
        // 重连/第二 worker 重建后会把阶段推导为 running，门命令被拒为
        // STAGE_INVALID（campaign 单飞用例实测回归），用户重连即失去门内反馈能力。
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::AuthorRun,
            agent: Some(self.session.author_provider.clone()),
            stage: super::WorkspaceStage::HumanConfirm,
            round: None,
            title: "Work Item Plan 生成".to_string(),
            summary: None,
            status: TimelineNodeStatus::Active,
        })
        .await
    }

    pub(crate) fn mark_human_gate_turn_running(&mut self, turn_id: &str) -> Result<(), String> {
        use crate::product::models::HumanGateTurnStatus;
        let store = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        let mut turn = store
            .get_human_gate_turn(&self.session.session_id, turn_id)
            .map_err(|error| error.to_string())?;
        if turn.status == HumanGateTurnStatus::Running {
            return Ok(());
        }
        if turn.status != HumanGateTurnStatus::Reserved {
            return Err(format!("human gate turn {turn_id} is not reservable"));
        }
        turn.status = HumanGateTurnStatus::Running;
        turn.updated_at = Utc::now().to_rfc3339();
        let saved = store
            .update_human_gate_turn(&expected, turn)
            .map_err(|error| error.to_string())?;
        self.session.provider_start_ledger = saved.provider_start_ledger;
        Ok(())
    }

    pub(crate) async fn fail_human_gate_turn(
        &mut self,
        turn_id: &str,
        failure_class: crate::product::models::HumanGateTurnFailureClass,
    ) -> Result<(), String> {
        use crate::product::models::HumanGateTurnStatus;
        let store = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        let mut turn = store
            .get_human_gate_turn(&self.session.session_id, turn_id)
            .map_err(|error| error.to_string())?;
        if matches!(
            turn.status,
            HumanGateTurnStatus::Completed | HumanGateTurnStatus::Failed
        ) {
            return Ok(());
        }
        turn.status = HumanGateTurnStatus::Failed;
        turn.failure_class = Some(failure_class);
        turn.updated_at = Utc::now().to_rfc3339();
        let saved = store
            .update_human_gate_turn(&expected, turn)
            .map_err(|error| error.to_string())?;
        self.session.provider_start_ledger = saved.provider_start_ledger;
        Ok(())
    }
}

include!("conversational_gate_parts/revision_turn.inc.rs");

impl super::WorkspaceEngine {
    pub(crate) async fn handle_human_gate_feedback(
        &mut self,
        input: HumanGateFeedbackInput,
    ) -> Result<HumanGateCommandOutcome, String> {
        // A replay is intentionally checked before stage and budget checks so
        // reconnects can safely resend a command after the session advanced.
        if let Err(error) = validate_command_id(&input.command_id) {
            let (code, reason) = error
                .split_once(':')
                .map_or(("INVALID_COMMAND_ID", error.as_str()), |(code, reason)| {
                    (code, reason.trim())
                });
            return Ok(rejected(code, reason));
        }
        let store = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        if let Some(turn) = store
            .get_human_gate_turn_by_command_id(&self.session.session_id, &input.command_id)
            .map_err(|error| error.to_string())?
        {
            return Ok(HumanGateCommandOutcome::Replayed { turn });
        }

        // REQ-GCE-03 场景二：attempt 处于 AwaitingPlanAmendment 期间，原 SC plan
        // session 以 amendment 上下文重开同一人工门接受 typed feedback。stage 不在
        // HumanConfirm 时先探测并校验 session/plan lineage 与 PlanAmendmentContext，
        // 命中才放行；否则保持既有 stage 拒绝语义（零副作用）。
        let amendment_gate = self.probe_amendment_gate_context()?;
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || (self.session.stage != super::WorkspaceStage::HumanConfirm
                && amendment_gate.is_none())
        {
            return Ok(rejected(
                "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID",
                "human gate feedback is only available for a single-candidate work-item plan in human_confirm",
            ));
        }
        if let Err(error) = validate_feedback(&input.feedback) {
            let (code, reason) = error.split_once(':').map_or(
                ("INVALID_HUMAN_GATE_FEEDBACK", error.as_str()),
                |(code, reason)| (code, reason.trim()),
            );
            return Ok(rejected(code, reason));
        }

        let turns = store
            .list_human_gate_turns(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        if let Some(turn) = turns.iter().find(|turn| non_terminal(turn)) {
            return Ok(HumanGateCommandOutcome::Busy {
                turn_id: turn.turn_id.clone(),
            });
        }

        let remaining_budget = self
            .session
            .human_gate_snapshot
            .as_ref()
            .map(|snapshot| snapshot.manual_repairs_remaining)
            .ok_or_else(|| "human gate snapshot is missing".to_string())?;
        if remaining_budget == 0 {
            return Ok(rejected(
                HUMAN_GATE_BUDGET_EXHAUSTED_CODE,
                "manual repair budget is exhausted",
            ));
        }

        // 构造完整 SC revision prompt 必须发生在 HumanGateTurn CAS 之前。这样候选或
        // 固定契约超出预算时，反馈请求只返回 bounded error，不消耗预算/ledger。
        let prompt = match self.build_sc_manual_revision_prompt_for_turn(&input.feedback) {
            Ok(prompt) => prompt,
            Err(error) => {
                let (code, reason) = error.split_once(':').map_or(
                    ("HUMAN_GATE_REVISION_CANDIDATE_MISSING", error.as_str()),
                    |(code, reason)| (code, reason.trim()),
                );
                return Ok(rejected(code, reason));
            }
        };

        // C2 Task 11（REQ-CG-03，#15）：完整预算口径——扣回合前以 UTF-8 字节
        // 计算完整组装输入并与所选 provider 静态预算比较：≤inline 整体内联
        // （零新增持久化，现行为）；超 inline 但硬限内先持久化组装 artifact
        // （readback digest 校验）经 ArtifactRef 传输完整原文并记录组装 digest；
        // artifact 不可读或完整输入超硬限在 turn CAS 之前拒绝停等（门状态、
        // manual_repairs_remaining 与 provider 启动计数不变），落大候选停等
        // 等待事实（Task 12 投影 large_candidate_blocked）。
        let prompt = match self.assemble_sc_revision_delivery(&input.command_id, prompt) {
            Ok(prompt) => prompt,
            Err((code, reason)) => return Ok(rejected(&code, reason)),
        };

        let now = Utc::now().to_rfc3339();
        let source_hash = self
            .session
            .artifact
            .as_ref()
            .map(|artifact| {
                use sha2::{Digest, Sha256};
                hex::encode(Sha256::digest(artifact.markdown_or_empty().as_bytes()))
            })
            .unwrap_or_default();
        let turn = HumanGateTurn {
            turn_id: format!("human_gate_turn_{}", uuid::Uuid::new_v4()),
            session_id: self.session.session_id.clone(),
            command_id: input.command_id.clone(),
            feedback_text: input.feedback,
            status: HumanGateTurnStatus::Reserved,
            attempt_no: 1,
            budget_reserved: 1,
            source_hash,
            result_artifact_ref: None,
            failure_class: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let reservation = HumanGateReservation {
            command_id: turn.command_id.clone(),
            turn_id: turn.turn_id.clone(),
            provider_start_idempotency_key: format!("human_gate:{}:attempt:1", turn.turn_id),
            reserved_at: now,
        };
        let expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        // Re-check the durable status immediately before the CAS.  A websocket
        // may have retained a fresh-looking in-memory stage while another
        // worker already advanced the session; reserving a turn in that stale
        // window would reopen a closed gate.
        let expected =
            if expected.status == WorkspaceSessionStatus::Confirmed && amendment_gate.is_some() {
                // amendment 上下文重开：CAS Confirmed→WaitingForHuman，门快照与预算
                // 原样保留（D11 单一预算源），不新建第二个门实例。
                match store.compare_and_reopen_amendment_gate(&expected) {
                    Ok(saved) => {
                        self.session.session_status = saved.status.clone();
                        self.session.human_gate_snapshot = saved.human_gate_snapshot.clone();
                        saved
                    }
                    Err(_) => {
                        return Ok(rejected(
                            "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID",
                            "amendment gate reopen lost the durable race",
                        ));
                    }
                }
            } else {
                expected
            };
        if expected.status != WorkspaceSessionStatus::WaitingForHuman {
            return Ok(rejected(
                "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID",
                "human gate feedback requires a waiting_for_human session",
            ));
        }
        let (saved, saved_turn) =
            match store.compare_and_reserve_human_gate_turn(&expected, turn, reservation) {
                Ok(result) => result,
                Err(error) => {
                    // Another websocket worker may have won the CAS. Reconcile
                    // from disk and report the durable single-flight owner.
                    if let Some(existing) = store
                        .get_human_gate_turn_by_command_id(
                            &self.session.session_id,
                            &input.command_id,
                        )
                        .map_err(|read_error| read_error.to_string())?
                    {
                        return Ok(HumanGateCommandOutcome::Replayed { turn: existing });
                    }
                    if let Some(existing) = store
                        .list_human_gate_turns(&self.session.session_id)
                        .map_err(|read_error| read_error.to_string())?
                        .into_iter()
                        .find(non_terminal)
                    {
                        return Ok(HumanGateCommandOutcome::Busy {
                            turn_id: existing.turn_id,
                        });
                    }
                    return Err(error.to_string());
                }
            };
        self.session.human_gate_snapshot = saved.human_gate_snapshot;
        self.session.provider_start_ledger = saved.provider_start_ledger;
        // C2 Task 11：成功开回合闭合大候选停等等待事实（如有）。
        self.clear_sc_revision_blocked();
        Ok(HumanGateCommandOutcome::TurnOpened {
            turn: saved_turn,
            remaining_budget: remaining_budget - 1,
            prompt,
        })
    }

    pub(crate) async fn handle_human_gate_termination(
        &mut self,
        decision: HumanGateCloseDecision,
    ) -> Result<HumanGateCloseOutcome, String> {
        // F-18（w2c 实测矩阵）：story/design 会话（legacy 流）恒停
        // author_confirm 门——approve 有 HTTP confirm 端点，terminate 此前
        // 零通路。typed abandon 对 story/design 门分流到本门关门语义
        //（门开态/幂等判据在 terminate_story_author_gate 内读 durable）；
        // WorkItemPlan 会话仍走 SC close。
        if matches!(decision, HumanGateCloseDecision::Abandon)
            && matches!(
                self.session.workspace_type,
                WorkspaceType::Story | WorkspaceType::Design
            )
        {
            return self.terminate_story_author_gate().await;
        }
        self.close_human_gate(decision).await
    }

    /// Atomically closes the single-candidate human gate. Approval enters the
    /// existing deterministic compile path and only becomes Confirmed after
    /// that path durably publishes the plan; termination is terminal without
    /// creating a compile transaction.
    pub(crate) async fn close_human_gate(
        &mut self,
        decision: HumanGateCloseDecision,
    ) -> Result<HumanGateCloseOutcome, String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || self.session.stage != super::WorkspaceStage::HumanConfirm
        {
            return Err("human gate close is only available for a single-candidate work-item plan in human_confirm".to_string());
        }
        self.last_gate_close_compile_failure = None;

        let expected = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        if expected.status != WorkspaceSessionStatus::WaitingForHuman {
            // 地雷 2（先到者赢）：durable 已非 WaitingForHuman 说明另一 worker 已
            // 关门/推进；迟到 approve/abandon 重读 durable 翻译（幂等
            // AlreadyClosed/明确已终止错误），不再把迟到者当噪音 Err 上抛
            //（曾致 session aborted）。
            return self
                .translate_lost_human_gate_close_race(
                    &lifecycle,
                    decision,
                    "human gate close requires a waiting_for_human session".to_string(),
                )
                .await;
        }
        if let Some(turn) = lifecycle
            .list_human_gate_turns(&self.session.session_id)
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(non_terminal)
        {
            return Ok(HumanGateCloseOutcome::Busy {
                turn_id: turn.turn_id,
            });
        }

        // C1（REQ-C1-GATE-01）：门快照携带候选恢复评估 label 时（C1 后开启
        // 的候选审批门），approve 前按 durable 事实重估完整性；不完整则
        // fail-closed——不扣预算、不启动 compile/provider、不伪造已批准。
        // label 缺席（旧会话/legacy 门）保持既有 approve 语义零回归。
        if matches!(decision, HumanGateCloseDecision::Approve)
            && expected
                .human_gate_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.candidate_recovery.as_ref())
                .is_some()
        {
            let facts = self.assess_candidate_snapshot()?;
            if !facts.complete {
                return Err(format!(
                    "CANDIDATE_SNAPSHOT_INCOMPLETE: missing {:?}; approve is fail-closed until the candidate gate is recovered or rebuilt",
                    facts.missing
                ));
            }
        }

        match decision {
            HumanGateCloseDecision::Approve => {
                let saved = match lifecycle
                    .compare_and_save_human_gate_close(&expected, WorkspaceSessionStatus::Running)
                {
                    Ok(saved) => saved,
                    Err(error) => {
                        return self
                            .translate_human_gate_close_cas_error(&lifecycle, decision, error)
                            .await;
                    }
                };
                self.session.session_status = saved.status;
                // 人工权威升级后的内存同步：close CAS 已把 durable phase 原子提升
                // 为 Approval，内存相位必须同步，否则 compile 链以内存 phase 判
                // auto_confirm 时 compile 会成功但不落 Confirmed 而回
                // enter_human_confirm。
                self.session.single_candidate_phase = saved.single_candidate_phase.clone();
                self.session.human_gate_snapshot = saved.human_gate_snapshot;
                self.enter_policy_valid_work_item_plan_compile().await;

                let durable = lifecycle
                    .get_workspace_session(&self.session.session_id)
                    .map_err(|error| error.to_string())?;
                self.session.session_status = durable.status.clone();
                self.session.single_candidate_phase = durable.single_candidate_phase.clone();
                self.session.human_gate_snapshot = durable.human_gate_snapshot.clone();
                if durable.status != WorkspaceSessionStatus::Confirmed
                    || durable.single_candidate_phase
                        != Some(crate::product::models::SingleCandidatePhase::Completed)
                {
                    return Err(self.single_candidate_gate_close_failure_error());
                }
                let _ = self
                    .event_tx
                    .send(super::EngineEvent::HumanGateClosed {
                        decision: "confirm".to_string(),
                        stage: self.session.stage.as_str().to_string(),
                    })
                    .await;
                Ok(HumanGateCloseOutcome::Confirmed)
            }
            HumanGateCloseDecision::Abandon => {
                let saved = match lifecycle.compare_and_save_human_gate_close(
                    &expected,
                    WorkspaceSessionStatus::Terminated,
                ) {
                    Ok(saved) => saved,
                    Err(error) => {
                        return self
                            .translate_human_gate_close_cas_error(&lifecycle, decision, error)
                            .await;
                    }
                };
                self.session.session_status = saved.status;
                self.session.human_gate_snapshot = saved.human_gate_snapshot;
                let terminal_stage = super::WorkspaceStage::Completed;
                self.session.stage = terminal_stage.clone();
                let _ = self
                    .event_tx
                    .send(super::EngineEvent::HumanGateClosed {
                        decision: "terminate".to_string(),
                        stage: terminal_stage.as_str().to_string(),
                    })
                    .await;
                self.complete_active_node(Some("已终止".to_string())).await;
                self.transition_stage(terminal_stage).await;
                let _ = self
                    .create_timeline_node(super::TimelineNodeDraft {
                        node_type: super::TimelineNodeType::Completed,
                        agent: None,
                        stage: super::WorkspaceStage::Completed,
                        round: None,
                        title: "流程终止".to_string(),
                        summary: Some("已终止".to_string()),
                        status: super::TimelineNodeStatus::Completed,
                    })
                    .await;
                Ok(HumanGateCloseOutcome::Abandoned)
            }
        }
    }

    /// close CAS 错误的 engine 层翻译（地雷 2）：store 的 expected 快照单飞
    /// 原子性一字不动；仅在 Conflict 时重读 durable 把迟到者翻译成幂等结果/明确
    /// 错误，其余错误与非幂等真冲突维持既有上抛。
    async fn translate_human_gate_close_cas_error(
        &mut self,
        lifecycle: &LifecycleStore,
        decision: HumanGateCloseDecision,
        error: ProductStoreError,
    ) -> Result<HumanGateCloseOutcome, String> {
        let fallback = error.to_string();
        if !matches!(error, ProductStoreError::Conflict { .. }) {
            return Err(fallback);
        }
        self.translate_lost_human_gate_close_race(lifecycle, decision, fallback)
            .await
    }

    /// 迟到 close 命令的 durable 重读翻译：先到者赢、后到者幂等友好。
    /// - durable 已 Terminated → 明确「gate 已终止」错误（非 conflict 噪音）
    /// - durable 已 Running/Confirmed 且本命令为 confirm 方向 → 幂等
    async fn translate_lost_human_gate_close_race(
        &mut self,
        lifecycle: &LifecycleStore,
        decision: HumanGateCloseDecision,
        fallback: String,
    ) -> Result<HumanGateCloseOutcome, String> {
        let durable = match lifecycle.get_workspace_session(&self.session.session_id) {
            Ok(durable) => durable,
            Err(_) => return Err(fallback),
        };
        let direction = match decision {
            HumanGateCloseDecision::Approve => "confirm",
            HumanGateCloseDecision::Abandon => "terminate",
        };
        match (&durable.status, decision) {
            (WorkspaceSessionStatus::Terminated, _) => Err(format!(
                "single-candidate human gate already terminated; late {direction} is rejected (session {})",
                self.session.session_id
            )),
            (
                status @ (WorkspaceSessionStatus::Running | WorkspaceSessionStatus::Confirmed),
                HumanGateCloseDecision::Approve,
            ) => {
                // 先到者已赢：迟到 confirm 是幂等 no-op。同步 in-memory 会话状态并
                // 发一条可见提示事件；不产生第二个 HumanGateClosed，也不 abort。
                self.session.session_status = status.clone();
                self.session.single_candidate_phase = durable.single_candidate_phase.clone();
                self.session.human_gate_snapshot = durable.human_gate_snapshot.clone();
                let _ = self
                    .event_tx
                    .send(super::EngineEvent::ProtocolError {
                        code: "HUMAN_GATE_ALREADY_CLOSED".to_string(),
                        message: "single-candidate human gate already closed by an earlier confirm; this late confirm is an idempotent no-op"
                            .to_string(),
                        context: Some(serde_json::json!({
                            "session_id": self.session.session_id,
                            "decision": "confirm",
                            "durable_status": serde_json::to_value(status).unwrap_or_default(),
                        })),
                    })
                    .await;
                Ok(HumanGateCloseOutcome::AlreadyClosed {
                    status: status.clone(),
                })
            }
            _ => Err(fallback),
        }
    }

    /// confirm 后 compile 未达 Confirmed/Completed 的失败错误：附加最近一次
    /// Failed compile transaction 的 failure_reason 与 validator findings 原文，
    /// 并缓存结构化副本供 web 层 ProtocolError.context 上抛（WS 客户端可见）。
    fn single_candidate_gate_close_failure_error(&mut self) -> String {
        let mut message =
            "single-candidate approval compile failed; human gate remains open".to_string();
        let failure = self.latest_single_candidate_compile_failure();
        if let Some(tx) = &failure {
            if let Some(reason) = tx.failure_reason.as_deref() {
                message.push_str("\nfailure_reason: ");
                message.push_str(reason);
            }
            if !tx.validator_findings.is_empty() {
                message.push_str("\nvalidator findings:");
                for finding in &tx.validator_findings {
                    message.push_str(&format!(
                        "\n[{}] {}: {} (work_items: {})",
                        finding.severity.as_str(),
                        finding.code,
                        finding.message,
                        finding.work_item_ids.join(", ")
                    ));
                }
            }
        }
        self.last_gate_close_compile_failure = failure.map(|tx| HumanGateCloseCompileFailure {
            failure_reason: tx.failure_reason.clone(),
            findings: tx.validator_findings.clone(),
        });
        message
    }

    fn latest_single_candidate_compile_failure(&self) -> Option<WorkItemPlanCompileTransaction> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan {
            return None;
        }
        self.work_item_plan_store()
            .ok()?
            .list_compile_transactions(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .ok()?
            .into_iter()
            .filter(|tx| tx.status == WorkItemPlanCompileStatus::Failed)
            .max_by(|left, right| left.created_at.cmp(&right.created_at))
    }

    /// web 层读取最近一次 gate close compile 失败的结构化 findings 上下文
    /// （仅在同一 close 调用内失败时非空，避免陈旧 findings 误挂）。
    pub(crate) fn last_human_gate_close_compile_failure_context(
        &self,
    ) -> Option<serde_json::Value> {
        let failure = self.last_gate_close_compile_failure.as_ref()?;
        Some(serde_json::json!({
            "failure_reason": failure.failure_reason,
            "findings": failure.findings,
        }))
    }
}
