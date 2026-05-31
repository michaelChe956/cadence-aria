use cadence_aria::interactive::checkpoint::{CheckpointService, RollbackPreviewRequest};
use cadence_aria::interactive::models::RuntimeCheckpoint;
use serde_json::json;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn rollback_preview_counts_later_records_and_dirty_files() {
    let workspace = tempdir().expect("workspace");
    git(workspace.path(), &["init", "-b", "main"]);
    git(workspace.path(), &["config", "user.name", "Aria Test"]);
    git(
        workspace.path(),
        &["config", "user.email", "aria-test@example.com"],
    );
    fs::write(workspace.path().join("file.txt"), "before\n").expect("file");
    git(workspace.path(), &["add", "file.txt"]);
    git(workspace.path(), &["commit", "-m", "before"]);
    let head = git_stdout(workspace.path(), &["rev-parse", "HEAD"]);

    let task_root = workspace.path().join(".aria/runtime/tasks/task_0001");
    fs::create_dir_all(task_root.join("turns")).expect("turns");
    fs::create_dir_all(task_root.join("node-runs")).expect("node runs");
    fs::create_dir_all(task_root.join("provider-runs/run_n16_0001")).expect("runs");
    fs::write(
        task_root.join("turns/turn_0001.json"),
        r#"{"turn_id":"turn_0001"}"#,
    )
    .expect("turn");
    fs::write(
        task_root.join("node-runs/nrun_0001.json"),
        r#"{"node_run_id":"nrun_0001"}"#,
    )
    .expect("node");
    fs::write(
        task_root.join("provider-runs/run_n16_0001/run.json"),
        r#"{"provider_run_id":"run_n16_0001"}"#,
    )
    .expect("run");
    fs::write(workspace.path().join("file.txt"), "dirty\n").expect("dirty");

    let service = CheckpointService::new(workspace.path(), "task_0001");
    service
        .write_checkpoint(&RuntimeCheckpoint {
            checkpoint_id: "ckpt_0001".to_string(),
            task_id: "task_0001".to_string(),
            session_id: "sess_task_0001".to_string(),
            turn_id: Some("turn_0001".to_string()),
            git_head: Some(head),
            dirty_summary: json!({"tracked":0}),
            state_snapshot_ref: "state@ckpt_0001.json".to_string(),
            projection_snapshot_ref: "projection@ckpt_0001.json".to_string(),
            artifact_boundary: 0,
            provider_run_boundary: 0,
            node_run_boundary: 0,
            created_at: "2026-05-09T00:00:00Z".to_string(),
        })
        .expect("checkpoint");

    let preview = service
        .preview_rollback(RollbackPreviewRequest {
            checkpoint_id: "ckpt_0001".to_string(),
        })
        .expect("preview");

    assert_eq!(preview.checkpoint_id, "ckpt_0001");
    assert!(preview.dirty);
    assert_eq!(preview.turns_to_drop, 1);
    assert_eq!(preview.node_runs_to_drop, 1);
    assert_eq!(preview.provider_runs_to_drop, 1);
    assert!(preview.files_may_change.contains(&"file.txt".to_string()));
}

fn git(cwd: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {:?} failed", args);
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
