# Aria npx 本地化分发 · P2：Rust 启动检测与 release 优化

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `aria web` 启动时检测 PATH 中的 codex/claude 并友好提示（不阻断启动）；新增 `[profile.release]` 控制嵌入前端后的二进制体积；为 stderr 绑定地址输出行加回归测试（保护 launcher 兜底解析契约）。

**Architecture:** 新增独立可单测的探测模块 `src/web/provider_probe.rs`：输入「程序名 + 一个 PATH 查找闭包」，输出「缺失列表 + 提示文案」，纯函数化便于 TDD。`serve_web` 启动时调用并 `eprintln!` 提示，缺失不返回错误。`Cargo.toml` 加 `[profile.release]`。绑定输出行测试固化 `listening on http://` 格式。

**Tech Stack:** Rust（std、axum）。

**对应设计：** v1.3 第 4.3、4.4、4.5 节；总览 P2 行。

**前置：** 所有 `cargo` 命令前先 `pnpm -C web build`（P1 引入的 build.rs 要求 `web/dist` 存在）。建议在 P1 完成后执行本分册。

---

## 文件结构

| 文件 | 职责 | 操作 |
|------|------|------|
| `src/web/provider_probe.rs` | 检测 codex/claude 是否在 PATH + 生成提示文案（纯函数） | Create |
| `src/web/mod.rs` | 注册 `provider_probe` 模块 | Modify |
| `src/web/app.rs` | `serve_web` 启动时调用探测并打印提示 | Modify |
| `Cargo.toml` | 新增 `[profile.release]` | Modify |
| `tests/it_web/web_provider_probe.rs` | 探测纯函数单测 | Create |
| `tests/it_web/web_listening_line.rs` | 绑定地址输出行格式回归测试 | Create |
| `tests/it_web.rs` | 注册两个新测试模块 | Modify |

---

## Task 1：provider_probe 探测模块（TDD）

**Files:**
- Create: `src/web/provider_probe.rs`
- Modify: `src/web/mod.rs`

设计要求探测逻辑「输入 PATH 查找结果、输出提示文案与是否缺失」，与真实 PATH 查找解耦以便单测。

- [ ] **Step 1: 注册模块**

在 `src/web/mod.rs` 的模块声明区按字母序加入：

```rust
pub mod provider_probe;
```

- [ ] **Step 2: 写探测模块（含纯函数 + 真实 PATH 查找）**

创建 `src/web/provider_probe.rs`：

```rust
//! 启动时探测外部 provider CLI（codex / claude）是否可用。
//! 缺失仅提示、不阻断 `aria web` 启动——工作台 UI 与 FakeProvider 演示不依赖外部 CLI。

use crate::cross_cutting::adapter_compatibility::default_compatibility_matrix;
use crate::protocol::contracts::ProviderType;

/// 一个待探测的 provider：展示名 + 可执行程序名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProbe {
    pub display: String,
    pub program: String,
}

/// 从兼容性矩阵取出 codex / claude 的程序名（避免硬编码漂移）。
pub fn provider_probes() -> Vec<ProviderProbe> {
    let matrix = default_compatibility_matrix();
    let mut probes = Vec::new();
    for (display, ty) in [
        ("Claude Code", ProviderType::ClaudeCode),
        ("Codex", ProviderType::Codex),
    ] {
        if let Some(entry) = matrix.entry_for(ty) {
            probes.push(ProviderProbe {
                display: display.to_string(),
                program: entry.provider_command.to_string_lossy().to_string(),
            });
        }
    }
    probes
}

/// 纯函数：给定待探测项与「程序是否在 PATH」的判定闭包，返回提示文案。
/// 返回 None 表示全部就绪（无需提示）；Some(text) 为面向用户的中文提示。
pub fn probe_message<F>(probes: &[ProviderProbe], is_on_path: F) -> Option<String>
where
    F: Fn(&str) -> bool,
{
    let missing: Vec<&ProviderProbe> = probes.iter().filter(|p| !is_on_path(&p.program)).collect();
    if missing.is_empty() {
        return None;
    }
    let mut lines = vec![
        "提示：以下 provider CLI 未在 PATH 中找到，相关真实执行功能将不可用（工作台界面与 Fake 演示不受影响）：".to_string(),
    ];
    for p in &missing {
        lines.push(format!(
            "  - {} (`{}`)：安装后即可使用其真实 provider。",
            p.display, p.program
        ));
    }
    lines.push("如需启用真实执行，请安装对应 CLI 并确保其在 PATH 中。".to_string());
    Some(lines.join("\n"))
}

/// 真实 PATH 查找：在 `PATH` 各目录下查找可执行文件。
pub fn is_program_on_path(program: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        let candidate = dir.join(program);
        candidate.is_file()
    })
}

/// 启动时调用：探测并把提示打印到 stderr（不阻断）。
pub fn emit_provider_probe_notice() {
    let probes = provider_probes();
    if let Some(msg) = probe_message(&probes, is_program_on_path) {
        eprintln!("{msg}");
    }
}
```

