use std::fs;

use cadence_aria::product::coding_models::{TestCommandStatus, TestingOverallStatus};
use cadence_aria::product::test_executor::{
    TestCommandSpec, discover_test_commands, execute_test_command, run_all_tests,
};
use tempfile::tempdir;

#[test]
fn discovers_project_test_commands_by_priority() {
    let root = tempdir().expect("root");
    fs::write(root.path().join("package.json"), "{}").expect("package");
    fs::write(root.path().join("Cargo.toml"), "[package]\nname='demo'\n").expect("cargo");

    let specs = discover_test_commands(root.path());

    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].id, "rust");
    assert_eq!(
        specs[0].command,
        vec!["cargo", "test", "--locked", "-j", "1"]
    );
}

#[tokio::test]
async fn executes_test_command_and_records_stdout_stderr_artifacts() {
    let root = tempdir().expect("root");
    let spec = TestCommandSpec {
        id: "unit".to_string(),
        command: vec![
            "sh".to_string(),
            "-c".to_string(),
            "printf stdout; printf stderr >&2".to_string(),
        ],
    };

    let command = execute_test_command(&spec, root.path())
        .await
        .expect("execute command");

    assert_eq!(command.command, spec.command);
    assert_eq!(command.cwd, root.path());
    assert_eq!(command.exit_code, Some(0));
    assert_eq!(command.status, TestCommandStatus::Passed);
    assert_eq!(
        fs::read_to_string(root.path().join(&command.stdout_ref)).expect("stdout"),
        "stdout"
    );
    assert_eq!(
        fs::read_to_string(root.path().join(&command.stderr_ref)).expect("stderr"),
        "stderr"
    );
}

#[tokio::test]
async fn run_all_tests_marks_report_failed_when_any_command_fails() {
    let root = tempdir().expect("root");
    let specs = vec![
        TestCommandSpec {
            id: "pass".to_string(),
            command: vec!["sh".to_string(), "-c".to_string(), "exit 0".to_string()],
        },
        TestCommandSpec {
            id: "fail".to_string(),
            command: vec!["sh".to_string(), "-c".to_string(), "exit 7".to_string()],
        },
    ];

    let report = run_all_tests("coding_attempt_0001", root.path(), &specs)
        .await
        .expect("run all tests");

    assert_eq!(report.attempt_id, "coding_attempt_0001");
    assert_eq!(report.commands.len(), 2);
    assert_eq!(report.commands[0].status, TestCommandStatus::Passed);
    assert_eq!(report.commands[1].exit_code, Some(7));
    assert_eq!(report.commands[1].status, TestCommandStatus::Failed);
    assert_eq!(report.overall_status, TestingOverallStatus::Failed);
    assert!(report.backend_verified);
    assert!(report.completed_at.is_some());
}
