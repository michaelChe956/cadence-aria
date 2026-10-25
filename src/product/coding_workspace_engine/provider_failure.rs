use super::*;

pub(crate) struct ReviewBlockedGateInput<'a> {
    pub(crate) attempt: &'a CodingExecutionAttempt,
    pub(crate) node_id: &'a str,
    pub(crate) stage: CodingExecutionStage,
    pub(crate) role: CodingProviderRole,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) reason_code: &'static str,
    pub(crate) evidence_refs: Vec<String>,
    pub(crate) raw_provider_output_ref: Option<String>,
}

/// C2 Task 8（REQ-CVT-03/04，#19）：可转入独立验证处理的门呈现面——
/// Code Review 三门（人工分诊／验证不完整／运营阻塞）＋ coder 输出人工
/// 分诊门。验证处理是这些门的旁路 DTO／面板入口，不进动作枚举、不改
/// available_actions。
pub(crate) const VERIFICATION_TRIAGE_ELIGIBLE_REASON_CODES: [&str; 5] = [
    "code_review_output_human_triage",
    "code_review_verification_incomplete",
    "code_review_operational_blocker",
    super::gates::CODING_OUTPUT_HUMAN_TRIAGE_REASON_CODE,
    super::gates::CODING_OPERATIONAL_BLOCKER_REASON_CODE,
];

/// 三个 Code Review 分诊门（`coding-code-review-triage` 的 verdict 派生规则适用面）。
///
/// 其余 reason_code（`code_review_blocked`、`code_review_report_not_accepted`、
/// `internal_review_*`、`reviewer_rework_limit_reached` 等）动作集合保持既有语义，
/// 不受本派生规则影响（change `triage-gate-actions-by-verdict` 非目标）。
pub(crate) const CODE_REVIEW_TRIAGE_REASON_CODES: [&str; 3] = [
    "code_review_output_human_triage",
    "code_review_verification_incomplete",
    "code_review_operational_blocker",
];

pub(crate) fn is_code_review_triage_reason_code(reason_code: &str) -> bool {
    CODE_REVIEW_TRIAGE_REASON_CODES.contains(&reason_code)
}

pub(crate) fn is_verification_triage_eligible_gate(gate: &CodingGateRequired) -> bool {
    gate.reason_code
        .as_deref()
        .is_some_and(|code| VERIFICATION_TRIAGE_ELIGIBLE_REASON_CODES.contains(&code))
}

