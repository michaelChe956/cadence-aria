# Task 1: Provider 目录与可用性（tasks 1.1, 1.2, 1.3）

> 自包含任务文件。执行前请先读 `00-overview.md` 的 Global Constraints 与跨任务接口契约。

**Goal:** 把 Pi 注册为流式 Provider（`ProviderName::Pi` + `ProviderType::Pi`），接入健康检查、状态 API、前端选择目录，并让 Task Runner 四层显式拒绝 Pi。

**对应 spec requirement:**
- 「Pi 在活跃 Provider 工作流中可发现且可选择」（健康状态条目 + 选择器）
- 「Pi 不扩大仓库初始化和 Task Runner 的 Provider 范围」（Task Runner 拒绝 Pi）

**Files:**
- Modify: `src/product/models/provider.rs:5`（`ProviderName` 加 `Pi`）
- Modify: `src/protocol/contracts.rs:45`（`ProviderType` 加 `Pi`）
- Modify: `src/product/workspace_engine/mappings.rs:29`、`src/product/coding_workspace_engine/tool_format.rs:106`、`src/product/work_item_split_engine/types.rs:129`（`provider_type_for_name` 加 `Pi => ProviderType::Pi`）
- Modify: `src/product/provider_workspace_runner.rs:130`（legacy Fake runner 的 `provider_type_for_name` 加 Pi 拒绝臂）
- Modify: `src/product/work_item_projection/render.rs:183`（`renderer_for(provider)` 加 Pi 映射）
- Modify: `src/task_run/provider_factory.rs:64-72`（`RoutingProviderAdapter::run` 加 Pi 拒绝臂）
- Modify: `src/task_run/step_runner.rs:111`（节点契约文本化，Pi 不产生）
- Modify: `src/cross_cutting/provider_health.rs`（`refresh()`/`uninitialized_snapshot()`/`real_workflow_blocked()` 加 Pi）
- Modify: `src/cross_cutting/provider_registry.rs:47`（`available_names()` 加 `ProviderName::Pi`）
- Modify: `src/web/handlers/providers.rs`（状态 API 数组 + `provider_dto()` 加 Pi）
- Modify: `src/web/handlers/dto.rs:742`（provider wire 文本化加 Pi）
- Modify: `web/src/api/types/provider.ts:1`（`RealProviderName` 加 `"pi"`）、`web/src/state/provider-options.ts`（catalog 加 Pi）
- Test: 上述各文件内联 `#[cfg(test)]` + `web/src/state/provider-options.test.ts`

**Interfaces:**
- Consumes: `CommandSpec::new(program: impl Into<String>, args: Vec<String>)`、`probe_provider(provider, command, checked_at, cancellation)`、`getProviderOptions(snapshot)`、`ProviderStatusResponse`。
- Produces: `ProviderName::Pi`/`ProviderType::Pi`（wire 均 `"pi"`）；`provider_type_for_name(&ProviderName::Pi) -> ProviderType::Pi`；健康检查 `pi_version_command()`；状态 API 返回 Pi 条目；`ProviderRegistry::available_names()` 含 `ProviderName::Pi`。**Task 2/3/4/5 依赖。**

**约束（Decision 1）：** `ProviderType::Pi` 只是共享类型变体；Task Runner 四层显式拒绝。健康检查 Pi version_command 内联（方案 A），不调 `matrix.entry_for(ProviderType::Pi)`（矩阵无 Pi 条目，会 panic）。仓库初始化只用 Claude Code 专用选项。

---

## Step 1: 写失败测试 —— `ProviderName::Pi` 序列化

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

- [ ] Run: `cargo test -p cadence-aria provider_name_pi_serializes_to_snake_case`
- Expected: FAIL —— `no variant Pi`

## Step 2: `ProviderName` 与 `ProviderType` 加 `Pi`

`src/product/models/provider.rs:5` 与 `src/protocol/contracts.rs:45`，各自在 `Codex` 后、`Fake` 前插入 `Pi`：

```rust
pub enum ProviderName { ClaudeCode, Codex, Pi, Fake }
pub enum ProviderType { ClaudeCode, Codex, Pi, Fake }
```

