# Pi Provider 接入实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在日常 Workspace（Story/Design/Work Item）与 Coding Workspace 中把 Pi 作为与 Claude Code、Codex 并列的真实流式 Provider 接入，支持选择、执行、取消、会话续接、Auto/Supervised 权限监督与失败直接报告。

**Architecture:** Pi 通过 `pi --mode rpc` 子进程（JSONL over stdin/stdout）执行；`session.rs` 驱动 RPC 往返并复用 `JsonRpcPeer` 与 `ApprovalBridge`；Supervised 模式由随运行加载的 Aria 扩展（`aria-gate.ts`）通过扩展 UI request/response 阻塞工具调用实现。每个角色运行 = 一个子进程 + 一对 stdio 管道，cwd 为项目代码库目录，会话标识交由 Pi 原生机制（`~/.pi`）管理，Aria 仅持有 `session-id` 用于续接。

**Tech Stack:** Rust (tokio) 后端、TypeScript/React 前端、Pi CLI 0.83.0 (`--mode rpc`)、serde、ast-grep/CodeGraph（代码阅读）。

**Contract:** `openspec/changes/add-pi-provider/`（提交 `61b6d66`，两轮 review APPROVED）。

## Global Constraints

- 必须用中文回答；代码本身用英文。
- 遵循 TDD：每个任务先写失败测试，再实现，再验证通过。
- 🔴 Rust 构建/测试/检查命令**禁止 `-j 1`**；用 `cargo test -p <pkg>` 定向快反馈；标准命令见 `cadence/project-rules/build-test-commands.md`。
- 🔴 代码阅读大范围检索用 CodeGraph，精确结构阅读优先 `ast-grep outline`。
- **Decision 1（已获批，`4ab592f`）**：`ProviderName` 和 `ProviderType` 都加 `Pi`；`ProviderType::Pi` 只是共享类型变体，Task Runner 在 HTTP 调度入口、`RoutingProviderAdapter`、兼容性矩阵、节点契约四层显式拒绝 Pi，不调度、不路由、不执行。
- **不扩大范围**：仓库初始化、Fake Provider Workspace Runner 不动；Task Runner 可调度范围和运行行为不变。
- **fail-fast，无运行期降级**：所选 Provider 启动/运行失败直接报告失败状态，不自动切换、重放或重试到其他 Provider。
- **会话目录不干预**：不传 `--session-dir`，Pi 用默认 `~/.pi`；Claude→`~/.claude`、Codex→`~/.codex` 同理。
- **工作目录 = 项目代码库目录**：`working_dir` 传 Aria Workspace 的代码库路径，Pi 在里面执行 read/edit/bash。
- **权限模式默认 Auto**：普通 Workspace 的 Author/Reviewer 与 Coding Workspace 各角色默认 `Auto`；保留每角色 `Supervised`。
- **Pi 健康检查命令**：`pi --version`（实测输出 `0.83.0`）。

---

## 任务清单（对应 tasks.md 的 6 个工作包 + 已完成 spike）

| Task | 对应 tasks.md | 内容 |
|---|---|---|
| 0 | —（已完成） | Phase 0 Spike：Pi RPC 三项能力真实验证（已验证通过，证据见下） |
| 1 | 1.1, 1.2 | ProviderName 加 Pi + 健康检查 + 前端 catalog + 仓库初始化不受影响 |
| 2 | 2.1, 2.2 | Pi RPC 流式适配器（session/parse）+ Aria 授权扩展 |
| 3 | 3.1, 3.2 | 普通 Workspace 权限持久化 + Pi 接入（含 fail-fast） |
| 4 | 4.1, 4.2 | Coding Workspace 默认改 Auto + Pi 接入（含 fail-fast） |
| 5 | 5.1, 5.2 | 前端 Provider 目录展示 + 权限控制 + 失败状态可见性 |
| 6 | 6.1, 6.2, 6.3 | 回归测试 + 边界验证 + 前后端质量检查 |

---

## Task 0: Phase 0 Spike —— Pi RPC 三项能力验证（✅ 已完成）

> 本任务已在 Plan 编写前真实完成，契约 Decision 2 的 spike gate 已闭合。此处仅记录结论与证据，无需再执行。

**验证结果（真实跑通）：**

| 能力 | 验证方式 | 结果 |
|---|---|---|
| 会话粒度 | `get_state` 返回 `sessionId=019fb778-edcc-7086-a299-103a924aa8d6` | ✅ |
| 会话级临时扩展加载 | `-e aria-gate.ts`（本次 spike 用 `--no-extensions` 隔离排除干扰；**正式实现不加** `--no-extensions`，保留用户全局扩展） | ✅ |
| 扩展 UI 往返阻塞（放行） | `tool_execution_start` → `extension_ui_request` → 回 `confirmed:true` → `tool_execution_end` | ✅ |
| 扩展 UI 往返阻塞（拒绝） | 回 `confirmed:false` → 工具不执行，返回 `{reason:"用户拒绝"}` 作为 `isError:true` 结果 | ✅ |

**关键证据（事件顺序，证明 Supervised 阻塞成立）：**
```
tool_execution_start: read          ← pi 准备执行工具
extension_ui_request: confirm       ← 扩展在 tool_call 钩子里发 confirm，pi 阻塞等待
  （客户端回 confirmed:true）
tool_execution_end: read            ← 收到响应后工具才真正执行完
```

**Spike 产物（供 Task 2 复用）：**
- 事件流 fixture：`/tmp/pi-spike-4Vlzge/events.jsonl`（Task 2 的 parse.rs 测试可参照录制新 fixture）
- 扩展骨架：见 Task 2 Step 1 的 `aria-gate.ts`

**结论：** 契约 Decision 2 技术地基 100% 成立，备选的 JSON 输出方案不启用。可以进入实现阶段。

---

## Task 1: ProviderName/ProviderType 加 Pi + 健康检查 + 前端 catalog（tasks 1.1, 1.2, 1.3）

**Files:**
- Modify: `src/product/models/provider.rs:5`（`ProviderName` 加 `Pi` 变体）
- Modify: `src/protocol/contracts.rs:45`（`ProviderType` 加 `Pi` 变体）
- Modify: `src/product/workspace_engine/mappings.rs:29` 与 `src/product/coding_workspace_engine/tool_format.rs:106`（`provider_type_for_name` 加 `Pi => ProviderType::Pi` 分支）
- Modify: `src/task_run/provider_factory.rs:64-72`（`RoutingProviderAdapter::run` 加 `ProviderType::Pi` 拒绝臂，Task Runner 不执行 Pi）
- Modify: `src/cross_cutting/provider_health.rs`（`refresh()`、`uninitialized_snapshot()`、`real_workflow_blocked()` 加 Pi；Pi 的 `version_command` 用 `CommandSpec::new("pi", vec!["--version".to_string()])` 内联，**不经过** `adapter_compatibility` 兼容性矩阵，矩阵不为 Pi 增加条目）
- Modify: `web/src/api/types/provider.ts:1`（`RealProviderName` 加 `"pi"`）
- Modify: `web/src/state/provider-options.ts`（`REAL_PROVIDER_CATALOG` 加 `{ value: "pi", fallbackLabel: "Pi" }`）
- Test: `src/product/models/provider.rs`（内联 `#[cfg(test)]`）、`src/cross_cutting/provider_health.rs`（内联 `#[cfg(test)]`）、`src/task_run/provider_factory.rs`（内联 `#[cfg(test)]`）
- Test: `web/src/state/provider-options.test.ts`

