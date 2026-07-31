# Pi Provider 接入实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在日常 Workspace（Story/Design/Work Item）与 Coding Workspace 中把 Pi 作为与 Claude Code、Codex 并列的真实流式 Provider 接入，支持选择、执行、取消、会话续接、Auto/Supervised 权限监督与失败直接报告。

**Architecture:** Pi 通过 `pi --mode rpc` 子进程（JSONL over stdin/stdout）执行；`session.rs` 驱动 RPC 往返并复用 `JsonRpcPeer` 与 `ApprovalBridge`；Supervised 模式由 Aria 随附的固定授权扩展（`aria-gate.ts`）在 `tool_call` 事件里发 `ctx.ui.confirm()`，RPC 下变成 `extension_ui_request/response` 往返阻塞工具执行。每个角色运行 = 一个子进程 + 一对 stdio 管道，cwd 为项目代码库目录，会话标识交由 Pi 原生机制（`~/.pi`）管理，Aria 仅持有 `session-id` 用于续接。

**Tech Stack:** Rust (tokio) 后端、TypeScript/React 前端、Pi CLI 0.83.0 (`--mode rpc`)、serde、ast-grep/CodeGraph（代码阅读）。

**Contract:** `openspec/changes/add-pi-provider/`（最新提交 `13e8a4d`，Decision 1 经 4 轮 review APPROVED；Decision 2 固定扩展 + fail-fast 澄清已并入）。

## Global Constraints

- 必须用中文回答；代码本身用英文。
- 遵循 TDD：每个任务先写失败测试，再实现，再验证通过。
- 🔴 Rust 构建/测试/检查命令**禁止 `-j 1`**；用 `cargo test -p cadence-aria <name>` 定向快反馈；标准命令见 `cadence/project-rules/build-test-commands.md`。
- 🔴 代码阅读大范围检索用 CodeGraph，精确结构阅读优先 `ast-grep outline`。
- **Decision 1（已获批）**：`ProviderName` 和 `ProviderType` 都加 `Pi`；`ProviderType::Pi` 只是共享类型变体，Task Runner 在 HTTP 调度入口、`RoutingProviderAdapter`、兼容性矩阵、节点契约四层显式拒绝 Pi，不调度、不路由、不执行。
- **Decision 2（已获批）**：授权扩展是 Aria 随附的**固定** `aria-gate.ts`，不每次运行临时生成；权限模式经环境变量 `ARIA_PERMISSION_MODE=auto|supervised` 按运行注入，不写全局 Pi 配置或项目版本库。
- **fail-fast 边界（已澄清）**：禁「切换/重放/重试到**其他** Provider」，**不禁**同 Provider 内部重试（Claude/Codex 现有 artifact retry、resume-stall fresh retry 保留）。Pi 不实现同 Provider 内部重试：启动或运行失败即终态失败。
- **不扩大范围**：仓库初始化、Fake Provider Workspace Runner 不动；Task Runner 可调度范围和运行行为不变。
- **会话目录不干预**：不传 `--session-dir`，Pi 用默认 `~/.pi`。
- **工作目录 = 项目代码库目录**：`working_dir` 传 Aria Workspace 的代码库路径。
- **权限模式默认 Auto**：普通 Workspace 的 Author/Reviewer 与 Coding Workspace 各角色默认 `Auto`；保留每角色 `Supervised`。
- **Pi 健康检查命令**：`pi --version`。

---

## 任务清单

| Task | 对应 tasks.md | 内容 |
|---|---|---|
| 0 | —（已完成） | Phase 0 Spike：Pi RPC 三项能力真实验证 |
| 1 | 1.1, 1.2, 1.3 | ProviderName/ProviderType 加 Pi + 穷尽 match 盘点 + 健康检查 + 状态 API + 前端 catalog + Task Runner 拒绝 |
| 2 | 2.1, 2.2 | aria-gate.ts 扩展 + Pi RPC 协议冻结 fixture + 流式适配器 + 生产/测试 registry 注册 |
| 3 | 3.1, 3.2 | 普通 Workspace 权限持久化（真实体）+ Pi 接入 + fail-fast |
| 4 | 4.1, 4.2 | Coding Workspace 默认 Auto + Pi 接入 + fail-fast |
| 5 | 5.1, 5.2 | 前端 Provider 目录 + 权限控制 + 失败状态可见性 |
| 6 | 6.1, 6.2, 6.3 | 回归测试 + 边界验证 + 前后端质量检查 |

---

## Task 0: Phase 0 Spike —— Pi RPC 三项能力验证（✅ 已完成）

> 已在 Plan 编写前真实完成，契约 Decision 2 的 spike gate 已闭合。此处仅记录结论，无需再执行。

| 能力 | 验证方式 | 结果 |
|---|---|---|
| 会话粒度 | `get_state` 返回 `sessionId` | ✅ |
| 加载 Aria 授权扩展 | `-e aria-gate.ts` | ✅ |
| 扩展 UI 往返阻塞（放行） | `tool_execution_start` → `extension_ui_request` → 回 `confirmed:true` → `tool_execution_end` | ✅ |
| 扩展 UI 往返阻塞（拒绝） | 回 `confirmed:false` → 工具不执行，返回 `{reason:"用户拒绝"}` 为 `isError:true` | ✅ |

**结论：** 契约 Decision 2 技术地基 100% 成立，备选 JSON 输出方案不启用。Task 2 的协议 fixture 将参照此 spike 重新录制并提交到仓库（不依赖 `/tmp`）。

---

## Task 1: ProviderName/ProviderType 加 Pi + 穷尽 match + 健康检查 + 状态 API + 前端 catalog（tasks 1.1, 1.2, 1.3）

**Files:**
- Modify: `src/product/models/provider.rs:5`（`ProviderName` 加 `Pi`）
- Modify: `src/protocol/contracts.rs:45`（`ProviderType` 加 `Pi`）
- Modify: `src/product/workspace_engine/mappings.rs:29`、`src/product/coding_workspace_engine/tool_format.rs:106`、`src/product/provider_workspace_runner.rs:130`、`src/product/work_item_split_engine/types.rs:129`、`src/product/work_item_projection/render.rs:183`、`src/task_run/step_runner.rs:111`、`src/web/handlers/dto.rs:742`（穷尽 match 补臂，见 Step 6 分类表）
- Modify: `src/task_run/provider_factory.rs:64-72`（`RoutingProviderAdapter::run` 加 Pi 拒绝臂）
- Modify: `src/cross_cutting/provider_health.rs`（`refresh()`/`uninitialized_snapshot()`/`real_workflow_blocked()` 加 Pi）
- Modify: `src/web/handlers/providers.rs`（状态 API 数组 + `provider_dto()` 加 Pi）
- Modify: `web/src/api/types/provider.ts:1`（`RealProviderName` 加 `"pi"`）、`web/src/state/provider-options.ts`（catalog 加 Pi）
- Test: 上述各文件内联 `#[cfg(test)]` + `web/src/state/provider-options.test.ts`

