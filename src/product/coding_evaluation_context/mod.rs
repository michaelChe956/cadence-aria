use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::product::coding_models::QualityGateBypassAudit;

mod builder;
mod methods;
mod repo;
mod sanitize;
mod specs;

#[cfg(test)]
mod tests;

pub use builder::build_evaluation_context_pack;

pub(super) const MAX_CONTEXT_SECTION_CHARS: usize = 30_000;
pub(super) const MAX_DIFF_CONTEXT_CHARS: usize = 12_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationContextRole {
    Coder,
    Tester,
    Analyst,
    CodeReviewer,
    InternalReviewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationContextPack {
    pub issue_id: String,
    pub attempt_id: String,
    pub provider_role: EvaluationContextRole,
    pub story_specs: Vec<EvaluationSpecContext>,
    pub design_specs: Vec<EvaluationSpecContext>,
    pub work_item: EvaluationWorkItemContext,
    pub group_context: Option<CodingGroupContextPack>,
    pub repo_context: EvaluationRepoContext,
    pub openspec_context: OpenSpecContext,
    pub superpowers_context: SuperpowersContext,
    pub quality_bypass_audits: Vec<QualityGateBypassAudit>,
    pub context_warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationSpecContext {
    pub artifact_id: String,
    pub version_id: Option<String>,
    pub version: Option<u32>,
    pub title: String,
    pub raw_markdown_or_sections: String,
    pub workspace_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationWorkItemContext {
    pub artifact_id: String,
    pub version_id: Option<String>,
    pub version: Option<u32>,
    pub title: String,
    pub repository_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub raw_markdown_or_sections: String,
    pub workspace_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingGroupContextPack {
    pub plan_id: String,
    pub current_work_item_id: String,
    pub sibling_work_item_ids: Vec<String>,
    pub dependency_handoff_refs: Vec<String>,
    pub source_outline_id: Option<String>,
    pub source_draft_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationRepoContext {
    pub repository_id: Option<String>,
    pub branch_name: String,
    pub base_branch: String,
    pub worktree_path: Option<String>,
    pub changed_files: Vec<String>,
    pub diff_stat: String,
    pub diff_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenSpecContext {
    pub enabled: bool,
    pub active_change_id: Option<String>,
    pub relevant_requirements: Vec<String>,
    pub traceability_notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuperpowersContext {
    pub enabled: bool,
    pub required_methods_by_role: BTreeMap<String, Vec<String>>,
}
