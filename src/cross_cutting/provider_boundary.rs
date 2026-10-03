//! Provider 写边界(write boundary)的纯 DTO owner(Task 1a,REQ-LCG-03/07)。
//!
//! `ProviderBoundaryPlan` 是 validated launch 持有的不可伪造边界意图:
//! 进程 cwd(canonical LC root)、唯一可写 target(Coding;read-only action
//! 为 `None`)与受保护根(`.git`/`.aria` 等元数据位置)。`ProviderBoundary
//! Evidence` 是真实 probe(6c)产出并经三方一致性校验(2d)后导入 durable
//! capability 的证据记录。本文件只拥有 DTO 形状;`ProcessManager::
//! spawn_with_boundary` 等 launcher 行为归 Task 6a,probe 归 Task 6c。
//!
//! 字段私有+`pub(crate)` 构造:plan/evidence 只能由 gateway/probe 装配,
//! 调用方不能自造;读取经 pub getter(只读投影)。

use std::path::{Path, PathBuf};

use crate::product::models::ProviderName;

/// 边界模式:read-only action 无任何可写 target;Coding 恰一个 canonical
/// target 可写(target-only)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderBoundaryMode {
    /// Planning/Review:root、成员与元数据均不可写。
    ReadOnly,
    /// Coding:唯一 writable root 恰为 target worktree。
    TargetWriteOnly,
}

/// validated launch 持有的不可伪造边界计划(Task 1a 冻结形状)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderBoundaryPlan {
    mode: ProviderBoundaryMode,
    /// 进程 cwd(canonical `provider_context_root`)。target 独立解析,
    /// 不得替代 cwd(Global Constraints)。
    working_directory: PathBuf,
    /// 唯一可写 target(Coding);read-only action 为 `None`。
    target_root: Option<PathBuf>,
    /// 受保护根/路径(`.git`、`.aria` 等元数据位置;root 与非 target 的
    /// 保护由 launcher/负向探针执行)。
    protected_roots: Vec<PathBuf>,
}

impl ProviderBoundaryPlan {
    /// crate 内构造(gateway/probe 装配;调用方不能自造)。
    pub(crate) fn new(
        mode: ProviderBoundaryMode,
        working_directory: PathBuf,
        target_root: Option<PathBuf>,
        protected_roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            mode,
            working_directory,
            target_root,
            protected_roots,
        }
    }

    pub fn mode(&self) -> ProviderBoundaryMode {
        self.mode
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub fn target_root(&self) -> Option<&Path> {
        self.target_root.as_deref()
    }

    pub fn protected_roots(&self) -> &[PathBuf] {
        &self.protected_roots
    }
}

/// 真实 boundary probe 的证据记录(Task 1a 冻结形状;6c 产出、2d 导入)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderBoundaryEvidence {
    provider: ProviderName,
    /// 被探测 provider 的 exact CLI version(与 capability record 比对)。
    exact_version: String,
    boundary_mode: ProviderBoundaryMode,
    /// 当次 probe 比对的 projection digest(与 session projection、
    /// capability action row 三方一致才可导入 Confirmed)。
    projection_digest: String,
    /// probe 工件引用(快照/日志的 durable 引用)。
    artifact_ref: String,
    probed_at: String,
}

impl ProviderBoundaryEvidence {
    /// crate 内构造(6c probe 产出;调用方不能自造)。
    pub(crate) fn new(
        provider: ProviderName,
        exact_version: String,
        boundary_mode: ProviderBoundaryMode,
        projection_digest: String,
        artifact_ref: String,
        probed_at: String,
    ) -> Self {
        Self {
            provider,
            exact_version,
            boundary_mode,
            projection_digest,
            artifact_ref,
            probed_at,
        }
    }

    pub fn provider(&self) -> &ProviderName {
        &self.provider
    }

    pub fn exact_version(&self) -> &str {
        &self.exact_version
    }

    pub fn boundary_mode(&self) -> ProviderBoundaryMode {
        self.boundary_mode
    }

    pub fn projection_digest(&self) -> &str {
        &self.projection_digest
    }

    pub fn artifact_ref(&self) -> &str {
        &self.artifact_ref
    }

    pub fn probed_at(&self) -> &str {
        &self.probed_at
    }
}

/// 边界执行/校验错误(稳定判别码 + 上下文)。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderBoundaryError {
    /// 当前 OS/运行时不支持可控写边界,不得退回无隔离 spawn。
    #[error("provider_boundary_unsupported: {0}")]
    Unsupported(String),
    /// plan 非法(cwd/target/protected roots 缺失或不一致)。
    #[error("provider_boundary_invalid_plan: {0}")]
    InvalidPlan(String),
    /// 真实 probe 失败(越界写成功/保护位置可写等)。
    #[error("provider_boundary_probe_failed: {0}")]
    ProbeFailed(String),
}

// ============================================================================
// Task 6a:产品拥有的写边界 launcher/helper(REQ-LCG-03/07)
//
// Linux 上以 bubblewrap 构建产品 owned 沙箱:只读 host + 冻结 target(原路径
// rw)+ 既有授权 git-dir(identity 链冻结)+ 自然 session/runtime 窄面 +
// 隔离 temp。保持网络(provider API/MCP 需要)与 root 原路径/既有配置发现,
// 不照搬 Kimi terminal 先例的 `--unshare-net` 与 cwd fd 改 `/tmp/work`。
// 缺 bwrap/user namespace 或 plan 非法一律失败关闭,绝不回退无隔离 spawn。
// ============================================================================

