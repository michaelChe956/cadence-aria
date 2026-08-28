use super::policy_routing::{GateSnapshotContext, route_outcome};
use super::*;
use crate::product::lifecycle_store::workspace::PolicyRoutePersist;
use crate::product::work_item_plan_policy::{
    FatalReason, PolicyDiagnostic, ReviewInvocationScope, ReviewPhase, WorkItemPlanFlowKind,
};
use crate::product::work_item_plan_source_store::{
    SourceStoreError, SourceStoreScope, WorkItemPlanSourceStore,
};

fn scope_for_action(
    durable_scope: Option<&ReviewInvocationScope>,
    invocation: &ReviewInvocationScope,
    action: &RoutingAction,
) -> Option<ReviewInvocationScope> {
    if matches!(
        action,
        RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            ..
        }
    ) {
        return durable_scope.cloned();
    }
    Some(invocation.clone())
}

fn validate_single_candidate_scope(
    scope: ReviewInvocationScope,
    phase: ReviewPhase,
) -> Result<ReviewInvocationScope, RoutingAction> {
    let phase_matches = matches!(
        (phase, &scope),
        (ReviewPhase::Initial, ReviewInvocationScope::Initial { .. })
            | (
                ReviewPhase::Verification,
                ReviewInvocationScope::Verification { .. }
            )
    );
    if !phase_matches {
        return Err(RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            diagnostics: vec![PolicyDiagnostic {
                code: "verification_scope_violation".to_string(),
                message: "review invocation scope phase does not match review cycle phase"
                    .to_string(),
                field: Some("phase".to_string()),
            }],
        });
    }
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

