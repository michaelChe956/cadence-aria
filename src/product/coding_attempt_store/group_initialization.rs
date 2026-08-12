use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::coding_models::{
    CodingAttemptPlanBinding, CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt,
    CodingExecutionStage, CodingExecutionUnit, CodingExecutionUnitStatus,
    CodingRoleProviderConfigSnapshot,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

use super::group_validation::AuthoritativeCodingUnitBinding;
use super::locking::ExclusiveFileLock;
use super::{
    CreateGroupCodingAttemptInput, WorkItemAttemptCreationGuard, incomplete_group_attempt,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingGroupInitializationPhase {
    Prepared,
    AttemptPersisted,
    WorktreeBound,
    PlanBindingSaved,
    UnitsMaterialized,
    Completed,
}

impl CodingGroupInitializationPhase {
    fn order(self) -> u8 {
        match self {
            Self::Prepared => 0,
            Self::AttemptPersisted => 1,
            Self::WorktreeBound => 2,
            Self::PlanBindingSaved => 3,
            Self::UnitsMaterialized => 4,
            Self::Completed => 5,
        }
    }

    pub fn has_reached(self, phase: Self) -> bool {
        self.order() >= phase.order()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingGroupInitializationJournal {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub lock_work_item_id: String,
    pub worktree_lease_id: String,
    pub attempt: CodingExecutionAttempt,
    pub provider_config: CodingRoleProviderConfigSnapshot,
    pub plan_binding: CodingAttemptPlanBinding,
    pub units: Vec<CodingExecutionUnit>,
    pub phase: CodingGroupInitializationPhase,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct CodingGroupInitializationGuard {
    _lock: ExclusiveFileLock,
}

impl super::CodingAttemptStore {
    pub fn acquire_group_initialization_arbitration(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<CodingGroupInitializationGuard, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        Ok(CodingGroupInitializationGuard {
            _lock: ExclusiveFileLock::acquire(
                &self.group_initialization_arbitration_path(project_id, issue_id),
            )?,
        })
    }

    pub async fn acquire_group_initialization_arbitration_async(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<CodingGroupInitializationGuard, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        Ok(CodingGroupInitializationGuard {
            _lock: ExclusiveFileLock::acquire_async(
                &self.group_initialization_arbitration_path(project_id, issue_id),
            )
            .await?,
        })
    }

    pub fn prepare_group_initialization(
        &self,
        input: &CreateGroupCodingAttemptInput,
        bound_plan_revision_id: &str,
        unit_bindings: &[AuthoritativeCodingUnitBinding],
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        validate_group_initialization_input(input, bound_plan_revision_id, unit_bindings)?;
        let path = self.group_initialization_journal_path(
            &input.project_id,
            &input.issue_id,
            &input.plan_id,
        );
        if super::path_is_regular_file(&path)? {
            let journal: CodingGroupInitializationJournal = read_json(&path)?;
            validate_group_initialization_journal(&journal)?;
            if journal_matches_request(&journal, input, bound_plan_revision_id, unit_bindings) {
                return Ok(journal);
            }
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "initialization journal identity differs from the authoritative request",
            ));
        }

        if let Some(existing) = self.get_attempt_for_work_item_group(
            &input.project_id,
            &input.issue_id,
            &input.plan_id,
        )? {
            return Err(incomplete_group_attempt(
                &existing.id,
                "group attempt exists without an initialization journal",
            ));
        }
        let existing_attempts: Vec<CodingExecutionAttempt> = super::list_json_records(
            &self.coding_attempts_root(&input.project_id, &input.issue_id),
        )?;
        if let Some(active) = existing_attempts
            .into_iter()
            .find(|attempt| attempt.status.is_active())
        {
            return Err(ProductStoreError::Io(format!(
                "active_coding_attempt_exists: {}",
                active.id
            )));
        }

        let journal =
            self.build_group_initialization_journal(input, bound_plan_revision_id, unit_bindings)?;
        write_json(&path, &journal)?;
        Ok(journal)
    }

    pub fn get_group_initialization(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        for id in [project_id, issue_id, plan_id] {
            validate_relative_id(id)?;
        }
        let path = self.group_initialization_journal_path(project_id, issue_id, plan_id);
        if !super::path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "coding_group_initialization_journal",
                id: plan_id.to_string(),
            });
        }
        let journal: CodingGroupInitializationJournal = read_json(&path)?;
        validate_group_initialization_journal(&journal)?;
        if journal.project_id != project_id
            || journal.issue_id != issue_id
            || journal.plan_id != plan_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_group_initialization_journal",
                id: plan_id.to_string(),
            });
        }
        Ok(journal)
    }

    pub fn ensure_group_initialization_attempt(
        &self,
        journal: &CodingGroupInitializationJournal,
        guard: &WorkItemAttemptCreationGuard,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        guard.validate_identity(
            self,
            &journal.project_id,
            &journal.issue_id,
            &journal.lock_work_item_id,
        )?;
        let active_attempts = super::list_json_records::<CodingExecutionAttempt>(
            &self.coding_attempts_root(&journal.project_id, &journal.issue_id),
        )?
        .into_iter()
        .filter(|attempt| attempt.status.is_active())
        .collect::<Vec<_>>();
        let attempt_path =
            self.attempt_path(&journal.project_id, &journal.issue_id, &journal.attempt.id);
        if super::path_is_regular_file(&attempt_path)? {
            let existing: CodingExecutionAttempt = read_json(&attempt_path)?;
            if existing != journal.attempt {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    "persisted attempt differs from initialization journal",
                ));
            }
            if active_attempts.len() != 1 || active_attempts[0].id != journal.attempt.id {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    "another active attempt exists during initialization replay",
                ));
            }
        } else {
            if let Some(active) = active_attempts.first() {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    &format!(
                        "active attempt {} differs from initialization journal",
                        active.id
                    ),
                ));
            }
            write_json(&attempt_path, &journal.attempt)?;
        }

        let provider_path = self.role_provider_config_path(
            &journal.project_id,
            &journal.issue_id,
            &journal.attempt.id,
        );
        if super::path_is_regular_file(&provider_path)? {
            let existing: CodingRoleProviderConfigSnapshot = read_json(&provider_path)?;
            if existing != journal.provider_config {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    "provider config differs from initialization journal",
                ));
            }
        } else {
            write_json(&provider_path, &journal.provider_config)?;
        }
        Ok(journal.attempt.clone())
    }

    pub fn validate_materialized_group_initialization_attempt(
        &self,
        journal: &CodingGroupInitializationJournal,
        guard: &WorkItemAttemptCreationGuard,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        guard.validate_identity(
            self,
            &journal.project_id,
            &journal.issue_id,
            &journal.lock_work_item_id,
        )?;
        let attempt_path =
            self.attempt_path(&journal.project_id, &journal.issue_id, &journal.attempt.id);
        if !super::path_is_regular_file(&attempt_path)? {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "persisted attempt is missing during bound replay",
            ));
        }
        let persisted_attempt: CodingExecutionAttempt = read_json(&attempt_path)?;
        if persisted_attempt != journal.attempt {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "persisted attempt differs from initialization journal",
            ));
        }
        let active_attempts = super::list_json_records::<CodingExecutionAttempt>(
            &self.coding_attempts_root(&journal.project_id, &journal.issue_id),
        )?
        .into_iter()
        .filter(|attempt| attempt.status.is_active())
        .collect::<Vec<_>>();
        if active_attempts.len() != 1 || active_attempts[0].id != journal.attempt.id {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "another active attempt exists during bound replay",
            ));
        }
        let provider_path = self.role_provider_config_path(
            &journal.project_id,
            &journal.issue_id,
            &journal.attempt.id,
        );
        if !super::path_is_regular_file(&provider_path)? {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "provider config is missing during bound replay",
            ));
        }
        let provider_config: CodingRoleProviderConfigSnapshot = read_json(&provider_path)?;
        if provider_config != journal.provider_config {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "provider config differs from initialization journal",
            ));
        }
        Ok(persisted_attempt)
    }

    pub fn ensure_group_initialization_plan_binding(
        &self,
        journal: &CodingGroupInitializationJournal,
    ) -> Result<(), ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        self.save_plan_binding(&journal.attempt, &journal.plan_binding)
    }

    pub fn ensure_group_initialization_unit(
        &self,
        journal: &CodingGroupInitializationJournal,
        index: usize,
    ) -> Result<CodingExecutionUnit, ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        let expected =
            journal
                .units
                .get(index)
                .ok_or_else(|| ProductStoreError::IdentityMismatch {
                    kind: "coding_group_initialization_unit",
                    id: index.to_string(),
                })?;
        let materialized =
            self.list_coding_units(&journal.project_id, &journal.issue_id, &journal.attempt.id)?;
        if materialized.len() > journal.units.len()
            || materialized
                .iter()
                .zip(journal.units.iter())
                .any(|(existing, expected)| existing != expected)
        {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "materialized units differ from initialization journal",
            ));
        }
        if let Some(existing) = materialized.get(index) {
            return Ok(existing.clone());
        }
        if materialized.len() != index {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "materialized units are not a contiguous journal prefix",
            ));
        }
        write_json(
            &self.coding_unit_path(
                &expected.project_id,
                &expected.issue_id,
                &expected.attempt_id,
                &expected.id,
            ),
            expected,
        )?;
        Ok(expected.clone())
    }

    pub fn advance_group_initialization_phase(
        &self,
        expected: &CodingGroupInitializationJournal,
        next: CodingGroupInitializationPhase,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        validate_group_initialization_journal(expected)?;
        let path = self.group_initialization_journal_path(
            &expected.project_id,
            &expected.issue_id,
            &expected.plan_id,
        );
        let mut current: CodingGroupInitializationJournal = read_json(&path)?;
        validate_group_initialization_journal(&current)?;
        if !same_group_initialization_identity(&current, expected) {
            return Err(incomplete_group_attempt(
                &expected.attempt.id,
                "initialization journal changed during replay",
            ));
        }
        if current.phase.order() >= next.order() {
            return Ok(current);
        }
        if next.order() != current.phase.order() + 1 {
            return Err(incomplete_group_attempt(
                &expected.attempt.id,
                "initialization phase advance is not contiguous",
            ));
        }
        current.phase = next;
        current.error = None;
        current.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &current)?;
        Ok(current)
    }

    pub(crate) fn delete_group_initialization_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
            return Ok(());
        };
        let path =
            self.group_initialization_journal_path(&attempt.project_id, &attempt.issue_id, plan_id);
        if !super::path_is_regular_file(&path)? {
            return Ok(());
        }
        let journal: CodingGroupInitializationJournal = read_json(&path)?;
        validate_group_initialization_journal(&journal)?;
        if journal.attempt.id != attempt.id {
            return Err(incomplete_group_attempt(
                &attempt.id,
                "delete target differs from initialization journal",
            ));
        }
        super::remove_file_if_exists(&path)
    }

    fn build_group_initialization_journal(
        &self,
        input: &CreateGroupCodingAttemptInput,
        bound_plan_revision_id: &str,
        unit_bindings: &[AuthoritativeCodingUnitBinding],
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        let id = self.allocate_coding_attempt_id();
        let attempt_no = self
            .list_attempts_for_work_item(
                &input.project_id,
                &input.issue_id,
                &input.current_work_item_id,
            )?
            .iter()
            .map(|attempt| attempt.attempt_no)
            .max()
            .unwrap_or(0)
            + 1;
        let now = Utc::now().to_rfc3339();
        let first_unit_id = next_sequential_id("coding_unit", 0);
        let attempt = CodingExecutionAttempt {
            id: id.clone(),
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            work_item_id: input.current_work_item_id.clone(),
            attempt_no,
            scope: CodingAttemptScope::WorkItemGroup,
            status: CodingAttemptStatus::Created,
            stage: CodingExecutionStage::PrepareContext,
            base_branch: input.base_branch.clone(),
            branch_name: input.branch_name.clone(),
            worktree_path: input.worktree_path.clone(),
            provider_config_snapshot: input.provider_config_snapshot.clone(),
            rework_count: 0,
            max_auto_rework: input.max_auto_rework,
            work_item_group_id: Some(input.plan_id.clone()),
            current_work_item_id: Some(input.current_work_item_id.clone()),
            active_unit_id: Some(first_unit_id),
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: now.clone(),
            updated_at: now.clone(),
            target_snapshot: input.target_snapshot.clone(),
            completed_at: None,
        };
        let units = unit_bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| CodingExecutionUnit {
                id: next_sequential_id("coding_unit", index),
                attempt_id: id.clone(),
                project_id: input.project_id.clone(),
                issue_id: input.issue_id.clone(),
                plan_id: input.plan_id.clone(),
                logical_work_item_id: binding.logical_work_item_id.clone(),
                work_item_revision_id: binding.work_item_revision_id.clone(),
                dependency_logical_work_item_ids: binding.dependency_logical_work_item_ids.clone(),
                order_index: index as u32,
                status: if index == 0 {
                    CodingExecutionUnitStatus::Running
                } else {
                    CodingExecutionUnitStatus::Pending
                },
                started_at: (index == 0).then(|| now.clone()),
                completed_at: None,
                latest_handoff_revision_id: None,
                completion_commit: None,
                summary: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .collect::<Vec<_>>();
        let journal = CodingGroupInitializationJournal {
            id: format!("coding_group_initialization_{}", input.plan_id),
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            plan_id: input.plan_id.clone(),
            lock_work_item_id: input.current_work_item_id.clone(),
            worktree_lease_id: format!("issue_worktree_lease_{}", uuid::Uuid::new_v4().simple()),
            provider_config: CodingRoleProviderConfigSnapshot::from(
                &input.provider_config_snapshot,
            ),
            plan_binding: CodingAttemptPlanBinding {
                attempt_id: id,
                plan_id: input.plan_id.clone(),
                bound_plan_revision_id: bound_plan_revision_id.to_string(),
                applied_amendment_ids: Vec::new(),
                updated_at: now.clone(),
            },
            attempt,
            units,
            phase: CodingGroupInitializationPhase::Prepared,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        };
        validate_group_initialization_journal(&journal)?;
        Ok(journal)
    }
}

