#[path = "routing_scope.rs"]
mod routing_scope;
use super::feedback::format_review_feedback;
use super::policy_routing::RoutingAction;
use super::*;
use crate::product::lifecycle_store::workspace::PolicyRoutePersist;
use crate::product::work_item_plan_policy::{
    ClassificationError, ClassifiedFinding, EvaluationDecision, FatalReason, FindingClass,
    FindingFingerprint, HumanReason, PlanOutcome, PolicyDiagnostic, ReviewCycleState,
    ReviewEvaluationInput, ReviewFindingCategory, ReviewInvocationScope, ReviewPhase, RunBudgets,
    RunHistory, RunHistoryDelta, RunPolicy, WorkItemPlanFlowKind, classify_review, evaluate,
};
use routing_scope::{route_outcome_from_decision, single_candidate_review_cycle};
impl WorkspaceEngine {
    pub(crate) async fn complete_review(
        &mut self,
        completion: ProviderCompletion,
        verdict: ReviewVerdict,
    ) {
        // 终态会话丢弃迟到评审 verdict；但重开中的 amendment 门（REQ-GCE-03
        // 场景二）durable phase 仍为 Completed、会话状态已回到 WaitingForHuman，
        // 其重启评审的 Pass 必须继续走 policy route 重建 Approval 门（I-1），
        // 不得被本守卫当作已完结会话丢弃。
        if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate
            && matches!(
                self.session.single_candidate_phase,
                Some(
                    crate::product::models::SingleCandidatePhase::Completed
                        | crate::product::models::SingleCandidatePhase::Failed
                )
            )
            && !self.is_reopened_amendment_gate()
        {
            return;
        }
        let node_id = self
            .active_node_id
            .clone()
            .unwrap_or_else(|| "review_unknown".to_string());
        let round = self.active_review_round().unwrap_or(1);
        let active_node_type = self.active_node_type();
        self.record_review_message(completion.readable_output);
        self.latest_review_verdict = Some(verdict.clone());
        let reviewer = self
            .active_node_agent()
            .or_else(|| self.session.reviewer_provider.clone());
        let _ = self
            .persist_review_verdict(
                &node_id,
                serde_json::to_value(&verdict).unwrap_or(serde_json::Value::Null),
            )
            .await;
        let _ = self
            .event_tx
            .send(review_complete_event_from_verdict(
                node_id.clone(),
                round,
                &verdict,
            ))
            .await;
        self.update_timeline_node(
            &node_id,
            TimelineNodeStatus::Completed,
            Some(verdict.summary.clone()),
        )
        .await;
        let artifact_verdict = match &verdict.review_gate {
            ReviewGate::RequiresRevision => ReviewVerdictType::Revise,
            ReviewGate::UserConfirmAllowed if verdict.verdict == ReviewVerdictType::Pass => {
                ReviewVerdictType::Pass
            }
            ReviewGate::UserConfirmAllowed | ReviewGate::UserTriageRequired => {
                ReviewVerdictType::NeedsHuman
            }
        };
        self.mark_latest_artifact_reviewed(reviewer, Some(artifact_verdict));

        // Every WorkItemPlan review, including a plan-repair candidate review,
        // must first consume the durable policy route.  The legacy dispatch
        // below still reaches the plan-repair protocol for an allowed outcome,
        // but it must never bypass its CAS, cycle budget, or terminal matrix.
        if let Some(action) = self.work_item_policy_action(&node_id, &verdict) {
            self.apply_policy_route(action, verdict, active_node_type, round)
                .await;
            return;
        }

        self.route_legacy_review(verdict, active_node_type, round, false)
            .await;
    }

    /// The policy layer is deliberately restricted to the existing WorkItemPlan
    /// path. Story/design and the other workspace routes retain their byte-level
    /// legacy decisions and do not acquire a second workflow.
    pub(crate) fn work_item_policy_action(
        &mut self,
        node_id: &str,
        verdict: &ReviewVerdict,
    ) -> Option<RoutingAction> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan {
            return None;
        }