**Interfaces:**
- Consumes: 现有 `CommandSpec::new(program: impl Into<String>, args: Vec<String>)`、`probe_provider(provider, command, checked_at, cancellation)`、`getProviderOptions(snapshot)`。
- Produces: `ProviderName::Pi`（wire value `"pi"`）；`ProviderType::Pi`（wire value `"pi"`）；`provider_type_for_name(&ProviderName::Pi) -> ProviderType::Pi`；健康检查对 Pi 用 `pi --version`。**后续任务依赖 `ProviderName::Pi` 与 `ProviderType::Pi`。**

**约束（Decision 1）：**
- `ProviderType::Pi` 只作为共享类型变体存在；Task Runner 在 HTTP 调度入口（`parse_provider_type` 拒非 claude/codex，**天然拒 `"pi"`**）、`RoutingProviderAdapter`（加拒绝臂）、兼容性矩阵（不加条目）、节点契约（不产生 Pi）四层拒绝 Pi。
- 健康检查里 Pi 的 version_command 内联（方案 A），不调用 `matrix.entry_for(ProviderType::Pi)`（矩阵无 Pi 条目，会 panic）。
- 仓库初始化只用 Claude Code 专用选项，不引用通用 catalog。

- [ ] **Step 1: 写失败测试 —— `ProviderName::Pi` 序列化**

在 `src/product/models/provider.rs` 末尾加：

```rust
#[cfg(test)]
mod tests {
    use super::ProviderName;

    #[test]
    fn provider_name_pi_serializes_to_snake_case() {
        let json = serde_json::to_string(&ProviderName::Pi).expect("serialize Pi");
        assert_eq!(json, "\"pi\"");
        let back: ProviderName = serde_json::from_str("\"pi\"").expect("deserialize pi");
        assert_eq!(back, ProviderName::Pi);
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p cadence-aria provider_name_pi_serializes_to_snake_case`
Expected: FAIL —— `no variant Pi`

- [ ] **Step 3: 在 `ProviderName` 加 `Pi` 变体**

`src/product/models/provider.rs:5`：

```rust
pub enum ProviderName {
    ClaudeCode,
    Codex,
    Pi,
    Fake,
}
```

`Pi` 放在 `Fake` 之前，保持「真实 provider 在前、fake 在后」语义。`#[serde(rename_all = "snake_case")]` 把 `Pi` 序列化成 `"pi"`。

- [ ] **Step 4: 运行测试，确认通过**

Run: `cargo test -p cadence-aria provider_name_pi_serializes_to_snake_case`
Expected: PASS

- [ ] **Step 4b: 写失败测试 —— `ProviderType::Pi` 变体 + `provider_type_for_name` 映射**

在 `src/protocol/contracts.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn provider_type_pi_serializes_to_snake_case() {
    let json = serde_json::to_string(&ProviderType::Pi).expect("serialize Pi");
    assert_eq!(json, "\"pi\"");
}
```

在 `src/product/workspace_engine/mappings.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn provider_type_for_name_maps_pi() {
    assert_eq!(provider_type_for_name(&ProviderName::Pi), ProviderType::Pi);
}
```

Run: `cargo test -p cadence-aria provider_type`
Expected: FAIL —— `ProviderType::Pi` 不存在；`provider_type_for_name` 对 `Pi` 不匹配。

- [ ] **Step 4c: 在 `ProviderType` 加 `Pi` 变体 + 两个 `provider_type_for_name` 映射**

`src/protocol/contracts.rs:45`：

```rust
pub enum ProviderType {
    ClaudeCode,
    Codex,
    Pi,
    Fake,
}
```

`src/product/workspace_engine/mappings.rs:29` 与 `src/product/coding_workspace_engine/tool_format.rs:106` 各加分支：

```rust
ProviderName::Pi => ProviderType::Pi,
```

注意：加 `ProviderType::Pi` 后，所有对 `ProviderType` 的穷尽 match 都会编译错误，需逐一补 `Pi` 分支。其中 **`RoutingProviderAdapter::run`（Step 4d）必须返回错误**（Task Runner 拒绝调度）；其他穷尽 match 按「Pi 不参与该路径」的最小语义补臂（多数返回现有"不支持"错误或跳过）。用 `cargo build -p cadence-aria 2>&1 | grep "non-exhaustive"` 定位所有需补臂处。

- [ ] **Step 4d: 写失败测试 —— Task Runner `RoutingProviderAdapter` 拒绝 Pi**

在 `src/task_run/provider_factory.rs` 的 `#[cfg(test)]` 加（参照现有 recording/no-call 测试模式 `provider_factory.rs:219-238`）：

```rust
#[test]
fn routing_adapter_rejects_pi_without_calling_real_providers() {
    let adapter = real_routing_provider_for_test(); // 现有测试构造
    let input = adapter_input_with_provider(ProviderType::Pi); // 构造 Pi 的 AdapterInput
    let result = adapter.run(&input);
    assert!(result.is_err());
    // 断言底层 claude/codex adapter 调用次数为零（recording adapter）
}
```

Run: `cargo test -p cadence-aria provider_factory`
Expected: FAIL —— `RoutingProviderAdapter::run` 对 `ProviderType::Pi` 未匹配（编译错或 panic）。

- [ ] **Step 4e: `RoutingProviderAdapter::run` 加 Pi 拒绝臂**

`src/task_run/provider_factory.rs:64-72`：

```rust
        match &input.provider_type {
            ProviderType::ClaudeCode => self.claude.run(input),
            ProviderType::Codex => self.codex.run(input),
            ProviderType::Pi => Err(ProviderAdapterError::incompatible_output(
                "task run routing provider does not schedule pi",
                String::new(),
                String::new(),
            )),
            ProviderType::Fake => Err(ProviderAdapterError::incompatible_output(
                "task run routing provider does not execute fake provider inputs",
                String::new(),
                String::new(),
            )),
        }
```

注意：`parse_provider_type`（`provider_availability.rs:185-193`）**不用改**——它只匹配 `"claude_code"`/`"codex"`，其他值（含 `"pi"`）天然返回 `web_runtime_provider_type` 错误，这正满足 spec 的「HTTP 入口拒绝 Pi」。回归测试在 Task 6.3 补。

Run: `cargo test -p cadence-aria provider_factory` → PASS

- [ ] **Step 5: 写失败测试 —— 健康检查探测 Pi**

在 `src/cross_cutting/provider_health.rs` 的 `#[cfg(test)]` 模块加：

```rust
#[test]
fn pi_version_command_uses_pi_binary() {
    let command = pi_version_command();
    assert_eq!(command.program, "pi");
    assert_eq!(command.args, vec!["--version".to_string()]);
}
```

