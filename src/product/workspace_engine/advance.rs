use std::process::Command;

#[cfg(test)]
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AdvanceInitializationFailpoint {
    RecordPersisted,
    JournalPrepared,
    GroupAttemptPersisted,
    AttemptPersisted,
    WorktreeBound,
    PlanBindingSaved,
    UnitsMaterialized,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdvanceInitializationFailpointAction {
    Crash,
    Error,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdvanceInitializationFailpointMode {
    Crash,
    Error,
}

#[cfg(test)]
fn maybe_fail_advance_initialization(
    input: &AdvanceInput,
    checkpoint: AdvanceInitializationFailpoint,
) -> Result<(), String> {
    let key = AdvanceInitializationFailpointKey {
        project_id: input.project_id.clone(),
        issue_id: input.issue_id.clone(),
        plan_id: input.plan_id.clone(),
        command_id: input.command_id.clone(),
        checkpoint,
    };
    let action = advance_initialization_failpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&key)
        .map(|(_, action)| action);
    match action {
        Some(AdvanceInitializationFailpointAction::Crash) => {
            panic!("advance_initialization_failpoint:{checkpoint:?}");
        }
        Some(AdvanceInitializationFailpointAction::Error) => {
            Err(format!("advance_initialization_failpoint:{checkpoint:?}"))
        }
        None => Ok(()),
    }
}

#[cfg(not(test))]
fn maybe_fail_advance_initialization(
    _input: &AdvanceInput,
    _checkpoint: AdvanceInitializationFailpoint,
) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
pub(crate) fn register_advance_initialization_failpoint(
    input: &AdvanceInput,
    checkpoint: AdvanceInitializationFailpoint,
    mode: AdvanceInitializationFailpointMode,
) -> AdvanceInitializationFailpointGuard {
    let key = AdvanceInitializationFailpointKey {
        project_id: input.project_id.clone(),
        issue_id: input.issue_id.clone(),
        plan_id: input.plan_id.clone(),
        command_id: input.command_id.clone(),
        checkpoint,
    };
    let registration_id =
        NEXT_ADVANCE_INITIALIZATION_FAILPOINT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let action = match mode {
        AdvanceInitializationFailpointMode::Crash => AdvanceInitializationFailpointAction::Crash,
        AdvanceInitializationFailpointMode::Error => AdvanceInitializationFailpointAction::Error,
    };
    let previous = advance_initialization_failpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key.clone(), (registration_id, action));
    assert!(
        previous.is_none(),
        "advance initialization failpoint already registered"
    );
    AdvanceInitializationFailpointGuard {
        key,
        registration_id,
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AdvanceInitializationFailpointKey {
    project_id: String,
    issue_id: String,
    plan_id: String,
    command_id: String,
    checkpoint: AdvanceInitializationFailpoint,
}

#[cfg(test)]
pub(crate) struct AdvanceInitializationFailpointGuard {
    key: AdvanceInitializationFailpointKey,
    registration_id: u64,
}

#[cfg(test)]
static ADVANCE_INITIALIZATION_FAILPOINTS: OnceLock<
    Mutex<
        std::collections::HashMap<
            AdvanceInitializationFailpointKey,
            (u64, AdvanceInitializationFailpointAction),
        >,
    >,
> = OnceLock::new();

#[cfg(test)]
static NEXT_ADVANCE_INITIALIZATION_FAILPOINT_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

#[cfg(test)]
fn advance_initialization_failpoints() -> &'static Mutex<
    std::collections::HashMap<
        AdvanceInitializationFailpointKey,
        (u64, AdvanceInitializationFailpointAction),
    >,
> {
    ADVANCE_INITIALIZATION_FAILPOINTS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
impl Drop for AdvanceInitializationFailpointGuard {
    fn drop(&mut self) {
        let mut failpoints = advance_initialization_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failpoints
            .get(&self.key)
            .is_some_and(|(registration_id, _)| *registration_id == self.registration_id)
        {
            failpoints.remove(&self.key);
        }
    }
}

use crate::product::advance_store::AdvanceTargetAttemptBinding;
pub use crate::product::advance_store::{
    AdvanceInitializationPhase, AdvanceInput, AdvanceOutcome, AdvanceRecord, AdvanceStatus,
    AdvanceStore,
};
use crate::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot;
use crate::product::coding_attempt_store::{
    AuthoritativeGroupPlanBinding, CodingAttemptStore, CreateGroupCodingAttemptInput,
    UnitsByTarget, topologically_order_unit_bindings, units_by_target,
};
use crate::product::coding_models::CodingAdmissionKind;
use crate::product::issue_store::IssueStore;
use crate::product::logical_codebase::{RepositoryRouting, resolve_issue_logical_codebase_id};
use crate::product::models::{IssueWorkItemPlanStatus, WorkspaceType};
use crate::product::repository_store::RepositoryStore;
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

use super::types::WorkspaceEngine;

impl WorkspaceEngine {
    /// Runs only the first-request preflight. Record creation and group
    /// initialization belong to the following advance tasks; until then a valid
    /// request is deliberately rejected without durable side effects.
    pub async fn handle_advance(&mut self, input: AdvanceInput) -> Result<AdvanceOutcome, String> {
        if input.project_id != self.session.project_id
            || input.issue_id != self.session.issue_id
            || input.plan_id != self.session.entity_id
        {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_IDENTITY_MISMATCH".to_string(),
                reason: "advance identity does not match the workspace session".to_string(),
            });
        }

        let app_paths = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?
            .app_paths();
        let advance_store = AdvanceStore::new(app_paths.clone());

        // A completed/terminal record is an idempotent replay. An initializing
        // record must continue through the durable checkpoint path so a
        // restarted request resumes the same attempt rather than stopping at
        // the preflight facade.
        if let Some(record) = advance_store
            .get_advance_by_command_id(&input.project_id, &input.issue_id, &input.command_id)
            .map_err(|error| format!("load advance command record failed: {error}"))?
        {
            if record.plan_id != input.plan_id {
                return Ok(AdvanceOutcome::Rejected {
                    record: Some(record),
                    code: "ADVANCE_IDENTITY_MISMATCH".to_string(),
                    reason: "command_id is already bound to another plan".to_string(),
                });
            }
            if record.status != AdvanceStatus::Initializing {
                return Ok(AdvanceOutcome::Replayed { record });
            }
        }
        if let Some(record) = advance_store
            .get_advance_for_plan(&input.project_id, &input.issue_id, &input.plan_id)
            .map_err(|error| format!("load advance plan record failed: {error}"))?
            && record.status != AdvanceStatus::Initializing
        {
            return Ok(AdvanceOutcome::Replayed { record });
        }

        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .expect("lifecycle store checked above");
        let plan = match lifecycle.get_issue_work_item_plan(
            &input.project_id,
            &input.issue_id,
            &input.plan_id,
        ) {
            Ok(plan) => plan,
            Err(error) => {
                return Ok(AdvanceOutcome::Rejected {
                    record: None,
                    code: "ADVANCE_PLAN_NOT_FOUND".to_string(),
                    reason: format!("load confirmed work item plan failed: {error}"),
                });
            }
        };
        if plan.status != IssueWorkItemPlanStatus::Confirmed {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_PLAN_NOT_CONFIRMED".to_string(),
                reason: "work item plan must be durably confirmed before advance".to_string(),
            });
        }

        let revision_store = WorkItemRevisionStore::new(app_paths.clone());
        let lineage = match revision_store.get_plan_lineage(
            &input.project_id,
            &input.issue_id,
            &input.plan_id,
        ) {
            Ok(lineage) => lineage,
            Err(error) => {
                return Ok(AdvanceOutcome::Rejected {
                    record: None,
                    code: "ADVANCE_PLAN_REVISION_MISSING".to_string(),
                    reason: format!("load active plan revision failed: {error}"),
                });
            }
        };
        let Some(active_revision_id) = lineage.active_revision_id.as_deref() else {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_PLAN_REVISION_MISSING".to_string(),
                reason: "confirmed work item plan has no active plan revision".to_string(),
            });
        };
        if lineage.active_amendment_id.is_some() {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_ACTIVE_PLAN_REVISION".to_string(),
                reason: "work item plan has an active amendment/revision".to_string(),
            });
        }

        let plan_store = WorkItemPlanStore::new(app_paths.clone());
        let active_compile = plan_store
            .list_compile_transactions(&input.project_id, &input.issue_id, &input.plan_id)
            .map_err(|error| format!("load plan compile transactions failed: {error}"))?
            .into_iter()
            .find(|transaction| {
                matches!(
                    transaction.status,
                    crate::product::models::WorkItemPlanCompileStatus::Preparing
                        | crate::product::models::WorkItemPlanCompileStatus::Validating
                        | crate::product::models::WorkItemPlanCompileStatus::Committing
                        | crate::product::models::WorkItemPlanCompileStatus::RecoveryRequired
                )
            });
        if let Some(transaction) = active_compile {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_ACTIVE_PLAN_COMPILE".to_string(),
                reason: format!("plan compile {} is still active", transaction.compile_id),
            });
        }

        let child_sessions = lifecycle
            .list_workspace_sessions(&input.project_id, &input.issue_id)
            .map_err(|error| format!("list plan child sessions failed: {error}"))?;
        let missing_child = plan.work_item_ids.iter().find(|work_item_id| {
            !child_sessions.iter().any(|session| {
                session.workspace_type == WorkspaceType::WorkItem
                    && session.entity_id == **work_item_id
            })
        });
        if let Some(work_item_id) = missing_child {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_CHILD_SESSION_MISSING".to_string(),
                reason: format!("work item child session is missing: {work_item_id}"),
            });
        }

        let coding_store = CodingAttemptStore::new(app_paths.clone());

        let authoritative = coding_store
            .resolve_authoritative_group_plan_binding_for_revision(
                &input.project_id,
                &input.issue_id,
                &input.plan_id,
                active_revision_id,
            )
            .map_err(|error| format!("resolve authoritative group plan binding failed: {error}"))?;
        if authoritative.units.is_empty() {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_GROUP_EMPTY".to_string(),
                reason: "confirmed work item plan has no authoritative coding units".to_string(),
            });
        }

        self.initialize_advance(input, advance_store, coding_store, authoritative)
            .await
            .map_err(|error| error.to_string())
    }

    async fn initialize_advance(
        &mut self,
        input: AdvanceInput,
        advance_store: AdvanceStore,
        coding_store: CodingAttemptStore,
        authoritative: AuthoritativeGroupPlanBinding,
    ) -> Result<AdvanceOutcome, String> {
        let result = self
            .initialize_advance_inner(
                input.clone(),
                advance_store.clone(),
                coding_store,
                authoritative,
            )
            .await;
        if let Err(error) = &result
            && let Ok(Some(record)) = advance_store.get_advance_by_command_id(
                &input.project_id,
                &input.issue_id,
                &input.command_id,
            )
        {
            // REQ-MTG-02（分流失败标记适配，D2.1）：多 target（journal 数 ≥2）下
            // error 落每个 per-target journal；单 target 保持原分支零变化。
            let split_journals = {
                let journals = CodingAttemptStore::new(advance_store.app_paths())
                    .list_group_initialization_journals_for_plan(
                        &record.project_id,
                        &record.issue_id,
                        &record.plan_id,
                    );
                match journals {
                    Ok(journals) if journals.len() >= 2 => journals,
                    _ => Vec::new(),
                }
            };
            for group_journal in &split_journals {
                let _ = CodingAttemptStore::new(advance_store.app_paths())
                    .mark_group_initialization_error(group_journal, error);
            }
            if let Ok(Some(journal)) = advance_store.get_advance_initialization(&record) {
                let _ = advance_store.mark_advance_initialization_error(&record, &journal, error);
            } else if let Some(group_journal) = split_journals.first() {
                // 分流早期失败（外层 journal 尚未建）：record 落 attempt 集身份
                // （attempt 集身份保留——per-target journal 的 attempt 全集回落）。
                let mut failed = record.clone();
                failed.status = AdvanceStatus::Failed;
                failed.error = Some(error.clone());
                failed.attempt_id = Some(group_journal.attempt.id.clone());
                failed.target_attempts = split_journals
                    .iter()
                    .filter_map(|journal| {
                        journal.attempt.target_snapshot.as_ref().map(|snapshot| {
                            AdvanceTargetAttemptBinding {
                                target_repository_id: snapshot.logical_repository_id.0.to_string(),
                                attempt_id: journal.attempt.id.clone(),
                            }
                        })
                    })
                    .collect();
                failed.updated_at = chrono::Utc::now().to_rfc3339();
                let _ = advance_store.update_record(&failed);
            } else if let Ok(group_journal) = CodingAttemptStore::new(advance_store.app_paths())
                .get_group_initialization(&record.project_id, &record.issue_id, &record.plan_id)
            {
                let _ = CodingAttemptStore::new(advance_store.app_paths())
                    .mark_group_initialization_error(&group_journal, error);
                let mut failed = record.clone();
                failed.status = AdvanceStatus::Failed;
                failed.error = Some(error.clone());
                failed.attempt_id = Some(group_journal.attempt.id);
                failed.updated_at = chrono::Utc::now().to_rfc3339();
                let _ = advance_store.update_record(&failed);
            } else if record.status == AdvanceStatus::Initializing {
                let mut failed = record.clone();
                failed.status = AdvanceStatus::Failed;
                failed.error = Some(error.clone());
                failed.updated_at = chrono::Utc::now().to_rfc3339();
                let _ = advance_store.update_record(&failed);
            }
        }
        result
    }

    async fn initialize_advance_inner(
        &mut self,
        input: AdvanceInput,
        advance_store: AdvanceStore,
        coding_store: CodingAttemptStore,
        authoritative: AuthoritativeGroupPlanBinding,
    ) -> Result<AdvanceOutcome, String> {
        let _initialization_guard = coding_store
            .acquire_group_initialization_arbitration(&input.project_id, &input.issue_id)
            .map_err(|error| format!("acquire advance initialization lock failed: {error}"))?;
        let record = advance_store
            .persist_advance_record_if_absent(&input, &authoritative.plan_revision_id)
            .map_err(|error| format!("persist advance record failed: {error}"))?;
        maybe_fail_advance_initialization(&input, AdvanceInitializationFailpoint::RecordPersisted)?;
        if record.plan_revision_id != authoritative.plan_revision_id {
            return Err(
                "advance record plan revision differs from authoritative plan revision".to_string(),
            );
        }

        // REQ-MTG-01/02（D1 显式两分支）：authoritative units 分出 ≥2 个 target 桶
        // 时进分流循环（per-target CreateGroupCodingAttemptInput）；单 target（含
        // 0-target focus 唯一）走下方现行路径零变化。
        let grouped = units_by_target(&authoritative);
        if grouped.by_target.len() >= 2 {
            return self
                .initialize_advance_split(
                    input,
                    advance_store,
                    coding_store,
                    authoritative,
                    grouped,
                    record,
                )
                .await;
        }

        let repository =
            Self::resolve_advance_repository(&advance_store.app_paths(), &input, &authoritative)?;
        let current_unit = authoritative
            .units
            .first()
            .ok_or_else(|| "authoritative group has no first unit".to_string())?;
        let existing_group_journal = match coding_store.get_group_initialization(
            &input.project_id,
            &input.issue_id,
            &input.plan_id,
        ) {
            Ok(journal) => {
                if journal.attempt.admission_kind != CodingAdmissionKind::ScAdvance
                    || journal.plan_binding.bound_plan_revision_id != authoritative.plan_revision_id
                {
                    return Err(
                        "existing group initialization is bound to another advance identity"
                            .to_string(),
                    );
                }
                Some(journal)
            }
            Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => None,
            Err(error) => {
                return Err(format!(
                    "load existing group initialization failed: {error}"
                ));
            }
        };
        let provider_config = existing_group_journal
            .as_ref()
            .map(|journal| ProviderConfigSnapshot {
                author: journal.attempt.provider_config_snapshot.author.clone(),
                reviewer: journal.attempt.provider_config_snapshot.reviewer.clone(),
                review_rounds: journal.attempt.provider_config_snapshot.review_rounds,
                permission_modes: journal
                    .attempt
                    .provider_config_snapshot
                    .permission_modes
                    .clone(),
            })
            .unwrap_or_else(|| Self::advance_provider_config(&self.session, current_unit));
        let branch_name = existing_group_journal
            .as_ref()
            .map(|journal| journal.attempt.branch_name.clone())
            .unwrap_or_else(|| format!("aria/issues/{}", input.issue_id));
        let base_branch = existing_group_journal
            .as_ref()
            .map(|journal| journal.attempt.base_branch.clone())
            .unwrap_or_else(|| {
                Self::current_git_branch(&repository.path).unwrap_or_else(|| "HEAD".to_string())
            });
        let worktree_path = existing_group_journal
            .as_ref()
            .and_then(|journal| journal.attempt.worktree_path.clone())
            .unwrap_or_else(|| {
                repository
                    .path
                    .join(".worktrees")
                    .join("aria-issues")
                    .join(&input.issue_id)
            });
        let target_snapshot = existing_group_journal
            .as_ref()
            .and_then(|journal| journal.attempt.target_snapshot.clone())
            .or(Self::advance_target_snapshot(
                &advance_store.app_paths(),
                &input,
                &authoritative,
            )?);
        let group_input = CreateGroupCodingAttemptInput {
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            plan_id: input.plan_id.clone(),
            current_work_item_id: current_unit.logical_work_item_id.clone(),
            base_branch,
            branch_name,
            worktree_path: Some(worktree_path.clone()),
            provider_config_snapshot: provider_config,
            target_snapshot,
            max_auto_rework: 2,
        };
        let mut group_journal = match existing_group_journal {
            Some(journal) => journal,
            None => coding_store
                .prepare_group_initialization_with_admission(
                    &group_input,
                    &authoritative.plan_revision_id,
                    &authoritative.units,
                    CodingAdmissionKind::ScAdvance,
                )
                .map_err(|error| format!("prepare group initialization failed: {error}"))?,
        };
        if group_journal.attempt.admission_kind != CodingAdmissionKind::ScAdvance {
            return Err("advance initialization journal is not an SC admission".to_string());
        }
        let mut outer = advance_store
            .load_or_prepare_advance_initialization(&record, &group_journal)
            .map_err(|error| format!("persist advance initialization failed: {error}"))?;
        maybe_fail_advance_initialization(&input, AdvanceInitializationFailpoint::JournalPrepared)?;
        if outer.phase.order_for_engine()
            >= AdvanceInitializationPhase::AttemptPersisted.order_for_engine()
            && record.attempt_id.as_deref() != Some(outer.attempt_id.as_str())
        {
            return Err(
                "advance record attempt identity differs from initialization journal".to_string(),
            );
        }
        if !group_journal.phase.has_reached(
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted,
        ) {
            let record_attempt = coding_store
                .ensure_group_initialization_attempt(
                    &group_journal,
                    &coding_store
                        .acquire_work_item_attempt_creation(
                            &input.project_id,
                            &input.issue_id,
                            &current_unit.logical_work_item_id,
                        )
                        .map_err(|error| {
                            format!("acquire attempt creation lock failed: {error}")
                        })?,
                )
                .map_err(|error| format!("persist group attempt failed: {error}"))?;
            maybe_fail_advance_initialization(
                &input,
                AdvanceInitializationFailpoint::GroupAttemptPersisted,
            )?;
            group_journal = coding_store
                .advance_group_initialization_phase(
                    &group_journal,
                    crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted,
                )
                .map_err(|error| format!("checkpoint group attempt persistence failed: {error}"))?;
            maybe_fail_advance_initialization(
                &input,
                AdvanceInitializationFailpoint::AttemptPersisted,
            )?;
            if group_journal.attempt.id != record_attempt.id {
                return Err(
                    "group initialization attempt identity changed during replay".to_string(),
                );
            }
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::AttemptPersisted.order_for_engine()
        {
            let mut updated = record.clone();
            updated.attempt_id = Some(group_journal.attempt.id.clone());
            updated.updated_at = chrono::Utc::now().to_rfc3339();
            advance_store
                .update_record(&updated)
                .map_err(|error| format!("bind attempt to advance record failed: {error}"))?;
            outer = advance_store
                .advance_initialization_phase(
                    &updated,
                    &outer,
                    AdvanceInitializationPhase::AttemptPersisted,
                )
                .map_err(|error| format!("checkpoint attempt persistence failed: {error}"))?;
        }
        if !group_journal.phase.has_reached(
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::WorktreeBound,
        ) {
            let lifecycle = self
                .lifecycle_store
                .as_ref()
                .ok_or("lifecycle store unavailable")?;
            lifecycle
                .upsert_issue_shared_worktree(
                    crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput {
                        project_id: input.project_id.clone(),
                        issue_id: input.issue_id.clone(),
                        repository_id: repository.id.clone(),
                        branch_name: group_journal.attempt.branch_name.clone(),
                        worktree_path: worktree_path.clone(),
                        base_branch: group_journal.attempt.base_branch.clone(),
                    },
                )
                .map_err(|error| format!("persist shared worktree failed: {error}"))?;
            let lease = lifecycle
                .try_acquire_issue_worktree_lock(
                    &input.project_id,
                    &input.issue_id,
                    &group_journal.lock_work_item_id,
                    &group_journal.worktree_lease_id,
                )
                .map_err(|error| format!("acquire shared worktree failed: {error}"))?;
            if !lease.acquired
                && lease.worktree.current_lock_owner_id.as_deref()
                    != Some(group_journal.attempt.id.as_str())
            {
                return Err("shared worktree is owned by another attempt".to_string());
            }
            lifecycle
                .bind_issue_worktree_lock_to_attempt(
                    &input.project_id,
                    &input.issue_id,
                    &group_journal.lock_work_item_id,
                    &group_journal.attempt.id,
                )
                .map_err(|error| format!("bind shared worktree failed: {error}"))?;
            group_journal = coding_store
                .advance_group_initialization_phase(
                    &group_journal,
                    crate::product::coding_attempt_store::CodingGroupInitializationPhase::WorktreeBound,
                )
                .map_err(|error| format!("checkpoint group worktree binding failed: {error}"))?;
            maybe_fail_advance_initialization(
                &input,
                AdvanceInitializationFailpoint::WorktreeBound,
            )?;
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::WorktreeBound.order_for_engine()
        {
            let current_record = advance_store
                .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
                .map_err(|error| format!("reload advance record failed: {error}"))?
                .ok_or("advance record disappeared")?;
            outer = advance_store
                .advance_initialization_phase(
                    &current_record,
                    &outer,
                    AdvanceInitializationPhase::WorktreeBound,
                )
                .map_err(|error| format!("checkpoint worktree binding failed: {error}"))?;
        }
        let current_record = advance_store
            .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
            .map_err(|error| format!("reload advance record failed: {error}"))?
            .ok_or("advance record disappeared")?;
        if !group_journal.phase.has_reached(
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::PlanBindingSaved,
        ) {
            coding_store
                .ensure_group_initialization_plan_binding(&group_journal)
                .map_err(|error| format!("persist group plan binding failed: {error}"))?;
            group_journal = coding_store
                .advance_group_initialization_phase(
                    &group_journal,
                    crate::product::coding_attempt_store::CodingGroupInitializationPhase::PlanBindingSaved,
                )
                .map_err(|error| format!("checkpoint group plan binding failed: {error}"))?;
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::PlanBindingSaved.order_for_engine()
        {
            outer = advance_store
                .advance_initialization_phase(
                    &current_record,
                    &outer,
                    AdvanceInitializationPhase::PlanBindingSaved,
                )
                .map_err(|error| format!("checkpoint plan binding failed: {error}"))?;
            maybe_fail_advance_initialization(
                &input,
                AdvanceInitializationFailpoint::PlanBindingSaved,
            )?;
        }
        if !group_journal.phase.has_reached(
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::UnitsMaterialized,
        ) {
            for index in 0..group_journal.units.len() {
                coding_store
                    .ensure_group_initialization_unit(&group_journal, index)
                    .map_err(|error| format!("persist group unit failed: {error}"))?;
            }
            group_journal = coding_store
                .advance_group_initialization_phase(
                    &group_journal,
                    crate::product::coding_attempt_store::CodingGroupInitializationPhase::UnitsMaterialized,
                )
                .map_err(|error| format!("checkpoint group units materialization failed: {error}"))?;
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::UnitsMaterialized.order_for_engine()
        {
            outer = advance_store
                .advance_initialization_phase(
                    &current_record,
                    &outer,
                    AdvanceInitializationPhase::UnitsMaterialized,
                )
                .map_err(|error| format!("checkpoint units materialization failed: {error}"))?;
            maybe_fail_advance_initialization(
                &input,
                AdvanceInitializationFailpoint::UnitsMaterialized,
            )?;
        }
        let persisted_attempt = coding_store
            .get_attempt(
                &input.project_id,
                &input.issue_id,
                &group_journal.attempt.id,
            )
            .map_err(|error| format!("load initialized attempt failed: {error}"))?;
        coding_store
            .validate_group_attempt_integrity(&persisted_attempt)
            .map_err(|error| format!("validate initialized group failed: {error}"))?;
        let final_record = advance_store
            .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
            .map_err(|error| format!("reload final advance record failed: {error}"))?
            .ok_or("advance record disappeared")?;
        if !group_journal.phase.has_reached(
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::Completed,
        ) {
            coding_store
                .advance_group_initialization_phase(
                    &group_journal,
                    crate::product::coding_attempt_store::CodingGroupInitializationPhase::Completed,
                )
                .map_err(|error| {
                    format!("checkpoint group initialization completion failed: {error}")
                })?;
        }
        advance_store
            .advance_initialization_phase(&final_record, &outer, AdvanceInitializationPhase::Ready)
            .map_err(|error| format!("checkpoint ready initialization failed: {error}"))?;
        let mut ready_record = final_record;
        ready_record.status = AdvanceStatus::Ready;
        ready_record.attempt_id = Some(persisted_attempt.id.clone());
        ready_record.workspace_entry = Some(Self::advance_workspace_entry(&persisted_attempt));
        ready_record.updated_at = chrono::Utc::now().to_rfc3339();
        advance_store
            .update_record(&ready_record)
            .map_err(|error| format!("persist ready advance record failed: {error}"))?;
        Ok(AdvanceOutcome::Completed {
            record: ready_record,
            attempt_id: persisted_attempt.id.clone(),
            workspace_entry: Self::advance_workspace_entry(&persisted_attempt),
            // 单 target 路径：集绑定为空（wire 不发送），单值 attempt_id 承载不变。
            target_attempts: Vec::new(),
        })
    }

    /// REQ-MTG-01/02（WP2 分流创建，k3 F3 必改点一）：多 target（`units_by_target`
    /// 桶数 ≥2）显式分流循环。单 target（含 0-target focus 唯一）不进入本函数
    /// ——`initialize_advance_inner` 现行路径零变化（D1 显式两分支）。
    ///
    /// 每 target：独立 `CreateGroupCodingAttemptInput`（OQ2 命名
    /// branch=`aria/issues/{issue_id}/{logical_id}`、
    /// worktree=`.worktrees/aria-issues/{issue_id}/{logical_id}`、base_branch=各仓
    /// 当前分支、per-target 冻结快照 `build_attempt_target_snapshot`）→
    /// `prepare_group_initialization_with_admission_for_target(Some(t))` per-target
    /// journal 子路径 → `ensure_group_initialization_attempt` → repo 维三元键
    /// worktree 三件套（含 T2S3 嵌套共存检查）→ units 分组物化。外层 advance
    /// journal 用集绑定形态（`load_or_prepare_advance_initialization_for_attempts`：
    /// `attempt_id`=全局拓扑序首个+`target_attempt_ids` 全集，集合一致性比对）。
    /// 本函数只建组——无自动跨 attempt 编排（REQ-MTG-03，StartCoding 唯一入口）。
    async fn initialize_advance_split(
        &mut self,
        input: AdvanceInput,
        advance_store: AdvanceStore,
        coding_store: CodingAttemptStore,
        authoritative: AuthoritativeGroupPlanBinding,
        grouped: UnitsByTarget,
        record: AdvanceRecord,
    ) -> Result<AdvanceOutcome, String> {
        // D2.1 对偶（A2 不静默归属）：≥2 target 桶下无归属 unit 不入任何桶、也
        // 不回退 focus——显式 fail-closed。
        if !grouped.unattributed.is_empty() {
            return Err(
                "advance split requires every authoritative unit to carry a target repository"
                    .to_string(),
            );
        }
        let paths = advance_store.app_paths();
        let lc_id = resolve_issue_logical_codebase_id(&paths, &input.project_id, &input.issue_id)
            .map_err(|error| format!("resolve advance target codebase failed: {error}"))?;
        let RepositoryRouting::Logical { manifest, .. } =
            RepositoryRouting::load_for_issue(&paths, &input.project_id, &input.issue_id)
                .map_err(|error| format!("load advance repository routing failed: {error}"))?
        else {
            // Legacy 路由无逻辑仓可分流（快照/仓解析均按 logical id 寻址）。
            return Err("advance split requires logical repository routing".to_string());
        };

        // 既有 per-target journal（幂等重放的权威输入）：按 attempt 冻结快照的
        // target 归桶；准入身份与 plan revision 绑定校验与单 target 路径同语义。
        let mut existing_by_target = std::collections::BTreeMap::new();
        for journal in coding_store
            .list_group_initialization_journals_for_plan(
                &input.project_id,
                &input.issue_id,
                &input.plan_id,
            )
            .map_err(|error| format!("load existing split journals failed: {error}"))?
        {
            if journal.attempt.admission_kind != CodingAdmissionKind::ScAdvance
                || journal.plan_binding.bound_plan_revision_id != authoritative.plan_revision_id
            {
                return Err(
                    "existing group initialization is bound to another advance identity"
                        .to_string(),
                );
            }
            let target = journal
                .attempt
                .target_snapshot
                .as_ref()
                .map(|snapshot| snapshot.logical_repository_id)
                .ok_or_else(|| {
                    "existing split group initialization has no target snapshot".to_string()
                })?;
            existing_by_target.insert(target, journal);
        }
        if existing_by_target
            .keys()
            .any(|target| !grouped.by_target.contains_key(target))
        {
            return Err(
                "existing split group initialization covers a target absent from the plan"
                    .to_string(),
            );
        }

        // 全局拓扑序：每 target 以其 units 的最小全局序定位——record.attempt_id 与
        // `target_attempts` 顺序由全局拓扑序首个 unit 所属 target 承载。
        let global_order = topologically_order_unit_bindings(&authoritative.units)
            .map_err(|error| format!("order split units failed: {error}"))?;
        let global_position: std::collections::BTreeMap<&str, usize> = global_order
            .iter()
            .enumerate()
            .map(|(index, unit)| (unit.logical_work_item_id.as_str(), index))
            .collect();

        struct SplitTarget {
            target: crate::product::logical_codebase::LogicalRepositoryId,
            journal: crate::product::coding_attempt_store::CodingGroupInitializationJournal,
            order_index: usize,
        }
        let mut targets: Vec<SplitTarget> = Vec::with_capacity(grouped.by_target.len());
        for (target, units) in grouped.by_target {
            let ordered_units = topologically_order_unit_bindings(&units)
                .map_err(|error| format!("order split units failed: {error}"))?;
            if !manifest.member_ids.contains(&target) {
                return Err("advance logical repository target is not selected".to_string());
            }
            let repository = RepositoryStore::new(paths.clone())
                .resolve_logical_repository_for_issue_codebase(
                    &input.project_id,
                    lc_id.as_deref(),
                    target,
                )
                .map(|(_, _, repository)| repository)
                .map_err(|error| format!("resolve advance logical repository failed: {error}"))?;
            let existing = existing_by_target.get(&target);
            let provider_config = existing
                .map(|journal| ProviderConfigSnapshot {
                    author: journal.attempt.provider_config_snapshot.author.clone(),
                    reviewer: journal.attempt.provider_config_snapshot.reviewer.clone(),
                    review_rounds: journal.attempt.provider_config_snapshot.review_rounds,
                    permission_modes: journal
                        .attempt
                        .provider_config_snapshot
                        .permission_modes
                        .clone(),
                })
                .unwrap_or_else(|| Self::advance_provider_config(&self.session, &ordered_units[0]));
            // OQ2 命名：branch/worktree 以 target UUID 限定；重放以 journal 冻结值
            // 为权威（快照 captured_at 不可重捕获）。
            let branch_name = existing
                .map(|journal| journal.attempt.branch_name.clone())
                .unwrap_or_else(|| format!("aria/issues/{}/{}", input.issue_id, target.0));
            let base_branch = existing
                .map(|journal| journal.attempt.base_branch.clone())
                .unwrap_or_else(|| {
                    Self::current_git_branch(&repository.path).unwrap_or_else(|| "HEAD".to_string())
                });
            let worktree_path = existing
                .and_then(|journal| journal.attempt.worktree_path.clone())
                .unwrap_or_else(|| {
                    repository
                        .path
                        .join(".worktrees")
                        .join("aria-issues")
                        .join(&input.issue_id)
                        .join(target.0.to_string())
                });
            let target_snapshot = existing
                .and_then(|journal| journal.attempt.target_snapshot.clone())
                .or(Some(
                    build_attempt_target_snapshot(
                        &paths,
                        &input.project_id,
                        target,
                        lc_id.as_deref(),
                    )
                    .map_err(|error| format!("capture advance target snapshot failed: {error}"))?,
                ));
            let group_input = CreateGroupCodingAttemptInput {
                project_id: input.project_id.clone(),
                issue_id: input.issue_id.clone(),
                plan_id: input.plan_id.clone(),
                current_work_item_id: ordered_units[0].logical_work_item_id.clone(),
                base_branch,
                branch_name,
                worktree_path: Some(worktree_path),
                provider_config_snapshot: provider_config,
                target_snapshot,
                max_auto_rework: 2,
            };
            let journal = match existing {
                Some(journal) => journal.clone(),
                None => coding_store
                    .prepare_group_initialization_with_admission_for_target(
                        &group_input,
                        &authoritative.plan_revision_id,
                        &ordered_units,
                        CodingAdmissionKind::ScAdvance,
                        Some(target),
                    )
                    .map_err(|error| format!("prepare group initialization failed: {error}"))?,
            };
            let order_index = journal
                .units
                .iter()
                .map(|unit| {
                    global_position
                        .get(unit.logical_work_item_id.as_str())
                        .copied()
                        .unwrap_or(usize::MAX)
                })
                .min()
                .unwrap_or(usize::MAX);
            targets.push(SplitTarget {
                target,
                journal,
                order_index,
            });
        }
        targets.sort_by_key(|split| split.order_index);
        let first_attempt_id = targets
            .first()
            .ok_or("advance split has no target attempts")?
            .journal
            .attempt
            .id
            .clone();
        let target_attempts: Vec<AdvanceTargetAttemptBinding> = targets
            .iter()
            .map(|split| AdvanceTargetAttemptBinding {
                target_repository_id: split.target.0.to_string(),
                attempt_id: split.journal.attempt.id.clone(),
            })
            .collect();
        let target_attempt_ids: Vec<String> = targets
            .iter()
            .map(|split| split.journal.attempt.id.clone())
            .collect();

        // 外层 advance journal 集绑定（OQ1）：store 侧做集合一致性比对（:508-515
        // 断言的集合化升级）；重放时 record 集绑定与 journal 全集互证。
        let mut outer = advance_store
            .load_or_prepare_advance_initialization_for_attempts(
                &record,
                &first_attempt_id,
                &target_attempt_ids,
            )
            .map_err(|error| format!("persist advance initialization failed: {error}"))?;
        maybe_fail_advance_initialization(&input, AdvanceInitializationFailpoint::JournalPrepared)?;
        if outer.phase.order_for_engine()
            >= AdvanceInitializationPhase::AttemptPersisted.order_for_engine()
        {
            let current_record = advance_store
                .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
                .map_err(|error| format!("reload advance record failed: {error}"))?
                .ok_or("advance record disappeared")?;
            let mut persisted_attempts: Vec<String> = current_record
                .target_attempts
                .iter()
                .map(|binding| binding.attempt_id.clone())
                .collect();
            let mut expected_attempts: Vec<String> = target_attempt_ids.clone();
            persisted_attempts.sort_unstable();
            expected_attempts.sort_unstable();
            if current_record.attempt_id.as_deref() != Some(outer.attempt_id.as_str())
                || persisted_attempts != expected_attempts
            {
                return Err(
                    "advance record attempt set differs from initialization journal".to_string(),
                );
            }
        }

        // per-target attempt 持久化：各自 creation lock 与 failpoint 前缀，与单
        // target 路径同套（中断恢复由 journal.phase.has_reached 前缀判定续走）。
        for split in &mut targets {
            if !split.journal.phase.has_reached(
                crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted,
            ) {
                let record_attempt = coding_store
                    .ensure_group_initialization_attempt(
                        &split.journal,
                        &coding_store
                            .acquire_work_item_attempt_creation(
                                &input.project_id,
                                &input.issue_id,
                                &split.journal.lock_work_item_id,
                            )
                            .map_err(|error| {
                                format!("acquire attempt creation lock failed: {error}")
                            })?,
                    )
                    .map_err(|error| format!("persist group attempt failed: {error}"))?;
                maybe_fail_advance_initialization(
                    &input,
                    AdvanceInitializationFailpoint::GroupAttemptPersisted,
                )?;
                split.journal = coding_store
                    .advance_group_initialization_phase(
                        &split.journal,
                        crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted,
                    )
                    .map_err(|error| {
                        format!("checkpoint group attempt persistence failed: {error}")
                    })?;
                maybe_fail_advance_initialization(
                    &input,
                    AdvanceInitializationFailpoint::AttemptPersisted,
                )?;
                if split.journal.attempt.id != record_attempt.id {
                    return Err(
                        "group initialization attempt identity changed during replay"
                            .to_string(),
                    );
                }
            }
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::AttemptPersisted.order_for_engine()
        {
            let mut updated = record.clone();
            updated.attempt_id = Some(first_attempt_id.clone());
            updated.target_attempts = target_attempts.clone();
            updated.updated_at = chrono::Utc::now().to_rfc3339();
            advance_store
                .update_record(&updated)
                .map_err(|error| format!("bind attempt to advance record failed: {error}"))?;
            outer = advance_store
                .advance_initialization_phase(
                    &updated,
                    &outer,
                    AdvanceInitializationPhase::AttemptPersisted,
                )
                .map_err(|error| format!("checkpoint attempt persistence failed: {error}"))?;
        }

        // per-target worktree 绑定：repo 维三元键三件套 + T2S3 嵌套共存检查。
        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .ok_or("lifecycle store unavailable")?;
        for split in &mut targets {
            if !split.journal.phase.has_reached(
                crate::product::coding_attempt_store::CodingGroupInitializationPhase::WorktreeBound,
            ) {
                let worktree_path = split
                    .journal
                    .attempt
                    .worktree_path
                    .clone()
                    .ok_or("split group attempt has no worktree path")?;
                let parent = worktree_path
                    .parent()
                    .ok_or("split worktree path has no parent")?
                    .to_path_buf();
                Self::ensure_split_worktree_parent_free(
                    lifecycle,
                    &input.project_id,
                    &input.issue_id,
                    &parent,
                    split.target,
                )?;
                lifecycle
                    .upsert_repo_shared_worktree(
                        crate::product::lifecycle_store::UpsertRepoSharedWorktreeInput {
                            project_id: input.project_id.clone(),
                            issue_id: input.issue_id.clone(),
                            repository_id: split.target,
                            branch_name: split.journal.attempt.branch_name.clone(),
                            worktree_path,
                            base_branch: split.journal.attempt.base_branch.clone(),
                        },
                    )
                    .map_err(|error| format!("persist shared worktree failed: {error}"))?;
                // repo 维 lease 与 journal.worktree_lease_id（issue 维前缀）解耦：
                // 确定性派生自 journal id（重放同值幂等）；bind 语义要求
                // `repo_worktree_lease_` 前缀或 owner 已是 attempt id。
                let lease_id = format!("repo_worktree_lease_{}", split.journal.id);
                let lease = lifecycle
                    .try_acquire_repo_worktree_lock(
                        &input.project_id,
                        &input.issue_id,
                        split.target,
                        &split.journal.lock_work_item_id,
                        &lease_id,
                    )
                    .map_err(|error| format!("acquire shared worktree failed: {error}"))?;
                if !lease.acquired
                    && lease.worktree.current_lock_owner_id.as_deref()
                        != Some(split.journal.attempt.id.as_str())
                {
                    return Err("shared worktree is owned by another attempt".to_string());
                }
                lifecycle
                    .bind_repo_worktree_lock_to_attempt(
                        &input.project_id,
                        &input.issue_id,
                        split.target,
                        &split.journal.lock_work_item_id,
                        &split.journal.attempt.id,
                    )
                    .map_err(|error| format!("bind shared worktree failed: {error}"))?;
                split.journal = coding_store
                    .advance_group_initialization_phase(
                        &split.journal,
                        crate::product::coding_attempt_store::CodingGroupInitializationPhase::WorktreeBound,
                    )
                    .map_err(|error| {
                        format!("checkpoint group worktree binding failed: {error}")
                    })?;
            }
        }
        maybe_fail_advance_initialization(&input, AdvanceInitializationFailpoint::WorktreeBound)?;
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::WorktreeBound.order_for_engine()
        {
            let current_record = advance_store
                .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
                .map_err(|error| format!("reload advance record failed: {error}"))?
                .ok_or("advance record disappeared")?;
            outer = advance_store
                .advance_initialization_phase(
                    &current_record,
                    &outer,
                    AdvanceInitializationPhase::WorktreeBound,
                )
                .map_err(|error| format!("checkpoint worktree binding failed: {error}"))?;
        }

        // per-target plan binding 与 units 物化；外层相位在全部 per-target journal
        // 达到对应相位后推进。
        for split in &mut targets {
            if !split.journal.phase.has_reached(
                crate::product::coding_attempt_store::CodingGroupInitializationPhase::PlanBindingSaved,
            ) {
                coding_store
                    .ensure_group_initialization_plan_binding(&split.journal)
                    .map_err(|error| format!("persist group plan binding failed: {error}"))?;
                split.journal = coding_store
                    .advance_group_initialization_phase(
                        &split.journal,
                        crate::product::coding_attempt_store::CodingGroupInitializationPhase::PlanBindingSaved,
                    )
                    .map_err(|error| {
                        format!("checkpoint group plan binding failed: {error}")
                    })?;
            }
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::PlanBindingSaved.order_for_engine()
        {
            let current_record = advance_store
                .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
                .map_err(|error| format!("reload advance record failed: {error}"))?
                .ok_or("advance record disappeared")?;
            outer = advance_store
                .advance_initialization_phase(
                    &current_record,
                    &outer,
                    AdvanceInitializationPhase::PlanBindingSaved,
                )
                .map_err(|error| format!("checkpoint plan binding failed: {error}"))?;
            maybe_fail_advance_initialization(
                &input,
                AdvanceInitializationFailpoint::PlanBindingSaved,
            )?;
        }
        for split in &mut targets {
            if !split.journal.phase.has_reached(
                crate::product::coding_attempt_store::CodingGroupInitializationPhase::UnitsMaterialized,
            ) {
                for index in 0..split.journal.units.len() {
                    coding_store
                        .ensure_group_initialization_unit(&split.journal, index)
                        .map_err(|error| format!("persist group unit failed: {error}"))?;
                }
                split.journal = coding_store
                    .advance_group_initialization_phase(
                        &split.journal,
                        crate::product::coding_attempt_store::CodingGroupInitializationPhase::UnitsMaterialized,
                    )
                    .map_err(|error| {
                        format!("checkpoint group units materialization failed: {error}")
                    })?;
            }
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::UnitsMaterialized.order_for_engine()
        {
            let current_record = advance_store
                .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
                .map_err(|error| format!("reload advance record failed: {error}"))?
                .ok_or("advance record disappeared")?;
            outer = advance_store
                .advance_initialization_phase(
                    &current_record,
                    &outer,
                    AdvanceInitializationPhase::UnitsMaterialized,
                )
                .map_err(|error| format!("checkpoint units materialization failed: {error}"))?;
            maybe_fail_advance_initialization(
                &input,
                AdvanceInitializationFailpoint::UnitsMaterialized,
            )?;
        }

        let mut persisted_attempts = Vec::with_capacity(targets.len());
        for split in &targets {
            let persisted = coding_store
                .get_attempt(
                    &input.project_id,
                    &input.issue_id,
                    &split.journal.attempt.id,
                )
                .map_err(|error| format!("load initialized attempt failed: {error}"))?;
            coding_store
                .validate_group_attempt_integrity(&persisted)
                .map_err(|error| format!("validate initialized group failed: {error}"))?;
            persisted_attempts.push(persisted);
        }
        for split in &targets {
            if !split.journal.phase.has_reached(
                crate::product::coding_attempt_store::CodingGroupInitializationPhase::Completed,
            ) {
                coding_store
                    .advance_group_initialization_phase(
                        &split.journal,
                        crate::product::coding_attempt_store::CodingGroupInitializationPhase::Completed,
                    )
                    .map_err(|error| {
                        format!("checkpoint group initialization completion failed: {error}")
                    })?;
            }
        }
        let final_record = advance_store
            .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
            .map_err(|error| format!("reload final advance record failed: {error}"))?
            .ok_or("advance record disappeared")?;
        advance_store
            .advance_initialization_phase(&final_record, &outer, AdvanceInitializationPhase::Ready)
            .map_err(|error| format!("checkpoint ready initialization failed: {error}"))?;
        let workspace_entry = Self::advance_workspace_entry(&persisted_attempts[0]);
        let mut ready_record = final_record;
        ready_record.status = AdvanceStatus::Ready;
        ready_record.attempt_id = Some(first_attempt_id.clone());
        ready_record.target_attempts = target_attempts.clone();
        ready_record.workspace_entry = Some(workspace_entry.clone());
        ready_record.updated_at = chrono::Utc::now().to_rfc3339();
        advance_store
            .update_record(&ready_record)
            .map_err(|error| format!("persist ready advance record failed: {error}"))?;
        // REQ-MTG-05（WP5）审计挂点：`record_split_audit`（本文件占位）在 T5 落地
        // 真实现并接线；本 Task 不在分流成功点落任何 durable 审计记录。
        Ok(AdvanceOutcome::Completed {
            record: ready_record,
            attempt_id: first_attempt_id,
            workspace_entry,
            target_attempts,
        })
    }

    /// T2S3（OQ2 披露，worktree 嵌套共存检查）：per-target worktree 父路径
    /// `.worktrees/aria-issues/{issue_id}` 若已被 issue 级 shared worktree 或其他
    /// repo 维 worktree 记录占用（比对 worktree_path）→ fail-closed。本 target
    /// 既有记录由随后的 repo 维 upsert 覆写归一（同三元键权威自愈）。
    fn ensure_split_worktree_parent_free(
        lifecycle: &crate::product::lifecycle_store::LifecycleStore,
        project_id: &str,
        issue_id: &str,
        parent: &std::path::Path,
        target: crate::product::logical_codebase::LogicalRepositoryId,
    ) -> Result<(), String> {
        if let Some(issue_worktree) = lifecycle
            .get_issue_shared_worktree(project_id, issue_id)
            .map_err(|error| format!("load issue shared worktree failed: {error}"))?
            && issue_worktree.worktree_path == parent
        {
            return Err(format!(
                "split worktree parent path is already registered as the issue shared worktree: {}",
                parent.display()
            ));
        }
        for repository in lifecycle
            .list_repo_shared_worktrees(project_id, issue_id)
            .map_err(|error| format!("list repo shared worktrees failed: {error}"))?
        {
            if repository == target {
                continue;
            }
            if let Some(record) = lifecycle
                .get_repo_shared_worktree(project_id, issue_id, repository)
                .map_err(|error| format!("load repo shared worktree failed: {error}"))?
                && record.worktree_path == parent
            {
                return Err(format!(
                    "split worktree parent path is already registered as another repository worktree: {}",
                    parent.display()
                ));
            }
        }
        Ok(())
    }

    fn advance_target_snapshot(
        paths: &crate::product::app_paths::ProductAppPaths,
        input: &AdvanceInput,
        authoritative: &AuthoritativeGroupPlanBinding,
    ) -> Result<Option<crate::product::coding_models::AttemptTargetSnapshot>, String> {
        let Some(logical_id) = authoritative
            .units
            .iter()
            .filter_map(|unit| unit.target_repository_id)
            .next()
        else {
            return Ok(None);
        };
        let lc_id = resolve_issue_logical_codebase_id(paths, &input.project_id, &input.issue_id)
            .map_err(|error| format!("resolve advance target codebase failed: {error}"))?;
        build_attempt_target_snapshot(paths, &input.project_id, logical_id, lc_id.as_deref())
            .map(Some)
            .map_err(|error| format!("capture advance target snapshot failed: {error}"))
    }
    fn advance_workspace_entry(
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
    ) -> String {
        attempt
            .worktree_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("workspace://{}", attempt.id))
    }

    fn current_git_branch(path: &std::path::Path) -> Option<String> {
        let output = Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(path)
            .output()
            .ok()?;
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (!branch.is_empty()).then_some(branch)
    }

    fn advance_provider_config(
        session: &super::types::WorkspaceSession,
        _unit: &crate::product::coding_attempt_store::AuthoritativeCodingUnitBinding,
    ) -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: session.author_provider.clone(),
            reviewer: session.reviewer_provider.clone(),
            review_rounds: session.review_rounds,
            permission_modes: session.permission_modes.clone(),
        }
    }

    fn resolve_advance_repository(
        paths: &crate::product::app_paths::ProductAppPaths,
        input: &AdvanceInput,
        authoritative: &AuthoritativeGroupPlanBinding,
    ) -> Result<crate::product::models::RepositoryRecord, String> {
        let issue = IssueStore::new(paths.clone())
            .get(&input.project_id, &input.issue_id)
            .map_err(|error| format!("load advance issue failed: {error}"))?;
        match RepositoryRouting::load_for_issue(paths, &input.project_id, &input.issue_id)
            .map_err(|error| format!("load advance repository routing failed: {error}"))?
        {
            RepositoryRouting::Legacy { .. } => {
                let repository_id = issue.repo_id.ok_or("advance issue has no repository")?;
                let project = crate::product::project_store::ProjectStore::new(paths.clone())
                    .get(&input.project_id)
                    .map_err(|error| format!("load advance project failed: {error}"))?;
                let store = RepositoryStore::for_project(paths.clone(), &project);
                store
                    .resolve_legacy_physical_repository_if_dual(&input.project_id, &repository_id)
                    .map(|(_, _, repository)| repository)
                    .or_else(|_| {
                        store
                            .list(&input.project_id)
                            .map_err(|error| format!("list advance repositories failed: {error}"))?
                            .into_iter()
                            .find(|repository| repository.id == repository_id)
                            .ok_or_else(|| "advance repository not found".to_string())
                    })
            }
            RepositoryRouting::Logical {
                manifest,
                selection,
            } => {
                let logical_id = authoritative
                    .units
                    .iter()
                    .filter_map(|unit| unit.target_repository_id)
                    .next()
                    .or_else(|| selection.focus_repository_ids.first().copied())
                    .ok_or("advance logical repository target missing")?;
                if !manifest.member_ids.contains(&logical_id) {
                    return Err("advance logical repository target is not selected".to_string());
                }
                let lc_id =
                    resolve_issue_logical_codebase_id(paths, &input.project_id, &input.issue_id)
                        .map_err(|error| format!("resolve advance codebase failed: {error}"))?;
                RepositoryStore::new(paths.clone())
                    .resolve_logical_repository_for_issue_codebase(
                        &input.project_id,
                        lc_id.as_deref(),
                        logical_id,
                    )
                    .map(|(_, _, repository)| repository)
                    .map_err(|error| format!("resolve advance logical repository failed: {error}"))
            }
            RepositoryRouting::FailClosed { reason, .. } => Err(reason),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn advance_record_store(&self) -> Option<AdvanceStore> {
        self.lifecycle_store
            .as_ref()
            .map(|store| AdvanceStore::new(store.app_paths()))
    }
}

/// REQ-MTG-05（WP5 增殖审计）：分流创建成功点的审计挂点占位（T2 预留，T5 落地）。
///
/// T5 将以真实 durable 审计记录实现替换（`coding_attempt_store::split_audit`
/// 新模块：增殖审计记录+per-(plan,target) 检索）并在 `initialize_advance_split`
/// 成功返回前接线调用；本 Task 只钉住调用形态（编译期接口），不落任何记录、
/// 不接任何调用。
#[allow(dead_code)]
fn record_split_audit(
    _project_id: &str,
    _issue_id: &str,
    _plan_id: &str,
    _command_id: &str,
    _target_attempts: &[AdvanceTargetAttemptBinding],
) {
}
