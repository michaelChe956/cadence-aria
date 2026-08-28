use std::collections::BTreeMap;

use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::{
    WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftStatus, WorkItemGenerationMode,
    WorkItemOutline, WorkItemOutlineSessionFit, WorkItemPlanDraftActiveIndex, WorkItemPlanOutline,
};
use crate::product::work_item_plan_compiler::{
    PlanCandidateIr, PlanCandidateMechanicalReport, PlanCandidatePublicationProvenance,
};
use crate::web::workspace_ws_types::WorkItemPlanOutlineCandidateDto;

use super::{InitialPlanCompileDurableContext, InitialPlanCompileInput};
use crate::product::models::IssueWorkItemPlan;

/// IR adapter 的所有动态值均由 durable Approval/reservation 注入；不能从 engine 内存恢复。
#[derive(Debug, Clone)]
pub(crate) struct IrCompileAdapterContext {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub previous_plan: IssueWorkItemPlan,
    pub source_revision_id: String,
    pub source_revision_ref: String,
    pub plan_candidate_ir_ref: String,
    pub mechanical_report_ref: String,
    pub publication_provenance_ref: String,
    pub logical_targets: Option<BTreeMap<LogicalRepositoryId, String>>,
    pub repository_id: String,
    pub change_order: Vec<LogicalRepositoryId>,
    pub compile_id: String,
    pub now: String,
}

/// 将已经过 source-store/freshness 验证的 immutable IR 机械投影为旧 compile 核心的输入。
/// `InitialPlanCompileInput` 的 13 个字段不携带 durable refs，避免纯 prepare 发生 store read。
pub(crate) fn initial_plan_compile_input_from_ir(
    context: &IrCompileAdapterContext,
    ir: &PlanCandidateIr,
    mechanical_report: &PlanCandidateMechanicalReport,
) -> Result<InitialPlanCompileInput, String> {
    if context.project_id != context.previous_plan.project_id
        || context.issue_id != context.previous_plan.issue_id
        || context.plan_id != context.previous_plan.id
    {
        return Err("IR compile adapter context identity mismatch".to_string());
    }
    if ir.items.is_empty() {
        return Err("IR compile adapter requires at least one candidate item".to_string());
    }
    if mechanical_report.has_errors()
        || mechanical_report.source_revision_hash != ir.source_revision_hash
        || mechanical_report.compiler_version != ir.compiler_version
    {
        return Err("IR compile adapter mechanical report is not publishable".to_string());
    }

    let mut outline_to_current_draft_id = BTreeMap::new();
    let mut draft_statuses = BTreeMap::new();
    let mut work_item_outlines = Vec::with_capacity(ir.items.len());
    let mut draft_records = Vec::with_capacity(ir.items.len());
    for (index, item) in ir.items.iter().enumerate() {
        let logical_id = item.contract.identity.logical_work_item_id.clone();
        let outline_id = logical_id.clone();
        let target_repository_id = logical_target_for_ir_item(context, &item.target_repository_id)?;
        let draft_id = format!("draft_{}_{}", context.compile_id, index + 1);
        if outline_to_current_draft_id
            .insert(outline_id.clone(), draft_id.clone())
            .is_some()
        {
            return Err(format!(
                "IR contains duplicate logical work item `{logical_id}`"
            ));
        }
        draft_statuses.insert(draft_id.clone(), WorkItemDraftStatus::Accepted);
        work_item_outlines.push(WorkItemOutline {
            target_repository_id,
            outline_id: outline_id.clone(),
            logical_work_item_id: logical_id.clone(),
            title: item.contract.identity.title.clone(),
            kind: parse_work_item_kind(&item.contract.identity.kind),
            goal: item.contract.goal.summary.clone(),
            scope: item.contract.write_policy.exclusive_scopes.clone(),
            non_goals: item.contract.non_goals.clone(),
            estimated_context_tokens: Some(30_000),
            session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
            source_story_spec_ids: context.previous_plan.source_story_spec_ids.clone(),
            source_design_spec_ids: context.previous_plan.source_design_spec_ids.clone(),
            exclusive_write_scopes: item.contract.write_policy.exclusive_scopes.clone(),
            forbidden_write_scopes: item.contract.write_policy.forbidden_scopes.clone(),
            depends_on: item.contract.depends_on.clone(),
            verification_intent: item
                .verification_plan
                .checks
                .iter()
                .filter_map(|check| {
                    check
                        .command
                        .as_deref()
                        .or(check.manual_instruction.as_deref())
                        .map(str::to_string)
                })
                .collect(),
            trusted_verification_commands: item.trusted_commands.clone(),
            handoff_notes: item.contract.handoff_contract.required_fields.join(", "),
        });
        draft_records.push(WorkItemDraftRecord {
            project_id: context.project_id.clone(),
            issue_id: context.issue_id.clone(),
            plan_id: context.plan_id.clone(),
            draft_id,
            outline_id,
            generation_round_id: format!("single_candidate_{}", context.compile_id),
            batch_id: None,
            attempt_index: 1,
            outline_version_ref: context.compile_id.clone(),
            generation_mode: WorkItemGenerationMode::Serial,
            generation_diagnostics: None,
            candidate: WorkItemDraftCandidate {
                target_repository_id,
                outline_id: logical_id.clone(),
                logical_work_item_id: logical_id,
                canonical_contract_candidate: item.contract.clone(),
                verification_plan: item.verification_plan.clone(),
            },
            status: WorkItemDraftStatus::Accepted,
            active: true,
            superseded_by_draft_id: None,
            supersede_reason: None,
            copied_from_draft_id: None,
            review_node_id: None,
            review_verdict_ref: None,
            generated_from_node_id: context.compile_id.clone(),
            accepted_at: Some(context.now.clone()),
            superseded_at: None,
            created_at: context.now.clone(),
            updated_at: context.now.clone(),
        });
    }

    let mut outline = WorkItemPlanOutline {
        id: context.compile_id.clone(),
        project_id: context.project_id.clone(),
        issue_id: context.issue_id.clone(),
        source_story_spec_ids: context.previous_plan.source_story_spec_ids.clone(),
        source_design_spec_ids: context.previous_plan.source_design_spec_ids.clone(),
        strategy_summary: "由已验证 PlanCandidateIr 确定性投影".to_string(),
        work_item_outlines,
        dependency_graph: Vec::new(),
        risks: Vec::new(),
        handoff_strategy: "由 canonical handoff schema 投影".to_string(),
        status: "accepted".to_string(),
    };
    outline.normalize_dependency_graph_from_depends_on();
    let outline_order = outline
        .work_item_outlines
        .iter()
        .map(|item| item.outline_id.clone())
        .collect::<Vec<_>>();
    let active_index = WorkItemPlanDraftActiveIndex {
        project_id: context.project_id.clone(),
        issue_id: context.issue_id.clone(),
        plan_id: context.plan_id.clone(),
        current_generation_round_id: format!("single_candidate_{}", context.compile_id),
        outline_state: "accepted".to_string(),
        active_outline_id: None,
        outline_to_current_draft_id,
        draft_statuses,
        batches: Vec::new(),
        updated_at: context.now.clone(),
    };
    Ok(InitialPlanCompileInput {
        project_id: context.project_id.clone(),
        issue_id: context.issue_id.clone(),
        plan_id: context.plan_id.clone(),
        previous_plan: context.previous_plan.clone(),
        active_index,
        outline_candidate: WorkItemPlanOutlineCandidateDto {
            outline,
            design_context_gaps: Vec::new(),
            validator_findings: Vec::new(),
            context_blockers: Vec::new(),
            current_generation_round_id: None,
            selected_generation_mode: None,
        },
        outline_order,
        draft_records,
        logical_targets: context.logical_targets.clone(),
        repository_id: context.repository_id.clone(),
        change_order: context.change_order.clone(),
        compile_id: context.compile_id.clone(),
        now: context.now.clone(),
    })
}