**Interfaces:**
- Consumes: `CommandSpec::new`、`probe_provider`、`getProviderOptions`、`ProviderStatusResponse`。
- Produces: `ProviderName::Pi`/`ProviderType::Pi`（wire `"pi"`）；`provider_type_for_name(&ProviderName::Pi) -> ProviderType::Pi`；健康检查探测 `pi --version`；状态 API 返回 Pi 条目。**Task 2/3/4/5 依赖这些类型与状态 API。**

**约束（Decision 1）：** `ProviderType::Pi` 只是共享类型变体；Task Runner 四层显式拒绝。健康检查 Pi version_command 内联（方案 A），不调 `matrix.entry_for(ProviderType::Pi)`。仓库初始化只用 Claude Code 专用选项。

- [ ] **Step 1: 写失败测试 —— `ProviderName::Pi` 序列化**

`src/product/models/provider.rs` 末尾加：

```rust
#[cfg(test)]
mod tests {
    use super::ProviderName;

    #[test]
    fn provider_name_pi_serializes_to_snake_case() {
        assert_eq!(serde_json::to_string(&ProviderName::Pi).unwrap(), "\"pi\"");
        let back: ProviderName = serde_json::from_str("\"pi\"").unwrap();
        assert_eq!(back, ProviderName::Pi);
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p cadence-aria provider_name_pi_serializes_to_snake_case`
Expected: FAIL —— `no variant Pi`

- [ ] **Step 3: `ProviderName` 加 `Pi` + `ProviderType` 加 `Pi`**

`src/product/models/provider.rs:5` 与 `src/protocol/contracts.rs:45`，各自在 `Codex` 后、`Fake` 前插入 `Pi`：

```rust
pub enum ProviderName { ClaudeCode, Codex, Pi, Fake }
pub enum ProviderType { ClaudeCode, Codex, Pi, Fake }
```

- [ ] **Step 4: 运行确认通过**

Run: `cargo test -p cadence-aria provider_name_pi_serializes_to_snake_case`
Expected: PASS

- [ ] **Step 5: 写失败测试 —— `provider_type_for_name` 映射 Pi**

`src/product/workspace_engine/mappings.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn provider_type_for_name_maps_pi() {
    assert_eq!(provider_type_for_name(&ProviderName::Pi), ProviderType::Pi);
}
```

Run: `cargo test -p cadence-aria provider_type_for_name_maps_pi`
Expected: FAIL —— `provider_type_for_name` 对 `Pi` 不匹配（穷尽 match 编译错或返回错误）

- [ ] **Step 6: 穷尽 match 完整盘点并按分类补臂**

先列出全部穷尽 match（不靠编译报错逐个碰）：

```bash
rg -n "ProviderName::(ClaudeCode|Codex|Fake)" src/ -g '*.rs' | grep -v test
rg -n "ProviderType::(ClaudeCode|Codex|Fake)" src/ -g '*.rs' | grep -v test
```

按下表分类补臂：

| 位置 | Pi 策略 | 补臂代码 |
|---|---|---|
| `workspace_engine/mappings.rs:29` `provider_type_for_name` | 映射 | `ProviderName::Pi => ProviderType::Pi,` |
| `coding_workspace_engine/tool_format.rs:106` `provider_type_for_name` | 映射 | `ProviderName::Pi => ProviderType::Pi,` |
| `work_item_split_engine/types.rs:129` `provider_type_for_name` | 映射 | `ProviderName::Pi => ProviderType::Pi,` |
| `provider_workspace_runner.rs:130` `provider_type_for_name`（legacy Fake runner） | 拒绝 | `ProviderName::Pi => unreachable!("legacy fake runner does not support pi"),` 或返回既有不支持错误 |
| `work_item_projection/render.rs:183` `renderer_for(provider)` | 映射到与现有 provider 相同 renderer | `ProviderName::Pi => renderer_for_claude(),`（与 Claude 同一 renderer，Coding 需要 Pi） |
| `web/handlers/dto.rs:742` provider wire 文本化 | 映射 | `"pi"` |
| `task_run/step_runner.rs:111` 节点契约文本化 | 不出现于静态节点契约 | 保持只文本化 Claude/Codex/Fake；Pi 不在此处产生 |
| 仓库初始化 provider 选择 | Claude-only | 不加 Pi 分支 |

每个补臂对应一个测试锁定语义（映射值正确 / 拒绝返回错误 / 不加分支）。

- [ ] **Step 7: 运行确认通过**

Run: `cargo test -p cadence-aria provider_type_for_name_maps_pi`
Expected: PASS

- [ ] **Step 8: 写失败测试 —— Task Runner router 拒绝 Pi**

`src/task_run/provider_factory.rs` 的 `#[cfg(test)]`（参照现有 recording/no-call 模式 `provider_factory.rs:219-238`）加：

```rust
#[test]
fn routing_adapter_rejects_pi_without_calling_real_providers() {
    let adapter = real_routing_provider_for_test();
    let input = adapter_input_with_provider(ProviderType::Pi);
    let result = adapter.run(&input);
    assert!(result.is_err());
    // recording adapter 断言底层 claude/codex 调用次数为零
}
```

Run: `cargo test -p cadence-aria provider_factory`
Expected: FAIL —— `RoutingProviderAdapter::run` 对 `ProviderType::Pi` 未匹配

- [ ] **Step 9: `RoutingProviderAdapter::run` 加 Pi 拒绝臂**

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

Run: `cargo test -p cadence-aria provider_factory` → PASS

- [ ] **Step 10: 写失败测试 —— 健康检查探测 Pi**

`src/cross_cutting/provider_health.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn pi_version_command_uses_pi_binary() {
    let command = pi_version_command();
    assert_eq!(command.program, "pi");
    assert_eq!(command.args, vec!["--version".to_string()]);
}
```

Run: `cargo test -p cadence-aria pi_version_command_uses_pi_binary`
Expected: FAIL —— `pi_version_command` 未定义

- [ ] **Step 11: 健康检查加 Pi 探测**

`src/cross_cutting/provider_health.rs`：

1. 加辅助函数：

```rust
fn pi_version_command() -> CommandSpec {
    CommandSpec::new("pi", vec!["--version".to_string()])
}
```

2. `refresh()` 把 `tokio::join!` 扩为含 Pi（`providers: vec![claude, codex, pi]`）。
3. `uninitialized_snapshot()` 显式构造三条目（Claude/Codex 走 matrix，Pi 走 `pi_version_command()`）。
4. `real_workflow_blocked()` 数组扩为 `[ClaudeCode, Codex, Pi]`。

Run: `cargo test -p cadence-aria provider_health` → PASS

- [ ] **Step 12: 写失败测试 —— 状态 API 返回 Pi**

