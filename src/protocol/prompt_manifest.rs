use crate::protocol::contracts::{NodePromptTemplateRef, PromptSection};
use crate::protocol::enums::NodeId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PromptManifest {
    pub entries: Vec<PromptManifestEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PromptManifestEntry {
    pub node_id: NodeId,
    pub template_id: String,
    pub output_schema_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptTemplate {
    pub template_ref: NodePromptTemplateRef,
    pub sections: BTreeMap<PromptSection, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PromptRenderError {
    #[error("prompt_render_missing_variable: {0}")]
    MissingVariable(String),
    #[error("prompt_render_missing_section: {0:?}")]
    MissingSection(PromptSection),
}

pub fn planning_prompt_manifest() -> PromptManifest {
    PromptManifest {
        entries: vec![
            entry(
                "N04",
                "tpl_n04_clarification_v1",
                "schema://aria/artifacts/clarification_record/v1",
            ),
            entry(
                "N05",
                "tpl_n05_spec_authoring_v1",
                "schema://aria/artifacts/spec/v1",
            ),
            entry(
                "N06",
                "tpl_n06_spec_gate_advisory_v1",
                "schema://aria/advisory/spec_gate_review/v1",
            ),
            entry(
                "N07",
                "tpl_n07_design_authoring_v1",
                "schema://aria/artifacts/design/v1",
            ),
            entry(
                "N08",
                "tpl_n08_design_review_v1",
                "schema://aria/artifacts/design_review/v1",
            ),
            entry(
                "N09",
                "tpl_n09_design_revision_v1",
                "schema://aria/artifacts/design_revision_record/v1",
            ),
            entry(
                "N10",
                "tpl_n10_readiness_check_v1",
                "schema://aria/artifacts/readiness_check/v1",
            ),
            entry(
                "N11",
                "tpl_n11_plan_authoring_v1",
                "schema://aria/artifacts/plan/v1",
            ),
            entry(
                "N12",
                "tpl_n12_dispatch_authoring_v1",
                "schema://aria/artifacts/dispatch_package/v1",
            ),
        ],
    }
}

pub fn phase1_prompt_manifest() -> PromptManifest {
    let mut manifest = planning_prompt_manifest();
    manifest.entries.extend([
        entry(
            "N16",
            "tpl_n16_coding_v1",
            "schema://aria/artifacts/coding_report/v1",
        ),
        entry(
            "N17",
            "tpl_n17_testing_v1",
            "schema://aria/artifacts/testing_report/v1",
        ),
        entry(
            "N18",
            "tpl_n18_code_review_v1",
            "schema://aria/artifacts/code_review_report/v1",
        ),
        entry(
            "N19",
            "tpl_n19_rework_v1",
            "schema://aria/artifacts/coding_report/v1",
        ),
        entry(
            "N20",
            "tpl_n20_ready_advisory_v1",
            "schema://aria/advisory/ready_advisory/v1",
        ),
        entry(
            "N24",
            "tpl_n24_integration_verify_advisory_v1",
            "schema://aria/advisory/integration_verify_advisory/v1",
        ),
        entry(
            "N25",
            "tpl_n25_final_review_v1",
            "schema://aria/artifacts/final_review/v1",
        ),
        entry(
            "N26",
            "tpl_n26_patch_followup_dispatch_v1",
            "schema://aria/artifacts/dispatch_package/v1",
        ),
        entry(
            "N27",
            "tpl_n27_final_summary_v1",
            "schema://aria/artifacts/final_summary/v1",
        ),
    ]);
    manifest
}

pub fn render_prompt_template(
    template: &PromptTemplate,
    variables: &BTreeMap<String, String>,
) -> Result<String, PromptRenderError> {
    let mut rendered = String::new();
    for section in &template.template_ref.render_order {
        let source = template
            .sections
            .get(section)
            .ok_or(PromptRenderError::MissingSection(*section))?;
        rendered.push_str(&render_section(source, variables)?);
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        rendered.push('\n');
    }
    Ok(rendered)
}

pub fn required_prompt_sections() -> Vec<PromptSection> {
    vec![
        PromptSection::System,
        PromptSection::NodeContract,
        PromptSection::CanonicalInputs,
        PromptSection::ProjectionSummary,
        PromptSection::ConstraintSummary,
        PromptSection::WorkflowDiscipline,
        PromptSection::OutputSchema,
        PromptSection::CompletionOrFailure,
    ]
}

fn render_section(
    source: &str,
    variables: &BTreeMap<String, String>,
) -> Result<String, PromptRenderError> {
    let mut output = String::new();
    let mut rest = source;
    while let Some(start) = rest.find("{{") {
        let before = &rest[..start];
        output.push_str(before);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            output.push_str(&rest[start..]);
            return Ok(output);
        };
        let variable = after_start[..end].trim();
        let value = variables
            .get(variable)
            .ok_or_else(|| PromptRenderError::MissingVariable(variable.to_string()))?;
        output.push_str(value);
        rest = &after_start[end + 2..];
    }
    output.push_str(rest);
    Ok(output)
}

fn entry(node_id: &str, template_id: &str, output_schema_ref: &str) -> PromptManifestEntry {
    PromptManifestEntry {
        node_id: node_id.to_string(),
        template_id: template_id.to_string(),
        output_schema_ref: output_schema_ref.to_string(),
    }
}
