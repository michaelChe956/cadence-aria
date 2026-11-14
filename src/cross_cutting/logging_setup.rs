//! `aria web` 服务形态的生产 tracing subscriber 初始化。
//!
//! 为什么必须有这一层：`tracing` 的 `info!` / `warn!` 在**没有全局
//! subscriber 时是 no-op**——没有它，安装作业决策链、聚合索引覆盖判定这类
//! 结构化诊断日志在客户机上根本不会落盘，现场只能靠复现。
//!
//! 口径（计划文档 §「客户机定位指引」同源）：
//! - 过滤器：默认 `warn`（全局），只把本轮诊断相关模块
//!   （`provider_installation` / `aggregate_index`）打开到 `info`；
//!   **`RUST_LOG` 存在且非空时完全取代默认口径**（客户机现场第一开关）。
//! - 输出：**同时**写 stderr 与追加文件 `<状态根>/logs/aria.log`；状态根
//!   与 `ProductAppPaths` 一致（`<workspace>/.aria`）。目录不存在则创建。
//! - 文件不可写 ⇒ 降级为只写 stderr 并打一行 `[aria-logging] …` 告警，
//!   **绝不让服务起不来**。
//! - 不泄密：本模块只读 `RUST_LOG`，不 dump 环境变量、不打印任何凭据；
//!   日志正文的脱敏由各打点处负责。
//! - 幂等：进程内只装一次（`set_global_default` 本身也只接受一次），重复
//!   调用返回 [`TracingInit::AlreadyInstalled`]，不重复挂 subscriber。

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::Dispatch;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;

/// 日志目录名（`<状态根>/logs`）。
pub const LOG_DIR: &str = "logs";

/// 日志文件名（追加写入，无轮转）。
pub const LOG_FILE_NAME: &str = "aria.log";

/// 客户机覆盖开关（存在且非空时取代默认口径）。
pub const RUST_LOG_ENV: &str = "RUST_LOG";

/// 默认过滤口径：全局 `warn` + 本轮两个诊断模块 `info`。
pub const DEFAULT_FILTER_DIRECTIVES: &str = "warn,\
cadence_aria::product::provider_installation=info,\
cadence_aria::product::logical_codebase::aggregate_index=info";

/// 日志落点实际情况。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogSink {
    /// stderr + 追加文件（正常路径）。
    FileAndStderr { log_file: PathBuf },
    /// 文件打不开 ⇒ 只写 stderr；`reason` 供启动告警与现场排查使用。
    StderrOnly { log_file: PathBuf, reason: String },
}

impl LogSink {
    /// 本 sink 对应的日志文件路径（降级时即「本想写但写不了」的路径）。
    pub fn log_file(&self) -> &Path {
        match self {
            Self::FileAndStderr { log_file } | Self::StderrOnly { log_file, .. } => log_file,
        }
    }
}

/// [`init_web_tracing`] 的结果（幂等判定依据）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TracingInit {
    /// 本次调用完成安装。
    Installed {
        sink: LogSink,
        /// `true` ⇒ 过滤器来自 `RUST_LOG`（非默认口径）。
        rust_log_override: bool,
    },
    /// 早前已安装（或被进程内其他 subscriber 占用），本次未改动。
    AlreadyInstalled,
}

/// 日志文件路径：`<状态根>/logs/aria.log`。
pub fn log_file_path(state_root: &Path) -> PathBuf {
    state_root.join(LOG_DIR).join(LOG_FILE_NAME)
}

/// 进程级安装闸门：只有首次 `claim` 返回 `true`。测试可注入独立实例。
#[derive(Debug, Default)]
pub(crate) struct InstallGate {
    claimed: AtomicBool,
}

impl InstallGate {
    pub(crate) const fn new() -> Self {
        Self {
            claimed: AtomicBool::new(false),
        }
    }

    /// `true` = 本次调用取得安装权（进程内首次）。
    pub(crate) fn claim(&self) -> bool {
        !self.claimed.swap(true, Ordering::SeqCst)
    }
}

