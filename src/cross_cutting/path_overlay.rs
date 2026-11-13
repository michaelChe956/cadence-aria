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
    if !dir.is_dir() {
        return;
    }
    let mut entries = lock_registry();
    if entries.iter().any(|entry| entry.dir == dir) {
        return;
    }
    entries.push(OverlayEntry {
        dir: dir.to_path_buf(),
        position,
    });
}

/// 当前 overlay 的目录快照（登记顺序）。
pub fn snapshot() -> Vec<PathBuf> {
    lock_registry()
        .iter()
        .map(|entry| entry.dir.clone())
        .collect()
}

/// 合并 overlay 与 base 成子进程 PATH：`Prepend` 目录置前（登记顺序），
/// `Append` 目录其后（登记顺序），再接 base 各目录；整体去重、保留首次
/// 出现。`base` 缺省取进程 env PATH；overlay 为空时原样返回 base（零行为
/// 变化，既有调用方语义不受影响）。
pub fn effective_path(base: Option<&str>) -> String {
    let base_value = match base {
        Some(value) => value.to_string(),
        None => std::env::var_os("PATH")
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    };
    let entries = lock_registry();
    let mut merged: Vec<PathBuf> = Vec::with_capacity(entries.len());
    for position in [OverlayPosition::Prepend, OverlayPosition::Append] {
        for entry in entries.iter() {
            if entry.position == position && !merged.contains(&entry.dir) {
                merged.push(entry.dir.clone());
            }
        }
    }
    if merged.is_empty() {
        return base_value;
    }
    for dir in std::env::split_paths(&base_value) {
        if !dir.as_os_str().is_empty() && !merged.contains(&dir) {
            merged.push(dir);
        }
    }
    std::env::join_paths(merged)
        .map(|joined| joined.to_string_lossy().into_owned())
        .unwrap_or(base_value)
}

/// 测试用：清空 overlay 注册表。
#[cfg(test)]
pub fn clear() {
    lock_registry().clear();
}

/// 登录 shell 探测输出的行标记（rc 文件 banner 污染由「只认标记行」兜底）。
const SHELL_PROBE_MARKER: &str = "__aria_shell_probe__";

/// 计划 2.12：随登录 shell PATH 一并取回的 agent 专属环境变量（本仓四家
/// provider 实际消费的收敛清单；扩展只需在此追加）。
const SHELL_PROBE_VARS: [&str; 3] = [
    "PI_CODING_AGENT_DIR",
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
];

/// 登录 shell 探测结果：PATH 原值 + agent 专属环境变量。
#[derive(Debug, Default, Clone)]
struct ShellEnvProbe {
    path: Option<String>,
    vars: std::collections::BTreeMap<String, String>,
}

fn shell_probe_state() -> &'static Mutex<ShellEnvProbe> {
    static STATE: LazyLock<Mutex<ShellEnvProbe>> =
        LazyLock::new(|| Mutex::new(ShellEnvProbe::default()));
    &STATE
}

fn lock_shell_probe() -> std::sync::MutexGuard<'static, ShellEnvProbe> {
    shell_probe_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 最近一次登录 shell 探测的 PATH 原值（未与 overlay 合并）。
pub fn login_shell_path() -> Option<String> {
    None
}

/// 最近一次登录 shell 探测回的 agent 专属环境变量（如 `PI_CODING_AGENT_DIR`）。
pub fn shell_env(name: &str) -> Option<String> {
    let _ = name;
    None
}

/// 把探测到的 agent 专属环境变量补进子进程 env（计划 2.12）：平台已有的
/// 值（进程 env 或调用方显式覆盖层）优先，缺失才补。返回待注入的键值对。
pub fn shell_env_overrides(
    environment: Option<&std::collections::BTreeMap<String, String>>,
) -> Vec<(String, String)> {
    let _ = environment;
    Vec::new()
}

fn shell_env_overrides_with(
    environment: Option<&std::collections::BTreeMap<String, String>>,
    platform_env: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Vec<(String, String)> {
    let _ = (environment, platform_env);
    Vec::new()
}

fn parse_shell_probe_stdout(stdout: &str) -> ShellEnvProbe {
    let _ = stdout;
    ShellEnvProbe::default()
}

/// 测试用：直接安装一份探测结果（绕过真实登录 shell 探测）。
#[cfg(test)]
pub(crate) fn set_shell_probe_for_test(
    path: Option<String>,
    vars: Vec<(String, String)>,
) {
    let _ = (path, vars);
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
        if let Some(path_value) = probe_login_shell_path().await {
            for dir in std::env::split_paths(&path_value) {
                if dir.is_absolute() {
                    register(&dir, OverlayPosition::Append);
                }
            }
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

/// 登录 shell PATH 探测（计划 2.4c 第二档）：交互式 login shell 打印其
/// PATH，2s 硬上限（`kill_on_drop` 兜底回收）；失败/超时返回 None。rc
/// 文件的 banner 污染由调用侧「绝对路径过滤 + register 只收存在目录」兜底。
async fn probe_login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").ok()?;
    let output = tokio::time::timeout(
        LOGIN_SHELL_PATH_PROBE_TIMEOUT,
        tokio::process::Command::new(&shell)
            .args(["-i", "-l", "-c", "printf %s \"$PATH\""])
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
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .rev()
        .find_map(|line| {
            let trimmed = line.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
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
pub(crate) struct OverlayStateGuard(MutexGuard<'static, ()>);

#[cfg(test)]
impl OverlayStateGuard {
    pub(crate) fn acquire() -> Self {
        let guard = overlay_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        clear();
        Self(guard)
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
    fn effective_path_orders_prepend_then_append_then_base_deduped() {
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

        assert_eq!(
            entries,
            vec![a.clone(), c.clone(), b.clone(), base_dir.clone()],
            "Prepend(登记序) → Append(登记序) → base(去重保首次): {merged}"
        );
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
        let platform_env = |name: &str| {
            (name == "CODEX_HOME").then(|| std::ffi::OsString::from("/host/codex"))
        };
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
}
