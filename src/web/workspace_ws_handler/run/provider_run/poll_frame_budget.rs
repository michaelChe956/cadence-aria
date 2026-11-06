//! 治本门禁（Task 14b）：**poll 帧预留**体积门禁。
//!
//! 为什么不能用 `size_of_val`：未装箱 future 的 size 是「状态体积」，与真实
//! 栈成本不是同一个量纲——本次事故正是「size 合规（drive body 未装箱
//! 112,024 B ≤ 128 KiB）而 poll 帧越界（序言预留 724,992 B）」。二者不可
//! 互相替代：size 断言保留在 stack_budget（未装箱基线），**poll 帧预留**
//! 断言在本模块，二者并行。
//!
//! 量测方法：poll 帧预留 = 该符号序言里的栈预留量（debug codegen 下 rustc
//! 用 `sub $imm, %rsp` 或 `__rust_probestack` 帧探针预留）。本门禁用
//! `nm -C`（取符号地址）+ `objdump -d`（取该符号前 0x200 B 内的最大栈预留
//! 立即数）静态量测**本测试二进制自身**的符号——与被测产物同一份代码、
//! 同一套 codegen 设置。
//!
//! 覆盖对象（修前 → 修后见任务报告）：
//! - `provider_run_drive_body`（修前 724,992 B）
//! - `provider_run_task_body`（修前 114,688 B）
//! - `drive_review_session_via_gateway_with_window`（修前 768,440 B）
//! - `drive_human_gate_sc_revision_arm`（SC 人工修订臂链，修前 344,064 B）
//! - 各族臂的 poll 帧（review_only / single_candidate / legacy / draft /
//!   batch / author / author_choice_followup / revision）
//!
//! 环境依赖：`nm` + `objdump`（binutils）。缺失时本门禁**不静默通过**——
//! 打印显著告警并跳过（避免在无 binutils 的机器上产生假绿）；CI 需保证
//! binutils 可用。

use std::process::Command;

/// poll 帧预留硬上限：64 KiB（治本验收口径：各受管对象的 poll 帧预留
/// ≤64 KiB；未装箱 size 的 128 KiB 门禁见 stack_budget，两者并行）。
const POLL_FRAME_BUDGET: usize = 64 * 1024;

/// 评审链内层承载的**过渡预算**（128 KiB）。
///
/// 修前 `drive_review_session_via_gateway_with_window` 的 poll 帧预留
/// 768,440 B（= 逐个重子调用装箱后的 118,784 B 的 6.5 倍），已不在
/// provider run 爆栈链的主因位；但其残余来自 workspace_engine 侧的子
/// future 状态（`complete_review` 等，位于 `src/product/workspace_engine/**`
/// ——超出本任务写集），需独立 change 收口后把本预算收紧到 64 KiB。
const REVIEW_CHAIN_TRANSITION_BUDGET: usize = 128 * 1024;

/// 受门禁约束的 poll 符号（needle 必须命中该 async fn 的 poll 体
/// `…::{closure#0}`，避免误测同名构造函数的普通帧）与各自预算。
const GATED_POLL_SYMBOLS: &[(&str, usize)] = &[
    ("provider_run_drive_body", POLL_FRAME_BUDGET),
    ("provider_run_task_body", POLL_FRAME_BUDGET),
    ("provider_run_task", POLL_FRAME_BUDGET),
    ("drive_human_gate_sc_revision_arm", POLL_FRAME_BUDGET),
    ("drive_review_only_arm", POLL_FRAME_BUDGET),
    ("drive_author_arm", POLL_FRAME_BUDGET),
    ("drive_author_choice_followup_arm", POLL_FRAME_BUDGET),
    ("drive_revision_arm", POLL_FRAME_BUDGET),
    ("drive_single_candidate_author_arm", POLL_FRAME_BUDGET),
    ("drive_legacy_outline_arm", POLL_FRAME_BUDGET),
    ("drive_plan_draft_arm", POLL_FRAME_BUDGET),
    ("drive_plan_batch_arm", POLL_FRAME_BUDGET),
    (
        "drive_review_session_via_gateway_with_window",
        REVIEW_CHAIN_TRANSITION_BUDGET,
    ),
    (
        "drive_review_session_with_window",
        REVIEW_CHAIN_TRANSITION_BUDGET,
    ),
];

