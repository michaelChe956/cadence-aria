mod claude_code;
mod codex;
mod fake;

use std::collections::BTreeSet;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::product::coding_workspace_engine::reviewer_process_evidence_boundary_contract;
use crate::product::models::ProviderName;
use crate::product::plan_repair::plan_defect_structured_output_contract;

use super::{
    CoderExecutionEnvelope, CoderWorkItemProjection, RenderedExecutionContext,
    ReviewerExecutionEnvelope, ReviewerWorkItemProjection,
};

use claude_code::ClaudeCodeProjectionRenderer;
use codex::CodexProjectionRenderer;
use fake::FakeProjectionRenderer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProjectionRenderRole {
    Coder,
    Reviewer,
}

impl ProjectionRenderRole {
    fn label(self) -> &'static str {
        match self {
            Self::Coder => "Coder",
            Self::Reviewer => "Reviewer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ProjectionSectionId {
    WorkItemIdentityRevision,
    Objective,
    ResolvedInputs,
    ImplementationTasks,
    WritePolicy,
    AcceptanceCriteria,
    VerificationChecks,
    BlockerRouting,
    HandoffRequirements,
    ExecutionEnvelope,
    PreviousReview,
    AcceptanceCriteriaRequirementMatrix,
    ScopePolicy,
    InputContractChecks,
    OutputContractChecks,
    VerificationEvidenceRules,
    ReviewExecutionEvidence,
    ReviewerProcessEvidenceBoundary,
}

const CODER_MANDATORY_SECTIONS: &[ProjectionSectionId] = &[
    ProjectionSectionId::WorkItemIdentityRevision,
    ProjectionSectionId::Objective,
    ProjectionSectionId::ResolvedInputs,
    ProjectionSectionId::ImplementationTasks,
    ProjectionSectionId::WritePolicy,
    ProjectionSectionId::AcceptanceCriteria,
    ProjectionSectionId::VerificationChecks,
    ProjectionSectionId::BlockerRouting,
    ProjectionSectionId::HandoffRequirements,
    ProjectionSectionId::ExecutionEnvelope,
    ProjectionSectionId::PreviousReview,
];

const REVIEWER_MANDATORY_SECTIONS: &[ProjectionSectionId] = &[
    ProjectionSectionId::WorkItemIdentityRevision,
    ProjectionSectionId::AcceptanceCriteriaRequirementMatrix,
    ProjectionSectionId::ScopePolicy,
    ProjectionSectionId::InputContractChecks,
    ProjectionSectionId::OutputContractChecks,
    ProjectionSectionId::VerificationEvidenceRules,
    ProjectionSectionId::BlockerRouting,
    ProjectionSectionId::ReviewExecutionEvidence,
    ProjectionSectionId::ReviewerProcessEvidenceBoundary,
];

impl ProjectionSectionId {
    fn mandatory_for(role: ProjectionRenderRole) -> &'static [Self] {
        match role {
            ProjectionRenderRole::Coder => CODER_MANDATORY_SECTIONS,
            ProjectionRenderRole::Reviewer => REVIEWER_MANDATORY_SECTIONS,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::WorkItemIdentityRevision => "Work Item Identity/Revision",
            Self::Objective => "Objective",
            Self::ResolvedInputs => "Resolved Inputs",
            Self::ImplementationTasks => "Implementation Tasks",
            Self::WritePolicy => "Write Policy",
            Self::AcceptanceCriteria => "Acceptance Criteria",
            Self::VerificationChecks => "Verification Checks",
            Self::BlockerRouting => "Blocker Routing",
            Self::HandoffRequirements => "Handoff Requirements",
            Self::ExecutionEnvelope => "Execution Envelope",
            Self::PreviousReview => "Previous Review",
            Self::AcceptanceCriteriaRequirementMatrix => "Acceptance Criteria / Requirement Matrix",
            Self::ScopePolicy => "Scope Policy",
            Self::InputContractChecks => "Input Contract Checks",
            Self::OutputContractChecks => "Output Contract Checks",
            Self::VerificationEvidenceRules => "Verification Evidence Rules",
            Self::ReviewExecutionEvidence => "Review Execution Evidence",
            Self::ReviewerProcessEvidenceBoundary => "Reviewer Process Evidence Boundary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectionSection {
    id: ProjectionSectionId,
    title: String,
    body: String,
}

impl ProjectionSection {
    fn new(id: ProjectionSectionId, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            body: body.into(),
        }
    }

