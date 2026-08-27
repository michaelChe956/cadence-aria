use super::*;
use crate::product::lifecycle_store::workspace::PolicyRoutePersist;
use crate::product::work_item_plan_policy::{
    FatalReason, PolicyDiagnostic, ReviewInvocationScope, ReviewPhase, WorkItemPlanFlowKind,
};

impl WorkspaceEngine {
    pub(super) fn policy_invocation(
        &self,
        phase: ReviewPhase,
        cycle_key: &str,
    ) -> Result<ReviewInvocationScope, RoutingAction> {
        if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate {
            return self.single_candidate_policy_invocation(phase, cycle_key);
        }
        Ok(match phase {
            ReviewPhase::Initial => ReviewInvocationScope::initial(cycle_key.to_string()),
            ReviewPhase::Verification => ReviewInvocationScope::verification(
                self.session.run_history.seen_fingerprints.clone(),
                cycle_key.to_string(),
                "legacy_mechanical_report",
            ),
        })
    }

    pub(super) fn single_candidate_policy_invocation(
        &self,
        phase: ReviewPhase,
        cycle_key: &str,
    ) -> Result<ReviewInvocationScope, RoutingAction> {
        let scope = match self.session.review_invocation_scope.clone() {
            Some(scope) => scope,
            // A direct policy invocation may be used by recovery code before
            // the provider-drive hook has run. Construct the initial scope
            // here; the CAS below makes it durable before routing continues.
            None if phase == ReviewPhase::Initial => {
                ReviewInvocationScope::initial(cycle_key.to_string())
            }
            None => {
                return Err(RoutingAction::AbortFatal {
                    reason: FatalReason::ProtocolViolation,
                    diagnostics: vec![PolicyDiagnostic {
                        code: "verification_scope_violation".to_string(),
                        message: "single-candidate review invocation scope is not durable"
                            .to_string(),
                        field: Some("review_invocation_scope".to_string()),
                    }],
                });
            }
        };
        if let Err(error) = scope.validate_digest() {
            return Err(RoutingAction::AbortFatal {
                reason: FatalReason::ProtocolViolation,
                diagnostics: vec![PolicyDiagnostic {
                    code: "verification_scope_violation".to_string(),
                    message: format!("review invocation scope digest invalid: {error}"),
                    field: Some("scope_digest".to_string()),
                }],
            });
        }
        Ok(scope)
    }

    /// 在 reviewer provider 启动前为 SingleCandidate 建立服务端 scope，并以
    /// 与策略路由相同的 CAS 持久化。该入口只写 scope/其余字段原样复用，因而
    /// 不会提前消费 review counter 或改变 stage/history。
    pub(crate) async fn ensure_review_invocation_scope(&mut self) -> Result<(), String> {
        if self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate {
            return Ok(());
        }

        let node_id = self
            .active_node_id
            .clone()
            .unwrap_or_else(|| "review_unknown".to_string());
        let cycle_key = self.review_cycle_key_for_active_node(&node_id);
        let cycle = self
            .session
            .run_history
            .review_cycles
            .get(&cycle_key)
            .cloned()
            .unwrap_or_default();
        let is_initial = cycle.initial_count == 0 && cycle.verification_count == 0;
        let scope = if is_initial {
            match self.session.review_invocation_scope.as_ref() {
                Some(scope @ ReviewInvocationScope::Initial { .. }) => scope.clone(),
                _ => ReviewInvocationScope::initial(self.review_revision_id(&node_id, true)),
            }
        } else {
            let repaired_revision_id = self.review_revision_id(&node_id, false);
            let mechanical_report_ref = self
                .session
                .review_invocation_scope
                .as_ref()
                .and_then(|scope| match scope {
                    ReviewInvocationScope::Verification {
                        mechanical_report_ref,
                        ..
                    } => Some(mechanical_report_ref.clone()),
                    ReviewInvocationScope::Initial { .. } => None,
                })
                .ok_or_else(|| {
                    "verification review requires a durable mechanical report".to_string()
                })
                .map_err(|error| {
                    // The scope is constructed server-side; an absent report is a
                    // protocol failure rather than a provider-controlled fallback.
                    (
                        ReviewInvocationScope::initial(repaired_revision_id.clone()),
                        error,
                    )
                });
            let mechanical_report_ref = match mechanical_report_ref {
                Ok(reference) => reference,
                Err((_, error)) => return Err(error),
            };
            ReviewInvocationScope::verification(
                self.session.run_history.seen_fingerprints.clone(),
                repaired_revision_id,
                mechanical_report_ref,
            )
        };
        scope
            .validate_digest()
            .map_err(|error| format!("review invocation scope digest invalid: {error}"))?;

        if self.session.review_invocation_scope.as_ref() == Some(&scope) {
            return Ok(());
        }
        let Some(store) = self.lifecycle_store.as_ref() else {
            return Err("lifecycle_store unavailable for review invocation scope".to_string());
        };
        let expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| format!("load workspace session for review scope failed: {error}"))?;
        if expected.review_invocation_scope.as_ref() == Some(&scope) {
            self.refresh_policy_state(&expected);
            return Ok(());
        }
        let updated = store
            .compare_and_save_policy_route(
                &expected,
                PolicyRoutePersist {
                    status: expected.status.clone(),
                    run_history: expected.run_history.clone(),
                    scope: Some(scope),
                    gate: expected.human_gate_snapshot.clone(),
                    diagnostics: expected.policy_diagnostics.clone(),
                    repair_reservation: expected.repair_reservation.clone(),
                    provider_start_ledger: expected.provider_start_ledger.clone(),
                },
            )
            .map_err(|error| format!("persist review invocation scope failed: {error}"))?;
        self.refresh_policy_state(&updated);
        Ok(())
    }

    fn review_revision_id(&self, node_id: &str, initial: bool) -> String {
        if !initial
            && let Some(ReviewInvocationScope::Verification {
                repaired_revision_id,
                ..
            }) = self.session.review_invocation_scope.as_ref()
        {
            return repaired_revision_id.clone();
        }
        self.session
            .review_invocation_scope
            .as_ref()
            .map(|scope| match scope {
                ReviewInvocationScope::Initial {
                    initial_revision_id,
                    ..
                } => initial_revision_id.clone(),
                ReviewInvocationScope::Verification {
                    repaired_revision_id,
                    ..
                } => repaired_revision_id.clone(),
            })
            .unwrap_or_else(|| format!("revision:{node_id}"))
    }

    fn review_cycle_key_for_active_node(&self, node_id: &str) -> String {
        match self.active_node_type() {
            Some(TimelineNodeType::WorkItemPlanOutlineReview) => self
                .latest_work_item_plan_outline_candidate()
                .map(|candidate| format!("outline:{}", candidate.outline.id))
                .unwrap_or_else(|_| format!("outline:{node_id}")),
            Some(TimelineNodeType::WorkItemDraftReview) => self
                .current_work_item_draft_candidate_payload()
                .map(|candidate| format!("draft:{}", candidate.draft_record.outline_id))
                .unwrap_or_else(|_| format!("draft:{node_id}")),
            _ => format!("review:{node_id}"),
        }
    }
}
