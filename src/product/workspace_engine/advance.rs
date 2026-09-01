use std::process::Command;

pub use crate::product::advance_store::{
    AdvanceInitializationPhase, AdvanceInput, AdvanceOutcome, AdvanceRecord, AdvanceStatus,
    AdvanceStore,
};
use crate::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot;
use crate::product::coding_attempt_store::{
    AuthoritativeGroupPlanBinding, CodingAttemptStore, CreateGroupCodingAttemptInput,
};
use crate::product::coding_models::CodingAdmissionKind;
use crate::product::issue_store::IssueStore;
use crate::product::logical_codebase::{RepositoryRouting, resolve_issue_logical_codebase_id};
use crate::product::models::{IssueWorkItemPlan, IssueWorkItemPlanStatus, WorkspaceType};
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

        self.initialize_advance(input, advance_store, coding_store, authoritative, plan)
            .await
            .map_err(|error| error.to_string())
    }

    async fn initialize_advance(
        &mut self,
        input: AdvanceInput,
        advance_store: AdvanceStore,
        coding_store: CodingAttemptStore,
        authoritative: AuthoritativeGroupPlanBinding,
        plan: IssueWorkItemPlan,
    ) -> Result<AdvanceOutcome, String> {
        let record = advance_store
            .persist_advance_record_if_absent(&input, &authoritative.plan_revision_id)
            .map_err(|error| format!("persist advance record failed: {error}"))?;
        if let Some(existing) = advance_store
            .get_advance_initialization(&record)
            .map_err(|error| format!("load advance initialization failed: {error}"))?
        {
            return if existing.phase == AdvanceInitializationPhase::Ready
                && record.status == AdvanceStatus::Ready
            {
                Ok(Self::advance_replay_outcome(record))
            } else if matches!(
                record.status,
                AdvanceStatus::Failed | AdvanceStatus::Aborted
            ) {
                Ok(AdvanceOutcome::Replayed { record })
            } else {
                Err("advance initialization replay requires durable group journal".to_string())
            };
        }

        let repository =
            Self::resolve_advance_repository(&advance_store.app_paths(), &input, &authoritative)?;
        let current_unit = authoritative
            .units
            .first()
            .ok_or_else(|| "authoritative group has no first unit".to_string())?;
        let provider_config = Self::advance_provider_config(&self.session, current_unit);
        let branch_name = format!("aria/issues/{}", input.issue_id);
        let base_branch =
            Self::current_git_branch(&repository.path).unwrap_or_else(|| "HEAD".to_string());
        let worktree_path = repository
            .path
            .join(".worktrees")
            .join("aria-issues")
            .join(&input.issue_id);
        let group_input = CreateGroupCodingAttemptInput {
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            plan_id: input.plan_id.clone(),
            current_work_item_id: current_unit.logical_work_item_id.clone(),
            base_branch,
            branch_name,
            worktree_path: Some(worktree_path.clone()),
            provider_config_snapshot: provider_config,
            target_snapshot: Self::advance_target_snapshot(
                &advance_store.app_paths(),
                &input,
                &authoritative,
            )?,
            max_auto_rework: 2,
        };
        let group_journal = coding_store
            .prepare_group_initialization_with_admission(
                &group_input,
                &authoritative.plan_revision_id,
                &authoritative.units,
                CodingAdmissionKind::ScAdvance,
            )
            .map_err(|error| format!("prepare group initialization failed: {error}"))?;
        if group_journal.attempt.admission_kind != CodingAdmissionKind::ScAdvance {
            return Err("advance initialization journal is not an SC admission".to_string());
        }
        let mut outer = advance_store
            .load_or_prepare_advance_initialization(&record, &group_journal)
            .map_err(|error| format!("persist advance initialization failed: {error}"))?;
        if outer.phase.order_for_engine()
            >= AdvanceInitializationPhase::AttemptPersisted.order_for_engine()
            && record.attempt_id.as_deref() != Some(outer.attempt_id.as_str())
        {
            return Err(
                "advance record attempt identity differs from initialization journal".to_string(),
            );
        }
        if outer.phase == AdvanceInitializationPhase::JournalPrepared {
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
            let mut updated = record.clone();
            updated.attempt_id = Some(record_attempt.id.clone());
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
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::WorktreeBound.order_for_engine()
        {
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
            let current_record = advance_store
                .get_advance_by_command_id(&input.project_id, &input.issue_id, &input.command_id)
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
            .get_advance_by_command_id(&input.project_id, &input.issue_id, &input.command_id)
            .map_err(|error| format!("reload advance record failed: {error}"))?
            .ok_or("advance record disappeared")?;
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::PlanBindingSaved.order_for_engine()
        {
            coding_store
                .ensure_group_initialization_plan_binding(&group_journal)
                .map_err(|error| format!("persist group plan binding failed: {error}"))?;
            outer = advance_store
                .advance_initialization_phase(
                    &current_record,
                    &outer,
                    AdvanceInitializationPhase::PlanBindingSaved,
                )
                .map_err(|error| format!("checkpoint plan binding failed: {error}"))?;
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::UnitsMaterialized.order_for_engine()
        {
            for index in 0..group_journal.units.len() {
                coding_store
                    .ensure_group_initialization_unit(&group_journal, index)
                    .map_err(|error| format!("persist group unit failed: {error}"))?;
            }
            outer = advance_store
                .advance_initialization_phase(
                    &current_record,
                    &outer,
                    AdvanceInitializationPhase::UnitsMaterialized,
                )
                .map_err(|error| format!("checkpoint units materialization failed: {error}"))?;
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
            .get_advance_by_command_id(&input.project_id, &input.issue_id, &input.command_id)
            .map_err(|error| format!("reload final advance record failed: {error}"))?
            .ok_or("advance record disappeared")?;
        let final_journal = advance_store
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
        let _ = plan;
        let _ = final_journal;
        Ok(AdvanceOutcome::Completed {
            record: ready_record,
            attempt_id: persisted_attempt.id.clone(),
            workspace_entry: Self::advance_workspace_entry(&persisted_attempt),
        })
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
    fn advance_replay_outcome(record: AdvanceRecord) -> AdvanceOutcome {
        match (record.attempt_id.clone(), record.workspace_entry.clone()) {
            (Some(attempt_id), Some(workspace_entry)) => AdvanceOutcome::Completed {
                record,
                attempt_id,
                workspace_entry,
            },
            _ => AdvanceOutcome::Replayed { record },
        }
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
