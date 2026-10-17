use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

use super::ProcessManager;

/// 批2 Task 3:spawn 不再隐式继承宿主环境——进程树 fixture 内部以裸名
/// 调用 sh/sleep,测试须显式携带 PATH(与 boundary 用例同款注入)。
fn unix_path_env() -> BTreeMap<String, String> {
    BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())])
}

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
        &unix_path_env(),
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
        &unix_path_env(),
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
        &unix_path_env(),
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
        &unix_path_env(),
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
        &unix_path_env(),
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
                &unix_path_env(),
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

/// 批2 Task 10(旧物理 launcher 退役):provider 进程统一受信 Host 启动
///(`ProcessManager::spawn`,env_clear 白名单环境)。旧 bwrap 写边界的
/// target-only/readonly/mount 物理拒写断言随 `spawn_with_boundary[_resolved]`
/// 与 `ProviderBoundaryLauncher` 退役——逻辑交付边界(gateway 双阶段
/// validate/spawn 复验同源拒绝 + write_boundary 分格仅当前语义 Confirmed
/// 放行)与当前证据守卫由 `production_policy_resolvers`/`provider_gateway`
/// 测试面持有,不在此重证。Host 启动的进程组回收语义保持:
/// ManagedProcess drop 必须整组 kill,后代不得逃逸写 late marker。
#[tokio::test]
async fn lcg_t10_host_spawn_child_tree_is_killed_on_drop() {
    let root = tempdir().expect("tempdir");
    let target = root.path().join("target");
    fs::create_dir_all(&target).expect("target worktree");
    let fixture = process_tree_fixture(&target);
    let command_text = fixture.command.to_string_lossy().to_string();

    let process = ProcessManager::spawn(
        &command_text,
        &[],
        fixture.command.parent().expect("fixture root"),
        &unix_path_env(),
        CancellationToken::new(),
    )
    .await
    .expect("host spawn process tree (no bwrap prerequisite)");
    wait_for_path(&fixture.parent_pid).await;
    wait_for_path(&fixture.grandchild_pid).await;

    drop(process);
    assert_process_tree_stopped(&fixture).await;
}

/// T14:Coder code_delivery 资源窗进程面——经 `start_owned_process` 登记
/// 的 run-owned 进程树在 quiesce 后以 /proc 证明真实静止(killpg 只是
/// 其中一步);未登记的用户进程与文件不受影响;登记 scratch 清理、
/// 未登记文件保留;`aria.worktree` 协调标签由 scope 注入(硬约束2)。
#[tokio::test]
async fn t14_code_delivery_scope_quiesce_stops_owned_process_tree_only() {
    use crate::cross_cutting::provider_resources::{
        OwnedResourceKind, OwnedResourceSpec, PROVIDER_RESOURCE_LABEL_WORKTREE,
        ProviderRunResourceScope, QuiesceOutcome, ResourceScopeKey, StartedResourceIdentity,
    };

    let base = tempdir().expect("tempdir");
    let store_root = base.path().join("resource-store");
    let private_root = base.path().join("private-root");
    fs::create_dir_all(&private_root).expect("private root");
    let key = ResourceScopeKey {
        scope_id: "code-delivery-coding_attempt_0001".to_string(),
        run_id: "coding_role_run_0001".to_string(),
        boot_identity: "aria-engine-drop-test".to_string(),
    };
    let scope =
        ProviderRunResourceScope::open_for_recovery(&key, None, &private_root, &store_root, None)
            .expect("open code_delivery scope")
            .with_worktree_key("/drop-test-worktree");

    // 登记的 run-owned scratch + 未登记用户文件(同在私有根下)。
    let scratch = private_root.join("scratch");
    fs::create_dir_all(&scratch).expect("scratch");
    fs::write(scratch.join("temp.txt"), "run-owned\n").expect("temp artifact");
    let user_file = private_root.join("user-notes.txt");
    fs::write(&user_file, "user file\n").expect("user file");
    let canonical_scratch = scratch.canonicalize().expect("canonical scratch");
    let scratch_reservation = scope
        .reserve_owned_resource(
            &OwnedResourceSpec::new(
                "coder-run-scratch",
                OwnedResourceKind::Directory,
                "code_delivery run-owned scratch directory",
            )
            .with_private_path(canonical_scratch.clone()),
        )
        .expect("reserve scratch");
    scope
        .confirm_started_resource(
            &scratch_reservation,
            StartedResourceIdentity::directory(&canonical_scratch),
        )
        .expect("confirm scratch");

    // 登记的 run-owned 进程树(真实 sh 孙进程)。
    let fixture = process_tree_fixture(base.path());
    let command = fixture.command.to_string_lossy().to_string();
    let (reservation, mut managed) = scope
        .start_owned_process(
            OwnedResourceSpec::new(
                "coder-worker",
                OwnedResourceKind::Process,
                "coder worker process tree",
            ),
            &command,
            &[],
            fixture.command.parent().expect("fixture root"),
            &unix_path_env(),
            CancellationToken::new(),
        )
        .await
        .expect("spawn owned process tree");
    wait_for_path(&fixture.parent_pid).await;
    wait_for_path(&fixture.grandchild_pid).await;

    // 未登记的用户进程必须不受资源窗静止影响。
    let mut outsider = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn unregistered user process");
    let outsider_pid = outsider.id() as i32;

    // 硬约束2:aria.worktree 协调标签由 scope 注入。
    let labels = reservation.reserved_labels().clone();
    assert_eq!(
        labels
            .get(PROVIDER_RESOURCE_LABEL_WORKTREE)
            .map(String::as_str),
        Some("/drop-test-worktree"),
        "aria.worktree 协调标签必须由 scope 注入(T19 quiesce_targets 枚举依据)"
    );

    // 先 quiesce(静止证明)再清:
    let proof = match scope.quiesce_scope().await.expect("quiesce delivery scope") {
        QuiesceOutcome::Quiescent(proof) => proof,
        QuiesceOutcome::RecoveryRequired(unknown) => {
            panic!(
                "owned process tree must quiesce: {} {}",
                unknown.reason, unknown.detail
            )
        }
    };
    // 组长收割(quiesce 只发信号;不收割则僵尸态不代表存活但会污染
    // /proc 断言——收割后以真实消亡断言)。
    let _ = managed.child.wait().await;
    assert_process_tree_stopped(&fixture).await;
    assert!(
        process_state(outsider_pid).is_some(),
        "未登记用户进程不得被资源窗静止波及"
    );
    scope
        .cleanup_owned_scope(&proof)
        .expect("cleanup delivery scope");
    assert!(!scratch.exists(), "登记 run-owned scratch 必须清理");
    assert!(
        user_file.exists(),
        "私有根内未登记文件不得清理(保留≠忽略未知 resource)"
    );

    // 测试卫生:收尾未登记进程(owned 子进程已在上方收割)。
    let _ = outsider.kill();
    let _ = outsider.wait();
}
