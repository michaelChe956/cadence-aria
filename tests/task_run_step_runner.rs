use cadence_aria::interactive::controller::{PendingProviderStep, StepRunner, StepRunnerResult};
use cadence_aria::interactive::policy::NodeWriteClass;
use cadence_aria::task_run::step_runner::{ScriptedStepRunner, StepScriptItem};
use serde_json::json;

#[test]
fn scripted_step_runner_exposes_provider_steps_in_order() {
    let mut runner = ScriptedStepRunner::new(vec![
        StepScriptItem::Provider(PendingProviderStep {
            node_id: "N16".to_string(),
            provider_type: "codex".to_string(),
            prompt: "编码".to_string(),
            input_summary: json!({"worktask_id":"work_wt_001"}),
            output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
            write_class: NodeWriteClass::WritesWorkspace,
        }),
        StepScriptItem::Provider(PendingProviderStep {
            node_id: "N17".to_string(),
            provider_type: "codex".to_string(),
            prompt: "测试".to_string(),
            input_summary: json!({}),
            output_schema: "schema://aria/artifacts/testing_report/v1".to_string(),
            write_class: NodeWriteClass::ReadOnly,
        }),
    ]);

    assert_eq!(
        runner
            .next_provider_step()
            .expect("first")
            .expect("step")
            .node_id,
        "N16"
    );
    assert!(matches!(
        runner
            .run_provider_step(
                PendingProviderStep {
                    node_id: "N16".to_string(),
                    provider_type: "codex".to_string(),
                    prompt: "编码".to_string(),
                    input_summary: json!({}),
                    output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
                    write_class: NodeWriteClass::WritesWorkspace,
                },
                "确认后的编码 prompt".to_string()
            )
            .expect("run"),
        StepRunnerResult::CompletedStep { .. }
    ));
    assert_eq!(
        runner
            .next_provider_step()
            .expect("second")
            .expect("step")
            .node_id,
        "N17"
    );
}

#[test]
fn scripted_step_runner_rejects_mismatched_step_without_consuming_queue() {
    let mut runner = ScriptedStepRunner::new(vec![StepScriptItem::Provider(PendingProviderStep {
        node_id: "N16".to_string(),
        provider_type: "codex".to_string(),
        prompt: "编码".to_string(),
        input_summary: json!({"worktask_id":"work_wt_001"}),
        output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
        write_class: NodeWriteClass::WritesWorkspace,
    })]);

    let error = runner
        .run_provider_step(
            PendingProviderStep {
                node_id: "N17".to_string(),
                provider_type: "codex".to_string(),
                prompt: "测试".to_string(),
                input_summary: json!({}),
                output_schema: "schema://aria/artifacts/testing_report/v1".to_string(),
                write_class: NodeWriteClass::ReadOnly,
            },
            "wrong prompt".to_string(),
        )
        .expect_err("mismatched step");

    assert_eq!(error.code, "scripted_step_mismatch");
    assert_eq!(
        runner
            .next_provider_step()
            .expect("next")
            .expect("step")
            .node_id,
        "N16"
    );
}
