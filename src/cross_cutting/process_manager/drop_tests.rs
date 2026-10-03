use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

use super::ProcessManager;

struct ProcessTreeFixture {
    command: PathBuf,
    parent_pid: PathBuf,
    grandchild_pid: PathBuf,
    late_marker: PathBuf,
}

fn process_tree_fixture(root: &Path) -> ProcessTreeFixture {
    let command = root.join("fake-ssh");
    let parent_pid = root.join("parent.pid");
    let grandchild_pid = root.join("grandchild.pid");
    let late_marker = root.join("late-marker");
    fs::write(
        &command,
        format!(
            "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nsh -c 'printf \"%s\" \"$$\" > \"$1\"; sleep 1; printf late > \"$2\"' fake-ssh-child '{}' '{}' &\nwait\n",
            parent_pid.display(),
            grandchild_pid.display(),
            late_marker.display(),
        ),
    )
    .expect("write fake ssh");
    fs::set_permissions(&command, fs::Permissions::from_mode(0o755)).expect("chmod fake ssh");
    ProcessTreeFixture {
        command,
        parent_pid,
        grandchild_pid,
        late_marker,
    }
}

async fn wait_for_path(path: &Path) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("process fixture path");
}

fn read_pid(path: &Path) -> i32 {
    fs::read_to_string(path)
        .expect("read pid")
        .trim()
        .parse()
        .expect("parse pid")
}

fn process_state(pid: i32) -> Option<char> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(") ")
        .and_then(|(_, tail)| tail.chars().next())
}

