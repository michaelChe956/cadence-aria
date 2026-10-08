use super::coding::CoderExecutionOutcome;
use super::*;
use crate::product::id::next_sequential_id_from_existing;

impl CodingWorkspaceEngine {
    pub async fn execute_coder_fix_from_review(
        &self,
        attempt: &CodingExecutionAttempt,
        review_report: &CodeReviewReport,
        context: &CodingExecutionContext,
        provider: &dyn StreamingProviderAdapter,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        Ok(self
            .execute_coder_fix_from_review_outcome(
                attempt,
                review_report,
                context,
                provider,
                command_rx,
            )
            .await?
            .attempt)
    }

    pub(crate) async fn execute_coder_fix_from_review_outcome(
        &self,
        attempt: &CodingExecutionAttempt,
        review_report: &CodeReviewReport,
        context: &CodingExecutionContext,
        provider: &dyn StreamingProviderAdapter,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CoderExecutionOutcome, CodingWorkspaceEngineError> {
        let current = self.admit_provider_run(attempt, &CodingExecutionStage::Coding, "rework")?;
        let rework_round = current.rework_count + 1;
        if current.rework_count >= current.max_auto_rework {
            let actions = vec![
                coding_gate_action_for_id("provide_context").expect("provide context action"),
                coding_gate_action_for_id("send_to_coder").expect("send to coder action"),
                coding_gate_action_for_id("abort").expect("abort action"),
            ];
            let gate = self.store.create_blocked_gate(&current, CreateBlockedGateInput {
                attempt_id: current.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: None,
                role: Some(CodingProviderRole::CodeReviewer),
                title: "Code Review 修复超上限".to_string(),
                description: format!(
                    "code review 连续要求修复 {} 次，已达上限，请人工介入。\n\n最新 findings:\n{}",
                    current.rework_count,
                    review_findings_summary(review_report)
                ),
                reason_code: Some("reviewer_rework_limit_reached".to_string()),
                evidence_refs: review_report_evidence_refs(review_report),
                raw_provider_output_ref: review_report.raw_provider_output_ref.clone(),
                available_actions: actions,
            })?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingGateRequired { gate })
                .await;
            return self
                .store
                .update_attempt_status(
                    &current.project_id,
                    &current.issue_id,
                    &current.id,
                    CodingAttemptStatus::WaitingForHuman,
                )
                .map(|attempt| CoderExecutionOutcome {
                    attempt,
                    plan_defect_decision: None,
                    plan_defect_report: None,
                })
                .map_err(CodingWorkspaceEngineError::from);
        }

        let Some(worktree_path) = current.worktree_path.clone() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                current.id.clone(),
            ));
        };

        let existing_instructions = self.store.list_rework_instructions(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?;
        let instruction = CodingReworkInstruction {
            id: next_sequential_id_from_existing(
                "coding_rework_instruction",
                existing_instructions
                    .iter()
                    .map(|instruction| instruction.id.as_str()),
            ),
            attempt_id: current.id.clone(),
            source_stage: CodingExecutionStage::CodeReview,
            rework_round,
            summary: if review_report.summary.trim().is_empty() {
                format!("code review round {} 要求修改", review_report.round)
            } else {
                review_report.summary.clone()
            },
            fix_hints: review_findings_fix_hints(review_report),
            questions: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            consumed_by_node_id: None,
            consumed_at: None,
        };
        self.store.save_rework_instruction(&current, &instruction)?;

        let running = if current.status == CodingAttemptStatus::Running {
            current
        } else {
            self.store.admit_and_transition_attempt_to_executable(
                &current.project_id,
                &current.issue_id,
                &current.id,
            )?
        };
        let coding_attempt = self.store.update_attempt_stage(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingExecutionStage::Coding,
        )?;
        let updated = self.store.increment_attempt_rework_count(
            &coding_attempt.project_id,
            &coding_attempt.issue_id,
            &coding_attempt.id,
        )?;
        let node = self.create_coding_timeline_node(&updated)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        let coder_provider_name = self
            .store
            .get_role_provider_config_snapshot(&updated.project_id, &updated.issue_id, &updated.id)?
            .coder;
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &updated,
            &CodingProviderRole::Coder,
            &coder_provider_name,
        );
        let policy = self
            .resolve_launch_policy_for_role(&updated, CodingProviderRole::Coder, &worktree_path)
            .map_err(|error| CodingWorkspaceEngineError::ProviderStream(error.to_string()))?;
        let routing_context = policy
            .as_ref()
            .map(routing_reference_context_from_policy)
            .unwrap_or_default();
        // C-1b（oracle 裁决）：恢复重驱对账 rework-claims journal——认领
        // 存在而其 node 无 role run 输出（消费标记后、spawn 前中断）时，
        // 以 claim.instruction_ids 强制入渲染并落 instruction_claim_interrupted
        // 等待事实；不重写 claim、不二次消费（强制回放 id 不进本次认领集合）。
        let interrupted_renders = self.store.reconcile_interrupted_rework_claims(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
        )?;
        let forced_replay_lines: Vec<String> = interrupted_renders
            .iter()
            .flat_map(|render| {
                let instruction_lines = render.instructions.iter().map(|instruction| {
                    format!(
                        "- [中断认领 {} 强制回放] {}（修复提示：{}）",
                        render.claim.claim_id,
                        instruction.summary,
                        instruction.fix_hints.join("；")
                    )
                });
                let note_lines = render.notes.iter().map(|note| {
                    format!(
                        "- [中断认领 {} 强制回放 ContextNote {}] {}",
                        render.claim.claim_id,
                        note.id,
                        note.content.trim()
                    )
                });
                instruction_lines.chain(note_lines).collect::<Vec<_>>()
            })
            .collect();
        let forced_replay_section = if forced_replay_lines.is_empty() {
            String::new()
        } else {
            format!(
                "\n中断认领强制回放（上一轮认领后未进入任何 prompt，本轮必须优先完成）:\n{}\n",
                forced_replay_lines.join("\n")
            )
        };
        let rendered_context =
            self.render_coder_unit_run_context(&updated, &coder_provider_name)?;
        let mut delta_prompt = build_coding_delta_prompt(
            &updated,
            context,
            Some(&instruction),
            None,
            &routing_context,
        );
        let mut full_prompt = rendered_context
            .as_ref()
            .map(|rendered| format!("{}\n\n{}", rendered.text, delta_prompt))
            .unwrap_or_else(|| {
                build_coding_prompt(
                    &updated,
                    context,
                    Some(&instruction),
                    None,
                    &routing_context,
                )
            });
        // 强制回放段进实际 prompt 与 fresh prompt 各恰一次（认领绑定实际
        // prompt 全文，含强制回放内容）。
        full_prompt.push_str(&forced_replay_section);
        // r54 越界根修:LC attempt 的 rework coder prompt 硬钉 worktree 纪律段
        //(同 coding.rs;oracle 裁决 delta/rework 变体必带禁线)。
        if updated.target_snapshot.is_some() {
            full_prompt.push_str(&worktree_discipline_section(&worktree_path));
        }
        let prompt_mode = if rendered_context.is_some() || resume_provider_session_id.is_none() {
            CodingPromptMode::FullConversation
        } else {
            CodingPromptMode::DeltaOnly
        };
        let prompt = match prompt_mode {
            CodingPromptMode::FullConversation => full_prompt.clone(),
            CodingPromptMode::DeltaOnly => {
                delta_prompt.push_str(&forced_replay_section);
                if updated.target_snapshot.is_some() {
                    delta_prompt.push_str(&worktree_discipline_section(&worktree_path));
                }
                delta_prompt
            }
        };
        // C2 Task 7（#18／BYPASS-18）：先渲染后消费——完整 prompt 渲染完成
        // 后，一次可重放原子写入完成「认领→绑定渲染结果→标记消费」，最后
        // spawn；渲染失败（含空渲染）禁消费，指令保持可读取、可重试。
        use crate::product::coding_attempt_store::ReworkClaimOutcome;
        match self.store.claim_and_consume_rework_instructions(
            &updated,
            &node.id,
            rework_round,
            &prompt,
            rendered_context
                .as_ref()
                .map(|rendered| rendered.content_hash.as_str()),
            std::slice::from_ref(&instruction.id),
        )? {
            ReworkClaimOutcome::Claimed { .. } | ReworkClaimOutcome::Replayed { .. } => {}
            ReworkClaimOutcome::RenderFailed => {
                return Err(CodingWorkspaceEngineError::ProviderStream(
                    "rework_render_failed_empty_prompt".to_string(),
                ));
            }
        }
        let role_run = self.store.create_role_run(
            &updated,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some(node.id.clone()),
        )?;
        let retry_success = self
            .run_coder_with_retry_cycle(CoderRetryCycleInput {
                attempt: &updated,
                initial_node: node,
                initial_role_run: role_run,
                provider,
                provider_name: &coder_provider_name,
                worktree_path: &worktree_path,
                initial_prompt: prompt,
                fresh_prompt: full_prompt,
                initial_prompt_mode: prompt_mode,
                initial_resume_provider_session_id: resume_provider_session_id,
                command_rx,
            })
            .await?;
        let full_output = retry_success.outcome.full_output;
        let role_run = retry_success.role_run;
        let node = retry_success.node;
        let (plan_defect_report, plan_defect_decision, plan_defect_error) =
            match parse_execution_plan_defects(PlanDefectSource::Coder, &full_output) {
                Ok(report) if report.findings.is_empty() => (None, None, None),
                Ok(report) => {
                    let projection = self.reviewer_projection_for_attempt(&updated)?;
                    let decision = execution_plan_defect_flow_decision(&report, &projection);
                    (Some(report), Some(decision), None)
                }
                Err(error) => (
                    None,
                    Some(CodeReviewFlowDecision::StopForHumanTriage),
                    Some(error.to_string()),
                ),
            };
        let plan_defect_route = plan_defect_decision.map(CodeReviewFlowDecision::label);
        let raw_provider_output_ref = self.store.save_provider_raw_output(
            &updated,
            CodingExecutionStage::Coding,
            "coder_output",
            &full_output,
        )?;
        self.store.update_role_run_refs(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
            &role_run.id,
            vec![raw_provider_output_ref.clone()],
            Vec::new(),
        )?;
        let completed_role_run = self.store.update_role_run_status(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
            &role_run.id,
            CodingRoleRunStatus::Completed,
            None,
        )?;
        self.complete_timeline_node(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
            &node.id,
            CodingTimelineNodeStatus::Completed,
            Some(format!("reviewer 修复 round {}", rework_round)),
        )
        .await?;
        self.emit_coder_output_chat_entry(CoderOutputChatEntryInput {
            attempt: &updated,
            node_id: &node.id,
            provider_name: &coder_provider_name,
            role_run: &completed_role_run,
            full_output: &full_output,
            raw_provider_output_ref: &raw_provider_output_ref,
            source: "coding",
            plan_defect_route,
        })
        .await;

        let attempt = if plan_defect_decision == Some(CodeReviewFlowDecision::StopForHumanTriage) {
            self.open_coding_output_human_triage_gate(
                &updated,
                &node.id,
                plan_defect_report.as_ref(),
                plan_defect_error.as_deref(),
                Some(raw_provider_output_ref.clone()),
            )
            .await?
        } else if plan_defect_decision
            .is_some_and(|decision| decision != CodeReviewFlowDecision::RunCoderFix)
        {
            updated
        } else {
            self.store
                .update_attempt_stage(
                    &updated.project_id,
                    &updated.issue_id,
                    &updated.id,
                    CodingExecutionStage::CodeReview,
                )
                .map_err(CodingWorkspaceEngineError::from)?
        };
        Ok(CoderExecutionOutcome {
            attempt,
            plan_defect_decision,
            plan_defect_report,
        })
    }

    pub fn send_review_limit_feedback_to_coder_for_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        self.send_review_limit_feedback_to_coder(&current, extra_context)
    }

    pub(crate) fn send_review_limit_feedback_to_coder(
        &self,
        current: &CodingExecutionAttempt,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        if current.stage != CodingExecutionStage::CodeReview
            || !matches!(
                current.status,
                CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman
            )
            || current.rework_count < current.max_auto_rework
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "send_to_coder_not_available".to_string(),
            ));
        }

        if let Some(content) = extra_context
            && !content.trim().is_empty()
        {
            self.store
                .create_context_note(current, content.trim().to_string())?;
        }

        let review_report = self
            .store
            .list_code_review_reports(&current.project_id, &current.issue_id, &current.id)?
            .into_iter()
            .last()
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "send_to_coder_missing_code_review_report".to_string(),
                )
            })?;
        let can_continue_from_review = review_report.verdict == ReviewVerdict::RequestChanges
            || (review_report.verdict == ReviewVerdict::Blocked
                && code_review_report_has_actionable_findings(&review_report));
        if !can_continue_from_review {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "send_to_coder_latest_review_not_actionable".to_string(),
            ));
        }

        let existing = self.store.list_rework_instructions(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?;
        let instruction = CodingReworkInstruction {
            id: next_sequential_id_from_existing(
                "coding_rework_instruction",
                existing.iter().map(|instruction| instruction.id.as_str()),
            ),
            attempt_id: current.id.clone(),
            source_stage: CodingExecutionStage::CodeReview,
            rework_round: current.rework_count + 1,
            summary: if review_report.summary.trim().is_empty() {
                format!("code review round {} 要求修改", review_report.round)
            } else {
                review_report.summary.clone()
            },
            fix_hints: review_findings_fix_hints(&review_report),
            questions: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            consumed_by_node_id: None,
            consumed_at: None,
        };
        self.store.save_rework_instruction(current, &instruction)?;

        let running = if current.status == CodingAttemptStatus::Running {
            current.clone()
        } else {
            self.store.admit_and_transition_attempt_to_executable(
                &current.project_id,
                &current.issue_id,
                &current.id,
            )?
        };
        let coding_attempt = self.store.update_attempt_stage(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingExecutionStage::Coding,
        )?;
        let updated = self.store.increment_attempt_rework_count(
            &coding_attempt.project_id,
            &coding_attempt.issue_id,
            &coding_attempt.id,
        )?;
        Ok(updated)
    }

    pub(crate) fn send_code_review_feedback_to_coder(
        &self,
        current: &CodingExecutionAttempt,
        extra_context: Option<String>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        if current.stage != CodingExecutionStage::CodeReview
            || !matches!(
                current.status,
                CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman
            )
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "send_to_coder_not_available".to_string(),
            ));
        }

        let operator_context = extra_context
            .map(|content| content.trim().to_string())
            .filter(|content| !content.is_empty())
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "coding_gate_extra_context_required".to_string(),
                )
            })?;
        self.store.create_context_note(current, operator_context)?;

        let review_report = self
            .store
            .list_code_review_reports(&current.project_id, &current.issue_id, &current.id)?
            .into_iter()
            .last()
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "send_to_coder_missing_code_review_report".to_string(),
                )
            })?;
        if !matches!(
            review_report.verdict,
            ReviewVerdict::Blocked | ReviewVerdict::RequestChanges
        ) {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "send_to_coder_latest_review_not_actionable".to_string(),
            ));
        }

        let existing = self.store.list_rework_instructions(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?;
        let instruction = CodingReworkInstruction {
            id: next_sequential_id_from_existing(
                "coding_rework_instruction",
                existing.iter().map(|instruction| instruction.id.as_str()),
            ),
            attempt_id: current.id.clone(),
            source_stage: CodingExecutionStage::CodeReview,
            rework_round: current.rework_count + 1,
            summary: if review_report.summary.trim().is_empty() {
                format!("code review round {} 被阻塞", review_report.round)
            } else {
                review_report.summary.clone()
            },
            fix_hints: review_findings_fix_hints(&review_report),
            questions: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            consumed_by_node_id: None,
            consumed_at: None,
        };
        self.store.save_rework_instruction(current, &instruction)?;

        let running = if current.status == CodingAttemptStatus::Running {
            current.clone()
        } else {
            self.store.admit_and_transition_attempt_to_executable(
                &current.project_id,
                &current.issue_id,
                &current.id,
            )?
        };
        let coding_attempt = self.store.update_attempt_stage(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingExecutionStage::Coding,
        )?;
        let updated = self.store.increment_attempt_rework_count(
            &coding_attempt.project_id,
            &coding_attempt.issue_id,
            &coding_attempt.id,
        )?;
        Ok(updated)
    }
}