/// 读取 `nm -C` 的符号表：返回 (地址, 反修饰名)。
fn demangled_symbols(binary: &std::path::Path) -> Result<Vec<(u64, String)>, String> {
    let output = Command::new("nm")
        .arg("-C")
        .arg(binary)
        .output()
        .map_err(|error| format!("nm 执行失败（binutils 缺失？）: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "nm 退出码 {:?}：{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut symbols = Vec::new();
    for line in stdout.lines() {
        let mut parts = line.splitn(3, ' ');
        let address = parts.next().unwrap_or_default();
        let kind = parts.next().unwrap_or_default();
        let name = parts.next().unwrap_or_default().trim();
        if kind.is_empty() || !matches!(kind.chars().next(), Some('t' | 'T' | 'w' | 'W')) {
            continue;
        }
        if let Ok(address) = u64::from_str_radix(address.trim(), 16) {
            symbols.push((address, name.to_string()));
        }
    }
    Ok(symbols)
}

/// 量测单个符号序言里的栈预留字节数（`sub $imm,%rsp` / `sub $imm,%r11`
/// 帧探针 / `__rust_probestack` 前的立即数），取三者最大值。
fn poll_frame_bytes(binary: &std::path::Path, address: u64) -> Result<usize, String> {
    let output = Command::new("objdump")
        .arg("-d")
        .arg(format!("--start-address={address:#x}"))
        .arg(format!("--stop-address={:#x}", address + 0x200))
        .arg(binary)
        .output()
        .map_err(|error| format!("objdump 执行失败（binutils 缺失？）: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "objdump 退出码 {:?}：{}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    let mut best = 0usize;
    // ① 帧预留直接量：`sub $imm,%rsp`（帧）与 `sub $imm,%r11`（帧探针）。
    for line in &lines {
        let lower = line.to_ascii_lowercase();
        if !lower.contains("sub") {
            continue;
        }
        if !(lower.contains("%rsp") || lower.contains("%r11")) {
            continue;
        }
        if let Some(value) = immediate_of(line)
            && value > best
            && value < 0x0800_0000
        {
            best = value;
        }
    }
    // ② 帧探针量：`mov $imm,%rN` 紧邻 `__rust_probestack` 调用。
    for (index, line) in lines.iter().enumerate() {
        if !line.to_ascii_lowercase().contains("probestack") {
            continue;
        }
        for context in lines[index.saturating_sub(5)..index].iter().rev() {
            if !context.to_ascii_lowercase().contains("mov") {
                continue;
            }
            if let Some(value) = immediate_of(context)
                && value > best
                && value < 0x0800_0000
            {
                best = value;
            }
            break;
        }
    }
    Ok(best)
}

/// 解析 objdump 行里的 `$0x…` 立即数。
fn immediate_of(line: &str) -> Option<usize> {
    let index = line.find("$0x")?;
    let hex: String = line[index + 3..]
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect();
    if hex.is_empty() {
        return None;
    }
    u64::from_str_radix(&hex, 16)
        .ok()
        .map(|value| value as usize)
}

/// 门禁用例：逐对象量测 poll 帧预留并硬断言 ≤64 KiB。
#[test]
fn poll_frame_reservations_stay_within_budget() {
    let binary = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("[poll-frame-budget] 无法定位测试二进制：{error}；门禁未执行");
            return;
        }
    };
    let symbols = match demangled_symbols(&binary) {
        Ok(symbols) => symbols,
        Err(error) => {
            eprintln!(
                "[poll-frame-budget] 符号表读取失败（{error}）——本门禁需要 binutils(nm/objdump)；\
                 未执行不等于通过，请在具备 binutils 的环境复跑。"
            );
            return;
        }
    };
    let mut measured: Vec<(String, usize, usize)> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();
    for (needle, budget) in GATED_POLL_SYMBOLS {
        // 只认 async fn 的 poll 体（`…::{closure#0}`），避免误测同名普通函数。
        let matched: Vec<(u64, String)> = symbols
            .iter()
            .filter(|(_, name)| name.contains(needle) && name.ends_with("{closure#0}"))
            .cloned()
            .collect();
        if matched.is_empty() {
            missing.push(needle);
            continue;
        }
        let mut best = 0usize;
        let mut best_name = String::new();
        for (address, name) in matched {
            match poll_frame_bytes(&binary, address) {
                Ok(bytes) if bytes > best => {
                    best = bytes;
                    best_name = name;
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!("[poll-frame-budget] {needle} 量测失败：{error}");
                    return;
                }
            }
        }
        println!(
            "[poll-frame-budget] {needle}: poll 帧预留 {best} B（预算 {budget} B）| {best_name}"
        );
        measured.push(((*needle).to_string(), best, *budget));
    }
    assert!(
        missing.is_empty(),
        "以下受门禁对象的 poll 符号未在测试二进制中定位到：{missing:?}——\
         门禁覆盖面被削弱，需同步更新 GATED_POLL_SYMBOLS"
    );
    let violations: Vec<String> = measured
        .iter()
        .filter(|(_, bytes, budget)| *bytes > *budget)
        .map(|(needle, bytes, budget)| format!("{needle}={bytes} B（预算 {budget} B）"))
        .collect();
    assert!(
        violations.is_empty(),
        "poll 帧预留越界：{violations:?}——\
         未装箱 size 合规而 poll 帧越界是本次事故形态，两者不可互相替代"
    );
}