use std::collections::BTreeMap;
use std::ffi::OsString;

use crate::cross_cutting::process_manager::ProcessManager;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;

/// 可信二进制目录(launcher 自身解析 bwrap/git 用;子进程环境由调用方决定)。
const BOUNDARY_TRUSTED_PATH_DIRS: [&str; 5] =
    ["/usr/bin", "/bin", "/usr/local/bin", "/usr/sbin", "/sbin"];

/// 各 provider 自然 session/runtime 可写目录(HOME 下窄面;只列存在的)。
/// 用途(沿 2026-10-01 写边界实测):
/// - `.claude`、`.claude.json`:Claude Code 会话/项目状态;
/// - `.codex`:Codex sessions、projects trust、config;
/// - `.pi`:Pi agent 会话(`~/.pi/agent/sessions`);
/// - `.kimi-code`:Kimi sessions 与 workspace-trust。
///
/// 不得整 HOME、整 root 或 MCP 目录宽写:Aria 注入的 MCP 由 gateway 经
/// argv/env 控制并落在沙箱内;不可隔离的外部 MCP 写通道只能命中只读挂载面
/// 而被阻断,本集合永不为其开洞。
pub const PROVIDER_RUNTIME_WRITABLE_DIRS: [&str; 5] =
    [".claude", ".claude.json", ".codex", ".pi", ".kimi-code"];

/// 解析当前会话的自然 session/runtime 可写目录集合(窄面)。HOME 取
/// `env_vars`(gateway 冻结值)优先、进程环境兜底;缺 HOME 或目录不存在
/// 时返回更小集合——不预创建、不扩大写面。
pub fn provider_runtime_writable_roots(env_vars: &BTreeMap<String, String>) -> Vec<PathBuf> {
    let home = env_vars
        .get("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from));
    let Some(home) = home else {
        return Vec::new();
    };
    PROVIDER_RUNTIME_WRITABLE_DIRS
        .iter()
        .map(|dir| home.join(dir))
        .filter(|path| path.exists())
        .collect()
}

/// 产品写边界 launcher:持有已验证的 bwrap 路径;`None` = 本机不可用
/// (缺 bwrap 或 user namespace/mount 实测失败)。
#[derive(Debug, Clone)]
pub struct ProviderBoundaryLauncher {
    bwrap: Option<PathBuf>,
}

impl ProviderBoundaryLauncher {
    /// 探测环境:可信目录中的 bwrap + 真实最小沙箱运行验证 user
    /// namespace/mount 可用。任一步失败即不可用(不猜测、不放宽)。
    pub fn probe_environment() -> Self {
        Self {
            bwrap: probe_bwrap_with_namespace(),
        }
    }

    /// 注入式构造(测试/装配 seam;`None` 模拟缺 sandbox/namespace 机器)。
    #[cfg(test)]
    pub(crate) fn from_bwrap(bwrap: Option<PathBuf>) -> Self {
        Self { bwrap }
    }

    pub fn is_available(&self) -> bool {
        self.bwrap.is_some()
    }

    pub fn bwrap_path(&self) -> Option<&Path> {
        self.bwrap.as_deref()
    }

    /// 能力视角:launcher 自身永不签发 Confirmed/Denied——缺 bwrap/user
    /// namespace 的机器与「可用但未经 6c 真实正负探针」都返回 Unknown;
    /// 真实 Confirmed 只能由 probe evidence 经 2d 三方一致性导入。不可用
    /// 时 spawn 失败关闭,绝不 skip→pass 或退回无隔离 spawn。
    pub fn write_boundary_state(&self) -> ProviderCapabilityEvidence {
        ProviderCapabilityEvidence::Unknown
    }

    /// 构造 bwrap argv(不含 bwrap 程序自身)。挂载顺序是安全语义:
    /// `--ro-bind / /` 最早;`--tmpfs /tmp` 之后的所有 bind 恢复被 tmpfs
    /// 遮蔽的原路径可见性(root 原路径不变);后置只读挂载(段 2)再遮蔽
    /// rw 面内的受保护位置。
    pub(crate) fn build_boundary_argv(
        &self,
        command: &str,
        args: &[&str],
        working_dir: &Path,
        env_vars: &BTreeMap<String, String>,
        plan: &ProviderBoundaryPlan,
    ) -> Result<Vec<OsString>, ProviderBoundaryError> {
        validate_boundary_plan(plan)?;
        let mut argv = Vec::<OsString>::new();
        // 只读 host(root 原路径;一切未显式授权的写面默认拒绝)。
        for value in ["--ro-bind", "/", "/"] {
            argv.push(OsString::from(value));
        }
        for value in ["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"] {
            argv.push(OsString::from(value));
        }
        argv.push(OsString::from("--die-with-parent"));
        // root(进程 cwd)只读,原路径;后置于 tmpfs 以在 root 位于 /tmp 下时
        // 仍可见。
        push_mount(&mut argv, "--ro-bind", working_dir);
        // 冻结 target:Coding 唯一宽写 bind,原路径 rw(read-only 无 target)。
        if let Some(target) = plan.target_root() {
            push_mount(&mut argv, "--bind", target);
            // 既有授权 git-dir:linked worktree 的 git 元数据在 target 外,沿
            // identity 链冻结(plain repo `.git` 在 target 内,无额外 bind,
            // 即已交付授权,不新增整个 `.git` 宽写)。
            for git_path in frozen_git_dir_binds(target) {
                push_mount(&mut argv, "--bind", &git_path);
            }
        }
        // 自然 session/runtime 窄面(用途见 PROVIDER_RUNTIME_WRITABLE_DIRS)。
        for runtime_root in provider_runtime_writable_roots(env_vars) {
            push_mount(&mut argv, "--bind", &runtime_root);
        }
        // 后置只读挂载保护:target `.git` 指针(linked-worktree 文件形态)与
        // 所有 `.aria`(及 plan 受保护根)在 rw bind 之后重新遮蔽为只读
        // ——bwrap 按 argv 顺序应用挂载,后挂载遮蔽先挂载。
        for protected in protected_shadow_roots(plan) {
            push_mount(&mut argv, "--ro-bind", &protected);
        }
        // cwd 保持 root 原路径(不照搬 Kimi terminal 的 /tmp/work 改写)。
        argv.push(OsString::from("--chdir"));
        argv.push(working_dir.as_os_str().to_os_string());
        // payload:环境经继承 + ProcessManager overlay(不 --clearenv,
        // 保持既有配置发现)。
        argv.push(OsString::from(command));
        for arg in args {
            argv.push(OsString::from(*arg));
        }
        Ok(argv)
    }
}

/// 后置只读挂载保护集合(段 2):plan 受保护根 + 自动派生的 target 元数据
/// ——linked worktree 的 `.git` 指针(文件形态;plain repo 的 `.git` 目录
/// 沿 target 写面=既有授权,不遮蔽,Main 裁决不新增整个 `.git` 写授权)与
/// target `.aria`(root/成员 `.aria` 已由只读 host 覆盖)。仅保留存在的
/// 路径(bwrap bind 缺失源会失败),排序去重。
pub(crate) fn protected_shadow_roots(plan: &ProviderBoundaryPlan) -> Vec<PathBuf> {
    let mut shadows: Vec<PathBuf> = plan.protected_roots().to_vec();
    if let Some(target) = plan.target_root() {
        let git_pointer = target.join(".git");
        if std::fs::symlink_metadata(&git_pointer)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
        {
            shadows.push(git_pointer);
        }
        shadows.push(target.join(".aria"));
    }
    shadows.retain(|path| path.exists());
    shadows.sort();
    shadows.dedup();
    shadows
}

// ============================================================================
// Task 6a:写面探针(受控隔离 fixture 的探测通道;6c 真实 probe 与验收复用)
// ============================================================================

/// 写探针通道:覆盖 provider 进程自身与其后代写面(terminal/MCP/extension
/// 均为 provider 派生的子孙进程,天然落在同一 mount namespace 内)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryWriteChannel {
    /// provider 进程自身直接写(内建写工具面)。
    Builtin,
    /// terminal 子进程写(provider 起 shell 的通道)。
    Terminal,
    /// MCP 后代写(MCP server 进程链;不可隔离的外部 MCP 写通道同样只
    /// 命中只读挂载面而被阻断)。
    Mcp,
    /// extension 子进程写(如 Pi extension)。
    Extension,
}