- [ ] **Step 6: 运行测试，确认失败**

Run: `cargo test -p cadence-aria pi_version_command_uses_pi_binary`
Expected: FAIL —— `pi_version_command` 未定义

- [ ] **Step 7: 在 `provider_health.rs` 加 Pi 探测**

在 `src/cross_cutting/provider_health.rs`：

1. 加辅助函数（放 `uninitialized_snapshot` 附近）：

```rust
fn pi_version_command() -> CommandSpec {
    CommandSpec::new("pi", vec!["--version".to_string()])
}
```

2. `refresh()`（约 `provider_health.rs:176-196`）把 `tokio::join!` 扩展含 Pi：

```rust
        let claude = matrix.entry_for(ProviderType::ClaudeCode)
            .expect("default Claude compatibility entry").version_command.clone();
        let codex = matrix.entry_for(ProviderType::Codex)
            .expect("default Codex compatibility entry").version_command.clone();
        let pi = pi_version_command();

        let (claude, codex, pi) = tokio::join!(
            self.probe_provider(ProviderName::ClaudeCode, claude, checked_at, cancellation.clone()),
            self.probe_provider(ProviderName::Codex, codex, checked_at, cancellation.clone()),
            self.probe_provider(ProviderName::Pi, pi, checked_at, cancellation)
        );
        let diagnostic = Arc::new(ProviderHealthSnapshot {
            schema_version: PROVIDER_HEALTH_SCHEMA_VERSION,
            generation,
            checked_at,
            providers: vec![claude, codex, pi],
        });
```

3. `uninitialized_snapshot()`（约 `provider_health.rs:269-288`）：原 `[(ClaudeCode, ProviderType::ClaudeCode), (Codex, ProviderType::Codex)]` 循环改为显式构造，Pi 走 `pi_version_command()`（无 `ProviderType`）：

```rust
fn uninitialized_snapshot() -> ProviderHealthSnapshot {
    let checked_at = Utc.timestamp_opt(0, 0).single().expect("Unix epoch timestamp");
    let matrix = default_compatibility_matrix();
    let claude_cmd = matrix.entry_for(ProviderType::ClaudeCode).expect("default Claude entry").version_command.clone();
    let codex_cmd = matrix.entry_for(ProviderType::Codex).expect("default Codex entry").version_command.clone();
    let pi_cmd = pi_version_command();
    let providers = vec![
        unavailable_entry(ProviderName::ClaudeCode, format_command(&claude_cmd), checked_at, ProviderHealthReasonCode::CommandMissing, "not initialized".to_string()),
        unavailable_entry(ProviderName::Codex, format_command(&codex_cmd), checked_at, ProviderHealthReasonCode::CommandMissing, "not initialized".to_string()),
        unavailable_entry(ProviderName::Pi, format_command(&pi_cmd), checked_at, ProviderHealthReasonCode::CommandMissing, "not initialized".to_string()),
    ];
    ProviderHealthSnapshot { schema_version: PROVIDER_HEALTH_SCHEMA_VERSION, generation: 0, checked_at, providers }
}
```

注意：`unavailable_entry`/`format_command` 实参以现有 `provider_health.rs` 签名为准；若与上面不完全一致，**保持「Claude/Codex 走 matrix、Pi 走 `pi_version_command()`」的边界重写**，别破坏现有 snapshot 组装。

4. `real_workflow_blocked()`（约 `provider_health.rs:63`）把数组扩为含 Pi：

```rust
    pub fn real_workflow_blocked(&self) -> bool {
        [ProviderName::ClaudeCode, ProviderName::Codex, ProviderName::Pi]
            .iter()
            // 保留原有 all/any 判定逻辑，仅扩数组
    }
```

- [ ] **Step 8: 运行健康检查测试，确认通过**

Run: `cargo test -p cadence-aria provider_health`
Expected: PASS（含 `pi_version_command_uses_pi_binary`）

- [ ] **Step 9: 写失败测试 —— 前端 catalog 含 Pi**

`web/src/state/provider-options.test.ts` 加：

```ts
it("pi available 时出现在 provider 选项中", () => {
  const snapshot = {
    real_workflow_blocked: false,
    state_error: null,
    state_status: "ready" as const,
    test_provider_enabled: false,
    providers: [
      { provider: "pi", display_name: "Pi", available: true, version: "0.83.0", reason_code: null, reason: null, checked_at: "", install_hint: "" },
    ],
  };
  const options = getProviderOptions(snapshot as any);
  const pi = options.find((o) => o.value === "pi");
  expect(pi).toBeDefined();
  expect(pi?.available).toBe(true);
  expect(pi?.real).toBe(true);
});
```

- [ ] **Step 10: 运行前端测试，确认失败**

Run: `cd web && npm test provider-options`
Expected: FAIL —— `REAL_PROVIDER_CATALOG` 无 `"pi"`；`RealProviderName` 类型不含 `"pi"`。

- [ ] **Step 11: 前端类型 + catalog 加 Pi**

`web/src/api/types/provider.ts:1`：

```ts
export type RealProviderName = "claude_code" | "codex" | "pi";
```

`web/src/state/provider-options.ts`（`REAL_PROVIDER_CATALOG`）：

```ts
const REAL_PROVIDER_CATALOG: readonly RealProviderCatalogEntry[] = [
  { value: "claude_code", fallbackLabel: "Claude Code" },
  { value: "codex", fallbackLabel: "Codex" },
  { value: "pi", fallbackLabel: "Pi" },
];
```

- [ ] **Step 12: 运行前端测试，确认通过**

Run: `cd web && npm test provider-options`
Expected: PASS

- [ ] **Step 13: 验证仓库初始化不受影响**

用 CodeGraph 确认初始化页的 provider 来源：

Run: `codegraph explore "仓库初始化 / 添加代码库 页面 provider 选项来源，是否引用 REAL_PROVIDER_CATALOG"`
Expected: 初始化路径有独立 Claude Code 专用选项，**不**读 `REAL_PROVIDER_CATALOG`。若发现引用，标记到 Task 6.3 加守卫。

- [ ] **Step 14: 全量受影响测试 + Commit**

Run:
```bash
cargo test -p cadence-aria provider
cargo test -p cadence-aria provider_factory
cd web && npm test provider-options && cd ..
git add src/product/models/provider.rs src/protocol/contracts.rs src/product/workspace_engine/mappings.rs src/product/coding_workspace_engine/tool_format.rs src/task_run/provider_factory.rs src/cross_cutting/provider_health.rs web/src/api/types/provider.ts web/src/state/provider-options.ts web/src/state/provider-options.test.ts
git commit -m "feat(provider): register Pi in provider name/type, health check, frontend catalog; Task Runner rejects Pi"
```

---

## Task 2a: Aria 授权扩展 `aria-gate.ts`（tasks 2.2）

**背景决策（已定）：** Supervised 拦截必须在 Pi 扩展的 `tool_call` 事件内做（工具执行前、可 `block`），不能靠 `tool_execution_start`（工具已开始、拦截不住）。授权请求通过扩展的 `ctx.ui.confirm()` 发出，在 RPC 模式下变成 `extension_ui_request` 流到 Aria；Aria 的 `session.rs`（Task 2b）把它接到 `ApprovalBridge` 并回 `extension_ui_response`。

