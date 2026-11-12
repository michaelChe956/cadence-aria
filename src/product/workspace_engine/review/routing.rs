#[path = "routing_scope.rs"]
mod routing_scope;
use super::feedback::{format_actionable_review_feedback, format_review_feedback};
use super::policy_routing::{RoutingAction, carry_forward_open_gate_budget};
use super::*;
use crate::product::lifecycle_store::PolicyRoutePersist;
use crate::product::work_item_plan_policy::{
    ClassifiedFinding, EvaluationDecision, FatalReason, FindingClass, HumanReason, PlanOutcome,
    PolicyDiagnostic, ReviewCycleState, ReviewEvaluationInput, ReviewInvocationScope, ReviewPhase,
    RunBudgets, RunHistory, RunHistoryDelta, RunPolicy, WorkItemPlanFlowKind, classify_review,
    evaluate,
};
#[path = "routing_platform_rescue.rs"]
mod routing_platform_rescue;
use routing_platform_rescue::{
    classification_error_action, legacy_work_item_plan_repair_finding,
    merge_platform_compile_failure_origin, rescue_platform_compile_failure_repair,
};
use routing_scope::{route_outcome_from_decision, single_candidate_review_cycle};
impl WorkspaceEngine {
    pub(crate) async fn complete_review(
        &mut self,
        completion: ProviderCompletion,
        verdict: ReviewVerdict,
    ) {
        // 终态会话丢弃迟到评审 verdict；但重开中的 amendment 门（REQ-GCE-03
        // 场景二：durable 重开签名 + probe 放行重开的同一完整谓词命中，见
        // durable_reopened_amendment_record）不是已完结会话，其重启评审的 Pass
        // 必须继续走 policy route 重建 Approval 门（I-1）；不得被本守卫当作已
        // 完结会话丢弃。伪造的 Completed+WaitingForHuman 三元组（无 amendment
        // context）仍被丢弃。I-1 round3（F-A）：判别三态——明确无 amendment
        // 事实 → 丢弃（普通门语义）；判别命中 → 放行；无法读取/校验判别所需
        // durable 事实 → 持久化失败 fail-closed 终止（不得静默丢弃，更不得放行
        // 进重置公式）。
        if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate
            && matches!(
                self.session.single_candidate_phase,
                Some(
                    crate::product::models::SingleCandidatePhase::Completed
                        | crate::product::models::SingleCandidatePhase::Failed
                )
            )
        {
            match self.is_reopened_amendment_gate() {
                Ok(true) => {}
                Ok(false) => return,
                Err(error) => {
                    self.finish_policy_failure(
                        FatalReason::PersistenceFailure,
                        vec![PolicyDiagnostic {
                            code: FatalReason::PersistenceFailure.as_code().to_owned(),
                            message: error.to_string(),
                            field: Some("plan_amendment_context".to_owned()),
                        }],
                    )
                    .await;
                    return;
                }
            }
        }
        // SG6b（plan v1.1 Task SG6b / REQ-CG-03:74、REQ-SG-03/04A/05）：
        // SC 结构授权窗口（durable current_grant）的 verdict 面分域——
        // - phase=Reviewing：本 verdict 即该 grant 复评的收口 verdict，
        //   路由后执行 turn Completed + grant Resolved finalize（携
        //   review_node_id/scope digest）；
        // - 终态相位（Resolved/SystemFault/BusinessGate）：tombstone——
        //   迟到 review 全量拒收（两 attempt 满后 review Revise 第三
        //   start 0、single_candidate_author key/reservation/repairs_used
        //   增量 0；同 turn 已收口的重复 verdict 同族）；
        // - 人工窗口活动相位（AuthorRunning/Assessing/InstructionFrozen/
        //   Attempt2Claimed/BlockedByLcRecovery）：未知外部调用等待——
        //   人工窗口拥有流转，verdict 不路由（旧 policy 自动返修不得
        //   旁路结构 attempt 上限）。
        // durable 判据不可读＝持久化失败 fail-closed（I-1 round3 F-A 同
        // 构），不得静默按无窗口处理。
        let mut sc_reviewing_closure: Option<String> = None;
        if self.is_single_candidate_plan() {
            let Some(store) = self.lifecycle_store.as_ref() else {
                return;
            };
            let record = match store.get_workspace_session(&self.session.session_id) {
                Ok(record) => record,
                Err(error) => {
                    self.finish_policy_failure(
                        FatalReason::PersistenceFailure,
                        vec![PolicyDiagnostic {
                            code: FatalReason::PersistenceFailure.as_code().to_owned(),
                            message: error.to_string(),
                            field: Some("structural_governance_state".to_owned()),
                        }],
                    )
                    .await;
                    return;
                }
            };
            if let Some(grant) = record
                .structural_governance_state
                .as_ref()
                .and_then(|state| state.current_grant.as_ref())
            {
                match grant.phase {
                    Some(crate::product::models::workspace::StructuralGrantPhase::Reviewing) => {
                        sc_reviewing_closure = grant.turn_id.clone();
                    }
                    Some(
                        crate::product::models::workspace::StructuralGrantPhase::Resolved
                        | crate::product::models::workspace::StructuralGrantPhase::SystemFault
                        | crate::product::models::workspace::StructuralGrantPhase::BusinessGate,
                    ) => {
                        // 终态 grant tombstone：迟到 review 零 durable 写。
                        return;
                    }
                    Some(_) => {
                        // 人工窗口活动相位：等待（不路由、零写）。
                        return;
                    }
                    None => {}
                }
            }
        }
        let node_id = self
            .active_node_id
            .clone()
            .unwrap_or_else(|| "review_unknown".to_string());
        let round = self.active_review_round().unwrap_or(1);
        let active_node_type = self.active_node_type();
        let verdict = self
            .finalize_review_verdict(&node_id, completion.readable_output, verdict)
            .await;
        let reviewer = self
            .active_node_agent()
            .or_else(|| self.session.reviewer_provider.clone());
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
        } else {
            self.route_legacy_review(verdict, active_node_type, round, false)
                .await;
        }
        // SG6b（REQ-CG-03:74）：Reviewing grant 的 verdict 收口——路由
        // 完成后 turn Completed + grant 携 review 证据终态 Resolved（幂
        // 等；失败可观测上抛，grant 留待恢复对账，不假称成功）。
        if let Some(turn_id) = sc_reviewing_closure
            && let Err(error) = self
                .close_reviewing_structural_grant(&turn_id, &node_id)
                .await
        {
            let _ = self
                .event_tx
                .send(EngineEvent::Error {
                    message: format!(
                        "structural verdict closure for turn {turn_id} failed: {error}"
                    ),
                })
                .await;
        }
    }

    /// SG6b（REQ-CG-03:74 / SG6a.3⑧1 移交）：有 reviewer 分支的 verdict
    /// 面收口——turn 终态化 Completed（幂等）+ `finalize_structural_
    /// revision_grant` 携 review_node_id/scope digest 终态 Resolved
    ///（store 单源幂等；turn/grant/会话快照同 CAS 链重读）。
    async fn close_reviewing_structural_grant(
        &mut self,
        turn_id: &str,
        review_node_id: &str,
    ) -> Result<(), String> {
        let store = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let mut expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        let mut turn = store
            .get_human_gate_turn(&self.session.session_id, turn_id)
            .map_err(|error| error.to_string())?;
        if turn.status != crate::product::models::HumanGateTurnStatus::Completed {
            turn.status = crate::product::models::HumanGateTurnStatus::Completed;
            turn.failure_class = None;
            turn.updated_at = chrono::Utc::now().to_rfc3339();
            expected = store
                .update_human_gate_turn(&expected, turn)
                .map_err(|error| error.to_string())?;
            turn = store
                .get_human_gate_turn(&self.session.session_id, turn_id)
                .map_err(|error| error.to_string())?;
        }
        let state = expected
            .structural_governance_state
            .as_ref()
            .ok_or_else(|| "structural governance state missing".to_string())?;
        let grant = state
            .current_grant
            .as_ref()
            .ok_or_else(|| "no active structural revision grant".to_string())?;
        if grant.turn_id.as_deref() != Some(turn_id) {
            return Err(format!(
                "structural grant is bound to turn {:?}, not {turn_id}",
                grant.turn_id
            ));
        }
        let scope_digest =
            crate::product::workspace_engine::conversational_gate::sc_revision_review_scope_digest(
                expected
                    .work_item_plan_source_revision_ref
                    .as_deref()
                    .unwrap_or_default(),
                expected
                    .plan_candidate_ir_ref
                    .as_deref()
                    .unwrap_or_default(),
                expected
                    .mechanical_report_ref
                    .as_deref()
                    .unwrap_or_default(),
                state.current_context_digest.as_deref().unwrap_or_default(),
            );
        let saved = store
            .finalize_structural_revision_grant(
                &expected,
                turn,
                &grant.grant_id,
                Some(review_node_id),
                Some(&scope_digest),
            )
            .map_err(|error| error.to_string())?;
        self.reload_session_from_record(saved);
        Ok(())
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
            let (invocation, mut action, history) =
                self.evaluate_work_item_policy_route(node_id, verdict)?;
            // C2（REQ-HGC-01/REQ-CG-02 预算重置边界修订，F-52）：同一 logical
            // gate 内的 HumanRequired 重建 MUST 接续 durable 快照剩余预算与
            // gate-local 轮次——纯路由公式 `max − manual_repairs_used` 只对
            // 新 logical gate（durable 快照缺席）成立；门 episode 在场（快照
            // 存留，含自动返修期间的保留快照）时在此覆盖，避免每轮回填默认 3。
            carry_forward_open_gate_budget(
                &mut action,
                expected
                    .as_ref()
                    .and_then(|record| record.human_gate_snapshot.as_ref()),
            );

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
                match single_candidate_review_cycle(
                    self.session.review_invocation_scope.as_ref(),
                    self.session.plan_candidate_ir_ref.as_deref(),
                    &self.session.run_history,
                ) {
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
            // A3/REQ-WSC-25：平台编译失败轮的指纹并入本轮 origin（详见
            // merge_platform_compile_failure_origin 的红线注释）。
            Ok(scope) => merge_platform_compile_failure_origin(scope, phase, verdict),
            Err(action) => {
                let scope = self
                    .session
                    .review_invocation_scope
                    .clone()
                    .unwrap_or_else(|| ReviewInvocationScope::initial(cycle_key.clone()));
                return Some((scope, *action, self.session.run_history.clone()));
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
            Err(action) => return Some((invocation, *action, self.session.run_history.clone())),
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
            Err(action) => return Some((invocation, *action, self.session.run_history.clone())),
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
            // A3/REQ-WSC-25：纯平台编译失败轮的返修资格复原（详见
            // rescue_platform_compile_failure_repair 的注释与红线）。
            rescue_platform_compile_failure_repair(
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
                ),
                phase,
                &cycle,
                verdict,
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
        // C2（REQ-HGC-03 场景 2/F-52 R6）：解析降级轮（已尝试结构化输出
        // 恢复——同 invocation 静默重试或 JSON repair——但失败）不计为一
        // 次完整 review：review/cycle 计数零增量。现场 R6 的 invalid_json
        // 白耗轮即反例锚点；降级轮 verdict 无 findings，seen 集合并入也为空。
        if verdict
            .structured_output_diagnostic
            .as_ref()
            .is_some_and(|diagnostic| diagnostic.repair_attempted && !diagnostic.repair_succeeded)
        {
            decision.history_delta = RunHistoryDelta::default();
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
        // SG6b（Interfaces「同 grant route 先于旧 policy/repair
        // reservation」）：SC 会话存在结构授权窗口（durable
        // current_grant——人工反馈轮的结构 attempt 通道）时，旧 policy
        // 自动返修不得旁路结构 attempt 上限——TriggerAggregateRepair
        // 在 repairs_used/cycle 计数合并之前改道原人工门（Interactive
        // → EnterHumanGate / AutoIfValid → StopNeedsHuman；L2 退役同
        // 语义）：零 author start、零 single_candidate_author 预领键、
        // 零 repair_reservation、零 repairs_used/cycle 增量。
        let action = if matches!(action, RoutingAction::TriggerAggregateRepair { .. })
            && self.is_single_candidate_plan()
            && self.durable_structural_grant_present()
        {
            let findings = match action {
                RoutingAction::TriggerAggregateRepair { findings } => findings,
                _ => unreachable!("guarded by matches! above"),
            };
            let snapshot = HumanGateSnapshot {
                candidate_recovery: None,
                findings,
                repeated_fingerprints: Vec::new(),
                attempts_used: history
                    .repairs_used
                    .saturating_add(history.manual_repairs_used),
                manual_repairs_remaining: RunBudgets::default()
                    .max_manual_repairs
                    .saturating_sub(history.manual_repairs_used),
                trigger: HumanReason::RepairBudgetExhausted,
                resumable: self.session.run_policy == RunPolicy::Interactive,
                accepted_feedback_turns: Some(0),
            };
            if self.session.run_policy == RunPolicy::Interactive {
                RoutingAction::EnterHumanGate { snapshot }
            } else {
                RoutingAction::StopNeedsHuman { snapshot }
            }
        } else {
            action
        };
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
                if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate
                    && self.durable_structural_grant_present() =>
            {
                // SG6b「scope/dispatch 二次 guard」：评估层转换漏网（如
                // 内存回退分支）时，dispatch 面仍不得以旧 policy 自动返修
                // 旁路结构 attempt 上限——改道原人工门，零 author start。
                self.enter_human_confirm(Some(verdict.summary)).await;
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
                    author_rewrite_budget: record.author_rewrite_budget.clone(),
                },
            )
            .map_err(|error| error.to_string())?;
        self.refresh_policy_state(&saved);
        Ok(())
    }
    fn is_single_candidate_plan(&self) -> bool {
        self.session.workspace_type == WorkspaceType::WorkItemPlan
            && self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate
    }

    /// SG6b：durable 结构授权窗口在场探测（dispatch 二次 guard 用；读取
    /// 失败按无窗口处理——评估层的一级 guard 已以 expected 记录权威拦截，
    /// 此处只作纵深防御）。
    fn durable_structural_grant_present(&self) -> bool {
        self.lifecycle_store
            .as_ref()
            .and_then(|store| store.get_workspace_session(&self.session.session_id).ok())
            .and_then(|record| {
                record
                    .structural_governance_state
                    .and_then(|state| state.current_grant)
            })
            .is_some()
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
            _ => {
                // F5-B 防打转闸门：run1d（policy 旁路臂）连续 2 轮同指纹非 advisory
                // findings 即强制 human_confirm，不再进入下一轮 revise。
                if self.session.workspace_type == WorkspaceType::WorkItemPlan
                    && self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate
                    && matches!(verdict.review_gate, ReviewGate::RequiresRevision)
                    && let Some(count) =
                        self.single_candidate_consecutive_repeated_finding_count(&verdict)
                {
                    self.enter_human_confirm(Some(format!(
                        "repeated_findings：reviewer 连续 2 轮给出实质相同的非 advisory findings（{count} 项），自动修订无法收敛，等待人工确认"
                    )))
                    .await;
                    return;
                }
                match &verdict.review_gate {
                    ReviewGate::UserConfirmAllowed | ReviewGate::UserTriageRequired => {
                        self.enter_human_confirm(Some(verdict.summary.clone()))
                            .await;
                    }
                    // L2 退役（T5/REQ-RET-02）：SC 改道 human gate（review_decision
                    // 消息族已删）；非 SC 保留原阶段=在途限制（REQ-RET-03）。
                    ReviewGate::RequiresRevision if self.is_single_candidate_plan() => {
                        self.enter_human_confirm(Some(verdict.summary.clone()))
                            .await;
                    }
                    ReviewGate::RequiresRevision => {
                        self.enter_review_decision(round, verdict.summary.clone())
                            .await;
                    }
                }
            }
        }
    }

    /// Story/Design：review 报告进对话流回 AuthorConfirm（pass/revise 统一，reviewer
    /// 结论不自动定稿）；WorkItem/WorkItemPlan 维持既有路由（design.md「WorkItem 不受影响」）。
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
                self.pending_revision_context = Some(format_actionable_review_feedback(&verdict));
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
