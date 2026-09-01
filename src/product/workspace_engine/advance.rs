pub use crate::product::advance_store::{
    AdvanceInput, AdvanceOutcome, AdvanceRecord, AdvanceStatus,
};

use crate::product::advance_store::AdvanceStore;
use crate::product::coding_attempt_store::CodingAttemptStore;

use crate::product::models::{IssueWorkItemPlanStatus, WorkspaceType};
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

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

        // Idempotency is checked before every current-state precondition. A
        // replay remains observable even when original inputs have changed.
        if let Some(record) = advance_store
            .get_advance_by_command_id(&input.project_id, &input.issue_id, &input.command_id)
            .map_err(|error| format!("load advance command record failed: {error}"))?
        {
            return Ok(AdvanceOutcome::Replayed { record });
        }
        if let Some(record) = advance_store
            .get_advance_for_plan(&input.project_id, &input.issue_id, &input.plan_id)
            .map_err(|error| format!("load advance plan record failed: {error}"))?
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

        // Read existing group state solely to fail closed. This task must not
        // create or mutate an attempt, journal, binding, lock, unit, or ledger.
        let coding_store = CodingAttemptStore::new(app_paths);
        if let Some(attempt) = coding_store
            .get_attempt_for_work_item_group(&input.project_id, &input.issue_id, &input.plan_id)
            .map_err(|error| format!("load existing group attempt failed: {error}"))?
        {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_ATTEMPT_ALREADY_EXISTS".to_string(),
                reason: format!("group coding attempt already exists: {}", attempt.id),
            });
        }
        match coding_store.get_group_initialization(
            &input.project_id,
            &input.issue_id,
            &input.plan_id,
        ) {
            Ok(journal) => {
                return Ok(AdvanceOutcome::Rejected {
                    record: None,
                    code: "ADVANCE_INITIALIZATION_ALREADY_EXISTS".to_string(),
                    reason: format!(
                        "group initialization journal already exists: {}",
                        journal.id
                    ),
                });
            }
            Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => {}
            Err(error) => {
                return Err(format!(
                    "load existing group initialization failed: {error}"
                ));
            }
        }

        Ok(AdvanceOutcome::Rejected {
            record: None,
            code: "ADVANCE_NOT_WIRED".to_string(),
            reason: format!(
                "advance preflight passed for active plan revision {active_revision_id}; durable initialization is not wired yet"
            ),
        })
    }

    #[allow(dead_code)]
    pub(crate) fn advance_record_store(&self) -> Option<AdvanceStore> {
        self.lifecycle_store
            .as_ref()
            .map(|store| AdvanceStore::new(store.app_paths()))
    }
}