**Files:**
- Create: `src/cross_cutting/pi_provider/aria-gate.ts`（Aria 自带的固定授权扩展，**不是**每次运行临时生成；权限模式通过 stdin 命令或环境变量传入，见下）
- Test: 集成测试在 Task 2b 的 `tests.rs` 里（用录制 fixture 验证 UI 往返）

**Interfaces:**
- Consumes: Pi 扩展 API（`pi.on("tool_call", handler)`、`ctx.ui.confirm(title, message)`）、环境变量 `ARIA_PERMISSION_MODE`（`auto` / `supervised`）。
- Produces: 对 Supervised 模式的每次工具调用发一次 `extension_ui_request{method:"confirm"}`，收到 `confirmed:false` 时 `return { block: true, reason: "用户拒绝" }`；Auto 模式直接放行。

- [ ] **Step 1: 写 `aria-gate.ts`**

`src/cross_cutting/pi_provider/aria-gate.ts`：

```typescript
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

// Aria 注入的权限模式。每次 Pi 运行通过进程环境变量传入，不写全局配置。
const MODE = (process.env.ARIA_PERMISSION_MODE ?? "auto").toLowerCase();

export default function (pi: ExtensionAPI) {
  pi.on("tool_call", async (event, ctx) => {
    if (MODE !== "supervised") {
      // Auto：直接放行，交给 Aria 侧 ApprovalBridge 记录审计事件。
      return;
    }
    const allowed = await ctx.ui.confirm(
      "Aria 工具授权",
      `允许 Pi 执行工具 ${event.toolName}？`,
    );
    if (!allowed) {
      return { block: true, reason: "用户拒绝" };
    }
  });
}
```

注意：
- 扩展只做「Supervised 拦截 + confirm 往返」；Auto 放行不在扩展里发事件（避免重复审计，Auto 审计由 Aria 侧 `ApprovalBridge` 处理）。
- 权限模式经环境变量传入，**不写** `~/.pi` 或项目版本库（符合 Non-Goal）。

- [ ] **Step 2: 用真实 Pi 冒烟验证扩展**

参照 Task 0 的 spike 方式，起一个 RPC 会话挂这个扩展，Supervised 模式下回 `confirmed:true`，确认工具在授权后才执行（与 spike 行为一致）。这步用 Task 0 的驱动脚本改造，验证扩展本身加载与 UI 往返。

Run: 复用 `/tmp/pi-spike-drive.py` 思路，扩展路径换成 `src/cross_cutting/pi_provider/aria-gate.ts`，环境变量 `ARIA_PERMISSION_MODE=supervised`。
Expected: `extension_ui_request{method:confirm}` 出现，回 true 后 `tool_execution_end` 出现。

- [ ] **Step 3: Commit（扩展先行，session 在 Task 2b 接上）**

```bash
git add src/cross_cutting/pi_provider/aria-gate.ts
git commit -m "feat(pi): add aria-gate extension for supervised tool authorization"
```

---

## Task 2b: Pi RPC 流式适配器 `pi_provider`（tasks 2.1, 2.2）

**Files:**
- Create: `src/cross_cutting/pi_provider/mod.rs`（`PiProvider` 实现 `StreamingProviderAdapter`）
- Create: `src/cross_cutting/pi_provider/session.rs`（驱动 RPC 往返：启动会话、发 prompt、读事件流、取消、续接、UI 往返对接 `ApprovalBridge`）
- Create: `src/cross_cutting/pi_provider/parse.rs`（把 Pi 事件 JSON 映射成 `ProviderEvent`）
- Create: `src/cross_cutting/pi_provider/tests.rs`（录制 fixture 测试）
- Modify: `src/cross_cutting/mod.rs`（`pub mod pi_provider;`）
- Modify: `src/cross_cutting/provider_registry.rs`（`available_names()` 数组加 `ProviderName::Pi`）
- Test: `src/cross_cutting/pi_provider/tests.rs`

**Interfaces:**
- Consumes: `StreamingProviderInput`（`working_dir`、`prompt`、`permission_mode`、`resume_provider_session_id`、`env_vars`、`timeout_secs`）、`JsonRpcPeer`、`ApprovalBridge::request_tool(tool_name, description, risk_level, cancel)`、`CancellationToken`。
- Produces: `PiProvider::new(command: PathBuf)`、`impl StreamingProviderAdapter for PiProvider`、`ProviderName::Pi` 在 registry 可选。**后续 Task 3/4 通过 registry 选 Pi。**

**结构模板（与 codex_provider 同构）：**
- `mod.rs`：`build_args()` 构造 `pi --mode rpc` 参数；`start()` spawn 子进程 + `JsonRpcPeer` + `ApprovalBridge` + `session::run_pi_session`。
- `session.rs`：仿 `run_codex_session`，驱动初始化 → （可选）resume → prompt → 事件循环。
- `parse.rs`：仿 `parse.rs`（codex），但解析 Pi 的 JSONL 事件（`message_update`/`tool_execution_*`/`extension_ui_request`/`agent_settled`）。

- [ ] **Step 1: 写失败测试 —— parse.rs 把 Pi 文本事件映射成 ProviderEvent**

`src/cross_cutting/pi_provider/tests.rs`（用录制 JSON 而不是真起进程）：

```rust
use serde_json::json;

#[test]
fn parse_text_delta_from_message_update() {
    let event = json!({
        "type": "message_update",
        "message": {},
        "assistantMessageEvent": {
            "type": "text_delta",
            "contentIndex": 0,
            "delta": "Hello",
        }
    });
    let text = parse_pi_text_delta(&event);
    assert_eq!(text.as_deref(), Some("Hello"));
}
```

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test -p cadence-aria pi_provider`
Expected: FAIL —— `parse_pi_text_delta` 未定义 / `pi_provider` 模块不存在

- [ ] **Step 3: 建 `mod.rs` 骨架 + `parse.rs` 最小实现**

`src/cross_cutting/pi_provider/mod.rs`（骨架，后续 Step 补全）：

```rust
use std::path::PathBuf;

mod parse;
mod session;

#[cfg(test)]
pub mod tests;

pub(crate) use parse::*;

pub const PI_COMMAND: &str = "pi";

#[derive(Debug, Clone)]
pub struct PiProvider {
    command: PathBuf,
}

impl PiProvider {
    pub fn new(command: PathBuf) -> Self {
        Self { command }
    }
}
```

`src/cross_cutting/pi_provider/parse.rs`：

```rust
use serde_json::Value;

/// 从 message_update 的 assistantMessageEvent.text_delta 提取增量文本。
pub(crate) fn parse_pi_text_delta(value: &Value) -> Option<String> {
    if value.get("type")?.as_str()? != "message_update" {
        return None;
    }
    let ev = value.get("assistantMessageEvent")?;
    if ev.get("type")?.as_str()? != "text_delta" {
        return None;
    }
    ev.get("delta")?.as_str().map(ToString::to_string)
}
```

`src/cross_cutting/mod.rs` 加 `pub mod pi_provider;`。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test -p cadence-aria pi_provider`
Expected: PASS

