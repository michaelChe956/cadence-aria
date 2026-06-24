use super::*;

impl CodingWorkspaceEngine {
    pub(crate) fn evaluation_context_json_for_role(
        &self,
        attempt: &CodingExecutionAttempt,
        provider_role: EvaluationContextRole,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let context = build_evaluation_context_pack(self.store.paths(), attempt, provider_role)?;
        serde_json::to_string_pretty(&context).map_err(|error| {
            CodingWorkspaceEngineError::ProviderStream(format!(
                "serialize_evaluation_context_failed: {error}"
            ))
        })
    }

    pub(crate) fn retry_diagnostic_for_previous_run(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: &CodingRoleRun,
    ) -> Result<Option<String>, CodingWorkspaceEngineError> {
        let Some(previous_run_id) = role_run.supersedes_run_id.as_deref() else {
            return Ok(None);
        };
        self.store
            .role_run_retry_diagnostic_summary(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                previous_run_id,
            )
            .map_err(CodingWorkspaceEngineError::Store)
    }

    pub(crate) fn work_item_markdown_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Option<String>, ProductStoreError> {
        let lifecycle = LifecycleStore::new(self.store.paths());
        let sessions = lifecycle.list_workspace_sessions(&attempt.project_id, &attempt.issue_id)?;
        let Some(session) = sessions.iter().rev().find(|session| {
            session.entity_id == attempt.work_item_id
                && session.workspace_type == WorkspaceType::WorkItem
        }) else {
            return Ok(None);
        };
        Ok(lifecycle
            .list_artifact_versions(&session.id)?
            .into_iter()
            .last()
            .map(|version| version.to_markdown_string()))
    }

    pub(crate) fn build_code_review_report(
        &self,
        attempt: &CodingExecutionAttempt,
        full_output: &str,
        raw_provider_output_ref: Option<String>,
        role_run: &CodingRoleRun,
    ) -> Result<CodeReviewReport, ProductStoreError> {
        let existing = self.store.list_code_review_reports(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let payload = parse_review_payload(full_output, CodingExecutionStage::CodeReview);
        Ok(CodeReviewReport {
            id: next_sequential_id("code_review", existing.len()),
            attempt_id: attempt.id.clone(),
            round: existing.len() as u32 + 1,
            verdict: payload.verdict,
            findings: payload.findings,
            tested_evidence_refs: payload.tested_evidence_refs,
            diff_refs: payload.diff_refs,
            summary: payload.summary,
            created_at: Utc::now().to_rfc3339(),
            raw_provider_output_ref,
            role_run_id: Some(role_run.id.clone()),
            run_no: Some(role_run.run_no),
        })
    }

    pub(crate) fn build_internal_pr_review(
        &self,
        attempt: &CodingExecutionAttempt,
        review_request: &ReviewRequest,
        full_output: &str,
        raw_provider_output_ref: Option<String>,
        role_run: &CodingRoleRun,
    ) -> Result<InternalPrReview, ProductStoreError> {
        let existing = self.store.list_internal_pr_reviews(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let payload = parse_review_payload(full_output, CodingExecutionStage::InternalPrReview);
        Ok(InternalPrReview {
            id: next_sequential_id("internal_review", existing.len()),
            attempt_id: attempt.id.clone(),
            review_request_id: review_request.id.clone(),
            verdict: payload.verdict,
            findings: payload.findings,
            impact_scope: payload.impact_scope,
            pr_description: payload.pr_description,
            commit_message_suggestion: payload.commit_message_suggestion,
            tested_evidence_refs: payload.tested_evidence_refs,
            diff_refs: payload.diff_refs,
            summary: payload.summary,
            created_at: Utc::now().to_rfc3339(),
            raw_provider_output_ref,
            role_run_id: Some(role_run.id.clone()),
            run_no: Some(role_run.run_no),
        })
    }

    pub(crate) async fn emit_code_review_chat_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        report: &CodeReviewReport,
    ) {
        let entry = CodingChatEntry {
            id: format!("{node_id}_code_review_report"),
            attempt_id: attempt.id.clone(),
            node_id: Some(node_id.to_string()),
            role: CodingAgentRole::Reviewer,
            entry_type: CodingEntryType::AssistantMessage,
            content: Some(report.summary.clone()),
            metadata: Some(serde_json::json!({
                "source": "code_review",
                "review_id": &report.id,
                "verdict": &report.verdict,
                "findings_count": report.findings.len(),
                "role_run_id": report.role_run_id,
                "run_no": report.run_no,
            })),
            created_at: Utc::now().to_rfc3339(),
        };
        self.save_and_emit_chat_entry(entry).await;
    }

    pub(crate) async fn emit_internal_pr_review_chat_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        review: &InternalPrReview,
    ) {
        let entry = CodingChatEntry {
            id: format!("{node_id}_internal_pr_review"),
            attempt_id: attempt.id.clone(),
            node_id: Some(node_id.to_string()),
            role: CodingAgentRole::Reviewer,
            entry_type: CodingEntryType::AssistantMessage,
            content: Some(review.summary.clone()),
            metadata: Some(serde_json::json!({
                "source": "internal_pr_review",
                "review_id": &review.id,
                "review_request_id": &review.review_request_id,
                "verdict": &review.verdict,
                "impact_scope": &review.impact_scope,
                "role_run_id": review.role_run_id,
                "run_no": review.run_no,
            })),
            created_at: Utc::now().to_rfc3339(),
        };
        self.save_and_emit_chat_entry(entry).await;
    }