/// 计划内写探针(通道 + 目标文件)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedBoundaryWrite {
    channel: BoundaryWriteChannel,
    path: PathBuf,
}

impl PlannedBoundaryWrite {
    pub fn new(channel: BoundaryWriteChannel, path: PathBuf) -> Self {
        Self { channel, path }
    }

    pub fn channel(&self) -> BoundaryWriteChannel {
        self.channel
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// 单次写探针结果:拒绝必须带证据(errno/shell 报文),未观测到证据的
/// 「拒绝」不可计入支持面。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryWriteAttempt {
    channel: BoundaryWriteChannel,
    path: PathBuf,
    refused: bool,
    evidence: String,
}

impl BoundaryWriteAttempt {
    fn allowed(channel: BoundaryWriteChannel, path: PathBuf) -> Self {
        Self {
            channel,
            path,
            refused: false,
            evidence: String::new(),
        }
    }

    fn refused_with(channel: BoundaryWriteChannel, path: PathBuf, evidence: String) -> Self {
        Self {
            channel,
            path,
            refused: true,
            evidence,
        }
    }

    /// 拒绝且带证据(可审计):`was_refused_with_evidence` 是写边界验收的
    /// 唯一「拒绝」口径。
    pub fn was_refused_with_evidence(&self) -> bool {
        self.refused && !self.evidence.trim().is_empty()
    }

    /// 写结果:Ok(())= 写成功并真实落盘;Err(evidence)= 被拒并带证据。
    pub fn result(&self) -> Result<(), String> {
        if self.refused {
            Err(self.evidence.clone())
        } else {
            Ok(())
        }
    }