`src/web/handlers/providers.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn provider_status_includes_pi_when_available() {
    // 构造含 Pi 可用条目的健康快照，调 response_from_snapshot
    // 断言返回 providers 含 provider == "pi" 且 display_name == "Pi"
}
```

Run: `cargo test -p cadence-aria providers`
Expected: FAIL —— `response_from_snapshot` 只枚举 Claude/Codex；`provider_dto` 无 Pi 分支

- [ ] **Step 13: 状态 API 加 Pi**

`src/web/handlers/providers.rs`：
1. `response_from_snapshot()` 的数组 `[ProviderName::ClaudeCode, ProviderName::Codex]` 扩为含 `ProviderName::Pi`。
2. `provider_dto()` 的 match 加：

```rust
        ProviderName::Pi => (
            "pi",
            "Pi",
            "Install Pi CLI and ensure `pi` is available on PATH.",
        ),
```

Run: `cargo test -p cadence-aria providers` → PASS

- [ ] **Step 14: 写失败测试 —— 前端 catalog 含 Pi**

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

Run: `cd web && npm test provider-options`
Expected: FAIL —— catalog 无 `"pi"`；`RealProviderName` 类型不含 `"pi"`

- [ ] **Step 15: 前端类型 + catalog 加 Pi**

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

Run: `cd web && npm test provider-options` → PASS

- [ ] **Step 16: 验证仓库初始化不受影响**

Run: `codegraph explore "仓库初始化 / 添加代码库 页面 provider 选项来源，是否引用 REAL_PROVIDER_CATALOG"`
Expected: 初始化路径有独立 Claude Code 专用选项，不读 `REAL_PROVIDER_CATALOG`。

- [ ] **Step 17: 全量测试 + Commit**

Run:
```bash
cargo test -p cadence-aria provider
cargo test -p cadence-aria provider_factory
cargo test -p cadence-aria providers
cd web && npm test provider-options && cd ..
git add src/product/models/provider.rs src/protocol/contracts.rs src/product/workspace_engine/mappings.rs src/product/coding_workspace_engine/tool_format.rs src/product/provider_workspace_runner.rs src/product/work_item_split_engine/types.rs src/product/work_item_projection/render.rs src/task_run/provider_factory.rs src/task_run/step_runner.rs src/cross_cutting/provider_health.rs src/web/handlers/providers.rs src/web/handlers/dto.rs web/src/api/types/provider.ts web/src/state/provider-options.ts web/src/state/provider-options.test.ts
git commit -m "feat(provider): register Pi across provider name/type, health, status API, frontend catalog; Task Runner rejects Pi"
```

---

## Task 2: Aria 授权扩展 + Pi RPC 流式适配器 + registry 注册（tasks 2.1, 2.2）

**背景决策：** Supervised 拦截必须在 Pi 扩展的 `tool_call` 事件内做（工具执行前、可 `block`），不能靠 `tool_execution_start`（工具已开始、拦截不住）。授权请求经 `ctx.ui.confirm()` 发出，RPC 下变成 `extension_ui_request` 流到 Aria；`session.rs` 把它接到 `ApprovalBridge` 并回 `extension_ui_response`。授权扩展是**固定** `aria-gate.ts`，权限模式经环境变量注入。

**Files:**
- Create: `src/cross_cutting/pi_provider/aria-gate.ts`（固定授权扩展）
- Create: `src/cross_cutting/pi_provider/mod.rs`（`PiProvider` 实现 `StreamingProviderAdapter`）
- Create: `src/cross_cutting/pi_provider/session.rs`（驱动 RPC 往返）
- Create: `src/cross_cutting/pi_provider/parse.rs`（Pi 事件 JSON → `ProviderEvent`）
- Create: `src/cross_cutting/pi_provider/tests.rs` + `src/cross_cutting/pi_provider/tests/fixtures/*.jsonl`（协议冻结 fixture）
- Modify: `src/cross_cutting/mod.rs`（`pub mod pi_provider;`）
- Modify: `src/web/state.rs:296-339`（`default_provider_registry()` 生产 + 测试分支注册 Pi）

**Interfaces:**
- Consumes: `StreamingProviderInput`、`JsonRpcPeer`、`ApprovalBridge::request_tool(tool_name, description, risk_level, cancel)`、`CancellationToken`、`ProviderRegistry::register_gated`。
- Produces: `PiProvider::new(command: PathBuf)`、`impl StreamingProviderAdapter for PiProvider`、生产/测试 registry 含 `ProviderName::Pi`。**Task 3/4 经 registry 选 Pi。**

### Task 2a: 协议冻结 fixture（先于实现）

- [ ] **Step 1: 录制并提交协议冻结 fixture**

参照 Task 0 spike，用真实 Pi 录制三条 JSONL 会话，提交到 `src/cross_cutting/pi_provider/tests/fixtures/`（**不**用 `/tmp`）：

1. `auto_allow.jsonl`：Auto 模式，一次文本输出 + 一次工具执行 + `agent_settled`。
2. `supervised_allow.jsonl`：Supervised，一次 `extension_ui_request(confirm)` → 回 `confirmed:true` → 工具执行。
3. `supervised_deny.jsonl`：Supervised，回 `confirmed:false` → 工具被 block，返回 `{reason:"用户拒绝"}` 为 `isError:true`。

每行标注方向（client→Pi 为命令，Pi→client 为事件/响应）。录制命令参照 Task 0 的驱动脚本，但把 `ARIA_PERMISSION_MODE` 设为对应模式，扩展用 Task 2b 的 `aria-gate.ts`。

- [ ] **Step 2: 从 fixture 提取协议包络常量**

从 fixture 确认并写成 `parse.rs` 的常量/函数：
- Pi 命令响应包络：`{"type":"response","id":...,"command":...,"success":...}`（**不是** JSON-RPC 的 `result`）。
- `JsonRpcPeer::request()` 的 response 判别（`json_rpc_peer.rs:52-110`）能否识别该包络；若不能，Pi 的命令响应走 `send()` + 事件循环按 `id` 匹配，不用 `request()`。**先写一个测试验证 `request()` 是否兼容 Pi 响应包络，再决定。**

测试（`tests.rs`）：解析 fixture 里每行的 `type` 字段，断言能识别 `response`/`message_update`/`tool_execution_*`/`extension_ui_request`/`agent_settled`。

Run: `cargo test -p cadence-aria pi_provider`
Expected: FAIL —— `pi_provider` 模块不存在

### Task 2b: aria-gate.ts 扩展

- [ ] **Step 3: 写 `aria-gate.ts`**

`src/cross_cutting/pi_provider/aria-gate.ts`：

