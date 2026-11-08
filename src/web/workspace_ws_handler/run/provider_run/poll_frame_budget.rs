//! 治本门禁（Task 14b）：**poll 帧预留**体积门禁（fail-closed）。
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
//! 覆盖对象（分级预算见 `GATED_POLL_SYMBOLS`；修前 → 修后实测见任务报告）：
//! - P0 provider run 核心（≤64 KiB）：drive body / task body / 各族臂；
//! - P0 spawn / 准入侧（≤32 KiB）：三个 spawn 入口；
//! - 宏体对象（过渡 ≤128 KiB）：PlanRevision 宏体、followups 宏体；
//! - P1 engine 侧评审链（≤256 KiB）：评审链四对象 + `complete_review`。
//!
//! ## fail-closed 合同（与行为一致，不得再出现「告警 + 跳过 + 用例 ok」）
//!
//! 1. **工具缺失 ⇒ 用例失败**：必需工具 `nm` / `objdump` 任一不可用 ⇒ 直接
//!    FAIL，失败信息点名缺失的工具并给出可执行的处理办法（安装 binutils 等）。
//!    只有**显式**设置环境变量 `ARIA_POLL_FRAME_BUDGET_ALLOW_MISSING_TOOLS=1`
//!    才允许在本地无 binutils 时跳过（默认＝失败；该开关名写在失败信息里）。
//! 2. **探针量测为 0 ⇒ 用例失败**：预算 > 0 而实测 0 B 时**不得**当作「0 字节
//!    很棒」放过——序言扫描窗口是启发式（0x200 B），量出 0 多半是探针窗口
//!    失效（编译器布局变化）或符号定位错误 ⇒ 显式失败。
//! 3. 分级阈值本身**不因上述护栏放宽**（P0 核心 ≤64 KiB / P0 spawn 侧
//!    ≤32 KiB / P1 engine ≤256 KiB / 宏体过渡 ≤128 KiB）。

use std::path::Path;
use std::process::Command;

/// P0 provider run 核心：poll 帧预留硬上限 64 KiB（治本验收口径；未装箱
/// size 的 128 KiB 门禁见 stack_budget，两者并行、不可互相替代）。
const P0_CORE_BUDGET: usize = 64 * 1024;

/// P0 spawn/准入侧：poll 帧预留上限 32 KiB（spawn 搬运帧与准入链）。
const P0_SPAWN_BUDGET: usize = 32 * 1024;

/// P1 engine 侧评审链：上限 256 KiB（`complete_review` 等 engine 内重子
/// 调用位于 `src/product/workspace_engine/**`——超出本任务写集，由独立
/// change 继续收口）。
const P1_ENGINE_BUDGET: usize = 256 * 1024;

/// **过渡预算**（128 KiB）：两个宏体对象——PlanRevision 宏体
/// （`drive_plan_revision_arm`，实测 118,784 B）与 followups 宏体
/// （`drive_provider_run_followups`，实测 86,016 B）。二者的宏文本位于
/// `src/web/workspace_ws_handler/run/followups.rs`（超出本任务写集），宏体
/// 内部的 engine 重子调用尚不能逐个装箱；待该宏按设计 §3.2 迁入
/// `provider_run/` 并装箱后，本预算收紧到 64 KiB。
const MACRO_BODY_TRANSITION_BUDGET: usize = 128 * 1024;

