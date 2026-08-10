# Task 1: Provider 目录与可用性（tasks 1.1, 1.2, 1.3）

> 自包含任务文件。执行前请先读 `00-overview.md` 的 Global Constraints 与跨任务接口契约。

**Goal:** 把 Kimi Code 注册为流式 Provider（`ProviderName::KimiCode` + `ProviderType::KimiCode`，wire `"kimi_code"`），接入健康检查（`kimi --version`，≥0.34.0）、initialize 后 capability 校验、状态 API、前端选择目录，并让 Task Runner 四入口稳定拒绝 Kimi（不用 `unreachable!`），仓库初始化过滤 Kimi。

**对应 spec requirement:**
- 「Kimi 在活跃 Provider 工作流中可发现且可选择」（含版本过低 Scenario）
- 「Kimi 复用既有授权与失败边界」（task-run 稳定拒绝）
- 「Kimi 不参与仓库初始化」

**Files:**
- Modify: `src/product/models/provider.rs:5-9`（`ProviderName` 加 `KimiCode`，`Pi` 后、`Fake` 前）
- Modify: `src/protocol/contracts.rs:45-49`（`ProviderType` 加 `KimiCode`，`Pi` 后、`Fake` 前）
- Modify: 所有 `provider_type_for_name` / `From<ProviderName> for ProviderType` 穷尽 match：
  - `src/product/workspace_engine/mappings.rs`
  - `src/product/coding_workspace_engine/tool_format.rs`
  - `src/product/work_item_split_engine/types.rs`
  - `src/product/image_create/models.rs:115`（`From<ProviderName>`）
  - `src/product/provider_workspace_runner.rs`（legacy Fake runner 的 `provider_type_for_name`，加 KimiCode 拒绝臂）
- Modify: `src/product/work_item_projection/render.rs`（`renderer_for(provider)` 加 KimiCode 分支：**此 task 创建最小占位 renderer `KimiCodeProjectionRenderer`**，见 Step 3；Task 4 补全 profile）
- Modify: `src/cross_cutting/provider_health.rs`（新增 `kimi_version_command()`；`refresh()`/`uninitialized_snapshot()`/`real_workflow_blocked()` 加 Kimi）
- Modify: `src/cross_cutting/provider_registry.rs:47`（`available_names()` 加 `ProviderName::KimiCode`，置 `Pi` 之后、`Fake` 之前）
- Modify: `src/web/handlers/providers.rs:82-99`（`provider_dto()` match 加 KimiCode 分支：`"kimi_code"` / `"Kimi Code"` / install_hint）
- Modify: `src/web/provider_availability.rs`（`parse_provider_name` 接受 `"kimi_code"`；`provider_name_key` 返回 `"kimi_code"`；PATH 探测加 kimi）
- Modify: `src/task_run/provider_factory.rs:66-73`（`RoutingProviderAdapter::run` 加 `KimiCode` 拒绝臂 `Err(incompatible_output)`）
- Modify: `src/task_run/step_runner.rs:113-116`（`provider_type_text`：`KimiCode => "kimi_code"`，**禁止 `unreachable!`**）
- Modify: `src/web/runtime/provider.rs:498`（`KimiCode => None`）
- Modify: `src/web/runtime/utils.rs:62`（KimiCode 返回稳定空/错误，**禁止 `unreachable!`**）
- Modify: `web/src/api/types/provider.ts`（`RealProviderName` 加 `"kimi_code"`）
- Modify: `web/src/state/provider-options.ts`（`REAL_PROVIDER_CATALOG` 加 `{value:"kimi_code",fallbackLabel:"Kimi Code"}`；`PROVIDER_ORDER` 加 `"kimi_code"` 于 `"pi"` 后、`"fake"` 前）
- Modify: `web/src/components/lifecycle/CreateRepositoryDialog.tsx`（仓库初始化过滤 Kimi，加 capability policy 注释）
- Test: 上述各文件内联 `#[cfg(test)]` + `web/src/state/provider-options.test.ts`

**Interfaces:**
- Consumes: `CommandSpec::new(program, args)`、`probe_provider(provider, command, checked_at, cancellation)`、`ProviderAdapterError::incompatible_output`、`getProviderOptions(snapshot)`。
- Produces: `ProviderName::KimiCode`/`ProviderType::KimiCode`（wire `"kimi_code"`）；所有 `provider_type_for_name(&ProviderName::KimiCode) -> ProviderType::KimiCode`；健康检查 `kimi_version_command()`；状态 API 返回 Kimi 条目；`ProviderRegistry::available_names()` 含 Kimi。**Task 2-7 依赖。**