    fn typed(id: ProjectionSectionId, body: String) -> Self {
        Self::new(id, id.title(), body)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionRenderError {
    MandatorySectionMissing(String),
    Serialization(String),
}

impl std::fmt::Display for ProjectionRenderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MandatorySectionMissing(section) => {
                write!(formatter, "mandatory projection section missing: {section}")
            }
            Self::Serialization(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ProjectionRenderError {}

pub trait ProviderProjectionRenderer: Send + Sync {
    fn renderer_version(&self) -> &'static str;

    fn render_coder(
        &self,
        projection: &CoderWorkItemProjection,
        envelope: &CoderExecutionEnvelope,
    ) -> Result<RenderedExecutionContext, ProjectionRenderError>;

    fn render_reviewer(
        &self,
        projection: &ReviewerWorkItemProjection,
        envelope: &ReviewerExecutionEnvelope,
    ) -> Result<RenderedExecutionContext, ProjectionRenderError>;
}

/// 内部 typed section 机制不属于 crate 公共 API。
///
/// ```compile_fail
/// use cadence_aria::product::work_item_projection::render::ProjectionSectionId;
///
/// fn main() {
///     let _ = ProjectionSectionId::Objective;
/// }
/// ```
pub fn renderer_for(provider: &ProviderName) -> Box<dyn ProviderProjectionRenderer> {
    match provider {
        ProviderName::Codex => Box::new(CodexProjectionRenderer),
        ProviderName::ClaudeCode => Box::new(ClaudeCodeProjectionRenderer),
        ProviderName::Fake => Box::new(FakeProjectionRenderer),
    }
}

fn validate_mandatory_sections(
    role: ProjectionRenderRole,
    sections: &[ProjectionSection],
) -> Result<(), ProjectionRenderError> {
    let present = sections
        .iter()
        .map(|section| section.id)
        .collect::<BTreeSet<_>>();

    for required in ProjectionSectionId::mandatory_for(role) {
        if !present.contains(required) {
            return Err(ProjectionRenderError::MandatorySectionMissing(
                required.title().to_string(),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct ProviderRenderProfile {
    provider_label: &'static str,
    renderer_version: &'static str,
    permission_and_tool_hint: &'static str,
    structured_output_wrapper: &'static str,
}

fn render_coder_with_profile(
    profile: ProviderRenderProfile,
    projection: &CoderWorkItemProjection,
    envelope: &CoderExecutionEnvelope,
) -> Result<RenderedExecutionContext, ProjectionRenderError> {
    let sections = coder_sections(projection, envelope)?;
    render(profile, ProjectionRenderRole::Coder, sections)
}

fn render_reviewer_with_profile(
    profile: ProviderRenderProfile,
    projection: &ReviewerWorkItemProjection,
    envelope: &ReviewerExecutionEnvelope,
) -> Result<RenderedExecutionContext, ProjectionRenderError> {
    let sections = reviewer_sections(projection, envelope)?;
    render(profile, ProjectionRenderRole::Reviewer, sections)
}

fn coder_sections(
    projection: &CoderWorkItemProjection,
    envelope: &CoderExecutionEnvelope,
) -> Result<Vec<ProjectionSection>, ProjectionRenderError> {
    #[derive(Serialize)]
    struct Revision<'a> {
        work_item_revision_id: &'a str,
    }

    #[derive(Serialize)]
    struct Objective<'a> {
        objective: &'a str,
    }

    #[derive(Serialize)]
    struct ResolvedInputs<'a> {
        required_input_contracts: &'a [crate::product::work_item_contract::RequiredInputContract],
    }

    #[derive(Serialize)]
    struct ImplementationTasks<'a> {
        task_refs: &'a [String],
        tasks: &'a [crate::product::work_item_contract::WorkItemTask],
    }

    #[derive(Serialize)]
    struct WritePolicy<'a> {
        write_policy: &'a crate::product::work_item_contract::WorkItemWritePolicy,
    }

    #[derive(Serialize)]
    struct AcceptanceCriteria<'a> {
        acceptance_criteria: &'a [crate::product::work_item_contract::AcceptanceCriterion],
    }

    #[derive(Serialize)]
    struct VerificationChecks<'a> {
        verification_checks: &'a [crate::product::work_item_contract::VerificationCheck],
    }

    #[derive(Serialize)]
    struct BlockerRouting<'a> {
        blocker_rules: &'a [crate::product::work_item_contract::BlockerRule],
    }

    #[derive(Serialize)]
    struct HandoffRequirements<'a> {
        handoff_contract: &'a crate::product::work_item_contract::HandoffContract,
    }

