use super::*;

pub(crate) fn recover_work_item_plan_outline_review_schema_fallback(
    lifecycle_store: &LifecycleStore,
    session: &mut WorkspaceSession,
    timeline_nodes: &mut Vec<TimelineNode>,
    active_node_id: &mut Option<String>,
) -> Result<bool, String> {
    if session.workspace_type != WorkspaceType::WorkItemPlan {
        return Ok(false);
    }
    let Some(human_confirm_node_id) = active_node_id.clone() else {
        return Ok(false);
    };
    let Some(human_confirm_index) = timeline_nodes.iter().position(|node| {
        node.node_id == human_confirm_node_id
            && node.node_type == TimelineNodeType::HumanConfirm
            && node.status == TimelineNodeStatus::Active
    }) else {
        return Ok(false);
    };
    let Some(review_node_index) = timeline_nodes.iter().rposition(|node| {
        node.node_type == TimelineNodeType::WorkItemPlanOutlineReview
            && node.status == TimelineNodeStatus::Completed
    }) else {
        return Ok(false);
    };
    if review_node_index + 1 != human_confirm_index {
        return Ok(false);
    }
    let review_node_id = timeline_nodes[review_node_index].node_id.clone();
    let mut detail = lifecycle_store
        .load_node_detail_for_issue_session(
            &session.project_id,
            &session.issue_id,
            &session.session_id,
            &review_node_id,
        )
        .map_err(|error| {
            format!("load outline review detail for schema recovery failed: {error}")
        })?;
    let persisted_verdict = detail
        .verdict
        .clone()
        .and_then(|value| serde_json::from_value::<ReviewVerdict>(value).ok());
    let recoverable_diagnostic = persisted_verdict.as_ref().is_some_and(|verdict| {
        verdict
            .structured_output_diagnostic
            .as_ref()
            .is_some_and(|diagnostic| diagnostic.code == "invalid_outline_reference")
    });

    let recovered_verdict = if persisted_verdict
        .as_ref()
        .is_some_and(WorkspaceEngine::work_item_plan_optional_pass_review)
    {
        persisted_verdict.expect("checked persisted optional pass verdict")
    } else if recoverable_diagnostic {
        let Some(ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate }) =
            session.artifact.as_ref()
        else {
            return Ok(false);
        };
        let valid_outline_ids = outline_candidate
            .outline
            .work_item_outlines
            .iter()
            .map(|outline| outline.outline_id.clone())
            .collect::<Vec<_>>();
        let Some((comments, json)) = extract_structured_json(&detail.streaming_content) else {
            return Ok(false);
        };
        let value: serde_json::Value = serde_json::from_str(&json)
            .map_err(|error| format!("parse recovered outline review JSON failed: {error}"))?;
        let verdict = parse_work_item_plan_review_value(
            &value,
            &comments,
            &valid_outline_ids,
            WorkItemPlanReviewScope::Outline,
        )
        .map_err(|error| {
            format!(
                "validate recovered outline review JSON failed: {}",
                error.as_str()
            )
        })?;
        if !WorkspaceEngine::work_item_plan_optional_pass_review(&verdict) {
            return Ok(false);
        }
        verdict
    } else {
        return Ok(false);
    };

    let now = chrono::Utc::now().to_rfc3339();
    let mut recovered_timeline_nodes = timeline_nodes.clone();
    let round = recovered_timeline_nodes[review_node_index]
        .round
        .unwrap_or(1);
    let artifact_ref = recovered_timeline_nodes[review_node_index]
        .artifact_ref
        .clone();
    recovered_timeline_nodes[review_node_index].summary = Some(recovered_verdict.summary.clone());
    recovered_timeline_nodes[human_confirm_index].status = TimelineNodeStatus::Completed;
    recovered_timeline_nodes[human_confirm_index].summary =
        Some("结构化审核结果已恢复".to_string());
    recovered_timeline_nodes[human_confirm_index].completed_at = Some(now.clone());
    let review_decision_node_id =
        format!("timeline_node_{:03}", recovered_timeline_nodes.len() + 1);
    recovered_timeline_nodes.push(TimelineNode {
        node_id: review_decision_node_id.clone(),
        node_type: TimelineNodeType::ReviewDecision,
        agent: None,
        stage: WsWorkspaceStage::ReviewDecision,
        round: Some(round),
        status: TimelineNodeStatus::Paused,
        title: format!("Review Decision Round {round}"),
        summary: Some(recovered_verdict.summary.clone()),
        started_at: now,
        completed_at: None,
        duration_ms: None,
        artifact_ref,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: session.author_provider.clone(),
            reviewer: session.reviewer_provider.clone(),
            review_rounds: session.review_rounds,
            permission_modes: session.permission_modes.clone(),
        },
        retry: None,
    });

    let original_verdict = detail.verdict.clone();
    detail.verdict =
        Some(serde_json::to_value(&recovered_verdict).map_err(|error| {
            format!("serialize recovered outline review verdict failed: {error}")
        })?);
    lifecycle_store
        .save_node_detail(&session.session_id, &review_node_id, &detail)
        .map_err(|error| format!("save recovered outline review detail failed: {error}"))?;
    lifecycle_store
        .update_workspace_session_status(
            &session.session_id,
            WorkspaceSessionStatus::WaitingForHuman,
        )
        .map_err(|error| format!("save recovered outline review session status failed: {error}"))?;
    if let Err(error) =
        lifecycle_store.save_timeline_nodes(&session.session_id, &recovered_timeline_nodes)
    {
        detail.verdict = original_verdict;
        let rollback_error = lifecycle_store
            .save_node_detail(&session.session_id, &review_node_id, &detail)
            .err()
            .map(|rollback_error| format!("; rollback review verdict failed: {rollback_error}"))
            .unwrap_or_default();
        return Err(format!(
            "commit recovered outline review timeline failed: {error}{rollback_error}"
        ));
    }

    session.stage = WorkspaceStage::ReviewDecision;
    *timeline_nodes = recovered_timeline_nodes;
    *active_node_id = Some(review_decision_node_id);
    Ok(true)
}
