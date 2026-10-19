//! B1-1(波 2):Claude adapter 的受信 launch 归属接线测试。
//!
//! 两条真实 spawn 面(raw `start_owned` 与 LC `start_validated`)都在
//! spawn 点 consume 同一受信载体:预留(durable,失败零 spawn)→ spawn →
//! 回填真实 PID+`/proc/<pid>/stat` starttime → 会话真实收口后注销
//! (协议 `Completed` 不提前注销;错误/取消收口后注销)。
//!
//! 归属 scope 以 engine 真实键型(`scope_id`/`run_id`/boot/`aria.worktree`)
//! 打开;断言 adapter 的 Process 登记与 scope 同源,不新造 scope。
#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_resources::{
    OwnedResourceKind, ProviderResourceRecord, ProviderRunResourceScope, ResourceScopeKey,
    StartedIdentity, TrustedLaunchOwnership,
};
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderToolPolicy, StreamingProviderAdapter,
};
use crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink;
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
use crate::product::provider_resource_store::ProviderResourceStore;
use crate::protocol::contracts::AdapterRole;

use super::*;

// ---------- fixtures ----------

struct B1Fixture {
    _base: tempfile::TempDir,
    store_root: PathBuf,
    private_root: PathBuf,
}

fn b1_fixture() -> B1Fixture {
    let base = tempfile::tempdir().expect("b1 fixture base");
    let store_root = base.path().join("resource-store");
    let private_root = base.path().join("private-root");
    std::fs::create_dir_all(&private_root).expect("create private root");
    B1Fixture {
        _base: base,
        store_root,
        private_root,
    }
}

/// 以 engine 真实键型开 scope(coder delivery 同款:真实 scope/run/boot +
/// canonical worktree → tree_key 标签),不新造 scope。
fn b1_scope(
    fixture: &B1Fixture,
    scope_id: &str,
    run_id: &str,
) -> (ProviderRunResourceScope, String) {
    let worktree = fixture
        .private_root
        .canonicalize()
        .expect("canonical worktree");
    let tree_key = crate::product::reviewer_window_store::tree_key_of(&worktree);
    let key = ResourceScopeKey {
        scope_id: scope_id.to_string(),
        run_id: run_id.to_string(),
        boot_identity: format!("aria-engine-{}", std::process::id()),
    };
    let scope = ProviderRunResourceScope::open_for_recovery(
        &key,
        None,
        &fixture.private_root,
        &fixture.store_root,
        None,
    )
    .expect("open b1 scope")
    .with_worktree_key(&tree_key);
    (scope, tree_key)
}

fn b1_process_records(fixture: &B1Fixture, key: &ResourceScopeKey) -> Vec<ProviderResourceRecord> {
    ProviderResourceStore::new(&fixture.store_root)
        .list_scope(key)
        .expect("list scope")
        .into_iter()
        .filter(|record| matches!(record.kind, OwnedResourceKind::Process))
        .collect()
}

