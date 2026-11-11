mod ac_paths;
mod baseline_candidates;
mod check_target_forbidden;
mod equivalence;
pub mod freshness;
pub mod grammar;
mod lower;
pub mod normalize;
mod parse;
pub mod types;
mod validate;
mod write_scope_bound;

pub use equivalence::{CompiledPlanAudit, compile_work_item_plan_audited};
pub use freshness::{FreshnessError, verify_publish_freshness};
pub use grammar::*;
pub use lower::{
    PlanCandidateIr, PlanCandidateItemIr, PlanMemberTargets, WorkItemPlanSourceContext,
    compile_work_item_plan, lower_work_item_plan,
};
pub use normalize::{
    NormalizedPlanSource, PLAN_EARS_SPACING_NORMALIZATION_DIAGNOSTIC,
    PLAN_HEADING_NORMALIZATION_DIAGNOSTIC, normalize_delivery_before_compile,
    normalize_ears_statement_spacing, normalize_structural_headings,
};
pub use parse::{lint_work_item_plan_source, parse_work_item_plan};
pub use types::*;
pub use validate::{validate_plan_candidate_ir, validate_work_item_intent_contract};

#[cfg(test)]
mod equivalence_tests;
#[cfg(test)]
mod tests;
