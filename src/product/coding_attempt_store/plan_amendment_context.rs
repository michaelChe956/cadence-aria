use chrono::Utc;

use crate::product::coding_models::{
    CodingExecutionAttempt, PlanAmendmentContext, PlanAmendmentContextStatus,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::AmendmentResumeTarget;
use crate::product::workspace_engine::canonical_plan_repair_parent_session;

use super::locking::with_exclusive_lock;

/// Durable diagnostic retained when a `PlanAmendmentContext` fails closed.
/// Kept as a sidecar record so the context record itself stays stable as the
/// contract-pinned field set (REQ-GCE-03).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanAmendmentContextDiagnostic {
    pub context_id: String,
    pub group_attempt_id: String,
    pub reason: String,
    pub recorded_at: String,
}

impl super::CodingAttemptStore {
    pub(crate) fn open_plan_amendment_context(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_id: &str,
        finding_id: &str,
        resume_target: AmendmentResumeTarget,
    ) -> Result<PlanAmendmentContext, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        validate_relative_id(unit_id)?;
        validate_relative_id(finding_id)?;
        if !matches!(
            current.status,
            crate::product::coding_models::CodingAttemptStatus::AwaitingPlanAmendment
                | crate::product::coding_models::CodingAttemptStatus::ApplyingPlanAmendment
                | crate::product::coding_models::CodingAttemptStatus::AmendmentApplyFailed
        ) {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_amendment_context_attempt_status",
                id: current.id,
            });
        }
        let path = self.attempt_path(&current.project_id, &current.issue_id, &current.id);
        with_exclusive_lock(&path, || {
            let existing =
                self.find_plan_amendment_context_by_finding_locked(&current, finding_id)?;
            if let Some(context) = existing {
                // 幂等：重复 defect/finding 事件返回原 context，不重复开门。
                return Ok(context);
            }
            let snapshot = self
                .linked_active_plan_repair_snapshot(&current)?
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "coding_linked_plan_repair",
                    id: current.id.clone(),
                })?;
            let amendment_id = snapshot
                .request
                .amendment_id
                .clone()
                .ok_or_else(|| ProductStoreError::IdentityMismatch {
                    kind: "coding_plan_amendment_context_amendment",
                    id: snapshot.request.id,
                })?;
            let lifecycle = LifecycleStore::new(self.paths());
            let parent = canonical_plan_repair_parent_session(
                &lifecycle,
                &current.project_id,
                &current.issue_id,
                &snapshot.request.plan_id,
                &amendment_id,
            )
            .map_err(plan_amendment_context_lineage_error)?;
            let trigger_unit = self
                .list_coding_units(&current.project_id, &current.issue_id, &current.id)?
                .into_iter()
                .find(|unit| unit.id == unit_id)
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "coding_plan_amendment_context_trigger_unit",
                    id: unit_id.to_string(),
                })?;
            let previous_plan_revision_id = self.get_plan_binding(&current)?.bound_plan_revision_id;
            let contexts = self.list_plan_amendment_contexts(&current)?;
            let id = next_sequential_id("coding_plan_amendment_context", contexts.len());
            let now = Utc::now().to_rfc3339();
            let context = PlanAmendmentContext {
                id,
                plan_session_id: parent.id,
                group_attempt_id: current.id.clone(),
                trigger_unit_id: trigger_unit.id,
                trigger_finding_id: finding_id.to_string(),
                previous_plan_revision_id,
                new_plan_revision_id: None,
                resume_target,
                status: PlanAmendmentContextStatus::Open,
                created_at: now.clone(),
                updated_at: now,
            };
            let root =
                self.plan_amendment_contexts_root(&current.project_id, &current.issue_id, &current.id);
            write_json(&root.join(format!("{}.json", context.id)), &context)?;
            Ok(context)
        })
    }

    pub fn find_plan_amendment_context_by_finding(
        &self,
        attempt: &CodingExecutionAttempt,
        finding_id: &str,
    ) -> Result<Option<PlanAmendmentContext>, ProductStoreError> {
        validate_relative_id(finding_id)?;
        self.find_plan_amendment_context_by_finding_locked(attempt, finding_id)
    }

    fn find_plan_amendment_context_by_finding_locked(
        &self,
        attempt: &CodingExecutionAttempt,
        finding_id: &str,
    ) -> Result<Option<PlanAmendmentContext>, ProductStoreError> {
        let mut matching = self
            .list_plan_amendment_contexts(attempt)?
            .into_iter()
            .filter(|context| context.trigger_finding_id == finding_id);
        let found = matching.next();
        if found.is_some() && matching.next().is_some() {
            return Err(ProductStoreError::Ambiguous {
                kind: "coding_plan_amendment_context_finding",
                id: finding_id.to_string(),
            });
        }
        Ok(found)
    }

    pub fn list_plan_amendment_contexts(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Vec<PlanAmendmentContext>, ProductStoreError> {
        let root =
            self.plan_amendment_contexts_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let mut contexts = Vec::new();
        for path in super::json_file_paths(&root)? {
            let context: PlanAmendmentContext = read_json(&path)?;
            if context.group_attempt_id != attempt.id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_plan_amendment_context",
                    id: context.id,
                });
            }
            contexts.push(context);
        }
        contexts.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(contexts)
    }

    /// Locate the open amendment gate host context for an original plan
    /// session. Used by the workspace decision routing to validate that a
    /// `human_gate_feedback` on the original session may reopen the gate in
    /// amendment context (REQ-GCE-03 scenario 2).
    pub fn find_open_plan_amendment_context_for_plan_session(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        plan_session_id: &str,
    ) -> Result<Option<PlanAmendmentContext>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        validate_relative_id(plan_session_id)?;
        let Some(attempt) = self.get_attempt_for_work_item_group(project_id, issue_id, plan_id)?
        else {
            return Ok(None);
        };
        let mut matching = self
            .list_plan_amendment_contexts(&attempt)?
            .into_iter()
            .filter(|context| {
                context.plan_session_id == plan_session_id
                    && matches!(
                        context.status,
                        PlanAmendmentContextStatus::Open
                            | PlanAmendmentContextStatus::Applying
                    )
            });
        let found = matching.next();
        if found.is_some() && matching.next().is_some() {
            return Err(ProductStoreError::Ambiguous {
                kind: "coding_plan_amendment_context_session",
                id: plan_session_id.to_string(),
            });
        }
        Ok(found)
    }

    pub(crate) fn transition_plan_amendment_context_to_applying(
        &self,
        attempt: &CodingExecutionAttempt,
        context_id: &str,
    ) -> Result<PlanAmendmentContext, ProductStoreError> {
        self.update_plan_amendment_context_status(
            attempt,
            context_id,
            &[
                (PlanAmendmentContextStatus::Open, PlanAmendmentContextStatus::Applying),
                (PlanAmendmentContextStatus::Applying, PlanAmendmentContextStatus::Applying),
            ],
            None,
        )
    }

    pub(crate) fn revert_plan_amendment_context_to_open(
        &self,
        attempt: &CodingExecutionAttempt,
        context_id: &str,
    ) -> Result<PlanAmendmentContext, ProductStoreError> {
        self.update_plan_amendment_context_status(
            attempt,
            context_id,
            &[(PlanAmendmentContextStatus::Applying, PlanAmendmentContextStatus::Open)],
            None,
        )
    }

    pub(crate) fn complete_plan_amendment_context(
        &self,
        attempt: &CodingExecutionAttempt,
        context_id: &str,
        new_plan_revision_id: &str,
        resume_target: &AmendmentResumeTarget,
    ) -> Result<PlanAmendmentContext, ProductStoreError> {
        let path = self
            .plan_amendment_contexts_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join(format!("{context_id}.json"));
        with_exclusive_lock(
            &self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
            || {
                let mut context: PlanAmendmentContext = read_json(&path)?;
                if context.group_attempt_id != attempt.id {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "coding_plan_amendment_context",
                        id: context.id,
                    });
                }
                match context.status {
                    PlanAmendmentContextStatus::Applied => return Ok(context),
                    PlanAmendmentContextStatus::Open | PlanAmendmentContextStatus::Applying => {}
                    PlanAmendmentContextStatus::FailedClosed => {
                        return Err(ProductStoreError::IdentityMismatch {
                            kind: "coding_plan_amendment_context_terminal",
                            id: context.id,
                        });
                    }
                }
                context.new_plan_revision_id = Some(new_plan_revision_id.to_string());
                context.resume_target = resume_target.clone();
                context.status = PlanAmendmentContextStatus::Applied;
                context.updated_at = Utc::now().to_rfc3339();
                write_json(&path, &context)?;
                Ok(context)
            },
        )
    }

    pub(crate) fn fail_closed_plan_amendment_context(
        &self,
        attempt: &CodingExecutionAttempt,
        context_id: &str,
        diagnostic: &str,
    ) -> Result<PlanAmendmentContext, ProductStoreError> {
        let context = self.update_plan_amendment_context_status(
            attempt,
            context_id,
            &[
                (PlanAmendmentContextStatus::Open, PlanAmendmentContextStatus::FailedClosed),
                (
                    PlanAmendmentContextStatus::Applying,
                    PlanAmendmentContextStatus::FailedClosed,
                ),
                (
                    PlanAmendmentContextStatus::FailedClosed,
                    PlanAmendmentContextStatus::FailedClosed,
                ),
            ],
            Some(diagnostic),
        )?;
        Ok(context)
    }

    pub fn get_plan_amendment_context_diagnostic(
        &self,
        attempt: &CodingExecutionAttempt,
        context_id: &str,
    ) -> Result<Option<PlanAmendmentContextDiagnostic>, ProductStoreError> {
        let path = self
            .plan_amendment_contexts_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join(format!("{context_id}.diagnostic.json"));
        if !super::path_exists(&path)? {
            return Ok(None);
        }
        let diagnostic: PlanAmendmentContextDiagnostic = read_json(&path)?;
        if diagnostic.context_id != context_id || diagnostic.group_attempt_id != attempt.id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_amendment_context_diagnostic",
                id: context_id.to_string(),
            });
        }
        Ok(Some(diagnostic))
    }

    fn update_plan_amendment_context_status(
        &self,
        attempt: &CodingExecutionAttempt,
        context_id: &str,
        transitions: &[(PlanAmendmentContextStatus, PlanAmendmentContextStatus)],
        diagnostic: Option<&str>,
    ) -> Result<PlanAmendmentContext, ProductStoreError> {
        validate_relative_id(context_id)?;
        let context_path = self
            .plan_amendment_contexts_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join(format!("{context_id}.json"));
        with_exclusive_lock(
            &self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
            || {
                let mut context: PlanAmendmentContext = read_json(&context_path)?;
                if context.group_attempt_id != attempt.id {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "coding_plan_amendment_context",
                        id: context.id,
                    });
                }
                let from = context.status.clone();
                let mut next_status = None;
                for (allowed_from, to) in transitions {
                    if &from == allowed_from {
                        next_status = Some(to.clone());
                        break;
                    }
                }
                let Some(next_status) = next_status else {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "coding_plan_amendment_context_transition",
                        id: context.id,
                    });
                };
                if from != next_status || diagnostic.is_some() {
                    context.status = next_status;
                    context.updated_at = Utc::now().to_rfc3339();
                    write_json(&context_path, &context)?;
                }
                if let Some(reason) = diagnostic {
                    let record = PlanAmendmentContextDiagnostic {
                        context_id: context.id.clone(),
                        group_attempt_id: attempt.id.clone(),
                        reason: reason.to_string(),
                        recorded_at: Utc::now().to_rfc3339(),
                    };
                    write_json(
                        &self
                            .plan_amendment_contexts_root(
                                &attempt.project_id,
                                &attempt.issue_id,
                                &attempt.id,
                            )
                            .join(format!("{}.diagnostic.json", context.id)),
                        &record,
                    )?;
                }
                Ok(context)
            },
        )
    }
}

fn plan_amendment_context_lineage_error(
    error: crate::product::plan_repair::PlanRepairError,
) -> ProductStoreError {
    match error {
        crate::product::plan_repair::PlanRepairError::Store(error) => error,
        error => ProductStoreError::IdentityMismatch {
            kind: "coding_plan_amendment_context_lineage",
            id: format!("{error:?}"),
        },
    }
}