async fn b1_wait_records(
    fixture: &B1Fixture,
    key: &ResourceScopeKey,
    predicate: impl Fn(&[ProviderResourceRecord]) -> bool + Send,
) -> Vec<ProviderResourceRecord> {
    for _ in 0..poll_attempts_for_test_timeout() {
        let records = b1_process_records(fixture, key);
        if predicate(&records) {
            return records;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    b1_process_records(fixture, key)
}

#[cfg(target_os = "linux")]
fn proc_start_time_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit_once(')')?.1;
    after_comm.split_whitespace().nth(19)?.parse().ok()
}

/// 不存在的目标路径(临时目录下的子路径):由 fixture/测试显式创建,
/// 避免 `NamedTempFile` 预创建空文件把「等待创建」的语义抹掉。
fn b1_spawn_path(name: &str) -> PathBuf {
    tempfile::tempdir().expect("b1 path dir").keep().join(name)
}

fn b1_pid_path() -> PathBuf {
    b1_spawn_path("fixture.pid")
}

fn b1_hold_path() -> PathBuf {
    b1_spawn_path("fixture.hold")
}

/// raw/non-policy fixture:登记真实 PID 后等 hold 文件再吐 result 退出
/// (断言期间子进程保持存活,starttime 可读)。路径直接写进脚本,避免
/// 环境变量传递面。
fn b1_raw_fixture(pid_path: &std::path::Path, hold_path: &std::path::Path) -> PathBuf {
    write_fixture(
        "claude_b1_raw_fixture.sh",
        &format!(
            "#!/usr/bin/env bash\nset -u\nif [[ \"${{1:-}}\" == \"--version\" ]]; then echo 'claude 2.1.119'; exit 0; fi\nprintf '%s\\n' \"$$\" > '{pid}'\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"user\"'* ]]; then\n    while [[ ! -f '{hold}' ]]; do sleep 0.01; done\n    echo '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"b1 raw done\",\"session_id\":\"sess-b1-raw\"}}'\n    exit 0\n  fi\ndone\n",
            pid = pid_path.display(),
            hold = hold_path.display()
        ),
    )
}

/// LC validated fixture:init 握手后等 hold 文件再吐 result(登记断言与
/// 会话真实收口解耦)。
fn b1_lc_fixture(pid_path: &std::path::Path, hold_path: &std::path::Path) -> PathBuf {
    write_fixture(
        "claude_b1_lc_fixture.sh",
        &format!(
            "#!/usr/bin/env bash\nset -u\nif [[ \"${{1:-}}\" == \"--version\" ]]; then echo 'claude 2.0.4-lc-fixture'; exit 0; fi\nprintf '%s\\n' \"$$\" > '{pid}'\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"user\"'* ]]; then\n    echo '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"sess-b1-lc\"}}'\n    while [[ ! -f '{hold}' ]]; do sleep 0.01; done\n    echo '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"b1 lc done\",\"session_id\":\"sess-b1-lc\"}}'\n    exit 0\n  fi\ndone\n",
            pid = pid_path.display(),
            hold = hold_path.display()
        ),
    )
}

/// 策略会话握手失败 fixture:收到 user 后写 stderr 即退出,永不吐 init。
fn b1_handshake_fail_fixture() -> PathBuf {
    write_fixture(
        "claude_b1_handshake_fail_fixture.sh",
        "#!/usr/bin/env bash\nset -u\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"user\"'* ]]; then\n    echo 'b1 handshake evidence' >&2\n    exit 7\n  fi\ndone\n",
    )
}

/// 输出文本后挂起 fixture:取消后由会话尾部 kill+wait 收口。
fn b1_hanging_after_output_fixture(pid_path: &std::path::Path) -> PathBuf {
    write_fixture(
        "claude_b1_hanging_fixture.sh",
        &format!(
            "#!/usr/bin/env bash\nset -u\nprintf '%s\\n' \"$$\" > '{pid}'\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"user\"'* ]]; then\n    echo '{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"started\"}}]}},\"session_id\":\"claude_fixture_session\"}}'\n    sleep 30\n  fi\ndone\n",
            pid = pid_path.display()
        ),
    )
}

/// Completed 后子进程组外的 stderr 持有者:sleep 不在本组(setsid),killpg
/// 不能终止它,stderr join 因此有界延迟——协议 Completed 之后、会话真实
/// 收口之前有确定窗口可断言登记仍在。
fn b1_completed_hold_fixture(holder_path: &std::path::Path) -> PathBuf {
    write_fixture(
        "claude_b1_completed_hold_fixture.sh",
        &format!(
            "#!/usr/bin/env bash\nset -u\nif [[ \"${{1:-}}\" == \"--version\" ]]; then echo 'claude 2.1.119'; exit 0; fi\nSETSID_BIN=\"$(command -v setsid || echo /usr/sbin/setsid)\"\n\"$SETSID_BIN\" sh -c 'printf \"%s\\n\" \"$$\" > \"$1\"; exec sleep 10' sh '{holder}' &\nwhile [[ ! -f '{holder}' ]]; do sleep 0.01; done\nwhile IFS= read -r line; do\n  if [[ \"$line\" == *'\"user\"'* ]]; then\n    echo '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"b1 completed\",\"session_id\":\"sess-b1-completed\"}}'\n    sleep 30\n  fi\ndone\n",
            holder = holder_path.display()
        ),
    )
}

fn b1_raw_input() -> StreamingProviderInput {
    streaming_input(
        crate::protocol::contracts::ProviderType::ClaudeCode,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
    )
}

fn b1_policy_input(sink: Arc<dyn ToolPolicyAuditSink>) -> StreamingProviderInput {
    let mut input = streaming_input(
        crate::protocol::contracts::ProviderType::ClaudeCode,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
    );
    input.role = AdapterRole::Orchestrator;
    input.tool_policy = Some(ProviderToolPolicy::deny_file_write_builtins());
    input.audit_sink = Some(sink);
    input
}

fn b1_version_supplier() -> crate::cross_cutting::streaming_provider::ProviderVersionSupplier {
    Arc::new(|| Ok("claude 1.0.99-b1-fixture".to_string()))
}

async fn b1_drain(events: &mut tokio::sync::mpsc::Receiver<ProviderEvent>) {
    while tokio::time::timeout(TEST_TIMEOUT, events.recv())
        .await
        .expect("session events must terminate (终结保证契约)")
        .is_some()
    {}
}

// ---------- tests ----------

/// raw direct 面(非 LC):`start_owned` 的载体在 spawn 点回填真实
/// PID+starttime,Process 记录归属与 engine scope 完全同源;会话真实收口
/// 后注销。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn claude_owned_raw_start_backfills_real_identity_and_deregisters_after_tail() {
    let fixture = b1_fixture();
    let (scope, tree_key) = b1_scope(&fixture, "code-delivery-b1-raw", "role-run-b1-raw");
    let ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "code_delivery provider process");
    let pid_path = b1_pid_path();
    let hold_path = b1_hold_path();
    let provider = ClaudeCodeProvider::new(b1_raw_fixture(&pid_path, &hold_path));

    let mut session = provider
        .start_owned(
            b1_raw_input(),
            Some(ownership.clone()),
            CancellationToken::new(),
        )
        .await
        .expect("owned raw start");

    // 1) Process 登记在 spawn 后立即出现,归属与引擎 scope 同源。
    let records = b1_wait_records(&fixture, ownership.scope_key(), |records| {
        records.len() == 1
    })
    .await;
    assert_eq!(records.len(), 1, "恰一条 Process 登记: {records:?}");
    let record = &records[0];
    assert_eq!(record.key, *ownership.scope_key(), "同源 scope key");
    assert_eq!(record.resource_id, ownership.resource_id());
    assert!(
        record.resource_id.starts_with("provider-process-"),
        "每次 launch 唯一资源 ID: {}",
        record.resource_id
    );
    assert_eq!(
        record
            .reserved_labels
            .get("aria.scope_id")
            .map(String::as_str),
        Some("code-delivery-b1-raw")
    );
    assert_eq!(
        record
            .reserved_labels
            .get("aria.run_id")
            .map(String::as_str),
        Some("role-run-b1-raw")
    );
    assert_eq!(
        record
            .reserved_labels
            .get("aria.worktree")
            .map(String::as_str),
        Some(tree_key.as_str())
    );

    // 2) 真实 PID+starttime:与 fixture 进程自身及 /proc 观察逐字相等。
    wait_for_file(&pid_path).await;
    let fixture_pid = std::fs::read_to_string(&pid_path)
        .expect("read fixture pid")
        .trim()
        .parse::<u32>()
        .expect("fixture pid");
    match record.started_identity.as_ref() {
        Some(StartedIdentity::Process {
            pid,
            start_time_ticks,
        }) => {
            assert_eq!(*pid, fixture_pid, "回填的是真实子进程 PID");
            let observed = proc_start_time_ticks(fixture_pid).expect("observe starttime");
            assert_eq!(*start_time_ticks, observed, "回填 /proc starttime");
            assert_ne!(*start_time_ticks, 0);
        }
        other => panic!("spawn 点必须回填真实身份,实际 {other:?}"),
    }

    // 3) 会话真实收口(子进程退出)后注销。
    std::fs::write(&hold_path, "").expect("release fixture");
    b1_drain(&mut session.events).await;
    let records = b1_wait_records(&fixture, ownership.scope_key(), |records| {
        records.is_empty()
    })
    .await;
    assert!(records.is_empty(), "真实收口后注销: {records:?}");
}

