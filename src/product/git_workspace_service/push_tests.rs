use super::{push_output_is_explicit_remote_rejection, remote_delete_output_is_missing_ref};

#[test]
fn missing_remote_ref_delete_output_is_idempotent() {
    for stderr in [
        "error: unable to delete 'aria/work-items/work_item_0001/attempt-1': remote ref does not exist\nerror: failed to push some refs to 'origin'",
        "remote: remote ref does not exist",
    ] {
        assert!(
            remote_delete_output_is_missing_ref(stderr),
            "expected missing-ref delete output: {stderr}"
        );
    }
}

#[test]
fn genuine_remote_delete_failures_are_not_missing_ref() {
    for stderr in [
        "fatal: 'does-not-exist' does not appear to be a git repository",
        "fatal: Could not read from remote repository.",
        "! [remote rejected] aria/work-items/work_item_0001/attempt-1 (pre-receive hook declined)",
        "send-pack: unexpected disconnect while reading sideband packet",
        "",
    ] {
        assert!(
            !remote_delete_output_is_missing_ref(stderr),
            "must remain a real failure: {stderr}"
        );
    }
}

#[test]
fn porcelain_remote_rejections_are_terminal() {
    for stderr in [
        "! [remote rejected] feature -> feature (pre-receive hook declined)",
        "! [rejected] feature -> feature (non-fast-forward)",
        "remote: error: GH006: Protected branch update failed",
    ] {
        assert!(
            push_output_is_explicit_remote_rejection(stderr),
            "expected explicit rejection: {stderr}"
        );
    }
}

#[test]
fn transport_and_local_failures_remain_ambiguous() {
    for stderr in [
        "send-pack: unexpected disconnect while reading sideband packet",
        "fatal: the remote end hung up unexpectedly",
        "fatal: unable to access: connection reset by peer",
        "fatal: could not read Username for 'https://example.test'",
    ] {
        assert!(
            !push_output_is_explicit_remote_rejection(stderr),
            "transport/local failure must remain ambiguous: {stderr}"
        );
    }
}
