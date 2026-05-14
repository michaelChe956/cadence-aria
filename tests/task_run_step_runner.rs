use cadence_aria::interactive::controller::{PendingProviderStep, StepRunner, StepRunnerResult};
use cadence_aria::interactive::policy::NodeWriteClass;
use cadence_aria::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};
use cadence_aria::task_run::step_runner::{
    ScriptedStepRunner, StepScriptItem, provider_step_from_adapter_input,
};
use serde_json::json;

#[test]
fn scripted_step_runner_exposes_provider_steps_in_order() {
    let mut runner = ScriptedStepRunner::new(vec![
        StepScriptItem::Provider(PendingProviderStep {
            node_id: "N16".to_string(),
            provider_type: "codex".to_string(),
            runtime_role: "executor".to_string(),
            adapter_role: "executor".to_string(),
            prompt: "编码".to_string(),
            input_summary: json!({"worktask_id":"work_wt_001"}),
            output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
            write_class: NodeWriteClass::WritesWorkspace,
            allowed_write_scope: vec!["src/".to_string(), "tests/".to_string()],
            forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
            verification_commands: vec!["cargo test --locked -j 1".to_string()],
            checkpoint_id: Some("ckpt_0001".to_string()),
        }),
        StepScriptItem::Provider(PendingProviderStep {
            node_id: "N17".to_string(),
            provider_type: "codex".to_string(),
            runtime_role: "executor".to_string(),
            adapter_role: "executor".to_string(),
            prompt: "测试".to_string(),
            input_summary: json!({}),
            output_schema: "schema://aria/artifacts/testing_report/v1".to_string(),
            write_class: NodeWriteClass::ReadOnly,
            allowed_write_scope: Vec::new(),
            forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
            verification_commands: vec!["cargo test --locked -j 1".to_string()],
            checkpoint_id: Some("ckpt_0001".to_string()),
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
                    runtime_role: "executor".to_string(),
                    adapter_role: "executor".to_string(),
                    prompt: "编码".to_string(),
                    input_summary: json!({}),
                    output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
                    write_class: NodeWriteClass::WritesWorkspace,
                    allowed_write_scope: vec!["src/".to_string(), "tests/".to_string()],
                    forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
                    verification_commands: vec!["cargo test --locked -j 1".to_string()],
                    checkpoint_id: Some("ckpt_0001".to_string()),
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
        runtime_role: "executor".to_string(),
        adapter_role: "executor".to_string(),
        prompt: "编码".to_string(),
        input_summary: json!({"worktask_id":"work_wt_001"}),
        output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
        write_class: NodeWriteClass::WritesWorkspace,
        allowed_write_scope: vec!["src/".to_string(), "tests/".to_string()],
        forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
        verification_commands: vec!["cargo test --locked -j 1".to_string()],
        checkpoint_id: Some("ckpt_0001".to_string()),
    })]);

    let error = runner
        .run_provider_step(
            PendingProviderStep {
                node_id: "N17".to_string(),
                provider_type: "codex".to_string(),
                runtime_role: "executor".to_string(),
                adapter_role: "executor".to_string(),
                prompt: "测试".to_string(),
                input_summary: json!({}),
                output_schema: "schema://aria/artifacts/testing_report/v1".to_string(),
                write_class: NodeWriteClass::ReadOnly,
                allowed_write_scope: Vec::new(),
                forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
                verification_commands: vec!["cargo test --locked -j 1".to_string()],
                checkpoint_id: Some("ckpt_0001".to_string()),
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

#[test]
fn provider_step_from_adapter_input_maps_node_write_class_and_schema() {
    let input = AdapterInput {
        provider_type: ProviderType::Codex,
        role: AdapterRole::Executor,
        prompt: "prompt body".to_string(),
        worktree_path: Some("/tmp/worktree".to_string()),
        context_files: vec!["src/lib.rs".to_string()],
        output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
        timeout: 30,
        max_retries: 1,
    };

    let step = provider_step_from_adapter_input("N16", &input).expect("provider step");
    assert_eq!(step.node_id, "N16");
    assert_eq!(step.provider_type, "codex");
    assert_eq!(
        step.output_schema,
        "schema://aria/artifacts/coding_report/v1"
    );
    assert_eq!(step.write_class, NodeWriteClass::WritesWorkspace);
}

#[test]
fn provider_step_from_adapter_input_exposes_web_confirmation_metadata() {
    let input = AdapterInput {
        provider_type: ProviderType::Codex,
        role: AdapterRole::Executor,
        prompt: "prompt body".to_string(),
        worktree_path: Some("/tmp/worktree".to_string()),
        context_files: vec!["src/lib.rs".to_string()],
        output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
        timeout: 30,
        max_retries: 1,
    };

    let step = provider_step_from_adapter_input("N16", &input).expect("provider step");
    assert_eq!(step.runtime_role, "executor");
    assert_eq!(step.adapter_role, "executor");
    assert_eq!(
        step.allowed_write_scope,
        vec!["src/".to_string(), "tests/".to_string()]
    );
    assert!(
        step.verification_commands
            .iter()
            .any(|command| command.contains("test"))
    );
}