/// LC validated 面:载体绑定进 validated input,`start_validated` 在 spawn
/// 点回填同一真实身份;会话收口后注销。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn claude_validated_start_backfills_real_identity_from_bound_carrier() {
    let launch_fixture = LcLaunchFixture::new();
    let fixture = b1_fixture();
    let (scope, tree_key) = b1_scope(&fixture, "plan-review-b1-lc", "role-run-b1-lc");
    let ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "plan-review provider process");
    let pid_path = b1_pid_path();
    let hold_path = b1_hold_path();
    let provider = ClaudeCodeProvider::new(b1_lc_fixture(&pid_path, &hold_path));
    let sink = RecordingToolPolicyAuditSink::new();

    let raw = launch_fixture.lc_streaming_input(
        AdapterRole::Executor,
        None,
        Some(sink.clone().bound()),
        None,
    );
    let validated = launch_fixture
        .validated_coding_input(raw)
        .with_launch_ownership(ownership.clone());

    let mut session = provider
        .start_validated(validated, CancellationToken::new())
        .await
        .expect("validated owned start");
    assert_eq!(session.native_session_id.as_deref(), Some("sess-b1-lc"));

    // 1) 同源登记 + 真实身份回填。
    let records = b1_wait_records(&fixture, ownership.scope_key(), |records| {
        records.len() == 1
    })
    .await;
    assert_eq!(records.len(), 1, "恰一条 Process 登记: {records:?}");
    let record = &records[0];
    assert_eq!(record.key, *ownership.scope_key());
    assert_eq!(
        record
            .reserved_labels
            .get("aria.worktree")
            .map(String::as_str),
        Some(tree_key.as_str())
    );
    wait_for_file(&pid_path).await;
    let fixture_pid = std::fs::read_to_string(&pid_path)
        .expect("read fixture pid")
        .trim()
        .parse::<u32>()
        .expect("fixture pid");
    match record.started_identity.as_ref() {
        Some(StartedIdentity::Process {
            pid,
            start_time_ticks,
        }) => {
            assert_eq!(*pid, fixture_pid);
            assert_eq!(
                *start_time_ticks,
                proc_start_time_ticks(fixture_pid).expect("observe starttime")
            );
        }
        other => panic!("validated spawn 点必须回填真实身份,实际 {other:?}"),
    }

    // 2) 收口后注销。
    std::fs::write(&hold_path, "").expect("release fixture");
    assert_eq!(recv_completed(&mut session.events).await, "b1 lc done");
    b1_drain(&mut session.events).await;
    let records = b1_wait_records(&fixture, ownership.scope_key(), |records| {
        records.is_empty()
    })
    .await;
    assert!(records.is_empty(), "LC 会话收口后注销: {records:?}");
}