    pub fn channel(&self) -> BoundaryWriteChannel {
        self.channel
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn evidence(&self) -> &str {
        &self.evidence
    }
}

/// shell 单引号转义(探针路径/脚本片段嵌入用)。
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// 生成指定通道的一次写命令(嵌入探针脚本;路径/脚本经单引号转义)。
fn probe_write_command(channel: BoundaryWriteChannel, quoted_path: &str) -> String {
    let builtin = r#"printf aria-boundary-probe >"$1""#;
    match channel {
        BoundaryWriteChannel::Builtin => {
            format!("printf aria-boundary-probe >{quoted_path}")
        }
        BoundaryWriteChannel::Terminal => {
            format!("sh -c {} aria-terminal {quoted_path}", shell_quote(builtin))
        }
        BoundaryWriteChannel::Extension => format!(
            "sh -c {} aria-extension {quoted_path}",
            shell_quote(builtin)
        ),
        // MCP 通道 = 两层后代:外层 shell 再起内层 MCP server 代写。
        BoundaryWriteChannel::Mcp => {
            let inner = format!("sh -c {} aria-mcp-inner \"$1\"", shell_quote(builtin));
            format!("sh -c {} aria-mcp {quoted_path}", shell_quote(&inner))
        }
    }
}

/// 内置通道写探针(provider 进程自身直接写):在真实产品写边界沙箱内逐
/// 路径尝试写入并结构化回报结果。进程退出非 0(如 bwrap 无法建
/// namespace/mount)直接报错——不把「沙箱起不来」当拒绝或成功。
pub async fn run_builtin_write_probe(
    launcher: &ProviderBoundaryLauncher,
    plan: &ProviderBoundaryPlan,
    env_vars: &BTreeMap<String, String>,
    paths: &[PathBuf],
) -> Result<Vec<BoundaryWriteAttempt>, ProviderAdapterError> {
    let attempts: Vec<PlannedBoundaryWrite> = paths
        .iter()
        .map(|path| PlannedBoundaryWrite::new(BoundaryWriteChannel::Builtin, path.clone()))
        .collect();
    run_write_surface_probe(launcher, plan, env_vars, &attempts).await
}

/// 多通道写面探针(段 3):在真实产品写边界沙箱内,以 provider 自身
/// (builtin)与其后代(terminal/MCP/extension)通道逐路径尝试写入并
/// 结构化回报;后代进程与 provider 共享同一 mount namespace,任何通道
/// 的越界写都必须被只读挂载拒绝并带证据。
pub async fn run_write_surface_probe(
    launcher: &ProviderBoundaryLauncher,
    plan: &ProviderBoundaryPlan,
    env_vars: &BTreeMap<String, String>,
    attempts: &[PlannedBoundaryWrite],
) -> Result<Vec<BoundaryWriteAttempt>, ProviderAdapterError> {
    let mut script = String::new();
    for (index, planned) in attempts.iter().enumerate() {
        let quoted = shell_quote(&planned.path.to_string_lossy());
        let write = probe_write_command(planned.channel, &quoted);
        let error_file = format!("/tmp/.aria-boundary-probe-{index}.err");
        script.push_str(&format!(
            "if ( {write} ) 2>{error_file}; then printf 'A {index} ok\\n'; \
             else printf 'A {index} refused %s\\n' \"$(tr '\\n' ' ' <{error_file} | cut -c1-160)\"; fi\n"
        ));
    }
    run_probe_script(launcher, plan, env_vars, &script, attempts).await
}

/// 在写边界沙箱内执行探针脚本并解析 `A <i> ok|refused <evidence>` 哨兵。
async fn run_probe_script(
    launcher: &ProviderBoundaryLauncher,
    plan: &ProviderBoundaryPlan,
    env_vars: &BTreeMap<String, String>,
    script: &str,
    planned: &[PlannedBoundaryWrite],
) -> Result<Vec<BoundaryWriteAttempt>, ProviderAdapterError> {
    use tokio::io::AsyncReadExt;
    use tokio_util::sync::CancellationToken;

    let process = ProcessManager::spawn_with_boundary_resolved(
        launcher.clone(),
        "sh",
        &["-c", script],
        plan.working_directory(),
        env_vars,
        plan,
        CancellationToken::new(),
    )
    .await?;
    drop(process.stdin);
    let (mut stdout, mut stderr) = (process.stdout, process.stderr);
    let mut child = process.child;
    let (status, stdout_text, mut stderr_text) =
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            let mut stdout_text = String::new();
            let mut stderr_text = String::new();
            stdout.read_to_string(&mut stdout_text).await.ok();
            stderr.read_to_string(&mut stderr_text).await.ok();
            let status = child.wait().await;
            (status, stdout_text, stderr_text)
        })
        .await
        .map_err(|_| ProviderAdapterError::timeout(String::new(), String::new(), 60_000))?;
    let status = status.map_err(|error| {
        ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
    })?;
    if !status.success() {
        // 沙箱自身失败(bwrap/namespace/mount):显式报错,不产生 attempt。
        stderr_text.truncate(400);
        return Err(ProviderAdapterError::execution_failed(
            status.code(),
            stdout_text,
            stderr_text,
            0,
        ));
    }
    let mut attempts = Vec::with_capacity(planned.len());
    for (index, planned) in planned.iter().enumerate() {
        let sentinel = stdout_text
            .lines()
            .find_map(|line| line.strip_prefix(&format!("A {index} ")));
        let attempt = match sentinel {
            Some(rest) if rest.starts_with("ok") => {
                BoundaryWriteAttempt::allowed(planned.channel, planned.path.clone())
            }
            Some(rest) => BoundaryWriteAttempt::refused_with(
                planned.channel,
                planned.path.clone(),
                rest.trim_start_matches("refused").trim().to_string(),
            ),
            None => BoundaryWriteAttempt::refused_with(
                planned.channel,
                planned.path.clone(),
                "probe sentinel missing (write unobserved)".to_string(),
            ),
        };
        attempts.push(attempt);
    }
    Ok(attempts)
}
fn push_mount(argv: &mut Vec<OsString>, flag: &str, source: &Path) {
    // 非 UTF-8 路径不做 panic:argv 本就是 OsString,挂载源原样直传
    // (flag/dest 与源同值)。
    argv.push(OsString::from(flag));
    argv.push(source.as_os_str().to_os_string());
    argv.push(source.as_os_str().to_os_string());
}

