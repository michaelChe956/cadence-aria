//! aria CLI 入口。
//!
//! 阶段 1 止血（provider run 爆栈）：手写 Tokio 多线程 runtime，把 worker
//! 线程栈从 tokio 默认 2 MiB 固定到 8 MiB（代码保证，不依赖部署环境变量）。
//! `tokio::main` 宏没有栈选项，因此必须手写 Builder。

/// Tokio worker 线程栈大小（字节）：dev / release 统一 8 MiB（8_388_608）。
///
/// 依据（2026-11-06 实测）：provider run 崩溃链 = 深 inbound 链（约 1.5 MiB）
/// 叠加评审链 poll 帧（约 0.7 MiB）≈ 1.9 MiB，超过 tokio 默认 2 MiB 必爆；
/// 8 MiB 约为实测链 4 倍余量。owner 已裁决两档同取 8 MiB，
/// 原「prod 先 16 MiB、验完降 8」方案取消。
const WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // 仅 web 子命令启动时向 stderr 打印一行运行时诊断；
    // 其余子命令的 stdout / stderr / 退出码保持逐字不变。
    if args.first().is_some_and(|command| command == "web") {
        let profile = if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        };
        eprintln!("[aria-runtime] profile={profile} worker_stack_bytes={WORKER_STACK_SIZE}");
    }
    // 手写 Builder（宏无栈选项）；worker 数、线程名、blocking 上限不覆写，
    // 沿用与原宏展开一致的 tokio 默认值，仅固定 worker 线程栈大小。
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(WORKER_STACK_SIZE)
        .enable_all()
        .build()
        .expect("构建 Tokio 多线程 runtime 失败");
    runtime.block_on(async move {
        match cadence_aria::cli::run_cli_async(args).await {
            Ok(cadence_aria::cli::CliOutput::Text(text)) if !text.is_empty() => println!("{text}"),
            Ok(_) => {}
            Err(error) => {
                eprintln!("{}: {}", error.code, error.message);
                std::process::exit(1);
            }
        }
    });
}
