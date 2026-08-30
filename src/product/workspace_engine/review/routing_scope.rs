use super::policy_routing::{GateSnapshotContext, route_outcome};
use super::routing::policy_route_record_values;
use super::*;
use crate::product::lifecycle_store::workspace::PolicyRoutePersist;
use crate::product::work_item_plan_policy::{
    FatalReason, FindingClass, FindingFingerprint, PolicyDiagnostic, ReviewFindingCategory,
    ReviewInvocationScope, ReviewPhase, RunHistory, WorkItemPlanFlowKind,
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

/// Returns the only durable review-cycle identity and phase for a SingleCandidate
/// reviewer invocation. A SingleCandidate review always stays attached to its
/// ReviewerRun node, regardless of any legacy WorkItemPlan node subtype.
pub(super) fn single_candidate_review_cycle(
    active_node_id: Option<&str>,
    run_history: &RunHistory,
) -> Result<(String, ReviewPhase), String> {
    let reviewer_node_id = active_node_id
        .filter(|node_id| !node_id.is_empty() && *node_id != "review_unknown")
        .ok_or_else(|| {
            "active ReviewerRun node is unavailable for SingleCandidate review".to_string()
        })?;
    let cycle_key = format!("review:{reviewer_node_id}");
    let cycle = run_history
        .review_cycles
        .get(&cycle_key)
        .cloned()
        .unwrap_or_default();
    let phase = if cycle.initial_count == 0 && cycle.verification_count == 0 {
        ReviewPhase::Initial
    } else {
        ReviewPhase::Verification
    };
    Ok((cycle_key, phase))
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

        let node_id = self.active_node_id.as_deref().ok_or_else(|| {
            "active ReviewerRun node is unavailable for SingleCandidate review".to_string()
        })?;
        let (_, phase) = single_candidate_review_cycle(Some(node_id), &self.session.run_history)?;
        let scope = if phase == ReviewPhase::Initial {
            match self.session.review_invocation_scope.as_ref() {
                Some(scope @ ReviewInvocationScope::Initial { .. }) => scope.clone(),
                _ => ReviewInvocationScope::initial(self.review_revision_id(node_id, true)),
            }
        } else {
            let repaired_revision_id = self.review_revision_id(node_id, false);
            // Verification scope must bind the durable report produced by compile/evaluate,
            // rather than reusing a prior invocation scope (which may still be Initial after
            // a repair). Missing durable evidence is a protocol failure, fail-closed.
            let mechanical_report_ref = self
                .session
                .mechanical_report_ref
                .clone()
                .ok_or("verification review requires a durable mechanical report")?;
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
                    single_candidate_phase: expected.single_candidate_phase.clone(),
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

    pub(super) fn single_candidate_mechanical_findings(
        &self,
    ) -> Result<Vec<ClassifiedFinding>, RoutingAction> {
        if self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || self.session.single_candidate_phase.clone()
                != Some(crate::product::models::SingleCandidatePhase::Evaluate)
        {
            return Ok(Vec::new());
        }
        let Some(report_ref) = self.session.mechanical_report_ref.as_deref() else {
            return Ok(Vec::new());
        };
        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| RoutingAction::AbortFatal {
                reason: FatalReason::PersistenceFailure,
                diagnostics: vec![PolicyDiagnostic {
                    code: FatalReason::PersistenceFailure.as_code().to_owned(),
                    message: "lifecycle_store unavailable for mechanical report".to_string(),
                    field: Some("mechanical_report_ref".to_string()),
                }],
            })?;
        let scope = SourceStoreScope {
            project_id: self.session.project_id.clone(),
            issue_id: self.session.issue_id.clone(),
            plan_id: self.session.entity_id.clone(),
        };
        let report = WorkItemPlanSourceStore::new(lifecycle.app_paths())
            .get_mechanical_report(&scope, report_ref)
            .map_err(|error| RoutingAction::AbortFatal {
                reason: FatalReason::ProtocolViolation,
                diagnostics: vec![PolicyDiagnostic {
                    code: "mechanical_report_invalid".to_string(),
                    message: format!("mechanical report rejected: {}", error.code()),
                    field: Some("mechanical_report_ref".to_string()),
                }],
            })?;
        Ok(report
            .report
            .findings
            .into_iter()
            .filter(|finding| {
                finding.severity == crate::product::models::WorkItemSplitFindingSeverity::Error
            })
            .map(|finding| ClassifiedFinding {
                class: FindingClass::MechanicalError,
                fingerprint: FindingFingerprint::for_finding(
                    Some(ReviewFindingCategory::Other),
                    FindingClass::MechanicalError,
                    &finding.message,
                    Some(&finding.code),
                ),
                category: Some(ReviewFindingCategory::Other),
                severity: "error".to_string(),
                message: finding.message,
                evidence: Some(report_ref.to_string()),
                required_action: Some(finding.code.clone()),
                contract_field: Some(finding.code),
            })
            .collect())
    }

    pub(super) fn persist_policy_route(
        &self,
        expected: Option<&WorkspaceSessionRecord>,
        invocation: &ReviewInvocationScope,
        action: &RoutingAction,
        history: &RunHistory,
    ) -> Result<Option<WorkspaceSessionRecord>, ProductStoreError> {
        let (mut status, mut gate, diagnostics) = policy_route_record_values(action);
        if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate
            && self.session.run_policy == RunPolicy::Interactive
            && matches!(action, RoutingAction::ContinueToCompleted)
        {
            status = WorkspaceSessionStatus::WaitingForHuman;
            gate = Some(self.single_candidate_approval_gate(history));
        }
        let scope = self.policy_scope_for_action(
            invocation,
            action,
            expected.and_then(|record| record.review_invocation_scope.as_ref()),
        );
        let Some(store) = &self.lifecycle_store else {
            return Ok(None);
        };
        let Some(expected) = expected else {
            return Ok(None);
        };
        store
            .compare_and_save_policy_route(
                expected,
                PolicyRoutePersist {
                    status,
                    single_candidate_phase: self.single_candidate_phase_for_action(action),
                    run_history: history.clone(),
                    scope,
                    gate,
                    diagnostics,
                    repair_reservation: expected.repair_reservation.clone(),
                    provider_start_ledger: expected.provider_start_ledger.clone(),
                },
            )
            .map(Some)
    }

    pub(super) fn apply_policy_route_state(
        &mut self,
        invocation: &ReviewInvocationScope,
        action: &RoutingAction,
        history: RunHistory,
    ) {
        let (mut status, mut gate, diagnostics) = policy_route_record_values(action);
        if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate
            && self.session.run_policy == RunPolicy::Interactive
            && matches!(action, RoutingAction::ContinueToCompleted)
        {
            status = WorkspaceSessionStatus::WaitingForHuman;
            gate = Some(self.single_candidate_approval_gate(&history));
        }
        self.session.run_history = history;
        self.session.review_invocation_scope =
            self.policy_scope_for_action(invocation, action, None);
        self.session.human_gate_snapshot = gate;
        self.session.policy_diagnostics = diagnostics;
        self.session.session_status = status;
        if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate {
            self.session.single_candidate_phase = self.single_candidate_phase_for_action(action);
        }
    }

    pub(super) fn refresh_policy_state(&mut self, record: &WorkspaceSessionRecord) {
        self.session.session_status = record.status.clone();
        self.session.run_history = record.run_history.clone();
        self.session.review_invocation_scope = record.review_invocation_scope.clone();
        self.session.human_gate_snapshot = record.human_gate_snapshot.clone();
        self.session.repair_reservation = record.repair_reservation.clone();
        self.session.policy_diagnostics = record.policy_diagnostics.clone();
        self.session.provider_start_ledger = record.provider_start_ledger.clone();
        // stale worker CAS retry 重新评估前必须完整恢复 SingleCandidate immutable refs；
        // 否则 durable scope 已更新而内存仍指向旧 IR/report，会被误判为 protocol violation。
        self.session.work_item_plan_source_revision_ref =
            record.work_item_plan_source_revision_ref.clone();
        self.session.plan_candidate_ir_ref = record.plan_candidate_ir_ref.clone();
        self.session.mechanical_report_ref = record.mechanical_report_ref.clone();
        self.session.publication_provenance_ref = record.publication_provenance_ref.clone();
        self.session.single_candidate_phase = record.single_candidate_phase.clone();
    }

    pub(super) fn single_candidate_approval_gate(&self, history: &RunHistory) -> HumanGateSnapshot {
        HumanGateSnapshot {
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            attempts_used: history
                .repairs_used
                .saturating_add(history.manual_repairs_used),
            manual_repairs_remaining: RunBudgets::default()
                .max_manual_repairs
                .saturating_sub(history.manual_repairs_used),
            // 阶段 1 的 gate schema 是唯一的人工审批快照载体；有效候选无 finding，
            // 仍用其既有 trigger 以避免新增决策协议。
            trigger: HumanReason::NativeHumanRequired,
            resumable: true,
        }
    }

    pub(super) fn single_candidate_phase_for_action(
        &self,
        action: &RoutingAction,
    ) -> Option<crate::product::models::SingleCandidatePhase> {
        use crate::product::models::SingleCandidatePhase;
        if self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate {
            return None;
        }
        match action {
            RoutingAction::ContinueToCompleted => Some(SingleCandidatePhase::Approval),
            RoutingAction::AbortFatal { .. } => Some(SingleCandidatePhase::Failed),
            RoutingAction::TriggerAggregateRepair { .. } => Some(SingleCandidatePhase::Generate),
            RoutingAction::EnterHumanGate { .. } | RoutingAction::StopNeedsHuman { .. } => {
                self.session.single_candidate_phase.clone()
            }
        }
    }

    pub(crate) async fn route_single_candidate_evaluate_without_reviewer(&mut self) {
        let verdict = ReviewVerdict {
            verdict: ReviewVerdictType::Pass,
            comments: "未启用 reviewer；仅执行 SingleCandidate mechanical Evaluate".to_string(),
            summary: "SingleCandidate mechanical Evaluate 已完成，等待 Approval".to_string(),
            findings: Vec::new(),
            review_gate: ReviewGate::UserConfirmAllowed,
            work_item_plan_review: None,
            structured_output_diagnostic: None,
        };
        let node_id = self
            .active_node_id
            .clone()
            .unwrap_or_else(|| "single_candidate_evaluate".to_string());
        let active_node_type = self.active_node_type();
        let round = self.active_review_round().unwrap_or(1);
        if let Some(action) = self.work_item_policy_action(&node_id, &verdict) {
            self.apply_policy_route(action, verdict, active_node_type, round)
                .await;
            return;
        }
        let diagnostics = vec![PolicyDiagnostic {
            code: FatalReason::StateCorruption.as_code().to_string(),
            message: "SingleCandidate Evaluate did not produce a policy route".to_string(),
            field: Some("single_candidate_phase".to_string()),
        }];
        self.persist_single_candidate_terminal_phase(
            crate::product::models::SingleCandidatePhase::Failed,
        );
        self.finish_policy_failure(FatalReason::StateCorruption, diagnostics)
            .await;
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
    use crate::product::work_item_plan_policy::ReviewCycleState;

    #[test]
    fn single_candidate_cycle_is_reviewer_node_scoped_and_counter_derived() {
        let (initial_key, initial_phase) =
            single_candidate_review_cycle(Some("reviewer-node"), &RunHistory::default())
                .expect("reviewer node has an initial cycle");
        assert_eq!(initial_key, "review:reviewer-node");
        assert_eq!(initial_phase, ReviewPhase::Initial);

        let history = RunHistory {
            review_cycles: std::collections::BTreeMap::from([(
                "review:reviewer-node".to_string(),
                ReviewCycleState {
                    initial_count: 1,
                    ..ReviewCycleState::default()
                },
            )]),
            ..RunHistory::default()
        };
        assert_eq!(
            single_candidate_review_cycle(Some("reviewer-node"), &history),
            Ok((
                "review:reviewer-node".to_string(),
                ReviewPhase::Verification
            ))
        );

        let outline_or_batch_history = RunHistory {
            review_cycles: std::collections::BTreeMap::from([(
                "batch:generation-round".to_string(),
                ReviewCycleState {
                    initial_count: 1,
                    verification_count: 1,
                    ..ReviewCycleState::default()
                },
            )]),
            ..RunHistory::default()
        };
        assert_eq!(
            single_candidate_review_cycle(Some("reviewer-node"), &outline_or_batch_history),
            Ok(("review:reviewer-node".to_string(), ReviewPhase::Initial))
        );
        assert!(single_candidate_review_cycle(None, &RunHistory::default()).is_err());
        assert!(
            single_candidate_review_cycle(Some("review_unknown"), &RunHistory::default()).is_err()
        );
    }

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
