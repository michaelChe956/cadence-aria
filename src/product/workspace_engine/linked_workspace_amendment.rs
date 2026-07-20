use chrono::Utc;

use crate::cross_cutting::document_ops::compute_sha256;
use crate::product::json_store::ProductStoreError;
use crate::product::models::{
    PlanRepairSessionStage, WorkspaceReturnContext, WorkspaceSessionLink,
    WorkspaceSessionLinkTrigger, WorkspaceSessionRelation, WorkspaceType,
};
use crate::product::plan_repair::PlanRepairError;

use super::*;

impl WorkspaceEngine {
    pub fn start_linked_workspace_amendment(
        &self,
        target: LinkedWorkspaceAmendmentTarget,
    ) -> Result<LinkedWorkspaceSessionSnapshot, PlanRepairError> {
        validate_amendment_target(&target)?;
        let lifecycle = self.lifecycle_store.as_ref().ok_or_else(|| {
            PlanRepairError::Store(ProductStoreError::Io(
                "linked workspace amendment requires a persistent workspace engine".to_string(),
            ))
        })?;
        let repair = self.plan_repair_snapshot.as_ref().ok_or_else(|| {
            invalid("linked workspace amendment requires a Plan Repair child snapshot")
        })?;
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || !matches!(
                repair.stage,
                PlanRepairSessionStage::Triaging
                    | PlanRepairSessionStage::AuthoringRevision
                    | PlanRepairSessionStage::ValidatingContract
                    | PlanRepairSessionStage::GeneratingProjections
                    | PlanRepairSessionStage::PlanReview
                    | PlanRepairSessionStage::AwaitingConfirmation
            )
        {
            return Err(invalid(
                "linked workspace amendment requires an active WorkItemPlan Repair child",
            ));
        }
        let (repair_link, repair_child) = linked_child_session(
            lifecycle,
            &self.session.project_id,
            &self.session.issue_id,
            &repair.request,
        )?
        .ok_or_else(|| invalid("canonical Plan Repair child link is missing"))?;
        if repair.link != repair_link
            || repair_child.id != self.session.session_id
            || repair_child.entity_id != self.session.entity_id
        {
            return Err(invalid(
                "current workspace is not the canonical Plan Repair child",
            ));
        }
        let plan = WorkItemRevisionStore::new(lifecycle.app_paths())
            .get_plan_lineage(
                &self.session.project_id,
                &self.session.issue_id,
                &repair.request.plan_id,
            )
            .map_err(PlanRepairError::Store)?;
        let target_refs = match target.workspace_type {
            WorkspaceType::Story => &plan.story_spec_refs,
            WorkspaceType::Design => &plan.design_spec_refs,
            WorkspaceType::WorkItem | WorkspaceType::WorkItemPlan => unreachable!("validated"),
        };
        if !target_refs.iter().any(|id| id == &target.entity_id) {
            return Err(invalid(
                "linked workspace amendment target is not referenced by the active plan",
            ));
        }
        validate_target_record(
            lifecycle,
            &self.session.project_id,
            &self.session.issue_id,
            &target,
        )?;

        let amendment_id = repair
            .request
            .amendment_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| invalid("Plan Repair amendment identity is missing"))?;
        let (link_id, child_session_id) = linked_amendment_ids(
            &self.session.session_id,
            &target.relation,
            &target.entity_id,
            amendment_id,
        );
        if let Some(link) = find_existing_link(
            lifecycle,
            repair,
            &target,
            LinkedAmendmentLookup {
                project_id: &self.session.project_id,
                issue_id: &self.session.issue_id,
                parent_session_id: &self.session.session_id,
                link_id: &link_id,
                child_session_id: &child_session_id,
            },
        )? {
            validate_upgrade_link(
                &self.session.session_id,
                repair,
                &target,
                &link,
                &link_id,
                &child_session_id,
            )?;
            let child = lifecycle
                .get_workspace_session(&link.child_session_id)
                .map_err(PlanRepairError::Store)?;
            validate_target_child(self, &target, &child, &child_session_id)?;
            return restore_linked_workspace_snapshot(
                lifecycle,
                &self.session.project_id,
                &self.session.issue_id,
                &link,
            );
        }

