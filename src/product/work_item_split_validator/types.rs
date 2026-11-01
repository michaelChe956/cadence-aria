use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemSplitValidationReport {
    pub findings: Vec<WorkItemSplitFinding>,
}

impl WorkItemSplitValidationReport {
    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == WorkItemSplitFindingSeverity::Error)
    }
}

pub struct WorkItemSplitValidator;

impl WorkItemSplitValidator {
    pub fn validate(
        plan: &IssueWorkItemPlan,
        work_items: &[LifecycleWorkItemRecord],
        repository_profile: Option<&RepositoryProfile>,
        verification_plans: &[VerificationPlan],
    ) -> WorkItemSplitValidationReport {
        let mut findings = Vec::new();
        plan::validate_plan_membership(plan, work_items, &mut findings);
        plan::validate_dependencies(plan, work_items, &mut findings);
        plan::validate_scopes_and_budgets(plan, work_items, &mut findings);
        plan::validate_semantics(plan, work_items, &mut findings);
        plan::validate_verification_plans(
            plan,
            work_items,
            repository_profile,
            verification_plans,
            &mut findings,
        );
        WorkItemSplitValidationReport { findings }
    }
}

pub struct WorkItemPlanOutlineValidator;

impl WorkItemPlanOutlineValidator {
    pub fn validate(outline: &WorkItemPlanOutline) -> WorkItemSplitValidationReport {
        Self::validate_with_catalog(outline)
    }

    pub(crate) fn validate_for_single_candidate(
        outline: &WorkItemPlanOutline,
    ) -> WorkItemSplitValidationReport {
        let mut findings = Vec::new();
        outline::validate_outline_ids(outline, &mut findings);
        outline::validate_outline_traceability_scopes_and_budget(outline, &mut findings);
        let edges = outline::validate_outline_dependencies(outline, &mut findings);
        outline::validate_outline_dependency_cycles(outline, &edges, &mut findings);
        outline::validate_outline_scope_conflicts(outline, &edges, &mut findings);
        WorkItemSplitValidationReport { findings }
    }

    fn validate_with_catalog(outline: &WorkItemPlanOutline) -> WorkItemSplitValidationReport {
        let mut findings = Vec::new();
        outline::validate_outline_ids(outline, &mut findings);
        outline::validate_outline_traceability_and_scopes(outline, &mut findings);
        let edges = outline::validate_outline_dependencies(outline, &mut findings);
        outline::validate_outline_dependency_cycles(outline, &edges, &mut findings);
        outline::validate_outline_scope_conflicts(outline, &edges, &mut findings);
        WorkItemSplitValidationReport { findings }
    }
}

pub struct WorkItemDraftLocalValidator;

impl WorkItemDraftLocalValidator {
    pub fn validate(
        current: &WorkItemDraftCandidate,
        accepted_dependencies: &[WorkItemDraftCandidate],
        outline: &WorkItemPlanOutline,
    ) -> WorkItemSplitValidationReport {
        Self::validate_draft_common(current, accepted_dependencies, outline, true)
    }

    pub(crate) fn validate_for_single_candidate(
        current: &WorkItemDraftCandidate,
        accepted_dependencies: &[WorkItemDraftCandidate],
        outline: &WorkItemPlanOutline,
    ) -> WorkItemSplitValidationReport {
        Self::validate_draft_common(current, accepted_dependencies, outline, false)
    }

    fn validate_draft_common(
        current: &WorkItemDraftCandidate,
        accepted_dependencies: &[WorkItemDraftCandidate],
        outline: &WorkItemPlanOutline,
        include_trusted_catalog: bool,
    ) -> WorkItemSplitValidationReport {
        let mut findings = Vec::new();
        draft::validate_canonical_contract_candidate(current, &mut findings);
        let current_outline =
            draft::validate_draft_identity_and_find_outline(current, outline, &mut findings);
        draft::validate_draft_provider_logical_ids(current, outline, &mut findings);
        draft::validate_draft_scopes(current, &mut findings);
        draft::validate_draft_verification_plan(current, &mut findings);
        if include_trusted_catalog && let Some(current_outline) = current_outline {
            draft::validate_draft_trusted_verification_commands(
                current,
                current_outline,
                &mut findings,
            );
        }
        draft::validate_draft_direct_dependency_scopes(
            current,
            accepted_dependencies,
            &mut findings,
        );
        WorkItemSplitValidationReport { findings }
    }
}