    #[derive(Serialize)]
    struct ExecutionEnvelope<'a> {
        repository_state_ref: &'a str,
        resolved_handoff_revision_ids: &'a [String],
        unit_run_id: &'a str,
        start_commit: &'a Option<String>,
    }

    #[derive(Serialize)]
    struct PreviousReview<'a> {
        previous_actionable_review: &'a Option<String>,
    }

    Ok(vec![
        typed_section(
            ProjectionSectionId::WorkItemIdentityRevision,
            &Revision {
                work_item_revision_id: &projection.work_item_revision_id,
            },
        )?,
        typed_section(
            ProjectionSectionId::Objective,
            &Objective {
                objective: &projection.objective,
            },
        )?,
        typed_section(
            ProjectionSectionId::ResolvedInputs,
            &ResolvedInputs {
                required_input_contracts: &projection.required_input_contracts,
            },
        )?,
        typed_section(
            ProjectionSectionId::ImplementationTasks,
            &ImplementationTasks {
                task_refs: &projection.task_refs,
                tasks: &projection.tasks,
            },
        )?,
        typed_section(
            ProjectionSectionId::WritePolicy,
            &WritePolicy {
                write_policy: &projection.write_policy,
            },
        )?,
        typed_section(
            ProjectionSectionId::AcceptanceCriteria,
            &AcceptanceCriteria {
                acceptance_criteria: &projection.acceptance_criteria,
            },
        )?,
        typed_section(
            ProjectionSectionId::VerificationChecks,
            &VerificationChecks {
                verification_checks: &projection.verification_checks,
            },
        )?,
        typed_section(
            ProjectionSectionId::BlockerRouting,
            &BlockerRouting {
                blocker_rules: &projection.blocker_rules,
            },
        )?,
        typed_section(
            ProjectionSectionId::HandoffRequirements,
            &HandoffRequirements {
                handoff_contract: &projection.handoff_contract,
            },
        )?,
        typed_section(
            ProjectionSectionId::ExecutionEnvelope,
            &ExecutionEnvelope {
                repository_state_ref: &envelope.repository_state_ref,
                resolved_handoff_revision_ids: &envelope.resolved_handoff_revision_ids,
                unit_run_id: &envelope.unit_run_id,
                start_commit: &envelope.start_commit,
            },
        )?,
        typed_section(
            ProjectionSectionId::PreviousReview,
            &PreviousReview {
                previous_actionable_review: &envelope.previous_actionable_review,
            },
        )?,
    ])
}

fn reviewer_sections(
    projection: &ReviewerWorkItemProjection,
    envelope: &ReviewerExecutionEnvelope,
) -> Result<Vec<ProjectionSection>, ProjectionRenderError> {
    #[derive(Serialize)]
    struct Revision<'a> {
        work_item_revision_id: &'a str,
    }

    #[derive(Serialize)]
    struct RequirementMatrix<'a> {
        criterion_refs: &'a [String],
        requirement_matrix: &'a [super::ReviewerRequirementCheck],
    }

    #[derive(Serialize)]
    struct ScopePolicy<'a> {
        scope_policy: &'a crate::product::work_item_contract::WorkItemWritePolicy,
    }

    #[derive(Serialize)]
    struct InputContractChecks<'a> {
        input_contract_checks: &'a [crate::product::work_item_contract::RequiredInputContract],
    }

    #[derive(Serialize)]
    struct OutputContractChecks<'a> {
        output_contract_checks: &'a [crate::product::work_item_contract::PromisedOutputContract],
    }

    #[derive(Serialize)]
    struct VerificationEvidenceRules<'a> {
        verification_evidence_rules: &'a [crate::product::work_item_contract::VerificationCheck],
    }

    #[derive(Serialize)]
    struct BlockerRouting<'a> {
        blocker_routing: &'a [crate::product::work_item_contract::BlockerRule],
    }

    Ok(vec![
        typed_section(
            ProjectionSectionId::WorkItemIdentityRevision,
            &Revision {
                work_item_revision_id: &projection.work_item_revision_id,
            },
        )?,
        typed_section(
            ProjectionSectionId::AcceptanceCriteriaRequirementMatrix,
            &RequirementMatrix {
                criterion_refs: &projection.criterion_refs,
                requirement_matrix: &projection.requirement_matrix,
            },
        )?,
        typed_section(
            ProjectionSectionId::ScopePolicy,
            &ScopePolicy {
                scope_policy: &projection.scope_policy,
            },
        )?,
        typed_section(
            ProjectionSectionId::InputContractChecks,
            &InputContractChecks {
                input_contract_checks: &projection.input_contract_checks,
            },
        )?,
        typed_section(
            ProjectionSectionId::OutputContractChecks,
            &OutputContractChecks {
                output_contract_checks: &projection.output_contract_checks,
            },
        )?,
        typed_section(
            ProjectionSectionId::VerificationEvidenceRules,
            &VerificationEvidenceRules {
                verification_evidence_rules: &projection.verification_evidence_rules,
            },
        )?,
        typed_section(
            ProjectionSectionId::BlockerRouting,
            &BlockerRouting {
                blocker_routing: &projection.blocker_routing,
            },
        )?,
        typed_section(ProjectionSectionId::ReviewExecutionEvidence, envelope)?,
        ProjectionSection::new(
            ProjectionSectionId::ReviewerProcessEvidenceBoundary,
            ProjectionSectionId::ReviewerProcessEvidenceBoundary.title(),
            reviewer_process_evidence_boundary_contract(),
        ),
    ])
}