/// plan 形状校验:mode 与 target 一致、路径绝对且存在、cwd 与 target 分离。
fn validate_boundary_plan(plan: &ProviderBoundaryPlan) -> Result<(), ProviderBoundaryError> {
    let invalid = |details: &str| Err(ProviderBoundaryError::InvalidPlan(details.to_string()));
    if !plan.working_directory().is_absolute() || !plan.working_directory().is_dir() {
        return invalid("working directory must be an absolute existing directory");
    }
    if plan
        .protected_roots()
        .iter()
        .any(|root| !root.is_absolute())
    {
        return invalid("protected roots must be absolute");
    }
    match plan.mode() {
        ProviderBoundaryMode::ReadOnly => {
            if plan.target_root().is_some() {
                return invalid("read-only plan must not carry a writable target");
            }
        }
        ProviderBoundaryMode::TargetWriteOnly => {
            let Some(target) = plan.target_root() else {
                return invalid("target-write-only plan requires a target root");
            };
            if !target.is_absolute() || !target.is_dir() {
                return invalid("target root must be an absolute existing directory");
            }
            if target == plan.working_directory() {
                return invalid("target must stay independent of the process cwd");
            }
        }
    }
    Ok(())
}

/// bwrap 探测:可信目录定位 + `--version` + 真实最小沙箱(`ro-bind / /` 下
/// 执行 `/bin/true`)验证 user namespace/mount。失败返回 None(不可用)。
fn probe_bwrap_with_namespace() -> Option<PathBuf> {
    let candidate = BOUNDARY_TRUSTED_PATH_DIRS
        .iter()
        .map(|dir| Path::new(dir).join("bwrap"))
        .find(|candidate| candidate.is_file())?;
    let version = std::process::Command::new(&candidate)
        .arg("--version")
        .output()
        .ok()?;
    if !version.status.success() {
        return None;
    }
    let namespace_check = std::process::Command::new(&candidate)
        .args([
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--",
            "/bin/true",
        ])
        .output()
        .ok()?;
    if !namespace_check.status.success() {
        return None;
    }
    Some(candidate)
}

/// git-dir 授权链冻结:与 Kimi client services 共用同一冻结面(`.provider-
/// session-cache/<key>/writable_git_paths.json`,host 领土)。linked
/// worktree 解析 git/common dir 并做 round-trip 校验(`<gitdir>/gitdir` 指回
/// 字面 `<target>/.git`),plain repo 返回空(target 内 `.git` 即既有授权);
/// 校验失败安全降级为空。首解析持久化,后续轮次信任冻结面——coder 改写
/// `.git` 指针不能把下一轮的 rw bind 指向任意 host git dir。
pub(crate) fn frozen_git_dir_binds(target: &Path) -> Vec<PathBuf> {
    let cache_dir = target
        .parent()
        .unwrap_or(target)
        .join(".provider-session-cache")
        .join(provider_cache_key(target));
    let cache_file = cache_dir.join("writable_git_paths.json");
    if let Ok(content) = std::fs::read_to_string(&cache_file)
        && let Ok(paths) = serde_json::from_str::<Vec<String>>(&content)
    {
        return paths.into_iter().map(PathBuf::from).collect();
    }
    let resolved = resolve_writable_git_dir_binds(target);
    let persisted: Vec<String> = resolved
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    if std::fs::create_dir_all(&cache_dir).is_ok()
        && let Ok(json) = serde_json::to_string(&persisted)
    {
        let _ = std::fs::write(&cache_file, json);
    }
    resolved
}