/// 受门禁约束的 poll 符号（needle 必须命中该 async fn 的 poll 体
/// `…::{closure#0}`，避免误测同名构造函数的普通帧）与各自预算。
const GATED_POLL_SYMBOLS: &[(&str, usize)] = &[
    // —— P0 provider run 核心（≤64 KiB）——
    ("provider_run_drive_body", P0_CORE_BUDGET),
    ("provider_run_task_body", P0_CORE_BUDGET),
    ("provider_run_task", P0_CORE_BUDGET),
    ("drive_author_arm", P0_CORE_BUDGET),
    ("drive_author_choice_followup_arm", P0_CORE_BUDGET),
    ("drive_revision_arm", P0_CORE_BUDGET),
    ("drive_review_only_arm", P0_CORE_BUDGET),
    ("drive_single_candidate_author_arm", P0_CORE_BUDGET),
    ("drive_legacy_outline_arm", P0_CORE_BUDGET),
    ("drive_plan_draft_arm", P0_CORE_BUDGET),
    ("drive_plan_batch_arm", P0_CORE_BUDGET),
    ("drive_human_gate_sc_revision_arm", P0_CORE_BUDGET),
    // —— P0 spawn / 准入侧（≤32 KiB）——
    ("spawn_provider_run_from_event", P0_SPAWN_BUDGET),
    ("spawn_provider_run_from_handler", P0_SPAWN_BUDGET),
    ("spawn_provider_run_claiming_idle", P0_SPAWN_BUDGET),
    // —— 宏体对象（过渡 ≤128 KiB；宏文本在写集外）——
    ("drive_plan_revision_arm", MACRO_BODY_TRANSITION_BUDGET),
    ("drive_provider_run_followups", MACRO_BODY_TRANSITION_BUDGET),
    // —— P1 engine 侧评审链（≤256 KiB；engine 侧文件在写集外）——
    (
        "drive_review_session_via_gateway_with_window",
        P1_ENGINE_BUDGET,
    ),
    ("drive_review_session_with_window", P1_ENGINE_BUDGET),
    ("drive_review_session", P1_ENGINE_BUDGET),
    ("drive_review_session_via_gateway", P1_ENGINE_BUDGET),
    ("complete_review", P1_ENGINE_BUDGET),
];

/// **release 口径已知盲区**（owner 知情的技术事实，非临时 opt-out，故为代码内
/// 显式常量而非环境变量）：release（`opt-level="s"` + `lto=true`）下这些 async fn
/// 的 poll 体被内联进调用方，符号表（`nm -C`，已用 `release-ci-gate` 保留符号实测）
/// 里只剩 `drop_glue` / `boxed_child` 包装形态，没有以 `{closure#0}` 结尾的独立
/// poll 符号可量测。关掉 LTO / 放宽过滤到 `Pin<Box<..>>::poll` 形态均已实测无效或
/// 会让数字失真，故显式登记。它们仍保留在 `GATED_POLL_SYMBOLS` 内：debug 口径
/// 照常量测并硬断言，release 口径只在这里豁免「符号缺失」，且每次运行逐条打印
/// `[release-blind-spot]` 行（覆盖面被削弱的清单可见，不静默）。
const RELEASE_BLIND_SPOTS: &[&str] =
    &["provider_run_task_body", "spawn_provider_run_claiming_idle"];

/// `needle` 符号缺失是否可被豁免：仅「release 口径 + 在盲区清单内」。
fn missing_is_excused(needle: &str, release: bool) -> bool {
    release && RELEASE_BLIND_SPOTS.contains(&needle)
}

/// 显式 opt-in 开关：**仅当**该环境变量被设为 `"1"` 时，工具缺失才允许跳过
/// （默认＝失败）。失败信息必须写明该开关名（不得「偷偷跳过」）。
pub(crate) const ALLOW_MISSING_TOOLS_ENV: &str = "ARIA_POLL_FRAME_BUDGET_ALLOW_MISSING_TOOLS";

/// 序言扫描窗口（字节，启发式）：只在该符号前的窗口内找栈预留立即数。
const PROBE_WINDOW_BYTES: u64 = 0x200;

/// 错误分类前缀：工具缺失（可被显式 opt-in 跳过）。
const MISSING_TOOL_MARKER: &str = "[missing-tool]";
/// 错误分类前缀：探针量测为 0（**不可**跳过）。
const PROBE_WINDOW_MARKER: &str = "[probe-window]";

/// 门禁所用外部工具路径（可注入 ⇒ 便于用例驱动「工具缺失」分支）。
#[derive(Clone, Debug)]
pub(crate) struct ToolPaths {
    pub(crate) nm: String,
    pub(crate) objdump: String,
}

/// 生产路径使用的工具路径。
pub(crate) fn default_tool_paths() -> ToolPaths {
    ToolPaths {
        nm: "nm".to_string(),
        objdump: "objdump".to_string(),
    }
}

