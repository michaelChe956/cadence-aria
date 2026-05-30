use cadence_aria::interactive::checkpoint::{CheckpointService, RollbackRequest};
use cadence_aria::interactive::models::RuntimeCheckpoint;
use serde_json::json;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn rollback_restores_git_head_and_marks_later_history_dropped() {
    let workspace = tempdir().expect("workspace");
    git(workspace.path(), &["init", "-b", "main"]);
    git(workspace.path(), &["config", "user.name", "Aria Test"]);
    git(
        workspace.path(),
        &["config", "user.email", "aria-test@example.com"],
    );
    fs::write(workspace.path().join("file.txt"), "before\n").expect("write before");
    git(workspace.path(), &["add", "file.txt"]);
    git(workspace.path(), &["commit", "-m", "before"]);
    let before_head = git_stdout(workspace.path(), &["rev-parse", "HEAD"]);

    let task_root = workspace.path().join(".aria/runtime/tasks/task_0001");
    fs::create_dir_all(task_root.join("checkpoints")).expect("checkpoints");
    fs::create_dir_all(task_root.join("turns")).expect("turns");
    fs::create_dir_all(task_root.join("node-runs")).expect("node runs");
    fs::create_dir_all(task_root.join("provider-runs/run_n16_0001")).expect("provider runs");
    fs::create_dir_all(task_root.join("artifacts/execution")).expect("artifacts");
    fs::create_dir_all(task_root.join("reports")).expect("reports");
    fs::write(
        task_root.join("checkpoints/state@ckpt_0001.json"),
        serde_json::to_vec_pretty(&json!({"phase":"before_checkpoint"})).expect("state snapshot"),
    )
    .expect("write state snapshot");
    fs::write(
        task_root.join("checkpoints/projection@ckpt_0001.json"),
        serde_json::to_vec_pretty(&json!({"overview":{"phase":"before_checkpoint"}}))
            .expect("projection snapshot"),
    )
    .expect("write projection snapshot");
    fs::write(
        task_root.join("state.json"),
        serde_json::to_vec_pretty(&json!({"phase":"after_checkpoint"})).expect("state active"),
    )
    .expect("write active state");
    fs::write(
        task_root.join("projection.json"),
        serde_json::to_vec_pretty(&json!({"overview":{"phase":"after_checkpoint"}}))
            .expect("projection active"),
    )
    .expect("write active projection");
    fs::write(
        task_root.join("turns/turn_0001.json"),
        serde_json::to_vec_pretty(&json!({"turn_id":"turn_0001","dropped":false}))
            .expect("turn json"),
    )
    .expect("write turn");
    fs::write(
        task_root.join("node-runs/nrun_0001.json"),
        serde_json::to_vec_pretty(&json!({"node_run_id":"nrun_0001","dropped":false}))
            .expect("node json"),
    )
    .expect("write node");
    fs::write(
        task_root.join("provider-runs/run_n16_0001/run.json"),
        serde_json::to_vec_pretty(&json!({"provider_run_id":"run_n16_0001","dropped":false}))
            .expect("run json"),
    )
    .expect("write run");
    fs::write(
        task_root.join("artifacts/execution/0000.json"),
        serde_json::to_vec_pretty(&json!({"artifact_ref":"artifact_0001","dropped":false}))
            .expect("artifact json"),
    )
    .expect("write artifact");
    fs::write(
        task_root.join("reports/report_0001.json"),
        serde_json::to_vec_pretty(&json!({"report_id":"report_0001","dropped":false}))
            .expect("report json"),
    )
    .expect("write report");

    fs::write(workspace.path().join("file.txt"), "after\n").expect("write after");
    git(workspace.path(), &["add", "file.txt"]);
    git(workspace.path(), &["commit", "-m", "after"]);

    let service = CheckpointService::new(workspace.path(), "task_0001");
    let checkpoint = RuntimeCheckpoint {
        checkpoint_id: "ckpt_0001".to_string(),
        task_id: "task_0001".to_string(),
        session_id: "sess_task_0001".to_string(),
        turn_id: Some("turn_0001".to_string()),
        git_head: Some(before_head.clone()),
        dirty_summary: json!({"tracked":0,"untracked":0}),
        state_snapshot_ref: "state@ckpt_0001.json".to_string(),
        projection_snapshot_ref: "projection@ckpt_0001.json".to_string(),
        artifact_boundary: 0,
        provider_run_boundary: 0,
        node_run_boundary: 0,
        created_at: "2026-05-07T00:00:00Z".to_string(),
    };
    service
        .write_checkpoint(&checkpoint)
        .expect("write checkpoint");

    service
        .rollback(RollbackRequest {
            checkpoint_id: "ckpt_0001".to_string(),
            force_when_dirty: true,
        })
        .expect("rollback");

    assert_eq!(
        git_stdout(workspace.path(), &["rev-parse", "HEAD"]),
        before_head
    );
    assert_eq!(
        fs::read_to_string(workspace.path().join("file.txt")).expect("file"),
        "before\n"
    );
    assert!(
        fs::read_to_string(task_root.join("state.json"))
            .expect("state")
            .contains("before_checkpoint")
    );
    assert!(
        fs::read_to_string(task_root.join("projection.json"))
            .expect("projection")
            .contains("before_checkpoint")
    );
    assert!(
        fs::read_to_string(task_root.join("turns/turn_0001.json"))
            .expect("turn")
            .contains("\"dropped\": true")
    );
    assert!(
        fs::read_to_string(task_root.join("node-runs/nrun_0001.json"))
            .expect("node")
            .contains("\"dropped\": true")
    );
    assert!(
        fs::read_to_string(task_root.join("provider-runs/run_n16_0001/run.json"))
            .expect("run")
            .contains("\"dropped\": true")
    );
    assert!(
        fs::read_to_string(task_root.join("artifacts/execution/0000.json"))
            .expect("artifact")
            .contains("\"dropped\": true")
    );
    assert!(
        fs::read_to_string(task_root.join("reports/report_0001.json"))
            .expect("report")
            .contains("\"dropped\": true")
    );
}

fn git(cwd: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {:?} failed stdout={} stderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(cwd: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
