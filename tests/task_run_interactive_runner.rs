use cadence_aria::interactive::controller::StepRunner;
use cadence_aria::task_run::interactive_runner::InteractiveTaskRunner;
use cadence_aria::web::types::CreateTaskRequest;
use tempfile::tempdir;

#[test]
fn interactive_task_runner_exposes_planning_execution_and_final_provider_nodes() {
    let workspace = tempdir().expect("workspace");
    let mut runner = InteractiveTaskRunner::new_fake(
        workspace.path().to_path_buf(),
        CreateTaskRequest {
            request_text: "实现 Fibonacci square sum".to_string(),
            change_id: "aria-fibonacci-square".to_string(),
            policy_preset: "manual-write".to_string(),
            provider_mode: "fake".to_string(),
            timeout_secs: 2400,
        },
    )
    .expect("runner");

    let first = runner.next_provider_step().expect("first").expect("step");
    assert_eq!(first.node_id, "N04");
    let second = runner
        .run_provider_step(first, "确认规划".to_string())
        .expect("run");
    assert!(format!("{second:?}").contains("CompletedStep"));
}
