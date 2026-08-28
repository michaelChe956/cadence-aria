use super::*;

pub(crate) async fn handle_plan_amendment_confirmation_from_handler(
    app_state: WebAppState,
    engine: Arc<Mutex<WorkspaceEngine>>,
    outbound_tx: mpsc::Sender<OutboundControl>,
    amendment_id: String,
) {
    let result = {
        let mut engine = engine.lock().await;
        engine
            .confirm_and_publish_plan_amendment(&amendment_id, "workspace_user")
            .await
            .map(|_| {
                engine.plan_repair_session_state().map(|snapshot| {
                    (
                        engine.session().project_id.clone(),
                        engine.session().issue_id.clone(),
                        snapshot.request.trigger_attempt_id.clone(),
                    )
                })
            })
    };
    match result {
        Ok(Some((project_id, issue_id, attempt_id))) => {
            if let Err(error) =
                activate_published_plan_amendment(&app_state, &project_id, &issue_id, &attempt_id)
                    .await
            {
                let message = WsOutMessage::ProtocolError {
                    code: "PLAN_AMENDMENT_ACTIVATION_FAILED".to_string(),
                    message: format!("{error:?}"),
                    context: Some(serde_json::json!({ "amendment_id": amendment_id })),
                };
                let _ = send_json_outbound(&outbound_tx, &message).await;
                return;
            }
            let state = engine.lock().await.build_session_state();
            let _ = send_json_outbound(&outbound_tx, &state).await;
        }
        Ok(None) => {
            let message = WsOutMessage::ProtocolError {
                code: "PLAN_AMENDMENT_CONFIRMATION_FAILED".to_string(),
                message: "published plan amendment state is missing".to_string(),
                context: Some(serde_json::json!({ "amendment_id": amendment_id })),
            };
            let _ = send_json_outbound(&outbound_tx, &message).await;
        }
        Err(error) => {
            let message = WsOutMessage::ProtocolError {
                code: "PLAN_AMENDMENT_CONFIRMATION_FAILED".to_string(),
                message: format!("{error:?}"),
                context: Some(serde_json::json!({ "amendment_id": amendment_id })),
            };
            let _ = send_json_outbound(&outbound_tx, &message).await;
        }
    }
}

pub(crate) async fn handle_plan_amendment_cancel_from_handler(
    engine: Arc<Mutex<WorkspaceEngine>>,
    outbound_tx: mpsc::Sender<OutboundControl>,
    amendment_id: String,
    reason: Option<String>,
) {
    let result = {
        let mut engine = engine.lock().await;
        engine.cancel_plan_amendment(&amendment_id, reason).await
    };
    match result {
        Ok(()) => {
            let state = engine.lock().await.build_session_state();
            let _ = send_json_outbound(&outbound_tx, &state).await;
        }
        Err(error) => {
            let message = WsOutMessage::ProtocolError {
                code: "PLAN_AMENDMENT_CANCEL_FAILED".to_string(),
                message: format!("{error:?}"),
                context: Some(serde_json::json!({ "amendment_id": amendment_id })),
            };
            let _ = send_json_outbound(&outbound_tx, &message).await;
        }
    }
}

