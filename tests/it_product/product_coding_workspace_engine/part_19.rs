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
        store
            .get_coding_git_operation(&prepared)
            .expect("journal")
            .is_some_and(|journal| {
                journal.phase
                    == cadence_aria::product::coding_attempt_store::CodingGitOperationPhase::Completed
                    && journal.push_status == Some(PushStatus::Failed)
            })
    );
    assert_eq!(
        store
            .get_attempt(&prepared.project_id, &prepared.issue_id, &prepared.id)
            .expect("blocked attempt")
            .status,
        CodingAttemptStatus::Blocked
    );
    let remote_ref = format!("refs/heads/{}", prepared.branch_name);
    let remote_head = Command::new("git")
        .args(["rev-parse", "--verify", &remote_ref])
        .current_dir(&remote)
        .output()
        .expect("query remote ref");
    assert!(!remote_head.status.success());
}

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