- [ ] **Step 5: 写失败测试 —— parse.rs 解析 extension_ui_request / tool_execution_end / agent_settled**

`tests.rs` 加（覆盖 Supervised 授权请求、工具完成、会话结束三类关键事件）：

```rust
#[test]
fn parse_extension_ui_confirm_request() {
    let event = serde_json::json!({
        "type": "extension_ui_request",
        "id": "req-1",
        "method": "confirm",
        "title": "Aria 工具授权",
        "message": "允许 Pi 执行工具 bash？"
    });
    let req = parse_pi_ui_confirm_request(&event).expect("confirm request");
    assert_eq!(req.id, "req-1");
    assert_eq!(req.title, "Aria 工具授权");
}

#[test]
fn parse_tool_execution_end_result() {
    let event = serde_json::json!({
        "type": "tool_execution_end",
        "toolCallId": "call_1",
        "toolName": "bash",
        "isError": false,
        "result": { "content": [{"type":"text","text":"done"}] }
    });
    assert!(parse_pi_tool_end(&event).is_some());
}

#[test]
fn parse_agent_settled_as_terminal() {
    let event = serde_json::json!({"type": "agent_settled"});
    assert!(is_pi_terminal(&event));
}
```

- [ ] **Step 6: 运行，确认失败，再在 parse.rs 实现这三个解析函数**

Run: `cargo test -p cadence-aria pi_provider`
Expected: FAIL → 实现 `parse_pi_ui_confirm_request`（返回 `PiUiConfirmRequest { id: String, title: String, message: Option<String> }`）、`parse_pi_tool_end`、`is_pi_terminal`，再跑 PASS。

- [ ] **Step 7: 写失败测试 —— `PiProvider::build_args()` 构造正确命令行**

`tests.rs` 加：

```rust
#[test]
fn build_args_rpc_mode_with_extension() {
    let provider = PiProvider::new("pi".into());
    let args = provider.build_args(&std::path::PathBuf::from("/ext/aria-gate.ts"), None);
    assert!(args.contains(&"--mode".to_string()));
    assert!(args.contains(&"rpc".to_string()));
    assert!(args.contains(&"-e".to_string()));
    // 首次运行不传 --session-id
    assert!(!args.contains(&"--session-id".to_string()));
}

#[test]
fn build_args_resume_includes_session_id() {
    let provider = PiProvider::new("pi".into());
    let args = provider.build_args(
        &std::path::PathBuf::from("/ext/aria-gate.ts"),
        Some("sess-123"),
    );
    assert!(args.contains(&"--session-id".to_string()));
    assert!(args.contains(&"sess-123".to_string()));
}
```

- [ ] **Step 8: 实现 `build_args()`（含扩展路径、resume、不设 --session-dir、不设 --no-extensions）**

`mod.rs` 加：

```rust
impl PiProvider {
    /// 构造 pi RPC 命令行。
    /// - 不设 --session-dir：Pi 用默认 ~/.pi（与 Claude/Codex 一致）。
    /// - 不设 --no-extensions：保留用户全局扩展能力。
    /// - cwd 由 spawn 时传 working_dir（项目代码库目录）。
    pub(crate) fn build_args(
        &self,
        gate_extension: &std::path::Path,
        resume_session_id: Option<&str>,
    ) -> Vec<String> {
        let mut args = vec![
            "--mode".to_string(),
            "rpc".to_string(),
            "-e".to_string(),
            gate_extension.display().to_string(),
        ];
        if let Some(session_id) = resume_session_id.map(str::trim).filter(|s| !s.is_empty()) {
            args.push("--session-id".to_string());
            args.push(session_id.to_string());
        }
        args
    }
}
```

Run: `cargo test -p cadence-aria pi_provider` → PASS

- [ ] **Step 9: 实现 `StreamingProviderAdapter for PiProvider`（spawn + session 驱动）**

`mod.rs` 仿 `codex_provider/mod.rs` 的 `start()`：spawn `ProcessManager::spawn(&command, &args, &input.working_dir, &input.env_vars, cancel)`、`JsonRpcPeer::new(stdout, stdin)`、`ApprovalBridge::new(input.permission_mode.clone(), event_tx.clone())`、`tokio::spawn(async move { session::run_pi_session(...) })`。`ARIA_PERMISSION_MODE` 注入 `env_vars`：Supervised→`supervised`，Auto→`auto`。

注意：这步先写 session.rs 的最小驱动（初始化 → prompt → 读事件直到 `agent_settled`），取消用 `abort` 命令；UI 往返在 Step 10 接 `ApprovalBridge`。

- [ ] **Step 10: session.rs 接 UI 往返到 `ApprovalBridge`**

在事件循环里，遇到 `extension_ui_request{method:confirm}` 时：从 `message`/`title` 提取工具信息 → 调 `bridge.request_tool(tool_name, description, RiskLevel::Medium, cancel)` → 拿 `PermissionDecision{approved}` → 用 `peer.send(json!({"type":"extension_ui_response","id":req.id,"confirmed":approved}))` 回给 Pi。

`tests.rs` 加失败测试验证「confirm 请求 → ApprovalBridge 决定 → extension_ui_response」链路（用 mock peer / 录制 fixture）。

- [ ] **Step 11: 录制真实 fixture 覆盖事件流**

参照 Task 0 的 `events.jsonl`，录制一份「Auto 放行 + 一次文本输出 + 一次工具执行 + agent_settled」与「Supervised 拒绝」两条 fixture，存到 `src/cross_cutting/pi_provider/tests/fixtures/`，在 tests.rs 用它们驱动 parse + session 逻辑。

- [ ] **Step 12: 注册到 registry + 全量测试 + Commit**

`provider_registry.rs` 的 `available_names()` 数组加 `ProviderName::Pi`。

Run:
```bash
cargo test -p cadence-aria pi_provider
cargo test -p cadence-aria provider_registry
git add src/cross_cutting/pi_provider/ src/cross_cutting/mod.rs src/cross_cutting/provider_registry.rs
git commit -m "feat(pi): implement Pi RPC streaming adapter with supervised authorization bridge"
```

---

## Task 3: 普通 Workspace 权限持久化 + Pi 接入（tasks 3.1, 3.2）

**背景：** 普通 Workspace 的 `ProviderConfigSnapshot`（`common.rs:112`）目前只有 `author`/`reviewer`/`review_rounds`，**无权限模式字段**；运行链路多处把 `StreamingProviderInput.permission_mode` 硬编码为 `ProviderPermissionMode::Supervised`（`prompts.rs:154/177`、`review.rs` 多处、`review_repair.rs:49`、`revision.rs:55`）。本任务新增 per-role 权限模式持久化，默认 `Auto`，并把硬编码改为读配置。

