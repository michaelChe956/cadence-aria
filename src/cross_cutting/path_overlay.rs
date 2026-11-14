//! 共享 PATH overlay（计划 2026-11-13 包 2.4/2.4b/2.4c）。
//!
//! 平台「命令可见 PATH」的进程级注册表：装完 CLI 的 bin 目录、启动期发现的
//! 用户工具目录都登记进来；两个命令汇聚点（`process_manager.rs` 的
//! `spawn_with_environment` 与 `cli_adapter.rs` 的 `run_command_capture_with_stream`）
//! 在构造子进程 env 时用 [`effective_path`] 合并 overlay。
//!
//! **不改进程真实 env**（仓库 edition 2024，`std::env::set_var` 是 `unsafe fn`
//! 且多线程改全局 env 有竞态）——只提供合并后的 PATH 给子进程。overlay 是
//! 进程级共享状态：只追加、可去重、测试可清空；一切触碰注册表的用例必须持
//! [`overlay_test_lock`]（或 [`OverlayStateGuard::acquire`]）执行并在结束清空
//! （计划 §6.1 测试隔离纪律），否则会污染并行的 PATH 断言用例。

#[cfg(test)]
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::Duration;

/// overlay 目录的合并位置：装完的目录用 `Prepend`（置于合并 PATH 最前）；
/// 启动期发现的目录用 `Append`（不抢占用户已有工具）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayPosition {
    Prepend,
    Append,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OverlayEntry {
    dir: PathBuf,
    position: OverlayPosition,
}

// 测试构建的线程级存储：overlay 用例与并行用例（如 F10 provider environ
// 逐字断言）**不共享锁** ⇒ 注册表按线程隔离，他线程的登记不得影响本线程
// 的合并结果/子进程 env（竞态修复）；生产构建仍走进程级全局注册表。
#[cfg(test)]
thread_local! {
    static TEST_OVERLAY: RefCell<Vec<OverlayEntry>> = const { RefCell::new(Vec::new()) };
    static TEST_SHELL_PROBE: RefCell<Option<ShellEnvProbe>> = const { RefCell::new(None) };
}

/// overlay 注册表**存储实现**（F12）：生产 = **进程级全局**（跨线程共享是
/// 生产语义，子进程 env 由任意工作线程构造）；测试默认 = **线程级**（并行
/// 用例互不污染，见 `TEST_OVERLAY`）。两种实现都能被同一组断言直接驱动
/// （`*_in` 系函数）⇒ 生产语义不再只靠「测试构建换掉存储」来假定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverlayStorage {
    ProcessGlobal,
    ThreadLocal,
}

/// 默认存储：测试构建走线程级（隔离），生产构建走进程级（跨线程共享）。
fn default_storage() -> OverlayStorage {
    if cfg!(test) {
        OverlayStorage::ThreadLocal
    } else {
        OverlayStorage::ProcessGlobal
    }
}

fn registry() -> &'static Mutex<Vec<OverlayEntry>> {
    static REGISTRY: LazyLock<Mutex<Vec<OverlayEntry>>> = LazyLock::new(|| Mutex::new(Vec::new()));
    &REGISTRY
}

fn lock_registry() -> MutexGuard<'static, Vec<OverlayEntry>> {
    registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 登记一个目录进 overlay。不存在的目录（含普通文件）被忽略；同一目录
/// 重复登记只保留首次的位置（去重，只追加）。只动注册表：不改进程真实
/// env、不写盘、不创建目录。
pub fn register(dir: &Path, position: OverlayPosition) {
    register_in(default_storage(), dir, position);
}

/// 按存储实现登记（F12 seam）：两种实现共用同一套「忽略不存在目录 / 同目录
/// 只保留首次位置」规则。
fn register_in(storage: OverlayStorage, dir: &Path, position: OverlayPosition) {
    if !dir.is_dir() {
        return;
    }
    let add = |entries: &mut Vec<OverlayEntry>| {
        if !entries.iter().any(|entry| entry.dir == dir) {
            entries.push(OverlayEntry {
                dir: dir.to_path_buf(),
                position,
            });
        }
    };
    match storage {
        OverlayStorage::ProcessGlobal => add(&mut lock_registry()),
        OverlayStorage::ThreadLocal => with_thread_overlay_mut(add),
    }
}