fn validate_group_initialization_input(
    input: &CreateGroupCodingAttemptInput,
    bound_plan_revision_id: &str,
    unit_bindings: &[AuthoritativeCodingUnitBinding],
) -> Result<(), ProductStoreError> {
    for id in [
        input.project_id.as_str(),
        input.issue_id.as_str(),
        input.plan_id.as_str(),
        input.current_work_item_id.as_str(),
        bound_plan_revision_id,
    ] {
        validate_relative_id(id)?;
    }
    super::validate_max_auto_rework(input.max_auto_rework)?;
    if unit_bindings.is_empty()
        || unit_bindings[0].logical_work_item_id != input.current_work_item_id
    {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "coding_group_initialization",
            id: input.plan_id.clone(),
        });
    }
    for binding in unit_bindings {
        validate_relative_id(&binding.logical_work_item_id)?;
        validate_relative_id(&binding.work_item_revision_id)?;
        validate_relative_id(&binding.verification_plan_revision_id)?;
        validate_relative_id(&binding.projection_bundle_id)?;
        for dependency_id in &binding.dependency_logical_work_item_ids {
            validate_relative_id(dependency_id)?;
        }
    }
    Ok(())
}

fn validate_group_initialization_journal(
    journal: &CodingGroupInitializationJournal,
) -> Result<(), ProductStoreError> {
    for id in [
        journal.id.as_str(),
        journal.project_id.as_str(),
        journal.issue_id.as_str(),
        journal.plan_id.as_str(),
        journal.lock_work_item_id.as_str(),
        journal.worktree_lease_id.as_str(),
        journal.attempt.id.as_str(),
        journal.plan_binding.bound_plan_revision_id.as_str(),
    ] {
        validate_relative_id(id)?;
    }
    let first = journal.units.first().ok_or_else(|| {
        incomplete_group_attempt(&journal.attempt.id, "initialization journal has no units")
    })?;
    let identity_is_valid = journal.attempt.project_id == journal.project_id
        && journal.attempt.issue_id == journal.issue_id
        && journal.attempt.scope == CodingAttemptScope::WorkItemGroup
        && journal.attempt.work_item_group_id.as_deref() == Some(journal.plan_id.as_str())
        && journal.attempt.work_item_id == journal.lock_work_item_id
        && journal.attempt.current_work_item_id.as_deref()
            == Some(journal.lock_work_item_id.as_str())
        && journal.attempt.active_unit_id.as_deref() == Some(first.id.as_str())
        && journal.provider_config
            == CodingRoleProviderConfigSnapshot::from(&journal.attempt.provider_config_snapshot)
        && journal.plan_binding.attempt_id == journal.attempt.id
        && journal.plan_binding.plan_id == journal.plan_id
        && journal.units.iter().enumerate().all(|(index, unit)| {
            unit.id == next_sequential_id("coding_unit", index)
                && unit.attempt_id == journal.attempt.id
                && unit.project_id == journal.project_id
                && unit.issue_id == journal.issue_id
                && unit.plan_id == journal.plan_id
                && unit.order_index == index as u32
                && unit.status
                    == if index == 0 {
                        CodingExecutionUnitStatus::Running
                    } else {
                        CodingExecutionUnitStatus::Pending
                    }
        });
    if !identity_is_valid {
        return Err(incomplete_group_attempt(
            &journal.attempt.id,
            "initialization journal identity is invalid",
        ));
    }
    Ok(())
}

