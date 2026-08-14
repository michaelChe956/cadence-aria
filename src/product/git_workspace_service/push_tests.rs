use std::ffi::OsStr;
use std::path::Path;

use super::{
    git_command, push_output_is_explicit_remote_rejection, remote_delete_output_is_missing_ref,
};

#[test]
fn localized_missing_ref_output_is_not_recognized() {
    // 评审 I1：非 C locale 下 git 会本地化诊断短语，例如
    //   LC_ALL=zh_CN.utf8 git push --delete ...
    // 输出「远程引用不存在」而非英文 "remote ref does not exist"。
    // 该匹配器只认英文（见下），故 `run_git_allow_failure` 必须固定
    // `LC_ALL=C`（见 git_command）才能保证 delete_remote_branch 幂等判定
    // 在任意 locale 下都成立；本测试钉死「匹配器只认英文」这一前提。
    for stderr in [
        "error: 无法删除 'aria/work-items/work_item_0001/attempt-1'：远程引用不存在\nerror: 无法推送一些引用到 'origin'",
        "error: unable to delete 'aria/work-items/work_item_0001/attempt-1': 远程引用不存在",
    ] {
        assert!(
            !remote_delete_output_is_missing_ref(stderr),
            "localized stderr must not be recognized as missing-ref: {stderr}"
        );
    }
}

#[test]
fn git_command_forces_c_locale() {
    // 不依赖 locale 的行为级证据：git 子进程命令必须显式携带 LC_ALL=C，
    // 与上面的「匹配器只认英文」配套，构成幂等判定的完整保证（评审 I1）。
    let command = git_command(Path::new("/unused"), &["push", "origin", "--delete", "x"]);
    let lc_all = command
        .as_std()
        .get_envs()
        .find_map(|(key, value)| (key == OsStr::new("LC_ALL")).then_some(value));
    assert_eq!(
        lc_all,
        Some(Some(OsStr::new("C"))),
        "run_git_allow_failure must force LC_ALL=C so git cannot localize error phrases"
    );
}

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
