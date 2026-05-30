use cadence_aria::cross_cutting::worktree::{
    WorktreeError, WorktreeLeaseManager, WorktreeLeaseStatus, scopes_may_overlap,
    validate_write_path,
};
use std::fs;

#[test]
fn non_overlapping_scopes_acquire_in_parallel_and_overlapping_scope_waits() {
    let workspace = tempfile::tempdir().expect("workspace");
    fs::create_dir_all(workspace.path().join("src/auth")).expect("auth dir");
    fs::create_dir_all(workspace.path().join("src/billing")).expect("billing dir");

    let mut manager =
        WorktreeLeaseManager::new("session_001", "task_001", workspace.path(), "main");
    let auth = manager
        .acquire(
            "worktask_auth",
            "aria/worktask_auth",
            vec!["src/auth/".to_string()],
        )
        .expect("auth lease");
    let billing = manager
        .acquire(
            "worktask_billing",
            "aria/worktask_billing",
            vec!["src/billing/".to_string()],
        )
        .expect("billing lease");
    let waiting = manager
        .acquire(
            "worktask_login",
            "aria/worktask_login",
            vec!["src/auth/login.rs".to_string()],
        )
        .expect("waiting lease");

    assert_eq!(auth.status, WorktreeLeaseStatus::Acquired);
    assert_eq!(billing.status, WorktreeLeaseStatus::Acquired);
    assert_eq!(waiting.status, WorktreeLeaseStatus::Waiting);
    assert_eq!(
        waiting.blocked_by_lease_id.as_deref(),
        Some(auth.lease_id.as_str())
    );
    assert_eq!(
        manager.waiting_edges(),
        vec![(auth.lease_id.clone(), waiting.lease_id.clone())]
    );
    assert!(manager.events().iter().any(|event| {
        event.event_type == "worktree.lease_acquired"
            && event.payload["lease_id"] == auth.lease_id
            && event.payload["worktask_id"] == "worktask_auth"
            && event.payload.get("acquired_at").is_some()
    }));
    assert!(manager.snapshots().iter().any(|snapshot| {
        snapshot.node_id == "N14"
            && snapshot.node_specific_fields["lease_id"] == waiting.lease_id
            && snapshot.node_specific_fields["status"] == "waiting"
            && snapshot.node_specific_fields["reason"] == "worktree_scope_conflict_waiting"
    }));

    manager.release(&auth.lease_id).expect("release auth");
    let promoted = manager.lease(&waiting.lease_id).expect("promoted lease");
    assert_eq!(promoted.status, WorktreeLeaseStatus::Acquired);
    assert!(promoted.blocked_by_lease_id.is_none());
    assert!(promoted.acquired_at.is_some());
}

#[test]
fn scope_overlap_and_path_normalization_reject_forbidden_or_escaping_paths() {
    assert!(scopes_may_overlap(
        &["src/auth/".to_string()],
        &["src/auth/login.rs".to_string()],
        true,
    ));
    assert!(!scopes_may_overlap(
        &["src/auth/".to_string()],
        &["src/billing/".to_string()],
        true,
    ));
    assert!(scopes_may_overlap(
        &["SRC/Auth/".to_string()],
        &["src/auth/login.rs".to_string()],
        false,
    ));
    assert!(!scopes_may_overlap(
        &["SRC/Auth/".to_string()],
        &["src/auth/login.rs".to_string()],
        true,
    ));

    let workspace = tempfile::tempdir().expect("workspace");
    fs::create_dir_all(workspace.path().join("src/auth")).expect("auth dir");
    fs::create_dir_all(workspace.path().join(".git")).expect("git dir");
    fs::write(workspace.path().join(".git/config"), "").expect("git config");
    fs::write(workspace.path().join("src/auth/login.rs"), "").expect("login file");

    assert_eq!(
        validate_write_path(
            workspace.path(),
            &["*".to_string()],
            &workspace.path().join(".git/config"),
            true,
        )
        .expect_err(".git writes are forbidden"),
        WorktreeError::ForbiddenPath(".git/config".to_string())
    );
    assert_eq!(
        validate_write_path(
            workspace.path(),
            &Vec::<String>::new(),
            &workspace.path().join("src/auth/login.rs"),
            true,
        )
        .expect_err("empty scope is read-only"),
        WorktreeError::ScopeDenied("src/auth/login.rs".to_string())
    );

    let outside = tempfile::tempdir().expect("outside");
    let outside_file = outside.path().join("secret.txt");
    fs::write(&outside_file, "secret").expect("outside file");
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        &outside_file,
        workspace.path().join("src/auth/outside_link"),
    )
    .expect("symlink");

    #[cfg(unix)]
    assert_eq!(
        validate_write_path(
            workspace.path(),
            &["src/auth/".to_string()],
            &workspace.path().join("src/auth/outside_link"),
            true,
        )
        .expect_err("symlink escape is rejected"),
        WorktreeError::SymlinkEscape
    );
}