        // A stale websocket worker must not apply the action it evaluated
        // against an older record. Re-read durable state and evaluate again.
        for attempt in 0..=1 {
            let expected = self
                .lifecycle_store
                .as_ref()
                .and_then(|store| store.get_workspace_session(&self.session.session_id).ok());
            if let Some(action) = self.fail_invalid_single_candidate_scope(expected.as_ref()) {
                return Some(action);
            }
            if let Some(record) = expected.as_ref() {
                self.refresh_policy_state(record);
            }
            let (invocation, action, history) =
                self.evaluate_work_item_policy_route(node_id, verdict)?;

            #[cfg(test)]
            if let Some(hook) = self.policy_route_before_persist.take()
                && let Some(store) = self.lifecycle_store.as_ref()
            {
                hook(store, &self.session.session_id);
            }

            match self.persist_policy_route(expected.as_ref(), &invocation, &action, &history) {
                Ok(record) => {
                    if let Some(record) = record.as_ref() {
                        self.refresh_policy_state(record);
                    } else {
                        self.apply_policy_route_state(&invocation, &action, history);
                    }
                    return Some(action);
                }
                Err(ProductStoreError::Conflict { .. }) if attempt == 0 => {
                    continue;
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
                    self.apply_policy_route_state(
                        &invocation,
                        &action,
                        self.session.run_history.clone(),
                    );
                    return Some(action);
                }
            }
        }
        unreachable!("the bounded CAS retry loop always returns")
    }
    fn evaluate_work_item_policy_route(
        &self,
        node_id: &str,
        verdict: &ReviewVerdict,
    ) -> Option<(ReviewInvocationScope, RoutingAction, RunHistory)> {
        let envelope_value = serde_json::to_value(verdict).ok()?;
        let (cycle_key, single_candidate_phase) =
            if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate {
                match single_candidate_review_cycle(Some(node_id), &self.session.run_history) {
                    Ok((cycle_key, phase)) => (cycle_key, Some(phase)),
                    Err(message) => {
                        let scope =
                            self.session
                                .review_invocation_scope
                                .clone()
                                .unwrap_or_else(|| {
                                    ReviewInvocationScope::initial("unavailable-reviewer-node")
                                });
                        return Some((
                            scope,
                            RoutingAction::AbortFatal {
                                reason: FatalReason::ProtocolViolation,
                                diagnostics: vec![PolicyDiagnostic {
                                    code: "verification_scope_violation".to_string(),
                                    message,
                                    field: Some("active_node_id".to_string()),
                                }],
                            },
                            self.session.run_history.clone(),
                        ));
                    }
                }
            } else {
                (self.review_cycle_key(node_id, verdict), None)
            };
        let cycle = self
            .session
            .run_history
            .review_cycles
            .get(&cycle_key)
            .cloned()
            .unwrap_or_default();
        let phase = single_candidate_phase.unwrap_or(
            if cycle.initial_count == 0 && cycle.verification_count == 0 {
                ReviewPhase::Initial
            } else {
                ReviewPhase::Verification
            },
        );
        let invocation = match self.policy_invocation(phase, &cycle_key) {
            Ok(scope) => scope,
            Err(action) => {
                let scope = self
                    .session
                    .review_invocation_scope
                    .clone()
                    .unwrap_or_else(|| ReviewInvocationScope::initial(cycle_key.clone()));
                return Some((scope, action, self.session.run_history.clone()));
            }
        };
        if let Some(diagnostic) = verdict.structured_output_diagnostic.as_ref() {
            let fatal = match diagnostic.code.as_str() {
                "unknown_finding_category" => Some((
                    FatalReason::UnknownCategory,
                    diagnostic.code.clone(),
                    diagnostic.message.clone(),
                )),
                "unknown_class_hint" => Some((
                    FatalReason::UnknownClassHint,
                    diagnostic.code.clone(),
                    diagnostic.message.clone(),
                )),
                "verification_scope_violation"
                    if phase == ReviewPhase::Verification
                        && self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate =>
                {
                    Some((
                        FatalReason::ProtocolViolation,
                        diagnostic.code.clone(),
                        diagnostic.message.clone(),
                    ))
                }
                _ => None,
            };
            if let Some((reason, code, message)) = fatal {
                return Some((
                    invocation,
                    RoutingAction::AbortFatal {
                        reason,
                        diagnostics: vec![PolicyDiagnostic {
                            code,
                            message,
                            field: Some("findings".to_string()),
                        }],
                    },
                    self.session.run_history.clone(),
                ));
            }
        }
        let raw_verdict = verdict.verdict.clone();
        let envelope =
            match crate::product::workspace_engine::parse_review_envelope(&envelope_value) {
                Ok(envelope) => envelope,
                Err(error) => {
                    return Some((
                        invocation,
                        RoutingAction::AbortFatal {
                            reason: FatalReason::InvalidStructuredOutput,
                            diagnostics: vec![PolicyDiagnostic {
                                code: error.code().to_string(),
                                message: format!("review envelope rejected: {error:?}"),
                                field: Some("findings".to_string()),
                            }],
                        },
                        self.session.run_history.clone(),
                    ));
                }
            };
        let verified_mechanical_report_ref = match self.verified_mechanical_report_ref(&invocation)
        {
            Ok(reference) => reference,
            Err(action) => return Some((invocation, action, self.session.run_history.clone())),
        };
        let mut findings = match classify_review(
            &envelope,
            &invocation,
            verified_mechanical_report_ref.as_deref(),
        ) {
            Ok(findings) => findings,
            Err(error) => {
                return Some((
                    invocation,
                    classification_error_action(error),
                    self.session.run_history.clone(),
                ));
            }
        };
        let legacy_work_item_plan_repair = legacy_work_item_plan_repair_finding(verdict);
        if let Some(finding) = legacy_work_item_plan_repair.clone() {
            // ReviseBatch and PlanReopenRequired retain their dedicated legacy
            // execution mechanisms, but are policy-level repair candidates:
            // their review cycle must consume the same automatic budget as all
            // other repairable findings before that mechanism is entered.
            findings.push(finding);
        }
        if matches!(raw_verdict, ReviewVerdictType::Revise)
            && self.plan_repair_session_state().is_none()
            && findings.iter().all(|finding| {
                finding.category.is_none() && finding.class != FindingClass::Repairable
            })
        {
            return None;
        }
        let mechanical = match self.single_candidate_mechanical_findings() {
            Ok(findings) => findings,
            Err(action) => return Some((invocation, action, self.session.run_history.clone())),
        };
        let mut decision = if cycle.initial_count == 1 && cycle.verification_count >= 1 {
            EvaluationDecision {
                outcome: PlanOutcome::HumanRequired {
                    findings: findings.clone(),
                    repeated_fingerprints: Vec::new(),
                    reason: HumanReason::RepairBudgetExhausted,
                },
                history_delta: Default::default(),
            }
        } else {
            evaluate(
                &ReviewEvaluationInput {
                    mechanical: &mechanical,
                    review: &findings,
                    cycle_key: &cycle_key,
                    phase,
                    invocation: &invocation,
                },
                &self.session.run_history,
                &RunBudgets::default(),
            )
        };
        if raw_verdict == ReviewVerdictType::NeedsHuman
            && legacy_work_item_plan_repair.is_none()
            && !matches!(
                decision.outcome,
                PlanOutcome::HumanRequired {
                    reason: HumanReason::VerificationNewFindings,
                    ..
                }
            )
        {
            // 结构化 needs_human 仍须作为原生人工需求交给策略层；仅
            // ReviseBatch/PlanReopenRequired 是可预算的专用返修裁决。复评
            // scope 外 finding 已由 evaluator 降级，必须保留其稳定原因。
            decision.outcome = PlanOutcome::HumanRequired {
                findings: findings.clone(),
                repeated_fingerprints: Vec::new(),
                reason: HumanReason::NativeHumanRequired,
            };
        }
        let mut history = self.session.run_history.clone();
        if let Err(error) = decision
            .history_delta
            .merge_into(&mut history, &RunBudgets::default())
        {
            let action = route_outcome_from_decision(
                EvaluationDecision {
                    outcome: error,
                    history_delta: Default::default(),
                },
                self.session.run_policy,
                &history,
                &invocation,
                findings,
            );
            return Some((invocation, action, history));
        }
        let action = route_outcome_from_decision(
            decision,
            self.session.run_policy,
            &history,
            &invocation,
            findings,
        );
        if matches!(action, RoutingAction::TriggerAggregateRepair { .. }) {
            // Keep the cross-cycle session total for observability, while
            // consuming the automatic-repair budget only inside this artifact
            // cycle. Both increments are merged atomically before CAS.
            let repair_delta = RunHistoryDelta {
                repairs_used_delta: 1,
                review_cycles_to_add: std::collections::BTreeMap::from([(
                    cycle_key,
                    ReviewCycleState {
                        repairs_used: 1,
                        ..ReviewCycleState::default()
                    },
                )]),
                ..RunHistoryDelta::default()
            };
            if let Err(outcome) = repair_delta.merge_into(&mut history, &RunBudgets::default()) {
                let PlanOutcome::Fatal {
                    reason,
                    diagnostics,
                } = outcome
                else {
                    unreachable!("history delta merge failures are always fatal")
                };
                return Some((
                    invocation,
                    RoutingAction::AbortFatal {
                        reason,
                        diagnostics,
                    },
                    history,
                ));
            }
        }
        Some((invocation, action, history))
    }
    fn review_cycle_key(&self, node_id: &str, verdict: &ReviewVerdict) -> String {
        match self.active_node_type() {
            Some(TimelineNodeType::WorkItemPlanOutlineReview) => self
                .latest_work_item_plan_outline_candidate()
                .map(|candidate| format!("outline:{}", candidate.outline.id))
                .unwrap_or_else(|_| format!("outline:{node_id}")),
            Some(TimelineNodeType::WorkItemDraftReview) => self
                .current_work_item_draft_candidate_payload()
                .map(|candidate| format!("draft:{}", candidate.draft_record.outline_id))
                .unwrap_or_else(|_| format!("draft:{node_id}")),
            Some(TimelineNodeType::WorkItemBatchReview) => verdict
                .work_item_plan_review
                .as_ref()
                // A batch rewrite creates a replacement batch record but keeps
                // the same generation round. The round is therefore the
                // durable review-cycle identity: keying by the replacement
                // batch id would reset its repair budget and recreate the
                // revise_batch loop after every regeneration.
                .map(|review| format!("batch:{}", review.generation_round_id))
                .unwrap_or_else(|| format!("batch:{node_id}")),
            _ => format!("review:{node_id}"),
        }
    }
    /// 未启用 reviewer 时，SingleCandidate 的真实机械报告仍必须经阶段 1
    /// evaluate/route_outcome/CAS 进入 Approval；不得由 provider 完成回调直接跳阶段。
    pub(super) async fn apply_policy_route(
        &mut self,
        action: RoutingAction,
        verdict: ReviewVerdict,
        active_node_type: Option<TimelineNodeType>,
        round: u32,
    ) {
        let stage_before = self.session.stage.clone();
        match action {
            RoutingAction::ContinueToCompleted
                if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate =>
            {
                if self.session.run_policy == RunPolicy::AutoIfValid {
                    self.enter_policy_valid_work_item_plan_compile().await;
                } else {
                    self.enter_human_confirm(Some(
                        "SingleCandidate 已通过 Evaluate，等待 Approval".to_string(),
                    ))
                    .await;
                }
            }
            RoutingAction::ContinueToCompleted => {
                self.route_legacy_review(verdict, active_node_type, round, true)
                    .await;
            }
            RoutingAction::TriggerAggregateRepair { .. }
                if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate =>
            {
                self.request_provider_run(ProviderRunKind::WorkItemPlanSingleCandidateAuthor)
                    .await;
            }
            RoutingAction::TriggerAggregateRepair { .. } => {
                if self.is_legacy_work_item_plan_repair(&verdict) {
                    self.run_legacy_work_item_plan_repair(verdict).await;
                } else {
                    // The current legacy author decision remains the only provider
                    // entry point. This preserves pass/revise parity while making
                    // the policy decision explicit and bounded.
                    self.route_legacy_review(verdict, active_node_type, round, true)
                        .await;
                }
            }
            RoutingAction::EnterHumanGate { .. } => {
                self.enter_human_confirm(Some(verdict.summary)).await;
            }
            RoutingAction::StopNeedsHuman { .. } => {
                self.stop_needs_human(verdict.summary).await;
            }
            RoutingAction::AbortFatal {
                reason,
                diagnostics,
            } => {
                self.finish_policy_failure(reason, diagnostics).await;
            }
        }
        if self.session.stage != stage_before {
            let _ = self.record_policy_transition();
        }
    }
    fn is_legacy_work_item_plan_repair(&self, verdict: &ReviewVerdict) -> bool {
        self.session.workspace_type == WorkspaceType::WorkItemPlan
            && verdict
                .work_item_plan_review
                .as_ref()
                .is_some_and(|review| {
                    matches!(
                        review.verdict,
                        WorkItemPlanReviewVerdict::ReviseBatch
                            | WorkItemPlanReviewVerdict::PlanReopenRequired
                    )
                })
    }
    pub(crate) async fn request_provider_run(&mut self, kind: ProviderRunKind) {
        let _ = self
            .event_tx
            .send(EngineEvent::ProviderRunRequested {
                kind,
                node_id: self.active_timeline_node_id(),
            })
            .await;
    }
    async fn run_legacy_work_item_plan_repair(&mut self, verdict: ReviewVerdict) {
        let Some(review) = verdict.work_item_plan_review.as_ref() else {
            return;
        };
        let result = match review.verdict {
            WorkItemPlanReviewVerdict::ReviseBatch => self
                .rewrite_current_work_item_batch()
                .await
                .map(provider_run_kind_for_batch_outcome),
            WorkItemPlanReviewVerdict::PlanReopenRequired => self
                .prepare_work_item_plan_outline_revision(
                    None,
                    WorkItemPlanOutlineRevisionSource::ReviewDecision,
                    OutlineRevisionPersistencePolicy::RequireActiveRound,
                )
                .await
                .map(|feedback| Some(ProviderRunKind::WorkItemPlanOutlineRevision { feedback })),
            _ => return,
        };
        match result {
            Ok(Some(kind)) => self.request_provider_run(kind).await,
            Ok(None) => {}
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.enter_human_confirm(Some("自动返修准备失败，等待人工处理".to_string()))
                    .await;
            }
        }
    }

    pub(crate) fn record_manual_policy_repair(&mut self) -> Result<(), String> {
        if self.session.run_history.manual_repairs_used >= RunBudgets::default().max_manual_repairs
        {
            return Err("manual repair budget is exhausted".to_string());
        }
        self.update_policy_history(|history| {
            history.manual_repairs_used = history
                .manual_repairs_used
                .checked_add(1)
                .ok_or_else(|| "manual repair counter overflow".to_string())?;
            Ok(())
        })
    }
    fn record_policy_transition(&mut self) -> Result<(), String> {
        if self.session.run_history.transitions_used >= RunBudgets::default().max_transitions {
            return Err("stage transition budget is exhausted".to_string());
        }
        self.update_policy_history(|history| {
            history.transitions_used = history
                .transitions_used
                .checked_add(1)
                .ok_or_else(|| "stage transition counter overflow".to_string())?;
            Ok(())
        })
    }
    fn update_policy_history(
        &mut self,
        update: impl Fn(&mut RunHistory) -> Result<(), String>,
    ) -> Result<(), String> {
        let Some(store) = self.lifecycle_store.as_ref() else {
            let mut history = self.session.run_history.clone();
            update(&mut history)?;
            self.session.run_history = history;
            return Ok(());
        };
        let record = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        let mut history = record.run_history.clone();
        update(&mut history)?;
        let saved = store
            .compare_and_save_policy_route(
                &record,
                PolicyRoutePersist {
                    status: record.status.clone(),
                    single_candidate_phase: record.single_candidate_phase.clone(),
                    run_history: history,
                    scope: record.review_invocation_scope.clone(),
                    gate: record.human_gate_snapshot.clone(),
                    diagnostics: record.policy_diagnostics.clone(),
                    repair_reservation: record.repair_reservation.clone(),
                    provider_start_ledger: record.provider_start_ledger.clone(),
                },
            )
            .map_err(|error| error.to_string())?;
        self.refresh_policy_state(&saved);
        Ok(())
    }
    async fn route_legacy_review(
        &mut self,
        verdict: ReviewVerdict,
        active_node_type: Option<TimelineNodeType>,
        round: u32,
        policy_valid: bool,
    ) {
        match active_node_type {
            Some(TimelineNodeType::WorkItemPlanOutlineReview)
                if self.plan_repair_session_state().is_some_and(|snapshot| {
                    snapshot.stage == PlanRepairSessionStage::PlanReview
                }) =>
            {
                self.route_plan_repair_candidate_review(verdict).await;
            }
            Some(TimelineNodeType::WorkItemPlanOutlineReview) => {
                self.route_work_item_plan_outline_review(verdict).await;
            }
            Some(TimelineNodeType::WorkItemDraftReview) => {
                self.route_work_item_draft_review_with_policy_valid(verdict, policy_valid)
                    .await;
            }
            Some(TimelineNodeType::WorkItemBatchReview) => {
                self.route_work_item_batch_review_with_policy_valid(verdict, policy_valid)
                    .await;
            }
            _ if matches!(
                self.session.workspace_type,
                WorkspaceType::Story | WorkspaceType::Design
            ) =>
            {
                self.route_review_report_to_author_confirm(&verdict).await;
            }
            _ => match &verdict.review_gate {
                ReviewGate::UserConfirmAllowed | ReviewGate::UserTriageRequired => {
                    self.enter_human_confirm(Some(verdict.summary.clone()))
                        .await;
                }
                ReviewGate::RequiresRevision => {
                    self.enter_review_decision(round, verdict.summary.clone())
                        .await;
                }
            },
        }
    }

    /// Story/Design：review 报告进对话流，回到 AuthorConfirm（spec「review 结果回对话流」2 场景）。
    /// 无论 pass/revise 统一回 AuthorConfirm，reviewer 结论不自动定稿；
    /// WorkItem/WorkItemPlan 维持既有 HumanConfirm/ReviewDecision 路由（design.md「WorkItem 不受影响」）。
    async fn route_review_report_to_author_confirm(&mut self, verdict: &ReviewVerdict) {
        let report = format_review_feedback(verdict);
        self.record_review_message(report);
        self.complete_active_node(Some("Review 完成，报告已进入对话流".to_string()))
            .await;
        self.enter_author_confirm(Some("请基于 Review 报告继续修订或确认定稿".to_string()))
            .await;
    }

    pub(crate) async fn enter_review_decision(&mut self, round: u32, summary: String) {
        self.transition_stage(WorkspaceStage::ReviewDecision).await;
        let decision_node_id = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::ReviewDecision,
                agent: None,
                stage: WorkspaceStage::ReviewDecision,
                round: Some(round),
                title: format!("Review Decision Round {round}"),
                summary: Some(summary),
                status: TimelineNodeStatus::Paused,
            })
            .await;
        let _ = self
            .event_tx
            .send(EngineEvent::ReviewDecisionRequired {
                node_id: decision_node_id,
                round,
                options: self.review_decision_options(),
            })
            .await;
    }

    pub(crate) fn review_decision_options(&self) -> Vec<String> {
        if self.latest_work_item_plan_optional_pass_review().is_some() {
            return vec![
                "apply_optional_findings".to_string(),
                "skip_optional_findings".to_string(),
            ];
        }
        vec![
            "continue".to_string(),
            "continue_with_context".to_string(),
            "human_intervene".to_string(),
        ]
    }

    pub(crate) fn latest_work_item_plan_optional_pass_review(
        &self,
    ) -> Option<&WorkItemPlanReviewComplete> {
        self.latest_review_verdict
            .as_ref()
            .filter(|verdict| Self::work_item_plan_optional_pass_review(verdict))
            .and_then(|verdict| verdict.work_item_plan_review.as_ref())
    }

    pub(crate) fn work_item_plan_optional_pass_review(verdict: &ReviewVerdict) -> bool {
        verdict.verdict == ReviewVerdictType::Pass
            && verdict.review_gate == ReviewGate::UserConfirmAllowed
            && !verdict.findings.is_empty()
            && verdict
                .work_item_plan_review
                .as_ref()
                .is_some_and(|review| {
                    review.verdict == WorkItemPlanReviewVerdict::Pass
                        && review.review_action == WorkItemPlanReviewAction::Continue
                })
    }

    pub(crate) async fn route_work_item_plan_outline_review(&mut self, verdict: ReviewVerdict) {
        let outline_verdict = verdict
            .work_item_plan_review
            .as_ref()
            .map(|review| review.verdict.clone());
        match outline_verdict.unwrap_or(match verdict.verdict {
            ReviewVerdictType::Pass => WorkItemPlanReviewVerdict::Pass,
            ReviewVerdictType::Revise => WorkItemPlanReviewVerdict::Revise,
            ReviewVerdictType::NeedsHuman => WorkItemPlanReviewVerdict::NeedsHuman,
        }) {
            WorkItemPlanReviewVerdict::Pass => {
                if Self::work_item_plan_optional_pass_review(&verdict) {
                    let round = self.active_review_round().unwrap_or(1);
                    self.enter_review_decision(round, verdict.summary).await;
                    return;
                }
                self.enter_work_item_generation_mode(Some(
                    "Outline review 通过，请选择 Work Item 生成模式".to_string(),
                ))
                .await;
            }
            WorkItemPlanReviewVerdict::Revise | WorkItemPlanReviewVerdict::PlanReopenRequired => {
                let round = self.active_review_round().unwrap_or(1);
                self.enter_review_decision(round, verdict.summary).await;
            }
            WorkItemPlanReviewVerdict::NeedsHuman | WorkItemPlanReviewVerdict::ReviseBatch => {
                self.enter_human_confirm(Some(verdict.summary)).await;
            }
        }
    }

    #[cfg(test)]
    pub(crate) async fn route_work_item_draft_review(&mut self, verdict: ReviewVerdict) {
        self.route_work_item_draft_review_with_policy_valid(verdict, false)
            .await;
    }
    async fn route_work_item_draft_review_with_policy_valid(
        &mut self,
        verdict: ReviewVerdict,
        policy_valid: bool,
    ) {
        let draft_payload = match self.current_work_item_draft_candidate_payload() {
            Ok(payload) => payload,
            Err(message) => {
                let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                self.enter_human_confirm(Some("Work Item Draft artifact 缺失".to_string()))
                    .await;
                return;
            }
        };
        let current_outline_id = draft_payload.draft_record.outline_id.clone();
        let review = verdict.work_item_plan_review.clone();
        let item_verdict = review
            .as_ref()
            .map(|review| review.verdict.clone())
            .unwrap_or(match verdict.verdict {
                ReviewVerdictType::Pass => WorkItemPlanReviewVerdict::Pass,
                ReviewVerdictType::Revise => WorkItemPlanReviewVerdict::Revise,
                ReviewVerdictType::NeedsHuman => WorkItemPlanReviewVerdict::NeedsHuman,
            });
        let target_outline_id = review
            .as_ref()
            .and_then(|review| review.target_outline_id.clone())
            .unwrap_or_else(|| current_outline_id.clone());

        match item_verdict {
            WorkItemPlanReviewVerdict::Pass => {
                if Self::work_item_plan_optional_pass_review(&verdict) {
                    let round = self.active_review_round().unwrap_or(1);
                    self.enter_review_decision(round, verdict.summary).await;
                    return;
                }
                if let Err(message) = self
                    .continue_after_work_item_draft_review_pass_with_policy_valid(
                        &current_outline_id,
                        policy_valid,
                    )
                    .await
                {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some(
                        "继续生成下一个 Work Item Draft 失败".to_string(),
                    ))
                    .await;
                }
            }
            WorkItemPlanReviewVerdict::Revise => {
                if target_outline_id != current_outline_id {
                    self.enter_human_confirm(Some(
                        "Reviewer 要求修改非当前 Work Item，已转人工确认".to_string(),
                    ))
                    .await;
                    return;
                }
                self.pending_revision_context = Some(format_review_feedback(&verdict));
                if let Err(message) = self
                    .start_serial_work_item_draft_run_for(&current_outline_id)
                    .await
                {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some("重写当前 Work Item Draft 失败".to_string()))
                        .await;
                } else if policy_valid {
                    self.request_provider_run(ProviderRunKind::WorkItemPlanDraft {
                        feedback: None,
                    })
                    .await;
                }
            }
            WorkItemPlanReviewVerdict::PlanReopenRequired => {
                if let Err(message) = self.mark_work_item_plan_outline_revising() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some("Outline 返修状态保存失败".to_string()))
                        .await;
                    return;
                }
                self.enter_human_confirm(Some(
                    "Reviewer 要求重开 Outline，已暂停逐项生成".to_string(),
                ))
                .await;
            }
            WorkItemPlanReviewVerdict::NeedsHuman | WorkItemPlanReviewVerdict::ReviseBatch => {
                self.enter_human_confirm(Some(verdict.summary)).await;
            }
        }
    }

    #[cfg(test)]
    pub(crate) async fn route_work_item_batch_review(&mut self, verdict: ReviewVerdict) {
        self.route_work_item_batch_review_with_policy_valid(verdict, false)
            .await;
    }
    async fn route_work_item_batch_review_with_policy_valid(
        &mut self,
        verdict: ReviewVerdict,
        policy_valid: bool,
    ) {
        let review = verdict.work_item_plan_review.clone();
        let batch_verdict = review
            .as_ref()
            .map(|review| review.verdict.clone())
            .unwrap_or(match verdict.verdict {
                ReviewVerdictType::Pass => WorkItemPlanReviewVerdict::Pass,
                ReviewVerdictType::Revise => WorkItemPlanReviewVerdict::ReviseBatch,
                ReviewVerdictType::NeedsHuman => WorkItemPlanReviewVerdict::NeedsHuman,
            });

        match batch_verdict {
            WorkItemPlanReviewVerdict::Pass => {
                if Self::work_item_plan_optional_pass_review(&verdict) {
                    let round = self.active_review_round().unwrap_or(1);
                    self.enter_review_decision(round, verdict.summary).await;
                    return;
                }
                if let Err(message) = self.mark_current_work_item_batch_review_done() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some("Batch review 状态保存失败".to_string()))
                        .await;
                    return;
                }
                if policy_valid {
                    self.enter_policy_valid_work_item_plan_compile().await;
                } else {
                    self.enter_work_item_plan_compile().await;
                }
            }
            WorkItemPlanReviewVerdict::ReviseBatch => {
                self.enter_work_item_batch_confirm(Some(verdict.summary))
                    .await;
            }
            WorkItemPlanReviewVerdict::PlanReopenRequired => {
                if let Err(message) = self.mark_work_item_plan_outline_revising() {
                    let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                    self.enter_human_confirm(Some("Outline 返修状态保存失败".to_string()))
                        .await;
                    return;
                }
                self.enter_human_confirm(Some(
                    "Reviewer 要求重开 Outline，已暂停自动生成流程".to_string(),
                ))
                .await;
            }
            WorkItemPlanReviewVerdict::NeedsHuman | WorkItemPlanReviewVerdict::Revise => {
                self.enter_human_confirm(Some(verdict.summary)).await;
            }
        }
    }

    pub(crate) fn mark_current_work_item_batch_review_done(&self) -> Result<(), String> {
        let store = self.work_item_plan_store()?;
        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let batch_id = current_work_item_batch(&index)?.batch_id.clone();
        let batch = index
            .batches
            .iter_mut()
            .find(|batch| batch.batch_id == batch_id)
            .ok_or_else(|| format!("batch `{batch_id}` not found"))?;
        batch.status = WorkItemBatchStatus::ReviewDone;
        index.updated_at = chrono::Utc::now().to_rfc3339();
        store
            .save_active_index(&index)
            .map_err(|error| format!("save work item plan active index failed: {error}"))?;
        Ok(())
    }

    pub(crate) async fn continue_after_work_item_draft_review_pass(
        &mut self,
        outline_id: &str,
    ) -> Result<(), String> {
        self.continue_after_work_item_draft_review_pass_with_policy_valid(outline_id, false)
            .await
    }

    async fn continue_after_work_item_draft_review_pass_with_policy_valid(
        &mut self,
        outline_id: &str,
        policy_valid: bool,
    ) -> Result<(), String> {
        let store = self.work_item_plan_store()?;
        let mut index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let outline_order = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let current_pos = outline_order
            .iter()
            .position(|id| id == outline_id)
            .ok_or_else(|| format!("outline {outline_id} not found in order"))?;
        let now = chrono::Utc::now().to_rfc3339();

        if let Some(next_outline_id) = outline_order.get(current_pos + 1).cloned() {
            index.active_outline_id = Some(next_outline_id.clone());
            index.updated_at = now;
            store
                .save_active_index(&index)
                .map_err(|error| format!("save work item plan active index failed: {error}"))?;
            self.create_serial_work_item_draft_run_node(&next_outline_id)
                .await;
            if policy_valid {
                self.request_provider_run(ProviderRunKind::WorkItemPlanDraft { feedback: None })
                    .await;
            }
        } else {
            index.active_outline_id = None;
            index.updated_at = now;
            store
                .save_active_index(&index)
                .map_err(|error| format!("save work item plan active index failed: {error}"))?;
            if policy_valid {
                self.enter_policy_valid_work_item_plan_compile().await;
            } else {
                self.enter_work_item_plan_compile().await;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn parse_review_verdict(output: &str) -> ReviewVerdict {
        Self::parse_review_verdict_for_workspace(output, &WorkspaceType::Story)
    }

    pub(crate) fn parse_review_verdict_for_workspace(
        output: &str,
        workspace_type: &WorkspaceType,
    ) -> ReviewVerdict {
        let trimmed = output.trim();
        let parsed = extract_structured_json(trimmed).and_then(|(comments, json)| {
            if *workspace_type == WorkspaceType::WorkItemPlan {
                parse_historical_work_item_plan_review_json(
                    &json,
                    &comments,
                    &[],
                    WorkItemPlanReviewScope::Outline,
                )
            } else {
                parse_historical_review_json(&json, &comments)
            }
        });

        parsed.unwrap_or_else(|| ReviewVerdict {
            verdict: ReviewVerdictType::NeedsHuman,
            comments: output.to_string(),
            summary: "需要人工确认".to_string(),
            findings: Vec::new(),
            review_gate: ReviewGate::UserTriageRequired,
            work_item_plan_review: None,
            structured_output_diagnostic: None,
        })
    }

    async fn stop_needs_human(&mut self, summary: String) {
        self.session.session_status = WorkspaceSessionStatus::StoppedNeedsHuman;
        self.transition_stage(WorkspaceStage::Completed).await;
        if let Some(store) = &self.lifecycle_store
            && let Ok(record) = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::StoppedNeedsHuman,
            )
        {
            self.session.session_status = record.status;
        }
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::HumanConfirm,
                agent: None,
                stage: WorkspaceStage::Completed,
                round: None,
                title: "需要人工接管".to_string(),
                summary: Some(summary),
                status: TimelineNodeStatus::Completed,
            })
            .await;
    }

    pub(super) async fn finish_policy_failure(
        &mut self,
        reason: FatalReason,
        diagnostics: Vec<PolicyDiagnostic>,
    ) {
        let message = diagnostics
            .first()
            .map(|diagnostic| diagnostic.message.clone())
            .unwrap_or_else(|| reason.to_string());
        let _ = self
            .event_tx
            .send(EngineEvent::Error {
                message: message.clone(),
            })
            .await;
        if let Some(node_id) = self.active_node_id.clone() {
            self.update_timeline_node(&node_id, TimelineNodeStatus::Failed, Some(message))
                .await;
        }
        self.session.session_status = WorkspaceSessionStatus::Failed;
        self.transition_stage(WorkspaceStage::Completed).await;
        if let Some(store) = &self.lifecycle_store
            && let Ok(record) = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::Failed,
            )
        {
            self.session.session_status = record.status;
        }
    }
}