impl WorkspaceEngine {
    /// 当 worker 已持有本 invocation 的 scope 时，必须在 reload durable state
    /// 之前拒绝坏 digest；否则 reload 会静默覆盖该 protocol violation。
    pub(super) fn fail_invalid_single_candidate_scope(
        &mut self,
        expected: Option<&WorkspaceSessionRecord>,
    ) -> Option<RoutingAction> {
        if self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate {
            return None;
        }
        let scope = self.session.review_invocation_scope.clone()?;
        if scope.validate_digest().is_ok() {
            return None;
        }

        let action = RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            diagnostics: vec![PolicyDiagnostic {
                code: "verification_scope_violation".to_string(),
                message: "review invocation scope digest invalid".to_string(),
                field: Some("scope_digest".to_string()),
            }],
        };
        let history = expected
            .map(|record| record.run_history.clone())
            .unwrap_or_else(|| self.session.run_history.clone());
        match self.persist_policy_route(expected, &scope, &action, &history) {
            Ok(record) => {
                if let Some(record) = record.as_ref() {
                    self.refresh_policy_state(record);
                } else {
                    self.apply_policy_route_state(&scope, &action, history);
                }
                Some(action)
            }
            Err(error) => {
                let action = RoutingAction::AbortFatal {
                    reason: FatalReason::PersistenceFailure,
                    diagnostics: vec![PolicyDiagnostic {
                        code: FatalReason::PersistenceFailure.as_code().to_owned(),
                        message: error.to_string(),
                        field: Some("workspace_session".to_owned()),
                    }],
                };
                self.apply_policy_route_state(&scope, &action, history);
                Some(action)
            }
        }
    }

    pub(super) fn policy_scope_for_action(
        &self,
        invocation: &ReviewInvocationScope,
        action: &RoutingAction,
        durable_scope: Option<&ReviewInvocationScope>,
    ) -> Option<ReviewInvocationScope> {
        scope_for_action(
            durable_scope.or(self.session.review_invocation_scope.as_ref()),
            invocation,
            action,
        )
    }

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
        validate_single_candidate_scope(scope, phase)
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

    /// 对 SingleCandidate 的复评，只接受本 invocation scope 精确指向的 immutable
    /// mechanical report，且该 report 必须绑定 scope 所声明的 repaired IR。这里不做
    /// changed-path/region 归因：finding 的身份边界由阶段 1 fingerprint classifier 保持。
    pub(super) fn verified_mechanical_report_ref(
        &self,
        invocation: &ReviewInvocationScope,
    ) -> Result<Option<String>, RoutingAction> {
        let ReviewInvocationScope::Verification {
            mechanical_report_ref,
            repaired_revision_id,
            ..
        } = invocation
        else {
            return Ok(None);
        };

        if self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate {
            // legacy 没有 source-store mechanical report；保持其既有评审路径，但让
            // classifier 仍可确认 invocation 内的 ref 未被调用方替换。
            return Ok(Some(mechanical_report_ref.clone()));
        }

        if self.session.plan_candidate_ir_ref.as_deref() != Some(repaired_revision_id)
            || self.session.mechanical_report_ref.as_deref() != Some(mechanical_report_ref)
        {
            return Err(verification_scope_store_error(
                "verification invocation scope does not match durable IR/report refs".to_string(),
                false,
            ));
        }

        let Some(lifecycle_store) = self.lifecycle_store.as_ref() else {
            return Err(verification_scope_store_error(
                "lifecycle_store unavailable for verification mechanical report".to_string(),
                false,
            ));
        };
        let scope = SourceStoreScope {
            project_id: self.session.project_id.clone(),
            issue_id: self.session.issue_id.clone(),
            plan_id: self.session.entity_id.clone(),
        };
        let source_store = WorkItemPlanSourceStore::new(lifecycle_store.app_paths());
        let report = source_store
            .get_mechanical_report(&scope, mechanical_report_ref)
            .map_err(|error| {
                verification_scope_source_store_error(error, "mechanical_report_ref")
            })?;
        let repaired_ir = source_store
            .get_plan_candidate_ir(&scope, repaired_revision_id)
            .map_err(|error| {
                verification_scope_source_store_error(error, "repaired_revision_id")
            })?;

        if report.ir_id != repaired_ir.id
            || report.source_revision_id != repaired_ir.source_revision_id
            || report.report.source_revision_hash != repaired_ir.ir.source_revision_hash
            || report.report.compiler_version != repaired_ir.ir.compiler_version
        {
            return Err(verification_scope_store_error(
                "verification mechanical report does not match the repaired IR".to_string(),
                false,
            ));
        }

        Ok(Some(mechanical_report_ref.clone()))
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

pub(super) fn route_outcome_from_decision(
    decision: EvaluationDecision,
    policy: RunPolicy,
    history: &RunHistory,
    invocation: &ReviewInvocationScope,
    findings: Vec<ClassifiedFinding>,
) -> RoutingAction {
    let (outcome, outcome_findings, repeated_fingerprints, trigger) = match &decision.outcome {
        PlanOutcome::HumanRequired {
            findings,
            repeated_fingerprints,
            reason,
        } => (
            decision.outcome.clone(),
            findings.clone(),
            repeated_fingerprints.clone(),
            *reason,
        ),
        PlanOutcome::Repairable { findings } => (
            decision.outcome.clone(),
            findings.clone(),
            Vec::new(),
            HumanReason::RepairBudgetExhausted,
        ),
        _ => (
            decision.outcome.clone(),
            findings,
            Vec::new(),
            HumanReason::NativeHumanRequired,
        ),
    };
    route_outcome(
        outcome,
        policy,
        GateSnapshotContext {
            history: history.clone(),
            budgets: RunBudgets::default(),
            invocation: invocation.clone(),
            findings: outcome_findings,
            repeated_fingerprints,
            trigger,
        },
    )
}

fn verification_scope_source_store_error(error: SourceStoreError, field: &str) -> RoutingAction {
    let persistence_failure = matches!(
        error,
        SourceStoreError::Io(_) | SourceStoreError::Json(_) | SourceStoreError::Serialize(_)
    );
    verification_scope_store_error(
        format!("verification {field} rejected: {}", error.code()),
        persistence_failure,
    )
}

fn verification_scope_store_error(message: String, persistence_failure: bool) -> RoutingAction {
    let reason = if persistence_failure {
        FatalReason::PersistenceFailure
    } else {
        FatalReason::ProtocolViolation
    };
    RoutingAction::AbortFatal {
        reason,
        diagnostics: vec![PolicyDiagnostic {
            code: if persistence_failure {
                FatalReason::PersistenceFailure.as_code().to_string()
            } else {
                "verification_scope_violation".to_string()
            },
            message,
            field: Some("review_invocation_scope".to_string()),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_candidate_scope_rejects_initial_scope_during_verification() {
        let result = validate_single_candidate_scope(
            ReviewInvocationScope::initial("revision-001"),
            ReviewPhase::Verification,
        );
        assert!(matches!(
            result,
            Err(RoutingAction::AbortFatal {
                reason: FatalReason::ProtocolViolation,
                ..
            })
        ));
    }

    #[test]
    fn single_candidate_scope_preserves_existing_scope_on_protocol_fatal() {
        let durable_scope = ReviewInvocationScope::initial("durable-revision");
        let replacement_scope = ReviewInvocationScope::initial("replacement-revision");
        let action = RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            diagnostics: Vec::new(),
        };
        assert_eq!(
            scope_for_action(Some(&durable_scope), &replacement_scope, &action),
            Some(durable_scope)
        );
        assert_eq!(scope_for_action(None, &replacement_scope, &action), None);
    }

    #[test]
    fn single_candidate_scope_rejects_verification_scope_during_initial() {
        let result = validate_single_candidate_scope(
            ReviewInvocationScope::verification(
                std::collections::BTreeSet::new(),
                "revision-001",
                "report-001",
            ),
            ReviewPhase::Initial,
        );
        assert!(matches!(
            result,
            Err(RoutingAction::AbortFatal {
                reason: FatalReason::ProtocolViolation,
                ..
            })
        ));
    }
}