**Files:**
- Modify: `src/web/workspace_ws_types/common.rs:112`（`ProviderConfigSnapshot` 加 `permission_modes` 字段）
- Modify: `src/product/workspace_engine/prompts.rs`、`review.rs`、`review_repair.rs`、`revision.rs`（硬编码 `Supervised` 改为读配置）
- Modify: `src/product/workspace_engine/lifecycle.rs:117,619`（快照构造含权限模式）
- Modify: `src/web/workspace_ws_types/in_.rs`（创建输入/更新消息含权限模式）
- Test: `src/web/workspace_ws_types/common.rs`（反序列化兼容）、`src/product/workspace_engine/tests/`（权限读配置）

**Interfaces:**
- Consumes: `ProviderConfigSnapshot`、`streaming_provider::ProviderPermissionMode`、`StreamingProviderInput`。
- Produces: `WorkspaceRolePermissionModes { author: ProviderPermissionMode, reviewer: ProviderPermissionMode }`（普通 Workspace 用，**独立于** Coding 的 `CodingProviderPermissionMode`，不合并）；`ProviderConfigSnapshot.permission_modes: WorkspaceRolePermissionModes`（`#[serde(default)]`，缺失按 `Auto`）。

**约束（Decision 3）：** 两套权限类型不合并；旧持久化会话缺字段按 `Auto` 反序列化；权限模式变更只影响后续启动的运行，活动会话保持启动时模式。

- [ ] **Step 1: 写失败测试 —— `WorkspaceRolePermissionModes` 默认 Auto + 旧快照缺字段反序列化**

`src/web/workspace_ws_types/common.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn workspace_role_permission_modes_default_is_auto() {
    let modes = WorkspaceRolePermissionModes::default();
    assert_eq!(modes.author, ProviderPermissionMode::Auto);
    assert_eq!(modes.reviewer, ProviderPermissionMode::Auto);
}

#[test]
fn provider_config_snapshot_without_permission_modes_deserializes_to_auto() {
    // 旧持久化记录没有 permission_modes 字段
    let json = serde_json::json!({
        "author": "claude_code",
        "reviewer": "codex",
        "review_rounds": 1
    });
    let snapshot: ProviderConfigSnapshot = serde_json::from_value(json).expect("deserialize old record");
    assert_eq!(snapshot.permission_modes.author, ProviderPermissionMode::Auto);
    assert_eq!(snapshot.permission_modes.reviewer, ProviderPermissionMode::Auto);
}
```

Run: `cargo test -p cadence-aria workspace_ws_types`
Expected: FAIL —— `WorkspaceRolePermissionModes` / `permission_modes` 字段不存在。

- [ ] **Step 2: 定义 `WorkspaceRolePermissionModes` + `ProviderConfigSnapshot` 加字段**

`src/web/workspace_ws_types/common.rs:112`：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceRolePermissionModes {
    pub author: ProviderPermissionMode,
    pub reviewer: ProviderPermissionMode,
}