- [ ] **Step 3: 验证编译**

Run: `pnpm -C web build && cargo check --locked`
Expected: 通过（`emit_provider_probe_notice` 暂未被调用，可能 unused，Task 2 接入消除）。

- [ ] **Step 4: 提交**

```bash
git add src/web/mod.rs src/web/provider_probe.rs
git commit -m "feat: 新增 provider_probe 启动检测模块(纯函数+PATH查找)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2：探测纯函数单测（TDD）

**Files:**
- Create: `tests/it_web/web_provider_probe.rs`
- Modify: `tests/it_web.rs`

- [ ] **Step 1: 注册测试模块**

在 `tests/it_web.rs` 加入：

```rust
mod web_provider_probe;
```

- [ ] **Step 2: 写测试**

创建 `tests/it_web/web_provider_probe.rs`：

```rust
use cadence_aria::web::provider_probe::{ProviderProbe, probe_message, provider_probes};

fn probes() -> Vec<ProviderProbe> {
    vec![
        ProviderProbe { display: "Claude Code".into(), program: "claude".into() },
        ProviderProbe { display: "Codex".into(), program: "codex".into() },
    ]
}

#[test]
fn all_present_yields_no_message() {
    let msg = probe_message(&probes(), |_| true);
    assert!(msg.is_none(), "全部就绪时不应提示");
}

#[test]
fn missing_one_lists_it_without_blocking() {
    // claude 缺失、codex 存在
    let msg = probe_message(&probes(), |program| program != "claude").expect("应有提示");
    assert!(msg.contains("Claude Code"), "应列出缺失的 Claude Code");
    assert!(msg.contains("`claude`"), "应含程序名 claude");
    assert!(!msg.contains("Codex") || !msg.contains("`codex`：安装"), "不应把存在的 codex 列为缺失");
}

#[test]
fn missing_all_lists_all() {
    let msg = probe_message(&probes(), |_| false).expect("应有提示");
    assert!(msg.contains("Claude Code") && msg.contains("Codex"), "应列出全部缺失项");
}

#[test]
fn provider_probes_resolved_from_matrix() {
    // 真实矩阵应解析出 claude 与 codex 两个程序名
    let resolved = provider_probes();
    let programs: Vec<String> = resolved.iter().map(|p| p.program.clone()).collect();
    assert!(programs.iter().any(|p| p == "claude"), "矩阵应含 claude");
    assert!(programs.iter().any(|p| p == "codex"), "矩阵应含 codex");
}
```

- [ ] **Step 3: 运行测试**

Run: `pnpm -C web build && cargo test --locked --test it_web web_provider_probe`
Expected: 4 个用例 PASS。

- [ ] **Step 4: 提交**

```bash
git add tests/it_web.rs tests/it_web/web_provider_probe.rs
git commit -m "test: provider_probe 探测提示纯函数单测

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3：serve_web 接入启动检测

**Files:**
- Modify: `src/web/app.rs`（`serve_web` 内，bind 之后、serve 之前）

- [ ] **Step 1: 接入探测调用**

在 `src/web/app.rs` 的 `serve_web` 中，找到：

```rust
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    eprintln!("aria web listening on http://{bound_addr}");
```

在 `eprintln!("aria web listening ...")` **之前**插入一行探测调用：

```rust
    crate::web::provider_probe::emit_provider_probe_notice();
    let listener = TcpListener::bind(addr).await?;
    let bound_addr = listener.local_addr()?;
    eprintln!("aria web listening on http://{bound_addr}");
```

> 探测提示在「listening on」行之前打印，使 launcher 解析就绪行时不被提示文案干扰（launcher 匹配 `listening on http://`）。

- [ ] **Step 2: 全量验证**

Run:
```bash
pnpm -C web build
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
```
Expected: 全绿，无 unused 警告。

- [ ] **Step 3: 提交**

```bash
git add src/web/app.rs
git commit -m "feat: serve_web 启动时检测 codex/claude 并提示(不阻断)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 4：绑定地址输出行格式回归测试

**Files:**
- Create: `tests/it_web/web_listening_line.rs`
- Modify: `tests/it_web.rs`

> launcher 的就绪兜底解析依赖 stderr 中 `listening on http://<addr>` 行格式稳定。该格式由 `serve_web` 内联 `eprintln!` 产生，难以直接单测字符串。改为固化「格式契约」于一个常量 + 测试，任何人改动需同步更新，形成回归护栏。