pub(crate) async fn handle_review_decision_from_handler(
    run_context: ProviderRunContext,
    outbound_tx: mpsc::Sender<OutboundControl>,
    decision: String,
    extra_context: Option<String>,
) {
    let outcome = {
        let mut engine = run_context.engine.lock().await;
        engine.handle_review_decision(decision, extra_context).await
    };

    match outcome {
        Ok(ReviewDecisionOutcome::HumanConfirm) => {}
        Ok(ReviewDecisionOutcome::ConfirmedWithChildSessions { .. }) => {
            // Review decision path never produces child sessions; defensive no-op.
        }
        Ok(ReviewDecisionOutcome::StartWorkItemPlanOutline) => {
            let run_kind = ProviderRunKind::work_item_plan_author_for_durable_flow(
                run_context.session_record.flow_kind,
            );
            if let Err(message) =
                spawn_provider_run_from_handler(run_context, run_kind, outbound_tx.clone()).await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Ok(ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { feedback }) => {
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::WorkItemPlanOutlineRevision { feedback },
                outbound_tx.clone(),
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Ok(ReviewDecisionOutcome::StartWorkItemDraft { feedback }) => {
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::WorkItemPlanDraft { feedback },
                outbound_tx.clone(),
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Ok(ReviewDecisionOutcome::StartWorkItemBatch) => {
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::WorkItemPlanBatch,
                outbound_tx.clone(),
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Ok(ReviewDecisionOutcome::StartRevision) => {
            let run_kind = {
                let engine = run_context.engine.lock().await;
                if engine.session().workspace_type == WorkspaceType::WorkItemPlan {
                    ProviderRunKind::WorkItemPlanRevision {
                        feedback: engine.work_item_plan_revision_feedback(),
                    }
                } else {
                    ProviderRunKind::Revision
                }
            };
            if let Err(message) =
                spawn_provider_run_from_handler(run_context, run_kind, outbound_tx.clone()).await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Err(message) => {
            let err = WsOutMessage::Error { message };
            let _ = send_json_outbound(&outbound_tx, &err).await;
        }
    }
}

pub(crate) async fn handle_author_decision_from_handler(
    run_context: ProviderRunContext,
    outbound_tx: mpsc::Sender<OutboundControl>,
    decision: crate::web::workspace_ws_types::AuthorDecision,
) {
    let outcome = {
        let mut engine = run_context.engine.lock().await;
        engine.handle_author_decision(decision).await
    };

    match outcome {
        Ok(AuthorDecisionOutcome::StartReview) => {
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::ReviewOnly,
                outbound_tx.clone(),
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Ok(AuthorDecisionOutcome::StartWorkItemPlanOutlineRevision { feedback }) => {
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::WorkItemPlanOutlineRevision { feedback },
                outbound_tx.clone(),
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Ok(AuthorDecisionOutcome::HumanConfirm) => {}
        Ok(AuthorDecisionOutcome::PrepareContext) => {
            let state_msg = {
                let engine = run_context.engine.lock().await;
                engine.build_session_state()
            };
            let _ = send_json_outbound(&outbound_tx, &state_msg).await;
        }
        Ok(AuthorDecisionOutcome::StartRevision { feedback: _ }) => {
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::Revision,
                outbound_tx.clone(),
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Ok(AuthorDecisionOutcome::Finalized) => {}
        Err(message) => {
            let err = WsOutMessage::ProtocolError {
                code: "INVALID_AUTHOR_DECISION".to_string(),
                message,
                context: None,
            };
            let _ = send_json_outbound(&outbound_tx, &err).await;
        }
    }
}

pub(crate) async fn handle_human_confirm_from_handler(
    run_context: ProviderRunContext,
    outbound_tx: mpsc::Sender<OutboundControl>,
    decision: HumanConfirmDecision,
    payload: Option<serde_json::Value>,
) {
    let outcome = {
        let mut engine = run_context.engine.lock().await;
        engine.handle_human_confirm(decision, payload).await
    };

    match outcome {
        Ok(ReviewDecisionOutcome::HumanConfirm) => {}
        Ok(ReviewDecisionOutcome::StartWorkItemPlanOutline) => {
            let run_kind = ProviderRunKind::work_item_plan_author_for_durable_flow(
                run_context.session_record.flow_kind,
            );
            if let Err(message) =
                spawn_provider_run_from_handler(run_context, run_kind, outbound_tx.clone()).await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Ok(ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { feedback }) => {
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::WorkItemPlanOutlineRevision { feedback },
                outbound_tx.clone(),
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Ok(ReviewDecisionOutcome::StartWorkItemDraft { feedback }) => {
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::WorkItemPlanDraft { feedback },
                outbound_tx.clone(),
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Ok(ReviewDecisionOutcome::StartWorkItemBatch) => {
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::WorkItemPlanBatch,
                outbound_tx.clone(),
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Ok(ReviewDecisionOutcome::ConfirmedWithChildSessions { .. }) => {}
        Ok(ReviewDecisionOutcome::StartRevision) => {
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::Revision,
                outbound_tx.clone(),
            )
            .await
            {
                let err = WsOutMessage::Error { message };
                let _ = send_json_outbound(&outbound_tx, &err).await;
            }
        }
        Err(message) => {
            let err = WsOutMessage::ProtocolError {
                code: "INVALID_HUMAN_CONFIRM_ACTION".to_string(),
                message,
                context: None,
            };
            let _ = send_json_outbound(&outbound_tx, &err).await;
        }
    }
}

mod inbound;
pub(crate) use inbound::{WorkspaceInboundContext, handle_workspace_inbound_message};
#[cfg(test)]
pub(crate) use inbound::{
    finish_interrupted_recovery_spawn_error, provider_run_kind_for_interrupted_recovery,
};
