//! 阶段 1 止血门禁：入口手写 Tokio runtime（worker 栈 8 MiB）且 CLI 语义逐字不变。
//!
//! 背景：provider run 崩溃链（深 inbound 链 ~1.5 MiB + 评审链 poll 帧 ~0.7 MiB）
//! 实测 ≈1.9 MiB，超过 tokio 默认 worker 栈 2 MiB；本门禁固化 owner 裁决：
//! dev / release 同档 `WORKER_STACK_SIZE = 8 * 1024 * 1024`，不依赖部署环境变量。

use std::path::PathBuf;
use std::process::Command;

use tempfile::tempdir;

/// 读取产品入口 src/main.rs 的源码（供结构断言）。
fn main_rs_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
    std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("读取 src/main.rs 失败: {error}"))
}

/// 真实 aria 制品路径（cargo 为集成测试预构建的 bin target）。
fn aria_binary() -> PathBuf {
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_aria"));
    assert!(
        binary.is_file(),
        "CARGO_BIN_EXE_aria 必须指向真实 aria 制品: {}",
        binary.display()
    );
    binary
}

/// 断言：入口不再依赖 tokio::main 宏，而是显式手写多线程 Builder；
/// worker 栈固定 8 MiB（8_388_608，dev/release 不分档）；
/// worker 数、线程名、blocking 上限保持 tokio 默认（未被覆写）。
#[test]
fn provider_run_runtime_builder_is_explicit() {
    let source = main_rs_source();

    assert!(
        !source.contains("#[tokio::main]"),
        "入口必须手写 runtime Builder，不得再用 tokio::main 宏（该宏无栈选项）"
    );
    assert!(
        source.contains("const WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;"),
        "必须定义 WORKER_STACK_SIZE 常量且取值恰为 8 MiB（8_388_608）"
    );
    assert!(
        source.contains("new_multi_thread()"),
        "必须显式构建多线程 runtime（new_multi_thread）"
    );
    assert!(
        source.contains(".thread_stack_size(WORKER_STACK_SIZE)"),
        "必须将 WORKER_STACK_SIZE 传入 thread_stack_size"
    );
    assert!(
        source.contains(".enable_all()"),
        "必须调用 enable_all（IO + 时间驱动，与原宏展开一致）"
    );

    // 原 tokio::main 用默认值：worker 数 = CPU 数、线程名 tokio-runtime-worker、
    // blocking 上限 512。保持默认即「未改」，入口不得覆写这三项。
    assert!(
        !source.contains(".worker_threads("),
        "worker 数必须保持 tokio 默认（不得覆写 worker_threads）"
    );
    assert!(
        !source.contains(".thread_name("),
        "线程名必须保持 tokio 默认（不得覆写 thread_name）"
    );
    assert!(
        !source.contains(".max_blocking_threads("),
        "blocking 线程上限必须保持 tokio 默认（不得覆写 max_blocking_threads）"
    );

    // owner 裁决：两个 profile 同取 8 MiB，不引入 release 分档常量。
    assert!(
        !source.contains("16 * 1024 * 1024"),
        "不得引入 16 MiB 分档常量（owner 已裁决 dev/release 同取 8 MiB）"
    );
    let debug_assertions_count = source.matches("debug_assertions").count();
    assert_eq!(
        debug_assertions_count, 1,
        "debug_assertions 只允许出现在启动日志的 profile 判定处（不得用于栈大小分档）"
    );
}

