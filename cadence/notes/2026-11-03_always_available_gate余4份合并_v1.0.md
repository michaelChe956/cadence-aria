# always_available_gate 余 4 份私有副本合并（v1.0）

- 日期：2026-11-03
- 工作树：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 目标：把 web 树里剩余 4 份 `always_available_gate` 同形私有副本合并到共享测试支持模块
  `src/web/handlers/automation_enrollment_test_support.rs`（`pub(crate)`，已含 `always_available_gate`）。
- 原则：**零行为变化**，只做「删除本地副本 + 调用共享模块」的替换；不动生产逻辑 / 断言 / spec / Plan / 技术方案；不删测试。

---

## 0. 共享模块基线（比对基准，未改动）

`src/web/handlers/automation_enrollment_test_support.rs:290` `pub(crate) fn always_available_gate()`

| 项 | 值 |
|---|---|
| provider 名单 | `[ClaudeCode, Codex, Pi, KimiCode]`（4 家，全部 `available: true`） |
| 每条 entry | `command: "stub"`、`version: Some("1.0")`、`reason_code: None`、`reason: None` |
| snapshot | `schema_version: 1`、`generation: 1`、`checked_at: Utc::now()` |
| `degraded()` | `false` |
| host readiness | `ProviderAvailabilityGate::new` 默认 `AlwaysReadyProviderHost` |

**共享模块本轮零改动**（未加可见性调整、未改既有内容）。

---

## 1. 逐个判定与行为比对

### #1 `src/web/gateway_factory.rs:454` — 已合并 ✅

- **是否在 `#[cfg(test)]` 里**：是。`src/web/gateway_factory.rs:366-367` 为 `#[cfg(test)] mod tests { … }`，
  私有副本定义在 `mod tests` 内（模块顶层 1-365 行的生产代码只有 `LogicalCodebaseGatewayFactory`，
  其 `availability_gate: Arc<ProviderAvailabilityGate>` 字段来自**生产**注入，本副本不参与生产路径）。
- **与共享模块比对**：**API 级不等价**——本地副本 provider 名单只有
  `[ClaudeCode, Codex]`（少 `Pi`/`KimiCode`）。
  差异后果：`ProviderAvailabilityGate::ensure_available(Pi|KimiCode)`
  本地副本返回 `Err("provider health entry is missing")`，共享副本返回 `Ok`。
  其余字段（entry 内容、`degraded()=false`、host readiness、返回类型 `Arc<ProviderAvailabilityGate>`）完全一致。
- **可观测性判定：零观测差异**（依据见 §2）。

### #2 `src/web/handlers/automation_gateway_preflight.rs:475` — 已合并 ✅

- **是否在 `#[cfg(test)]` 里**：是。`src/web/handlers/automation_gateway_preflight.rs:299-300` 为 `#[cfg(test)] mod tests { … }`，
  私有副本在 `mod tests` 内；生产函数 `validate_role_chain_*` 走 `WebAppState` 注入的真实 gate。
- **与共享模块比对**：**完全等价**——provider 名单同为
  `[ClaudeCode, Codex, Pi, KimiCode]`，entry 字段、`checked_at`、`degraded()=false`、
  返回类型逐字一致。**这是 4 份里唯一可直接合并的同形副本。**

### #3 `src/web/workspace_ws_handler/tests/gateway_start.rs:86` — 已合并 ✅

- **是否在 `#[cfg(test)]` 里**：是。文件位于 `src/web/workspace_ws_handler/tests/`，
  该目录由 `src/web/workspace_ws_handler/mod.rs:59-61` 的 `#[cfg(test)] mod tests;` 引入。
- **与共享模块比对**：**API 级不等价**——provider 名单同为 `[ClaudeCode, Codex]`（少 `Pi`/`KimiCode`），
  其余同 #1。**可观测性判定：零观测差异**（依据见 §2）。

### #4 `src/web/workspace_ws_handler/tests/single_candidate_lc_admission.rs:175`（`lc_always_available_gate`）— 已合并 ✅

