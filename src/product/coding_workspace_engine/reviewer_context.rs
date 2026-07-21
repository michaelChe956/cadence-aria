use super::*;
use crate::product::work_item_projection::{ReviewerExecutionEnvelope, renderer_for};

pub(crate) struct PreparedGroupReviewerContext {
    pub(crate) bindings: Vec<GroupReviewerProjectionBinding>,
    pub(crate) prompt_section: String,
}

impl CodingWorkspaceEngine {
    pub(crate) fn prepare_group_reviewer_context(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &ProviderName,
    ) -> Result<PreparedGroupReviewerContext, CodingWorkspaceEngineError> {
        let authoritative_bindings = self.authoritative_group_reviewer_bindings(attempt)?;
        let mut test_evidence_refs = self
            .store
            .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .into_iter()
            .map(|report| report.id)
            .collect::<Vec<_>>();
        test_evidence_refs.sort();
        test_evidence_refs.dedup();

        let renderer = renderer_for(provider);
        let mut sections = Vec::with_capacity(authoritative_bindings.len());
        for binding in &authoritative_bindings {
            let completion_commit = binding.run.completion_commit.clone().ok_or_else(|| {
                CodingWorkspaceEngineError::CompletionCommitMissing(binding.run.id.clone())
            })?;
            let diff_base = binding
                .run
                .start_commit
                .clone()
                .unwrap_or_else(|| attempt.base_branch.clone());
            let rendered = renderer
                .render_reviewer(
                    &binding.projection_binding.projection,
                    &ReviewerExecutionEnvelope {
                        unit_run_id: binding.run.id.clone(),
                        diff_ref: format!("{diff_base}..{completion_commit}"),
                        test_evidence_refs: test_evidence_refs.clone(),
                        handoff_revision_ids: binding.run.resolved_handoff_revision_ids.clone(),
                        contract_delta_refs: Vec::new(),
                        completion_commit,
                    },
                )
                .map_err(|error| {
                    CodingWorkspaceEngineError::ProviderStream(format!(
                        "group_reviewer_projection_render_failed: {error}"
                    ))
                })?;
            self.store.bind_unit_run_execution_context(
                attempt,
                &binding.run.id,
                CodingProviderRole::InternalReviewer,
                &rendered,
            )?;
            sections.push(format!(
                "### Work Item {} / UnitRun {}\n{}",
                binding.projection_binding.logical_work_item_id, binding.run.id, rendered.text
            ));
        }

        Ok(PreparedGroupReviewerContext {
            bindings: authoritative_bindings
                .into_iter()
                .map(|binding| binding.projection_binding)
                .collect(),
            prompt_section: format!(
                "Authoritative Group Reviewer Execution Contexts:\n\n{}",
                sections.join("\n\n")
            ),
        })
    }
}