    pub(crate) async fn save_and_emit_chat_entry(&self, entry: CodingChatEntry) {
        let _ = self.store.save_chat_entry(&entry);
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingChatEntryCreated { entry })
            .await;
    }

    pub(crate) async fn emit_analyst_verdict_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        rework_round: u32,
        source_stage: &CodingExecutionStage,
        decision: &AnalystDecision,
        role_run: &CodingRoleRun,
    ) {
        let mut metadata = serde_json::json!({
            "source": "analyst",
            "source_stage": source_stage,
            "rework_round": rework_round,
            "role_run_id": role_run.id,
            "run_no": role_run.run_no,
        });
        if let Some(object) = metadata.as_object_mut() {
            object.insert(
                "structured_verdict".to_string(),
                serde_json::json!(&decision.structured_verdict),
            );
            object.insert(
                "next_stage".to_string(),
                serde_json::json!(decision.next_stage.clone().unwrap_or_else(|| {
                    default_next_stage_for_legacy_verdict(
                        &decision.structured_verdict,
                        source_stage,
                    )
                })),
            );
            object.insert("reason".to_string(), serde_json::json!(&decision.reason));
            object.insert(
                "evidence_refs".to_string(),
                serde_json::json!(&decision.evidence_refs),
            );
            object.insert(
                "raw_provider_output_refs".to_string(),
                serde_json::json!(&decision.raw_provider_output_refs),
            );
            if let Some(instructions) = decision.rework_instructions.as_ref() {
                object.insert(
                    "rework_instructions".to_string(),
                    serde_json::json!(instructions),
                );
            }
            if let Some(human_gate) = decision.human_gate.as_ref() {
                object.insert("human_gate".to_string(), serde_json::json!(human_gate));
            }
            if !decision.fix_hints.is_empty() {
                object.insert(
                    "fix_hints".to_string(),
                    serde_json::json!(&decision.fix_hints),
                );
            }
            if !decision.questions.is_empty() {
                object.insert(
                    "questions".to_string(),
                    serde_json::json!(&decision.questions),
                );
            }
            if let Some(parse_error) = decision.parse_error.as_ref() {
                object.insert("parse_error".to_string(), serde_json::json!(parse_error));
            }
        }
        let entry = CodingChatEntry {
            id: format!("{node_id}_analyst_verdict"),
            attempt_id: attempt.id.clone(),
            node_id: Some(node_id.to_string()),
            role: CodingAgentRole::System,
            entry_type: CodingEntryType::AnalystVerdict {
                verdict: decision.verdict.clone(),
            },
            content: Some(decision.summary.clone()),
            metadata: Some(metadata),
            created_at: Utc::now().to_rfc3339(),
        };
        self.save_and_emit_chat_entry(entry).await;
    }

    pub(crate) async fn emit_rewrite_limit_warning_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        rework_round: u32,
        decision: &AnalystDecision,
    ) {
        let message = "已达到自动重写上限，跳过 Coding 并进入 CodeReview。".to_string();
        let entry = CodingChatEntry {
            id: format!("{node_id}_rewrite_limit_warning"),
            attempt_id: attempt.id.clone(),
            node_id: Some(node_id.to_string()),
            role: CodingAgentRole::System,
            entry_type: CodingEntryType::SystemEvent {
                event_type: "exceeded_rewrite_limit".to_string(),
                message: message.clone(),
            },
            content: Some(message),
            metadata: Some(serde_json::json!({
                "source": "analyst",
                "rework_round": rework_round,
                "rework_count": attempt.rework_count,
                "max_auto_rework": attempt.max_auto_rework,
                "analyst_summary": &decision.summary,
            })),
            created_at: Utc::now().to_rfc3339(),
        };
        self.save_and_emit_chat_entry(entry).await;
    }

    pub(crate) async fn apply_analyst_decision(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        source_stage: &CodingExecutionStage,
        rework_round: u32,
        decision: &AnalystDecision,
    ) -> Result<
        (CodingExecutionAttempt, CodingTimelineNodeStatus, String),
        CodingWorkspaceEngineError,
    > {
        let next_stage = decision.next_stage.clone().unwrap_or_else(|| {
            default_next_stage_for_legacy_verdict(&decision.structured_verdict, source_stage)
        });

        match next_stage {
            AnalystDecisionNextStage::Coding => {
                if attempt.rework_count < attempt.max_auto_rework {
                    let existing = self.store.list_rework_instructions(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                    )?;
                    let instruction_summary = decision
                        .rework_instructions
                        .as_ref()
                        .map(|instruction| instruction.summary.clone())
                        .unwrap_or_else(|| decision.summary.clone());
                    let instruction_fix_hints = decision
                        .rework_instructions
                        .as_ref()
                        .map(|instruction| {
                            instruction
                                .required_changes
                                .iter()
                                .chain(instruction.verification_expectations.iter())
                                .cloned()
                                .collect::<Vec<_>>()
                        })
                        .filter(|items| !items.is_empty())
                        .unwrap_or_else(|| decision.fix_hints.clone());
                    let instruction = CodingReworkInstruction {
                        id: next_sequential_id("coding_rework_instruction", existing.len()),
                        attempt_id: attempt.id.clone(),
                        source_stage: source_stage.clone(),
                        rework_round,
                        summary: instruction_summary,
                        fix_hints: instruction_fix_hints,
                        questions: decision.questions.clone(),
                        created_at: Utc::now().to_rfc3339(),
                        consumed_by_node_id: None,
                        consumed_at: None,
                    };
                    self.store.save_rework_instruction(&instruction)?;
                    let updated = self.store.increment_attempt_rework_count(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                    )?;
                    let updated = self.store.update_attempt_stage(
                        &updated.project_id,
                        &updated.issue_id,
                        &updated.id,
                        CodingExecutionStage::Coding,
                    )?;
                    Ok((
                        updated,
                        CodingTimelineNodeStatus::Completed,
                        format!("NeedsFix: {}", decision.summary),
                    ))
                } else {
                    self.emit_rewrite_limit_warning_entry(attempt, node_id, rework_round, decision)
                        .await;
                    let updated = self.store.update_attempt_status(
                        &attempt.project_id,
                        &attempt.issue_id,
                        &attempt.id,
                        CodingAttemptStatus::Blocked,
                    )?;
                    let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
                        attempt_id: attempt.id.clone(),
                        stage: CodingExecutionStage::Rework,
                        node_id: Some(node_id.to_string()),
                        role: Some(CodingProviderRole::Analyst),
                        title: "Rework limit reached".to_string(),
                        description: format!("{}；已达到自动重写上限", decision.summary),
                        reason_code: Some("max_auto_rework_exceeded".to_string()),
                        evidence_refs: decision.evidence_refs.clone(),
                        raw_provider_output_ref: decision.raw_provider_output_refs.first().cloned(),
                        available_actions: vec![
                            coding_gate_action_for_id("continue_rework")
                                .expect("continue rework action"),
                            coding_gate_action_for_id("provide_context")
                                .expect("provide context action"),
                            coding_gate_action_for_id("manual_continue")
                                .expect("manual continue action"),
                            coding_gate_action_for_id("abort").expect("abort action"),
                        ],
                    })?;
                    let _ = self
                        .event_tx
                        .send(CodingWsOutMessage::CodingGateRequired { gate })
                        .await;
                    match self
                        .ensure_issue_shared_worktree_clean(
                            &attempt.project_id,
                            &attempt.issue_id,
                            &attempt.id,
                            &attempt.work_item_id,
                        )
                        .await
                    {
                        Err(
                            error @ CodingWorkspaceEngineError::SharedWorktreeDirtyManualGate(_),
                        ) => {
                            let _ = error;
                        }
                        Err(error) => return Err(error),
                        Ok(()) => {
                            self.release_issue_shared_worktree_lock_if_holder(
                                &attempt.project_id,
                                &attempt.issue_id,
                                &attempt.work_item_id,
                            )?;
                        }
                    }
                    Ok((
                        updated,
                        CodingTimelineNodeStatus::Blocked,
                        format!("NeedsFix: {}；已达到自动重写上限", decision.summary),
                    ))
                }
            }
            AnalystDecisionNextStage::Testing => {
                let updated = self.store.update_attempt_stage(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingExecutionStage::Testing,
                )?;
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Completed,
                    format!("RerunTesting: {}", decision.summary),
                ))
            }
            AnalystDecisionNextStage::CodeReview => {
                let updated = self.store.update_attempt_stage(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingExecutionStage::CodeReview,
                )?;
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Completed,
                    format!("NextStage CodeReview: {}", decision.summary),
                ))
            }
            AnalystDecisionNextStage::ReviewRequest => {
                let updated = self.store.update_attempt_stage(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingExecutionStage::ReviewRequest,
                )?;
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Completed,
                    format!("NextStage ReviewRequest: {}", decision.summary),
                ))
            }
            AnalystDecisionNextStage::InternalPrReview => {
                let updated = self.store.update_attempt_stage(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingExecutionStage::InternalPrReview,
                )?;
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Completed,
                    format!("NextStage InternalPrReview: {}", decision.summary),
                ))
            }
            AnalystDecisionNextStage::FinalConfirm => {
                let updated = self.complete_attempt_after_final_rework(attempt).await?;
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Completed,
                    format!("NextStage FinalConfirm: {}", decision.summary),
                ))
            }
            AnalystDecisionNextStage::HumanGate => {
                let updated = self.store.update_attempt_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingAttemptStatus::Blocked,
                )?;
                let reason_code = decision
                    .human_gate
                    .as_ref()
                    .and_then(|gate| gate.reason_code.clone())
                    .unwrap_or_else(|| "analyst_human_gate".to_string());
                let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
                    attempt_id: attempt.id.clone(),
                    stage: CodingExecutionStage::Rework,
                    node_id: Some(node_id.to_string()),
                    role: Some(CodingProviderRole::Analyst),
                    title: "Analyst human gate".to_string(),
                    description: decision.reason.clone(),
                    reason_code: Some(reason_code),
                    evidence_refs: decision.evidence_refs.clone(),
                    raw_provider_output_ref: decision.raw_provider_output_refs.first().cloned(),
                    available_actions: analyst_human_gate_actions(decision.human_gate.as_ref()),
                })?;
                let _ = self
                    .event_tx
                    .send(CodingWsOutMessage::CodingGateRequired { gate })
                    .await;
                Ok((
                    updated,
                    CodingTimelineNodeStatus::Blocked,
                    format!("HumanGate: {}", decision.summary),
                ))
            }
        }
    }

    pub(crate) async fn emit_tester_tool_result_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        sequence: &mut usize,
        role_run: Option<&CodingRoleRun>,
        result: ProviderToolResult,
    ) {
        let metadata = role_run.map(|role_run| {
            serde_json::json!({
                "tool_use_id": result.tool_use_id.clone(),
                "role_run_id": role_run.id.clone(),
                "run_no": role_run.run_no
            })
        });
        let entry = tester_chat_entry(
            attempt,
            node_id,
            sequence,
            CodingEntryType::ToolResult {
                tool_use_id: result.tool_use_id,
                output: result.output,
                is_error: result.is_error,
            },
            None,
            metadata,
        );
        self.save_and_emit_chat_entry(entry).await;
    }
}
