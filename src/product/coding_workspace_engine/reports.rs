use super::*;
use crate::product::coding_models::{CodingExecutionUnit, CodingExecutionUnitStatus};

impl CodingWorkspaceEngine {
    pub(crate) fn latest_missing_required_steps(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Vec<String>, ProductStoreError> {
        let Some(report) = self
            .store
            .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .last()
        else {
            return Ok(Vec::new());
        };
        let mut steps = Vec::new();
        for step in report
            .missing_required_steps
            .into_iter()
            .chain(report.skipped_required_steps)
        {
            if !steps.contains(&step) {
                steps.push(step);
            }
        }
        Ok(steps)
    }

    pub(crate) fn collect_completed_group_unit_handoffs(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Vec<(CodingExecutionUnit, WorkItemHandoff)>, CodingWorkspaceEngineError> {
        let units =
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let mut handoffs = Vec::new();
        for unit in units
            .into_iter()
            .filter(|unit| unit.status == CodingExecutionUnitStatus::Completed)
        {
            let handoff = self
                .store
                .get_coding_unit_handoff(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &unit.id,
                )?
                .ok_or_else(|| {
                    CodingWorkspaceEngineError::WorkItemHandoffMissing(format!(
                        "{}:{}",
                        attempt.id, unit.id
                    ))
                })?;
            handoffs.push((unit, handoff));
        }
        Ok(handoffs)
    }

    pub(crate) fn format_group_unit_handoff_section(
        &self,
        handoffs: &[(CodingExecutionUnit, WorkItemHandoff)],
    ) -> String {
        if handoffs.is_empty() {
            return "- 无 completed units".to_string();
        }

        handoffs
            .iter()
            .map(|(unit, handoff)| {
                let completion_commit = unit
                    .completion_commit
                    .as_deref()
                    .or(handoff.commit_sha.as_deref())
                    .unwrap_or("无");
                let tests_run = if handoff.tests_run.is_empty() {
                    "无".to_string()
                } else {
                    handoff.tests_run.join("; ")
                };
                let risk_notes = if handoff.open_risks.is_empty() {
                    "无".to_string()
                } else {
                    handoff.open_risks.join("; ")
                };
                format!(
                    "- Unit: {}\n  Work Item: {}\n  Status: {:?}\n  Completion Commit: {}\n  Handoff Summary: {}\n  Tests Run: {}\n  Risk Notes: {}",
                    unit.id,
                    unit.work_item_id,
                    unit.status,
                    completion_commit,
                    handoff.summary,
                    tests_run,
                    risk_notes
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

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
        let active_work_item_id = self.active_work_item_id_for_attempt(attempt);
        let Some(session) = sessions.iter().rev().find(|session| {
            session.entity_id == active_work_item_id
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
