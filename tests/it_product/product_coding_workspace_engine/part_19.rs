#[cfg(unix)]
#[tokio::test]
async fn rejected_push_with_verified_missing_remote_ref_records_failed_review_request() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    let remote = root.path().join("remote.git");
    init_repo(&repo);
    run_git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
    let hook = remote.join("hooks/pre-receive");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").expect("rejecting hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).expect("chmod hook");
    run_git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let started = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");
    let _stage = rx.recv().await.expect("stage event");
    let _node = rx.recv().await.expect("node event");
    let prepared = engine
        .execute_worktree_prepare(&started, &repo)
        .await
        .expect("prepare worktree");
    let _worktree_complete = rx.recv().await.expect("worktree complete");
    let worktree = prepared.worktree_path.as_ref().expect("worktree path");
    fs::write(worktree.join("src.txt"), "hello\nrejected push\n").expect("modify file");

    let request = engine
        .execute_review_request(&prepared, "origin", "feat: rejected push")
        .await
        .expect("verified rejection creates failed review request");

    assert_eq!(request.push_status, PushStatus::Failed);
    assert!(
        request
            .push_error
            .as_ref()
            .is_some_and(|error| error.contains(&prepared.branch_name) && !error.is_empty()),
        "push_error should mention branch {} and keep detail; got {:?}",
        prepared.branch_name,
        request.push_error
    );
    assert!(
        store
            .get_coding_git_operation(&prepared)
            .expect("journal")
            .is_some_and(|journal| {
                journal.phase
                    == cadence_aria::product::coding_attempt_store::CodingGitOperationPhase::Completed
                    && journal.push_status == Some(PushStatus::Failed)
            })
    );
    assert_ne!(
        store
            .get_attempt(&prepared.project_id, &prepared.issue_id, &prepared.id)
            .expect("attempt after failed review request")
            .status,
        CodingAttemptStatus::Blocked,
        "push 失败不应再把 attempt 标记为 Blocked（应保持运行态以继续流转）"
    );
    let remote_ref = format!("refs/heads/{}", prepared.branch_name);
    let remote_head = Command::new("git")
        .args(["rev-parse", "--verify", &remote_ref])
        .current_dir(&remote)
        .output()
        .expect("query remote ref");
    assert!(!remote_head.status.success());
}

#[cfg(unix)]
#[tokio::test]
async fn nonzero_push_with_authoritatively_updated_remote_records_pushed_review_request() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    let remote = root.path().join("remote.git");
    init_repo(&repo);
    run_git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
    run_git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let git_exec_path = Command::new("git")
        .arg("--exec-path")
        .output()
        .expect("git exec path");
    assert!(git_exec_path.status.success());
    let receive_pack = PathBuf::from(
        String::from_utf8(git_exec_path.stdout)
            .expect("git exec path utf8")
            .trim(),
    )
    .join("git-receive-pack");
    let wrapper = root.path().join("receive-pack-nonzero");
    let wrapper_marker = root.path().join("receive-pack-finished");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\n'{}' \"$@\"\nstatus=$?\nprintf '%s' \"$status\" > '{}'\nexit 1\n",
            receive_pack.display(),
            wrapper_marker.display(),
        ),
    )
    .expect("receive-pack wrapper");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).expect("chmod wrapper");
    run_git(
        &repo,
        &[
            "config",
            "remote.origin.receivepack",
            wrapper.to_str().unwrap(),
        ],
    );
    let probe_push = Command::new("git")
        .args(["push", "origin", "HEAD:refs/heads/nonzero-probe"])
        .current_dir(&repo)
        .output()
        .expect("probe nonzero push");
    assert!(
        !probe_push.status.success(),
        "receive-pack wrapper must make the client exit nonzero"
    );
    let probe_remote = Command::new("git")
        .args(["rev-parse", "refs/heads/nonzero-probe"])
        .current_dir(&remote)
        .output()
        .expect("probe remote ref");
    assert!(
        probe_remote.status.success(),
        "nonzero client exit must still update the remote ref"
    );
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let started = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");
    let _stage = rx.recv().await.expect("stage event");
    let _node = rx.recv().await.expect("node event");
    let prepared = engine
        .execute_worktree_prepare(&started, &repo)
        .await
        .expect("prepare worktree");
    let _worktree_complete = rx.recv().await.expect("worktree complete");
    let worktree = prepared.worktree_path.as_ref().expect("worktree path");
    fs::write(worktree.join("src.txt"), "hello\naccepted nonzero push\n")
        .expect("modify file");

    let request = engine
        .execute_review_request(&prepared, "origin", "feat: accepted nonzero push")
        .await
        .expect("remote ref is authoritative");

    assert!(wrapper_marker.exists(), "receive-pack wrapper must run");
    assert_eq!(
        fs::read_to_string(&wrapper_marker).expect("wrapper status"),
        "0"
    );
    assert_eq!(request.push_status, PushStatus::Pushed);
    let remote_ref = format!("refs/heads/{}", prepared.branch_name);
    let remote_head = Command::new("git")
        .args(["rev-parse", &remote_ref])
        .current_dir(&remote)
        .output()
        .expect("read remote head");
    assert!(remote_head.status.success());
    assert_eq!(
        String::from_utf8(remote_head.stdout)
            .expect("remote head utf8")
            .trim(),
        request.commit_sha
    );
}

