use std::fs;
use std::path::Path;
use std::process::Command;

use cadence_aria::product::coding_models::{TestCommandStatus, TestingOverallStatus};
use cadence_aria::product::test_executor::{
    TestCommandSpec, discover_test_commands, execute_test_command, infer_test_commands,
    planned_test_commands_from_markdown, run_all_tests,
};
use tempfile::tempdir;

#[test]
fn discovers_mixed_rust_and_web_frontend_commands_without_forced_serial_cargo() {
    let root = tempdir().expect("root");
    fs::write(root.path().join("Cargo.toml"), "[package]\nname='demo'\n").expect("cargo");
    fs::create_dir_all(root.path().join("web")).expect("web dir");
    fs::write(
        root.path().join("web/package.json"),
        r#"{"scripts":{"test":"vitest --run"}}"#,
    )
    .expect("web package");

    let specs = discover_test_commands(root.path());

    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].id, "rust");
    assert_eq!(specs[0].command, vec!["cargo", "test", "--locked"]);
    assert_eq!(specs[1].id, "node_web");
    assert_eq!(specs[1].command, vec!["pnpm", "-C", "web", "test"]);
}

#[test]
fn infers_tester_commands_from_project_files() {
    let root = tempdir().expect("root");
    fs::write(root.path().join("Cargo.toml"), "[package]\nname='demo'\n").expect("cargo");

    let specs = infer_test_commands(root.path());

    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].id, "inferred_rust");
    assert_eq!(specs[0].command, vec!["cargo", "test"]);
}

#[test]
fn infers_mixed_rust_and_web_frontend_commands() {
    let root = tempdir().expect("root");
    fs::write(root.path().join("Cargo.toml"), "[package]\nname='demo'\n").expect("cargo");
    fs::create_dir_all(root.path().join("web")).expect("web dir");
    fs::write(
        root.path().join("web/package.json"),
        r#"{"scripts":{"test":"vitest --run"}}"#,
    )
    .expect("web package");

    let specs = infer_test_commands(root.path());

    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].id, "inferred_rust");
    assert_eq!(specs[0].command, vec!["cargo", "test"]);
    assert_eq!(specs[1].id, "inferred_node_web");
    assert_eq!(specs[1].command, vec!["pnpm", "-C", "web", "test"]);
}

#[test]
fn infers_node_command_only_when_package_test_script_exists() {
    let root = tempdir().expect("root");
    fs::write(
        root.path().join("package.json"),
        r#"{"scripts":{"test":"vitest"}}"#,
    )
    .expect("package");

    let specs = infer_test_commands(root.path());

    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].id, "inferred_node");
    assert_eq!(specs[0].command, vec!["pnpm", "test"]);

    let no_test_script = tempdir().expect("no test script");
    fs::write(
        no_test_script.path().join("package.json"),
        r#"{"scripts":{"build":"vite build"}}"#,
    )
    .expect("package");
    assert!(infer_test_commands(no_test_script.path()).is_empty());
}

#[test]
fn does_not_infer_child_package_without_test_script() {
    let root = tempdir().expect("root");
    fs::create_dir_all(root.path().join("web")).expect("web dir");
    fs::write(
        root.path().join("web/package.json"),
        r#"{"scripts":{"build":"vite build"}}"#,
    )
    .expect("web package");

    assert!(infer_test_commands(root.path()).is_empty());
}

#[test]
fn infers_pytest_command_from_pytest_config() {
    let root = tempdir().expect("root");
    fs::write(root.path().join("pytest.ini"), "[pytest]\n").expect("pytest");

    let specs = infer_test_commands(root.path());

    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].id, "inferred_python");
    assert_eq!(specs[0].command, vec!["pytest"]);
}

#[test]
fn extracts_planned_verification_commands_from_work_item_markdown() {
    let markdown = r#"
# 爬楼梯问题 Work Item

## 任务拆分

验证命令：
- `uv run python -m unittest -v tests.test_climbing_stairs`

预期结果：
- 测试通过

## 验证命令

主验证命令：
- `uv run python -m unittest -v tests.test_climbing_stairs`

辅助检查命令：
- `git diff -- climbing_stairs.py tests/test_climbing_stairs.py`

验收条件：
- `climb_stairs(1)` 返回 `1`。
"#;

    let specs = planned_test_commands_from_markdown(markdown);

    assert_eq!(specs.len(), 2);
    assert_eq!(specs[0].id, "planned_001");
    assert_eq!(
        specs[0].command,
        vec![
            "uv",
            "run",
            "python",
            "-m",
            "unittest",
            "-v",
            "tests.test_climbing_stairs"
        ]
    );
    assert_eq!(specs[1].id, "planned_002");
    assert_eq!(
        specs[1].command,
        vec![
            "git",
            "diff",
            "--",
            "climbing_stairs.py",
            "tests/test_climbing_stairs.py"
        ]
    );
}