/// 无归属请求(`start` 直调 / `start_owned(None)`)不产生任何登记——
/// 不得裸登记,也不得把无归属面伪造为有归属。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn claude_start_without_carrier_registers_nothing() {
    let fixture = b1_fixture();
    let (scope, _tree_key) = b1_scope(&fixture, "code-delivery-b1-none", "role-run-b1-none");
    let key = scope.key().clone();
    // scope 只用于给出枚举键;无归属请求不消费它。
    let _ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "unused");
    let pid_path = b1_pid_path();
    let hold_path = b1_hold_path();
    let provider = ClaudeCodeProvider::new(b1_raw_fixture(&pid_path, &hold_path));

    let mut session = provider
        .start(b1_raw_input(), CancellationToken::new())
        .await
        .expect("bare direct start");
    assert!(
        b1_process_records(&fixture, &key).is_empty(),
        "无归属零登记"
    );
    std::fs::write(&hold_path, "").expect("release fixture");
    b1_drain(&mut session.events).await;
    assert!(b1_process_records(&fixture, &key).is_empty());
}

/// 预留失败 → 零 spawn:登记落盘失败时 adapter 的 spawn 从未发生。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn claude_owned_start_reserve_failure_means_zero_spawn() {
    let base = tempfile::tempdir().expect("b1 reserve-fail base");
    let private_root = base.path().join("private-root");
    std::fs::create_dir_all(&private_root).expect("create private root");
    // store 根是普通文件 → reserve 落盘必败。
    let store_root = base.path().join("store-is-a-file");
    std::fs::write(&store_root, "not a directory").expect("write store blocker");
    let scope = ProviderRunResourceScope::open_for_recovery(
        &ResourceScopeKey {
            scope_id: "code-delivery-b1-reserve-fail".to_string(),
            run_id: "role-run-b1-reserve-fail".to_string(),
            boot_identity: format!("aria-engine-{}", std::process::id()),
        },
        None,
        &private_root,
        &store_root,
        None,
    )
    .expect("open scope with blocked store");
    let ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "code_delivery provider process");

    let marker = base.path().join("spawned-marker");
    let provider = ClaudeCodeProvider::new(write_fixture(
        "claude_b1_spawn_marker_fixture.sh",
        &format!(
            "#!/usr/bin/env bash\nset -u\nprintf 'spawned\\n' >> '{}'\nexit 0\n",
            marker.display()
        ),
    ));
    let input = streaming_input(
        crate::protocol::contracts::ProviderType::ClaudeCode,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
    );

    let error = match provider
        .start_owned(input, Some(ownership), CancellationToken::new())
        .await
    {
        Ok(_) => panic!("预留失败必须上抛,不得启动"),
        Err(error) => error,
    };
    assert!(
        error.details.contains("provider_resources_store"),
        "预留失败原因须可诊断: {}",
        error.details
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!marker.exists(), "预留失败零 spawn(adapter 从未执行)");
    assert!(!store_root.is_dir(), "store 根从未被初始化为目录");
}