```typescript
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

// Aria 注入的权限模式。每次 Pi 运行通过进程环境变量传入，不写全局配置。
const MODE = (process.env.ARIA_PERMISSION_MODE ?? "auto").toLowerCase();

export default function (pi: ExtensionAPI) {
  pi.on("tool_call", async (event, ctx) => {
    if (MODE !== "supervised") {
      return; // Auto：直接放行，审计由 Aria 侧 ApprovalBridge 记录
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

- [ ] **Step 4: 真实冒烟验证扩展（Supervised 放行 + 拒绝）**

参照 Task 0 驱动脚本，用本扩展在 Supervised 模式下分别回 `confirmed:true`/`false`，确认放行/拒绝行为与 fixture 一致。

Expected: 放行时 `tool_execution_start` → `extension_ui_request` → `tool_execution_end`；拒绝时工具不执行、`isError:true`。

### Task 2c: parse.rs（事件映射）

- [ ] **Step 5: 写失败测试 —— parse 文本增量 / confirm 请求 / 工具完成 / 终止**

`tests.rs` 用 fixture 驱动：

```rust
#[test]
fn parse_text_delta_from_message_update() {
    let event = serde_json::json!({
        "type": "message_update",
        "assistantMessageEvent": { "type": "text_delta", "contentIndex": 0, "delta": "Hello" }
    });
    assert_eq!(parse_pi_text_delta(&event).as_deref(), Some("Hello"));
}

#[test]
fn parse_extension_ui_confirm_request() {
    let event = serde_json::json!({
        "type": "extension_ui_request", "id": "req-1", "method": "confirm",
        "title": "Aria 工具授权", "message": "允许 Pi 执行工具 bash？"
    });
    let req = parse_pi_ui_confirm_request(&event).unwrap();
    assert_eq!(req.id, "req-1");
}

#[test]
fn parse_agent_settled_as_terminal() {
    assert!(is_pi_terminal(&serde_json::json!({"type": "agent_settled"})));
}
```

Run: `cargo test -p cadence-aria pi_provider`
Expected: FAIL —— parse 函数未定义

- [ ] **Step 6: 实现 parse.rs**

`src/cross_cutting/pi_provider/parse.rs` 实现 `parse_pi_text_delta`、`parse_pi_ui_confirm_request`（返回 `PiUiConfirmRequest { id: String, title: String, message: Option<String> }`）、`parse_pi_tool_end`、`is_pi_terminal`。`src/cross_cutting/mod.rs` 加 `pub mod pi_provider;`。

Run: `cargo test -p cadence-aria pi_provider` → PASS

### Task 2d: mod.rs + session.rs（适配器）

- [ ] **Step 7: 写失败测试 —— `build_args()`**

`tests.rs` 加：

```rust
#[test]
fn build_args_rpc_mode_with_extension_no_session_dir_no_no_extensions() {
    let provider = PiProvider::new("pi".into());
    let args = provider.build_args(&std::path::PathBuf::from("/ext/aria-gate.ts"), None);
    assert!(args.contains(&"--mode".to_string()));
    assert!(args.contains(&"rpc".to_string()));
    assert!(args.contains(&"-e".to_string()));
    assert!(!args.contains(&"--session-dir".to_string()));       // Pi 用默认 ~/.pi
    assert!(!args.contains(&"--no-extensions".to_string()));      // 保留用户全局扩展
    assert!(!args.contains(&"--session-id".to_string()));         // 首次运行不传
}

#[test]
fn build_args_resume_includes_session_id() {
    let provider = PiProvider::new("pi".into());
    let args = provider.build_args(&std::path::PathBuf::from("/ext/aria-gate.ts"), Some("sess-123"));
    assert!(args.contains(&"--session-id".to_string()));
    assert!(args.contains(&"sess-123".to_string()));
}
```

Run: `cargo test -p cadence-aria pi_provider`
Expected: FAIL —— `build_args` 未定义

- [ ] **Step 8: 实现 `PiProvider` + `build_args()`**

`mod.rs`：

```rust
use std::path::{Path, PathBuf};

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

    /// 构造 pi RPC 命令行。
    /// - 不设 --session-dir：Pi 用默认 ~/.pi。
    /// - 不设 --no-extensions：保留用户全局扩展。
    /// - cwd 由 spawn 时传 working_dir（项目代码库目录）。
    pub(crate) fn build_args(&self, gate_extension: &Path, resume_session_id: Option<&str>) -> Vec<String> {
        let mut args = vec![
            "--mode".to_string(),
            "rpc".to_string(),
            "-e".to_string(),
            gate_extension.display().to_string(),
        ];
        if let Some(sid) = resume_session_id.map(str::trim).filter(|s| !s.is_empty()) {
            args.push("--session-id".to_string());
            args.push(sid.to_string());
        }
        args
    }
}
```

Run: `cargo test -p cadence-aria pi_provider` → PASS

- [ ] **Step 9: 写失败测试 —— session 驱动（duplex 模拟 Pi peer）**

用 `tokio::io::duplex` 模拟 Pi 的 stdin/stdout，驱动 `run_pi_session`，逐行断言 outbound JSON（prompt 命令、extension_ui_response、abort）与 inbound 事件处理（文本/工具/confirm/终止）：

```rust
#[tokio::test]
async fn session_emits_prompt_and_reads_text_until_settled() {
    // 用 tokio::io::duplex 构造 fake peer，预置 fixture 的 inbound 行
    // 断言 run_pi_session 发出 prompt 命令、把文本增量转成 ProviderEvent、遇 agent_settled 终止
}

#[tokio::test]
async fn session_bridges_confirm_to_approval_and_responds() {
    // Supervised：fixture 含 extension_ui_request；断言 session 调 ApprovalBridge、按决定回 extension_ui_response
}

#[tokio::test]
async fn session_abort_on_cancel() {
    // 取消时断言发出 abort 命令
}
```

Run: `cargo test -p cadence-aria pi_provider`
Expected: FAIL —— `run_pi_session` 未实现

- [ ] **Step 10: 实现 `StreamingProviderAdapter for PiProvider` + `run_pi_session`**

`mod.rs` 仿 `codex_provider/mod.rs` 的 `start()`：`ProcessManager::spawn` + `JsonRpcPeer::new` + `ApprovalBridge::new(input.permission_mode.clone(), event_tx)` + `tokio::spawn(run_pi_session(...))`。`ARIA_PERMISSION_MODE` 注入 `env_vars`（Supervised→`supervised`，Auto→`auto`）。

`session.rs` 的 `run_pi_session`：
- 启动会话（首次无 `--session-id`，从 `get_state` 或事件拿 `sessionId` 存入 `ProviderConversationRef` 供续接）。
- 发 `prompt` 命令。
- 事件循环：文本增量→`ProviderEvent`；`tool_execution_*`→工具事件；`extension_ui_request(confirm)`→`bridge.request_tool(tool_name, description, RiskLevel::Medium, cancel)`→按 `PermissionDecision.approved` 回 `extension_ui_response{confirmed}`；`agent_settled`→完成。
- 取消：`abort` 命令。
- 错误/EOF→`ProviderEvent::Failed`（fail-fast，不重试）。

Run: `cargo test -p cadence-aria pi_provider` → PASS

### Task 2e: registry 注册（生产 + 测试）

- [ ] **Step 11: 写失败测试 —— 生产 registry 含 Pi**

`src/web/state.rs` 相关测试（或新测试）：构造生产模式 `default_provider_registry`，断言 `registry.get(&ProviderName::Pi).is_some()`。

Run: `cargo test -p cadence-aria state`
Expected: FAIL —— 生产 registry 只注册 Claude/Codex

- [ ] **Step 12: `default_provider_registry()` 注册 Pi**

`src/web/state.rs:296-339`：
- **测试模式**（`test_provider_enabled`）分支：在 Claude/Codex 的 Fake 注册后加 `registry.register(ProviderName::Pi, Arc::new(TestControlledFakeStreamingProvider::new(...)))`。
- **生产模式**分支：在 Codex 的 `register_gated` 后加：

```rust
    registry.register_gated(
        ProviderName::Pi,
        Arc::new(PiProvider::new(PathBuf::from("pi"))),
        provider_gate,
    );
