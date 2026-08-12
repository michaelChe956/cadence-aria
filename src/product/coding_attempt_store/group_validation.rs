use std::collections::{BTreeMap, BTreeSet};

use crate::product::coding_attempt_store::admission::{MIXED_TARGET_GROUP_REJECTED, StableCode};
use crate::product::coding_models::{
    AttemptTargetSnapshot, CodingAttemptScope, CodingExecutionAttempt, CodingExecutionStage,
    CodingExecutionUnit, CodingExecutionUnitStatus,
};
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryRouting};
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoritativeCodingUnitBinding {
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub verification_plan_revision_id: String,
    pub projection_bundle_id: String,
    /// `None` 仅表示 source draft 已校验存在且 target 未指定；回溯断链另存错误，
    /// 防止被 group B2 误判为可 focus 兜底的零 target。
    pub target_repository_id: Option<LogicalRepositoryId>,
    pub source_draft_error: Option<String>,
    pub dependency_logical_work_item_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoritativeGroupPlanBinding {
    pub plan_revision_id: String,
    pub dependency_graph_revision_id: String,
    pub plan_projection_bundle_id: String,
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

        let revision_store = WorkItemRevisionStore::new(self.paths());
        let lineage = revision_store.get_plan_lineage(project_id, issue_id, plan_id)?;
        let plan_revision_id = lineage
            .active_revision_id
            .clone()
            .ok_or_else(|| invalid_plan_binding("active plan revision is missing"))?;
        self.resolve_authoritative_group_plan_binding_for_revision(
            project_id,
            issue_id,
            plan_id,
            &plan_revision_id,
        )
    }

    pub fn resolve_authoritative_group_plan_binding_for_revision(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> Result<AuthoritativeGroupPlanBinding, ProductStoreError> {
        for id in [project_id, issue_id, plan_id, plan_revision_id] {
            validate_relative_id(id)?;
        }

        let revision_store = WorkItemRevisionStore::new(self.paths());
        let lineage = revision_store.get_plan_lineage(project_id, issue_id, plan_id)?;
        let revision =
            revision_store.get_plan_revision(project_id, issue_id, plan_id, plan_revision_id)?;
        if revision.plan_id != plan_id {
            return Err(invalid_plan_binding(
                "bound plan revision does not belong to the coding group",
            ));
        }
        let plan_projection = revision_store
            .get_plan_projection_bundle(&lineage, &revision.plan_projection_bundle_id)?;
        if plan_projection.plan_revision_id != revision.id
            || plan_projection.dependency_graph_revision_id != revision.dependency_graph_revision_id
            || plan_projection.coder_group_context.plan_id != plan_id
        {
            return Err(invalid_plan_binding(
                "plan projection bundle does not match the bound plan revision",
            ));
        }
        let ordered_ids = plan_projection
            .coder_group_context
            .ordered_logical_work_item_ids
            .clone();
        let expected_ids = ordered_ids.iter().cloned().collect::<BTreeSet<_>>();
        if ordered_ids.is_empty() || expected_ids.len() != ordered_ids.len() {
            return Err(invalid_plan_binding(
                "plan projection order is missing or contains duplicate work items",
            ));
        }
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
        if plan_projection.coder_group_context.dependency_edges != graph.edges
            || plan_projection.reviewer_group_matrix.dependency_edges != graph.edges
        {
            return Err(invalid_plan_binding(
                "plan projection dependencies do not match the bound dependency graph",
            ));
        }
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

        let mut draft_targets = BTreeMap::new();
        let mut ambiguous_draft_ids = BTreeSet::new();
        for draft in WorkItemPlanStore::new(self.paths())
            .list_draft_records(project_id, issue_id, plan_id)?
        {
            if draft_targets.contains_key(&draft.draft_id) {
                ambiguous_draft_ids.insert(draft.draft_id);
            } else {
                draft_targets.insert(draft.draft_id.clone(), draft);
            }
        }
        let mut units = Vec::with_capacity(ordered_ids.len());
        let mut projection_bundle_ids = BTreeSet::new();
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
            let verification = revision_store.get_verification_plan_revision(
                &lineage,
                &work_item_revision.verification_plan_revision_id,
            )?;
            let projection = revision_store.get_work_item_projection_bundle(
                &lineage,
                &work_item_revision.work_item_projection_bundle_id,
            )?;
            if verification.logical_work_item_id != logical_id
                || projection.work_item_revision_id != work_item_revision.id
                || projection.canonical_contract_hash != work_item_revision.canonical_contract_hash
            {
                return Err(invalid_plan_binding(
                    "work item revision dependencies do not match the bound logical work item",
                ));
            }
            let (target_repository_id, source_draft_error) =
                if ambiguous_draft_ids.contains(&work_item_revision.source_draft_revision_id) {
                    (
                        None,
                        Some("work item revision source draft is ambiguous".to_string()),
                    )
                } else {
                    match draft_targets.get(&work_item_revision.source_draft_revision_id) {
                        None => (
                            None,
                            Some("work item revision source draft is missing".to_string()),
                        ),
                        Some(source_draft)
                            if source_draft.candidate.logical_work_item_id != logical_id => (
                            None,
                            Some(
                                "work item revision source draft logical work item does not match"
                                    .to_string(),
                            ),
                        ),
                        Some(source_draft) => (source_draft.candidate.target_repository_id, None),
                    }
                };
            projection_bundle_ids.insert(projection.id.clone());
            units.push(AuthoritativeCodingUnitBinding {
                logical_work_item_id: logical_id.clone(),
                work_item_revision_id: revision_id,
                verification_plan_revision_id: verification.id,
                projection_bundle_id: projection.id,
                target_repository_id,
                source_draft_error,
                dependency_logical_work_item_ids: dependencies
                    .remove(&logical_id)
                    .expect("known logical work item"),
            });
        }
        let projection_bundle_refs = plan_projection
            .work_item_projection_bundle_refs
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if projection_bundle_refs.len() != plan_projection.work_item_projection_bundle_refs.len()
            || projection_bundle_refs != projection_bundle_ids
        {
            return Err(invalid_plan_binding(
                "plan projection bundle refs do not match bound work item projections",
            ));
        }

        Ok(AuthoritativeGroupPlanBinding {
            plan_revision_id: plan_revision_id.to_string(),
            dependency_graph_revision_id: revision.dependency_graph_revision_id,
            plan_projection_bundle_id: revision.plan_projection_bundle_id,
            units,
        })
    }

    pub fn validate_group_attempt_integrity(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<AuthoritativeGroupPlanBinding, ProductStoreError> {
        let (stored, authoritative, units) = self.validate_group_attempt_structure(attempt)?;
        let routing =
            RepositoryRouting::load_for_issue(&self.paths, &stored.project_id, &stored.issue_id)?;
        validate_group_single_target(
            &routing,
            &authoritative.units,
            stored.target_snapshot.as_ref(),
        )
        .map_err(|_| mixed_target_group_rejected())?;
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
        let binding = self
            .get_plan_binding(&stored)
            .map_err(|error| map_group_integrity_dependency_error(&stored.id, error))?;
        if binding.plan_id != plan_id {
            return Err(incomplete_group_attempt(
                &stored.id,
                "attempt plan binding targets another coding group",
            ));
        }
        let authoritative = self
            .resolve_authoritative_group_plan_binding_for_revision(
                &stored.project_id,
                &stored.issue_id,
                plan_id,
                &binding.bound_plan_revision_id,
            )
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

/// REQ-COD-04：routing-aware 的 group 单目标校验（创建/恢复/replay 三处调同一函数）。
///
/// 堵 v1.0 的 `Some(A)+None` 绕过（`filter_map` 让 None 消失）与 snapshot 漂移：
/// - Legacy：所有 unit target 必须全 `None`，出现任何 `Some` → 拒。
/// - Logical：所有 unit target 必须全 `Some` 且互相同，且与 attempt snapshot 的
///   `logical_repository_id` 一致 → 通过；任何 `None` / ≥2 不同 `Some` / 与 snapshot
///   不一致 → 拒。
/// - 拒绝任何 `source_draft_error`（draft 溯源已出错，不应进 group）。
///
/// `RepositoryRouting::FailClosed` 是路由失败态（非 mixed-target），由调用方在进入本
/// 函数前处理，此处不重复判定。
pub fn validate_group_single_target(
    routing: &RepositoryRouting,
    unit_bindings: &[AuthoritativeCodingUnitBinding],
    attempt_snapshot: Option<&AttemptTargetSnapshot>,
) -> Result<(), StableCode> {
    if unit_bindings
        .iter()
        .any(|unit| unit.source_draft_error.is_some())
    {
        return Err(StableCode::MixedTargetGroupRejected);
    }
    match routing {
        RepositoryRouting::Legacy { .. } => {
            if unit_bindings
                .iter()
                .any(|unit| unit.target_repository_id.is_some())
            {
                return Err(StableCode::MixedTargetGroupRejected);
            }
            Ok(())
        }
        RepositoryRouting::Logical { .. } => {
            let Some(first) = unit_bindings
                .first()
                .and_then(|unit| unit.target_repository_id)
            else {
                return Err(StableCode::MixedTargetGroupRejected);
            };
            if unit_bindings
                .iter()
                .any(|unit| unit.target_repository_id != Some(first))
            {
                return Err(StableCode::MixedTargetGroupRejected);
            }
            match attempt_snapshot {
                Some(snapshot) if snapshot.logical_repository_id == first => Ok(()),
                _ => Err(StableCode::MixedTargetGroupRejected),
            }
        }
        RepositoryRouting::FailClosed { .. } => Ok(()),
    }
}

pub(crate) fn mixed_target_group_rejected() -> ProductStoreError {
    ProductStoreError::Io(MIXED_TARGET_GROUP_REJECTED.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::coding_models::AttemptTargetSnapshot;
    use crate::product::logical_codebase::RepositoryRouting;
    use crate::product::logical_codebase::issue_selection::IssueCodebaseSelection;
    use crate::product::logical_codebase::store::LogicalCodebaseManifest;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn logical_id(u: u128) -> LogicalRepositoryId {
        LogicalRepositoryId(Uuid::from_u128(u))
    }

    fn unit(
        logical_work_item_id: &str,
        target: Option<LogicalRepositoryId>,
    ) -> AuthoritativeCodingUnitBinding {
        unit_with_draft_error(logical_work_item_id, target, None)
    }

    fn unit_with_draft_error(
        logical_work_item_id: &str,
        target: Option<LogicalRepositoryId>,
        source_draft_error: Option<&str>,
    ) -> AuthoritativeCodingUnitBinding {
        AuthoritativeCodingUnitBinding {
            logical_work_item_id: logical_work_item_id.to_string(),
            work_item_revision_id: format!("{logical_work_item_id}_rev"),
            verification_plan_revision_id: format!("{logical_work_item_id}_verification"),
            projection_bundle_id: format!("{logical_work_item_id}_projection"),
            target_repository_id: target,
            source_draft_error: source_draft_error.map(str::to_string),
            dependency_logical_work_item_ids: Vec::new(),
        }
    }

    fn snapshot(logical_repository_id: LogicalRepositoryId) -> AttemptTargetSnapshot {
        AttemptTargetSnapshot {
            logical_repository_id,
            checkout_id: crate::product::logical_codebase::RepositoryCheckoutId(Uuid::from_u128(
                0xaaaa,
            )),
            physical_repository_id: "physical_0001".to_string(),
            canonical_path: PathBuf::from("/tmp/repo"),
            git_dir_identity: "sha256:git".to_string(),
            revision: Some("deadbeef".to_string()),
            policy_digest: "policy_digest".to_string(),
            membership_revision: 1,
            captured_at: "2026-08-11T00:00:00Z".to_string(),
            capture_source: "test".to_string(),
        }
    }

    fn legacy_routing() -> RepositoryRouting {
        RepositoryRouting::Legacy {
            repository_id: "repository_0001".to_string(),
        }
    }

    fn logical_routing(member_ids: Vec<LogicalRepositoryId>) -> RepositoryRouting {
        RepositoryRouting::Logical {
            manifest: LogicalCodebaseManifest::new(
                "project_0001",
                PathBuf::from("/tmp/logical"),
                member_ids,
            ),
            selection: Box::new(IssueCodebaseSelection::all_members(
                "project_0001",
                "issue_0001",
                None,
            )),
        }
    }

    #[test]
    fn legacy_all_none_is_accepted() {
        let units = vec![unit("work_item_0001", None), unit("work_item_0002", None)];
        assert_eq!(
            validate_group_single_target(&legacy_routing(), &units, None),
            Ok(())
        );
    }

    #[test]
    fn legacy_any_some_is_rejected() {
        let units = vec![
            unit("work_item_0001", None),
            unit("work_item_0002", Some(logical_id(0x0001))),
        ];
        assert_eq!(
            validate_group_single_target(&legacy_routing(), &units, None),
            Err(StableCode::MixedTargetGroupRejected)
        );
    }

    #[test]
    fn logical_all_same_some_matching_snapshot_is_accepted() {
        let target = logical_id(0x0001);
        let units = vec![
            unit("work_item_0001", Some(target)),
            unit("work_item_0002", Some(target)),
        ];
        assert_eq!(
            validate_group_single_target(
                &logical_routing(vec![target]),
                &units,
                Some(&snapshot(target)),
            ),
            Ok(())
        );
    }

    #[test]
    fn logical_any_none_is_rejected() {
        let target = logical_id(0x0001);
        let units = vec![
            unit("work_item_0001", Some(target)),
            unit("work_item_0002", None),
        ];
        assert_eq!(
            validate_group_single_target(
                &logical_routing(vec![target]),
                &units,
                Some(&snapshot(target)),
            ),
            Err(StableCode::MixedTargetGroupRejected)
        );
    }

    #[test]
    fn logical_distinct_some_targets_are_rejected() {
        let target_one = logical_id(0x0001);
        let target_two = logical_id(0x0002);
        let units = vec![
            unit("work_item_0001", Some(target_one)),
            unit("work_item_0002", Some(target_two)),
        ];
        assert_eq!(
            validate_group_single_target(
                &logical_routing(vec![target_one, target_two]),
                &units,
                Some(&snapshot(target_one)),
            ),
            Err(StableCode::MixedTargetGroupRejected)
        );
    }

    #[test]
    fn logical_snapshot_mismatch_is_rejected() {
        let target = logical_id(0x0001);
        let other = logical_id(0x0002);
        let units = vec![
            unit("work_item_0001", Some(target)),
            unit("work_item_0002", Some(target)),
        ];
        assert_eq!(
            validate_group_single_target(
                &logical_routing(vec![target, other]),
                &units,
                Some(&snapshot(other)),
            ),
            Err(StableCode::MixedTargetGroupRejected)
        );
    }

    #[test]
    fn logical_missing_snapshot_is_rejected() {
        let target = logical_id(0x0001);
        let units = vec![
            unit("work_item_0001", Some(target)),
            unit("work_item_0002", Some(target)),
        ];
        assert_eq!(
            validate_group_single_target(&logical_routing(vec![target]), &units, None),
            Err(StableCode::MixedTargetGroupRejected)
        );
    }

    #[test]
    fn any_source_draft_error_is_rejected() {
        let target = logical_id(0x0001);
        let units = vec![
            unit("work_item_0001", Some(target)),
            unit_with_draft_error(
                "work_item_0002",
                Some(target),
                Some("work item revision source draft is missing"),
            ),
        ];
        assert_eq!(
            validate_group_single_target(
                &logical_routing(vec![target]),
                &units,
                Some(&snapshot(target)),
            ),
            Err(StableCode::MixedTargetGroupRejected)
        );
    }
}