- **是否在 `#[cfg(test)]` 里**：是。同 #3（`tests` 目录整体 `#[cfg(test)]`）。
- **与共享模块比对**：**API 级不等价**——provider 名单 `[ClaudeCode, Codex]`（少 `Pi`/`KimiCode`），
  其余同 #1。**可观测性判定：零观测差异**（依据见 §2）。
- **命名处置**：按 brief ③「行为等价则统一用同一个」，`lc_always_available_gate` **已删除**，
  全部调用点改用共享模块的 `always_available_gate`（不保留第二个名字）。
  同文件内的 `lc_scoped_availability_gate(codex_available, kimi_available)` 是**可参数化的受控 double**
  （非恒健康替身、provider 名单与可用性可调），**不在本轮范围，保持原样不动**。

---

## 2. 为什么「provider 名单差异」不产生可观测行为变化

`ensure_available` 在 web 侧只有两个消费点（`src/product/logical_codebase/provider_gateway.rs`）：

| 消费点 | 位置 | 在判定链中的次序 |
|---|---|---|
| `revalidate_before_spawn` | `provider_gateway.rs:2716` | 第 9 步（**最后一步**） |
| `admission_verdict` | `provider_gateway.rs:2892` | capability / trust / role / roots / projection 之后 |

`validate` 阶段**不查 availability** ⇒ capability 层失败先于 availability 层。

逐一核对「用到 Pi / KimiCode 的用例」：

1. `gateway_start_parts/author_launch_cases.inc.rs:269`
   `logical_plan_launch_fails_closed_for_unsupported_author_provider`（author 循环 `[Pi, KimiCode]`）：
   断言错误详情含 `provider_capability_launch_not_confirmed`。
   该错误由 capability 层（T14ClaudeOnlyCapabilitySource）对非 ClaudeCode provider 直接拒绝产生，
   发生在 `validate` 阶段 ⇒ **availability 门根本没被问到**，换 4-provider 门不改判定。
2. `single_candidate_lc_admission.rs` 内全部 KimiCode 臂（trust 缺失 / 冲突 / 人工预登记 / Story 通用 Author 臂 等）：
   均**显式注入** `availability: Some(lc_scoped_availability_gate(true, true))`（自带 KimiCode 条目）
   ⇒ 不经过本轮被合并的默认门。
3. 其余消费默认门（`lc_always_available_gate`）的夹具——
   `author_revision_fresh.rs`、`planning_resume.rs`、`lc_fixture_support.rs`、
   `single_candidate_provider_run.rs` 的默认变体、`gateway_start.rs` 的 t14 夹具——
   发起启动的 provider 只有 **ClaudeCode / Codex**（两者在 2-provider 与 4-provider 名单里都在场且 `available: true`）。
   （`author_revision_fresh.rs:658` 的 `ProviderName::KimiCode` 只是 durable waiting 记录的 provider 字段，不经 gateway 启动。）
4. #1 / #3 的门消费者：`gateway_factory.rs` 只注册 ClaudeCode adapter，T7 只读 trust 等待面查 Codex；
   `gateway_start.rs` 只用 ClaudeCode(author) / Codex(reviewer)。两者都在两版名单内且状态相同。

⇒ **对现有测试树，换成 4-provider 共享门的可观测结果与原来逐条相同**（Pi/KimiCode 在这 4 处的
唯一入口都先被 capability 层拒绝）。

---

## 3. 改前 → 改后（逐处）

### #1 `src/web/gateway_factory.rs`

改前：`mod tests` 内私有 `fn always_available_gate() -> Arc<ProviderAvailabilityGate>`（35 行，2-provider 名单）
+ 4 处调用 `always_available_gate()`（`factory_fixture` / `lcg_t01_*` / `web_app_state` 注入 / T7 readonly）。

改后：删除私有函数与随之失效的 3 组 import
（`use chrono::Utc;`、`provider_availability_gate::{ProviderAvailabilityGate, ProviderHealthSource}`、
`provider_health::{ProviderHealthEntry, ProviderHealthSnapshot}`）；4 处调用点改为

```rust
crate::web::handlers::automation_enrollment_test_support::always_available_gate(),
```