fn review_findings_summary(review_report: &CodeReviewReport) -> String {
    if review_report.findings.is_empty() {
        return "- reviewer 未提供结构化 findings".to_string();
    }
    review_report
        .findings
        .iter()
        .map(|finding| format!("- [{:?}] {}", finding.severity, finding.message))
        .collect::<Vec<_>>()
        .join("\n")
}

fn review_findings_fix_hints(review_report: &CodeReviewReport) -> Vec<String> {
    review_report
        .findings
        .iter()
        .filter(|finding| {
            matches!(
                finding.severity,
                crate::product::coding_models::FindingSeverity::Error
                    | crate::product::coding_models::FindingSeverity::Warning
            )
        })
        .map(review_finding_fix_hint)
        .collect()
}

fn review_finding_fix_hint(finding: &ReviewFinding) -> String {
    let location = match (&finding.file_path, finding.line) {
        (Some(path), Some(line)) => format!("{path}:{line} "),
        (Some(path), None) => format!("{path} "),
        _ => String::new(),
    };
    let action = finding
        .required_action
        .as_deref()
        .map(|action| format!(" -> {action}"))
        .unwrap_or_default();
    format!("{location}{}{action}", finding.message)
}

fn review_report_evidence_refs(review_report: &CodeReviewReport) -> Vec<String> {
    let mut refs = Vec::new();
    refs.extend(review_report.tested_evidence_refs.iter().cloned());
    refs.extend(review_report.diff_refs.iter().cloned());
    refs.extend(
        review_report
            .findings
            .iter()
            .flat_map(|finding| finding.evidence.iter().cloned()),
    );
    refs.sort();
    refs.dedup();
    refs
}

