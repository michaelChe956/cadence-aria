use std::collections::{BTreeMap, BTreeSet, VecDeque};

use chrono::Utc;

use crate::product::coding_workspace_engine::{CodingWorkspaceEngine, CodingWorkspaceEngineError};

use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionUnit, CodingExecutionUnitStatus, CodingUnitRunStatus,
    GroupDependencyGateSnapshot, GroupDependencyGateStatus,
};
use crate::product::json_store::ProductStoreError;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GroupDependencyGateAudit {
    pub(crate) dependency_unit_id: Option<String>,
    pub(crate) handoff_id: Option<String>,
    pub(crate) dependency_work_item_revision_id: Option<String>,
    pub(crate) handoff_work_item_revision_id: Option<String>,
}

pub(crate) fn dependency_gate_applies(attempt: &CodingExecutionAttempt) -> bool {
    attempt.admission_kind == crate::product::coding_models::CodingAdmissionKind::ScAdvance
}

pub(crate) enum GroupUnitSelectionOutcome {
    Ready {
        unit_id: String,
        audit: Option<GroupDependencyGateAudit>,
    },
    Waiting {
        pending_unit_ids: Vec<String>,
        reason_code: String,
        message: String,
        audit: Option<GroupDependencyGateAudit>,
    },
    FailedClosed {
        reason_code: String,
        message: String,
        audit: Option<GroupDependencyGateAudit>,
    },
    Complete,
}