/// 策略会话握手失败:错误链 kill+wait 子进程后,按登记身份收口并注销。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn claude_handshake_failure_releases_registration_after_tail() {
    let fixture = b1_fixture();
    let (scope, _tree_key) = b1_scope(&fixture, "code-delivery-b1-fail", "role-run-b1-fail");
    let ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "code_delivery provider process");
    let key = ownership.scope_key().clone();
    let sink = RecordingToolPolicyAuditSink::new();
    let provider = ClaudeCodeProvider::new(b1_handshake_fail_fixture())
        .with_version_supplier(b1_version_supplier());

    let error = match provider
        .start_owned(
            b1_policy_input(sink.clone().bound()),
            Some(ownership),
            CancellationToken::new(),
        )
        .await
    {
        Ok(_) => panic!("握手失败必须上抛"),
        Err(error) => error,
    };
    assert!(
        error.details.contains("handshake") || error.details.contains("stream ended before init"),
        "握手失败原因: {}",
        error.details
    );
    // 错误路径返回前已完成收口注销(不留 durable 记录)。
    let records = b1_wait_records(&fixture, &key, |records| records.is_empty()).await;
    assert!(records.is_empty(), "握手失败收口后注销: {records:?}");
}

/// 会话尾部取消:Aborted 收口(kill+wait)后注销登记。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn claude_session_tail_cancel_releases_registration() {
    let fixture = b1_fixture();
    let (scope, _tree_key) = b1_scope(&fixture, "code-delivery-b1-cancel", "role-run-b1-cancel");
    let ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "code_delivery provider process");
    let key = ownership.scope_key().clone();
    let pid_path = b1_pid_path();
    let provider = ClaudeCodeProvider::new(b1_hanging_after_output_fixture(&pid_path));
    let cancel = CancellationToken::new();
    let input = streaming_input(
        crate::protocol::contracts::ProviderType::ClaudeCode,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
    );

    let mut session = provider
        .start_owned(input, Some(ownership), cancel.clone())
        .await
        .expect("owned hanging start");
    let records = b1_wait_records(&fixture, &key, |records| records.len() == 1).await;
    assert_eq!(records.len(), 1, "运行中登记在位: {records:?}");
    loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider startup event")
            .expect("session channel open until cancellation")
        {
            ProviderEvent::TextDelta { content } if content == "started" => break,
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            _ => {}
        }
    }
    cancel.cancel();
    b1_drain(&mut session.events).await;
    let records = b1_wait_records(&fixture, &key, |records| records.is_empty()).await;
    assert!(records.is_empty(), "取消收口后注销: {records:?}");
    wait_for_file(&pid_path).await;
    let pid = std::fs::read_to_string(&pid_path)
        .expect("read fixture pid")
        .trim()
        .parse::<u32>()
        .expect("fixture pid");
    wait_for_process_absent(pid).await;
}