fn journal_matches_request(
    journal: &CodingGroupInitializationJournal,
    input: &CreateGroupCodingAttemptInput,
    bound_plan_revision_id: &str,
    unit_bindings: &[AuthoritativeCodingUnitBinding],
) -> bool {
    journal.project_id == input.project_id
        && journal.issue_id == input.issue_id
        && journal.plan_id == input.plan_id
        && journal.lock_work_item_id == input.current_work_item_id
        && journal.attempt.base_branch == input.base_branch
        && journal.attempt.branch_name == input.branch_name
        && journal.attempt.worktree_path == input.worktree_path
        && journal.attempt.provider_config_snapshot == input.provider_config_snapshot
        && journal.attempt.target_snapshot == input.target_snapshot
        && journal.attempt.max_auto_rework == input.max_auto_rework
        && journal.plan_binding.bound_plan_revision_id == bound_plan_revision_id
        && journal.units.len() == unit_bindings.len()
        && journal
            .units
            .iter()
            .zip(unit_bindings.iter())
            .all(|(unit, binding)| {
                unit.logical_work_item_id == binding.logical_work_item_id
                    && unit.work_item_revision_id == binding.work_item_revision_id
                    && unit.dependency_logical_work_item_ids
                        == binding.dependency_logical_work_item_ids
            })
}

fn same_group_initialization_identity(
    left: &CodingGroupInitializationJournal,
    right: &CodingGroupInitializationJournal,
) -> bool {
    left.id == right.id
        && left.project_id == right.project_id
        && left.issue_id == right.issue_id
        && left.plan_id == right.plan_id
        && left.lock_work_item_id == right.lock_work_item_id
        && left.worktree_lease_id == right.worktree_lease_id
        && left.attempt == right.attempt
        && left.provider_config == right.provider_config
        && left.plan_binding == right.plan_binding
        && left.units == right.units
        && left.created_at == right.created_at
}