```

注意：`use` 引入 `crate::cross_cutting::pi_provider::PiProvider`。

Run: `cargo test -p cadence-aria state` → PASS

- [ ] **Step 13: 全量受影响测试 + Commit**

Run:
```bash
cargo test -p cadence-aria pi_provider
cargo test -p cadence-aria state
git add src/cross_cutting/pi_provider/ src/cross_cutting/mod.rs src/web/state.rs
git commit -m "feat(pi): add aria-gate extension, RPC streaming adapter, and registry registration"
```

---

## Task 3: 普通 Workspace 权限持久化（真实体）+ Pi 接入 + fail-fast（tasks 3.1, 3.2）

**背景：** 普通 Workspace 的**真正持久化实体**是 `WorkspaceSessionRecord`（`src/product/models/workspace.rs:31`），运行时实体是 `WorkspaceSession`（`src/product/workspace_engine/types.rs:72`），两者目前只有 `author_provider`/`reviewer_provider`/`review_rounds`，**无权限模式字段**。`ProviderConfigSnapshot`（`common.rs:112`）只是传输/快照 DTO。本任务把权限模式持久化到真实体，并让运行链路读它而非硬编码 `Supervised`。

**Files:**
- Modify: `src/product/models/workspace.rs:31`（`WorkspaceSessionRecord` 加 `permission_modes` 字段）
- Modify: `src/product/workspace_engine/types.rs:72`（`WorkspaceSession` 加 `permission_modes`）+ `types.rs:91`（`from_record` 映射）
- Modify: `src/web/workspace_ws_types/common.rs:112`（`ProviderConfigSnapshot` 加 `permission_modes`，wire DTO）
- Modify: `src/cross_cutting/streaming_provider/mod.rs:34`（`ProviderPermissionMode` 加 `Serialize, Deserialize` derive）
- Modify: `src/product/workspace_engine/lifecycle.rs:617-649`（`start_generation()` 把 wire 权限模式锁定到 session + store）
- Modify: `src/product/workspace_engine/session_state.rs:205-222`、`session_state/timeline.rs:412-419`（timeline snapshot 复制 session 权限模式）
- Modify: `src/product/workspace_engine/prompts.rs:154,177`、`review.rs` 多处、`review_repair.rs:49`、`revision.rs:55`（硬编码 `Supervised` 改读配置）
- Modify: store 写入/更新 API（author/reviewer/rounds/modes 一起持久化）
- Test: `src/product/models/workspace.rs`（旧记录反序列化）、`src/product/workspace_engine/tests/`（权限读配置 + Pi 接入 + fail-fast）

**Interfaces:**
- Consumes: `ProviderConfigSnapshot`、`ProviderPermissionMode`、`StreamingProviderInput`、`ProviderName::Pi`（Task 1/2）。
- Produces: `WorkspaceRolePermissionModes { author: ProviderPermissionMode, reviewer: ProviderPermissionMode }`（独立于 Coding 类型，不合并）；`WorkspaceSessionRecord.permission_modes`（`#[serde(default)]`，旧记录缺字段按 `Auto`）。

**约束（Decision 3）：** 两套权限类型不合并；旧持久化会话缺字段按 `Auto`；权限模式变更只影响后续启动的运行。

### Task 3a: 权限模式持久化到真实体

- [ ] **Step 1: 写失败测试 —— `WorkspaceRolePermissionModes` 默认 Auto**

`src/product/models/workspace.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn workspace_role_permission_modes_default_is_auto() {
    let modes = WorkspaceRolePermissionModes::default();
    assert_eq!(modes.author, ProviderPermissionMode::Auto);
    assert_eq!(modes.reviewer, ProviderPermissionMode::Auto);
}
```

Run: `cargo test -p cadence-aria workspace`
Expected: FAIL —— `WorkspaceRolePermissionModes` 未定义

- [ ] **Step 2: 定义 `WorkspaceRolePermissionModes` + `ProviderPermissionMode` 加 serde derive**

`src/cross_cutting/streaming_provider/mod.rs:34` 给 `ProviderPermissionMode` 加 `Serialize, Deserialize`（当前只有 `Debug, Clone, PartialEq, Eq`）。

`src/product/models/workspace.rs` 定义：

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
```

Run: `cargo test -p cadence-aria workspace` → PASS

- [ ] **Step 3: 写失败测试 —— 旧 `WorkspaceSessionRecord` 缺字段反序列化为 Auto**

`src/product/models/workspace.rs` 的 `#[cfg(test)]` 加（用真实旧记录 JSON，非 DTO）：

```rust
#[test]
fn old_workspace_session_record_without_permission_modes_deserializes_to_auto() {
    // 旧持久化记录：无 permission_modes 字段
    let json = serde_json::json!({
        "id": "s1", "project_id": "p1", "issue_id": "i1", "entity_id": "e1",
        "workspace_type": "story", "status": "active",
        "author_provider": "claude_code", "reviewer_provider": "codex",
        "review_rounds": 1, "superpowers_enabled": false, "openspec_enabled": false,
        "messages": [], "created_at": "", "updated_at": ""
    });
    let record: WorkspaceSessionRecord = serde_json::from_value(json).unwrap();
    assert_eq!(record.permission_modes.author, ProviderPermissionMode::Auto);
    assert_eq!(record.permission_modes.reviewer, ProviderPermissionMode::Auto);
}
```

Run: `cargo test -p cadence-aria workspace`
Expected: FAIL —— `WorkspaceSessionRecord` 无 `permission_modes` 字段

- [ ] **Step 4: `WorkspaceSessionRecord` 加 `permission_modes`（`#[serde(default)]`）**