（外层生产代码的 `use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;`
保留——它是 `LogicalCodebaseGatewayFactory` 的字段类型，属生产代码。）

### #2 `src/web/handlers/automation_gateway_preflight.rs`

改前：`mod tests` 内私有 `fn always_available_gate()`（4-provider 名单，与共享模块逐字等价）+ 1 处调用。

改后：删除私有函数（含其函数内 3 条 import）与随之失效的
`use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;`；调用点改为

```rust
crate::web::handlers::automation_enrollment_test_support::always_available_gate(),
```

### #3 `src/web/workspace_ws_handler/tests/gateway_start.rs`

改前：文件级私有 `fn always_available_gate()`（2-provider 名单）+ 1 处调用
（`gateway_fixture_with_adapter`）+ 2 组 import。

改后：删除函数与
`use crate::cross_cutting::provider_availability_gate::{ProviderAvailabilityGate, ProviderHealthSource};`、
`use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};`；
调用点改为共享模块路径。

### #4 `src/web/workspace_ws_handler/tests/single_candidate_lc_admission.rs`

改前：文件级私有 `fn lc_always_available_gate()`（2-provider 名单）+ 1 处调用
（`register_alias_logical_codebase_with_extras` 里的
`extras.availability.unwrap_or_else(lc_always_available_gate)`）。

改后：删除函数；调用点改为

```rust
let availability_gate = extras.availability.unwrap_or_else(
    crate::web::handlers::automation_enrollment_test_support::always_available_gate,
);
```

（该文件原有的 `provider_availability_gate::{ProviderAvailabilityGate, ProviderHealthSource}` 与
`provider_health::{ProviderHealthEntry, ProviderHealthSnapshot}` import **保留**——
`lc_scoped_availability_gate` 仍在用它们。）

---

## 4. 被改文件行数（改前 → 改后）

| 文件 | 改前 | 改后 | 差 |
|---|---|---|---|
| `src/web/gateway_factory.rs` | 979 | **938** | −41 |
| `src/web/handlers/automation_gateway_preflight.rs` | 1169 | **1125** | −44 |
| `src/web/workspace_ws_handler/tests/gateway_start.rs` | 441 | **401** | −40 |
| `src/web/workspace_ws_handler/tests/single_candidate_lc_admission.rs` | 2638 | **2606** | −32 |
| `src/web/handlers/automation_enrollment_test_support.rs`（共享模块） | 943 | 945 | +2（**仅文档注释**，见 §7 ②；实现零改动） |

**`tests/it_core/large_file_guard.rs` 未碰**（按 brief）。其中与本次相关的登记项为**上限**语义
（`GRANDFATHERED` 登记 `(path, cap)`，`cap` 必须是上限且 > 硬线），本次文件变小 ⇒ 仍满足上限，guard 不需改即可通过：

| 登记项 | 登记上限 | 改后实际行数 |
|---|---|---|
| `src/web/handlers/automation_gateway_preflight.rs` | 1217 | 1125 |
| `src/web/workspace_ws_handler/tests/single_candidate_lc_admission.rs` | 2638 | 2606 |

`src/web/gateway_factory.rs`、`src/web/workspace_ws_handler/tests/gateway_start.rs` 未在登记表内。
是否把登记上限收紧到新行数，由 Main 统一决定。

---

## 5. 验证

- **未跑 `cargo test` / `cargo check`**（按 brief：有 worker 在 flock 排队）。
- 已用 `rustfmt --edition 2021 --check`（独立二进制，不占 cargo 锁）逐文件验证：
  - `gateway_factory.rs` / `gateway_start.rs` / `single_candidate_lc_admission.rs`：**格式与语法零差异**；
  - `automation_gateway_preflight.rs`：仅剩 2 处**既有**空行漂移（见 §6 concern 3），本次改动 hunk 无格式问题。
- 引用面复查：`src/web` 全树 `always_available_gate` 引用点全部指向共享模块，
  无残留 `lc_always_available_gate` / 本地 `always_available_gate` 引用。

## 6. Concerns