// ─── C2 Task 9（#2/#9，REQ-CVT-01/02/05）：重跑原计划命令 ───

/// 重跑请求（携带稳定 command_id＋gate/check 身份＋expected 版本；
/// expected 版本绑定 attempt.rework_count，旧页面 fail-closed 请刷新）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerunPlannedCommandRequest {
    pub command_id: String,
    pub gate_id: String,
    pub check_id: String,
    pub expected_version: u64,
}

/// 重跑结果：`replayed=true` 表示命中 Task 2 命令账本重放首次 durable
/// 结果（同一指令，不触发第二次返修）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerunPlannedCommandOutcome {
    pub attempt: CodingExecutionAttempt,
    pub instruction_id: String,
    pub replayed: bool,
}

impl CodingWorkspaceEngine {
    /// 以计划合同字面命令与 cwd 作明确返修指令走既有 rework 落地面：
    /// 落 rework instruction（字面命令全文进 fix_hints）＋admission 回
    /// Running＋返修计数推进；指令由下一次 coder run 经 Task 7 认领消费
    /// 事务进入实际 prompt。幂等经 Task 2 attempt 命令账本：同 command
    /// 同 payload 重放首次结果，异 payload fail-closed；不改写计划合同、
    /// 不清 finding、不跳过 Code Review。
    pub async fn rerun_planned_command(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        request: &RerunPlannedCommandRequest,
    ) -> Result<RerunPlannedCommandOutcome, CodingWorkspaceEngineError> {
        let attempt = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let payload_digest = format!(
            "rerun|{}|{}|{}",
            request.command_id, request.gate_id, request.check_id
        );
        // ① 命令账本重放：同 command 同 payload 返回首次 durable 结果。
        if let Some(existing) = self.store.find_attempt_command_result(
            project_id,
            issue_id,
            attempt_id,
            &request.command_id,
        )? {
            if existing.payload_digest != payload_digest {
                return Err(CodingWorkspaceEngineError::Store(
                    ProductStoreError::Conflict {
                        kind: "coding_attempt_command_ledger",
                        id: request.command_id.clone(),
                    },
                ));
            }
            let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
            // 重放：按稳定派生定位首次落地的指令（账本防重保证唯一）。
            let (_, revision) = self.verification_triage_current_revision(&current)?;
            let check =
                self.verification_triage_bound_check(&current, &revision, &request.check_id)?;
            let planned_command = check.command.clone().unwrap_or_default();
            let instruction = self
                .store
                .list_rework_instructions(project_id, issue_id, attempt_id)?
                .into_iter()
                .find(|instruction| {
                    instruction.summary == "重跑原计划命令"
                        && instruction
                            .fix_hints
                            .iter()
                            .any(|hint| hint.contains(&planned_command))
                })
                .ok_or_else(|| {
                    CodingWorkspaceEngineError::ProviderStream(
                        "coding_rerun_replay_instruction_missing".to_string(),
                    )
                })?;
            return Ok(RerunPlannedCommandOutcome {
                attempt: current,
                instruction_id: instruction.id,
                replayed: true,
            });
        }
        // ② expected 版本校验：错版本 fail-closed（请刷新），不触发返修。
        if request.expected_version != attempt.rework_count as u64 {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "coding_rerun_version_conflict: 请刷新后重试".to_string(),
            ));
        }
        // ③ 身份解析：开放验证类门＋绑定 check（复用 Task 8 权威解析）。
        let open_gate = self
            .store
            .list_open_blocked_gates(project_id, issue_id, attempt_id)?
            .into_iter()
            .find(|gate| gate.gate_id == request.gate_id)
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "coding_rerun_gate_not_open: 请刷新".to_string(),
                )
            })?;
        if !super::provider_failure::is_verification_triage_eligible_gate(&open_gate) {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "coding_rerun_gate_not_eligible".to_string(),
            ));
        }
        let (_plan_revision_id, revision) = self.verification_triage_current_revision(&attempt)?;
        let check = self.verification_triage_bound_check(&attempt, &revision, &request.check_id)?;
        let planned_command = check.command.clone().ok_or_else(|| {
            CodingWorkspaceEngineError::ProviderStream(
                "coding_rerun_check_has_no_command".to_string(),
            )
        })?;
        let worktree_cwd = attempt
            .worktree_path
            .as_ref()
            .map(|path| path.display().to_string());

        // ④ 落返修指令：计划合同字面命令全文＋worktree cwd（既有 rework
        //    写面；消费由下一次 coder run 的 Task 7 事务完成）。
        let existing_instructions = self.store.list_rework_instructions(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        let mut fix_hints = vec![format!("按计划合同字面命令执行：{planned_command}")];
        if let Some(cwd) = worktree_cwd {
            fix_hints.push(format!("工作目录：{cwd}"));
        }
        let instruction = CodingReworkInstruction {
            id: next_sequential_id_from_existing(
                "coding_rework_instruction",
                existing_instructions
                    .iter()
                    .map(|instruction| instruction.id.as_str()),
            ),
            attempt_id: attempt.id.clone(),
            source_stage: CodingExecutionStage::CodeReview,
            rework_round: attempt.rework_count + 1,
            summary: "重跑原计划命令".to_string(),
            fix_hints,
            questions: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            consumed_by_node_id: None,
            consumed_at: None,
        };
        self.store.save_rework_instruction(&attempt, &instruction)?;

        let running = if attempt.status == CodingAttemptStatus::Running {
            attempt.clone()
        } else {
            self.store.admit_and_transition_attempt_to_executable(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            )?
        };
        let coding_attempt = self.store.update_attempt_stage(
            &running.project_id,
            &running.issue_id,
            &running.id,
            CodingExecutionStage::Coding,
        )?;
        let updated = self.store.increment_attempt_rework_count(
            &coding_attempt.project_id,
            &coding_attempt.issue_id,
            &coding_attempt.id,
        )?;

        // ⑤ 原门收口（send_to_coder 同义动作），避免与 Running 态并存双操作面。
        self.store.resolve_blocked_gate_with_action(
            project_id,
            issue_id,
            attempt_id,
            &request.gate_id,
            Some("send_to_coder"),
        )?;

        // ⑥ durable-first：命令账本随指令落盘。
        self.store.append_attempt_command_result(
            project_id,
            issue_id,
            attempt_id,
            &crate::product::coding_attempt_store::CodingAttemptCommandRecord {
                command_id: request.command_id.clone(),
                payload_digest,
                state: crate::product::models::automation::OperationState::Accepted,
                recorded_at: Utc::now().to_rfc3339(),
            },
        )?;
        Ok(RerunPlannedCommandOutcome {
            attempt: updated,
            instruction_id: instruction.id,
            replayed: false,
        })
    }
}