`src/product/models/workspace.rs:31`：

```rust
    #[serde(default)]
    pub permission_modes: WorkspaceRolePermissionModes,
```

Run: `cargo test -p cadence-aria workspace` → PASS

- [ ] **Step 5: 运行时 `WorkspaceSession` 加字段 + `from_record` 映射 + timeline snapshot 复制**

`src/product/workspace_engine/types.rs:72` 加 `pub permission_modes: WorkspaceRolePermissionModes`；`types.rs:91` `from_record` 加 `permission_modes: record.permission_modes.clone()`。

`session_state.rs:205-222` 与 `session_state/timeline.rs:412-419` 构造 `ProviderConfigSnapshot` 处，把 `session.permission_modes` 复制进 DTO。

`src/web/workspace_ws_types/common.rs:112` `ProviderConfigSnapshot` 加 `#[serde(default)] pub permission_modes: WorkspaceRolePermissionModes`。

测试：构造带权限模式的 session，断言 timeline snapshot 的 `permission_modes` 与 session 一致。

Run: `cargo test -p cadence-aria workspace_engine` → PASS

- [ ] **Step 6: `start_generation()` 把 wire 权限模式锁定到 session + store**

`src/product/workspace_engine/lifecycle.rs:617-649`：`start_generation()` 从 wire `ProviderConfigSnapshot` 读 `permission_modes`，写入 `WorkspaceSession` 与 store（author/reviewer/rounds/modes 一起持久化）。若 reviewer 关闭（`reviewer: None`），明确 reviewer mode 语义（保留或归零），并测试。

Run: `cargo test -p cadence-aria workspace_engine` → PASS

### Task 3b: 运行链路读配置（不硬编码 Supervised）

- [ ] **Step 7: 写失败测试 —— Author 运行读 session 权限模式**

`src/product/workspace_engine/tests/` 加：构造 `permission_modes.author = Auto` 的 session，调 Author 运行的 `build_streaming_input`（`prompts.rs`），断言返回的 `StreamingProviderInput.permission_mode == Auto`（非硬编码 Supervised）。

Run: `cargo test -p cadence-aria workspace_engine`
Expected: FAIL —— 现有实现硬编码 `Supervised`

- [ ] **Step 8: 硬编码 `Supervised` 改读配置**

`prompts.rs:154,177`（Author）、`review.rs` 多处（Reviewer）、`review_repair.rs:49`、`revision.rs:55`：把 `permission_mode: ProviderPermissionMode::Supervised` 改为读 `session.permission_modes.author`（Author 运行）或 `session.permission_modes.reviewer`（Reviewer 运行）。逐一确认每个构造点是 Author 还是 Reviewer 语境。

Run: `cargo test -p cadence-aria workspace_engine` → PASS

### Task 3c: Pi 接入 + fail-fast

- [ ] **Step 9: 写失败测试 —— Author 选 Pi 走 PiProvider + fail-fast**

`src/product/workspace_engine/tests/` 加（用 recording `StreamingProviderAdapter`，参照 reviewer 验证的注入点 `provider_registry.rs:21-44`）：

```rust
#[tokio::test]
async fn author_run_with_pi_uses_pi_provider() {
    // 注册 recording Pi provider；构造 author 选 Pi 的 session
    // 跑 Author 运行，断言 PiProvider.start 被调一次
}

#[tokio::test]
async fn pi_start_failure_reports_failure_without_switching_or_retrying() {
    // recording Pi provider 的 start() 返回 Err（启动失败）
    // 断言：运行呈失败状态、PiProvider.start_count == 1、其他 provider start_count == 0
}
```

Run: `cargo test -p cadence-aria workspace_engine`
Expected: FAIL —— 运行链路未接 Pi / fail-fast 未保证

- [ ] **Step 10: 运行链路接 Pi + fail-fast 边界**

普通 Workspace 运行经 registry 取 provider（Task 2 已注册 Pi）。确认 Author/Reviewer/返修运行选 Pi 时走 `PiProvider` 并接 `ApprovalBridge`。

**fail-fast 边界（Decision + 已澄清）：** Pi 失败即终态，不重试。用 `codegraph explore "workspace_engine provider_drive artifact retry fresh restart 触发条件"` 确认现有 retry 分支（`provider_drive.rs:455-491`、`504-570`）的触发条件：
- 若 retry 是「同 provider 内部重试」（重跑同一 provider 补 artifact），按已澄清的边界**保留**（契约只禁跨 provider）。
- 若 retry 是「换 provider 重跑」，**Pi 必须跳过**（fail-fast）。
确认后加防护：Pi 的运行失败不进入任何换 provider 的重跑路径。测试断言 `pi_start_count == 1`、其他 provider `start_count == 0`。

Run: `cargo test -p cadence-aria workspace_engine` → PASS

- [ ] **Step 11: 全量受影响测试 + Commit**

Run:
```bash
cargo test -p cadence-aria workspace
cargo test -p cadence-aria workspace_engine
git add src/product/models/workspace.rs src/product/workspace_engine/ src/web/workspace_ws_types/common.rs src/cross_cutting/streaming_provider/mod.rs
git commit -m "feat(workspace): persist per-role permission mode in session record (default Auto) and support Pi with fail-fast"
```

---

## Task 4: Coding Workspace 默认 Auto + Pi 接入 + fail-fast（tasks 4.1, 4.2）

**背景：** Coding Workspace 已有 per-role 权限结构 `CodingRolePermissionModes`（`provider_config.rs:17`），默认全 `Supervised`。本任务把新建默认值改 `Auto`，并让 Coder/Code Reviewer/Internal Reviewer 支持 Pi。

**Files:**
- Modify: `src/product/coding_models/provider_config.rs:23-31`（`CodingRolePermissionModes::default()` 全改 `Auto`）
- Modify: `src/product/coding_workspace_engine/`（三角色运行接 Pi + 授权桥接；权限硬编码改读配置）
- Test: `src/product/coding_models/provider_config.rs`（默认值）、`tests/it_product/product_coding_workspace_engine/`（Pi 运行 + fail-fast）

**Interfaces:**
- Consumes: `CodingRolePermissionModes`、`CodingProviderPermissionMode`、`CodingRoleProviderConfigSnapshot`、`PiProvider`（Task 2）。
- Produces: `CodingRolePermissionModes::default()` 全 `Auto`；Coding 三角色可选 Pi。**不改** `CodingProviderPermissionMode` 类型。

- [ ] **Step 1: 写失败测试 —— 新建默认 Auto + 显式值保留 + 旧记录缺字段 Auto**

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
    let json = serde_json::json!({"coder":"supervised","code_reviewer":"supervised","internal_reviewer":"supervised"});
    let modes: CodingRolePermissionModes = serde_json::from_value(json).unwrap();
    assert_eq!(modes.coder, CodingProviderPermissionMode::Supervised);
}