fn resolve_writable_git_dir_binds(target: &Path) -> Vec<PathBuf> {
    let Some(git) = BOUNDARY_TRUSTED_PATH_DIRS
        .iter()
        .map(|dir| Path::new(dir).join("git"))
        .find(|candidate| candidate.is_file())
    else {
        return Vec::new();
    };
    let Ok(output) = std::process::Command::new(git)
        .arg("-C")
        .arg(target)
        .args(["rev-parse", "--absolute-git-dir", "--git-common-dir"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let mut dirs: Vec<PathBuf> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let path = Path::new(line);
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                target.join(path)
            };
            absolute.canonicalize().unwrap_or(absolute)
        })
        .collect();
    if dirs.len() != 2 {
        return Vec::new();
    }
    let common = dirs.pop().expect("common dir");
    let gitdir = dirs.pop().expect("git dir");
    if gitdir.starts_with(target) {
        // plain repo(或 gitdir 在 target 内的链):target rw bind 已覆盖。
        return Vec::new();
    }
    // round-trip 校验:合法 linked worktree 的 `<gitdir>/gitdir` 指回字面
    // `<canonical-target>/.git`;期望侧不做 `.git` 穿透 canonicalize。
    let points_back = std::fs::read_to_string(gitdir.join("gitdir"))
        .ok()
        .and_then(|content| {
            let back = PathBuf::from(content.trim());
            let back = if back.is_absolute() {
                back
            } else {
                gitdir.join(back)
            };
            std::fs::canonicalize(&back)
                .ok()
                .zip(
                    std::fs::canonicalize(target)
                        .map(|canonical| canonical.join(".git"))
                        .ok(),
                )
                .map(|(back, expected)| back == expected)
        })
        .unwrap_or(false);
    if !points_back {
        return Vec::new();
    }
    let mut outside: Vec<PathBuf> = [gitdir, common]
        .into_iter()
        .filter(|path| !path.starts_with(target))
        .collect();
    outside.sort();
    outside.dedup();
    // git dir 位于 common dir 之下:common bind 一个即覆盖两者。
    outside
        .iter()
        .filter(|path| {
            !outside
                .iter()
                .any(|other| other != *path && path.starts_with(other))
        })
        .cloned()
        .collect()
}

/// 冻结面缓存子目录 key(worktree 路径的跨进程稳定 FNV-1a 64 位,与 Kimi
/// 先例同算法:同一 target 的产品 launcher 与 Kimi terminal 共享一个冻结面)。
fn provider_cache_key(root: &Path) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in root.as_os_str().as_encoded_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DTO 形状锁:构造字段与 getter 往返一致(read-only 无 target)。
    #[test]
    fn provider_boundary_plan_round_trips_read_only_shape() {
        let plan = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::ReadOnly,
            PathBuf::from("/lc-root"),
            None,
            vec![
                PathBuf::from("/lc-root/.git"),
                PathBuf::from("/lc-root/.aria"),
            ],
        );
        assert_eq!(plan.mode(), ProviderBoundaryMode::ReadOnly);
        assert_eq!(plan.working_directory(), std::path::Path::new("/lc-root"));
        assert_eq!(plan.target_root(), None);
        assert_eq!(plan.protected_roots().len(), 2);
    }

    /// target-only 形状:唯一 target 可写,cwd 与 target 分离。
    #[test]
    fn provider_boundary_plan_round_trips_target_write_only_shape() {
        let plan = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::TargetWriteOnly,
            PathBuf::from("/lc-root"),
            Some(PathBuf::from("/work/api/.worktrees/issue_1")),
            vec![],
        );
        assert_eq!(plan.mode(), ProviderBoundaryMode::TargetWriteOnly);
        assert_eq!(
            plan.target_root(),
            Some(std::path::Path::new("/work/api/.worktrees/issue_1"))
        );
    }

    /// evidence getter 往返(2d 三方一致性校验消费的固定字段)。
    #[test]
    fn provider_boundary_evidence_exposes_probe_fields() {
        let evidence = ProviderBoundaryEvidence::new(
            ProviderName::ClaudeCode,
            "2.1.0".to_string(),
            ProviderBoundaryMode::TargetWriteOnly,
            "sha256:abc".to_string(),
            "probe://run/1".to_string(),
            "2026-10-03T00:00:00Z".to_string(),
        );
        assert_eq!(evidence.provider(), &ProviderName::ClaudeCode);
        assert_eq!(evidence.exact_version(), "2.1.0");
        assert_eq!(
            evidence.boundary_mode(),
            ProviderBoundaryMode::TargetWriteOnly
        );
        assert_eq!(evidence.projection_digest(), "sha256:abc");
        assert_eq!(evidence.artifact_ref(), "probe://run/1");
        assert_eq!(evidence.probed_at(), "2026-10-03T00:00:00Z");
    }
}

#[cfg(test)]
mod launcher_tests {
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::{
        ProviderBoundaryLauncher, ProviderBoundaryMode, ProviderBoundaryPlan, frozen_git_dir_binds,
        protected_shadow_roots, provider_runtime_writable_roots,
    };
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;

    fn argv_text(argv: &[OsString]) -> Vec<String> {
        argv.iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect()
    }

    fn coding_plan(root: &Path, target: &Path) -> ProviderBoundaryPlan {
        ProviderBoundaryPlan::new(
            ProviderBoundaryMode::TargetWriteOnly,
            root.to_path_buf(),
            Some(target.to_path_buf()),
            Vec::new(),
        )
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("run git fixture");
        assert!(status.success(), "git fixture {args:?} failed");
    }

