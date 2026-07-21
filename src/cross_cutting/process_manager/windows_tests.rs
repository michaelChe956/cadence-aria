use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

use super::ProcessManager;

#[tokio::test]
async fn windows_managed_process_uses_job_object_and_kills_child_tree_on_drop() {
    let root = tempdir().expect("tempdir");
    let entered = root.path().join("entered.txt");
    let late_marker = root.path().join("late-marker.txt");
    let child_script = root.path().join("late-child.cmd");
    let parent_script = root.path().join("process-tree.cmd");
    fs::write(
        &child_script,
        "@echo off\r\nping -n 3 127.0.0.1 >nul\r\necho late>\"%~1\"\r\n",
    )
    .expect("write child script");
    fs::write(
        &parent_script,
        "@echo off\r\necho entered>\"%~1\"\r\nstart \"\" /b cmd.exe /d /c call \"%~2\" \"%~3\"\r\nping -n 10 127.0.0.1 >nul\r\n",
    )
    .expect("write parent script");
    let parent = parent_script.to_string_lossy().to_string();
    let entered_arg = entered.to_string_lossy().to_string();
    let child = child_script.to_string_lossy().to_string();
    let late_arg = late_marker.to_string_lossy().to_string();
    let process = ProcessManager::spawn(
        "cmd.exe",
        &["/d", "/c", "call", &parent, &entered_arg, &child, &late_arg],
        root.path(),
        &BTreeMap::new(),
        CancellationToken::new(),
    )
    .await
    .expect("spawn Windows process tree");

    fn assert_job_object_child(_: &command_group::AsyncGroupChild) {}
    assert_job_object_child(&process.child.child);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !entered.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Windows parent process entered");

    drop(process);
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        !late_marker.exists(),
        "dropping the Job Object must terminate the child process tree"
    );
}