#[test]
fn old_coding_snapshot_without_permission_modes_deserializes_to_auto() {
    // CodingRoleProviderConfigSnapshot 的 permission_modes 用 #[serde(default)]；缺字段按新默认 Auto
    let json = serde_json::json!({"coder":"claude_code","code_reviewer":"codex","internal_reviewer":"claude_code","review_rounds":1});
    let snapshot: CodingRoleProviderConfigSnapshot = serde_json::from_value(json).unwrap();
    assert_eq!(snapshot.permission_modes.coder, CodingProviderPermissionMode::Auto);
}
```

Run: `cargo test -p cadence-aria coding_models`
Expected: FAIL —— 默认值是 `Supervised`

- [ ] **Step 2: `CodingRolePermissionModes::default()` 改 Auto**

`src/product/coding_models/provider_config.rs:23-31`，三个字段全改 `CodingProviderPermissionMode::Auto`。

⚠️ 注意：`CodingRoleProviderConfigSnapshot.permission_modes` 用 `#[serde(default)]`，改默认后旧记录缺字段按 Auto 反序列化——这正是契约要求，已用 Step 1 第三个测试覆盖。

Run: `cargo test -p cadence-aria coding_models` → PASS

- [ ] **Step 3: 写失败测试 —— 三角色可选 Pi + fail-fast**

`tests/it_product/product_coding_workspace_engine/` 参照现有 `execute_coding_*`（如 `part_04.rs`/`part_06.rs`），为三个角色各加一个测试：

```rust
#[tokio::test]
async fn coder_run_with_pi_uses_pi_provider() { /* recording Pi provider；coder 选 Pi；断言走 PiProvider */ }

#[tokio::test]
async fn code_reviewer_run_with_pi_uses_pi_provider() { /* 同上，code_reviewer 角色 */ }

#[tokio::test]
async fn internal_reviewer_run_with_pi_uses_pi_provider() { /* 同上，internal_reviewer 角色 */ }

#[tokio::test]
async fn pi_failure_reports_without_retrying_or_switching() {
    // recording Pi provider 运行失败；断言 pi start_count == 1、其他 provider start_count == 0、终态失败
}
```

Run: `cargo test -p cadence-aria product_coding_workspace_engine`
Expected: FAIL —— Coding 运行链路未接 Pi

- [ ] **Step 4: Coding 运行链路接 Pi + fail-fast**

`src/product/coding_workspace_engine/`：三角色运行经 registry 取 provider（Task 2 已注册 Pi）。确认 `StreamingProviderInput` 把 `permission_modes`（来自 `CodingRoleProviderConfigSnapshot`）传给 `PiProvider` 并接 `ApprovalBridge`。

用 `rg -n "CodingProviderPermissionMode::Supervised" src/product/coding_workspace_engine/ -g '*.rs'` 定位权限硬编码，改为读 `CodingRolePermissionModes` 对应角色。

**fail-fast：** 用 `codegraph explore "coding_workspace_engine resume stall fresh retry 触发条件 coding.rs provider_stream.rs"` 确认 Codex resume-stall fresh retry（`coding.rs:184-206`、`provider_stream.rs:542-555`）是同 provider 内部重试还是换 provider：同 provider 内部 retry 保留；Pi 不实现该 retry（Pi 失败即终态）。测试断言 `pi_start_count == 1`。

Run: `cargo test -p cadence-aria product_coding_workspace_engine` → PASS

- [ ] **Step 5: 全量受影响测试 + Commit**

Run:
```bash
cargo test -p cadence-aria coding_models
cargo test -p cadence-aria product_coding_workspace_engine
git add src/product/coding_models/provider_config.rs src/product/coding_workspace_engine/
git commit -m "feat(coding): default role permission modes to Auto and support Pi with fail-fast"
```

---

## Task 5: 前端 Provider 目录 + 权限控制 + 失败状态可见性（tasks 5.1, 5.2）

**背景：** Coding 前端已有权限控制 UI（`CodingProviderConfigPanel.tsx`，含 `permissionMode` 选择 + `auto`/`supervised` 文案）；普通 Workspace 前端**无**权限控制 UI（Task 3 才给后端加字段）。Task 1 已让 catalog 含 Pi。`ProviderConfigPanel` 的真实 props 是 `providers: WsProviderConfig | null`，健康快照从 `useProviderAvailabilityStore` 读（测试用 `setState` 设置），**不是** `healthSnapshot` prop。

**Files:**
- Modify: `web/src/api/types/workspace.ts`（`WsProviderConfig` 加 `permission_modes` 字段）
- Modify: `web/src/components/workspace/ProviderConfigPanel.tsx`（加 Author/Reviewer 权限模式选择 + Pi 选项）
- Modify: `web/src/components/coding-workspace/CodingProviderConfigPanel.tsx`（确认 Pi 出现在已有权限控件）
- Modify: 保存链路（`providerConfigFor()` / `start_generation` 相关 store/handler）把权限模式发给后端
- Test: `web/src/components/workspace/ProviderConfigPanel.test.tsx`、`web/src/components/coding-workspace/CodingProviderConfigPanel.test.tsx`

**Interfaces:**
- Consumes: `getProviderOptions(snapshot)`（Task 1 含 Pi）、`WsProviderConfig.permission_modes`（Task 3 wire）、`useProviderAvailabilityStore`。
- Produces: 普通 Workspace 面板含每角色 Auto/Supervised 选择；Pi 可选；不可用 Pi 禁用 + 原因；失败状态可见。

- [ ] **Step 1: 写失败测试 —— 普通 Workspace 面板展示 Pi + 权限模式选择**

`web/src/components/workspace/ProviderConfigPanel.test.tsx` 仿现有测试（用 `useProviderAvailabilityStore.setState()` 设健康快照）：

```ts
it("author 可选 Pi 并选择权限模式", () => {
  useProviderAvailabilityStore.setState({ snapshot: piAvailableSnapshot() });
  render(
    <ProviderConfigPanel
      providers={{ author: "pi", reviewer: "codex", review_rounds: 1 }}
      editable
      onSelectProvider={() => {}}
      reviewerEnabled
      onToggleReviewer={() => {}}
    />,
  );
  // 断言 author provider 选择器含 "pi" 选项
  // 断言 author 权限模式可切换 Auto/Supervised
});
```

注意：`providers` prop 用真实 `WsProviderConfig` 结构（含 `author`/`reviewer`/`review_rounds`），`piAvailableSnapshot()` 参照现有测试的 `setProviderHealth` helper 构造含 Pi 的快照。

Run: `cd web && npm test ProviderConfigPanel`
Expected: FAIL —— 面板无权限模式控件 / `WsProviderConfig` 无 `permission_modes`

- [ ] **Step 2: `WsProviderConfig` 加 `permission_modes` + 面板加权限控件 + Pi**

