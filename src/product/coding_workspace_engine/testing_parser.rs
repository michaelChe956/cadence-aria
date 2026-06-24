use super::*;

pub(crate) fn record_tester_step_result(
    plan: &TestPlan,
    call: &ProviderToolCall,
    command_result: Option<TestCommand>,
    result: &ProviderToolResult,
    outputs: TesterStepResultOutputs<'_>,
) {
    let Some(step_id) = tool_call_step_id(call) else {
        outputs
            .unplanned_evidence
            .push(unplanned_evidence_from_tool(
                call,
                command_result.as_ref(),
                result,
            ));
        if let Some(command) = command_result {
            outputs.unplanned_commands.push(command);
        }
        return;
    };

    if !plan.steps.iter().any(|step| step.id == step_id) {
        outputs
            .unplanned_evidence
            .push(unplanned_evidence_from_tool(
                call,
                command_result.as_ref(),
                result,
            ));
        if let Some(command) = command_result {
            outputs.unplanned_commands.push(command);
        }
        push_unique_warning(
            outputs.context_warnings,
            format!("unknown_step_id:{step_id}"),
        );
        return;
    }

    let status = command_result
        .as_ref()
        .map(|command| command.status.clone())
        .unwrap_or_else(|| {
            if result.is_error {
                TestCommandStatus::Failed
            } else {
                TestCommandStatus::Passed
            }
        });
    let mut evidence_refs = command_result
        .as_ref()
        .map(|command| {
            [command.stdout_ref.clone(), command.stderr_ref.clone()]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if evidence_refs.is_empty() {
        evidence_refs.push(format!("tool_result:{}", result.tool_use_id));
    }
    let command = command_result
        .as_ref()
        .map(|command| command.command.clone());
    let provider_analysis = (!result.output.trim().is_empty()).then(|| result.output.clone());

    if let Some(existing) = outputs
        .step_results
        .iter_mut()
        .find(|existing| existing.step_id == step_id)
    {
        if existing.status == TestCommandStatus::Passed && status != TestCommandStatus::Passed {
            existing.status = status;
        }
        for evidence_ref in evidence_refs {
            if !existing
                .evidence_refs
                .iter()
                .any(|value| value == &evidence_ref)
            {
                existing.evidence_refs.push(evidence_ref);
            }
        }
        if existing.command.is_none() {
            existing.command = command;
        }
        if let Some(provider_analysis) = provider_analysis {
            existing.provider_analysis = Some(match existing.provider_analysis.take() {
                Some(existing_analysis) => format!("{existing_analysis}\n{provider_analysis}"),
                None => provider_analysis,
            });
        }
        return;
    }

    outputs.step_results.push(TestingStepResult {
        step_id,
        status,
        evidence_refs,
        command,
        provider_analysis,
    });
}

pub(crate) struct TesterStepResultOutputs<'a> {
    pub(crate) step_results: &'a mut Vec<TestingStepResult>,
    pub(crate) unplanned_commands: &'a mut Vec<TestCommand>,
    pub(crate) unplanned_evidence: &'a mut Vec<TestingUnplannedEvidence>,
    pub(crate) context_warnings: &'a mut Vec<String>,
}

pub(crate) fn unplanned_evidence_from_tool(
    call: &ProviderToolCall,
    command_result: Option<&TestCommand>,
    result: &ProviderToolResult,
) -> TestingUnplannedEvidence {
    let status = command_result
        .map(|command| command.status.clone())
        .unwrap_or_else(|| {
            if result.is_error {
                TestCommandStatus::Failed
            } else {
                TestCommandStatus::Passed
            }
        });
    let mut evidence_refs = command_result
        .map(|command| {
            [command.stdout_ref.clone(), command.stderr_ref.clone()]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if evidence_refs.is_empty() {
        evidence_refs.push(format!("tool_result:{}", result.tool_use_id));
    }
    TestingUnplannedEvidence {
        tool_use_id: result.tool_use_id.clone(),
        tool_name: call.tool_name.clone(),
        status,
        evidence_refs,
        provider_analysis: (!result.output.trim().is_empty()).then(|| result.output.clone()),
    }
}

pub(crate) fn tool_call_step_id(call: &ProviderToolCall) -> Option<String> {
    call.input
        .get("step_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(crate) fn high_risk_test_step_block_reason(
    plan: &TestPlan,
    call: &ProviderToolCall,
) -> Option<&'static str> {
    let step_id = tool_call_step_id(call)?;
    let step = plan.steps.iter().find(|step| step.id == step_id)?;
    (step.required && step.risk_level == TestPlanRiskLevel::High)
        .then_some("high_risk_test_step_requires_permission")
}

/// 区分 `skipped_required_steps`（步骤被阻塞/跳过）与 `missing_required_steps`
/// （步骤缺失），避免归因错误误导排查。显式 reason_code（如高风险权限）优先。
pub(crate) fn derive_testing_blocked_reason_code(
    explicit_reason_code: Option<String>,
    report: &TestingReport,
) -> String {
    if let Some(reason_code) = explicit_reason_code {
        return reason_code;
    }
    if !report.missing_required_steps.is_empty() {
        return "missing_required_steps".to_string();
    }
    if !report.skipped_required_steps.is_empty() {
        return "skipped_required_steps".to_string();
    }
    "testing_blocked".to_string()
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProviderTestingStepResultsPayload {
    #[serde(default)]
    pub(crate) step_results: Vec<TestingStepResult>,
}

pub(crate) fn parse_testing_step_results_from_provider_output(
    output: &str,
) -> Vec<TestingStepResult> {
    let Some(json) = extract_json_object(output) else {
        return Vec::new();
    };
    serde_json::from_str::<ProviderTestingStepResultsPayload>(json)
        .map(|payload| payload.step_results)
        .unwrap_or_default()
}

pub fn testing_report_has_execution_evidence(report: &TestingReport) -> bool {
    (!report.steps.is_empty() && report.plan_id.is_some())
        || !report.commands.is_empty()
        || report
            .steps
            .iter()
            .any(|step| !step.evidence_refs.is_empty() || step.command.is_some())
        || report
            .unplanned_commands
            .iter()
            .any(|command| !command.stdout_ref.is_empty() || !command.stderr_ref.is_empty())
}

pub fn testing_report_should_enter_analyst(report: &TestingReport) -> bool {
    match report.overall_status {
        TestingOverallStatus::Failed
        | TestingOverallStatus::Blocked
        | TestingOverallStatus::SkippedByUserDecision
        | TestingOverallStatus::Passed
        | TestingOverallStatus::PassedWithWarnings => true,
    }
}

pub(crate) fn testing_blocked_report_needs_gate(report: &TestingReport, reason_code: &str) -> bool {
    !testing_report_should_enter_analyst(report)
        || matches!(
            reason_code,
            "plan_tests_timeout" | "execute_test_plan_timeout"
        )
}

pub(crate) fn push_unique_warning(warnings: &mut Vec<String>, warning: String) {
    if !warnings.iter().any(|existing| existing == &warning) {
        warnings.push(warning);
    }
}

pub(crate) fn testing_blocked_gate_actions() -> Vec<CodingGateAction> {
    vec![
        CodingGateAction {
            action_id: "retry_test_plan".to_string(),
            label: "重试测试计划".to_string(),
            action_type: CodingGateActionType::RetryTestPlan,
        },
        CodingGateAction {
            action_id: "rerun_missing_steps".to_string(),
            label: "补跑缺失步骤".to_string(),
            action_type: CodingGateActionType::RerunMissingSteps,
        },
        CodingGateAction {
            action_id: "provide_context".to_string(),
            label: "补充上下文".to_string(),
            action_type: CodingGateActionType::ProvideContext,
        },
        CodingGateAction {
            action_id: "manual_continue".to_string(),
            label: "人工继续".to_string(),
            action_type: CodingGateActionType::ManualContinue,
        },
        CodingGateAction {
            action_id: "abort".to_string(),
            label: "终止".to_string(),
            action_type: CodingGateActionType::Abort,
        },
    ]
}

pub(crate) fn testing_result_review_gate_actions() -> Vec<CodingGateAction> {
    vec![
        CodingGateAction {
            action_id: "accept_testing_result".to_string(),
            label: "结果可用，进入 Analyst".to_string(),
            action_type: CodingGateActionType::AcceptTestingResult,
        },
        CodingGateAction {
            action_id: "rerun_testing".to_string(),
            label: "不满意，重新测试".to_string(),
            action_type: CodingGateActionType::RerunTesting,
        },
        CodingGateAction {
            action_id: "abort".to_string(),
            label: "终止".to_string(),
            action_type: CodingGateActionType::Abort,
        },
    ]
}

pub(crate) fn testing_result_review_description(report: &TestingReport) -> String {
    let status = match report.overall_status {
        TestingOverallStatus::Passed => "测试通过",
        TestingOverallStatus::PassedWithWarnings => "测试通过但有警告",
        TestingOverallStatus::Failed => "测试失败",
        TestingOverallStatus::SkippedByUserDecision => "测试由用户决策跳过",
        TestingOverallStatus::Blocked => "测试被阻塞",
    };
    match report.plan_summary.as_deref() {
        Some(summary) if !summary.trim().is_empty() => {
            format!(
                "Tester 已完成测试报告 {}（{}）：{}。请确认是否进入 Analyst 或重新测试。",
                report.id,
                status,
                summary.trim()
            )
        }
        _ => format!(
            "Tester 已完成测试报告 {}（{}）。请确认是否进入 Analyst 或重新测试。",
            report.id, status
        ),
    }
}

pub(crate) fn testing_report_to_analyst_evidence(report: &TestingReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| {
        format!(
            "TestingReport serialization failed; overall_status={:?}",
            report.overall_status
        )
    })
}

pub(crate) fn rework_instruction_fields_from_analyst_record(
    decision: &AnalystDecisionRecord,
) -> (String, Vec<String>) {
    if let Some(instructions) = &decision.rework_instructions {
        let mut fix_hints = instructions
            .required_changes
            .iter()
            .chain(instructions.verification_expectations.iter())
            .cloned()
            .collect::<Vec<_>>();
        if fix_hints.is_empty() {
            fix_hints.push(decision.reason.clone());
        }
        return (instructions.summary.clone(), fix_hints);
    }
    (decision.reason.clone(), vec![decision.reason.clone()])
}

pub(crate) fn analyst_human_gate_actions(
    recommendation: Option<&AnalystHumanGateRecommendation>,
) -> Vec<CodingGateAction> {
    let mut actions = Vec::new();
    if let Some(recommendation) = recommendation {
        for action_id in &recommendation.available_actions {
            if let Some(action) = coding_gate_action_for_id(action_id)
                && !actions
                    .iter()
                    .any(|existing: &CodingGateAction| existing.action_id == action.action_id)
            {
                actions.push(action);
            }
        }
    }
    if actions.is_empty() {
        actions.push(coding_gate_action_for_id("retry_analyst").expect("retry analyst action"));
        actions.push(coding_gate_action_for_id("provide_context").expect("provide context action"));
        actions.push(coding_gate_action_for_id("manual_continue").expect("manual continue action"));
        actions.push(coding_gate_action_for_id("abort").expect("abort action"));
    }
    actions
}

pub(crate) fn coding_gate_action_for_id(action_id: &str) -> Option<CodingGateAction> {
    match action_id {
        "provide_context" => Some(CodingGateAction {
            action_id: "provide_context".to_string(),
            label: "补充上下文".to_string(),
            action_type: CodingGateActionType::ProvideContext,
        }),
        "continue_rework" => Some(CodingGateAction {
            action_id: "continue_rework".to_string(),
            label: "继续返修".to_string(),
            action_type: CodingGateActionType::ContinueRework,
        }),
        "manual_continue" => Some(CodingGateAction {
            action_id: "manual_continue".to_string(),
            label: "人工继续".to_string(),
            action_type: CodingGateActionType::ManualContinue,
        }),
        "accept_risk" => Some(CodingGateAction {
            action_id: "accept_risk".to_string(),
            label: "接受风险".to_string(),
            action_type: CodingGateActionType::AcceptRisk,
        }),
        "retry_test_plan" => Some(CodingGateAction {
            action_id: "retry_test_plan".to_string(),
            label: "重试测试计划".to_string(),
            action_type: CodingGateActionType::RetryTestPlan,
        }),
        "rerun_missing_steps" => Some(CodingGateAction {
            action_id: "rerun_missing_steps".to_string(),
            label: "补跑缺失步骤".to_string(),
            action_type: CodingGateActionType::RerunMissingSteps,
        }),
        "retry_review" => Some(CodingGateAction {
            action_id: "retry_review".to_string(),
            label: "重试审查".to_string(),
            action_type: CodingGateActionType::RetryReview,
        }),
        "retry_analyst" => Some(CodingGateAction {
            action_id: "retry_analyst".to_string(),
            label: "重试 Analyst".to_string(),
            action_type: CodingGateActionType::RetryAnalyst,
        }),
        "retry_internal_review" => Some(CodingGateAction {
            action_id: "retry_internal_review".to_string(),
            label: "重试 Internal Review".to_string(),
            action_type: CodingGateActionType::RetryInternalReview,
        }),
        "send_raw_output_to_analyst" => Some(CodingGateAction {
            action_id: "send_raw_output_to_analyst".to_string(),
            label: "转交分析官".to_string(),
            action_type: CodingGateActionType::SendRawOutputToAnalyst,
        }),
        "accept_testing_result" => Some(CodingGateAction {
            action_id: "accept_testing_result".to_string(),
            label: "结果可用，进入 Analyst".to_string(),
            action_type: CodingGateActionType::AcceptTestingResult,
        }),
        "rerun_testing" => Some(CodingGateAction {
            action_id: "rerun_testing".to_string(),
            label: "不满意，重新测试".to_string(),
            action_type: CodingGateActionType::RerunTesting,
        }),
        "abort" => Some(CodingGateAction {
            action_id: "abort".to_string(),
            label: "终止".to_string(),
            action_type: CodingGateActionType::Abort,
        }),
        _ => None,
    }
}

pub(crate) fn tester_chat_entry(
    attempt: &CodingExecutionAttempt,
    node_id: &str,
    sequence: &mut usize,
    entry_type: CodingEntryType,
    content: Option<String>,
    metadata: Option<serde_json::Value>,
) -> CodingChatEntry {
    let entry = CodingChatEntry {
        id: format!("coding_chat_entry_{:04}", *sequence),
        attempt_id: attempt.id.clone(),
        node_id: Some(node_id.to_string()),
        role: CodingAgentRole::Tester,
        entry_type,
        content,
        metadata,
        created_at: Utc::now().to_rfc3339(),
    };
    *sequence += 1;
    entry
}

pub(crate) fn bind_test_plan_role_run(plan: &mut TestPlan, role_run: &CodingRoleRun) {
    plan.role_run_id = Some(role_run.id.clone());
    plan.run_no = Some(role_run.run_no);
}

pub(crate) fn bind_testing_report_role_run(report: &mut TestingReport, role_run: &CodingRoleRun) {
    report.role_run_id = Some(role_run.id.clone());
    report.run_no = Some(role_run.run_no);
}

pub(crate) fn testing_role_run_status(report: &TestingReport) -> CodingRoleRunStatus {
    match report.overall_status {
        TestingOverallStatus::Passed | TestingOverallStatus::PassedWithWarnings => {
            CodingRoleRunStatus::Completed
        }
        TestingOverallStatus::Failed => CodingRoleRunStatus::Failed,
        TestingOverallStatus::Blocked => CodingRoleRunStatus::Blocked,
        TestingOverallStatus::SkippedByUserDecision => CodingRoleRunStatus::Completed,
    }
}

pub(crate) fn derive_testing_role_run_reason(report: &TestingReport) -> Option<String> {
    report
        .context_warnings
        .iter()
        .find(|warning| warning.contains("provider_start_failed") || warning.contains("timeout"))
        .cloned()
}
