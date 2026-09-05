// Task 3.3（REQ-ENV-09/GC9）：claude CLI `--version` 有界探测三态 fail-closed
// （成功非空=精确字符串；空输出/命令失败=Unavailable；超时=Timeout）。

use std::time::Duration;

use crate::cross_cutting::claude_code_provider::probe_claude_version;
use crate::cross_cutting::streaming_provider::VersionProbeError;

#[cfg(unix)]
fn version_fixture(kind: &str) -> std::path::PathBuf {
    let temp = tempfile::tempdir().expect("tempdir");
    let body = match kind {
        // 成功：精确输出（含前后空白，探测返回原样字符串）。
        "ok" => "#!/bin/sh\necho 'provider 1.2.3'\n",
        // 空输出（exit 0）：不可得。
        "empty" => "#!/bin/sh\nexit 0\n",
        // 命令失败（exit 1）：不可得。
        "failing" => "#!/bin/sh\nexit 1\n",
        // 超时：挂起超过有界超时。
        "timeout" => "#!/bin/sh\nsleep 5\n",
        other => panic!("unknown probe fixture kind: {other}"),
    };
    let path = temp.path().join(format!("claude-version-{kind}.sh"));
    std::fs::write(&path, body).expect("write fixture");
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("chmod fixture");
    let _ = temp.keep();
    path
}

#[cfg(unix)]
fn probe_fixture(kind: &str) -> Result<String, VersionProbeError> {
    // 有界超时用短值（测试环境），生产常量见 CLAUDE_VERSION_PROBE_TIMEOUT。
    tokio_test_block_on(kind)
}

#[cfg(unix)]
fn tokio_test_block_on(kind: &str) -> Result<String, VersionProbeError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
        .block_on(probe_claude_version(
            &version_fixture(kind),
            Duration::from_millis(300),
        ))
}

#[cfg(unix)]
#[test]
fn version_probe_is_fail_closed_for_success_empty_and_timeout() {
    assert_eq!(
        probe_fixture("ok").unwrap().trim(),
        "provider 1.2.3",
        "successful probe must return the exact CLI version string"
    );
    assert!(
        matches!(probe_fixture("empty"), Err(VersionProbeError::Unavailable)),
        "empty output must be Unavailable"
    );
    assert!(
        matches!(
            probe_fixture("failing"),
            Err(VersionProbeError::Unavailable)
        ),
        "command failure must be Unavailable"
    );
    assert!(
        matches!(probe_fixture("timeout"), Err(VersionProbeError::Timeout)),
        "bounded timeout must be Timeout"
    );
}
