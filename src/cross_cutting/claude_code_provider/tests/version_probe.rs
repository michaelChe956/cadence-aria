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

/// A1-b2(Bug45 批2 环境回退):版本探测裸名形态回归——探测请求的
/// `environment` 显式覆盖层携带前置 PATH 时裸名 executable 可解析执行;
/// 生产探测构造点现为空 env map(裸名靠底座回退宿主 PATH 解析,同语义
/// 由 bounded_command_runner 继承测试钉住),本测试钉覆盖层注入仍生效。
#[cfg(unix)]
#[tokio::test]
async fn bare_name_version_probe_resolves_via_explicit_path_override() {
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandRequest, TokioBoundedCommandRunner,
    };

    let root = std::path::Path::new("/tmp/aria-bug45/t3");
    std::fs::create_dir_all(root).expect("t3 fixture root");
    let dir = tempfile::tempdir_in(root).expect("t3 fixture dir");
    let script = dir.path().join("aria-t3-bare-version");
    std::fs::write(&script, "#!/bin/sh\necho 'provider 9.9.9'\n").expect("write fixture");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fixture");

    // 探测形态=裸名 executable;显式 PATH 覆盖层(fixture 目录前置)
    // 保证确定性。
    let host_path = std::env::var_os("PATH")
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default();
    let environment = std::collections::BTreeMap::from([(
        "PATH".to_string(),
        format!("{}:{host_path}", dir.path().display()),
    )]);

    let request = BoundedCommandRequest {
        executable: "aria-t3-bare-version".to_string(),
        argv: vec!["--version".to_string()],
        working_dir: dir.path().to_path_buf(),
        timeout: std::time::Duration::from_secs(10),
        cancellation: tokio_util::sync::CancellationToken::new(),
        environment,
        stdout_limit: 64 * 1024,
        stderr_limit: 64 * 1024,
    };
    let result = TokioBoundedCommandRunner
        .run_inherited(request)
        .await
        .expect("裸名探测必须经显式 PATH 可解析执行");
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.stdout.trim(), "provider 9.9.9");
}