#[cfg(unix)]
#[tokio::test]
async fn ambiguous_push_with_initial_old_ref_converges_after_remote_updates_later() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    let remote = root.path().join("remote.git");
    init_repo(&repo);
    run_git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
    run_git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    let (tx, mut rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let started = engine
        .start_attempt("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("start attempt");
    let _stage = rx.recv().await.expect("stage event");
    let _node = rx.recv().await.expect("node event");
    let prepared = engine
        .execute_worktree_prepare(&started, &repo)
        .await
        .expect("prepare worktree");
    let _worktree_complete = rx.recv().await.expect("worktree complete");
    let worktree = prepared.worktree_path.as_ref().expect("worktree path");
    fs::write(worktree.join("src.txt"), "hello\ndelayed remote update\n")
        .expect("modify file");
    let wrapper = root.path().join("receive-pack-delayed-update");
    let wrapper_entered = root.path().join("receive-pack-entered");
    let updated = root.path().join("remote-updated");
    let update_status = root.path().join("remote-update-status");
    let update_error = root.path().join("remote-update-error");
    let remote_ref = format!("refs/heads/{}", prepared.branch_name);
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nprintf entered > '{}'\nnohup sh -c 'sleep 0.25; git -C \"$1\" push \"$2\" \"HEAD:$3\" 2> \"$6\"; status=$?; printf \"%s\" \"$status\" > \"$5\"; if [ \"$status\" -eq 0 ]; then git --git-dir=\"$2\" rev-parse \"$3\" > \"$4\"; fi' delayed '{}' '{}' '{}' '{}' '{}' '{}' </dev/null >/dev/null 2>&1 &\nexit 1\n",
            wrapper_entered.display(),
            worktree.display(),
            remote.display(),
            remote_ref,
            updated.display(),
            update_status.display(),
            update_error.display(),
        ),
    )
    .expect("delayed receive-pack wrapper");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).expect("chmod wrapper");
    run_git(
        worktree,
        &[
            "config",
            "remote.origin.receivepack",
            wrapper.to_str().unwrap(),
        ],
    );

    let first_error = engine
        .execute_review_request(&prepared, "origin", "feat: delayed remote update")
        .await
        .expect_err("first old-ref query must remain indeterminate");
    assert!(matches!(
        first_error,
        cadence_aria::product::coding_workspace_engine::CodingWorkspaceEngineError::Git(
            cadence_aria::product::git_workspace_service::GitWorkspaceError::PushIndeterminate {
                ..
            }
        )
    ));
    assert!(wrapper_entered.exists(), "receive-pack wrapper must run");
    let pending_attempt = store
        .get_attempt(&prepared.project_id, &prepared.issue_id, &prepared.id)
        .expect("pending attempt");
    let pending_journal = store
        .get_coding_git_operation(&pending_attempt)
        .expect("pending journal")
        .expect("push journal");
    assert_eq!(
        pending_journal.phase,
        cadence_aria::product::coding_attempt_store::CodingGitOperationPhase::PushStarted
    );
    assert!(
        store
            .list_review_requests(&prepared.project_id, &prepared.issue_id, &prepared.id)
            .expect("review requests")
            .is_empty()
    );
    let updated_later = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !updated.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        updated_later.is_ok(),
        "remote must update later; status={:?}; error={:?}",
        fs::read_to_string(&update_status).ok(),
        fs::read_to_string(&update_error).ok()
    );

    let request = engine
        .execute_review_request(
            &pending_attempt,
            "origin",
            "feat: delayed remote update",
        )
        .await
        .expect("retry must converge from the remote ref");

    assert_eq!(request.push_status, PushStatus::Pushed);
    let remote_head = Command::new("git")
        .args(["rev-parse", &remote_ref])
        .current_dir(&remote)
        .output()
        .expect("read delayed remote head");
    assert!(remote_head.status.success());
    assert_eq!(
        String::from_utf8(remote_head.stdout)
            .expect("delayed remote head utf8")
            .trim(),
        request.commit_sha
    );
}
