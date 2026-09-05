// Task 3.3（REQ-ENV-09/GC9）：pi 版本映射（`PiVersion::Unknown(TimedOut)→Timeout`、
// 其余 `Unknown(_)→Unavailable`；仅策略会话走 fail-closed 映射，既有
// `ensure_pi_version_compatible` 的 unknown→Ok 旧路径零变化）。

use std::time::Duration;

use super::*;
use crate::cross_cutting::streaming_provider::VersionProbeError;

#[test]
fn pi_policy_version_maps_probe_failures_fail_closed() {
    // Known 版本 → 精确字符串。
    assert_eq!(
        pi_policy_version(&PiVersion::Known((0, 83, 0))).unwrap(),
        "pi 0.83.0"
    );
    // TimedOut → Timeout；其余 Unknown（CommandFailed/Unparseable）→ Unavailable。
    assert!(matches!(
        pi_policy_version(&PiVersion::Unknown(ProbeFailure::TimedOut)),
        Err(VersionProbeError::Timeout)
    ));
    assert!(matches!(
        pi_policy_version(&PiVersion::Unknown(ProbeFailure::CommandFailed)),
        Err(VersionProbeError::Unavailable)
    ));
    assert!(matches!(
        pi_policy_version(&PiVersion::Unknown(ProbeFailure::Unparseable)),
        Err(VersionProbeError::Unavailable)
    ));
}

#[test]
fn pi_unknown_version_keeps_legacy_compatibility_gate_unchanged() {
    // 既有 `ensure_pi_version_compatible`：unknown → Ok（旧路径零变化）。
    assert!(ensure_pi_version_compatible(&PiVersion::Unknown(ProbeFailure::TimedOut)).is_ok());
    assert!(ensure_pi_version_compatible(&PiVersion::Known((0, 83, 0))).is_ok());
    assert!(ensure_pi_version_compatible(&PiVersion::Known((0, 82, 0))).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn pi_probe_with_timeout_maps_real_fixture_failures() {
    // 真实 probe_pi_version_with_timeout 走 fixture CLI：空输出（unparseable）→
    // Unavailable；挂起 → Timeout。既有 probe 签名保持零变化。
    let temp = tempfile::tempdir().expect("tempdir");
    let empty = temp.path().join("pi-version-empty.sh");
    std::fs::write(&empty, "#!/bin/sh\nexit 0\n").expect("write fixture");
    let hanging = temp.path().join("pi-version-hanging.sh");
    std::fs::write(&hanging, "#!/bin/sh\nsleep 5\n").expect("write fixture");
    for fixture in [&empty, &hanging] {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(fixture).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(fixture, permissions).expect("chmod fixture");
    }

    let unparseable = probe_pi_version_with_timeout(&empty, Duration::from_millis(300)).await;
    assert!(
        matches!(unparseable, PiVersion::Unknown(ProbeFailure::Unparseable)),
        "empty output must surface as Unparseable"
    );
    assert!(matches!(
        pi_policy_version(&unparseable),
        Err(VersionProbeError::Unavailable)
    ));

    let timed_out = probe_pi_version_with_timeout(&hanging, Duration::from_millis(300)).await;
    assert!(
        matches!(timed_out, PiVersion::Unknown(ProbeFailure::TimedOut)),
        "hanging probe must surface as TimedOut"
    );
    assert!(matches!(
        pi_policy_version(&timed_out),
        Err(VersionProbeError::Timeout)
    ));
}