/// 协议 `Completed` 不提前注销:Completed 事件之后、会话真实收口之前登记
/// 仍在;stderr 持有者退出(收口可证)后才注销。
#[cfg(target_os = "linux")]
#[tokio::test]
async fn claude_completed_event_does_not_deregister_early() {
    let fixture = b1_fixture();
    let (scope, _tree_key) = b1_scope(
        &fixture,
        "code-delivery-b1-completed",
        "role-run-b1-completed",
    );
    let ownership = TrustedLaunchOwnership::mint(Arc::new(scope), "code_delivery provider process");
    let key = ownership.scope_key().clone();
    let holder_path = b1_pid_path();
    let provider = ClaudeCodeProvider::new(b1_completed_hold_fixture(&holder_path));
    let input = streaming_input(
        crate::protocol::contracts::ProviderType::ClaudeCode,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
    );

    let mut session = provider
        .start_owned(input, Some(ownership), CancellationToken::new())
        .await
        .expect("owned completed start");
    wait_for_file(&holder_path).await;
    let holder_pid = std::fs::read_to_string(&holder_path)
        .expect("read holder pid")
        .trim()
        .parse::<u32>()
        .expect("holder pid");

    // 1) 收到协议 Completed。
    let mut saw_completed = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .expect("completion event")
    {
        if let ProviderEvent::Completed(_) = event {
            saw_completed = true;
            break;
        }
    }
    assert!(saw_completed, "fixture 必须先发 Completed");
    // 2) Completed 之后、会话真实收口之前:登记仍在(组外 stderr 持有者
    //    使收口可证窗口确定存在)。
    let records = b1_process_records(&fixture, &key);
    assert_eq!(records.len(), 1, "协议 Completed 不提前注销: {records:?}");
    // 3) 持有者退出 → 收口可证 → 注销。
    unsafe {
        libc::kill(holder_pid as i32, libc::SIGKILL);
    }
    b1_drain(&mut session.events).await;
    let records = b1_wait_records(&fixture, &key, |records| records.is_empty()).await;
    assert!(records.is_empty(), "真实收口后注销: {records:?}");
}