#[test]
fn extracts_planned_verification_commands_from_fenced_code_blocks() {
    let markdown = r#"
# 爬楼梯问题 Work Item

## 验证命令

首选无第三方测试依赖命令：

```bash
uv run python -m unittest discover -s tests -v
```

范围检查命令：

```bash
git diff -- climbing_stairs.py tests/test_climbing_stairs.py
```
"#;

    let specs = planned_test_commands_from_markdown(markdown);

    assert_eq!(specs.len(), 2);
    assert_eq!(
        specs[0].command,
        vec![
            "uv", "run", "python", "-m", "unittest", "discover", "-s", "tests", "-v"
        ]
    );
    assert_eq!(
        specs[1].command,
        vec![
            "git",
            "diff",
            "--",
            "climbing_stairs.py",
            "tests/test_climbing_stairs.py"
        ]
    );
}

#[test]
fn normalizes_planned_pnpm_commands_from_cd_web_form() {
    let markdown = r#"
# Provider 依赖 Work Item

## 验证命令

- `cargo test --locked --lib provider_dependencies`
- `cd web && pnpm test`
- `cd web && pnpm build`
- `cd web && pnpm test:e2e`
"#;

    let specs = planned_test_commands_from_markdown(markdown);

    assert_eq!(specs.len(), 4);
    assert_eq!(
        specs[0].command,
        vec![
            "cargo",
            "test",
            "--locked",
            "--lib",
            "provider_dependencies"
        ]
    );
    assert_eq!(specs[1].command, vec!["pnpm", "-C", "web", "test"]);
    assert_eq!(specs[2].command, vec!["pnpm", "-C", "web", "build"]);
    assert_eq!(specs[3].command, vec!["pnpm", "-C", "web", "test:e2e"]);
}

#[tokio::test]
async fn executes_test_command_and_records_stdout_stderr_artifacts() {
    let root = tempdir().expect("root");
    let artifact_root = root.path().join("attempt-artifacts/test-output");
    let spec = TestCommandSpec {
        id: "unit".to_string(),
        command: vec![
            "sh".to_string(),
            "-c".to_string(),
            "printf stdout; printf stderr >&2".to_string(),
        ],
    };

    let command = execute_test_command(&spec, root.path(), &artifact_root)
        .await
        .expect("execute command");

    assert_eq!(command.command, spec.command);
    assert_eq!(command.cwd, root.path());
    assert_eq!(command.exit_code, Some(0));
    assert_eq!(command.status, TestCommandStatus::Passed);
    assert_eq!(command.stdout_ref, "unit.stdout.log");
    assert_eq!(command.stderr_ref, "unit.stderr.log");
    assert_eq!(
        fs::read_to_string(artifact_root.join(&command.stdout_ref)).expect("stdout"),
        "stdout"
    );
    assert_eq!(
        fs::read_to_string(artifact_root.join(&command.stderr_ref)).expect("stderr"),
        "stderr"
    );
}

#[tokio::test]
async fn run_all_tests_marks_report_failed_when_any_command_fails() {
    let root = tempdir().expect("root");
    let artifact_root = root.path().join("attempt-artifacts/test-output");
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

    let report = run_all_tests("coding_attempt_0001", root.path(), &artifact_root, &specs)
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

#[tokio::test]
async fn test_outputs_do_not_pollute_target_worktree_status() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "user.email", "aria@example.com"]);
    run_git(&repo, &["config", "user.name", "Aria Test"]);
    fs::write(repo.join("src.txt"), "hello\n").expect("seed");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "initial"]);
    let artifact_root = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/artifacts/test-output");
    let specs = vec![TestCommandSpec {
        id: "planned_001".to_string(),
        command: vec!["sh".to_string(), "-c".to_string(), "printf ok".to_string()],
    }];

    let report = run_all_tests("coding_attempt_0001", &repo, &artifact_root, &specs)
        .await
        .expect("run tests");

    assert_eq!(report.commands[0].stdout_ref, "planned_001.stdout.log");
    assert_eq!(
        fs::read_to_string(artifact_root.join("planned_001.stdout.log")).expect("stdout"),
        "ok"
    );
    assert!(!repo.join(".aria/coding-artifacts/test-output").exists());
    assert_eq!(git_stdout(&repo, &["status", "--short"]), "");
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:{}\nstderr:{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:{}\nstderr:{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}
