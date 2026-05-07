use cadence_aria::cli::{CliOutput, run_cli};
use tempfile::tempdir;

#[test]
fn cli_routes_tui_browse_with_workspace_and_task_id() {
    let workspace = tempdir().expect("workspace");
    let output = run_cli([
        "tui",
        "--workspace",
        workspace.path().to_str().expect("workspace path"),
        "--task-id",
        "task_0001",
    ])
    .expect("tui route");

    assert_eq!(
        output,
        CliOutput::Text(format!(
            "tui_browse:{}:task_0001",
            workspace.path().to_string_lossy()
        ))
    );
}

#[test]
fn cli_rejects_tui_task_id_without_value() {
    let error = run_cli(["tui", "--task-id"]).expect_err("missing value");
    assert_eq!(error.code, "invalid_cli_args");
    assert!(error.message.contains("--task-id"));
}
