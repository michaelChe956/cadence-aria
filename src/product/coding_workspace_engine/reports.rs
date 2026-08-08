use super::*;
use crate::product::coding_models::{
    CodingAttemptScope, CodingExecutionUnit, CodingExecutionUnitStatus,
};
use crate::product::models::work_item_revision::HandoffRevision;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

impl CodingWorkspaceEngine {
    /// 已完成 unit 及其 `HandoffRevision`（若已发布）。
    ///
    /// 交接摘要移除后，跨 unit 交接语义完全由 `HandoffRevision` 承担。尚未发布
    /// 交接的 unit 以 `None` 表示，不再因缺少摘要而失败关闭。
    pub(crate) fn collect_completed_group_unit_handoff_revisions(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Vec<(CodingExecutionUnit, Option<HandoffRevision>)>, CodingWorkspaceEngineError>
    {
        let units =
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let lineage = self.schema_v2_group_plan_lineage(attempt)?;
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let mut handoffs = Vec::new();
        for unit in units
            .into_iter()
            .filter(|unit| unit.status == CodingExecutionUnitStatus::Completed)
        {
            let revision = match (lineage.as_ref(), unit.latest_handoff_revision_id.as_deref()) {
                (Some(lineage), Some(handoff_id)) => Some(revision_store.get_handoff_revision(
                    lineage,
                    &unit.logical_work_item_id,
                    handoff_id,
                )?),
                _ => None,
            };
            handoffs.push((unit, revision));
        }
        Ok(handoffs)
    }

    pub(crate) fn format_group_unit_handoff_section(
        &self,
        handoffs: &[(CodingExecutionUnit, Option<HandoffRevision>)],
    ) -> String {
        if handoffs.is_empty() {
            return "- 无 completed units".to_string();
        }

        handoffs
            .iter()
            .map(|(unit, revision)| {
                let completion_commit = unit
                    .completion_commit
                    .as_deref()
                    .or_else(|| revision.as_ref().map(|r| r.commit_sha.as_str()))
                    .unwrap_or("无");
                let handoff_revision_id = unit
                    .latest_handoff_revision_id
                    .as_deref()
                    .unwrap_or("未发布");
                let provided_contracts = revision
                    .as_ref()
                    .filter(|r| !r.provided_contracts.is_empty())
                    .map(|r| r.provided_contracts.join("; "))
                    .unwrap_or_else(|| "无".to_string());
                let provided_capabilities = revision
                    .as_ref()
                    .filter(|r| !r.provided_capabilities.is_empty())
                    .map(|r| {
                        r.provided_capabilities
                            .iter()
                            .map(|(contract, capabilities)| {
                                format!("{contract}=[{}]", capabilities.join(", "))
                            })
                            .collect::<Vec<_>>()
                            .join("; ")
                    })
                    .unwrap_or_else(|| "无".to_string());
                format!(
                    "- Unit: {}\n  Work Item: {}\n  Status: {:?}\n  Completion Commit: {}\n  Handoff Revision: {}\n  Provided Contracts: {}\n  Provided Capabilities: {}",
                    unit.id,
                    unit.logical_work_item_id,
                    unit.status,
                    completion_commit,
                    handoff_revision_id,
                    provided_contracts,
                    provided_capabilities
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

    pub(crate) fn group_final_review_evaluation_context_json(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<String, CodingWorkspaceEngineError> {
        let plan_id = attempt.work_item_group_id.as_deref().ok_or_else(|| {
            CodingWorkspaceEngineError::ProviderStream(
                "group_final_review_plan_binding_missing".to_string(),
            )
        })?;
        let units = self
            .authoritative_group_reviewer_bindings(attempt)?
            .into_iter()
            .map(|binding| {
                serde_json::json!({
                    "logical_work_item_id": binding.projection_binding.logical_work_item_id,
                    "work_item_revision_id": binding.run.work_item_revision_id,
                    "canonical_contract_hash": binding.run.canonical_contract_hash,
                    "projection_bundle_id": binding.run.projection_bundle_id,
                    "projection_compiler_version": binding.run.projection_compiler_version,
                    "reviewer_projection_hash": binding.run.reviewer_projection_hash,
                    "resolved_handoff_revision_ids": binding.run.resolved_handoff_revision_ids,
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_v2_group_final_review": true,
            "attempt_id": attempt.id,
            "plan_id": plan_id,
            "units": units,
        }))
        .map_err(|error| {
            CodingWorkspaceEngineError::ProviderStream(format!(
                "serialize_group_final_review_evaluation_context_failed: {error}"
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
        Ok(
            crate::product::coding_work_item_context::load_coding_work_item_context(
                &self.store.paths(),
                attempt,
            )?
            .markdown,
        )
    }

    pub(crate) fn build_code_review_report(
        &self,
        attempt: &CodingExecutionAttempt,
        outcome: ProviderStreamOutcome,
        raw_provider_output_ref: Option<String>,
        role_run: &CodingRoleRun,
    ) -> Result<CodeReviewReport, ProductStoreError> {
        let existing = self.store.list_code_review_reports(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let payload = parse_code_review_outcome(&outcome);
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
            unit_run_id: if attempt.scope == CodingAttemptScope::WorkItemGroup {
                Some(self.store.get_active_unit_run(attempt)?.id)
            } else {
                None
            },
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
        plan_defect_source: &str,
        plan_defect_route: &str,
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
                "plan_defect_source": plan_defect_source,
                "plan_defect_route": plan_defect_route,
            })),
            created_at: Utc::now().to_rfc3339(),
        };
        self.save_and_emit_chat_entry(attempt, entry).await;
    }

    pub(crate) async fn emit_internal_pr_review_chat_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        review: &InternalPrReview,
        plan_defect_route: &str,
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
                "plan_defect_route": plan_defect_route,
            })),
            created_at: Utc::now().to_rfc3339(),
        };
        self.save_and_emit_chat_entry(attempt, entry).await;
    }

    pub(crate) async fn save_and_emit_chat_entry(
        &self,
        attempt: &CodingExecutionAttempt,
        entry: CodingChatEntry,
    ) {
        let _ = self.store.save_chat_entry(attempt, &entry);
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingChatEntryCreated { entry })
            .await;
    }
}
