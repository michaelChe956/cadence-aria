use chrono::Utc;

use crate::product::models::{
    HumanGateReservation, HumanGateTurn, HumanGateTurnStatus, WorkspaceType,
};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::web::workspace_ws_types::HumanConfirmDecision;

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

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HumanGateCloseOutcome {
    CompileStarted,
    Confirmed,
    Abandoned,
    Busy { turn_id: String },
}

fn rejected(code: &str, reason: impl Into<String>) -> HumanGateCommandOutcome {
    HumanGateCommandOutcome::Rejected {
        code: code.to_string(),
        reason: reason.into(),
    }
}

fn non_terminal(turn: &HumanGateTurn) -> bool {
    matches!(
        turn.status,
        HumanGateTurnStatus::Reserved | HumanGateTurnStatus::Running
    )
}

fn validate_command_id(command_id: &str) -> Result<(), String> {
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

impl super::WorkspaceEngine {
    /// Reserve one SC conversational-gate turn. This method deliberately
    /// stops at the durable reservation boundary; provider execution belongs
    /// to the later manual-revision task.
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

        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || self.session.stage != super::WorkspaceStage::HumanConfirm
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

        // 构造完整 SC revision prompt 必须发生在 HumanGateTurn CAS 之前。这样候选或
        // 固定契约超出独立预算时，反馈请求只返回 bounded error，不消耗预算/ledger。
        let candidate_markdown = self
            .session
            .artifact
            .clone()
            .and_then(|artifact| artifact.into_markdown())
            .unwrap_or_default();
        let grammar_boundary =
            crate::product::work_item_split_engine::prompts::work_item_plan_markdown_grammar();
        if let Err(error) = super::prompts::build_sc_manual_revision_prompt(
            super::prompts::ScManualRevisionPromptInput {
                candidate_markdown: &candidate_markdown,
                feedback: &input.feedback,
                grammar_boundary: &grammar_boundary,
                language_rule: super::prompts::LANGUAGE_RULE_FILE_CONTENT,
            },
        ) {
            let (code, reason) = error.split_once(':').map_or(
                ("HUMAN_GATE_REVISION_PROMPT_TOO_LARGE", error.as_str()),
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

        let now = Utc::now().to_rfc3339();
        let turn = HumanGateTurn {
            turn_id: format!("human_gate_turn_{}", uuid::Uuid::new_v4()),
            session_id: self.session.session_id.clone(),
            command_id: input.command_id.clone(),
            feedback_text: input.feedback,
            status: HumanGateTurnStatus::Reserved,
            attempt_no: 1,
            budget_reserved: 1,
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
        Ok(HumanGateCommandOutcome::TurnOpened {
            turn: saved_turn,
            remaining_budget: remaining_budget - 1,
        })
    }

    pub(crate) async fn handle_human_gate_termination(
        &mut self,
        decision: HumanConfirmDecision,
    ) -> Result<HumanGateCommandOutcome, String> {
        let store = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || self.session.stage != super::WorkspaceStage::HumanConfirm
        {
            return Ok(rejected(
                "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID",
                "human gate termination is only available for a single-candidate work-item plan in human_confirm",
            ));
        }
        if let Some(turn) = store
            .list_human_gate_turns(&self.session.session_id)
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(non_terminal)
        {
            return Ok(HumanGateCommandOutcome::Busy {
                turn_id: turn.turn_id,
            });
        }
        let _close_outcome = self.close_human_gate(decision).await?;
        Err("human gate close outcome mapping is deferred to Task 4.1".to_string())
    }

    /// Delegation point reserved for Task 4.1. It intentionally does not
    /// duplicate the legacy human-confirm close path.
    #[allow(dead_code)]
    pub(crate) async fn close_human_gate(
        &mut self,
        _decision: HumanConfirmDecision,
    ) -> Result<HumanGateCloseOutcome, String> {
        Err("close_human_gate is not implemented until Task 4.1".to_string())
    }
}

#[allow(dead_code)]
fn _close_outcome_marker(_outcome: HumanGateCloseOutcome) {}