impl Default for WorkspaceRolePermissionModes {
    fn default() -> Self {
        Self {
            author: ProviderPermissionMode::Auto,
            reviewer: ProviderPermissionMode::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfigSnapshot {
    pub author: ProviderName,
    pub reviewer: Option<ProviderName>,
    pub review_rounds: u32,
    #[serde(default)]
    pub permission_modes: WorkspaceRolePermissionModes,
}
```

注意：`ProviderPermissionMode`（`streaming_provider/mod.rs:34`）目前只 derive 了 `Debug, Clone, PartialEq, Eq`，**没有 Serialize/Deserialize**。需在 `streaming_provider/mod.rs:34` 加 `Serialize, Deserialize` derive 才能用于快照字段。

Run: `cargo test -p cadence-aria workspace_ws_types` → PASS

- [ ] **Step 3: 写失败测试 —— 运行链路读配置而非硬编码 Supervised**

在 `src/product/workspace_engine/tests/` 加一个测试：构造带 `permission_modes.author = Auto` 的 session，调用 `build_streaming_input`（`prompts.rs` 作者运行构造），断言返回的 `StreamingProviderInput.permission_mode == Auto`（而非硬编码 Supervised）。

Run: `cargo test -p cadence-aria workspace_engine`
Expected: FAIL —— 现有实现硬编码 `Supervised`。

- [ ] **Step 4: 把运行链路的硬编码 `Supervised` 改为读 `session` 的权限模式**

`src/product/workspace_engine/prompts.rs:154,177`、`review.rs` 各处、`review_repair.rs:49`、`revision.rs:55`：把 `permission_mode: ProviderPermissionMode::Supervised` 改为从 session 持久化的权限模式读（Author 运行读 `permission_modes.author`，Reviewer 运行读 `permission_modes.reviewer`）。

注意：先定位 session 上权限模式的存放点——若 `WorkspaceEngine` 的 session 未直接持 `permission_modes`，需从 `provider_config_snapshot` 传递。用 `ast-grep outline src/product/workspace_engine/prompts.rs --match build_streaming_input --view expanded` 看构造点能拿到什么，再决定从哪读。

Run: `cargo test -p cadence-aria workspace_engine` → PASS

- [ ] **Step 5: 接入 Pi（Author/Reviewer 可选 Pi）**

普通 Workspace 运行链路通过 registry 取 provider；Task 1/2 已让 `ProviderName::Pi` 可注册、`PiProvider` 可实现。本步确认普通 Workspace 的 Author/Reviewer/返修运行在选 Pi 时能走 `PiProvider`（含授权桥接）。

`src/web/workspace_ws_types/in_.rs`：创建输入/更新消息的 `provider_config` 已含 `ProviderConfigSnapshot`（Step 2 后含权限模式），确认 wire 层接受 `"pi"`。

测试：构造 author 选 Pi 的 session，跑 Author 运行，断言走 `PiProvider.start()`（用 recording/mocked registry）。失败直接报错（fail-fast），不切换。

- [ ] **Step 6: 全量受影响测试 + Commit**

Run:
```bash
cargo test -p cadence-aria workspace_engine
cargo test -p cadence-aria workspace_ws_types
git add src/web/workspace_ws_types/common.rs src/web/workspace_ws_types/in_.rs src/cross_cutting/streaming_provider/mod.rs src/product/workspace_engine/
git commit -m "feat(workspace): persist per-role permission mode (default Auto) and support Pi in workspace roles"
```

---

## Task 4: Coding Workspace 默认改 Auto + Pi 接入（tasks 4.1, 4.2）

**背景：** Coding Workspace 已有 per-role 权限模式结构 `CodingRolePermissionModes`（`provider_config.rs:17`），默认全 `Supervised`。本任务把新建默认值改为 `Auto`（保留旧记录的显式值），并让 Coder/Code Reviewer/Internal Reviewer 支持 Pi。

**Files:**
- Modify: `src/product/coding_models/provider_config.rs:23-31`（`CodingRolePermissionModes::default()` 全改 `Auto`）
- Modify: `src/product/coding_workspace_engine/`（Coder/Code Reviewer/Internal Reviewer 运行接 Pi + 授权桥接）
- Test: `src/product/coding_models/provider_config.rs`（默认值）、`tests/it_product/product_coding_workspace_engine/`（Pi 运行）

**Interfaces:**
- Consumes: `CodingRolePermissionModes`、`CodingProviderPermissionMode`、`CodingRoleProviderConfigSnapshot`、`PiProvider`（Task 2）。
- Produces: `CodingRolePermissionModes::default()` 全 `Auto`；Coding 三角色可选 Pi。**不改** `CodingProviderPermissionMode` 类型本身（保留独立，不合并）。

**约束（Decision 3）：** 只改**新建默认值**为 `Auto`；已持久化的显式值（Supervised）不受影响；旧记录缺 `permission_modes` 字段时 `#[serde(default)]` 反序列化按新默认（Auto）——注意确认这是否改变旧记录语义（见 Step 1）。

- [ ] **Step 1: 写失败测试 —— 新建默认 Auto**

`src/product/coding_models/provider_config.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn coding_role_permission_modes_default_is_auto() {
    let modes = CodingRolePermissionModes::default();
    assert_eq!(modes.coder, CodingProviderPermissionMode::Auto);
    assert_eq!(modes.code_reviewer, CodingProviderPermissionMode::Auto);
    assert_eq!(modes.internal_reviewer, CodingProviderPermissionMode::Auto);
}

#[test]
fn explicit_supervised_value_preserved() {
    // 已持久化的显式 Supervised 不受默认值改变影响
    let json = serde_json::json!({
        "coder": "supervised",
        "code_reviewer": "supervised",
        "internal_reviewer": "supervised"
    });
    let modes: CodingRolePermissionModes = serde_json::from_value(json).expect("deserialize");
    assert_eq!(modes.coder, CodingProviderPermissionMode::Supervised);
}
```

Run: `cargo test -p cadence-aria coding_models`
Expected: FAIL —— 默认值是 `Supervised`。

- [ ] **Step 2: `CodingRolePermissionModes::default()` 改 Auto**

`src/product/coding_models/provider_config.rs:23-31`：

```rust
impl Default for CodingRolePermissionModes {
    fn default() -> Self {
        Self {
            coder: CodingProviderPermissionMode::Auto,
            code_reviewer: CodingProviderPermissionMode::Auto,
            internal_reviewer: CodingProviderPermissionMode::Auto,
        }
    }
}
```

⚠️ **风险确认**：`CodingRoleProviderConfigSnapshot`（`provider_config.rs:47`）的 `permission_modes` 字段用 `#[serde(default)]`。改默认后，**旧持久化快照缺该字段时会按新默认 Auto 反序列化**——这正是契约 Decision 3 与 Risks 要求的（"读取时默认 Auto"）。但需确认：这是否会让原本隐式 Supervised 的旧 Coding 会话变成 Auto？用 `codegraph explore "CodingRoleProviderConfigSnapshot permission_modes 旧记录反序列化路径"` 确认 `#[serde(default)]` 命中范围，在测试里覆盖一条旧记录（缺字段）→ Auto 的用例。

Run: `cargo test -p cadence-aria coding_models` → PASS

- [ ] **Step 3: 写失败测试 —— Coding 三角色可选 Pi**

在 `tests/it_product/product_coding_workspace_engine/` 参照现有 Coding 运行测试（如 `part_04.rs`/`part_06.rs` 的 `execute_coding_*`），构造 coder 选 Pi 的 `CodingRoleProviderConfigSnapshot`，断言 Coding 运行走 `PiProvider` 并输出流式事件。失败直接报错（fail-fast）。

Run: `cargo test -p cadence-aria product_coding_workspace_engine`
Expected: FAIL —— Coding 运行链路未接 Pi。

- [ ] **Step 4: Coding 运行链路接 Pi + 授权桥接**

`src/product/coding_workspace_engine/`：Coder/Code Reviewer/Internal Reviewer 运行通过 registry 取 provider，Task 2 已让 `ProviderName::Pi` 可实现。确认 Coding 的 `StreamingProviderInput` 构造把 `permission_modes`（来自 `CodingRoleProviderConfigSnapshot`）传给 `PiProvider`，并接 `ApprovalBridge`。

注意：Coding 运行链路可能也有权限硬编码，用 `rg -n "CodingProviderPermissionMode::Supervised" src/product/coding_workspace_engine/ -g '*.rs'` 定位，改为读 `CodingRolePermissionModes` 对应角色。

Run: `cargo test -p cadence-aria product_coding_workspace_engine` → PASS

- [ ] **Step 5: 全量受影响测试 + Commit**

Run:
```bash
cargo test -p cadence-aria coding_models
cargo test -p cadence-aria product_coding_workspace_engine
git add src/product/coding_models/provider_config.rs src/product/coding_workspace_engine/
git commit -m "feat(coding): default role permission modes to Auto and support Pi in coding roles"
```

---

## Task 5: 前端 Provider 目录展示 + 权限控制 + 失败状态可见性（tasks 5.1, 5.2）

**背景：** Coding Workspace 前端已有权限控制 UI（`CodingProviderConfigPanel.tsx`，含 `permissionMode` 选择 + `auto`/`supervised` 文案）；普通 Workspace 前端**无**权限控制 UI（Task 3 才给后端加字段）。Task 1 已让 catalog 含 Pi。本任务：普通 Workspace 新增权限控制 UI、两侧确认 Pi 出现在选择器、失败状态可见。

**Files:**
- Modify: `web/src/components/workspace/ProviderConfigPanel.tsx`（普通 Workspace 加 Author/Reviewer 权限模式选择 + Pi 选项）
- Modify: `web/src/components/coding-workspace/CodingProviderConfigPanel.tsx`（确认 Pi 出现在已有权限控件；文案统一）
- Modify: `web/src/api/types/coding.ts` / 普通 Workspace 的 wire 类型（含 `"pi"` 与新权限字段）
- Test: `web/src/components/workspace/ProviderConfigPanel.test.tsx`、`web/src/components/coding-workspace/CodingProviderConfigPanel.test.tsx`

**Interfaces:**
- Consumes: `getProviderOptions(snapshot)`（Task 1，已含 Pi）、`ProviderConfigSnapshot.permission_modes`（Task 3）、`CodingRolePermissionModes`（Task 4）。
- Produces: 普通 Workspace Provider 配置面板含每角色 `Auto`/`Supervised` 选择；Pi 作为可选 Provider；不可用 Pi 显示禁用 + 原因/安装提示；运行失败状态在界面可见。

- [ ] **Step 1: 写失败测试 —— 普通 Workspace 面板展示 Pi + 权限模式选择**

`web/src/components/workspace/ProviderConfigPanel.test.tsx` 加：

```ts
it("author 可选 Pi 并选择权限模式", () => {
  const snapshot = { /* 含 pi available 的健康快照 */ } as any;
  render(<ProviderConfigPanel healthSnapshot={snapshot} /* ... */ />);
  // 断言 author provider 选择器含 Pi 选项
  // 断言 author 权限模式可切换 Auto/Supervised
});
```

Run: `cd web && npm test ProviderConfigPanel`
Expected: FAIL —— 面板无权限模式控件 / Pi 未传入。

- [ ] **Step 2: 普通 Workspace 面板加权限模式控件 + Pi**

参照 `CodingProviderConfigPanel.tsx` 的权限选择 UI（`:98` `permissionMode` 选择、`:50-51` `auto`/`supervised` 文案），在 `ProviderConfigPanel.tsx` 为 Author/Reviewer 各加一个权限模式选择（Auto/Supervised），数据源为 `ProviderConfigSnapshot.permission_modes`，provider 选择器数据源为 `getProviderOptions(snapshot)`（已含 Pi）。文案与 Coding 侧统一。

Run: `cd web && npm test ProviderConfigPanel` → PASS

- [ ] **Step 3: Coding 面板确认 Pi 出现在权限控件**

`CodingProviderConfigPanel.tsx` 已按 `getProviderOptions` 渲染 provider；确认 Task 1 的 catalog 改动让 Pi 出现在 Coder/Code Reviewer/Internal Reviewer 选择器。补测试断言三角色均可选 Pi。

Run: `cd web && npm test CodingProviderConfigPanel` → PASS

- [ ] **Step 4: 不可用 Pi 显示禁用 + 原因；运行失败状态可见**

确认：健康检查报 Pi 不可用时，选择器保留已配置 Pi 但禁用，并显示 `reason`/`install_hint`（复用现有 `blockedReason`/`realProviderOption` 逻辑，Task 1 已覆盖）。运行失败状态通过现有 `ProviderEvent` → 前端状态链路显示（fail-fast：失败即显示失败，不显示切换）。

补测试：Pi 不可用时选项 disabled 且显示原因；provider 失败后界面呈失败状态。

Run: `cd web && npm test` （相关套件）→ PASS

- [ ] **Step 5: 前端全量测试 + Commit**

Run:
```bash
cd web && npm test && npm run build && cd ..
git add web/src/components/workspace/ProviderConfigPanel.tsx web/src/components/coding-workspace/CodingProviderConfigPanel.tsx web/src/api/types/
git commit -m "feat(web): show Pi and per-role Auto/Supervised controls in workspace and coding provider config"
```

---

## Task 6: 回归验证 + 边界验证 + 前后端质量检查（tasks 6.1, 6.2, 6.3）

**Files:**
- Test: `src/cross_cutting/pi_provider/tests.rs`（健康/目录/会话协议/取消/恢复/双授权模式）
- Test: `src/product/workspace_engine/tests/`、`tests/it_product/product_coding_workspace_engine/`（Provider/权限/fail-fast）
- Test: `src/task_run/provider_factory.rs`（Task Runner 拒绝 Pi）、`src/web/provider_availability.rs`（HTTP 入口拒绝 Pi）
- Test: 仓库初始化不受影响

**Interfaces:**
- Consumes: 全部前序任务的实现。
- Produces: 完整回归测试覆盖契约所有 requirement 与 scenario。

- [ ] **Step 1: Pi 后端协议回归（tasks 6.1）**

`src/cross_cutting/pi_provider/tests.rs` 补齐：
- 健康检查（`pi --version` 解析，Task 1）
- 目录展示（Task 1）
- 会话协议：文本流、工具事件、完成、错误映射（Task 2，录制 fixture）
- 取消：`abort` 命令 → 会话终止 → 前端呈已取消状态
- 恢复：`--session-id` 续接（Task 2）
- Auto 授权：直接放行 + 审计事件
- Supervised 授权：confirm 请求 → 前端决定 → 放行/拒绝（含拒绝后工具不执行、记拒绝决定）

Run: `cargo test -p cadence-aria pi_provider` → PASS

- [ ] **Step 2: Workspace/Coding 角色回归（tasks 6.2）**

为 Story/Design/Work Item 三入口（共享 `workspace_engine`）及 Coding 角色补：
- Provider 选择（含 Pi）
- 权限模式（Auto/Supervised 读配置）
- **fail-fast**：所选 Provider 启动失败 → 直接报告失败状态、备用 Provider 零调用（启动失败用 `start()` 返回 `Err` 构造，参照 `streaming_provider/mod.rs:278-295`；运行中失败用 `ProviderEvent::Failed` 构造，参照 `src/web/test_controls/provider.rs:343-349`）

参照 reviewer 验证过的注入点：`provider_registry.rs:21-44` 注册测试 provider；`tests/it_product/product_coding_workspace_engine/part_10.rs:371-383` 已有启动失败替身先例。

Run: `cargo test -p cadence-aria workspace_engine product_coding_workspace_engine` → PASS

- [ ] **Step 3: 边界验证 —— 仓库初始化与 Task Runner 未被扩张（tasks 6.3）**

- 仓库初始化：确认添加代码库/初始化只用 Claude Code 专用选项，不含 Pi（Task 1 Step 13 的 CodeGraph 结论落地为测试）。
- Task Runner 拒绝 Pi（四层）：
  - HTTP 入口：`parse_provider_type("pi")` 返回 `web_runtime_provider_type` 错误（`provider_availability.rs:185-193`），错误文本含 `pi`
  - Router：`RoutingProviderAdapter` 拒 `ProviderType::Pi` 且 adapter 不调用（Task 1 Step 4d）
  - 兼容性矩阵：`default_compatibility_matrix().entry_for(ProviderType::Pi)` 为 `None`
  - 节点契约：静态节点契约不产生 Pi（断言无 `ProviderType::Pi` 出现于契约定义）

Run: `cargo test -p cadence-aria provider_factory task_run provider_availability` → PASS

- [ ] **Step 4: 前后端质量检查 + 契约同步**

```bash
# 后端
cargo test -p cadence-aria
cargo clippy -p cadence-aria --all-targets
cargo fmt --check
# 前端
cd web && npm test && npm run build && cd ..
# 契约 tasks 勾选（openspec）
```

按 `cadence/project-rules/build-test-commands.md` 的标准命令执行（🔴 禁止 `-j 1`）。全部通过后，勾选 `openspec/changes/add-pi-provider/tasks.md` 对应工作包。

- [ ] **Step 5: Final Commit**

```bash
git add -A
git commit -m "test(pi): regression coverage for Pi protocol, workspace/coding roles, and Task Runner boundary"
```

---

## 自检（Self-Review）

**1. Spec 覆盖：**
- Requirement「Pi 可发现可选择」→ Task 1（catalog/健康）+ Task 5（选择器）✅
- Requirement「流式执行 + 控制」→ Task 2（session/取消/恢复）✅
- Requirement「权限模式默认 Auto + 按角色监督」→ Task 3（普通 Workspace）+ Task 4（Coding）✅
- Requirement「失败直接报告不切换」→ Task 3/4（fail-fast）+ Task 6 Step 2（回归）✅
- Requirement「不扩大初始化/Task Runner」→ Task 1（Task Runner 拒绝）+ Task 6 Step 3（边界）✅

**2. 占位符扫描：** 无 TBD/TODO/「适当错误处理」等占位。部分 Steps 标注了「以现有代码风格重写」的注意事项，均给了具体边界而非空泛指示。

**3. 类型一致性：** `ProviderName::Pi`/`ProviderType::Pi`（Task 1）→ Task 2/3/4/6 一致引用；`WorkspaceRolePermissionModes`（Task 3）独立于 `CodingRolePermissionModes`（Task 4），不合并；`pi_version_command()`（Task 1）贯穿健康检查。