/// 工具是否可执行（`--version` 成功即视为可用）。
fn tool_available(path: &str) -> bool {
    Command::new(path)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// 工具缺失时的失败信息（点名工具 + 处理办法 + opt-in 开关名）。
fn missing_tool_error(tool: &str, path: &str) -> String {
    format!(
        "{MISSING_TOOL_MARKER} poll 帧门禁无法执行：必需工具 `{tool}`（路径 `{path}`）不可用。\
         处理办法：安装 binutils（Debian/Ubuntu: `apt-get install -y binutils`；\
         Alpine: `apk add --no-cache binutils`；Fedora/RHEL: `dnf install -y binutils`；\
         macOS: `xcode-select --install`；Rust 工具链自带 llvm-tools 时可用 \
         `llvm-nm`/`llvm-objdump` 替代）。\
         若确需在无 binutils 的本地环境跳过本门禁，请**显式**设置环境变量 \
         `{ALLOW_MISSING_TOOLS_ENV}=1`（默认＝失败，不允许静默跳过）。"
    )
}

/// 探针量测为 0 时的失败信息（说明真实含义，不当作「0 字节达标」）。
fn zero_probe_error(needle: &str, symbol: &str) -> String {
    format!(
        "{PROBE_WINDOW_MARKER} poll 帧预留量测失败：符号 `{symbol}`（needle `{needle}`）在序言扫描\
         窗口 0x{PROBE_WINDOW_BYTES:x} B 内未读到任何栈预留立即数（实测 0 B）——\
         多半是探针窗口失效（编译器布局变化）或符号定位错误；\
         **不得**当作「0 字节达标」放过，请同步更新探针窗口或核对符号名。"
    )
}

/// 读取 `nm -C` 的符号表：返回 (地址, 反修饰名)。
fn demangled_symbols(binary: &Path, nm: &str) -> Result<Vec<(u64, String)>, String> {
    let output = Command::new(nm)
        .arg("-C")
        .arg(binary)
        .output()
        .map_err(|error| format!("{MISSING_TOOL_MARKER} 工具 `{nm}` 执行失败：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{MISSING_TOOL_MARKER} 工具 `{nm}` 退出码 {:?}：{}",
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
fn poll_frame_bytes(binary: &Path, objdump: &str, address: u64) -> Result<usize, String> {
    let output = Command::new(objdump)
        .arg("-d")
        .arg(format!("--start-address={address:#x}"))
        .arg(format!(
            "--stop-address={:#x}",
            address + PROBE_WINDOW_BYTES
        ))
        .arg(binary)
        .output()
        .map_err(|error| format!("{MISSING_TOOL_MARKER} 工具 `{objdump}` 执行失败：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{MISSING_TOOL_MARKER} 工具 `{objdump}` 退出码 {:?}：{}",
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

/// 实测值为 0 时的护栏（探针窗口失效 / 符号定位错误 ⇒ 显式失败）。
fn ensure_measured(needle: &str, symbol: &str, best: usize) -> Result<usize, String> {
    if best == 0 {
        return Err(zero_probe_error(needle, symbol));
    }
    Ok(best)
}

/// 门禁主流程：量测全部受管对象（工具缺失 / 量测为 0 ⇒ Err，绝不静默跳过）。
fn measure_gated(binary: &Path, tools: &ToolPaths) -> Result<Vec<(String, usize, usize)>, String> {
    // ① 工具可用性（fail-closed）：缺一即失败。
    for (name, path) in [("nm", &tools.nm), ("objdump", &tools.objdump)] {
        if !tool_available(path) {
            return Err(missing_tool_error(name, path));
        }
    }
    let symbols = demangled_symbols(binary, &tools.nm)?;
    let mut measured: Vec<(String, usize, usize)> = Vec::new();
    let mut missing: Vec<&str> = Vec::new();
    for (needle, budget) in GATED_POLL_SYMBOLS {
        // 只认 async fn 的 poll 体（`…::{closure#0}`），避免误测同名普通函数；
        // `drop_glue::<…{closure#0}>` / `boxed_child::<…>` 等包装形态不以 `{closure#0}` 结尾，不会命中。
        let matched: Vec<(u64, String)> = symbols
            .iter()
            .filter(|(_, name)| name.contains(needle) && name.ends_with("{closure#0}"))
            .cloned()
            .collect();
        if matched.is_empty() {
            if missing_is_excused(needle, !cfg!(debug_assertions)) {
                // 显式盲区：不静默——逐条打印，job summary 里也能看到。
                println!(
                    "[poll-frame-budget] [release-blind-spot] {needle}: release 口径下无独立 poll 符号（poll 体已内联进调用方），本次未量测；debug 口径照常量测"
                );
                continue;
            }
            missing.push(needle);
            continue;
        }
        let mut best = 0usize;
        let mut best_name = String::new();
        for (address, name) in matched {
            let bytes = poll_frame_bytes(binary, &tools.objdump, address)?;
            if bytes > best {
                best = bytes;
                best_name = name;
            }
        }
        // ② 量测为 0 ⇒ 显式失败（不得当作「0 字节达标」）。
        let best = ensure_measured(needle, &best_name, best)?;
        println!(
            "[poll-frame-budget] {needle}: poll 帧预留 {best} B（预算 {budget} B）| {best_name}"
        );
        measured.push(((*needle).to_string(), best, *budget));
    }
    if !missing.is_empty() {
        return Err(format!(
            "[missing-symbol] 以下受门禁对象的 poll 符号未在测试二进制中定位到：{missing:?}——\
             门禁覆盖面被削弱，需同步更新 GATED_POLL_SYMBOLS"
        ));
    }
    Ok(measured)
}

/// 门禁用例：逐对象量测 poll 帧预留并硬断言 ≤预算（fail-closed）。
#[test]
fn poll_frame_reservations_stay_within_budget() {
    let binary = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => panic!("[poll-frame-budget] 无法定位测试二进制：{error}（门禁失败）"),
    };
    let measured = match measure_gated(&binary, &default_tool_paths()) {
        Ok(measured) => measured,
        Err(error) => {
            // 只有「工具缺失」且**显式 opt-in** 时才允许跳过；其余一律失败。
            let opt_in = std::env::var(ALLOW_MISSING_TOOLS_ENV).as_deref() == Ok("1");
            if opt_in && error.starts_with(MISSING_TOOL_MARKER) {
                eprintln!(
                    "[poll-frame-budget] 已按显式开关 {ALLOW_MISSING_TOOLS_ENV}=1 跳过：{error}"
                );
                return;
            }
            panic!("{error}");
        }
    };
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

/// 门禁护栏自测（注入式）：工具缺失 ⇒ 显式失败（点名工具 + 处理办法 +
/// opt-in 开关名），绝不允许「告警 + 跳过 + 用例 ok」。
#[cfg(test)]
mod fail_closed_guard_tests {
    use super::*;

    #[test]
    fn missing_tool_fails_closed_with_actionable_message() {
        let error = measure_gated(
            Path::new("/nonexistent/aria-poll-frame-binary"),
            &ToolPaths {
                nm: "aria-no-such-nm".to_string(),
                objdump: "aria-no-such-objdump".to_string(),
            },
        )
        .expect_err("工具缺失必须显式失败，不得静默跳过");
        assert!(
            error.contains("aria-no-such-nm"),
            "失败信息必须点名缺失的工具：{error}"
        );
        assert!(
            error.contains("binutils"),
            "失败信息必须给出可执行的处理办法：{error}"
        );
        assert!(
            error.contains(ALLOW_MISSING_TOOLS_ENV),
            "失败信息必须写明显式 opt-in 开关名：{error}"
        );
        assert!(
            error.starts_with(MISSING_TOOL_MARKER),
            "工具缺失必须带可识别的分类标记：{error}"
        );
    }

    #[test]
    fn zero_probe_measurement_fails_closed() {
        let error = ensure_measured("needle_x", "symbol_x", 0)
            .expect_err("量测为 0 必须显式失败，不得当作「0 字节达标」");
        assert!(
            error.contains(PROBE_WINDOW_MARKER),
            "失败信息必须带探针窗口分类标记：{error}"
        );
        assert!(error.contains("0x200"), "失败信息必须说明探针窗口：{error}");
        assert!(
            error.contains("symbol_x"),
            "失败信息必须点名量测失败的符号：{error}"
        );
    }

    #[test]
    fn release_blind_spots_are_gated_symbols_and_only_excused_in_release() {
        for needle in RELEASE_BLIND_SPOTS {
            assert!(
                GATED_POLL_SYMBOLS.iter().any(|(gated, _)| gated == needle),
                "盲区对象必须仍在 GATED_POLL_SYMBOLS 内（debug 口径照常量测）：{needle}"
            );
            assert!(
                missing_is_excused(needle, true),
                "release 口径下盲区对象可豁免：{needle}"
            );
            assert!(
                !missing_is_excused(needle, false),
                "debug 口径下盲区对象缺失必须失败：{needle}"
            );
        }
        assert!(
            !missing_is_excused("provider_run_task", true),
            "非盲区对象在 release 下缺失仍须失败"
        );
    }
}