/// 线程级存储的可变访问（非测试构建退化为进程级 —— 线程级只是测试隔离手段）。
fn with_thread_overlay_mut<R>(apply: impl FnOnce(&mut Vec<OverlayEntry>) -> R) -> R {
    #[cfg(test)]
    {
        TEST_OVERLAY.with(|overlay| apply(&mut overlay.borrow_mut()))
    }
    #[cfg(not(test))]
    {
        apply(&mut lock_registry())
    }
}

/// 当前 overlay 的目录快照（登记顺序）。
pub fn snapshot() -> Vec<PathBuf> {
    with_overlay_entries(|entries| {
        entries
            .iter()
            .map(|entry| entry.dir.clone())
            .collect::<Vec<_>>()
    })
}

/// 合并 overlay 与 base 成子进程 PATH：**`Prepend` 目录置前（登记顺序）→
/// base 各目录 → `Append` 目录（登记顺序）**；整体去重、保留首次出现。
/// `Append` 的语义是「**不抢占**用户已有工具」⇒ 必须在 base 之后（否则
/// `register_existing_candidates` 登记的 `/usr/local/bin` 会盖到 base 里更靠
/// 前的 venv / fnm 目录，抢用错误的 python / node / npm）。`base` 缺省取
/// 进程 env PATH；overlay 为空时原样返回 base（零行为变化）。
pub fn effective_path(base: Option<&str>) -> String {
    effective_path_in(default_storage(), base)
}

/// 按存储实现合并（F12 seam）。
fn effective_path_in(storage: OverlayStorage, base: Option<&str>) -> String {
    let base_value = match base {
        Some(value) => value.to_string(),
        None => std::env::var_os("PATH")
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    };
    let entries: Vec<OverlayEntry> = with_overlay_entries_in(storage, |entries| entries.to_vec());
    if entries.is_empty() {
        return base_value;
    }
    let mut merged: Vec<PathBuf> = Vec::with_capacity(entries.len());
    for entry in entries
        .iter()
        .filter(|entry| entry.position == OverlayPosition::Prepend)
    {
        if !merged.contains(&entry.dir) {
            merged.push(entry.dir.clone());
        }
    }
    for dir in std::env::split_paths(&base_value) {
        if !dir.as_os_str().is_empty() && !merged.contains(&dir) {
            merged.push(dir);
        }
    }
    for entry in entries
        .iter()
        .filter(|entry| entry.position == OverlayPosition::Append)
    {
        if !merged.contains(&entry.dir) {
            merged.push(entry.dir.clone());
        }
    }
    std::env::join_paths(merged)
        .map(|joined| joined.to_string_lossy().into_owned())
        .unwrap_or(base_value)
}

/// 测试用：清空 overlay 注册表（线程级与进程级都清）与登录 shell 探测结果。
#[cfg(test)]
pub fn clear() {
    TEST_OVERLAY.with(|overlay| overlay.borrow_mut().clear());
    lock_registry().clear();
    TEST_SHELL_PROBE.with(|probe| *probe.borrow_mut() = None);
}

/// 读取 overlay 注册表（默认存储：测试 = 线程级，生产 = 进程级）。
fn with_overlay_entries<R>(read: impl FnOnce(&[OverlayEntry]) -> R) -> R {
    with_overlay_entries_in(default_storage(), read)
}

/// 按存储实现读取注册表（F12 seam）。
fn with_overlay_entries_in<R>(
    storage: OverlayStorage,
    read: impl FnOnce(&[OverlayEntry]) -> R,
) -> R {
    match storage {
        OverlayStorage::ProcessGlobal => read(&lock_registry()),
        OverlayStorage::ThreadLocal => with_thread_overlay(read),
    }
}

/// 线程级存储的只读访问（非测试构建退化为进程级）。
fn with_thread_overlay<R>(read: impl FnOnce(&[OverlayEntry]) -> R) -> R {
    #[cfg(test)]
    {
        TEST_OVERLAY.with(|overlay| read(&overlay.borrow()))
    }
    #[cfg(not(test))]
    {
        read(&lock_registry())
    }
}

/// 登录 shell 探测输出的行标记（rc 文件 banner 污染由「只认标记行」兜底）。
const SHELL_PROBE_MARKER: &str = "__aria_shell_probe__";