impl CodingWorkspaceEngine {
    pub(crate) fn select_next_sc_group_unit(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<GroupUnitSelectionOutcome, CodingWorkspaceEngineError> {
        let binding = self.store.get_plan_binding(attempt)?;
        let plan_id = attempt.work_item_group_id.as_deref().ok_or_else(|| {
            ProductStoreError::IdentityMismatch {
                kind: "coding_attempt_plan_binding",
                id: attempt.id.clone(),
            }
        })?;
        if binding.plan_id != plan_id {
            return Ok(GroupUnitSelectionOutcome::FailedClosed {
                reason_code: "SC_GROUP_DEPENDENCY_UNKNOWN".to_string(),
                message: format!(
                    "attempt plan binding {} does not match group {}",
                    binding.plan_id, plan_id
                ),
                audit: None,
            });
        }
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let lineage =
            revision_store.get_plan_lineage(&attempt.project_id, &attempt.issue_id, plan_id)?;
        let plan_revision = revision_store.get_plan_revision(
            &attempt.project_id,
            &attempt.issue_id,
            plan_id,
            &binding.bound_plan_revision_id,
        )?;
        let graph = revision_store
            .get_dependency_graph_revision(&lineage, &plan_revision.dependency_graph_revision_id)?;
        if lineage.active_revision_id.as_deref() != Some(binding.bound_plan_revision_id.as_str()) {
            return Ok(GroupUnitSelectionOutcome::FailedClosed {
                reason_code: "SC_GROUP_HANDOFF_PLAN_BINDING_MISMATCH".to_string(),
                message: format!(
                    "attempt binding {} is not the active plan revision",
                    binding.bound_plan_revision_id
                ),
                audit: None,
            });
        }
        let units =
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let mut by_logical = BTreeMap::<String, CodingExecutionUnit>::new();
        let mut by_id = BTreeMap::<String, CodingExecutionUnit>::new();
        for unit in units {
            if unit.attempt_id != attempt.id
                || unit.project_id != attempt.project_id
                || unit.issue_id != attempt.issue_id
                || unit.plan_id != plan_id
                || by_logical
                    .insert(unit.logical_work_item_id.clone(), unit.clone())
                    .is_some()
                || by_id.insert(unit.id.clone(), unit).is_some()
            {
                return Ok(GroupUnitSelectionOutcome::FailedClosed {
                    reason_code: "SC_GROUP_DEPENDENCY_UNKNOWN".to_string(),
                    message: "SC group contains duplicate or foreign coding units".to_string(),
                    audit: None,
                });
            }
        }
        if by_logical.is_empty() {
            return Ok(GroupUnitSelectionOutcome::Complete);
        }

        let mut graph_dependencies = BTreeMap::<String, BTreeSet<String>>::new();
        for logical_id in by_logical.keys() {
            graph_dependencies.insert(logical_id.clone(), BTreeSet::new());
        }
        for edge in &graph.edges {
            let Some(dependent) = by_logical.get(&edge.to) else {
                return Ok(failed_unknown(
                    &edge.to,
                    "dependency graph points to an unknown work item",
                ));
            };
            if edge.from == edge.to {
                return Ok(failed_self(&edge.from));
            }
            if !by_logical.contains_key(&edge.from) {
                return Ok(failed_unknown(
                    &edge.from,
                    "dependency graph points from an unknown work item",
                ));
            }
            graph_dependencies
                .get_mut(&dependent.logical_work_item_id)
                .expect("unit map initialized")
                .insert(edge.from.clone());
        }
        for unit in by_logical.values() {
            let declared = unit
                .dependency_logical_work_item_ids
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            if declared.iter().any(|id| id == &unit.logical_work_item_id) {
                return Ok(failed_self(&unit.logical_work_item_id));
            }
            if declared.iter().any(|id| !by_logical.contains_key(id)) {
                let id = declared
                    .iter()
                    .find(|id| !by_logical.contains_key(*id))
                    .expect("unknown dependency exists");
                return Ok(failed_unknown(
                    id,
                    "coding unit references an unknown dependency",
                ));
            }
            if declared != graph_dependencies[&unit.logical_work_item_id] {
                return Ok(GroupUnitSelectionOutcome::FailedClosed {
                    reason_code: "SC_GROUP_DEPENDENCY_UNKNOWN".to_string(),
                    message: format!(
                        "coding unit {} disagrees with the authoritative dependency graph",
                        unit.id
                    ),
                    audit: None,
                });
            }
        }

        let (layers, cycle) = topological_layers(&graph_dependencies);
        if cycle {
            return Ok(GroupUnitSelectionOutcome::FailedClosed {
                reason_code: "SC_GROUP_DEPENDENCY_CYCLE".to_string(),
                message: "SC group dependency graph contains a cycle".to_string(),
                audit: None,
            });
        }

        let pending = by_logical
            .values()
            .filter(|unit| unit.status == CodingExecutionUnitStatus::Pending)
            .cloned()
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return Ok(GroupUnitSelectionOutcome::Complete);
        }
        let mut ready = Vec::new();
        let mut ready_audit = None;
        let mut blocked_by_waiting = false;
        let mut waiting_audit = None;
        let mut waiting_message = None;
        for unit in &pending {
            let mut unit_ready = true;
            for dependency_id in &graph_dependencies[&unit.logical_work_item_id] {
                let dependency = &by_logical[dependency_id];
                match self.dependency_handoff_ready(
                    attempt,
                    &lineage,
                    &plan_revision,
                    &binding.bound_plan_revision_id,
                    dependency,
                )? {
                    DependencyReadiness::Ready { audit } => {
                        ready_audit = audit;
                    }
                    DependencyReadiness::Waiting { audit, message } => {
                        unit_ready = false;
                        blocked_by_waiting = true;
                        waiting_audit = audit;
                        waiting_message = Some(message);
                    }
                    DependencyReadiness::FailedClosed { audit, message } => {
                        let reason_code = if message.contains("has no work-item binding") {
                            "SC_GROUP_DEPENDENCY_UNKNOWN"
                        } else {
                            "SC_GROUP_HANDOFF_PLAN_BINDING_MISMATCH"
                        };
                        return Ok(GroupUnitSelectionOutcome::FailedClosed {
                            reason_code: reason_code.to_string(),
                            message,
                            audit,
                        });
                    }
                }
            }
            if unit_ready {
                ready.push(unit);
            }
        }
        if let Some(unit) = ready.into_iter().min_by(|left, right| {
            layers[&left.logical_work_item_id]
                .cmp(&layers[&right.logical_work_item_id])
                .then_with(|| left.order_index.cmp(&right.order_index))
                .then_with(|| left.logical_work_item_id.cmp(&right.logical_work_item_id))
                .then_with(|| left.id.cmp(&right.id))
        }) {
            return Ok(GroupUnitSelectionOutcome::Ready {
                unit_id: unit.id.clone(),
                audit: ready_audit,
            });
        }
        let mut pending_unit_ids = pending.into_iter().map(|unit| unit.id).collect::<Vec<_>>();
        pending_unit_ids.sort();
        Ok(GroupUnitSelectionOutcome::Waiting {
            pending_unit_ids,
            message: waiting_message
                .unwrap_or_else(|| "SC group has pending dependencies".to_string()),
            reason_code: if blocked_by_waiting {
                "SC_GROUP_DEPENDENCY_HANDOFF_PENDING".to_string()
            } else {
                "SC_GROUP_DEPENDENCY_NOT_READY".to_string()
            },
            audit: waiting_audit,
        })
    }

