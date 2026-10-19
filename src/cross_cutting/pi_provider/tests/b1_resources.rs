// B1-1(波 2):Pi adapter 的受信资源归属接线测试。两个生产 spawn 面
// (direct `start_owned` 与 LC `start_validated`)统一生命周期:
// 载体→reserve(失败零 spawn)→spawn→回填真实 PID+starttime→会话真实
// 收口(错误/取消/尾部)后注销;协议 Completed 不提前注销。
//
// 归属同源沿用 engine 真实键型(coder delivery 同款
// `b1_scope_env` 形态):Process 记录的 scope key/labels 与铸造载体的
// scope 完全一致,PID/starttime 与 fixture 子进程的 /proc 事实同源。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::process_manager::process_start_time_ticks;
use crate::cross_cutting::provider_resources::{
    OwnedResourceKind, ProviderResourceRecord, ProviderRunResourceScope, ResourceScopeKey,
    StartedIdentity, TrustedLaunchOwnership,
};
use crate::cross_cutting::streaming_provider::{ProviderEvent, ProviderVersionSupplier};
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
use crate::product::provider_resource_store::ProviderResourceStore;
use crate::product::reviewer_window_store::tree_key_of;
use crate::protocol::contracts::AdapterRole;

use super::*;

/// 以 engine 真实键型开 B1 测试 scope(coder delivery 同款;私有根即可
/// 充当协调 worktree 锚)。返回 (base, scope, tree_key)。
#[cfg(unix)]
fn b1_pi_scope_env(tag: &str) -> (tempfile::TempDir, Arc<ProviderRunResourceScope>, String) {
    let base = tempfile::tempdir().expect("b1 pi scope base");
    let private_root = base.path().join("private-root");
    std::fs::create_dir_all(&private_root).expect("private root");
    let store_root = base.path().join("resource-store");
    std::fs::create_dir_all(&store_root).expect("store root");
    let key = ResourceScopeKey {
        scope_id: format!("code-delivery-attempt-{tag}"),
        run_id: format!("role-run-{tag}"),
        boot_identity: format!("aria-engine-{}-{tag}", std::process::id()),
    };
    let worktree = private_root.canonicalize().expect("canonical worktree");
    let tree_key = tree_key_of(&worktree);
    let scope =
        ProviderRunResourceScope::open_for_recovery(&key, None, &private_root, &store_root, None)
            .expect("open b1 pi scope")
            .with_worktree_key(&tree_key);
    (base, Arc::new(scope), tree_key)
}

/// 同 store_root 但把 store 根换成普通文件——reserve 落盘必败(预留失败
/// 零 spawn 断言用)。
#[cfg(unix)]
fn b1_pi_broken_store_scope_env(tag: &str) -> (tempfile::TempDir, Arc<ProviderRunResourceScope>) {
    let base = tempfile::tempdir().expect("b1 pi broken store base");
    let private_root = base.path().join("private-root");
    std::fs::create_dir_all(&private_root).expect("private root");
    let store_root = base.path().join("resource-store-file");
    std::fs::write(&store_root, b"not a dir").expect("store root file");
    let key = ResourceScopeKey {
        scope_id: format!("code-delivery-attempt-{tag}"),
        run_id: format!("role-run-{tag}"),
        boot_identity: format!("aria-engine-{}-{tag}", std::process::id()),
    };
    let worktree = private_root.canonicalize().expect("canonical worktree");
    let scope =
        ProviderRunResourceScope::open_for_recovery(&key, None, &private_root, &store_root, None)
            .expect("open scope (store root lazily used)")
            .with_worktree_key(&tree_key_of(&worktree));
    (base, Arc::new(scope))
}

#[cfg(unix)]
fn b1_pi_process_records(store_root: &Path, key: &ResourceScopeKey) -> Vec<ProviderResourceRecord> {
    ProviderResourceStore::new(store_root)
        .list_scope(key)
        .expect("list scope")
        .into_iter()
        .filter(|record| matches!(record.kind, OwnedResourceKind::Process))
        .collect()
}