- [ ] Run: `cargo test -p cadence-aria provider_name_pi_serializes_to_snake_case`
- Expected: PASS

## Step 3: 写失败测试 —— `ProviderType::Pi` 序列化

`src/protocol/contracts.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn provider_type_pi_serializes_to_snake_case() {
    assert_eq!(serde_json::to_string(&ProviderType::Pi).unwrap(), "\"pi\"");
}
```

- [ ] Run: `cargo test -p cadence-aria provider_type_pi_serializes_to_snake_case`
- Expected: PASS（`ProviderType` 已在 Step 2 加 `Pi`）

## Step 4: 穷尽 match 完整盘点

`ProviderName`/`ProviderType` 加 `Pi` 后所有穷尽 match 编译错。先列出全部，不靠编译报错逐个碰：

```bash
rg -n "ProviderName::(ClaudeCode|Codex|Fake)" src/ -g '*.rs' | grep -v test
rg -n "ProviderType::(ClaudeCode|Codex|Fake)" src/ -g '*.rs' | grep -v test
```

按下表分类补臂（已确认的关键位置；盘点中发现的其他位置按同原则归类）：

| 位置 | Pi 策略 | 补臂 |
|---|---|---|
| `workspace_engine/mappings.rs:29`、`coding_workspace_engine/tool_format.rs:106`、`work_item_split_engine/types.rs:129` `provider_type_for_name` | 流式映射 | `ProviderName::Pi => ProviderType::Pi,` |
| `provider_workspace_runner.rs:130` `provider_type_for_name`（legacy Fake runner） | 拒绝 | `ProviderName::Pi => unreachable!("legacy fake runner does not support pi"),` |
| `work_item_projection/render.rs:183` `renderer_for(provider)` | 映射到 Claude 同款 renderer | `ProviderName::Pi => renderer_for_claude(),` |
| `provider_registry.rs:47` `available_names()` | 纳入 | 数组加 `ProviderName::Pi` |
| `web/handlers/dto.rs:742` provider wire 文本化 | 映射 | `"pi"` |
| `provider_availability.rs` 各 helper | 映射/保留拒绝 | `provider_name_key` 映射 `"pi"`；`parse_provider_type` 保持拒 `"pi"` |
| `task_run/step_runner.rs:111` 节点契约文本化 | 不产生 | 保持只文本化 Claude/Codex/Fake |
| 仓库初始化 provider 选择 | Claude-only | 不加 Pi 分支 |

- [ ] 逐个补臂，每个配一个测试锁定语义。

## Step 5: 写失败测试 —— `provider_type_for_name` 映射 Pi

`src/product/workspace_engine/mappings.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn provider_type_for_name_maps_pi() {
    assert_eq!(provider_type_for_name(&ProviderName::Pi), ProviderType::Pi);
}
```

- [ ] Run: `cargo test -p cadence-aria provider_type_for_name_maps_pi`
- Expected: FAIL —— `provider_type_for_name` 对 `Pi` 不匹配

## Step 6: 实现 `provider_type_for_name` 的 Pi 映射

`src/product/workspace_engine/mappings.rs:29`、`coding_workspace_engine/tool_format.rs:106`、`work_item_split_engine/types.rs:129` 各加：

```rust
ProviderName::Pi => ProviderType::Pi,
```

- [ ] Run: `cargo test -p cadence-aria provider_type_for_name_maps_pi`
- Expected: PASS

## Step 7: 写失败测试 —— Task Runner router 拒绝 Pi

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

- [ ] Run: `cargo test -p cadence-aria provider_factory`
- Expected: FAIL —— `RoutingProviderAdapter::run` 对 `ProviderType::Pi` 未匹配

## Step 8: `RoutingProviderAdapter::run` 加 Pi 拒绝臂

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

- [ ] Run: `cargo test -p cadence-aria provider_factory`
- Expected: PASS

## Step 9: 写失败测试 —— 健康检查探测 Pi