fn provider_run_kind_for_batch_outcome(
    outcome: WorkItemBatchDecisionOutcome,
) -> Option<ProviderRunKind> {
    match outcome {
        WorkItemBatchDecisionOutcome::StartBatchRun => Some(ProviderRunKind::WorkItemPlanBatch),
        WorkItemBatchDecisionOutcome::StartDraftRun => {
            Some(ProviderRunKind::WorkItemPlanDraft { feedback: None })
        }
        WorkItemBatchDecisionOutcome::StartReview => Some(ProviderRunKind::ReviewOnly),
        WorkItemBatchDecisionOutcome::HumanConfirm => None,
    }
}

pub(super) fn policy_route_record_values(
    action: &RoutingAction,
) -> (
    WorkspaceSessionStatus,
    Option<crate::product::work_item_plan_policy::HumanGateSnapshot>,
    Vec<PolicyDiagnostic>,
) {
    match action {
        RoutingAction::EnterHumanGate { snapshot } => (
            WorkspaceSessionStatus::WaitingForHuman,
            Some(snapshot.clone()),
            Vec::new(),
        ),
        RoutingAction::StopNeedsHuman { snapshot } => (
            WorkspaceSessionStatus::StoppedNeedsHuman,
            Some(snapshot.clone()),
            Vec::new(),
        ),
        RoutingAction::AbortFatal { diagnostics, .. } => {
            (WorkspaceSessionStatus::Failed, None, diagnostics.clone())
        }
        RoutingAction::ContinueToCompleted | RoutingAction::TriggerAggregateRepair { .. } => {
            (WorkspaceSessionStatus::Running, None, Vec::new())
        }
    }
}