        let child = match lifecycle.get_workspace_session(&child_session_id) {
            Ok(child) => {
                validate_target_child(self, &target, &child, &child_session_id)?;
                child
            }
            Err(ProductStoreError::NotFound { .. }) => lifecycle
                .create_workspace_session_with_id(
                    CreateWorkspaceSessionInput {
                        project_id: self.session.project_id.clone(),
                        issue_id: self.session.issue_id.clone(),
                        entity_id: target.entity_id.clone(),
                        workspace_type: target.workspace_type.clone(),
                        author_provider: self.session.author_provider.clone(),
                        reviewer_provider: self
                            .session
                            .reviewer_provider
                            .clone()
                            .unwrap_or_else(|| self.session.author_provider.clone()),
                        review_rounds: self.session.review_rounds,
                        superpowers_enabled: self.session.superpowers_enabled,
                        openspec_enabled: self.session.openspec_enabled,
                    },
                    child_session_id.clone(),
                )
                .map_err(PlanRepairError::Store)?,
            Err(error) => return Err(PlanRepairError::Store(error)),
        };
        let link = WorkspaceSessionLink {
            id: link_id,
            relation: target.relation,
            parent_session_id: self.session.session_id.clone(),
            child_session_id: child.id,
            trigger: WorkspaceSessionLinkTrigger {
                attempt_id: repair.request.trigger_attempt_id.clone(),
                unit_run_id: repair.request.trigger_unit_run_id.clone(),
                review_id: repair.request.trigger_review_id.clone(),
                finding_id: repair.request.trigger_finding_id.clone(),
                repair_request_id: repair.request.id.clone(),
                amendment_id: amendment_id.to_string(),
                fingerprint: repair.request.fingerprint.clone(),
                base_plan_revision_id: repair.request.base_plan_revision_id.clone(),
            },
            return_context: WorkspaceReturnContext {
                original_attempt_id: repair.request.trigger_attempt_id.clone(),
                original_unit_run_id: repair.request.trigger_unit_run_id.clone(),
                timeline_anchor_id: repair.request.trigger_finding_id.clone(),
                original_route: format!("/workbench/workspace/{}", self.session.session_id),
            },
            created_at: Utc::now().to_rfc3339(),
        };
        lifecycle
            .put_session_link(&self.session.project_id, &self.session.issue_id, &link)
            .map_err(PlanRepairError::Store)?;
        restore_linked_workspace_snapshot(
            lifecycle,
            &self.session.project_id,
            &self.session.issue_id,
            &link,
        )
    }
}

pub fn restore_linked_workspace_snapshot(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    link: &WorkspaceSessionLink,
) -> Result<LinkedWorkspaceSessionSnapshot, PlanRepairError> {
    let link = load_persisted_link(lifecycle, project_id, issue_id, link)?;
    validate_complete_link_identity(&link)?;
    let child = lifecycle
        .get_workspace_session(&link.child_session_id)
        .map_err(PlanRepairError::Store)?;
    if child.project_id != project_id
        || child.issue_id != issue_id
        || !relation_matches_workspace(&link.relation, &child.workspace_type)
    {
        return Err(identity_mismatch("workspace_session", &child.id));
    }
    if matches!(
        link.relation,
        WorkspaceSessionRelation::StoryAmendment | WorkspaceSessionRelation::DesignAmendment
    ) {
        validate_linked_amendment_restore_authority(
            lifecycle, project_id, issue_id, &link, &child,
        )?;
    }
    let versions = lifecycle
        .list_artifact_versions_for_issue_session(project_id, issue_id, &child.id)
        .map_err(PlanRepairError::Store)?;
    let current_versions = versions
        .iter()
        .filter(|version| version.is_current)
        .map(|version| version.version)
        .collect::<Vec<_>>();
    if current_versions.len() > 1 || (!versions.is_empty() && current_versions.is_empty()) {
        return Err(identity_mismatch("workspace_artifact_version", &child.id));
    }
    let timeline_nodes = lifecycle
        .load_timeline_nodes_for_issue_session(project_id, issue_id, &child.id)
        .map_err(PlanRepairError::Store)?;
    let unique_node_ids = timeline_nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if unique_node_ids.len() != timeline_nodes.len() {
        return Err(identity_mismatch("workspace_timeline", &child.id));
    }
    Ok(LinkedWorkspaceSessionSnapshot {
        link,
        workspace_type: child.workspace_type,
        artifact_version_id: current_versions.first().copied(),
        selected_timeline_node_id: active_timeline_node_id(&timeline_nodes),
        timeline_nodes,
        human_confirm_state: child.status,
    })
}