`src/cross_cutting/provider_health.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn pi_version_command_uses_pi_binary() {
    let command = pi_version_command();
    assert_eq!(command.program, "pi");
    assert_eq!(command.args, vec!["--version".to_string()]);
}
```

- [ ] Run: `cargo test -p cadence-aria pi_version_command_uses_pi_binary`
- Expected: FAIL —— `pi_version_command` 未定义

## Step 10: 健康检查加 Pi 探测

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

- [ ] Run: `cargo test -p cadence-aria provider_health`
- Expected: PASS

## Step 11: 写失败测试 —— 状态 API 返回 Pi

`src/web/handlers/providers.rs` 的 `#[cfg(test)]` 加：

```rust
#[test]
fn provider_status_includes_pi_when_available() {
    // 构造含 Pi 可用条目的健康快照，调 response_from_snapshot
    // 断言返回 providers 含 provider == "pi" 且 display_name == "Pi"
}
```

- [ ] Run: `cargo test -p cadence-aria providers`
- Expected: FAIL —— `response_from_snapshot` 只枚举 Claude/Codex；`provider_dto` 无 Pi 分支

## Step 12: 状态 API 加 Pi

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

- [ ] Run: `cargo test -p cadence-aria providers`
- Expected: PASS

## Step 13: registry `available_names()` 加 Pi

`src/cross_cutting/provider_registry.rs:47` 的 `available_names()` 数组 `[ClaudeCode, Codex, Fake]` 扩为含 `ProviderName::Pi`。

`provider_registry.rs` 的 `#[cfg(test)]` 加测试断言 `available_names()` 含 `ProviderName::Pi`。

- [ ] Run: `cargo test -p cadence-aria provider_registry`
- Expected: PASS

## Step 14: 写失败测试 —— 前端 catalog 含 Pi

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

- [ ] Run: `cd web && npm test provider-options`
- Expected: FAIL —— catalog 无 `"pi"`；`RealProviderName` 类型不含 `"pi"`

## Step 15: 前端类型 + catalog 加 Pi

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

- [ ] Run: `cd web && npm test provider-options`
- Expected: PASS

## Step 16: 验证仓库初始化不受影响

- [ ] Run: `codegraph explore "仓库初始化 / 添加代码库 页面 provider 选项来源，是否引用 REAL_PROVIDER_CATALOG"`
- Expected: 初始化路径有独立 Claude Code 专用选项，不读 `REAL_PROVIDER_CATALOG`。

## Step 17: 全量测试 + Commit

- [ ] Run:

```bash
cargo test -p cadence-aria provider
cargo test -p cadence-aria provider_factory
cargo test -p cadence-aria providers
cargo test -p cadence-aria provider_registry
cd web && npm test provider-options && cd ..
git add src/product/models/provider.rs src/protocol/contracts.rs src/product/workspace_engine/mappings.rs src/product/coding_workspace_engine/tool_format.rs src/product/provider_workspace_runner.rs src/product/work_item_split_engine/types.rs src/product/work_item_projection/render.rs src/task_run/provider_factory.rs src/task_run/step_runner.rs src/cross_cutting/provider_health.rs src/cross_cutting/provider_registry.rs src/web/handlers/providers.rs src/web/handlers/dto.rs web/src/api/types/provider.ts web/src/state/provider-options.ts web/src/state/provider-options.test.ts
git commit -m "feat(provider): register Pi across provider name/type, health, status API, registry, frontend catalog; Task Runner rejects Pi"
```

---

## 完成检查（对应 tasks 1.1/1.2/1.3）

- [ ] 1.1：Pi 纳入 `ProviderName`、健康检查、状态接口、前端选择目录。
- [ ] 1.2：`ProviderType` 加 `Pi` 变体；`provider_type_for_name` 可映射；Task Runner HTTP 入口（`parse_provider_type` 拒 `"pi"`）与 `RoutingProviderAdapter` 拒绝 Pi 且 adapter 不调用；兼容性矩阵无 Pi 条目；静态节点契约不产生 Pi。
- [ ] 1.3：仓库初始化 Claude Code 专用选项与执行路径不受 Pi 影响。
