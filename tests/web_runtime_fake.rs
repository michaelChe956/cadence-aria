use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::types::{ConfirmTaskRequest, CreateTaskRequest};
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn web_runtime_fake_create_advance_confirm_and_projection() {
    let workspace = tempdir().expect("workspace");
    let mut runtime = WebRuntime::new_fake(workspace.path().to_path_buf());

    let created = runtime
        .create_task(CreateTaskRequest {
            request_text: "实现 Fibonacci square sum".to_string(),
            change_id: "aria-fibonacci-square".to_string(),
            policy_preset: "manual-write".to_string(),
            provider_mode: "fake".to_string(),
            timeout_secs: 2400,
        })
        .expect("create");
    assert_eq!(created.task_id, "task_0001");

    let paused = runtime.advance_task(&created.task_id).expect("advance");
    let pending = paused.expect_pending_step().expect("pending");
    assert_eq!(pending.node_id, "N16");

    let confirmed = runtime
        .confirm_task(
            &created.task_id,
            ConfirmTaskRequest {
                checkpoint_id: pending.checkpoint_id,
                prompt: "确认执行 N16".to_string(),
                policy_override: None,
            },
        )
        .expect("confirm");
    assert_eq!(confirmed.node_id, "N16");

    let projection = runtime
        .projection(Some(&created.task_id), Some("N16"))
        .expect("projection");
    assert_eq!(projection.active_task_id, Some("task_0001".to_string()));
    assert!(
        projection
            .timeline
            .iter()
            .any(|item| item["node_id"] == "N16")
    );
}

#[test]
fn web_runtime_fake_rollback_preview_and_execute_restores_workspace_history() {
    let workspace = tempdir().expect("workspace");
    git(workspace.path(), &["init", "-b", "main"]);
    git(workspace.path(), &["config", "user.name", "Aria Test"]);
    git(
        workspace.path(),
        &["config", "user.email", "aria-test@example.com"],
    );
    fs::create_dir_all(workspace.path().join("src")).expect("src");
    fs::write(
        workspace.path().join("src/fibonacciSquareSum.js"),
        "export const ok = false;\n",
    )
    .expect("before");
    git(workspace.path(), &["add", "src/fibonacciSquareSum.js"]);
    git(workspace.path(), &["commit", "-m", "before"]);

    let mut runtime = WebRuntime::new_fake(workspace.path().to_path_buf());
    let created = runtime
        .create_task(CreateTaskRequest {
            request_text: "实现 Fibonacci square sum".to_string(),
            change_id: "aria-fibonacci-square".to_string(),
            policy_preset: "manual-write".to_string(),
            provider_mode: "fake".to_string(),
            timeout_secs: 2400,
        })
        .expect("create");
    let pending = runtime
        .advance_task(&created.task_id)
        .expect("advance")
        .expect_pending_step()
        .expect("pending");
    runtime
        .confirm_task(
            &created.task_id,
            ConfirmTaskRequest {
                checkpoint_id: pending.checkpoint_id.clone(),
                prompt: "确认执行 N16".to_string(),
                policy_override: None,
            },
        )
        .expect("confirm");
    fs::write(
        workspace.path().join("src/fibonacciSquareSum.js"),
        "export const ok = true;\n",
    )
    .expect("dirty after");

    let preview = runtime
        .rollback_preview(&created.task_id, &pending.checkpoint_id)
        .expect("preview");
    assert_eq!(preview.checkpoint_id, "ckpt_0001");
    assert!(preview.dirty);
    assert!(preview.turns_to_drop > 0);

    let completed = runtime
        .rollback(&created.task_id, &pending.checkpoint_id, true)
        .expect("rollback");
    assert_eq!(completed.status, "rollback_completed");
    assert_eq!(
        fs::read_to_string(workspace.path().join("src/fibonacciSquareSum.js")).expect("source"),
        "export const ok = false;\n"
    );
    assert!(
        fs::read_to_string(
            workspace
                .path()
                .join(".aria/runtime/tasks/task_0001/turns/turn_0001.json")
        )
        .expect("turn")
        .contains("\"dropped\": true")
    );
    assert!(
        fs::read_to_string(
            workspace
                .path()
                .join(".aria/runtime/tasks/task_0001/artifacts/execution/0000.json")
        )
        .expect("artifact")
        .contains("\"dropped\": true")
    );
    let projection = runtime
        .projection(Some(&created.task_id), Some("N16"))
        .expect("projection");
    assert!(projection.pending_provider_step.is_some());
    assert!(projection.timeline.iter().any(|item| {
        item["node_id"] == "N16"
            && item.get("dropped").and_then(serde_json::Value::as_bool) == Some(true)
    }));
    assert!(!projection.timeline.iter().any(|item| {
        item["node_id"] == "N00"
            && item.get("dropped").and_then(serde_json::Value::as_bool) == Some(true)
    }));
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