/// 计划 2.12：随登录 shell PATH 一并取回的 agent 专属环境变量（本仓四家
/// provider 实际消费的收敛清单；扩展只需在此追加）。
const SHELL_PROBE_VARS: [&str; 3] = ["PI_CODING_AGENT_DIR", "CODEX_HOME", "CLAUDE_CONFIG_DIR"];

/// 登录 shell 探测结果：PATH 原值 + agent 专属环境变量。
#[derive(Debug, Default, Clone)]
struct ShellEnvProbe {
    path: Option<String>,
    vars: std::collections::BTreeMap<String, String>,
}

#[cfg(not(test))]
fn shell_probe_state() -> &'static Mutex<ShellEnvProbe> {
    static STATE: LazyLock<Mutex<ShellEnvProbe>> =
        LazyLock::new(|| Mutex::new(ShellEnvProbe::default()));
    &STATE
}

#[cfg(not(test))]
fn lock_shell_probe() -> std::sync::MutexGuard<'static, ShellEnvProbe> {
    shell_probe_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 探测结果的空默认（测试线程未安装探测时读取）。
#[cfg(test)]
static EMPTY_SHELL_PROBE: ShellEnvProbe = ShellEnvProbe {
    path: None,
    vars: std::collections::BTreeMap::new(),
};

/// 读取登录 shell 探测结果：测试构建读线程级存储（见 `TEST_SHELL_PROBE`），
/// 生产构建读进程级全局状态。
fn with_shell_probe<R>(read: impl FnOnce(&ShellEnvProbe) -> R) -> R {
    #[cfg(test)]
    {
        TEST_SHELL_PROBE.with(|probe| read(probe.borrow().as_ref().unwrap_or(&EMPTY_SHELL_PROBE)))
    }
    #[cfg(not(test))]
    {
        read(&lock_shell_probe())
    }
}

/// 安装探测结果（启动异步探测完成后写入；测试构建写线程级存储）。
fn install_shell_probe(probe: ShellEnvProbe) {
    #[cfg(test)]
    {
        TEST_SHELL_PROBE.with(|state| *state.borrow_mut() = Some(probe));
    }
    #[cfg(not(test))]
    {
        *lock_shell_probe() = probe;
    }
}

/// 最近一次登录 shell 探测的 PATH 原值（未与 overlay 合并）。
pub fn login_shell_path() -> Option<String> {
    with_shell_probe(|probe| probe.path.clone())
}

/// 最近一次登录 shell 探测回的 agent 专属环境变量（如 `PI_CODING_AGENT_DIR`）。
pub fn shell_env(name: &str) -> Option<String> {
    with_shell_probe(|probe| probe.vars.get(name).cloned())
}

/// 把探测到的 agent 专属环境变量补进子进程 env（计划 2.12）：平台已有的
/// 值（进程 env 或调用方显式覆盖层）优先，缺失才补。返回待注入的键值对。
pub fn shell_env_overrides(
    environment: Option<&std::collections::BTreeMap<String, String>>,
) -> Vec<(String, String)> {
    shell_env_overrides_with(environment, |name| std::env::var_os(name))
}