/// 断言：CLI 语义逐字不变 —— 真实二进制的 stdout / stderr / 退出码
/// 与改前基线逐字节一致（非 web 子命令不新增任何 stderr 输出）。
#[test]
fn provider_run_runtime_entry_keeps_cli_output_and_exit() {
    let source = main_rs_source();
    let binary = aria_binary();

    // 入口语义保持：args 来源与三分支 match 逐字保留（空 Text 不打印）。
    assert!(
        source.contains("std::env::args().skip(1)"),
        "CLI 参数来源必须保持 std::env::args().skip(1)"
    );
    assert!(
        source.contains("run_cli_async"),
        "入口必须继续经 run_cli_async 分发"
    );
    assert!(
        source.contains("if !text.is_empty() => println!(\"{text}\"),"),
        "非空 Text 打印分支必须逐字保留"
    );
    assert!(
        source.contains("Ok(_) => {}"),
        "空 Text / 非 Text 分支必须保持不打印"
    );
    assert!(
        source.contains("eprintln!(\"{}: {}\", error.code, error.message);"),
        "错误分支的 stderr 格式必须逐字保留"
    );
    assert!(
        source.contains("std::process::exit(1);"),
        "错误分支退出码必须保持 1"
    );

    // 行为级基线 1：未知子命令 → 退出码 1，stdout 空，stderr 逐字不变。
    let output = Command::new(&binary)
        .args(["no-such-command"])
        .output()
        .expect("运行 aria no-such-command 失败");
    assert_eq!(output.status.code(), Some(1), "未知子命令退出码必须为 1");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "",
        "未知子命令 stdout 必须为空"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "invalid_cli_args: expected daemon status, repl, task run, or web command\n",
        "未知子命令 stderr 必须与改前逐字一致（不得混入运行时日志）"
    );

    // 行为级基线 2：daemon status（干净工作区）→ 退出码 0，stderr 空。
    let workspace = tempdir().expect("temp workspace");
    let output = Command::new(&binary)
        .args([
            "daemon",
            "status",
            "--workspace",
            workspace.path().to_str().expect("workspace path"),
        ])
        .output()
        .expect("运行 aria daemon status 失败");
    assert_eq!(
        output.status.code(),
        Some(0),
        "daemon status 退出码必须为 0"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "daemon_not_found\n",
        "daemon status stdout 必须与改前逐字一致"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "",
        "daemon status stderr 必须保持为空"
    );

    // 行为级基线 3：repl --no-start（无守护进程）→ 退出码 1，stderr 无运行时日志。
    let output = Command::new(&binary)
        .args([
            "repl",
            "--no-start",
            "--workspace",
            workspace.path().to_str().expect("workspace path"),
        ])
        .output()
        .expect("运行 aria repl --no-start 失败");
    assert_eq!(
        output.status.code(),
        Some(1),
        "repl --no-start 退出码必须为 1"
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "",
        "repl --no-start stdout 必须为空"
    );
    let repl_stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !repl_stderr.contains("[aria-runtime]"),
        "非 web 子命令 stderr 不得出现运行时日志"
    );
}

/// 断言：仅 `aria web` 启动时向 stderr 打印一行运行时诊断
/// `[aria-runtime] profile=<debug|release> worker_stack_bytes=8388608`；
/// web 的 stdout 保持逐字不变；非 web CLI 的 stderr 逐字不变。
#[test]
fn provider_run_runtime_web_reports_profile_and_effective_stack() {
    let binary = aria_binary();
    let workspace = tempdir().expect("temp workspace");

    // web 子命令：stdout 保持 web_check_ok 逐字不变，stderr 恰为一行运行时日志。
    let output = Command::new(&binary)
        .args([
            "web",
            "--check",
            "--workspace",
            workspace.path().to_str().expect("workspace path"),
        ])
        .output()
        .expect("运行 aria web --check 失败");
    assert_eq!(output.status.code(), Some(0), "web --check 退出码必须为 0");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "web_check_ok:{}:127.0.0.1:auto\n",
            workspace.path().to_string_lossy()
        ),
        "web --check stdout 必须与改前逐字一致（运行时日志只允许出现在 stderr）"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "web 启动的 stderr 必须恰好一行运行时日志，实际: {stderr:?}"
    );
    let line = lines[0];
    assert!(
        line.starts_with("[aria-runtime] profile="),
        "运行时日志必须以 [aria-runtime] profile= 开头，实际: {line:?}"
    );
    let profile = line
        .strip_prefix("[aria-runtime] profile=")
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or_default();
    assert!(
        profile == "debug" || profile == "release",
        "profile 只能取 debug 或 release（跟随构建档位），实际: {profile:?}"
    );
    assert_eq!(
        line.strip_prefix("[aria-runtime] profile=")
            .and_then(|rest| rest.split_once(' '))
            .map(|(_, bytes)| bytes),
        Some("worker_stack_bytes=8388608"),
        "worker_stack_bytes 必须等于 8 MiB（8_388_608），实际: {line:?}"
    );

    // 空 Text 不打印：web --check 的 stdout 之外不得再有多余输出（上面已逐字断言），
    // 入口侧空 Text 分支由 keeps_cli_output 测试的源断言覆盖。
}