**约束：** Task Runner 四入口稳定拒绝（不用 `unreachable!`）；健康检查 Kimi version_command 内联（不调 `matrix.entry_for(KimiCode)`，矩阵无 Kimi 条目）；仓库初始化过滤 Kimi；`renderer_for` 此 task 创建最小占位 renderer（`KimiCodeProjectionRenderer`，label="Kimi Code"，profile 内容最简），Task 4 补全；image-create 此 task **仅补 `From<ProviderName>` 编译分支**，前端 dropdown 与回归测试在 Task 5。

---

## Step 1: 写失败测试 —— `ProviderName::KimiCode` 序列化

`src/product/models/provider.rs` 末尾加测试模块（若已有则追加）：

```rust
#[cfg(test)]
mod tests {
    use super::ProviderName;

    #[test]
    fn provider_name_kimi_code_serializes_to_snake_case() {
        assert_eq!(serde_json::to_string(&ProviderName::KimiCode).unwrap(), "\"kimi_code\"");
        let back: ProviderName = serde_json::from_str("\"kimi_code\"").unwrap();
        assert_eq!(back, ProviderName::KimiCode);
    }
}
```

- [ ] Run: `cargo test --locked --lib provider_name_kimi_code`
- Expected: FAIL —— `no variant KimiCode` / 编译错误

## Step 2: `ProviderName` 与 `ProviderType` 加 `KimiCode`

`src/product/models/provider.rs:8`（`Pi` 后）与 `src/protocol/contracts.rs:48`（`Pi` 后），各自加：

```rust
    KimiCode,
```
（位置：`Pi` 与 `Fake` 之间）

为 `ProviderType::KimiCode` 的 serde wire，确认 contracts.rs 的 `ProviderType` 序列化为 snake_case（与现有 Pi 一致）；若用 `#[serde(rename_all="snake_case")]` 则 `KimiCode` 自动变 `kimi_code`，需加往返测试断言。

- [ ] Run: `cargo test --locked --lib provider_name_kimi_code`
- Expected: PASS

## Step 3: 补全所有 `provider_type_for_name` / `From<ProviderName>` 穷尽 match + 最小占位 renderer

在以下文件的穷尽 match 中加 `ProviderName::KimiCode => ProviderType::KimiCode`（或等价）：
- `src/product/workspace_engine/mappings.rs`
- `src/product/coding_workspace_engine/tool_format.rs`
- `src/product/work_item_split_engine/types.rs`
- `src/product/image_create/models.rs`（`From<ProviderName> for ProviderType`）—— **仅补枚举映射编译分支**；image-create 前端 dropdown 与回归测试在 Task 5
- `src/product/provider_workspace_runner.rs`（legacy runner 加 KimiCode 拒绝臂，返回 incompatible）

`src/product/work_item_projection/render.rs` 的 `renderer_for` 返回 `Box<dyn ProviderProjectionRenderer>`（**非 Result**，不能返回错误占位）。此 task 创建最小占位 renderer：

新建 `src/product/work_item_projection/render/kimi_code.rs`（仿 `render/pi.rs` 最小形态）：
```rust
use super::*; // 复用 ProviderProjectionRenderer trait 与通用类型

pub(crate) struct KimiCodeProjectionRenderer;

impl ProviderProjectionRenderer for KimiCodeProjectionRenderer {
    // 最小实现：label="Kimi Code"，其余 profile 字段用通用默认/最简值
    // 具体 profile（Supervised tool hint、structured-output wrapper）在 Task 4 补全
    // TODO(Task4): 补全完整 profile
}
```
`render.rs` 加 `mod kimi_code;` + `use kimi_code::KimiCodeProjectionRenderer;` + `renderer_for` 加 `ProviderName::KimiCode => Box::new(KimiCodeProjectionRenderer),`。

- [ ] Run: `cargo check --locked`
- Expected: PASS（所有穷尽 match 已补全，无编译错误；占位 renderer 可编译）

## Step 4: 健康检查 `kimi_version_command()` 与接入

`src/cross_cutting/provider_health.rs`，仿 `pi_version_command()`（:268）新增：