fn legacy_work_item_plan_repair_finding(verdict: &ReviewVerdict) -> Option<ClassifiedFinding> {
    let review = verdict.work_item_plan_review.as_ref()?;
    let (kind, contract_field, required_action) = match review.verdict {
        WorkItemPlanReviewVerdict::ReviseBatch => (
            "revise_batch",
            "work_item_plan.batch",
            "rewrite the reviewed Work Item batch",
        ),
        WorkItemPlanReviewVerdict::PlanReopenRequired => (
            "plan_reopen_required",
            "work_item_plan.outline",
            "regenerate the Work Item plan outline",
        ),
        _ => return None,
    };
    let message = format!("{kind}: {}", verdict.summary);
    Some(ClassifiedFinding {
        class: FindingClass::Repairable,
        fingerprint: FindingFingerprint::for_finding(
            Some(ReviewFindingCategory::Other),
            FindingClass::Repairable,
            &message,
            Some(contract_field),
        ),
        category: Some(ReviewFindingCategory::Other),
        severity: "must_fix".to_string(),
        message,
        evidence: (!verdict.comments.trim().is_empty()).then(|| verdict.comments.clone()),
        required_action: Some(required_action.to_string()),
        contract_field: Some(contract_field.to_string()),
    })
}

fn classification_error_action(error: ClassificationError) -> RoutingAction {
    let (reason, code, message) = match error {
        ClassificationError::UnknownCategory(raw) => (
            FatalReason::UnknownCategory,
            "unknown_finding_category",
            format!("unknown finding category: {raw}"),
        ),
        ClassificationError::UnknownClassHint(raw) => (
            FatalReason::UnknownClassHint,
            "unknown_class_hint",
            format!("unknown finding class hint: {raw}"),
        ),
        ClassificationError::InvalidFinding(details) => (
            FatalReason::InvalidStructuredOutput,
            "invalid_finding_field",
            details,
        ),
        ClassificationError::VerificationScopeViolation(details) => (
            FatalReason::ProtocolViolation,
            "verification_scope_violation",
            details,
        ),
    };
    RoutingAction::AbortFatal {
        reason,
        diagnostics: vec![PolicyDiagnostic {
            code: code.to_string(),
            message,
            field: Some("findings".to_string()),
        }],
    }
}