fn validate_amendment_target(
    target: &LinkedWorkspaceAmendmentTarget,
) -> Result<(), PlanRepairError> {
    if target.entity_id.trim().is_empty()
        || !matches!(
            (&target.workspace_type, &target.relation),
            (
                WorkspaceType::Story,
                WorkspaceSessionRelation::StoryAmendment
            ) | (
                WorkspaceType::Design,
                WorkspaceSessionRelation::DesignAmendment
            )
        )
    {
        return Err(invalid(
            "linked workspace amendment target type and relation are inconsistent",
        ));
    }
    Ok(())
}

struct LinkedAmendmentLookup<'a> {
    project_id: &'a str,
    issue_id: &'a str,
    parent_session_id: &'a str,
    link_id: &'a str,
    child_session_id: &'a str,
}

fn find_existing_link(
    lifecycle: &LifecycleStore,
    repair: &PlanRepairSessionSnapshotDto,
    target: &LinkedWorkspaceAmendmentTarget,
    lookup: LinkedAmendmentLookup<'_>,
) -> Result<Option<WorkspaceSessionLink>, PlanRepairError> {
    let links = lifecycle
        .list_session_links(lookup.project_id, lookup.issue_id)
        .map_err(PlanRepairError::Store)?;
    let mut candidates = Vec::new();
    for link in links {
        if link.id == lookup.link_id
            || link.child_session_id == lookup.child_session_id
            || semantic_upgrade_link_candidate(
                lifecycle,
                lookup.parent_session_id,
                repair,
                target,
                &link,
            )?
        {
            candidates.push(link);
        }
    }
    if candidates.len() > 1 {
        return Err(identity_mismatch("workspace_session_link", lookup.link_id));
    }
    Ok(candidates.pop())
}

fn validate_upgrade_link(
    parent_session_id: &str,
    repair: &PlanRepairSessionSnapshotDto,
    target: &LinkedWorkspaceAmendmentTarget,
    link: &WorkspaceSessionLink,
    link_id: &str,
    child_session_id: &str,
) -> Result<(), PlanRepairError> {
    let amendment_id = repair.request.amendment_id.as_deref().unwrap_or_default();
    if link.id != link_id
        || link.relation != target.relation
        || link.parent_session_id != parent_session_id
        || link.child_session_id != child_session_id
        || link.trigger.attempt_id != repair.request.trigger_attempt_id
        || link.trigger.unit_run_id != repair.request.trigger_unit_run_id
        || link.trigger.review_id != repair.request.trigger_review_id
        || link.trigger.finding_id != repair.request.trigger_finding_id
        || link.trigger.repair_request_id != repair.request.id
        || link.trigger.amendment_id != amendment_id
        || link.trigger.fingerprint != repair.request.fingerprint
        || link.trigger.base_plan_revision_id != repair.request.base_plan_revision_id
        || link.return_context.original_attempt_id != repair.request.trigger_attempt_id
        || link.return_context.original_unit_run_id != repair.request.trigger_unit_run_id
        || link.return_context.timeline_anchor_id != repair.request.trigger_finding_id
        || link.return_context.original_route != format!("/workbench/workspace/{parent_session_id}")
    {
        return Err(identity_mismatch("workspace_session_link", &link.id));
    }
    Ok(())
}

fn load_persisted_link(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    supplied: &WorkspaceSessionLink,
) -> Result<WorkspaceSessionLink, PlanRepairError> {
    let mut matches = lifecycle
        .list_session_links(project_id, issue_id)
        .map_err(PlanRepairError::Store)?
        .into_iter()
        .filter(|link| {
            link.id == supplied.id || link.child_session_id == supplied.child_session_id
        });
    let persisted = matches
        .next()
        .ok_or_else(|| identity_mismatch("workspace_session_link", &supplied.id))?;
    if matches.next().is_some() || persisted != *supplied {
        return Err(identity_mismatch("workspace_session_link", &supplied.id));
    }
    Ok(persisted)
}

