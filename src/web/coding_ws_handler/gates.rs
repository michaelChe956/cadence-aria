use chrono::{Duration as ChronoDuration, Utc};
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionStage, CodingGateAction, CodingGateActionType,
    CodingGateKind, CodingGateRequired as CodingGateRequiredModel, CodingStageGateState,
    CodingStageGateStatus,
};
use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, CodingWorkspaceEngineError};
use crate::product::coding_workspace_runner::{
    CodingRunnerCommand, coding_provider_role_for_stage,
};

use super::{CodingWsOutMessage, build_coding_session_state, update_provider_selection};

const STAGE_GATE_COUNTDOWN_SECONDS: u64 = 5;

pub(crate) async fn await_stage_gate(
    command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    coding_store: &CodingAttemptStore,
    engine: &CodingWorkspaceEngine,
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    attempt: &CodingExecutionAttempt,
    stage: CodingExecutionStage,
) -> Result<Option<CodingExecutionAttempt>, CodingWorkspaceEngineError> {
    let Some(role) = coding_provider_role_for_stage(&stage) else {
        return Ok(Some(attempt.clone()));
    };
    let mut current =
        coding_store.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let provider_snapshot = coding_store.get_role_provider_config_snapshot(
        &current.project_id,
        &current.issue_id,
        &current.id,
    )?;
    let mut deadline = Instant::now() + Duration::from_secs(STAGE_GATE_COUNTDOWN_SECONDS);
    let expires_at = stage_gate_expires_at();
    let mut gate = coding_store.create_stage_gate(
        &current.id,
        stage.clone(),
        role,
        expires_at,
        provider_snapshot,
    )?;
    emit_stage_gate(event_tx, coding_store, &current, &gate).await?;

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                let _ = coding_store.update_stage_gate_status(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    &gate.gate_id,
                    CodingStageGateStatus::Expired,
                )?;
                let snapshot = build_coding_session_state(coding_store, current.clone())?;
                let _ = event_tx.send(snapshot).await;
                return Ok(Some(current));
            }
            command = command_rx.recv() => {
                let Some(command) = command else {
                    tokio::time::sleep_until(deadline).await;
                    let _ = coding_store.update_stage_gate_status(
                        &current.project_id,
                        &current.issue_id,
                        &current.id,
                        &gate.gate_id,
                        CodingStageGateStatus::Expired,
                    )?;
                    let snapshot = build_coding_session_state(coding_store, current.clone())?;
                    let _ = event_tx.send(snapshot).await;
                    return Ok(Some(current));
                };
                match command {
                    CodingRunnerCommand::StageGateConfirm { stage: confirm_stage }
                        if confirm_stage == stage =>
                    {
                        let _ = coding_store.update_stage_gate_status(
                            &current.project_id,
                            &current.issue_id,
                            &current.id,
                            &gate.gate_id,
                            CodingStageGateStatus::Confirmed,
                        )?;
                        let snapshot = build_coding_session_state(coding_store, current.clone())?;
                        let _ = event_tx.send(snapshot).await;
                        return Ok(Some(current));
                    }
                    CodingRunnerCommand::StageGateConfirm { .. } => {
                        let _ = event_tx
                            .send(CodingWsOutMessage::CodingProtocolError {
                                code: "coding_stage_gate_mismatch".to_string(),
                                message: "stage gate confirm did not match the open stage gate".to_string(),
                            })
                            .await;
                    }
                    CodingRunnerCommand::PermissionResponse { .. }
                    | CodingRunnerCommand::ChoiceResponse { .. } => {}
                    CodingRunnerCommand::ProviderSelect { role, provider } => {
                        let (updated, changed_role, changed_provider) =
                            match update_provider_selection(
                                coding_store,
                                &current,
                                &role,
                                provider,
                            ) {
                                Ok(result) => result,
                                Err(error) => {
                                    let _ = event_tx
                                        .send(CodingWsOutMessage::CodingProtocolError {
                                            code: "coding_provider_select_failed".to_string(),
                                            message: error.to_string(),
                                        })
                                        .await;
                                    continue;
                                }
                            };
                        current = updated;
                        let provider_snapshot =
                            coding_store.get_role_provider_config_snapshot(
                                &current.project_id,
                                &current.issue_id,
                                &current.id,
                            )?;
                        deadline =
                            Instant::now() + Duration::from_secs(STAGE_GATE_COUNTDOWN_SECONDS);
                        gate = coding_store.refresh_stage_gate(
                            &current.project_id,
                            &current.issue_id,
                            &current.id,
                            &gate.gate_id,
                            stage_gate_expires_at(),
                            provider_snapshot,
                        )?;
                        let _ = event_tx
                            .send(CodingWsOutMessage::CodingProviderConfigUpdated {
                                role: changed_role,
                                provider: changed_provider,
                            })
                            .await;
                        emit_stage_gate(event_tx, coding_store, &current, &gate).await?;
                    }
                    CodingRunnerCommand::AbortAttempt => {
                        let _ = coding_store.update_stage_gate_status(
                            &current.project_id,
                            &current.issue_id,
                            &current.id,
                            &gate.gate_id,
                            CodingStageGateStatus::Cancelled,
                        )?;
                        let updated = engine
                            .handle_abort(&current.project_id, &current.issue_id, &current.id)
                            .await?;
                        crate::web::coding_ws_handler::state::emit_current_session_state(
                            event_tx, coding_store, &updated,
                        )
                        .await?;
                        return Ok(None);
                    }
                }
            }
        }
    }
}

pub(crate) async fn emit_stage_gate(
    event_tx: &mpsc::Sender<CodingWsOutMessage>,
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    gate: &CodingStageGateState,
) -> Result<(), CodingWorkspaceEngineError> {
    let _ = event_tx
        .send(CodingWsOutMessage::CodingGateRequired {
            gate: stage_gate_required(gate.clone()),
        })
        .await;
    let snapshot = build_coding_session_state(coding_store, attempt.clone())?;
    let _ = event_tx.send(snapshot).await;
    Ok(())
}

fn stage_gate_expires_at() -> String {
    (Utc::now() + ChronoDuration::seconds(STAGE_GATE_COUNTDOWN_SECONDS as i64)).to_rfc3339()
}

pub(crate) fn stage_gate_required(gate: CodingStageGateState) -> CodingGateRequiredModel {
    CodingGateRequiredModel {
        gate_id: gate.gate_id,
        kind: CodingGateKind::StageGate,
        title: format!("{:?} Stage Gate", gate.stage),
        description: format!(
            "Waiting to start {:?} with {} provider until {}",
            gate.stage, gate.role, gate.expires_at
        ),
        stage: Some(gate.stage),
        role: Some(gate.role),
        expires_at: Some(gate.expires_at),
        provider_snapshot: Some(gate.provider_snapshot),
        available_actions: vec![
            CodingGateAction {
                action_id: "confirm_stage".to_string(),
                label: "立即开始".to_string(),
                action_type: CodingGateActionType::ConfirmStage,
            },
            CodingGateAction {
                action_id: "abort".to_string(),
                label: "中止 Attempt".to_string(),
                action_type: CodingGateActionType::Abort,
            },
        ],
        reason_code: None,
        evidence_refs: Vec::new(),
        raw_provider_output_ref: None,
    }
}
