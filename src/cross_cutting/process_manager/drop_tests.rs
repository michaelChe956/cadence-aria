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

/// 递归快照(rel path → bytes);受保护面 pre/post 对比用。
fn snapshot_tree(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, snapshot: &mut std::collections::BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).expect("snapshot read dir") {
            let entry = entry.expect("snapshot entry");
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, snapshot);
            } else {
                let rel = path
                    .strip_prefix(base)
                    .expect("snapshot prefix")
                    .to_string_lossy()
                    .into_owned();
                snapshot.insert(rel, fs::read(&path).unwrap_or_default());
            }
        }
    }
    let mut snapshot = std::collections::BTreeMap::new();
    walk(root, root, &mut snapshot);
    snapshot
}

/// Task 6a 段 2:Coding target-only 写边界——target 写成功、target 内受控
/// git commit 成功(git identity 沿授权链),root/成员/target `.git` 指针与
/// 所有 `.aria` 的写尝试全部被后置只读挂载拒绝,pre==post 快照零漂移。
#[tokio::test]
async fn lcg_t06_coding_allows_target_commit_and_denies_protected_roots() {
    use crate::cross_cutting::provider_boundary::{
        ProviderBoundaryLauncher, ProviderBoundaryMode, ProviderBoundaryPlan,
        run_builtin_write_probe,
    };

    let base = tempdir().expect("base dir");
    let root = base.path().join("lc-root");
    fs::create_dir_all(root.join(".aria")).expect("root aria");
    fs::write(root.join("AGENTS.md"), "# lc root\n").expect("agents");
    fs::write(root.join(".aria").join("state.json"), "{}").expect("root aria state");
    let member = root.join("member-a");
    fs::create_dir_all(&member).expect("member");
    fs::write(member.join("README.md"), "member\n").expect("member readme");
    let repo = base.path().join("repo");
    fs::create_dir_all(&repo).expect("repo");
    let target = base.path().join("target-wt");
    let home = base.path().join("home");
    fs::create_dir_all(home.join(".claude")).expect("runtime home");

    let git = |dir: &Path, args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("git fixture");
        assert!(status.success(), "git fixture {args:?} failed");
    };
    git(&root, &["init", "-q"]);
    git(&member, &["init", "-q"]);
    git(
        &member,
        &[
            "-c",
            "user.name=aria",
            "-c",
            "user.email=aria@aria",
            "commit",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=aria",
            "-c",
            "user.email=aria@aria",
            "commit",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            target.to_string_lossy().as_ref(),
            "-b",
            "t6a-coding",
        ],
    );
    fs::create_dir_all(target.join(".aria")).expect("target aria");
    fs::write(target.join(".aria").join("state.json"), "{}").expect("target aria state");

    let launcher = ProviderBoundaryLauncher::probe_environment();
    assert!(
        launcher.is_available(),
        "environment blocked: mandatory coding write-boundary case needs bwrap + user namespace"
    );
    let plan = ProviderBoundaryPlan::new(
        ProviderBoundaryMode::TargetWriteOnly,
        root.clone(),
        Some(target.clone()),
        Vec::new(),
    );
    let mut env = BTreeMap::new();
    env.insert("HOME".to_string(), home.to_string_lossy().into_owned());
    env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
    env.insert("GIT_CONFIG_GLOBAL".to_string(), "/dev/null".to_string());
    env.insert("GIT_CONFIG_NOSYSTEM".to_string(), "1".to_string());
    env.insert("GIT_AUTHOR_NAME".to_string(), "aria".to_string());
    env.insert("GIT_AUTHOR_EMAIL".to_string(), "aria@aria".to_string());
    env.insert("GIT_COMMITTER_NAME".to_string(), "aria".to_string());
    env.insert("GIT_COMMITTER_EMAIL".to_string(), "aria@aria".to_string());

    let protected_pre_snapshot = (
        snapshot_tree(&root),
        fs::read(target.join(".git")).expect("target git pointer"),
        snapshot_tree(&target.join(".aria")),
    );

    // 正向:target 内写(provider 内置通道)成功且真实落盘。
    let probe_file = target.join("aria-write-probe.txt");
    let target_attempts = run_builtin_write_probe(&launcher, &plan, &env, &[probe_file.clone()])
        .await
        .expect("target write probe");
    let write_target_result = target_attempts
        .first()
        .expect("target write attempt")
        .result();
    assert_eq!(write_target_result, Ok(()));
    assert_eq!(
        fs::read(&probe_file).expect("target probe file"),
        b"aria-boundary-probe".to_vec()
    );

    // 正向:target 内受控 git commit(git identity 沿冻结授权链)成功。
    let target_text = target.to_string_lossy().into_owned();
    let commit_script = format!(
        "git -C '{target_text}' add aria-write-probe.txt && \
         git -C '{target_text}' -c commit.gpgsign=false commit -q -m aria-boundary-probe-commit"
    );
    let mut commit_process = ProcessManager::spawn_with_boundary_resolved(
        launcher.clone(),
        "sh",
        &["-c", &commit_script],
        &root,
        &env,
        &plan,
        CancellationToken::new(),
    )
    .await
    .expect("controlled commit spawn");
    drop(commit_process.stdin);
    let commit_status = commit_process
        .child
        .wait()
        .await
        .expect("controlled commit wait");
    let committed_subject = std::process::Command::new("git")
        .arg("-C")
        .arg(&target)
        .args(["log", "-1", "--pretty=%s"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("host log verify");
    let controlled_target_commit_succeeded = commit_status.success()
        && String::from_utf8_lossy(&committed_subject.stdout).trim()
            == "aria-boundary-probe-commit";
    assert!(controlled_target_commit_succeeded);

    // 负向:受保护根(root/成员 `.git`、`.aria`、target `.git` 指针)全拒绝。
    let protected_paths = [
        root.join(".git").join("rogue"),
        root.join(".aria").join("rogue"),
        member.join(".git").join("rogue"),
        member.join("rogue"),
        target.join(".aria").join("rogue"),
        target.join(".git"),
    ];
    let protected_attempts = run_builtin_write_probe(&launcher, &plan, &env, &protected_paths)
        .await
        .expect("protected write probe");
    assert!(
        protected_attempts
            .iter()
            .all(|attempt| attempt.was_refused_with_evidence())
    );
    for path in &protected_paths {
        assert!(
            !path.exists() || path == &target.join(".git"),
            "rogue write landed at {}",
            path.display()
        );
    }

    // 受保护面 pre==post:零漂移(root 快照 + target `.git` 指针 + `.aria`)。
    let protected_post_snapshot = (
        snapshot_tree(&root),
        fs::read(target.join(".git")).expect("target git pointer post"),
        snapshot_tree(&target.join(".aria")),
    );
    assert_eq!(protected_pre_snapshot, protected_post_snapshot);
}

/// Task 6a 段 3:read-only 写边界覆盖 provider 后代写面——内建、terminal
/// 子进程、MCP 后代与 extension 子进程对 root/成员/元数据的写全部被拒绝
/// 且带证据,pre==post 快照零漂移;不可隔离的外部 MCP 写通道同样只命中
/// 只读挂载面而被阻断。
#[tokio::test]
async fn lcg_t06_readonly_blocks_builtin_terminal_mcp_and_child_writes() {
    use crate::cross_cutting::provider_boundary::{
        BoundaryWriteChannel, PlannedBoundaryWrite, ProviderBoundaryLauncher, ProviderBoundaryMode,
        ProviderBoundaryPlan, run_write_surface_probe,
    };

    let base = tempdir().expect("base dir");
    let root = base.path().join("lc-root");
    fs::create_dir_all(root.join(".aria")).expect("root aria");
    fs::write(root.join("AGENTS.md"), "# lc root\n").expect("agents");
    fs::write(root.join(".aria").join("state.json"), "{}").expect("root aria state");
    fs::write(root.join(".mcp.json"), "{}\n").expect("mcp config");
    let member = root.join("member-a");
    fs::create_dir_all(&member).expect("member");
    fs::write(member.join("README.md"), "member\n").expect("member readme");
    let member_b = root.join("member-b");
    fs::create_dir_all(&member_b).expect("member b");
    fs::write(member_b.join("note.txt"), "note\n").expect("member note");
    let home = base.path().join("home");
    fs::create_dir_all(home.join(".claude")).expect("runtime home");

    let git = |dir: &Path, args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("git fixture");
        assert!(status.success(), "git fixture {args:?} failed");
    };
    git(&root, &["init", "-q"]);
    git(&member, &["init", "-q"]);

    let launcher = ProviderBoundaryLauncher::probe_environment();
    assert!(
        launcher.is_available(),
        "environment blocked: mandatory read-only descendant case needs bwrap + user namespace"
    );
    let plan = ProviderBoundaryPlan::new(
        ProviderBoundaryMode::ReadOnly,
        root.clone(),
        None,
        Vec::new(),
    );
    let mut env = BTreeMap::new();
    env.insert("HOME".to_string(), home.to_string_lossy().into_owned());
    env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());

    let attempts = vec![
        PlannedBoundaryWrite::new(BoundaryWriteChannel::Builtin, root.join("rogue-builtin")),
        PlannedBoundaryWrite::new(
            BoundaryWriteChannel::Builtin,
            root.join(".git").join("rogue"),
        ),
        PlannedBoundaryWrite::new(
            BoundaryWriteChannel::Terminal,
            root.join(".aria").join("rogue"),
        ),
        PlannedBoundaryWrite::new(BoundaryWriteChannel::Terminal, member.join("rogue")),
        PlannedBoundaryWrite::new(BoundaryWriteChannel::Mcp, root.join(".mcp.json.rogue")),
        PlannedBoundaryWrite::new(BoundaryWriteChannel::Mcp, member.join(".git").join("rogue")),
        PlannedBoundaryWrite::new(BoundaryWriteChannel::Extension, member_b.join("rogue")),
        PlannedBoundaryWrite::new(BoundaryWriteChannel::Extension, root.join("AGENTS.md")),
    ];
    let protected_pre_snapshot = snapshot_tree(&root);

    let observed = run_write_surface_probe(&launcher, &plan, &env, &attempts)
        .await
        .expect("descendant write surface probe");
    let protected_attempts: Vec<_> = observed;
    assert_eq!(protected_attempts.len(), attempts.len());
    assert!(
        protected_attempts
            .iter()
            .all(|attempt| attempt.was_refused_with_evidence())
    );

    // 越界写零落盘(含既有 AGENTS.md 未被覆写)。
    for path in [
        root.join("rogue-builtin"),
        root.join(".git").join("rogue"),
        root.join(".aria").join("rogue"),
        member.join("rogue"),
        root.join(".mcp.json.rogue"),
        member.join(".git").join("rogue"),
        member_b.join("rogue"),
    ] {
        assert!(!path.exists(), "rogue write landed at {}", path.display());
    }

    let protected_post_snapshot = snapshot_tree(&root);
    assert_eq!(protected_pre_snapshot, protected_post_snapshot);
}