async fn process_disappeared(pid: i32) -> bool {
    tokio::time::timeout(Duration::from_millis(500), async {
        while process_state(pid).is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok()
}

async fn assert_process_tree_stopped(fixture: &ProcessTreeFixture) {
    let parent_pid = read_pid(&fixture.parent_pid);
    let grandchild_pid = read_pid(&fixture.grandchild_pid);
    let parent_gone = process_disappeared(parent_pid).await;
    let grandchild_gone = process_disappeared(grandchild_pid).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let late_marker = fixture.late_marker.exists();
    if !parent_gone || !grandchild_gone {
        unsafe {
            let _ = libc::killpg(parent_pid, libc::SIGKILL);
            let _ = libc::kill(parent_pid, libc::SIGKILL);
            let _ = libc::kill(grandchild_pid, libc::SIGKILL);
        }
    }
    assert!(
        parent_gone,
        "dropped process leader remained alive or zombie"
    );
    assert!(
        grandchild_gone,
        "dropped process grandchild remained alive or zombie"
    );
    assert!(!late_marker, "dropped process tree wrote a late marker");
}

async fn spawn_and_wait(fixture: &ProcessTreeFixture) {
    let command = fixture.command.to_string_lossy().to_string();
    let mut process = ProcessManager::spawn(
        &command,
        &[],
        fixture.command.parent().expect("fixture root"),
        &BTreeMap::new(),
        CancellationToken::new(),
    )
    .await
    .expect("spawn process tree");
    process.child.wait().await.expect("wait process tree");
}

#[test]
fn process_group_signal_target_rejects_reaped_or_reused_leader() {
    assert_eq!(
        super::unix_process_group_signal_target(Some(41), 41),
        Some(41)
    );
    assert_eq!(super::unix_process_group_signal_target(None, 41), None);
    assert_eq!(super::unix_process_group_signal_target(Some(42), 41), None);
}

#[tokio::test]
async fn unix_managed_process_wait_uses_tokio_cancel_safe_child() {
    let root = tempdir().expect("tempdir");
    let fixture = process_tree_fixture(root.path());
    let command = fixture.command.to_string_lossy().to_string();
    let mut process = ProcessManager::spawn(
        &command,
        &[],
        root.path(),
        &BTreeMap::new(),
        CancellationToken::new(),
    )
    .await
    .expect("spawn process tree");

    fn assert_tokio_child(_: &tokio::process::Child) {}
    assert_tokio_child(process.child.inner());
}

#[tokio::test]
async fn successful_wait_does_not_start_drop_reaper() {
    let root = tempdir().expect("tempdir");
    let fixture = process_tree_fixture(root.path());
    let command = fixture.command.to_string_lossy().to_string();
    let mut process = ProcessManager::spawn(
        &command,
        &[],
        root.path(),
        &BTreeMap::new(),
        CancellationToken::new(),
    )
    .await
    .expect("spawn process tree");

    let reaper_spawns = std::sync::Arc::clone(&process.child.drop_reaper_spawns);
    process.child.wait().await.expect("wait process tree");
    drop(process);

    assert!(
        reaper_spawns.load(std::sync::atomic::Ordering::Relaxed) == 0,
        "a successfully waited child must not be handed to the drop reaper"
    );
}

#[tokio::test]
async fn cancelled_wait_can_still_terminate_original_group() {
    let root = tempdir().expect("tempdir");
    let fixture = process_tree_fixture(root.path());
    let command = fixture.command.to_string_lossy().to_string();
    let mut process = ProcessManager::spawn(
        &command,
        &[],
        root.path(),
        &BTreeMap::new(),
        CancellationToken::new(),
    )
    .await
    .expect("spawn process tree");
    wait_for_path(&fixture.grandchild_pid).await;

    assert!(
        tokio::time::timeout(Duration::from_millis(50), process.child.wait())
            .await
            .is_err(),
        "process wait must still be pending while the process tree is alive"
    );
    tokio::time::timeout(Duration::from_millis(500), process.child.terminate())
        .await
        .expect("termination after cancelled wait must remain bounded");

    assert_process_tree_stopped(&fixture).await;
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_managed_process_child_is_non_blocking() {
    let root = tempdir().expect("tempdir");
    let fixture = process_tree_fixture(root.path());
    let command = fixture.command.to_string_lossy().to_string();
    let process = ProcessManager::spawn(
        &command,
        &[],
        root.path(),
        &BTreeMap::new(),
        CancellationToken::new(),
    )
    .await
    .expect("spawn process tree");
    wait_for_path(&fixture.grandchild_pid).await;

    let started = Instant::now();
    drop(process);
    assert!(
        started.elapsed() < Duration::from_millis(25),
        "Drop blocked the current-thread runtime for {:?}",
        started.elapsed()
    );
    assert_process_tree_stopped(&fixture).await;
}

#[tokio::test]
async fn task_abort_drops_and_kills_managed_process_group() {
    let root = tempdir().expect("tempdir");
    let fixture = process_tree_fixture(root.path());
    let task = tokio::spawn({
        let fixture = ProcessTreeFixture {
            command: fixture.command.clone(),
            parent_pid: fixture.parent_pid.clone(),
            grandchild_pid: fixture.grandchild_pid.clone(),
            late_marker: fixture.late_marker.clone(),
        };
        async move { spawn_and_wait(&fixture).await }
    });
    wait_for_path(&fixture.parent_pid).await;
    wait_for_path(&fixture.grandchild_pid).await;

    task.abort();
    assert!(task.await.expect_err("task abort").is_cancelled());

    assert_process_tree_stopped(&fixture).await;
}

#[tokio::test]
async fn task_panic_drops_and_kills_managed_process_group() {
    let root = tempdir().expect("tempdir");
    let fixture = process_tree_fixture(root.path());
    let task = tokio::spawn({
        let command = fixture.command.clone();
        let parent_pid = fixture.parent_pid.clone();
        let grandchild_pid = fixture.grandchild_pid.clone();
        async move {
            let command_text = command.to_string_lossy().to_string();
            let _process = ProcessManager::spawn(
                &command_text,
                &[],
                command.parent().expect("fixture root"),
                &BTreeMap::new(),
                CancellationToken::new(),
            )
            .await
            .expect("spawn process tree");
            wait_for_path(&parent_pid).await;
            wait_for_path(&grandchild_pid).await;
            panic!("intentional managed process panic");
        }
    });

    assert!(task.await.expect_err("task panic").is_panic());
    assert_process_tree_stopped(&fixture).await;
}

#[test]
fn current_thread_runtime_drop_kills_managed_process_group() {
    let root = tempdir().expect("tempdir");
    let fixture = process_tree_fixture(root.path());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let command = fixture.command.clone();
        tokio::spawn(async move {
            let fixture = ProcessTreeFixture {
                command: command.clone(),
                parent_pid: command.parent().expect("fixture root").join("parent.pid"),
                grandchild_pid: command
                    .parent()
                    .expect("fixture root")
                    .join("grandchild.pid"),
                late_marker: command.parent().expect("fixture root").join("late-marker"),
            };
            spawn_and_wait(&fixture).await;
        });
        wait_for_path(&fixture.parent_pid).await;
        wait_for_path(&fixture.grandchild_pid).await;
    });
    drop(runtime);

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("verification runtime")
        .block_on(assert_process_tree_stopped(&fixture));
}

/// Task 6a:缺 bwrap/user namespace 的机器,写边界能力状态必须明确 Unknown
/// (非 skip→pass),spawn_with_boundary 失败关闭;namespace/mount 实际失败
/// 也不得回退无隔离 plain spawn(回退会让子进程成功写宿主文件)。
#[tokio::test]
async fn lcg_t06_missing_sandbox_or_namespace_keeps_unknown() {
    use crate::cross_cutting::provider_boundary::{
        ProviderBoundaryLauncher, ProviderBoundaryMode, ProviderBoundaryPlan,
    };
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;

    let base = tempdir().expect("base dir");
    let root = base.path().join("lc-root");
    fs::create_dir_all(&root).expect("lc root");
    let aria = root.join(".aria");
    fs::create_dir_all(&aria).expect("aria metadata");
    let plan = ProviderBoundaryPlan::new(
        ProviderBoundaryMode::ReadOnly,
        root.clone(),
        None,
        Vec::new(),
    );
    let mut env = BTreeMap::new();
    env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());

    // 缺 bwrap/namespace:状态明确 Unknown,spawn 失败关闭并保留判别码。
    let unavailable = ProviderBoundaryLauncher::from_bwrap(None);
    let unavailable_boundary_state = unavailable.write_boundary_state();
    assert_eq!(
        unavailable_boundary_state,
        ProviderCapabilityEvidence::Unknown
    );
    let refused = ProcessManager::spawn_with_boundary_resolved(
        unavailable,
        "sh",
        &["-c", "printf rogue > /dev/null"],
        &root,
        &env,
        &plan,
        CancellationToken::new(),
    )
    .await;
    let refusal = refused.expect_err("missing sandbox must fail closed instead of spawning");
    assert!(
        refusal.details.contains("provider_boundary_unsupported"),
        "unexpected failure details: {}",
        refusal.details
    );

    // namespace/mount 实际起子进程后失败(fake bwrap 退出 1):若实现回退
    // plain spawn,sh 会成功执行并写入宿主 .aria,这里必须保持不可写。
    let fake_bwrap = base.path().join("fake-bwrap");
    fs::write(
        &fake_bwrap,
        "#!/bin/sh\necho 'bwrap: Creating new namespace failed: Operation not permitted' >&2\nexit 1\n",
    )
    .expect("write fake bwrap");
    fs::set_permissions(&fake_bwrap, fs::Permissions::from_mode(0o755)).expect("chmod fake bwrap");
    let fake = ProviderBoundaryLauncher::from_bwrap(Some(fake_bwrap));
    let mut process = ProcessManager::spawn_with_boundary_resolved(
        fake,
        "sh",
        &["-c", "printf rogue > '.aria/rogue-host-write'"],
        &root,
        &env,
        &plan,
        CancellationToken::new(),
    )
    .await
    .expect("fake bwrap still yields a managed child");
    drop(process.stdin);
    let status = process.child.wait().await.expect("wait fake bwrap child");
    assert!(
        !status.success(),
        "fake bwrap must fail loudly, not exit 0 like a plain spawn"
    );
    assert!(
        !aria.join("rogue-host-write").exists(),
        "no plain-spawn fallback may write host metadata"
    );

    // 真实机器:未经 6c 真实正负探针前同样保持 Unknown(不自报 Confirmed)。
    let real = ProviderBoundaryLauncher::probe_environment();
    assert_eq!(
        real.write_boundary_state(),
        ProviderCapabilityEvidence::Unknown
    );
}