impl CodingWorkspaceEngine {
    /// C2 Task 5（REQ-CRO-05）：reviewer 配置缺失停等——进入需要 Code Reviewer／
    /// Internal Reviewer 的阶段而快照该角色为空时，在建 role run／timeline node
    /// 之前落 reason_code `reviewer_configuration_missing` 的 blocked gate
    /// （重试＋终止动作），attempt 转 Blocked，绝不以 author 顶替。
    /// 幂等：同 attempt/stage 的 open gate 命中既有记录（store 去重）。
    pub(crate) async fn block_missing_reviewer_configuration(
        &self,
        attempt: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
    ) -> CodingWorkspaceEngineError {
        let retry_action = if stage == CodingExecutionStage::InternalPrReview {
            coding_gate_action_for_id("retry_internal_review")
                .expect("retry internal review action")
        } else {
            coding_gate_action_for_id("retry_review").expect("retry review action")
        };
        let updated = match self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        ) {
            Ok(updated) => updated,
            Err(error) => {
                return CodingWorkspaceEngineError::Store(error);
            }
        };
        let role_label = match role {
            CodingProviderRole::CodeReviewer => "Code Reviewer",
            CodingProviderRole::InternalReviewer => "Internal Reviewer",
            CodingProviderRole::Coder => "Coder",
        };
        let gate = match self.store.create_blocked_gate(
            &updated,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage,
                node_id: None,
                role: Some(role.clone()),
                title: "评审配置缺失".to_string(),
                description: format!(
                    "{role_label} provider 未配置（reviewer 有效值为空）；请在等待项配置合法 reviewer 后重试。系统不会以 author 顶替启动 reviewer。"
                ),
                reason_code: Some("reviewer_configuration_missing".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: vec![
                    retry_action,
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            },
        ) {
            Ok(gate) => gate,
            Err(error) => {
                return CodingWorkspaceEngineError::Store(error);
            }
        };
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingGateRequired { gate })
            .await;
        CodingWorkspaceEngineError::ReviewerConfigurationMissing {
            attempt_id: attempt.id.clone(),
            role,
        }
    }

    /// 该 attempt 最新一条 Code Review 结论的 verdict——分诊门动作集合派生的唯一事实源。
    ///
    /// 读取口径与 `send_code_review_feedback_to_coder`（`rework.rs`）完全一致：
    /// `list_code_review_reports(...)` 的最后一条（同 attempt 结论集合的最新结论）。
    /// 任何读不到的情形（无结论 / 结论不可解析 / 读取失败）返回 `None`，调用方按
    /// fail-closed 派生（不含 `send_to_coder`），绝不以默认值兜底。
    fn latest_code_review_verdict(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Option<ReviewVerdict> {
        self.store
            .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .ok()?
            .into_iter()
            .last()
            .map(|report| report.verdict)
    }

    pub(crate) async fn create_review_blocked_gate(
        &self,
        input: ReviewBlockedGateInput<'_>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let ReviewBlockedGateInput {
            attempt,
            node_id,
            stage,
            role,
            title,
            description,
            reason_code,
            evidence_refs,
            raw_provider_output_ref,
        } = input;
        let retry_action = match stage {
            CodingExecutionStage::CodeReview => CodingGateAction {
                action_id: "retry_review".to_string(),
                label: "重试代码审查".to_string(),
                action_type: CodingGateActionType::RetryReview,
            },
            CodingExecutionStage::InternalPrReview => {
                coding_gate_action_for_id("retry_internal_review")
                    .expect("retry internal review action")
            }
            _ => coding_gate_action_for_id("retry_review").expect("retry review action"),
        };
        let latest_verdict = if is_code_review_triage_reason_code(reason_code) {
            self.latest_code_review_verdict(attempt)
        } else {
            None
        };
        let available_actions = if reason_code == "code_review_provider_interrupted" {
            vec![retry_action]
        } else if matches!(
            reason_code,
            "group_final_review_blocked"
                | "internal_review_blocked"
                | "internal_review_change_requested"
                | "internal_review_verification_incomplete"
                | "internal_review_operational_blocker"
                | "internal_review_human_triage"
        ) {
            vec![
                retry_action,
                coding_gate_action_for_id("manual_continue").expect("manual continue action"),
                coding_gate_action_for_id("abort").expect("abort action"),
            ]
        } else if is_code_review_triage_reason_code(reason_code) {
            // change `triage-gate-actions-by-verdict`：三个 Code Review 分诊门的动作
            // 集合按该 attempt 最新一条 Code Review 结论的 verdict 派生——基础恰好
            // `retry_review` + `manual_continue` + `abort`；当且仅当 verdict ∈
            // {request_changes, blocked} 时额外含 `send_to_coder`，其余 verdict（含
            // approve）不含。读不到 verdict 时 fail-closed 按三项派生：缺事实只影响
            // 这一个动作，不影响门落地与其余三个出口。
            let mut actions = Vec::with_capacity(4);
            actions.push(retry_action);
            if matches!(
                latest_verdict,
                Some(ReviewVerdict::Blocked | ReviewVerdict::RequestChanges)
            ) {
                actions.push(
                    coding_gate_action_for_id("send_to_coder").expect("send to coder action"),
                );
            }
            actions.push(
                coding_gate_action_for_id("manual_continue").expect("manual continue action"),
            );
            actions.push(coding_gate_action_for_id("abort").expect("abort action"));
            actions
        } else {
            vec![
                retry_action,
                coding_gate_action_for_id("send_to_coder").expect("send to coder action"),
                coding_gate_action_for_id("abort").expect("abort action"),
            ]
        };
        let updated = self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )?;
        let gate = self.store.create_blocked_gate(
            &updated,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage,
                node_id: Some(node_id.to_string()),
                role: Some(role),
                title,
                description,
                reason_code: Some(reason_code.to_string()),
                evidence_refs,
                raw_provider_output_ref,
                available_actions,
            },
        )?;
        let gate = if is_code_review_triage_reason_code(reason_code) {
            // 派生事实可观察：把建门时派生 `available_actions` 所用的最新 verdict 落到
            // 门记录上（additive、独立回写，`available_actions` 仍是权威动作集合）。
            // 回写失败只让呈现面缺少文案依据，不影响已落盘的门（F-7：读不到事实绝不
            // 影响开门）。显式写 `None` 也会清掉去重复用同门实例时上一轮残留的 verdict。
            self.store
                .attach_blocked_gate_latest_review_verdict(
                    &updated.project_id,
                    &updated.issue_id,
                    &updated.id,
                    &gate.gate_id,
                    latest_verdict,
                )
                .unwrap_or(gate)
        } else {
            gate
        };
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingGateRequired { gate })
            .await;
        Ok(updated)
    }

    /// P4-a（REQ-PTR-IDLE）：provider 零活动挂起耗尽门——同 provider fresh
    /// 重试一次后仍挂起时，attempt 转 Blocked 并按角色落 stalled 门：
    /// coder 落 `coder_provider_stalled`（三动作，复用块2 CoderTriageCard，
    /// `retry_coding_with_context` 提交即重新开跑）；reviewer 落
    /// `code_review_provider_stalled`（按 stage 回对应审查动作，内部审查
    /// 在 P4-b 接线）。同帧发 CodingGateRequired（durable-first，同既有门
    /// 先例）；幂等去重由 store 承担。
    pub(crate) async fn open_provider_stalled_gate(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        role: CodingProviderRole,
        phase: &str,
        idle_secs: u64,
        idle_limit_secs: u64,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let (stage, role_label, reason_code, retry_action) = match role {
            CodingProviderRole::Coder => (
                CodingExecutionStage::Coding,
                "Coder",
                CODER_PROVIDER_STALLED_REASON_CODE,
                coding_gate_action_for_id("retry_coding").expect("retry coding action"),
            ),
            CodingProviderRole::CodeReviewer => (
                CodingExecutionStage::CodeReview,
                "Code Reviewer",
                CODE_REVIEW_PROVIDER_STALLED_REASON_CODE,
                coding_gate_action_for_id("retry_review").expect("retry review action"),
            ),
            CodingProviderRole::InternalReviewer => (
                CodingExecutionStage::InternalPrReview,
                "Internal Reviewer",
                CODE_REVIEW_PROVIDER_STALLED_REASON_CODE,
                coding_gate_action_for_id("retry_internal_review")
                    .expect("retry internal review action"),
            ),
        };
        let mut available_actions = vec![retry_action];
        if matches!(role, CodingProviderRole::Coder) {
            available_actions.push(
                coding_gate_action_for_id("retry_coding_with_context")
                    .expect("retry coding with context action"),
            );
        }
        available_actions.push(coding_gate_action_for_id("abort").expect("abort action"));
        let description = if matches!(role, CodingProviderRole::Coder) {
            format!(
                "{role_label} 已 {idle_secs} 秒没有任何输出（界 {idle_limit_secs} 秒，阶段 {phase}），自动重试一次后仍无进展，流程已暂停。可直接重试、说明情况后继续，或终止。"
            )
        } else {
            format!(
                "{role_label} 已 {idle_secs} 秒没有任何输出（界 {idle_limit_secs} 秒，阶段 {phase}），自动重试一次后仍无进展，流程已暂停。可重试对应阶段审查，或终止。"
            )
        };
        let updated = self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )?;
        let gate = self.store.create_blocked_gate(
            &updated,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage,
                node_id: Some(node_id.to_string()),
                role: Some(role),
                title: "AI 挂起了".to_string(),
                description,
                reason_code: Some(reason_code.to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions,
            },
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingGateRequired { gate })
            .await;
        Ok(updated)
    }

    pub(crate) async fn fail_provider_stream<T>(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
        message: String,
    ) -> Result<T, CodingWorkspaceEngineError> {
        self.validate_attempt_issue_shared_worktree_lock_if_present(attempt)?;
        #[cfg(test)]
        crate::product::coding_workspace_engine::mutation_test_pause::pause_coding_mutation_for_test(
            self.store.paths().root(),
            crate::product::coding_workspace_engine::mutation_test_pause::CodingMutationTestPoint::ProviderFailure,
        )
        .await;
        let current =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        if !current.status.is_active() || current.stage != attempt.stage {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "provider_failure_attempt_state_changed: {}",
                attempt.id
            )));
        }
        self.validate_attempt_issue_shared_worktree_lock_if_present(&current)?;
        let attempt = &current;
        if attempt.stage == CodingExecutionStage::CodeReview {
            self.complete_timeline_node(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                node_id,
                CodingTimelineNodeStatus::Failed,
                Some(message.clone()),
            )
            .await?;
            if let Some(role_run) = self.store.latest_role_run(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingExecutionStage::CodeReview,
                CodingProviderRole::CodeReviewer,
            )? && role_run.status == CodingRoleRunStatus::Running
            {
                self.store.update_role_run_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &role_run.id,
                    CodingRoleRunStatus::Failed,
                    Some("code_review_provider_interrupted".to_string()),
                )?;
            }
            self.create_review_blocked_gate(ReviewBlockedGateInput {
                attempt,
                node_id,
                stage: CodingExecutionStage::CodeReview,
                role: CodingProviderRole::CodeReviewer,
                title: "代码审查中断".to_string(),
                description: message.clone(),
                reason_code: "code_review_provider_interrupted",
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
            })
            .await?;
            return Err(CodingWorkspaceEngineError::ProviderStream(message));
        }
        if attempt.stage == CodingExecutionStage::Coding {
            self.complete_timeline_node(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                node_id,
                CodingTimelineNodeStatus::Failed,
                Some(message.clone()),
            )
            .await?;
            if let Some(role_run) = self.store.latest_role_run(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingExecutionStage::Coding,
                CodingProviderRole::Coder,
            )? && role_run.status == CodingRoleRunStatus::Running
            {
                self.store.update_role_run_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &role_run.id,
                    CodingRoleRunStatus::Failed,
                    Some("coder_provider_interrupted".to_string()),
                )?;
            }
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )?;
            let gate = self.store.create_blocked_gate(
                attempt,
                CreateBlockedGateInput {
                    attempt_id: attempt.id.clone(),
                    stage: CodingExecutionStage::Coding,
                    node_id: Some(node_id.to_string()),
                    role: Some(CodingProviderRole::Coder),
                    title: "Coder 执行中断".to_string(),
                    description: message.clone(),
                    reason_code: Some("coder_provider_interrupted".to_string()),
                    evidence_refs: Vec::new(),
                    raw_provider_output_ref: None,
                    available_actions: vec![
                        coding_gate_action_for_id("retry_coding").expect("retry coding action"),
                        coding_gate_action_for_id("abort").expect("abort action"),
                    ],
                },
            )?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingGateRequired { gate })
                .await;
            return Err(CodingWorkspaceEngineError::ProviderStream(message));
        }
        self.store.update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Failed,
        )?;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            node_id,
            CodingTimelineNodeStatus::Failed,
            Some(message.clone()),
        )
        .await?;
        self.handle_attempt_failed(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .await?;
        Err(CodingWorkspaceEngineError::ProviderStream(message))
    }

    #[allow(dead_code)]
    pub(crate) async fn fail_provider_stream_ended<T>(
        &self,
        attempt: &CodingExecutionAttempt,
        node_id: &str,
    ) -> Result<T, CodingWorkspaceEngineError> {
        self.fail_provider_stream(
            attempt,
            node_id,
            "provider stream ended before completion".to_string(),
        )
        .await
    }
}