- [ ] **Step 1: 在 app.rs 提取格式常量**

在 `src/web/app.rs` 中 `serve_web` 上方（或文件顶部 use 之后）新增公开常量与格式化函数：

```rust
/// launcher 依赖的就绪行前缀契约。修改即破坏 launcher 解析，须同步更新 bin/aria.js 与回归测试。
pub const LISTENING_LINE_PREFIX: &str = "aria web listening on http://";

/// 生成就绪行（统一格式来源）。
pub fn listening_line(addr: &SocketAddr) -> String {
    format!("{LISTENING_LINE_PREFIX}{addr}")
}
```

并将 `serve_web` 中的：

```rust
    eprintln!("aria web listening on http://{bound_addr}");
```

改为：

```rust
    eprintln!("{}", listening_line(&bound_addr));
```

- [ ] **Step 2: 注册测试模块**

在 `tests/it_web.rs` 加入：

```rust
mod web_listening_line;
```

- [ ] **Step 3: 写回归测试**

创建 `tests/it_web/web_listening_line.rs`：

```rust
use cadence_aria::web::app::{LISTENING_LINE_PREFIX, listening_line};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

#[test]
fn listening_line_prefix_is_stable_contract() {
    // launcher (bin/aria.js) 匹配此前缀判定就绪；修改即破坏分发，需同步更新。
    assert_eq!(LISTENING_LINE_PREFIX, "aria web listening on http://");
}

#[test]
fn listening_line_renders_addr() {
    let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 4317));
    let line = listening_line(&addr);
    assert_eq!(line, "aria web listening on http://127.0.0.1:4317");
    assert!(line.starts_with(LISTENING_LINE_PREFIX));
}
```

- [ ] **Step 4: 运行测试**

Run: `pnpm -C web build && cargo test --locked --test it_web web_listening_line`
Expected: 2 个用例 PASS。

- [ ] **Step 5: 提交**

```bash
git add src/web/app.rs tests/it_web.rs tests/it_web/web_listening_line.rs
git commit -m "test: 固化 launcher 就绪行格式契约 + 回归测试

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 5：新增 [profile.release] 体积优化

**Files:**
- Modify: `Cargo.toml`（文件末尾，与现有 `[profile.dev]` 并列）

- [ ] **Step 1: 添加 release profile**

在 `Cargo.toml` 末尾（现有 `[profile.dev.package."*"]` 块之后）追加：

```toml
# Release 构建：嵌入前端 + git 重依赖后控制二进制体积（npx 分发友好）
[profile.release]
strip = true
lto = true
opt-level = "s"
codegen-units = 1
```

> 先不设 `panic = "abort"`——它可能与现有 panic 处理/测试交互产生风险，作为开放事项按实测决定（见总览开放事项）。`opt-level` 初选 `"s"`（体积优先且比 `"z"` 通常运行更稳），如首次构建体积仍偏大可改 `"z"` 再实测。

- [ ] **Step 2: 构建 release 并记录体积**

Run:
```bash
pnpm -C web build
cargo build --release --locked
ls -lh target/release/aria
```
Expected: release 构建成功；记录 `aria` 体积（写入提交信息或开发文档）。冷构建可能数分钟（git 重依赖），属正常。

- [ ] **Step 3: 验证 release 二进制可运行**

Run:
```bash
target/release/aria web --check --workspace . 2>&1 | head -3
```
Expected: 输出 `web_check_ok:...`（`--check` 路径仅校验参数、不真正起服务），确认 release 二进制可执行、嵌入资源已打包。

- [ ] **Step 4: 提交**

```bash
git add Cargo.toml
git commit -m "build: 新增 [profile.release] 体积优化(strip/lto/opt-level=s)

release 二进制体积：<填入 Step 2 实测值>

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## P2 自检（完成所有 Task 后执行）

- [ ] **探测纯函数**：`cargo test --locked --test it_web web_provider_probe` 4 绿。
- [ ] **就绪行契约**：`cargo test --locked --test it_web web_listening_line` 2 绿。
- [ ] **不阻断**：探测逻辑返回提示文案/打印，不返回错误、不影响 `serve_web` 继续 bind。
- [ ] **release 可构建可运行**：`cargo build --release --locked` 成功，`target/release/aria web --check --workspace .` 输出 `web_check_ok`。
- [ ] **体积已记录**：release 二进制体积写入提交信息（供 P3/P4 体积 gate 参考）。
- [ ] **全量门禁**：fmt / clippy / test 全绿。

完成后进入 **P3**。