/// 构建过滤器：`RUST_LOG` 非空时以其为准，否则用 [`DEFAULT_FILTER_DIRECTIVES`]。
///
/// 非法指令按 `parse_lossy` 丢弃（**不得**因客户写错 `RUST_LOG` 让服务起不来）。
pub(crate) fn build_filter(rust_log: Option<&str>) -> EnvFilter {
    let directives = rust_log
        .map(str::trim)
        .filter(|directives| !directives.is_empty())
        .unwrap_or(DEFAULT_FILTER_DIRECTIVES);
    EnvFilter::builder()
        .with_default_directive(LevelFilter::ERROR.into())
        .parse_lossy(directives)
}

fn env_rust_log() -> Option<String> {
    std::env::var_os(RUST_LOG_ENV).map(|raw| raw.to_string_lossy().into_owned())
}

fn open_log_file(log_file: &Path) -> io::Result<File> {
    if let Some(parent) = log_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    OpenOptions::new().create(true).append(true).open(log_file)
}

/// 构建（不安装）subscriber：stderr 一层 + 文件一层，共用同一个过滤器。
pub(crate) fn build_dispatch(state_root: &Path, filter: EnvFilter) -> (Dispatch, LogSink) {
    let log_file = log_file_path(state_root);
    match open_log_file(&log_file) {
        Ok(file) => {
            let subscriber = tracing_subscriber::registry()
                .with(filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(std::io::stderr),
                )
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(Mutex::new(file)),
                );
            (
                Dispatch::new(subscriber),
                LogSink::FileAndStderr { log_file },
            )
        }
        Err(error) => {
            // 降级路径同样是完整可用的 subscriber：只少了文件这一层。
            let subscriber = tracing_subscriber::registry().with(filter).with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(std::io::stderr),
            );
            (
                Dispatch::new(subscriber),
                LogSink::StderrOnly {
                    log_file,
                    reason: error.to_string(),
                },
            )
        }
    }
}

/// 幂等初始化核心：闸门 + 可注入的安装动作（测试不碰进程全局 subscriber）。
pub(crate) fn init_with(
    state_root: &Path,
    gate: &InstallGate,
    install: impl FnOnce(Dispatch) -> bool,
) -> TracingInit {
    if !gate.claim() {
        return TracingInit::AlreadyInstalled;
    }
    let rust_log = env_rust_log();
    let rust_log_override = rust_log
        .as_deref()
        .map(str::trim)
        .is_some_and(|directives| !directives.is_empty());
    let (dispatch, sink) = build_dispatch(state_root, build_filter(rust_log.as_deref()));
    if let LogSink::StderrOnly { log_file, reason } = &sink {
        eprintln!(
            "[aria-logging] cannot write {}: {reason}; continuing with stderr only",
            log_file.display()
        );
    }
    if !install(dispatch) {
        return TracingInit::AlreadyInstalled;
    }
    eprintln!(
        "[aria-logging] log_file={} filter={}",
        sink.log_file().display(),
        if rust_log_override {
            RUST_LOG_ENV
        } else {
            "default"
        }
    );
    TracingInit::Installed {
        sink,
        rust_log_override,
    }
}

/// `aria web` 启动路径入口：把 subscriber 装到进程全局（幂等）。
///
/// 调用点：`crate::web::app::serve_web` 的最前一步——早于一切启动动作，
/// 使早期失败也有日志；幂等与降级语义见模块文档。
pub fn init_web_tracing(state_root: &Path) -> TracingInit {
    // 测试二进制是共享进程：`cross_cutting::tracing_capture` 的捕获 fixture
    // 依赖「进程级全局 subscriber 槽位无竞争者」，而 `serve_web` 也被 web 用例
    // 直接调用（`startup_window_compensation`）——在此抢装全局 subscriber 会
    // 让捕获为空、造成跨用例串扰（断言假红）。生产构建 `cfg!(test)` 恒为
    // false，真实安装逻辑不变；构建/安装本身由 `build_dispatch` /
    // `init_with` 的用例覆盖。
    if cfg!(test) {
        return TracingInit::AlreadyInstalled;
    }
    static GATE: InstallGate = InstallGate::new();
    init_with(state_root, &GATE, |dispatch| {
        tracing::dispatcher::set_global_default(dispatch).is_ok()
    })
}

#[cfg(test)]
include!("logging_setup_tests.inc.rs");
