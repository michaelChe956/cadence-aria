use cadence_aria::interactive::controller::{
    InteractiveController, PendingProviderStep, StepRunner, StepRunnerResult,
};
use cadence_aria::interactive::policy::{NodeWriteClass, PolicyPreset};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn controller_pauses_before_manual_write_provider_step() {
    let workspace = tempdir().expect("workspace");
    let runner = FakeRunner {
        next: Some(PendingProviderStep {
            node_id: "N16".to_string(),
            provider_type: "codex".to_string(),
            prompt: "实现功能".to_string(),
            input_summary: json!({"allowed_write_scope":["src/"]}),
            output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
            write_class: NodeWriteClass::WritesWorkspace,
        }),
    };
    let mut controller = InteractiveController::new(
        workspace.path().to_path_buf(),
        "task_0001".to_string(),
        PolicyPreset::ManualWrite,
        runner,
    );

    let result = controller.advance().expect("advance");
    assert!(matches!(result, StepRunnerResult::PausedForApproval(_)));
    let pending = controller.pending_step().expect("pending step");
    assert_eq!(pending.node_id, "N16");
    assert_eq!(pending.provider_type, "codex");
}

#[test]
fn controller_runs_readonly_step_automatically_under_manual_write() {
    let workspace = tempdir().expect("workspace");
    let runner = FakeRunner {
        next: Some(PendingProviderStep {
            node_id: "N17".to_string(),
            provider_type: "codex".to_string(),
            prompt: "运行测试".to_string(),
            input_summary: json!({}),
            output_schema: "schema://aria/artifacts/testing_report/v1".to_string(),
            write_class: NodeWriteClass::ReadOnly,
        }),
    };
    let mut controller = InteractiveController::new(
        workspace.path().to_path_buf(),
        "task_0001".to_string(),
        PolicyPreset::ManualWrite,
        runner,
    );

    let result = controller.advance().expect("advance");
    assert!(matches!(result, StepRunnerResult::CompletedStep { .. }));
    assert!(controller.pending_step().is_none());
}

#[test]
fn controller_does_not_advance_past_unconfirmed_pending_step() {
    let workspace = tempdir().expect("workspace");
    let runner = TwoStepRunner { calls: 0 };
    let mut controller = InteractiveController::new(
        workspace.path().to_path_buf(),
        "task_0001".to_string(),
        PolicyPreset::ManualWrite,
        runner,
    );

    let first = controller.advance().expect("first advance");
    let second = controller.advance().expect("second advance");

    assert_eq!(
        first,
        StepRunnerResult::PausedForApproval("N16".to_string())
    );
    assert_eq!(
        second,
        StepRunnerResult::PausedForApproval("N16".to_string())
    );
    assert_eq!(
        controller.pending_step().expect("pending").prompt,
        "实现功能"
    );
}

struct TwoStepRunner {
    calls: usize,
}

impl StepRunner for TwoStepRunner {
    fn next_provider_step(
        &mut self,
    ) -> Result<Option<PendingProviderStep>, cadence_aria::task_run::types::TaskRunError> {
        self.calls += 1;
        let step = if self.calls == 1 {
            PendingProviderStep {
                node_id: "N16".to_string(),
                provider_type: "codex".to_string(),
                prompt: "实现功能".to_string(),
                input_summary: json!({"allowed_write_scope":["src/"]}),
                output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
                write_class: NodeWriteClass::WritesWorkspace,
            }
        } else {
            PendingProviderStep {
                node_id: "N17".to_string(),
                provider_type: "codex".to_string(),
                prompt: "运行测试".to_string(),
                input_summary: json!({}),
                output_schema: "schema://aria/artifacts/testing_report/v1".to_string(),
                write_class: NodeWriteClass::ReadOnly,
            }
        };
        Ok(Some(step))
    }

    fn run_provider_step(
        &mut self,
        step: PendingProviderStep,
        prompt: String,
    ) -> Result<StepRunnerResult, cadence_aria::task_run::types::TaskRunError> {
        Ok(StepRunnerResult::CompletedStep {
            node_id: step.node_id,
            provider_run_id: "run_fake_0001".to_string(),
            prompt,
        })
    }
}

struct FakeRunner {
    next: Option<PendingProviderStep>,
}

impl StepRunner for FakeRunner {
    fn next_provider_step(
        &mut self,
    ) -> Result<Option<PendingProviderStep>, cadence_aria::task_run::types::TaskRunError> {
        Ok(self.next.clone())
    }

    fn run_provider_step(
        &mut self,
        step: PendingProviderStep,
        prompt: String,
    ) -> Result<StepRunnerResult, cadence_aria::task_run::types::TaskRunError> {
        Ok(StepRunnerResult::CompletedStep {
            node_id: step.node_id,
            provider_run_id: "run_fake_0001".to_string(),
            prompt,
        })
    }
}
