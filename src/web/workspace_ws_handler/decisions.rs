use super::*;

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
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::WorkItemPlanAuthor,
                outbound_tx.clone(),
            )
            .await
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
        Ok(AuthorDecisionOutcome::HumanConfirm) => {}
        Ok(AuthorDecisionOutcome::PrepareContext) => {
            let state_msg = {
                let engine = run_context.engine.lock().await;
                engine.build_session_state()
            };
            let _ = send_json_outbound(&outbound_tx, &state_msg).await;
        }
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
            if let Err(message) = spawn_provider_run_from_handler(
                run_context,
                ProviderRunKind::WorkItemPlanAuthor,
                outbound_tx.clone(),
            )
            .await
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
        Ok(ReviewDecisionOutcome::ConfirmedWithChildSessions { child_sessions }) => {
            let lifecycle = LifecycleStore::new(run_context.app_paths.clone());
            for session in child_sessions {
                if let Err(error) =
                    ensure_workspace_context_message(&run_context.app_paths, &lifecycle, session)
                {
                    let err = WsOutMessage::Error {
                        message: format!("ensure child workspace context failed: {error}"),
                    };
                    let _ = send_json_outbound(&outbound_tx, &err).await;
                    return;
                }
            }
        }
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