/// B1 fixture(fake pi RPC,POSIX sh):`--version` 打印兼容版本;rpc 循环
/// 前把 `$$` 写入 pid marker;get_state 按 `$PI_SESSION_ID` 应答(非空 →
/// `data.sessionId`,空 → success 无 sessionId),prompt 应答 success 并送
/// text_delta + agent_settled(协议 Completed);之后继续读 stdin 保持
/// 存活——终止只能来自测试 kill 或 adapter kill 链。
#[cfg(unix)]
fn b1_pi_rpc_fixture(dir: &Path, pid_marker: &Path) -> PathBuf {
    let body = r#"if [ "$1" = "--version" ]; then echo 0.83.0; exit 0; fi
echo $$ > "B1_PID_MARKER_PATH"
while IFS= read -r line; do
  id="$(printf '%s' "$line" | sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
  case "$line" in
    *'"get_state"'*)
      if [ -n "${PI_SESSION_ID:-}" ]; then
        echo "{\"type\":\"response\",\"id\":\"${id:-pi-1}\",\"command\":\"get_state\",\"success\":true,\"data\":{\"sessionId\":\"${PI_SESSION_ID}\"}}"
      else
        echo "{\"type\":\"response\",\"id\":\"${id:-pi-1}\",\"command\":\"get_state\",\"success\":true,\"data\":{}}"
      fi
      ;;
    *'"prompt"'*)
      echo "{\"type\":\"response\",\"id\":\"${id:-pi-2}\",\"success\":true}"
      echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"b1 pi done"}}'
      echo '{"type":"agent_settled"}'
      ;;
  esac
done"#
        .replace("B1_PID_MARKER_PATH", &pid_marker.display().to_string());
    write_executable(dir, "fake-pi-b1-rpc", &body)
}

/// 触碰 spawn marker 的 fixture:--version 分支不触碰;rpc 模式入口触碰
/// (零 spawn 断言用——版本探测子进程不算 provider 进程 spawn)。
#[cfg(unix)]
fn b1_pi_spawn_marker_fixture(dir: &Path, spawn_marker: &Path, version: &str) -> PathBuf {
    let body = format!(
        "if [ \"$1\" = \"--version\" ]; then echo {version}; exit 0; fi\ntouch \"{}\"\nwhile IFS= read -r line; do :; done",
        spawn_marker.display()
    );
    write_executable(dir, "fake-pi-b1-marker", &body)
}

