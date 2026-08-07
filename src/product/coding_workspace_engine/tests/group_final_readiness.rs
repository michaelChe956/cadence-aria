use super::*;

#[test]
fn group_final_readiness_store_is_available_to_workspace_engine() {
    let (_root, store, attempt) = running_attempt_with_worktree();

    assert_eq!(
        store
            .get_group_final_readiness_snapshot(&attempt)
            .expect("legacy attempt without snapshot"),
        None
    );
}