fn shell_env_overrides_with(
    environment: Option<&std::collections::BTreeMap<String, String>>,
    platform_env: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Vec<(String, String)> {
    with_shell_probe(|state| {
        state
            .vars
            .iter()
            .filter(|(name, _)| {
                environment.is_none_or(|env| !env.contains_key(name.as_str()))
                    && platform_env(name).is_none()
            })
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect::<Vec<_>>()
    })
}

fn parse_shell_probe_stdout(stdout: &str) -> ShellEnvProbe {
    let mut probe = ShellEnvProbe::default();
    for line in stdout.lines() {
        let Some(payload) = line.trim().strip_prefix(SHELL_PROBE_MARKER) else {
            continue;
        };
        let Some((name, value)) = payload.split_once('=') else {
            continue;
        };
        let value = value.trim();
        if name == "PATH" {
            probe.path = (!value.is_empty()).then(|| value.to_string());
        } else if value.is_empty() {
            probe.vars.remove(name);
        } else {
            probe.vars.insert(name.to_string(), value.to_string());
        }
    }
    probe
}

/// 测试用：直接安装一份探测结果（绕过真实登录 shell 探测）。
#[cfg(test)]
pub(crate) fn set_shell_probe_for_test(path: Option<String>, vars: Vec<(String, String)>) {
    install_shell_probe(ShellEnvProbe {
        path,
        vars: vars.into_iter().collect(),
    });
}

/// 登录 shell PATH 探测的硬上限（计划 2.4c：2s，超时不阻塞启动）。
const LOGIN_SHELL_PATH_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// 计划 2.4b：启动期把**实际存在**的用户工具目录登记进 overlay（bind
/// 成功后、首次 provider 探活前调用一次）。候选清单沿用 magpie
/// `internal/proc/path_other.go` 的 `userDirs`，全部 `Append`（不抢占用户
/// 已有工具）。只登记、不写盘、不创建目录。2.4c 第二档：登录 shell 的
/// PATH 异步探测（不阻塞启动，2s 硬上限），完成后把其中实际存在的绝对
/// 路径目录以 `Append` 追加。
pub async fn register_startup_overlay() {
    register_existing_candidates(user_home_dir().as_deref());
    tokio::spawn(async {
        if let Some(probe) = probe_login_shell_env().await {
            if let Some(path_value) = probe.path.clone() {
                for dir in std::env::split_paths(&path_value) {
                    if dir.is_absolute() {
                        register(&dir, OverlayPosition::Append);
                    }
                }
            }
            install_shell_probe(probe);
        }
    });
}

fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn register_existing_candidates(home: Option<&Path>) {
    let mut candidates = host_absolute_tool_dirs();
    if let Some(home) = home {
        candidates.extend(user_tool_dirs(home));
    }
    for dir in candidates {
        register(&dir, OverlayPosition::Append);
    }
}

fn host_absolute_tool_dirs() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]
}

/// 已知用户工具目录清单（magpie `userDirs`）：固定清单 + 环境驱动
/// （`$FNM_DIR`、`~/.npmrc` prefix、nvm 默认 node、mise node installs）。
fn user_tool_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        home.join(".local/bin"),
        home.join(".npm-global/bin"),
        home.join(".npm/bin"),
        home.join(".bun/bin"),
        home.join(".volta/bin"),
        home.join(".asdf/shims"),
        home.join(".local/share/mise/shims"),
        home.join(".cargo/bin"),
        home.join(".deno/bin"),
        home.join("Library/pnpm"),
        home.join(".local/share/pnpm"),
        home.join(".local/share/fnm"),
        home.join("Library/Application Support/fnm"),
    ];
    if let Some(fnm_dir) = std::env::var_os("FNM_DIR").filter(|value| !value.is_empty()) {
        dirs.push(PathBuf::from(fnm_dir));
    }
    if let Some(prefix) = npmrc_prefix(home) {
        dirs.push(prefix.join("bin"));
    }
    if let Some(bin_dir) = nvm_default_node_bin(home) {
        dirs.push(bin_dir);
    }
    dirs.extend(mise_node_bins(home));
    dirs
}

/// `~/.npmrc` 的 `prefix=` 值（`~/` 前缀按 home 展开；其余只接受绝对路径）。
fn npmrc_prefix(home: &Path) -> Option<PathBuf> {
    let npmrc = std::fs::read_to_string(home.join(".npmrc")).ok()?;
    let prefix = npmrc
        .lines()
        .find_map(|line| {
            let value = line.trim().strip_prefix("prefix")?;
            Some(value.trim().strip_prefix('=')?.trim().to_string())
        })
        .filter(|value| !value.is_empty())?;
    if let Some(rest) = prefix.strip_prefix("~/") {
        return Some(home.join(rest));
    }
    Path::new(&prefix)
        .is_absolute()
        .then(|| PathBuf::from(prefix))
}

/// nvm 默认 node 的 bin（`~/.nvm/alias/default` 指向的版本目录）。仅接受
/// 可直接解析的 `v<版本>` / `<版本>` 形态；`lts/*`、`node` 等别名无法低成本
/// 解析，跳过（best-effort）。
fn nvm_default_node_bin(home: &Path) -> Option<PathBuf> {
    let alias = std::fs::read_to_string(home.join(".nvm/alias/default"))
        .ok()?
        .trim()
        .to_string();
    if alias.is_empty() {
        return None;
    }
    let is_version = |value: &str| value.starts_with(|c: char| c.is_ascii_digit());
    let version = if let Some(rest) = alias.strip_prefix('v') {
        is_version(rest).then(|| alias.clone())?
    } else {
        is_version(&alias).then(|| format!("v{alias}"))?
    };
    Some(home.join(format!(".nvm/versions/node/{version}/bin")))
}