    /// launcher 核心:只读 host、冻结 target 原路径、保持网络与既有配置发现,
    /// 隔离 temp;不照搬 Kimi terminal 的 --unshare-net 与 /tmp/work cwd 改写。
    #[test]
    fn lcg_t06_boundary_argv_keeps_network_root_paths_and_readonly_host() {
        let base = tempdir().expect("base dir");
        let root = base.path().join("lc-root");
        let target = base.path().join("target");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::create_dir_all(&target).expect("target");
        let plan = coding_plan(&root, &target);
        let launcher = ProviderBoundaryLauncher::from_bwrap(Some(PathBuf::from("/usr/sbin/bwrap")));
        // 显式空 HOME:runtime 集合为空,target 才是唯一 rw bind(不依赖
        // 测试进程的真实 HOME)。
        let mut env = BTreeMap::new();
        env.insert(
            "HOME".to_string(),
            base.path()
                .join("empty-home")
                .to_string_lossy()
                .into_owned(),
        );

        let argv = launcher
            .build_boundary_argv("sh", &["-c", "true"], &root, &env, &plan)
            .expect("boundary argv");
        let text = argv_text(&argv);

        // 保持网络:provider API/MCP 需要,不使用 --unshare-net。
        assert!(!text.iter().any(|arg| arg == "--unshare-net"));
        // 只读 host 必须最早挂载(bwrap argv 顺序语义,后续 bind 才能遮蔽)。
        assert_eq!(
            &text[0..3],
            ["--ro-bind".to_string(), "/".to_string(), "/".to_string()]
        );
        // 冻结 target 以原路径 rw 挂载,且是唯一 rw bind。
        let target_text = target.to_string_lossy().into_owned();
        assert!(text.windows(3).any(|window| {
            window
                == [
                    "--bind".to_string(),
                    target_text.clone(),
                    target_text.clone(),
                ]
        }));
        assert_eq!(
            text.iter().filter(|arg| *arg == "--bind").count(),
            1,
            "target 是 Coding 唯一宽写 bind:{text:?}"
        );
        // cwd 保持 root 原路径:不照搬 Kimi terminal 的 /tmp/work cwd 改写。
        assert!(!text.iter().any(|arg| arg.contains("/tmp/work")));
        let root_text = root.to_string_lossy().into_owned();
        assert!(
            text.windows(2)
                .any(|window| window == ["--chdir".to_string(), root_text.clone()])
        );
        // 既有配置发现保持:不 --clearenv,环境经继承+overlay(非 bwrap --setenv)。
        assert!(!text.iter().any(|arg| arg == "--clearenv"));
        assert!(!text.iter().any(|arg| arg == "--setenv"));
        // 隔离 temp:每沙箱私有 /tmp。
        assert!(
            text.windows(2)
                .any(|window| window == ["--tmpfs".to_string(), "/tmp".to_string()])
        );
        // payload 命令与参数原样收尾。
        let tail: Vec<String> = text[text.len() - 3..].to_vec();
        assert_eq!(
            tail,
            ["sh".to_string(), "-c".to_string(), "true".to_string()]
        );
    }

    /// read-only action:无任何 rw bind,root/成员/元数据全部落在只读挂载面。
    #[test]
    fn lcg_t06_boundary_readonly_argv_has_no_writable_binds() {
        let base = tempdir().expect("base dir");
        let root = base.path().join("lc-root");
        std::fs::create_dir_all(&root).expect("root");
        let plan = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::ReadOnly,
            root.clone(),
            None,
            Vec::new(),
        );
        let launcher = ProviderBoundaryLauncher::from_bwrap(Some(PathBuf::from("/usr/sbin/bwrap")));
        // 显式空 HOME:runtime 集合为空,read-only 面 zero rw bind。
        let mut env = BTreeMap::new();
        env.insert(
            "HOME".to_string(),
            base.path()
                .join("empty-home")
                .to_string_lossy()
                .into_owned(),
        );

        let argv = launcher
            .build_boundary_argv("sh", &["-c", "true"], &root, &env, &plan)
            .expect("readonly boundary argv");
        let text = argv_text(&argv);

