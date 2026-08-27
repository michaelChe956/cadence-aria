use super::*;
use crate::product::work_item_plan_policy::ReviewInvocationScope;

impl WorkspaceEngine {
    /// Returns whether the persisted policy route belongs to the currently
    /// active provider node. A previous artifact's scope must not suppress the
    /// legacy follow-up that starts a newly-created serial draft node.
    pub(crate) fn policy_route_targets_active_node(&self) -> bool {
        let Some(scope) = self.session.review_invocation_scope.as_ref() else {
            return false;
        };
        let persisted_cycle_key = match scope {
            ReviewInvocationScope::Initial {
                initial_revision_id,
                ..
            } => initial_revision_id,
            ReviewInvocationScope::Verification {
                repaired_revision_id,
                ..
            } => repaired_revision_id,
        };
        let active_cycle_key = match self.active_node_type() {
            Some(TimelineNodeType::WorkItemPlanOutlineRun) => self
                .latest_work_item_plan_outline_candidate()
                .map(|candidate| format!("outline:{}", candidate.outline.id)),
            Some(TimelineNodeType::WorkItemDraftRun) => self
                .current_work_item_draft_candidate_payload()
                .map(|candidate| format!("draft:{}", candidate.draft_record.outline_id)),
            _ => return false,
        };

        active_cycle_key
            .ok()
            .as_deref()
            .is_some_and(|active_cycle_key| active_cycle_key == persisted_cycle_key)
    }
}
