use std::collections::{BTreeMap, BTreeSet};

use crate::product::coding_models::{
    CodingAttemptScope, CodingExecutionAttempt, CodingExecutionUnit,
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
            .map_err(|error| incomplete_group_attempt(&stored.id, &error.to_string()))?;
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
            .map_err(|error| incomplete_group_attempt(&stored.id, &error.to_string()))?;
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
            .map_err(|error| incomplete_group_attempt(&stored.id, &error.to_string()))?;
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
        if stored
            .active_unit_id
            .as_ref()
            .is_some_and(|active_id| !units.iter().any(|unit| &unit.id == active_id))
            || stored
                .current_work_item_id
                .as_ref()
                .is_some_and(|current_id| {
                    !authoritative
                        .units
                        .iter()
                        .any(|unit| &unit.logical_work_item_id == current_id)
                })
        {
            return Err(incomplete_group_attempt(
                &stored.id,
                "attempt active unit identity is inconsistent",
            ));
        }
        Ok(authoritative)
    }
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
    ProductStoreError::Io(format!("coding_plan_revision_binding_invalid: {reason}"))
}

fn incomplete_group_attempt(attempt_id: &str, reason: &str) -> ProductStoreError {
    ProductStoreError::Io(format!(
        "coding_group_attempt_incomplete: {attempt_id}: {reason}"
    ))
}