        assert!(!text.iter().any(|arg| arg == "--bind"));
        let root_text = root.to_string_lossy().into_owned();
        assert!(text.windows(3).any(|window| {
            window
                == [
                    "--ro-bind".to_string(),
                    root_text.clone(),
                    root_text.clone(),
                ]
        }));
    }

    /// 各 provider 自然 session/runtime 可写集合:HOME 下窄面目录,用途固定;
    /// 不得整 HOME、整 root 或 MCP 目录宽写。
    #[test]
    fn lcg_t06_provider_runtime_writable_roots_stay_narrow() {
        let base = tempdir().expect("base dir");
        let home = base.path().join("home");
        for dir in [".claude", ".codex", ".pi", ".kimi-code", ".config"] {
            std::fs::create_dir_all(home.join(dir)).expect("runtime dir");
        }
        std::fs::write(home.join(".claude.json"), "{}").expect("claude state file");
        let mut env = BTreeMap::new();
        env.insert("HOME".to_string(), home.to_string_lossy().into_owned());

        let roots = provider_runtime_writable_roots(&env);
        let expected: Vec<PathBuf> = [".claude", ".claude.json", ".codex", ".pi", ".kimi-code"]
            .iter()
            .map(|dir| home.join(dir))
            .collect();
        assert_eq!(roots, expected);
        assert!(!roots.iter().any(|path| path == &home));
        assert!(!roots.iter().any(|path| path.ends_with(".config")));

        // 不存在的自然目录不预创建、不扩大写面。
        std::fs::remove_dir_all(home.join(".pi")).expect("remove .pi");
        let narrowed = provider_runtime_writable_roots(&env);
        assert!(!narrowed.iter().any(|path| path.ends_with(".pi")));
    }

    /// git identity 授权链:plain repo 不加额外 bind;linked worktree 冻结
    /// 常见 git 目录(bind 面=common dir 祖先),冻结面持久化在 host 领土,
    /// 后续轮次即使 `.git` 指针被改写也信任冻结结果。
    #[test]
    fn lcg_t06_frozen_git_dir_binds_follow_identity_chain() {
        let base = tempdir().expect("base dir");

        let plain = base.path().join("plain");
        std::fs::create_dir_all(&plain).expect("plain repo");
        git(&plain, &["init", "-q"]);
        git(
            &plain,
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
        assert_eq!(frozen_git_dir_binds(&plain), Vec::<PathBuf>::new());

        let repo = base.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
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
        let target = base.path().join("target-wt");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                target.to_string_lossy().as_ref(),
                "-b",
                "probe-wt",
            ],
        );

        let binds = frozen_git_dir_binds(&target);
        let common = std::fs::canonicalize(repo.join(".git")).expect("canonical git common");
        assert_eq!(binds, vec![common]);

        // 冻结面持久化在 target 父目录 host 领土(.provider-session-cache)。
        let cache_root = target
            .parent()
            .expect("target parent")
            .join(".provider-session-cache");
        let persisted = std::fs::read_dir(cache_root)
            .expect("provider session cache")
            .filter_map(|entry| entry.ok())
            .any(|entry| entry.path().join("writable_git_paths.json").exists());
        assert!(
            persisted,
            "frozen git face must persist outside the writable target"
        );

        // 指针被改写后(round N+1)仍信任冻结面,不重新解析到伪造 git dir。
        std::fs::write(target.join(".git"), "gitdir: /definitely/not/real\n")
            .expect("swap pointer");
        let refrozen = frozen_git_dir_binds(&target);
        assert_eq!(
            refrozen, binds,
            "frozen face must survive a coder-swapped .git pointer"
        );
    }

    /// 后置只读挂载保护派生:linked worktree 的 target `.git` 指针(文件
    /// 形态)与 target `.aria` 被遮蔽;plain repo 的 `.git` 目录不遮蔽(沿
    /// target 写面=既有授权,Main 裁决不新增整个 `.git` 写授权);plan
    /// 受保护根并入;不存在的路径过滤,去重排序。
    #[test]
    fn lcg_t06_protected_shadow_roots_cover_git_pointer_and_aria() {
        let base = tempdir().expect("base dir");

        // linked worktree target:`.git` 是文件指针 → 遮蔽;`.aria` 遮蔽。
        let repo = base.path().join("repo");
        std::fs::create_dir_all(&repo).expect("repo");
        let linked = base.path().join("linked-wt");
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["init", "-q"])
            .status()
            .expect("git init");
        assert!(status.success());
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "-c",
                "user.name=aria",
                "-c",
                "user.email=aria@aria",
                "commit",
                "--allow-empty",
                "-m",
                "init",
            ])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("git commit");
        assert!(status.success());
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "worktree",
                "add",
                "-q",
                linked.to_string_lossy().as_ref(),
                "-b",
                "shadow-wt",
            ])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("git worktree add");
        assert!(status.success());
        std::fs::create_dir_all(linked.join(".aria")).expect("linked aria");
        let root = base.path().join("lc-root");
        std::fs::create_dir_all(&root).expect("root");
        let declared = root.join(".aria");
        std::fs::create_dir_all(&declared).expect("declared protected root");
        let linked_plan = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::TargetWriteOnly,
            root.clone(),
            Some(linked.clone()),
            vec![declared.clone()],
        );
        let shadows = protected_shadow_roots(&linked_plan);
        assert!(shadows.contains(&linked.join(".git")));
        assert!(shadows.contains(&linked.join(".aria")));
        assert!(shadows.contains(&declared));

        // plain repo target:`.git` 目录不进遮蔽面(沿 target 既有授权)。
        let plain = base.path().join("plain");
        std::fs::create_dir_all(&plain).expect("plain");
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&plain)
            .args(["init", "-q"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("plain git init");
        assert!(status.success());
        std::fs::create_dir_all(plain.join(".aria")).expect("plain aria");
        let plain_plan = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::TargetWriteOnly,
            root,
            Some(plain.clone()),
            Vec::new(),
        );
        let plain_shadows = protected_shadow_roots(&plain_plan);
        assert!(plain_shadows.contains(&plain.join(".aria")));
        assert!(!plain_shadows.contains(&plain.join(".git")));

        // 不存在的受保护根过滤,不进 bwrap bind(缺失源会失败)。
        assert!(!plain_shadows.contains(&plain.join(".aria").join("missing")));
    }

    /// 能力视角:缺 bwrap/user namespace 与「可用但未经真实 probe」都保持
    /// Unknown;launcher 自身永不签发 Confirmed(6c 真实正负探针才可)。
    #[test]
    fn lcg_t06_boundary_state_stays_unknown_with_or_without_sandbox() {
        let unavailable = ProviderBoundaryLauncher::from_bwrap(None);
        assert!(!unavailable.is_available());
        assert_eq!(
            unavailable.write_boundary_state(),
            ProviderCapabilityEvidence::Unknown
        );

        let available =
            ProviderBoundaryLauncher::from_bwrap(Some(PathBuf::from("/usr/sbin/bwrap")));
        assert!(available.is_available());
        assert_eq!(
            available.write_boundary_state(),
            ProviderCapabilityEvidence::Unknown
        );
    }
}