1. **provider 名单差异（唯一实质差异）—— 已由 Main 裁决（§7 ①）**：#1/#3/#4 原副本只列
   `[ClaudeCode, Codex]`，共享模块列 `[ClaudeCode, Codex, Pi, KimiCode]`。
   **定性**：共享模块那份（4-provider）是**较新**的一份（早前 3 份合并时已逐字核对为基准），
   本地这 4 份是**旧的 2-provider 版** ⇒ 统一到 4-provider 版是**收敛方向，不是放宽缺陷**。
   **偏离清单（交付说明记入）**：「4 处旧 2-provider 门副本收敛到共享 4-provider 门 ⇒
   `ensure_available(Pi/KimiCode)` 由 `Err` 变 `Ok`；当前测试树零可观测差异
   （唯一相关用例先被 capability 层拦）」。
   ⇒ **不做**「名单可参数化」入口（那是为已被取代的旧形态留后门 = 过度设计；旧形态无存活消费者）。
2. **未跑 cargo**：本次只到 rustfmt（解析/格式）级验证，**类型检查与测试由 Main 统一跑**。
   改动本身是纯调用替换 + 删无用 import，编译风险低；但 `mod tests` 内 `use super::*` 的
   名字解析仍建议由 Main 的 `cargo check --tests` 兜一次。
3. **preflight 既有格式漂移**：`automation_gateway_preflight.rs` 第 733、906 行附近各有 1 处多余空行
   （来自其他 worker 删除代码后的残留，非本次改动引入——本次 hunk 在 304 / 485 / 650 行）。
   ⇒ **按 Main 裁决不修**，交收口统一 `cargo fmt`（`preflight.rs` 是 mod 可达文件，会被格式化）。
4. **共享模块文档注释**：已按 Main 裁决更新（见 §7 ②），**只改注释、实现零改动**。
5. **登记值**：登记为**上限**语义 ⇒ 实际行数下降后仍绿灯，**不收紧**（Main 收口时统一重测再定）。
6. **mtime 避让**：4 个目标文件最后修改时间分别为 2026-11-02 20:08:03（×3）与 2026-11-03 10:22:15，
   均早于本轮开工 20 分钟窗口；`src/web/workspace_ws_handler/tests/plan_repair_activation.rs`
   （20:21:52，被 `FixPlanRepairActivation` 改动）**全程未碰**。
7. 未执行 `git add` / `git commit` / `git stash` / `git checkout`。

---

## 7. Main 裁决与落实（2026-11-03）

**合并已被 Main 接受**（判据：4 处均在 `#[cfg(test)]` 内 + 当前零可观测差异 +
唯一涉及 Pi/KimiCode 的用例先被 capability 层拦掉）。

| # | Main 裁决 | 落实 |
|---|---|---|
| ① | 名单差异定性 = **收敛到较新的 4-provider 基准**，非放宽缺陷；**不做**「名单可参数化」入口；须记入交付说明偏离清单 | 已记入本报告 §6-1 的偏离清单；未新增任何入口 |
| ② | 共享模块文档注释**要改**（归本任务），按 Main 给定文本改，**只改注释** | 已按给定文本更新 `automation_enrollment_test_support.rs:284-291`；实现零改动（仅 `///` 行变化，文件 943→945 行） |
| ③ | preflight 的 2 处既有空行漂移**不用本任务修**，交收口 `cargo fmt` | 未碰 |
| ④ | 登记值为上限语义，行数下降 ⇒ **不用收紧** | `tests/it_core/large_file_guard.rs` 未碰 |
| ⑤ | 4 个被改文件行数已收，不在需上调清单内 | 已报（§4） |

**改动面复核（共享模块）**：`git diff` 中本轮对共享模块的改动**只有 `///` 文档注释行**
（实现体、函数签名、provider 名单、`degraded()`、entry 字段全部未动）。
`rustfmt --edition 2021 --check` 对该文件仍报 2 处**既有**签名格式漂移
（`always_available_gate` 返回类型换行、`seed_single_member_logical_codebase` 签名）——
均为前一 worker 的存量形态，**非本轮引入**，同样交收口 `cargo fmt` 统一处理。