fn typed_section(
    id: ProjectionSectionId,
    payload: &impl Serialize,
) -> Result<ProjectionSection, ProjectionRenderError> {
    serde_json::to_string_pretty(payload)
        .map(|body| ProjectionSection::typed(id, body))
        .map_err(|error| ProjectionRenderError::Serialization(error.to_string()))
}

/// 各角色在 projection 路径下的结构化输出契约。
///
/// Reviewer 的最终结论由 coding_workspace_engine 的 review parser 统一解析，
/// 必须声明与解析器 Schema 对齐的 verdict JSON 契约；否则 Provider 只能自由
/// 发挥，输出的报告无法通过 Schema 校验而被误判为 blocked。
fn role_structured_output_contract(role: ProjectionRenderRole) -> &'static str {
    match role {
        ProjectionRenderRole::Coder => "",
        ProjectionRenderRole::Reviewer => concat!(
            "\nCode Review 结构化输出契约:\n",
            "- 最终审查结论必须只输出一个 JSON 对象：{\"verdict\":\"approve|request_changes|blocked\",\"summary\":\"...\",\"findings\":[...]}\n",
            "- verdict 只能使用 approve、request_changes、blocked；如果没有阻塞问题，verdict 使用 approve。\n",
            "- findings 必须包含 severity、file_path、line、message、required_action、source_stage=code_review。\n",
            "- defect_class=implementation_defect 的 finding 禁止填写 reason_code、contract_refs、capability_refs、repair_target、confidence、plan_defect_evidence；这些字段必须省略或留空。\n",
            "- implementation_defect 的证据写入 message 与 required_action 的自然语言描述；只有计划类缺陷（current_work_item_invalid、upstream_contract_invalid、dependency_graph_invalid 等）才允许携带 plan_defect_evidence 与路由字段。\n",
            "- 除最终结论 JSON 外，其余任何内容（包括路由回执、验证证据、示例和表格）不得出现 { 或 }；证据中的 JSON 片段必须改写为自然语言描述。\n",
            "- JSON 必须以 { 开头，以 } 结尾；不要输出 Markdown 代码块或自然语言总结。\n"
        ),
    }
}

fn render(
    profile: ProviderRenderProfile,
    role: ProjectionRenderRole,
    sections: Vec<ProjectionSection>,
) -> Result<RenderedExecutionContext, ProjectionRenderError> {
    validate_mandatory_sections(role, &sections)?;

    let mut text = format!(
        "# {} {} Work Item Projection\n\nPermission and Tool Guidance: {}\n\nStructured Output: {}\n{}{}",
        profile.provider_label,
        role.label(),
        profile.permission_and_tool_hint,
        profile.structured_output_wrapper,
        plan_defect_structured_output_contract(),
        role_structured_output_contract(role),
    );
    for section in sections {
        text.push_str("\n## ");
        text.push_str(&section.title);
        text.push_str("\n\n");
        text.push_str(&section.body);
        text.push('\n');
    }

    let content_hash = format!("{:x}", Sha256::digest(text.as_bytes()));
    Ok(RenderedExecutionContext {
        text,
        renderer_version: profile.renderer_version.to_string(),
        content_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_projection_renderer_mandatory_section_failure_uses_core_render_path() {
        let sections = ProjectionSectionId::mandatory_for(ProjectionRenderRole::Coder)
            .iter()
            .copied()
            .filter(|section_id| *section_id != ProjectionSectionId::WritePolicy)
            .map(|section_id| ProjectionSection::typed(section_id, "{}".to_string()))
            .collect::<Vec<_>>();
        let profile = ProviderRenderProfile {
            provider_label: "Test",
            renderer_version: "test-renderer-v1",
            permission_and_tool_hint: "test permissions",
            structured_output_wrapper: "test output",
        };

        let error = render(profile, ProjectionRenderRole::Coder, sections).unwrap_err();

        assert_eq!(
            error,
            ProjectionRenderError::MandatorySectionMissing("Write Policy".to_string())
        );
    }
}