/// mise 的 node installs（`<数据目录>/mise/installs/node/<版本>/bin`）。
fn mise_node_bins(home: &Path) -> Vec<PathBuf> {
    let mut bins = Vec::new();
    for installs_root in [
        home.join(".local/share/mise/installs/node"),
        home.join("Library/Application Support/mise/installs/node"),
    ] {
        let Ok(entries) = std::fs::read_dir(&installs_root) else {
            continue;
        };
        for entry in entries.flatten() {
            bins.push(entry.path().join("bin"));
        }
    }
    bins
}

/// 登录 shell 探测（计划 2.4c 第二档 + 2.12）：交互式 login shell 以
/// `__aria_shell_probe__` 标记行打印 PATH 与 agent 专属环境变量，2s 硬上限
/// （`kill_on_drop` 兜底回收）；失败/超时返回 None。rc 文件的 banner 污染
/// 由「只认标记行」+ 解析侧空值过滤兜底。
async fn probe_login_shell_env() -> Option<ShellEnvProbe> {
    let shell = std::env::var("SHELL").ok()?;
    let mut script = String::from("printf '");
    script.push_str(SHELL_PROBE_MARKER);
    script.push_str("PATH=%s\\n");
    for name in SHELL_PROBE_VARS {
        script.push_str(SHELL_PROBE_MARKER);
        script.push_str(name);
        script.push_str("=%s\\n");
    }
    script.push_str("' \"$PATH\"");
    for name in SHELL_PROBE_VARS {
        script.push_str(&format!(" \"${{{name}-}}\""));
    }
    let output = tokio::time::timeout(
        LOGIN_SHELL_PATH_PROBE_TIMEOUT,
        tokio::process::Command::new(&shell)
            .args(["-i", "-l", "-c", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_shell_probe_stdout(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// 进程级 overlay 测试锁：一切触碰注册表的用例（含其他文件里的合并点
/// 断言）必须持本锁执行，避免并行用例互相污染（计划 §6.1）。
#[cfg(test)]
pub(crate) fn overlay_test_lock() -> &'static Mutex<()> {
    static LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
    &LOCK
}

/// 持全局锁 + 进入时清空 + 退出（含 panic 展开）时清空的 RAII 守卫。
#[cfg(test)]
pub(crate) struct OverlayStateGuard {
    _guard: MutexGuard<'static, ()>,
}

#[cfg(test)]
impl OverlayStateGuard {
    pub(crate) fn acquire() -> Self {
        let guard = overlay_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        clear();
        Self { _guard: guard }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_ignores_missing_directories() {
        let _state = OverlayStateGuard::acquire();

        let missing = std::env::temp_dir().join("aria-path-overlay-definitely-missing");
        let _ = std::fs::remove_dir_all(&missing);
        register(&missing, OverlayPosition::Prepend);

        let holder = tempfile::tempdir().expect("tempdir");
        let plain_file = holder.path().join("plain-file");
        std::fs::write(&plain_file, "not a dir").expect("write file");
        register(&plain_file, OverlayPosition::Append);

        assert!(
            snapshot().is_empty(),
            "不存在/非目录的路径不得登记: {:?}",
            snapshot()
        );
        assert!(!missing.exists(), "登记不得创建目录");
    }

    #[test]
    fn register_deduplicates_same_directory() {
        let _state = OverlayStateGuard::acquire();

        let dir = tempfile::tempdir().expect("tempdir");
        register(dir.path(), OverlayPosition::Prepend);
        // 同目录重登（换位置）→ 去重，保留首次的 Prepend。
        register(dir.path(), OverlayPosition::Append);

        assert_eq!(snapshot(), vec![dir.path().to_path_buf()]);
        let merged = effective_path(Some("/usr/bin:/bin"));
        assert_eq!(
            std::env::split_paths(&merged).next(),
            Some(dir.path().to_path_buf()),
            "首次登记的 Prepend 位置必须保留: {merged}"
        );
    }

    #[test]
    fn effective_path_orders_prepend_then_base_then_append_deduped() {
        let _state = OverlayStateGuard::acquire();

        let root = tempfile::tempdir().expect("tempdir");
        let a = root.path().join("a");
        let b = root.path().join("b");
        let c = root.path().join("c");
        let base_dir = root.path().join("base");
        for dir in [&a, &b, &c, &base_dir] {
            std::fs::create_dir_all(dir).expect("mkdir");
        }

        register(&a, OverlayPosition::Prepend);
        register(&b, OverlayPosition::Append);
        register(&c, OverlayPosition::Prepend);

        let base = format!(
            "{base}:{a}:{base}",
            base = base_dir.display(),
            a = a.display()
        );
        let merged = effective_path(Some(&base));
        let entries: Vec<PathBuf> = std::env::split_paths(&merged).collect();

        // `Append` 的语义是「**不抢占**用户已有工具」⇒ 必须在 base 之后；
        // 否则 base 里更靠前的 python / node / npm 会被覆盖（F7）。
        assert_eq!(
            entries,
            vec![a.clone(), c.clone(), base_dir.clone(), b.clone()],
            "Prepend(登记序) → base(去重保首次) → Append(登记序): {merged}"
        );
    }

    #[test]
    fn effective_path_append_never_preempts_base_entries() {
        let _state = OverlayStateGuard::acquire();

        let root = tempfile::tempdir().expect("tempdir");
        let venv_bin = root.path().join("venv/bin");
        let usr_local_bin = root.path().join("usr/local/bin");
        for dir in [&venv_bin, &usr_local_bin] {
            std::fs::create_dir_all(dir).expect("mkdir");
        }

        // 启动期发现 `~/.local/bin` / `/usr/local/bin` 时用 `Append` 登记 ⇒
        // 不得跑到服务进程 PATH（venv 优先）之前抢用错误的 python / node。
        register(&usr_local_bin, OverlayPosition::Append);

        let base = format!(
            "{venv}:{usr_local}",
            venv = venv_bin.display(),
            usr_local = usr_local_bin.display()
        );
        let merged = effective_path(Some(&base));
        let entries: Vec<PathBuf> = std::env::split_paths(&merged).collect();

        assert_eq!(
            entries,
            vec![venv_bin.clone(), usr_local_bin.clone()],
            "Append 目录已在 base 里 ⇒ 保持 base 的原位置（不得前插）: {merged}"
        );
    }

    /// F12：**生产语义**的存储实现（进程级全局注册表）必须**跨线程可见**
    /// —— 登记发生在工作线程 A、`effective_path` 在服务主线程 B 构造子进程
    /// env；同时线程级存储（测试默认）保持隔离。
    #[test]
    fn process_global_registry_is_shared_across_threads() {
        let _state = OverlayStateGuard::acquire();

        let root = tempfile::tempdir().expect("tempdir");
        let shared = root.path().join("shared-bin");
        std::fs::create_dir_all(&shared).expect("mkdir");

        // 生产存储：另一个线程登记 ⇒ 本线程必须看得到。
        let registered = shared.clone();
        std::thread::spawn(move || {
            register_in(
                OverlayStorage::ProcessGlobal,
                &registered,
                OverlayPosition::Prepend,
            );
        })
        .join()
        .expect("thread");

        let merged = effective_path_in(OverlayStorage::ProcessGlobal, Some("/usr/bin"));
        assert_eq!(
            std::env::split_paths(&merged).next(),
            Some(shared.clone()),
            "进程级注册表必须跨线程共享（生产语义）: {merged}"
        );

        // 线程级存储（测试默认）看不到他线程的登记：隔离语义不得被削弱。
        let local = effective_path(Some("/usr/bin"));
        assert_eq!(
            local, "/usr/bin",
            "线程级存储必须保持隔离（不得被进程级登记污染）: {local}"
        );

        lock_registry().clear();
    }

    #[test]
    fn effective_path_returns_base_unchanged_when_overlay_empty() {
        let _state = OverlayStateGuard::acquire();

        assert_eq!(effective_path(Some("/usr/bin:/bin")), "/usr/bin:/bin");
        assert_eq!(effective_path(Some("")), "");
        assert_eq!(
            effective_path(None),
            std::env::var("PATH").unwrap_or_default(),
            "overlay 为空时 base 缺省取进程 env PATH 且原样返回"
        );
    }

    /// 计划 2.4b：启动登记只收**实际存在**的候选目录；不存在的候选不得被
    /// 登记、也不得在盘上创建。
    #[test]
    fn startup_registers_only_existing_install_bin_dirs() {
        let _state = OverlayStateGuard::acquire();

        let home = tempfile::tempdir().expect("temp home");
        for existing in [".local/bin", ".cargo/bin"] {
            std::fs::create_dir_all(home.path().join(existing)).expect("create existing candidate");
        }
        // ~/.npmrc 的 prefix + /bin
        let npm_prefix = home.path().join("npm-prefix");
        std::fs::create_dir_all(npm_prefix.join("bin")).expect("create npm prefix bin");
        std::fs::write(
            home.path().join(".npmrc"),
            format!("prefix={}\n", npm_prefix.display()),
        )
        .expect("write npmrc");
        // nvm 默认 node 的 bin
        std::fs::create_dir_all(home.path().join(".nvm/alias")).expect("create nvm alias dir");
        std::fs::write(home.path().join(".nvm/alias/default"), "v22.11.0\n")
            .expect("write nvm default alias");
        std::fs::create_dir_all(home.path().join(".nvm/versions/node/v22.11.0/bin"))
            .expect("create nvm node bin");
        // mise 的 node installs
        std::fs::create_dir_all(
            home.path()
                .join(".local/share/mise/installs/node/22.11.0/bin"),
        )
        .expect("create mise node bin");

        register_existing_candidates(Some(home.path()));

        let registered = snapshot();
        let expected = [
            home.path().join(".local/bin"),
            home.path().join(".cargo/bin"),
            npm_prefix.join("bin"),
            home.path().join(".nvm/versions/node/v22.11.0/bin"),
            home.path()
                .join(".local/share/mise/installs/node/22.11.0/bin"),
        ];
        for dir in &expected {
            assert!(
                registered.contains(dir),
                "存在的候选目录必须登记 {dir:?}: {registered:?}"
            );
        }
        // 临时 home 下的登记项必须全部来自预建目录（不得凭空多出）。
        for dir in registered.iter().filter(|dir| dir.starts_with(home.path())) {
            assert!(expected.contains(dir), "未预建的目录不得被登记 {dir:?}");
        }
        // 不存在的候选不得被登记，也不得在盘上创建。
        for missing in [
            ".npm-global/bin",
            ".npm/bin",
            ".bun/bin",
            ".volta/bin",
            ".asdf/shims",
            ".deno/bin",
        ] {
            let path = home.path().join(missing);
            assert!(!path.exists(), "登记不得创建目录: {missing}");
            assert!(
                !registered.contains(&path),
                "不存在的候选不得登记: {missing}"
            );
        }
    }

    /// 无 HOME 时只登记宿主绝对路径目录，不得出现相对路径候选。
    #[test]
    fn startup_without_home_registers_only_host_absolute_dirs() {
        let _state = OverlayStateGuard::acquire();

        register_existing_candidates(None);

        for dir in snapshot() {
            assert!(dir.is_absolute(), "无 HOME 时不得登记相对路径目录: {dir:?}");
        }
        assert!(
            !snapshot().iter().any(|dir| dir == Path::new(".local/bin")),
            "相对形态的候选不得出现"
        );
    }

    #[test]
    fn clear_empties_registry() {
        let _state = OverlayStateGuard::acquire();

        let dir = tempfile::tempdir().expect("tempdir");
        register(dir.path(), OverlayPosition::Prepend);
        assert_eq!(snapshot().len(), 1);

        clear();
        assert!(snapshot().is_empty());
        assert_eq!(
            effective_path(Some("/usr/bin:/bin")),
            "/usr/bin:/bin",
            "清空后合并退化为 base 原值"
        );
    }

    /// 计划 2.12：登录 shell 探测输出解析——只认标记行，PATH 与 agent 变量
    /// 都取末次出现，空值跳过，banner 噪声行忽略。
    #[test]
    fn parse_shell_probe_output_extracts_path_and_agent_vars() {
        let stdout = "zsh banner noise\n\
                      __aria_shell_probe__PATH=/shell/path/a:/shell/path/b\n\
                      __aria_shell_probe__PI_CODING_AGENT_DIR=/home/u/.pi/agent\n\
                      __aria_shell_probe__CODEX_HOME=\n\
                      __aria_shell_probe__CLAUDE_CONFIG_DIR=/home/u/.claude\n\
                      trailing noise\n";
        let probe = parse_shell_probe_stdout(stdout);
        assert_eq!(probe.path.as_deref(), Some("/shell/path/a:/shell/path/b"));
        assert_eq!(
            probe.vars.get("PI_CODING_AGENT_DIR").map(String::as_str),
            Some("/home/u/.pi/agent")
        );
        assert_eq!(
            probe.vars.get("CLAUDE_CONFIG_DIR").map(String::as_str),
            Some("/home/u/.claude")
        );
        assert!(
            !probe.vars.contains_key("CODEX_HOME"),
            "空值变量不得进入补入清单"
        );
    }

    /// 计划 2.12：补入清单「平台已有值优先」——进程 env 或显式覆盖层已有
    /// 的变量不补，缺失才补；getter 返回已安装的探测值。
    #[test]
    fn shell_env_overrides_skip_platform_owned_values() {
        let _state = OverlayStateGuard::acquire();

        set_shell_probe_for_test(
            Some("/shell/path".to_string()),
            vec![
                ("PI_CODING_AGENT_DIR".to_string(), "/probe/pi".to_string()),
                ("CODEX_HOME".to_string(), "/probe/codex".to_string()),
            ],
        );
        assert_eq!(login_shell_path().as_deref(), Some("/shell/path"));
        assert_eq!(
            shell_env("PI_CODING_AGENT_DIR").as_deref(),
            Some("/probe/pi")
        );

        // 平台进程 env 已有 CODEX_HOME(platform_env 返回 Some) ⇒ 不补。
        let platform_env =
            |name: &str| (name == "CODEX_HOME").then(|| std::ffi::OsString::from("/host/codex"));
        assert_eq!(
            shell_env_overrides_with(None, platform_env),
            vec![("PI_CODING_AGENT_DIR".to_string(), "/probe/pi".to_string())]
        );

        // 显式覆盖层已有 PI_CODING_AGENT_DIR ⇒ 也不补。
        let environment = std::collections::BTreeMap::from([(
            "PI_CODING_AGENT_DIR".to_string(),
            "/explicit/pi".to_string(),
        )]);
        assert!(
            shell_env_overrides_with(Some(&environment), platform_env).is_empty(),
            "覆盖层已有的变量不得被探测值覆盖"
        );
    }

    /// 竞态修复（全量 lib 暴露：4 条 F10 provider environ 逐字断言被并行
    /// overlay 用例的登记窗口打穿）：overlay 用例与 F10 同位用例**不共享
    /// 锁** ⇒ 注册表在测试构建里必须按线程隔离——他线程登记的目录只对
    /// 登记线程可见，不得泄漏进本线程的合并结果/快照；登记线程自身必须
    /// 照常看到合并。生产构建仍走进程级全局注册表。
    #[test]
    fn overlay_registrations_from_other_threads_do_not_leak_into_this_thread() {
        let _state = OverlayStateGuard::acquire();

        let holder = tempfile::tempdir().expect("tempdir");
        let registered = holder.path().join("bin");
        std::fs::create_dir_all(&registered).expect("mkdir");
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_registered = registered.clone();
        let worker = std::thread::spawn(move || {
            // 模拟并行 overlay 用例：在别的线程登记并在该线程内断言可见。
            register(&worker_registered, OverlayPosition::Prepend);
            let merged = effective_path(Some("/usr/bin"));
            sender.send(merged).expect("send merged");
        });
        let other_thread_merged = receiver.recv().expect("recv merged");
        worker.join().expect("join worker");

        assert!(
            std::env::split_paths(&other_thread_merged).any(|dir| dir == registered),
            "登记线程自身必须看到合并结果: {other_thread_merged}"
        );
        assert_eq!(
            effective_path(Some("/usr/bin")),
            "/usr/bin",
            "他线程的 overlay 登记不得泄漏进本线程的合并结果(F10 同位)"
        );
        assert!(
            !snapshot().contains(&registered),
            "他线程的登记不得出现在本线程快照"
        );
    }
}