fn validate_linked_amendment_restore_authority(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    link: &WorkspaceSessionLink,
    child: &WorkspaceSessionRecord,
) -> Result<(), PlanRepairError> {
    let repair = lifecycle
        .load_plan_repair_session_state(project_id, issue_id, &link.parent_session_id)
        .map_err(PlanRepairError::Store)?
        .ok_or_else(|| identity_mismatch("plan_repair_session_state", &link.parent_session_id))?;
    let (repair_link, repair_child) =
        linked_child_session(lifecycle, project_id, issue_id, &repair.request)?
            .ok_or_else(|| identity_mismatch("workspace_session_link", &repair.link.id))?;
    if repair.link != repair_link
        || repair_child.id != link.parent_session_id
        || repair_child.entity_id != repair.request.plan_id
    {
        return Err(identity_mismatch(
            "workspace_session",
            &link.parent_session_id,
        ));
    }

    let target = LinkedWorkspaceAmendmentTarget {
        entity_id: child.entity_id.clone(),
        workspace_type: child.workspace_type.clone(),
        relation: link.relation.clone(),
    };
    validate_amendment_target(&target)?;
    validate_target_record(lifecycle, project_id, issue_id, &target)?;
    let plan = WorkItemRevisionStore::new(lifecycle.app_paths())
        .get_plan_lineage(project_id, issue_id, &repair.request.plan_id)
        .map_err(PlanRepairError::Store)?;
    let target_refs = match target.workspace_type {
        WorkspaceType::Story => &plan.story_spec_refs,
        WorkspaceType::Design => &plan.design_spec_refs,
        WorkspaceType::WorkItem | WorkspaceType::WorkItemPlan => unreachable!("validated"),
    };
    if plan.active_revision_id.as_deref() != Some(repair.request.base_plan_revision_id.as_str())
        || !target_refs.iter().any(|id| id == &target.entity_id)
    {
        return Err(identity_mismatch("work_item_plan_lineage", &plan.id));
    }
    let amendment_id = repair
        .request
        .amendment_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| identity_mismatch("plan_repair_request", &repair.request.id))?;
    let (link_id, child_session_id) = linked_amendment_ids(
        &repair_child.id,
        &target.relation,
        &target.entity_id,
        amendment_id,
    );
    validate_upgrade_link(
        &repair_child.id,
        &repair,
        &target,
        link,
        &link_id,
        &child_session_id,
    )?;
    let existing = find_existing_link(
        lifecycle,
        &repair,
        &target,
        LinkedAmendmentLookup {
            project_id,
            issue_id,
            parent_session_id: &repair_child.id,
            link_id: &link_id,
            child_session_id: &child_session_id,
        },
    )?
    .ok_or_else(|| identity_mismatch("workspace_session_link", &link_id))?;
    if existing != *link {
        return Err(identity_mismatch("workspace_session_link", &link.id));
    }
    Ok(())
}

fn semantic_upgrade_link_candidate(
    lifecycle: &LifecycleStore,
    parent_session_id: &str,
    repair: &PlanRepairSessionSnapshotDto,
    target: &LinkedWorkspaceAmendmentTarget,
    link: &WorkspaceSessionLink,
) -> Result<bool, PlanRepairError> {
    let repair_identity_matches = link.parent_session_id == parent_session_id
        || link.trigger.repair_request_id == repair.request.id
        || repair
            .request
            .amendment_id
            .as_deref()
            .is_some_and(|amendment_id| link.trigger.amendment_id == amendment_id);
    if !repair_identity_matches {
        return Ok(false);
    }
    match lifecycle.get_workspace_session(&link.child_session_id) {
        Ok(child) => Ok(child.entity_id == target.entity_id),
        Err(ProductStoreError::NotFound { .. }) => Ok(link.relation == target.relation),
        Err(error) => Err(PlanRepairError::Store(error)),
    }
}