pub(crate) fn durable_compile_context_from_ir(
    context: &IrCompileAdapterContext,
    provenance: &PlanCandidatePublicationProvenance,
) -> Result<InitialPlanCompileDurableContext, String> {
    if provenance.plan_id != context.plan_id
        || provenance.source_revision_ref != context.source_revision_ref
        || provenance.plan_candidate_ir_ref != context.plan_candidate_ir_ref
        || provenance.mechanical_report_ref != context.mechanical_report_ref
        || provenance.id != context.compile_id
        || provenance.published_at != context.now
        || provenance.content_hash.is_empty()
    {
        return Err("IR compile durable provenance does not match reservation context".to_string());
    }
    Ok(InitialPlanCompileDurableContext {
        flow_kind: Some(
            crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
        ),
        source_revision_id: Some(context.source_revision_id.clone()),
        source_revision_ref: Some(context.source_revision_ref.clone()),
        plan_candidate_ir_ref: Some(context.plan_candidate_ir_ref.clone()),
        mechanical_report_ref: Some(context.mechanical_report_ref.clone()),
        publication_provenance_ref: Some(context.publication_provenance_ref.clone()),
        publication_provenance_content_hash: Some(provenance.content_hash.clone()),
    })
}

fn logical_target_for_ir_item(
    context: &IrCompileAdapterContext,
    physical_repository_id: &str,
) -> Result<Option<LogicalRepositoryId>, String> {
    let Some(logical_targets) = &context.logical_targets else {
        return Ok(None);
    };
    let matches = logical_targets
        .iter()
        .filter_map(|(logical_id, physical_id)| {
            (physical_id == physical_repository_id).then_some(*logical_id)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [logical_id] => Ok(Some(*logical_id)),
        [] => Err(format!(
            "work_item_target_missing: IR target `{physical_repository_id}` is not effective"
        )),
        _ => Err(format!(
            "IR target `{physical_repository_id}` maps to multiple logical repositories"
        )),
    }
}

fn parse_work_item_kind(value: &str) -> crate::product::models::WorkItemKind {
    crate::product::work_item_split_engine::types::parse_work_item_kind(value)
}