```rust
fn kimi_version_command() -> CommandSpec {
    CommandSpec::new("kimi", vec!["--version".to_string()])
}
```

在 `refresh()` 的并行 probe（`tokio::join!` 或 providers vector）加 Kimi 条目；`uninitialized_snapshot()` 加 Kimi；`real_workflow_blocked()` 的"全部不可用"判定含 Kimi。更新并行数与排序测试。

`provider_registry.rs:47` `available_names()`：在 `ProviderName::Pi` 后、`ProviderName::Fake` 前插入 `ProviderName::KimiCode`。

- [ ] Run: `cargo test --locked --lib provider_health` 与 `cargo test --locked --lib provider_registry`
- Expected: 各自覆盖 Kimi 可用/缺失/版本过低（解析 `< 0.34.0` 报不可用）/stable order；先 FAIL 后 PASS。

## Step 5: 状态 API DTO + provider_availability

`src/web/handlers/providers.rs:82` 的 match 加：

```rust
ProviderName::KimiCode => (
    "kimi_code",
    "Kimi Code",
    "Install Kimi Code CLI and ensure `kimi` is available on PATH.",
),
```

`src/web/provider_availability.rs`：`parse_provider_name` 加 `"kimi_code" => Some(ProviderName::KimiCode)`；`provider_name_key` 加 `ProviderName::KimiCode => "kimi_code"`；PATH 探测列表加 `kimi`。

- [ ] Run: `cargo test --locked --lib handlers::providers` 与 `cargo test --locked --lib provider_availability`
- Expected: DTO 返回 kimi_code/Kimi Code/install_hint；先 FAIL 后 PASS。

## Step 6: Task Runner 四入口稳定拒绝 Kimi

- `src/task_run/provider_factory.rs:66-73` `RoutingProviderAdapter::run`：加
  ```rust
  ProviderType::KimiCode => Err(ProviderAdapterError::incompatible_output(
      /* 与 Pi 同款错误构造，标识 kimi_code */
  )),
  ```
- `src/task_run/step_runner.rs:113` `provider_type_text`：加 `ProviderType::KimiCode => "kimi_code",`（**不写 `unreachable!`**）
- `src/web/runtime/provider.rs:498`：加 `ProviderType::KimiCode => None,`
- `src/web/runtime/utils.rs:62`：KimiCode 返回稳定空/错误（**不写 `unreachable!`**）

断言 `adapter_compatibility` 兼容性矩阵**无** Kimi 条目（不动矩阵）。

- [ ] Run: `cargo test --locked --lib provider_factory` 与 `cargo test --locked --lib step_runner` 与 `cargo test --locked --lib runtime::provider` 与 `cargo test --locked --lib runtime::utils`
- Expected: Kimi 返回 incompatible / 稳定文本 / None，不 panic；先 FAIL 后 PASS。

## Step 7: 前端 catalog + 仓库初始化过滤

`web/src/api/types/provider.ts:1` `RealProviderName` 加 `"kimi_code"`。

`web/src/state/provider-options.ts`：
- `REAL_PROVIDER_CATALOG` 加 `{ value: "kimi_code", fallbackLabel: "Kimi Code" }`
- `PROVIDER_ORDER` 在 `"pi"` 后、`"fake"` 前加 `"kimi_code"`

`web/src/components/lifecycle/CreateRepositoryDialog.tsx`：仓库初始化 provider 过滤逻辑加排除 `"kimi_code"`（与 Pi 同），加注释 `// capability policy: 仅 Claude Code 可初始化仓库，Kimi 同 Pi 不参与`。

更新 `web/src/state/provider-options.test.ts`：catalog 含 kimi_code、order 正确、可用/不可用/禁用断言。

- [ ] Run: `cd web && pnpm test -- provider-options` 与 `cd web && pnpm test -- CreateRepositoryDialog`
- Expected: 先 FAIL 后 PASS。

## Step 8: 质量检查与提交

- [ ] Run: `cargo fmt --check` / `cargo clippy --all-targets --all-features --locked -- -D warnings` / `cargo test --locked` / `cd web && pnpm tsc -b`
- Expected: 全绿
- [ ] Commit:
```bash
git add -A
git commit -m "feat(kimi): Task 1 注册 KimiCode provider（枚举/健康检查/状态API/前端catalog/task-run稳定拒绝/仓库初始化过滤）"
```
