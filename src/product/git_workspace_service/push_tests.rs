use super::push_output_is_explicit_remote_rejection;

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