    pub(crate) fn persist_sc_group_dependency_gate_outcome(
        &self,
        attempt: &CodingExecutionAttempt,
        outcome: &GroupUnitSelectionOutcome,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let binding = self.store.get_plan_binding(attempt)?;
        let (status, selected_unit_id, pending_unit_ids, reason_code, message, audit) =
            match outcome {
                GroupUnitSelectionOutcome::Ready { unit_id, audit } => (
                    GroupDependencyGateStatus::Ready,
                    Some(unit_id.clone()),
                    Vec::new(),
                    None,
                    None,
                    audit.clone(),
                ),
                GroupUnitSelectionOutcome::Waiting {
                    pending_unit_ids,
                    reason_code,
                    message,
                    audit,
                } => (
                    GroupDependencyGateStatus::Waiting,
                    None,
                    pending_unit_ids.clone(),
                    Some(reason_code.clone()),
                    Some(message.clone()),
                    audit.clone(),
                ),
                GroupUnitSelectionOutcome::FailedClosed {
                    reason_code,
                    message,
                    audit,
                } => (
                    GroupDependencyGateStatus::FailedClosed,
                    None,
                    Vec::new(),
                    Some(reason_code.clone()),
                    Some(message.clone()),
                    audit.clone(),
                ),
                GroupUnitSelectionOutcome::Complete => return Ok(()),
            };
        self.store.write_group_dependency_gate_snapshot(
            attempt,
            &GroupDependencyGateSnapshot {
                attempt_id: attempt.id.clone(),
                status,
                selected_unit_id,
                pending_unit_ids,
                reason_code,
                message,
                dependency_unit_id: audit.as_ref().and_then(|a| a.dependency_unit_id.clone()),
                handoff_id: audit.as_ref().and_then(|a| a.handoff_id.clone()),
                dependency_work_item_revision_id: audit
                    .as_ref()
                    .and_then(|a| a.dependency_work_item_revision_id.clone()),
                handoff_work_item_revision_id: audit
                    .as_ref()
                    .and_then(|a| a.handoff_work_item_revision_id.clone()),
                plan_revision_id: binding.bound_plan_revision_id,
                created_at: Utc::now().to_rfc3339(),
            },
        )?;
        Ok(())
    }
    fn dependency_handoff_ready(
        &self,
        attempt: &CodingExecutionAttempt,
        lineage: &crate::product::models::WorkItemPlanLineage,
        plan_revision: &crate::product::models::WorkItemPlanRevision,
        bound_plan_revision_id: &str,
        dependency: &CodingExecutionUnit,
    ) -> Result<DependencyReadiness, CodingWorkspaceEngineError> {
        if dependency.status != CodingExecutionUnitStatus::Completed {
            return Ok(DependencyReadiness::Waiting {
                audit: Some(GroupDependencyGateAudit {
                    dependency_unit_id: Some(dependency.id.clone()),
                    handoff_id: None,
                    dependency_work_item_revision_id: Some(
                        dependency.work_item_revision_id.clone(),
                    ),
                    handoff_work_item_revision_id: None,
                }),
                message: format!("dependency unit {} is not completed", dependency.id),
            });
        }
        let Some(handoff_id) = dependency.latest_handoff_revision_id.as_deref() else {
            return Ok(DependencyReadiness::Waiting {
                audit: Some(GroupDependencyGateAudit {
                    dependency_unit_id: Some(dependency.id.clone()),
                    handoff_id: None,
                    dependency_work_item_revision_id: Some(
                        dependency.work_item_revision_id.clone(),
                    ),
                    handoff_work_item_revision_id: None,
                }),
                message: format!("dependency unit {} has no published handoff", dependency.id),
            });
        };
        let handoff = match WorkItemRevisionStore::new(self.store.paths()).get_handoff_revision(
            lineage,
            &dependency.logical_work_item_id,
            handoff_id,
        ) {
            Ok(handoff) => handoff,
            Err(ProductStoreError::NotFound { .. }) => {
                return Ok(DependencyReadiness::Waiting {
                    audit: Some(GroupDependencyGateAudit {
                        dependency_unit_id: Some(dependency.id.clone()),
                        handoff_id: Some(handoff_id.to_string()),
                        dependency_work_item_revision_id: Some(
                            dependency.work_item_revision_id.clone(),
                        ),
                        handoff_work_item_revision_id: None,
                    }),
                    message: format!("dependency handoff {} was not found", handoff_id),
                });
            }
            Err(error) => {
                return Ok(DependencyReadiness::FailedClosed {
                    audit: Some(GroupDependencyGateAudit {
                        dependency_unit_id: Some(dependency.id.clone()),
                        handoff_id: Some(handoff_id.to_string()),
                        dependency_work_item_revision_id: Some(
                            dependency.work_item_revision_id.clone(),
                        ),
                        handoff_work_item_revision_id: None,
                    }),
                    message: format!("dependency handoff {} cannot be read: {error}", handoff_id),
                });
            }
        };
        let expected_revision = plan_revision
            .work_item_bindings
            .get(&dependency.logical_work_item_id);
        let Some(expected_revision) = expected_revision else {
            return Ok(DependencyReadiness::FailedClosed {
                audit: Some(GroupDependencyGateAudit {
                    dependency_unit_id: Some(dependency.id.clone()),
                    handoff_id: Some(handoff.id.clone()),
                    dependency_work_item_revision_id: Some(
                        dependency.work_item_revision_id.clone(),
                    ),
                    handoff_work_item_revision_id: Some(handoff.work_item_revision_id.clone()),
                }),
                message: format!(
                    "dependency unit {} has no work-item binding in plan revision {}; dependency_work_item_revision_id={}",
                    dependency.id, bound_plan_revision_id, dependency.work_item_revision_id
                ),
            });
        };
        if expected_revision != &dependency.work_item_revision_id {
            return Ok(DependencyReadiness::FailedClosed {
                audit: Some(GroupDependencyGateAudit {
                    dependency_unit_id: Some(dependency.id.clone()),
                    handoff_id: Some(handoff.id.clone()),
                    dependency_work_item_revision_id: Some(
                        dependency.work_item_revision_id.clone(),
                    ),
                    handoff_work_item_revision_id: Some(handoff.work_item_revision_id.clone()),
                }),
                message: format!(
                    "dependency {} is not bound to plan revision {}; dependency_work_item_revision_id={}, bound_work_item_revision_id={}",
                    dependency.id,
                    bound_plan_revision_id,
                    dependency.work_item_revision_id,
                    expected_revision
                ),
            });
        }
        if dependency.completion_commit.is_none()
            || handoff.logical_work_item_id != dependency.logical_work_item_id
            || handoff.work_item_revision_id != dependency.work_item_revision_id
            || handoff.commit_sha != dependency.completion_commit.as_deref().unwrap_or_default()
        {
            return Ok(DependencyReadiness::FailedClosed {
                audit: Some(GroupDependencyGateAudit {
                    dependency_unit_id: Some(dependency.id.clone()),
                    handoff_id: Some(handoff.id.clone()),
                    dependency_work_item_revision_id: Some(
                        dependency.work_item_revision_id.clone(),
                    ),
                    handoff_work_item_revision_id: Some(handoff.work_item_revision_id.clone()),
                }),
                message: format!(
                    "dependency handoff {} does not match completed unit {}",
                    handoff.id, dependency.id
                ),
            });
        }
        let run = self
            .store
            .list_coding_unit_runs(attempt, &dependency.id)?
            .into_iter()
            .find(|run| run.id == handoff.coding_unit_run_id);
        match run {
            Some(run)
                if run.status == CodingUnitRunStatus::Completed
                    && run.completion_commit.as_deref()
                        == dependency.completion_commit.as_deref() =>
            {
                Ok(DependencyReadiness::Ready {
                    audit: Some(GroupDependencyGateAudit {
                        dependency_unit_id: Some(dependency.id.clone()),
                        handoff_id: Some(handoff.id.clone()),
                        dependency_work_item_revision_id: Some(
                            dependency.work_item_revision_id.clone(),
                        ),
                        handoff_work_item_revision_id: Some(handoff.work_item_revision_id.clone()),
                    }),
                })
            }
            _ => Ok(DependencyReadiness::FailedClosed {
                audit: Some(GroupDependencyGateAudit {
                    dependency_unit_id: Some(dependency.id.clone()),
                    handoff_id: Some(handoff.id.clone()),
                    dependency_work_item_revision_id: Some(
                        dependency.work_item_revision_id.clone(),
                    ),
                    handoff_work_item_revision_id: Some(handoff.work_item_revision_id.clone()),
                }),
                message: format!(
                    "dependency handoff {} references a non-authoritative unit run",
                    handoff.id
                ),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DependencyReadiness {
    Ready {
        audit: Option<GroupDependencyGateAudit>,
    },
    Waiting {
        audit: Option<GroupDependencyGateAudit>,
        message: String,
    },
    FailedClosed {
        audit: Option<GroupDependencyGateAudit>,
        message: String,
    },
}

pub(crate) fn topological_layers(
    dependencies: &BTreeMap<String, BTreeSet<String>>,
) -> (BTreeMap<String, u32>, bool) {
    let mut indegree = dependencies
        .iter()
        .map(|(id, deps)| (id.clone(), deps.len()))
        .collect::<BTreeMap<_, _>>();
    let mut dependents = BTreeMap::<String, Vec<String>>::new();
    for (id, deps) in dependencies {
        for dependency in deps {
            dependents
                .entry(dependency.clone())
                .or_default()
                .push(id.clone());
        }
    }
    let mut queue = VecDeque::new();
    let mut layers = BTreeMap::new();
    for (id, degree) in &indegree {
        if *degree == 0 {
            queue.push_back(id.clone());
            layers.insert(id.clone(), 0);
        }
    }
    let mut count = 0;
    while let Some(id) = queue.pop_front() {
        count += 1;
        for dependent in dependents.get(&id).into_iter().flatten() {
            let degree = indegree.get_mut(dependent).expect("graph endpoint exists");
            *degree -= 1;
            let layer = layers[&id] + 1;
            layers
                .entry(dependent.clone())
                .and_modify(|current| *current = (*current).max(layer))
                .or_insert(layer);
            if *degree == 0 {
                queue.push_back(dependent.clone());
            }
        }
    }
    (layers, count != dependencies.len())
}

pub(crate) fn failed_unknown(id: &str, detail: &str) -> GroupUnitSelectionOutcome {
    GroupUnitSelectionOutcome::FailedClosed {
        reason_code: "SC_GROUP_DEPENDENCY_UNKNOWN".to_string(),
        message: format!("{detail}: {id}"),
        audit: None,
    }
}

pub(crate) fn failed_self(id: &str) -> GroupUnitSelectionOutcome {
    GroupUnitSelectionOutcome::FailedClosed {
        reason_code: "SC_GROUP_DEPENDENCY_SELF".to_string(),
        message: format!("SC group dependency self-reference: {id}"),
        audit: None,
    }
}
