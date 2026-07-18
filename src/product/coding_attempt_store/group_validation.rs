use std::collections::{BTreeMap, BTreeSet};

use crate::product::coding_models::{
    CodingAttemptScope, CodingExecutionAttempt, CodingExecutionStage, CodingExecutionUnit,
    CodingExecutionUnitStatus,
};
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoritativeCodingUnitBinding {
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub dependency_logical_work_item_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoritativeGroupPlanBinding {
    pub plan_revision_id: String,
    pub units: Vec<AuthoritativeCodingUnitBinding>,
}

impl super::CodingAttemptStore {
    pub fn resolve_authoritative_group_plan_binding(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<AuthoritativeGroupPlanBinding, ProductStoreError> {
        for id in [project_id, issue_id, plan_id] {
            validate_relative_id(id)?;
        }

        let lifecycle = LifecycleStore::new(self.paths());
        let plan = lifecycle.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
        let work_items = lifecycle.list_work_items(project_id, issue_id)?;
        let mut ordered = plan
            .work_item_ids
            .iter()
            .enumerate()
            .map(|(index, logical_id)| {
                work_items
                    .iter()
                    .find(|item| item.id == *logical_id)
                    .cloned()
                    .map(|item| (index, item))
                    .ok_or_else(|| invalid_plan_binding("coding group work item is missing"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        ordered.sort_by(|(left_index, left), (right_index, right)| {
            left.sequence_hint
                .unwrap_or(u32::MAX)
                .cmp(&right.sequence_hint.unwrap_or(u32::MAX))
                .then_with(|| left_index.cmp(right_index))
        });
        if ordered.is_empty()
            || ordered
                .iter()
                .any(|(_, item)| item.work_item_set_id.as_deref() != Some(plan_id))
        {
            return Err(invalid_plan_binding(
                "coding group membership is missing or inconsistent",
            ));
        }
        let ordered_ids = ordered
            .into_iter()
            .map(|(_, item)| item.id)
            .collect::<Vec<_>>();
        let expected_ids = ordered_ids.iter().cloned().collect::<BTreeSet<_>>();
        if expected_ids.len() != ordered_ids.len() {
            return Err(invalid_plan_binding(
                "coding group contains duplicate work items",
            ));
        }

        let revision_store = WorkItemRevisionStore::new(self.paths());
        let lineage = revision_store.get_plan_lineage(project_id, issue_id, plan_id)?;
        let plan_revision_id = lineage
            .active_revision_id
            .clone()
            .ok_or_else(|| invalid_plan_binding("active plan revision is missing"))?;
        let revision =
            revision_store.get_plan_revision(project_id, issue_id, plan_id, &plan_revision_id)?;
        let binding_ids = revision
            .work_item_bindings
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if binding_ids != expected_ids {
            return Err(invalid_plan_binding(
                "active plan bindings do not exactly match the coding group",
            ));
        }
        let graph = revision_store
            .get_dependency_graph_revision(&lineage, &revision.dependency_graph_revision_id)?;
        let mut dependencies = expected_ids
            .iter()
            .map(|logical_id| (logical_id.clone(), Vec::new()))
            .collect::<BTreeMap<_, _>>();
        let mut seen_edges = BTreeSet::new();
        for edge in &graph.edges {
            if edge.from == edge.to
                || !expected_ids.contains(&edge.from)
                || !expected_ids.contains(&edge.to)
                || !seen_edges.insert((edge.from.clone(), edge.to.clone()))
            {
                return Err(invalid_plan_binding(
                    "dependency graph contains an invalid coding unit edge",
                ));
            }
            dependencies
                .get_mut(&edge.to)
                .expect("validated dependency target")
                .push(edge.from.clone());
        }
        for values in dependencies.values_mut() {
            values.sort();
        }

        let mut units = Vec::with_capacity(ordered_ids.len());
        for logical_id in ordered_ids {
            let revision_id = revision
                .work_item_bindings
                .get(&logical_id)
                .expect("exact binding coverage")
                .clone();
            if logical_id == revision_id {
                return Err(invalid_plan_binding(
                    "logical work item ID aliases its revision ID",
                ));
            }
            let logical = revision_store.get_logical_work_item(&lineage, &logical_id)?;
            if logical.active_revision_id.as_deref() != Some(revision_id.as_str()) {
                return Err(invalid_plan_binding(
                    "logical work item active revision differs from the plan binding",
                ));
            }
            let work_item_revision =
                revision_store.get_work_item_revision(&lineage, &logical_id, &revision_id)?;
            if work_item_revision.logical_work_item_id != logical_id
                || work_item_revision
                    .canonical_contract
                    .identity
                    .logical_work_item_id
                    != logical_id
            {
                return Err(invalid_plan_binding(
                    "work item revision does not belong to the bound logical work item",
                ));
            }
            units.push(AuthoritativeCodingUnitBinding {
                logical_work_item_id: logical_id.clone(),
                work_item_revision_id: revision_id,
                dependency_logical_work_item_ids: dependencies
                    .remove(&logical_id)
                    .expect("known logical work item"),
            });
        }

        Ok(AuthoritativeGroupPlanBinding {
            plan_revision_id,
            units,
        })
    }

    pub fn validate_group_attempt_integrity(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<AuthoritativeGroupPlanBinding, ProductStoreError> {
        let (stored, authoritative, units) = self.validate_group_attempt_structure(attempt)?;
        validate_group_attempt_pointers(&stored, &units)?;
        Ok(authoritative)
    }

    pub(super) fn validate_group_attempt_structure(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<
        (
            CodingExecutionAttempt,
            AuthoritativeGroupPlanBinding,
            Vec<CodingExecutionUnit>,
        ),
        ProductStoreError,
    > {
        let stored = self.validate_attempt_lineage(attempt)?;
        let plan_id = match (&stored.scope, stored.work_item_group_id.as_deref()) {
            (CodingAttemptScope::WorkItemGroup, Some(plan_id)) => plan_id,
            _ => {
                return Err(incomplete_group_attempt(
                    &stored.id,
                    "scope or plan is missing",
                ));
            }
        };
        let authoritative = self
            .resolve_authoritative_group_plan_binding(&stored.project_id, &stored.issue_id, plan_id)
            .map_err(|error| map_group_integrity_dependency_error(&stored.id, error))?;
        if stored.work_item_id
            != authoritative
                .units
                .first()
                .map(|unit| unit.logical_work_item_id.as_str())
                .unwrap_or_default()
        {
            return Err(incomplete_group_attempt(
                &stored.id,
                "attempt root work item differs from authoritative order",
            ));
        }
        let binding = self
            .get_plan_binding(&stored)
            .map_err(|error| map_group_integrity_dependency_error(&stored.id, error))?;
        if binding.plan_id != plan_id
            || binding.bound_plan_revision_id != authoritative.plan_revision_id
        {
            return Err(incomplete_group_attempt(
                &stored.id,
                "attempt plan binding is stale or inconsistent",
            ));
        }
        let units = self
            .list_coding_units(&stored.project_id, &stored.issue_id, &stored.id)
            .map_err(|error| map_group_integrity_dependency_error(&stored.id, error))?;
        if units.len() != authoritative.units.len()
            || units
                .iter()
                .zip(authoritative.units.iter())
                .enumerate()
                .any(|(index, (unit, expected))| {
                    !unit_matches_authoritative(&stored, plan_id, index, unit, expected)
                })
        {
            return Err(incomplete_group_attempt(
                &stored.id,
                "coding unit set is incomplete or inconsistent",
            ));
        }
        Ok((stored, authoritative, units))
    }
}

pub fn is_group_business_validation_error(error: &ProductStoreError) -> bool {
    matches!(
        error,
        ProductStoreError::NotFound { .. } | ProductStoreError::IdentityMismatch { .. }
    ) || matches!(
        error,
        ProductStoreError::Io(message)
            if message.starts_with("coding_plan_revision_binding_missing:")
                || message.starts_with("coding_group_attempt_incomplete:")
    )
}

fn map_group_integrity_dependency_error(
    attempt_id: &str,
    error: ProductStoreError,
) -> ProductStoreError {
    if is_group_business_validation_error(&error) {
        incomplete_group_attempt(attempt_id, &error.to_string())
    } else {
        error
    }
}

fn validate_group_attempt_pointers(
    attempt: &CodingExecutionAttempt,
    units: &[CodingExecutionUnit],
) -> Result<(), ProductStoreError> {
    let active_units = units
        .iter()
        .filter(|unit| unit.status.is_active())
        .collect::<Vec<_>>();
    match active_units.as_slice() {
        [active] if attempt.status.is_active() => {
            if attempt.active_unit_id.as_deref() != Some(active.id.as_str())
                || attempt.current_work_item_id.as_deref()
                    != Some(active.logical_work_item_id.as_str())
            {
                return Err(incomplete_group_attempt(
                    &attempt.id,
                    "active and current pointers do not match the unique active unit",
                ));
            }
        }
        [] => {
            let pointers_are_empty =
                attempt.active_unit_id.is_none() && attempt.current_work_item_id.is_none();
            let terminal_units = units.iter().all(|unit| {
                !unit.status.is_active() && unit.status != CodingExecutionUnitStatus::Pending
            });
            let final_review_units = units
                .iter()
                .all(|unit| unit.status == CodingExecutionUnitStatus::Completed);
            let terminal_no_target_is_allowed = match attempt.status {
                crate::product::coding_models::CodingAttemptStatus::Completed => final_review_units,
                crate::product::coding_models::CodingAttemptStatus::Failed
                | crate::product::coding_models::CodingAttemptStatus::Aborted => terminal_units,
                _ => false,
            };
            let final_review_no_target_is_allowed = attempt.status.is_active()
                && attempt.status != crate::product::coding_models::CodingAttemptStatus::Created
                && attempt.stage.order() >= CodingExecutionStage::ReviewRequest.order()
                && final_review_units;
            let no_target_is_allowed =
                terminal_no_target_is_allowed || final_review_no_target_is_allowed;
            if !pointers_are_empty || !no_target_is_allowed {
                return Err(incomplete_group_attempt(
                    &attempt.id,
                    "attempt has no legal active or resume target",
                ));
            }
        }
        _ => {
            return Err(incomplete_group_attempt(
                &attempt.id,
                "attempt has multiple active units or a terminal status with an active unit",
            ));
        }
    }
    Ok(())
}

fn unit_matches_authoritative(
    attempt: &CodingExecutionAttempt,
    plan_id: &str,
    index: usize,
    unit: &CodingExecutionUnit,
    expected: &AuthoritativeCodingUnitBinding,
) -> bool {
    unit.attempt_id == attempt.id
        && unit.project_id == attempt.project_id
        && unit.issue_id == attempt.issue_id
        && unit.plan_id == plan_id
        && unit.logical_work_item_id == expected.logical_work_item_id
        && unit.work_item_revision_id == expected.work_item_revision_id
        && unit.dependency_logical_work_item_ids == expected.dependency_logical_work_item_ids
        && unit.order_index == index as u32
}

fn invalid_plan_binding(reason: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind: "coding_plan_revision_binding",
        id: reason.to_string(),
    }
}

pub(super) fn incomplete_group_attempt(attempt_id: &str, reason: &str) -> ProductStoreError {
    ProductStoreError::Io(format!(
        "coding_group_attempt_incomplete: {attempt_id}: {reason}"
    ))
}