/// 有界等待 fixture 登记子进程 pid(2s 上界)。
#[cfg(unix)]
fn b1_wait_for_pid_marker(marker: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(content) = std::fs::read_to_string(marker)
            && let Ok(pid) = content.trim().parse::<u32>()
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "fixture must register its pid at {}",
            marker.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// F10(adapter 级):direct/legacy 真实 spawn 的子进程环境必须继承宿主
/// PATH/HOME(A1 环境回退语义)——读 `/proc/<pid>/environ` 逐字比对。
#[cfg(target_os = "linux")]
fn assert_child_inherits_host_environment(pid: u32) {
    let environ = std::fs::read(format!("/proc/{pid}/environ"))
        .unwrap_or_else(|error| panic!("read /proc/{pid}/environ: {error}"));
    let entries: Vec<String> = environ
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
        .collect();
    let host_home = format!("HOME={}", std::env::var("HOME").expect("host HOME"));
    let host_path = format!("PATH={}", std::env::var("PATH").expect("host PATH"));
    assert!(
        entries.contains(&host_home),
        "F10:direct/legacy 子进程必须继承宿主 HOME(实测 environ 无 {host_home})"
    );
    assert!(
        entries.contains(&host_path),
        "F10:direct/legacy 子进程必须继承宿主 PATH(实测 environ 无宿主 PATH)"
    );
}

/// 有界轮询至 scope 内出现 Process 登记,返回首条(5s 上界)。
#[cfg(unix)]
fn b1_poll_process_record(store_root: &Path, key: &ResourceScopeKey) -> ProviderResourceRecord {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let records = b1_pi_process_records(store_root, key);
        if let Some(record) = records.into_iter().next() {
            return record;
        }
        assert!(
            Instant::now() < deadline,
            "Process 登记必须在 spawn 后出现: {store_root:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 有界轮询至 scope 内 Process 登记清空(收口注销,5s 上界)。
#[cfg(unix)]
fn b1_assert_records_released(store_root: &Path, key: &ResourceScopeKey) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if b1_pi_process_records(store_root, key).is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "会话收口后 Process 登记必须注销: {store_root:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 有界等待终态 Completed(返回产物全文;Failed 即 panic,10s 上界)。
#[cfg(unix)]
async fn b1_recv_completed(events: &mut tokio::sync::mpsc::Receiver<ProviderEvent>) -> String {
    let deadline = Duration::from_secs(10);
    loop {
        let event = tokio::time::timeout(deadline, events.recv())
            .await
            .expect("pi session must reach a terminal event within timeout")
            .expect("pi session event channel stays open");
        match event {
            ProviderEvent::Completed(completion) => return completion.full_output,
            ProviderEvent::Failed { message } => {
                panic!("b1 pi session must complete, failed instead: {message}")
            }
            _ => {}
        }
    }
}

/// 排空事件通道至关闭(会话真实终结;10s 上界)。
#[cfg(unix)]
async fn b1_drain_events_to_close(events: &mut tokio::sync::mpsc::Receiver<ProviderEvent>) {
    let deadline = Duration::from_secs(10);
    loop {
        match tokio::time::timeout(deadline, events.recv()).await {
            Ok(Some(_)) => continue,
            Ok(None) => return,
            Err(_) => panic!("pi session event channel must close within timeout"),
        }
    }
}

/// 向子进程发 SIGKILL(外部触发真实终止)。
#[cfg(unix)]
fn b1_kill_child(pid: u32) {
    let status = std::process::Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status()
        .expect("run kill");
    assert!(status.success(), "kill -9 {pid} must be delivered");
}

/// 有界轮询确认 pid 已从进程表消失(kill+reap 都发生;5s 上界)。
#[cfg(unix)]
fn b1_assert_process_killed_and_reaped(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let alive = std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !alive {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "pi child (pid {pid}) must be killed and reaped"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Process 记录核心断言:真实 PID+starttime(与 fixture 子进程 /proc 事实
/// 同源)与引擎 scope 归属同源(scope key/aria 标签/worktree/唯一资源 ID)。
#[cfg(unix)]
fn b1_assert_process_record_identity(
    record: &ProviderResourceRecord,
    ownership: &TrustedLaunchOwnership,
    pid: u32,
    tree_key: &str,
) {
    assert_eq!(
        record.started_identity,
        Some(StartedIdentity::Process {
            pid,
            start_time_ticks: process_start_time_ticks(pid)
                .expect("live fixture child starttime must be readable"),
        }),
        "登记身份必须是 spawn 点回填的真实 PID+starttime"
    );
    assert_eq!(
        record.key,
        *ownership.scope_key(),
        "登记归属同源于载体 scope"
    );
    assert_eq!(
        record
            .reserved_labels
            .get("aria.scope_id")
            .map(String::as_str),
        Some(ownership.scope_key().scope_id.as_str())
    );
    assert_eq!(
        record
            .reserved_labels
            .get("aria.run_id")
            .map(String::as_str),
        Some(ownership.scope_key().run_id.as_str())
    );
    assert_eq!(
        record
            .reserved_labels
            .get("aria.worktree")
            .map(String::as_str),
        Some(tree_key)
    );
    assert_eq!(
        record.reserved_labels.get("kind").map(String::as_str),
        Some("provider_process")
    );
    assert!(
        record.resource_id.starts_with("provider-process-"),
        "资源 ID 唯一形态: {}",
        record.resource_id
    );
    assert_eq!(record.resource_id, ownership.resource_id());
}

/// direct 面:载体经 `start_owned` 注入——spawn 点回填真实 PID+starttime,
/// Process 记录与引擎 scope 归属同源;协议 Completed 不提前注销;子进程
/// 真实终止(通道关闭)后才收口注销。
#[cfg(unix)]
#[tokio::test]
async fn pi_b1_start_owned_registers_process_and_releases_after_real_tail() {
    let (base, scope, tree_key) = b1_pi_scope_env("raw-happy");
    let store_root = base.path().join("resource-store");
    let marker_dir = tempfile::tempdir().expect("raw marker dir");
    let pid_marker = marker_dir.path().join("pi-b1-raw.pid");
    let provider = PiProvider::new(b1_pi_rpc_fixture(marker_dir.path(), &pid_marker));
    let ownership = TrustedLaunchOwnership::mint(
        Arc::clone(&scope),
        "code_delivery provider process: pi b1 raw",
    );
    let cancel = CancellationToken::new();

    let mut session = provider
        .start_owned(
            streaming_input_for_test(None),
            Some(ownership.clone()),
            cancel,
        )
        .await
        .expect("owned pi start");

    let pid = b1_wait_for_pid_marker(&pid_marker);
    // F10:direct/legacy 真实 spawn 环境含宿主 PATH/HOME。
    assert_child_inherits_host_environment(pid);
    let record = b1_poll_process_record(&store_root, scope.key());
    b1_assert_process_record_identity(&record, &ownership, pid, &tree_key);

    // 协议 Completed:登记仍在(不提前注销)。
    let completed = b1_recv_completed(&mut session.events).await;
    assert_eq!(completed, "b1 pi done");
    assert_eq!(
        b1_pi_process_records(&store_root, scope.key()).len(),
        1,
        "协议 Completed 不得提前注销"
    );

    // 真实终止:kill 子进程 → 泵终结 → 通道关闭 → 收口注销。
    b1_kill_child(pid);
    b1_drain_events_to_close(&mut session.events).await;
    b1_assert_records_released(&store_root, scope.key());
    b1_assert_process_killed_and_reaped(pid);
}

/// direct 面:预留失败(store 根不可写)→ 零 spawn——`start_owned` 直接
/// 失败,rpc 模式从未进入(marker 不存在)。
#[cfg(unix)]
#[tokio::test]
async fn pi_b1_start_owned_reserve_failure_means_zero_spawn() {
    let (base, scope) = b1_pi_broken_store_scope_env("raw-reserve-fail");
    let marker_dir = tempfile::tempdir().expect("reserve-fail marker dir");
    let spawn_marker = marker_dir.path().join("spawned");
    let provider = PiProvider::new(b1_pi_spawn_marker_fixture(
        marker_dir.path(),
        &spawn_marker,
        "0.83.0",
    ));
    let ownership =
        TrustedLaunchOwnership::mint(scope, "code_delivery provider process: pi b1 raw z");

    let result = provider
        .start_owned(
            streaming_input_for_test(None),
            Some(ownership),
            CancellationToken::new(),
        )
        .await;
    let error = match result {
        Ok(_session) => panic!("reserve 失败必须上抛,不得启动"),
        Err(error) => error,
    };
    assert!(
        !error.details.contains("not implemented"),
        "失败必须来自预留而非桩: {}",
        error.details
    );
    assert!(
        !spawn_marker.exists(),
        "预留失败 → 零 rpc spawn(marker 不得出现)"
    );
}

/// direct 面:reserve 成功后启动失败(版本门槛,零 spawn)→ 收口注销
/// (登记不残留)。
#[cfg(unix)]
#[tokio::test]
async fn pi_b1_start_owned_error_path_releases_registration() {
    let (base, scope, _tree_key) = b1_pi_scope_env("raw-error");
    let store_root = base.path().join("resource-store");
    let marker_dir = tempfile::tempdir().expect("error marker dir");
    let spawn_marker = marker_dir.path().join("spawned");
    let provider = PiProvider::new(b1_pi_spawn_marker_fixture(
        marker_dir.path(),
        &spawn_marker,
        "0.82.0",
    ));
    let scope_key = scope.key().clone();
    let ownership =
        TrustedLaunchOwnership::mint(scope, "code_delivery provider process: pi b1 raw err");

    let result = provider
        .start_owned(
            streaming_input_for_test(None),
            Some(ownership),
            CancellationToken::new(),
        )
        .await;
    assert!(result.is_err(), "版本不兼容必须拒绝启动");
    assert!(!spawn_marker.exists(), "错误路径同样零 rpc spawn");
    b1_assert_records_released(&store_root, &scope_key);
}

/// LC validated 面:载体随 validated input 传入——spawn 点回填真实
/// PID+starttime,Process 记录归属同源;Completed 不提前注销;真实终止
/// 后收口注销。
#[cfg(unix)]
#[tokio::test]
async fn pi_b1_lc_validated_launch_registers_process_and_releases_after_tail() {
    let fixture = LcLaunchFixture::new();
    let (base, scope, tree_key) = b1_pi_scope_env("lc-happy");
    let store_root = base.path().join("resource-store");
    let marker_dir = tempfile::tempdir().expect("lc marker dir");
    let pid_marker = marker_dir.path().join("pi-b1-lc.pid");
    let provider = PiProvider::new(b1_pi_rpc_fixture(marker_dir.path(), &pid_marker))
        .with_version_supplier(b1_pi_version_supplier());
    let sink = RecordingToolPolicyAuditSink::new();
    let ownership = TrustedLaunchOwnership::mint(
        Arc::clone(&scope),
        "code_delivery provider process: pi b1 lc",
    );
    let input = fixture.lc_streaming_input(
        AdapterRole::Executor,
        None,
        Some(sink.clone().bound()),
        None,
    );
    let validated = fixture
        .validated_coding_input(input)
        .with_launch_ownership(ownership.clone());

    let mut session = provider
        .start_validated(validated, CancellationToken::new())
        .await
        .expect("owned lc validated start");

    let pid = b1_wait_for_pid_marker(&pid_marker);
    // F10:direct/legacy 真实 spawn 环境含宿主 PATH/HOME。
    assert_child_inherits_host_environment(pid);
    let record = b1_poll_process_record(&store_root, scope.key());
    b1_assert_process_record_identity(&record, &ownership, pid, &tree_key);

    // 协议 Completed:登记仍在(不提前注销)。
    let completed = b1_recv_completed(&mut session.events).await;
    assert_eq!(completed, "b1 pi done");
    assert_eq!(
        b1_pi_process_records(&store_root, scope.key()).len(),
        1,
        "协议 Completed 不得提前注销"
    );

    // 真实终止:kill 子进程 → 泵终结 → 通道关闭 → 收口注销。
    b1_kill_child(pid);
    b1_drain_events_to_close(&mut session.events).await;
    b1_assert_records_released(&store_root, scope.key());
    b1_assert_process_killed_and_reaped(pid);
}

/// LC validated 面:握手失败(显式 resume 的 get_state 应答陌生 id)→
/// kill/reap 子进程后收口注销(登记不残留)。
#[cfg(unix)]
#[tokio::test]
async fn pi_b1_lc_validated_handshake_failure_releases_after_quiesce() {
    let fixture = LcLaunchFixture::new();
    let (base, scope, _tree_key) = b1_pi_scope_env("lc-handshake-fail");
    let store_root = base.path().join("resource-store");
    let marker_dir = tempfile::tempdir().expect("lc handshake marker dir");
    let pid_marker = marker_dir.path().join("pi-b1-lc-hs.pid");
    let provider = PiProvider::new(b1_pi_rpc_fixture(marker_dir.path(), &pid_marker))
        .with_version_supplier(b1_pi_version_supplier());
    let sink = RecordingToolPolicyAuditSink::new();
    let ownership = TrustedLaunchOwnership::mint(
        Arc::clone(&scope),
        "code_delivery provider process: pi b1 lc hs",
    );
    let mut input = fixture.lc_streaming_input(
        AdapterRole::Executor,
        None,
        Some(sink.clone().bound()),
        Some("pi-session-b1-resume".to_string()),
    );
    input.env_vars.insert(
        "PI_SESSION_ID".to_string(),
        "pi-session-b1-native-wrong".to_string(),
    );
    let validated = fixture
        .validated_coding_input(input)
        .with_launch_ownership(ownership);

    let result = provider
        .start_validated(validated, CancellationToken::new())
        .await;
    let error = match result {
        Ok(_session) => panic!("陌生原生 id 的 resume 必须失败"),
        Err(error) => error,
    };
    assert!(
        error.details.contains("session NOT resumed"),
        "失败必须显式记录未恢复: {error:?}"
    );
    let pid = b1_wait_for_pid_marker(&pid_marker);
    b1_assert_process_killed_and_reaped(pid);
    b1_assert_records_released(&store_root, scope.key());
}

/// LC validated 面:预留失败 → 零 spawn——start_validated 直接失败,
/// fixture 子进程从未进入 rpc 模式(pid marker 不存在)。
#[cfg(unix)]
#[tokio::test]
async fn pi_b1_lc_validated_reserve_failure_means_zero_spawn() {
    let fixture = LcLaunchFixture::new();
    let (_base, scope) = b1_pi_broken_store_scope_env("lc-reserve-fail");
    let marker_dir = tempfile::tempdir().expect("lc reserve-fail marker dir");
    let pid_marker = marker_dir.path().join("pi-b1-lc-z.pid");
    let provider = PiProvider::new(b1_pi_rpc_fixture(marker_dir.path(), &pid_marker))
        .with_version_supplier(b1_pi_version_supplier());
    let sink = RecordingToolPolicyAuditSink::new();
    let ownership =
        TrustedLaunchOwnership::mint(scope, "code_delivery provider process: pi b1 lc z");
    let input = fixture.lc_streaming_input(
        AdapterRole::Executor,
        None,
        Some(sink.clone().bound()),
        None,
    );
    let validated = fixture
        .validated_coding_input(input)
        .with_launch_ownership(ownership);

    let result = provider
        .start_validated(validated, CancellationToken::new())
        .await;
    let error = match result {
        Ok(_session) => panic!("reserve 失败必须上抛,不得启动"),
        Err(error) => error,
    };
    assert!(
        !error.details.contains("not supported"),
        "失败必须来自预留而非 unsupported 桩: {}",
        error.details
    );
    assert!(
        !pid_marker.exists(),
        "预留失败 → 零 rpc spawn(pid marker 不得出现)"
    );
}

#[cfg(unix)]
fn b1_pi_version_supplier() -> ProviderVersionSupplier {
    std::sync::Arc::new(|| Ok("pi 0.83.0-b1-fixture".to_string()))
}
