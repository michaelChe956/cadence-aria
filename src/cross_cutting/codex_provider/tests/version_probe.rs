// Task 3.3（REQ-ENV-09/GC9）：codex CLI `--version` 有界探测三态 fail-closed。

use std::time::Duration;

use crate::cross_cutting::codex_provider::probe_codex_version;
use crate::cross_cutting::streaming_provider::VersionProbeError;

#[cfg(unix)]
fn version_fixture(kind: &str) -> std::path::PathBuf {
    let temp = tempfile::tempdir().expect("tempdir");
    let body = match kind {
        "ok" => "#!/bin/sh\necho 'provider 1.2.3'\n",
        "empty" => "#!/bin/sh\nexit 0\n",
        "failing" => "#!/bin/sh\nexit 1\n",
        "timeout" => "#!/bin/sh\nsleep 5\n",
        other => panic!("unknown probe fixture kind: {other}"),
    };
    let path = temp.path().join(format!("codex-version-{kind}.sh"));
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
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
        .block_on(probe_codex_version(
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
    assert!(matches!(
        probe_fixture("empty"),
        Err(VersionProbeError::Unavailable)
    ));
    assert!(matches!(
        probe_fixture("failing"),
        Err(VersionProbeError::Unavailable)
    ));
    assert!(matches!(
        probe_fixture("timeout"),
        Err(VersionProbeError::Timeout)
    ));
}