fn validate_target_record(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    target: &LinkedWorkspaceAmendmentTarget,
) -> Result<(), PlanRepairError> {
    let matches = match target.workspace_type {
        WorkspaceType::Story => lifecycle
            .list_story_specs(project_id, issue_id)
            .map_err(PlanRepairError::Store)?
            .into_iter()
            .filter(|record| record.id == target.entity_id)
            .count(),
        WorkspaceType::Design => lifecycle
            .list_design_specs(project_id, issue_id)
            .map_err(PlanRepairError::Store)?
            .into_iter()
            .filter(|record| record.id == target.entity_id)
            .count(),
        WorkspaceType::WorkItem | WorkspaceType::WorkItemPlan => 0,
    };
    if matches != 1 {
        return Err(identity_mismatch(
            "linked_workspace_target",
            &target.entity_id,
        ));
    }
    Ok(())
}

fn linked_amendment_ids(
    parent_session_id: &str,
    relation: &WorkspaceSessionRelation,
    entity_id: &str,
    amendment_id: &str,
) -> (String, String) {
    let identity_hash = compute_sha256(
        format!(
            "{parent_session_id}\n{}\n{entity_id}\n{amendment_id}",
            relation_label(relation)
        )
        .as_bytes(),
    );
    (
        format!(
            "workspace_session_link_{}_{}",
            relation_label(relation),
            identity_hash
        ),
        format!(
            "workspace_session_{}_{}",
            relation_label(relation),
            identity_hash
        ),
    )
}

fn validate_target_child(
    engine: &WorkspaceEngine,
    target: &LinkedWorkspaceAmendmentTarget,
    child: &WorkspaceSessionRecord,
    child_session_id: &str,
) -> Result<(), PlanRepairError> {
    if child.id != child_session_id
        || child.project_id != engine.session.project_id
        || child.issue_id != engine.session.issue_id
        || child.entity_id != target.entity_id
        || child.workspace_type != target.workspace_type
    {
        return Err(identity_mismatch("workspace_session", &child.id));
    }
    Ok(())
}

fn validate_complete_link_identity(link: &WorkspaceSessionLink) -> Result<(), PlanRepairError> {
    let required = [
        link.id.as_str(),
        link.parent_session_id.as_str(),
        link.child_session_id.as_str(),
        link.trigger.attempt_id.as_str(),
        link.trigger.unit_run_id.as_str(),
        link.trigger.finding_id.as_str(),
        link.trigger.repair_request_id.as_str(),
        link.trigger.amendment_id.as_str(),
        link.trigger.fingerprint.as_str(),
        link.trigger.base_plan_revision_id.as_str(),
        link.return_context.original_attempt_id.as_str(),
        link.return_context.original_unit_run_id.as_str(),
        link.return_context.timeline_anchor_id.as_str(),
        link.return_context.original_route.as_str(),
    ];
    if required.iter().any(|value| value.trim().is_empty()) {
        return Err(identity_mismatch("workspace_session_link", &link.id));
    }
    Ok(())
}

fn relation_matches_workspace(
    relation: &WorkspaceSessionRelation,
    workspace_type: &WorkspaceType,
) -> bool {
    matches!(
        (relation, workspace_type),
        (
            WorkspaceSessionRelation::StoryAmendment,
            WorkspaceType::Story
        ) | (
            WorkspaceSessionRelation::DesignAmendment,
            WorkspaceType::Design
        ) | (
            WorkspaceSessionRelation::PlanRepair,
            WorkspaceType::WorkItem | WorkspaceType::WorkItemPlan
        )
    )
}

fn relation_label(relation: &WorkspaceSessionRelation) -> &'static str {
    match relation {
        WorkspaceSessionRelation::PlanRepair => "plan_repair",
        WorkspaceSessionRelation::StoryAmendment => "story_amendment",
        WorkspaceSessionRelation::DesignAmendment => "design_amendment",
    }
}

fn identity_mismatch(kind: &'static str, id: &str) -> PlanRepairError {
    PlanRepairError::Store(ProductStoreError::IdentityMismatch {
        kind,
        id: id.to_string(),
    })
}

fn invalid(message: impl Into<String>) -> PlanRepairError {
    PlanRepairError::InvalidRepairTarget(message.into())
}
