pub(crate) fn latest_review_verdict_from_messages(
    messages: &[SessionMessage],
    workspace_type: &WorkspaceType,
) -> Option<ReviewVerdict> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == "reviewer")
        .map(|message| {
            WorkspaceEngine::parse_review_verdict_for_workspace(&message.content, workspace_type)
        })
}

pub(crate) fn latest_review_verdict_from_node_details(
    lifecycle_store: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    session_id: &str,
    timeline_nodes: &[TimelineNode],
) -> Option<ReviewVerdict> {
    timeline_nodes
        .iter()
        .rev()
        .filter(|node| {
            matches!(
                node.node_type,
                TimelineNodeType::ReviewerRun
                    | TimelineNodeType::WorkItemPlanOutlineReview
                    | TimelineNodeType::WorkItemDraftReview
                    | TimelineNodeType::WorkItemBatchReview
            )
        })
        .find_map(|node| {
            let mut detail = lifecycle_store
                .load_node_detail_for_issue_session(project_id, issue_id, session_id, &node.node_id)
                .ok()?;
            let persisted_verdict = detail.verdict.clone()?;
            let verdict = match deserialize_historical_review_verdict(persisted_verdict.clone()) {
                Ok(verdict) => verdict,
                Err(error) => {
                    tracing::warn!(
                        node_id = %node.node_id,
                        %error,
                        "persisted review verdict failed normalization; entering user triage"
                    );
                    failed_historical_review_verdict(error.to_string())
                }
            };
            let normalized_verdict =
                serde_json::to_value(&verdict).expect("ReviewVerdict serialization must succeed");
            // P0-3：`finding_key` 是**投影**身份（按内容现算、不落 durable——
            // design §2.1/§4）。反序列化会按内容补 key，因此逐字比较必然不等；
            // 比较前两侧都剔除该键，读历史记录绝不因投影字段回写 durable。
            if verdict_without_finding_keys(&normalized_verdict)
                != verdict_without_finding_keys(&persisted_verdict)
            {
                detail.verdict = Some(normalized_verdict);
                if let Err(error) =
                    lifecycle_store.save_node_detail(session_id, &node.node_id, &detail)
                {
                    tracing::warn!(
                        node_id = %node.node_id,
                        %error,
                        "failed to persist normalized review verdict"
                    );
                }
            }
            Some(verdict)
        })
}

/// 剔除 findings 里的投影身份 `finding_key`（durable 字节比较用；不改变
/// 返回值语义——返回值仍带 key 供呈现/排除引用）。
fn verdict_without_finding_keys(value: &serde_json::Value) -> serde_json::Value {
    let mut value = value.clone();
    if let Some(findings) = value.get_mut("findings").and_then(|findings| findings.as_array_mut())
    {
        for finding in findings {
            if let Some(object) = finding.as_object_mut() {
                object.remove("finding_key");
            }
        }
    }
    value
}

/// 恢复/刷新路径的唯一派生入口：durable 节点详情 → 最新 review verdict，
/// 再按该会话 durable 已处置清单做**平台裁决**（REQ-WSC-14：owner 排除过的
/// 事实不得因重建而复活成 actionable）。
pub(crate) fn latest_review_verdict_for_session(
    lifecycle_store: &LifecycleStore,
    session: &WorkspaceSession,
    timeline_nodes: &[TimelineNode],
) -> Option<ReviewVerdict> {
    let verdict = latest_review_verdict_from_node_details(
        lifecycle_store,
        &session.project_id,
        &session.issue_id,
        &session.session_id,
        timeline_nodes,
    )
    .or_else(|| latest_review_verdict_from_messages(&session.messages, &session.workspace_type));
    verdict.map(|verdict| {
        crate::product::workspace_engine::review::normalize_review_verdict_with_store(
            lifecycle_store,
            &session.session_id,
            verdict,
        )
    })
}

fn failed_historical_review_verdict(error: String) -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "历史审核记录无法归一化，请人工确认。".to_string(),
        summary: "历史审核记录需要人工确认".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
        structured_output_diagnostic: Some(StructuredOutputDiagnostic {
            code: "historical_review_normalization_failed".to_string(),
            message: error,
            repair_attempted: false,
            repair_succeeded: false,
            raw_output_preview: None,
        }),
    }
}

pub(crate) fn review_complete_event_from_verdict(
    node_id: String,
    round: u32,
    verdict: &ReviewVerdict,
) -> EngineEvent {
    let findings = verdict.with_computed_finding_keys();
    EngineEvent::ReviewComplete {
        node_id,
        round,
        verdict: verdict.verdict.clone(),
        comments: verdict.comments.clone(),
        summary: verdict.summary.clone(),
        findings,
        review_gate: verdict.review_gate.clone(),
        work_item_plan_review: verdict.work_item_plan_review.clone(),
        structured_output_diagnostic: verdict.structured_output_diagnostic.clone(),
    }
}

