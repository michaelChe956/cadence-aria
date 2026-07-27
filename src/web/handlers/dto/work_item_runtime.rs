use super::*;
use crate::product::coding_models::CodingExecutionUnit;
use crate::product::work_item_projection::HumanGroupWorkItemSummary;
use crate::product::work_item_runtime_reader::ResolvedWorkItemRuntime;

pub(crate) struct LifecycleWorkItemRuntimeDtoInput<'a> {
    pub repository_id: &'a str,
    pub plan_id: &'a str,
    pub runtime: &'a ResolvedWorkItemRuntime,
    pub human_projection: &'a HumanGroupWorkItemSummary,
    pub latest_attempt: Option<CodingAttemptDto>,
    pub unit: Option<&'a CodingExecutionUnit>,
    pub session_id: Option<&'a str>,
    pub require_execution_plan_confirm: bool,
}

pub(crate) fn lifecycle_work_item_runtime_dto(
    lifecycle: &LifecycleStore,
    input: LifecycleWorkItemRuntimeDtoInput<'_>,
) -> ApiResult<LifecycleWorkItemDto> {
    let artifact_versions = artifact_version_dtos(
        lifecycle,
        &input.runtime.lineage.project_id,
        &input.runtime.lineage.issue_id,
        input.session_id,
    )?;
    Ok(LifecycleWorkItemDto {
        work_item_id: input.human_projection.logical_work_item_id.clone(),
        issue_id: input.runtime.lineage.issue_id.clone(),
        repository_id: input.repository_id.to_string(),
        story_spec_ids: input.runtime.lineage.story_spec_refs.clone(),
        design_spec_ids: input.runtime.lineage.design_spec_refs.clone(),
        title: input.human_projection.title.clone(),
        plan_status: "confirmed".to_string(),
        execution_status: input
            .unit
            .map(|unit| coding_execution_unit_status_text(&unit.status).to_string())
            .unwrap_or_else(|| "pending".to_string()),
        latest_attempt: input.latest_attempt,
        artifact_versions,
        work_item_set_id: None,
        source_work_item_plan_id: Some(input.plan_id.to_string()),
        source_outline_id: None,
        source_draft_id: Some(
            input
                .runtime
                .work_item_revision
                .source_draft_revision_id
                .clone(),
        ),
        planned_implementation_context: None,
        planned_handoff_summary: None,
        kind: input
            .runtime
            .work_item_revision
            .canonical_contract
            .identity
            .kind
            .clone(),
        sequence_hint: None,
        depends_on: input.human_projection.depends_on.clone(),
        exclusive_write_scopes: input.human_projection.scope_summary.owned_scopes.clone(),
        forbidden_write_scopes: input
            .human_projection
            .scope_summary
            .forbidden_scopes
            .clone(),
        context_budget: WorkItemContextBudgetDto::default(),
        required_handoff_from: input.human_projection.depends_on.clone(),
        verification_plan_ref: Some(input.runtime.verification_plan_revision.id.clone()),
        require_execution_plan_confirm: input.require_execution_plan_confirm,
        execution_plan_status: "not_started".to_string(),
        handoff_summary_ref: input
            .unit
            .and_then(|unit| unit.latest_handoff_revision_id.clone()),
        completion_commit: input.unit.and_then(|unit| unit.completion_commit.clone()),
        completion_diff_summary_ref: None,
    })
}