/// Task 6a:产品写边界内的 provider 进程树在 ManagedProcess drop 时必须被
/// 整组回收(killpg + --die-with-parent),后代不得逃逸写 late marker。
#[tokio::test]
async fn lcg_t06_boundary_child_tree_is_killed_on_drop() {
    use crate::cross_cutting::provider_boundary::{
        ProviderBoundaryLauncher, ProviderBoundaryMode, ProviderBoundaryPlan,
    };

    let base = tempdir().expect("base dir");
    let root = base.path().join("lc-root");
    fs::create_dir_all(&root).expect("lc root");
    let target = base.path().join("target");
    fs::create_dir_all(&target).expect("target worktree");
    let fixture = process_tree_fixture(&target);
    let plan = ProviderBoundaryPlan::new(
        ProviderBoundaryMode::TargetWriteOnly,
        root.clone(),
        Some(target.clone()),
        Vec::new(),
    );
    let launcher = ProviderBoundaryLauncher::probe_environment();
    assert!(
        launcher.is_available(),
        "environment blocked: mandatory boundary child-tree case needs bwrap + user namespace"
    );
    let mut env = BTreeMap::new();
    env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
    let command = fixture.command.to_string_lossy().into_owned();

    let process = ProcessManager::spawn_with_boundary_resolved(
        launcher,
        &command,
        &[],
        &root,
        &env,
        &plan,
        CancellationToken::new(),
    )
    .await
    .expect("spawn boundary process tree");
    wait_for_path(&fixture.parent_pid).await;
    wait_for_path(&fixture.grandchild_pid).await;

    drop(process);
    assert_process_tree_stopped(&fixture).await;
}
