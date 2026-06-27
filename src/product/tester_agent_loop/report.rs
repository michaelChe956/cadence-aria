use chrono::Utc;
use serde_json::{Value, json};

use crate::product::coding_models::{
    TestCommand, TestCommandStatus, TestPlan, TestingOverallStatus, TestingReport,
    TestingStepResult,
};

pub fn build_testing_report(
    attempt_id: &str,
    commands: Vec<TestCommand>,
    provider_output: &str,
    blocked_summary: Option<String>,
) -> TestingReport {
    let provider_claim = parse_provider_claim(provider_output, blocked_summary.as_deref());
    let overall_status = if blocked_summary.is_some() || commands.is_empty() {
        TestingOverallStatus::Blocked
    } else if commands
        .iter()
        .all(|command| command.status == TestCommandStatus::Passed)
    {
        TestingOverallStatus::Passed
    } else {
        TestingOverallStatus::Failed
    };
    TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        role_run_id: None,
        run_no: None,
        commands,
        overall_status,
        provider_claim,
        backend_verified: true,
        started_at: Utc::now().to_rfc3339(),
        completed_at: Some(Utc::now().to_rfc3339()),
        plan_id: None,
        plan_summary: None,
        steps: Vec::new(),
        unplanned_commands: Vec::new(),
        unplanned_evidence: Vec::new(),
        missing_required_steps: Vec::new(),
        skipped_required_steps: Vec::new(),
        context_warnings: Vec::new(),
        raw_provider_output_ref: None,
    }
}

pub fn build_plan_based_testing_report(
    report_id: &str,
    attempt_id: &str,
    plan: &TestPlan,
    steps: Vec<TestingStepResult>,
    unplanned_commands: Vec<TestCommand>,
    provider_claim: Option<Value>,
    raw_provider_output_ref: Option<String>,
) -> TestingReport {
    let mut missing_required_steps = Vec::new();
    let mut skipped_required_steps = Vec::new();
    let mut required_failed = false;
    let mut optional_failed = false;

    for plan_step in &plan.steps {
        let result = steps.iter().find(|result| result.step_id == plan_step.id);
        match (plan_step.required, result.map(|result| &result.status)) {
            (true, None) => missing_required_steps.push(plan_step.id.clone()),
            (true, Some(TestCommandStatus::Blocked)) => {
                skipped_required_steps.push(plan_step.id.clone());
            }
            (true, Some(TestCommandStatus::Failed | TestCommandStatus::TimedOut)) => {
                required_failed = true;
            }
            (
                false,
                Some(
                    TestCommandStatus::Failed
                    | TestCommandStatus::TimedOut
                    | TestCommandStatus::Blocked,
                ),
            ) => {
                optional_failed = true;
            }
            _ => {}
        }
    }

    let overall_status = if !missing_required_steps.is_empty() || !skipped_required_steps.is_empty()
    {
        TestingOverallStatus::Blocked
    } else if required_failed {
        TestingOverallStatus::Failed
    } else if !plan.context_warnings.is_empty() || optional_failed {
        TestingOverallStatus::PassedWithWarnings
    } else {
        TestingOverallStatus::Passed
    };

    TestingReport {
        id: report_id.to_string(),
        attempt_id: attempt_id.to_string(),
        role_run_id: None,
        run_no: None,
        commands: unplanned_commands.clone(),
        overall_status,
        provider_claim,
        backend_verified: true,
        started_at: Utc::now().to_rfc3339(),
        completed_at: Some(Utc::now().to_rfc3339()),
        plan_id: Some(plan.id.clone()),
        plan_summary: Some(plan.summary.clone()),
        steps,
        unplanned_commands,
        unplanned_evidence: Vec::new(),
        missing_required_steps,
        skipped_required_steps,
        context_warnings: plan.context_warnings.clone(),
        raw_provider_output_ref,
    }
}

pub fn format_test_plan_chat_summary(plan: &TestPlan) -> String {
    let mut output = format!("## Tester 测试计划\n\n{}\n\n", plan.summary.trim());
    if !plan.assumptions.is_empty() {
        output.push_str("### 假设\n");
        for assumption in &plan.assumptions {
            output.push_str("- ");
            output.push_str(assumption);
            output.push('\n');
        }
        output.push('\n');
    }
    output.push_str("### 步骤\n");
    for step in &plan.steps {
        output.push_str("- ");
        output.push_str(&step.id);
        output.push_str(" · ");
        output.push_str(&step.title);
        output.push_str(" · ");
        output.push_str(if step.required {
            "required"
        } else {
            "optional"
        });
        output.push_str(" · ");
        output.push_str(&format!("{:?}", step.risk_level).to_ascii_lowercase());
        output.push('\n');
        output.push_str("  - 证据预期：");
        output.push_str(&step.evidence_expectation);
        output.push('\n');
    }
    output
}

pub fn format_testing_report_chat_summary(report: &TestingReport) -> String {
    let mut output = format!(
        "## Tester 测试结果\n\n状态：`{:?}`\n",
        report.overall_status
    );
    if let Some(summary) = report.plan_summary.as_deref() {
        output.push_str("\n计划：");
        output.push_str(summary);
        output.push('\n');
    }
    if !report.missing_required_steps.is_empty() {
        output.push_str("\n### 缺失 required steps\n");
        for step in &report.missing_required_steps {
            output.push_str("- ");
            output.push_str(step);
            output.push('\n');
        }
    }
    if !report.skipped_required_steps.is_empty() {
        output.push_str("\n### 跳过 required steps\n");
        for step in &report.skipped_required_steps {
            output.push_str("- ");
            output.push_str(step);
            output.push('\n');
        }
    }
    if !report.steps.is_empty() {
        output.push_str("\n### 执行证据\n");
        for step in &report.steps {
            output.push_str("- ");
            output.push_str(&step.step_id);
            output.push_str(" · ");
            output.push_str(&format!("{:?}", step.status).to_ascii_lowercase());
            if !step.evidence_refs.is_empty() {
                output.push_str(" · ");
                output.push_str(&step.evidence_refs.join(", "));
            }
            output.push('\n');
        }
    }
    if let Some(raw_ref) = report.raw_provider_output_ref.as_deref() {
        output.push_str("\nraw：`");
        output.push_str(raw_ref);
        output.push_str("`\n");
    }
    output
}

fn parse_provider_claim(provider_output: &str, blocked_summary: Option<&str>) -> Option<Value> {
    if let Some(summary) = blocked_summary {
        return Some(json!({
            "summary": summary,
            "bugs_found": [],
            "warning": true
        }));
    }
    let trimmed = provider_output.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<Value>(trimmed)
        .ok()
        .or_else(|| Some(json!({"summary": trimmed, "bugs_found": []})))
}