`web/src/api/types/workspace.ts` 给 `WsProviderConfig` 加 `permission_modes?: { author: "auto"|"supervised"; reviewer: "auto"|"supervised" }`。

`ProviderConfigPanel.tsx` 参照 `CodingProviderConfigPanel.tsx` 的权限选择 UI（`permissionMode` 选择 + `auto`/`supervised` 文案），为 Author/Reviewer 各加权限模式选择，数据源 `providers.permission_modes`，provider 选择器数据源 `getProviderOptions(snapshot)`（已含 Pi）。文案与 Coding 侧统一。

Run: `cd web && npm test ProviderConfigPanel` → PASS

- [ ] **Step 3: 保存链路把权限模式发给后端**

确认并修改保存链路（`providerConfigFor()` / `start_generation` 相关）：把 `WsProviderConfig.permission_modes` 包含进发给后端的 `provider_config`。用 `rg -n "providerConfigFor|start_generation" web/src/ -g '*.ts*'` 定位，确保权限模式随 provider 一起提交。

Run: `cd web && npm test` （相关 store/handler 测试）→ PASS

- [ ] **Step 4: Coding 面板确认 Pi 出现在权限控件**

`CodingProviderConfigPanel.test.tsx` 补测试：三角色（coder/code_reviewer/internal_reviewer）均可选 Pi。

Run: `cd web && npm test CodingProviderConfigPanel` → PASS

- [ ] **Step 5: 不可用 Pi 禁用 + 原因；失败状态可见**

确认：Pi 不可用时选择器保留已配置值但禁用，显示 `reason`/`install_hint`（复用现有 `blockedReason`/`realProviderOption`，Task 1 已覆盖）。运行失败经 `ProviderEvent` → 前端状态链路显示（fail-fast：失败即显示失败）。

补测试：Pi 不可用时选项 disabled 且显示原因。

Run: `cd web && npm test` → PASS

- [ ] **Step 6: 前端全量测试 + Commit**

Run:
```bash
cd web && npm test && npm run build && cd ..
git add web/src/api/types/workspace.ts web/src/components/workspace/ProviderConfigPanel.tsx web/src/components/coding-workspace/CodingProviderConfigPanel.tsx
git commit -m "feat(web): show Pi and per-role Auto/Supervised controls in workspace and coding provider config"
```

---

## Task 6: 回归验证 + 边界验证 + 前后端质量检查（tasks 6.1, 6.2, 6.3）

**Files:**
- Test: `src/cross_cutting/pi_provider/tests.rs`（健康/目录/会话协议/取消/恢复/双授权模式）
- Test: `src/product/workspace_engine/tests/`、`tests/it_product/product_coding_workspace_engine/`（Provider/权限/fail-fast）
- Test: `src/task_run/provider_factory.rs`、`src/web/provider_availability.rs`（Task Runner 拒绝 Pi）

- [ ] **Step 1: Pi 后端协议回归（tasks 6.1）**

`src/cross_cutting/pi_provider/tests.rs` 用 Task 2 的 fixture 覆盖：健康检查、目录展示、文本流、工具事件、完成、错误映射、取消（abort）、恢复（--session-id）、Auto 放行 + 审计、Supervised confirm 放行/拒绝（拒绝后工具不执行、记拒绝决定）。

Run: `cargo test -p cadence-aria pi_provider` → PASS

- [ ] **Step 2: Workspace/Coding 角色回归（tasks 6.2）**

为 Story/Design/Work Item 三入口（共享 `workspace_engine`）及 Coding 角色补 Provider 选择（含 Pi）、权限模式（Auto/Supervised 读配置）、fail-fast（启动失败用 `start()` 返回 `Err` 构造，参照 `streaming_provider/mod.rs:278-295`；运行中失败用 `ProviderEvent::Failed` 构造，参照 `src/web/test_controls/provider.rs:343-349`）。断言：失败即终态、`pi_start_count == 1`、其他 provider `start_count == 0`。

Run: `cargo test -p cadence-aria workspace_engine product_coding_workspace_engine` → PASS

- [ ] **Step 3: 边界验证 —— 仓库初始化与 Task Runner 未被扩张（tasks 6.3）**

- 仓库初始化：确认只用 Claude Code 专用选项，不含 Pi（测试锁定）。
- Task Runner 拒绝 Pi（四层）：
  - HTTP 入口：`parse_provider_type("pi")` 返回 `web_runtime_provider_type` 错误且文本含 `pi`（`provider_availability.rs:185-193`）
  - Router：`RoutingProviderAdapter` 拒 `ProviderType::Pi` 且 adapter 不调用（Task 1 Step 8）
  - 兼容性矩阵：`default_compatibility_matrix().entry_for(ProviderType::Pi)` 为 `None`
  - 节点契约：遍历 `default_node_contracts()`（或等价静态契约集合），断言每个 `provider_type != ProviderType::Pi`（**不是**源码文字扫描）

Run: `cargo test -p cadence-aria provider_factory task_run provider_availability` → PASS

- [ ] **Step 4: 前后端质量检查 + 契约同步**

```bash
cargo test -p cadence-aria
cargo clippy -p cadence-aria --all-targets
cargo fmt --check
cd web && npm test && npm run build && cd ..
```

按 `cadence/project-rules/build-test-commands.md` 标准命令执行（🔴 禁止 `-j 1`）。全部通过后勾选 `openspec/changes/add-pi-provider/tasks.md` 对应工作包。

- [ ] **Step 5: Final Commit**

```bash
git add -A
git commit -m "test(pi): regression coverage for Pi protocol, workspace/coding roles, and Task Runner boundary"
```

---

## 自检（Self-Review）

**1. Spec 覆盖：**
- Requirement「Pi 可发现可选择」→ Task 1（catalog/健康/状态 API）+ Task 5（选择器）✅
- Requirement「流式执行 + 控制」→ Task 2（session/取消/恢复）✅
- Requirement「权限模式默认 Auto + 按角色监督」→ Task 3（普通 Workspace）+ Task 4（Coding）✅
- Requirement「失败直接报告不切换」→ Task 3/4（fail-fast）+ Task 6 Step 2（回归）✅
- Requirement「不扩大初始化/Task Runner」→ Task 1（Task Runner 拒绝）+ Task 6 Step 3（边界）✅

**2. 占位符扫描：** 无 TBD/TODO/「适当错误处理」/「类似 Task N」引用。所有测试给了真实代码或可定位的 helper 参照。

**3. 类型一致性：** `ProviderName::Pi`/`ProviderType::Pi`（Task 1）贯穿；`WorkspaceRolePermissionModes`（Task 3，普通 Workspace 真实体）独立于 `CodingRolePermissionModes`（Task 4），不合并；`pi_version_command()`（Task 1）贯穿健康检查；`PiProvider`/`run_pi_session`（Task 2）供 Task 3/4 经 registry 选用。
