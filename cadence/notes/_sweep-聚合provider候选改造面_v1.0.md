## 聚合初始化 provider 候选改造面

> pwd：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`

## Q1 provider 路径

- 写入点 `src/web/handlers/aggregate_initialization/handlers.inc.rs:92-99`：
  ```rust
  let input = crate::product::logical_codebase::AggregateInitializationOperationInput {
      idempotency_key: idempotency_key.to_string(),
      manifest_revision: manifest.membership_revision,
      policy_digest: "sha256:aggregate-policy".to_string(),
      profile_evidence_digest: Some("sha256:profile".to_string()),
      provider_context_root: manifest.provider_context_root.clone(),
      provider: "claude_code".to_string(),
  };
  ```
- 定义 `src/product/logical_codebase/aggregate_initialization.rs:351-360`：
  ```rust
  pub struct AggregateInitializationOperationInput {
      pub idempotency_key: String,
      pub manifest_revision: u64,
      pub policy_digest: String,
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub profile_evidence_digest: Option<String>,
      pub provider_context_root: PathBuf,
      pub provider: String,
  }
  ```
  operation 记录的 `input` 字段在同文件 `:368-376`。
- trust 清单硬编码 `&[crate::product::models::ProviderName::ClaudeCode]`（handlers.inc.rs 约 612 行）。`src/web/handlers/aggregate_initialization/production_dependencies.inc.rs:122-146` 每 turn 构 gateway 后调用 `claude_code_with_admission(...)`；兼容分支 `:189-192` 调 `claude_code(...)`。
- driver 固定 provider：`src/product/logical_codebase/coordinator_provider_turn.inc.rs:44-56` 构造 `ProviderRef::claude_code(...)`，`:110-127` request 取 `self.provider.clone()`。经 gateway streaming（`:11-15`），adapter registry 依 `provider_type` 选 adapter（`src/cross_cutting/gateway_sync_provider.rs:77-80`），CLI 最终由 `src/cross_cutting/process_manager.rs:207-243` 的 `Command::new(command)` / spawn 启动。

结论：aggregate 执行链未读取 `AggregateInitializationOperationInput.provider` 用于选择 provider；它是记录字段。实际选择来自 Claude 专属 driver。

## Q2 构造差异

- `claude_code` 固定 Claude ProviderRef、`admission_paths: None`：`coordinator_provider_turn.inc.rs:44-56`。
- `claude_code_with_admission` 设 `paths: Some` 并以 `..Self::claude_code(...)` 复用初始化：`:58-70`。预检成员规则、policy/capability，失败在 spawn 前 waiting（`:58-60`）。
- 执行经过 `LogicalCodebaseProviderGateway::start_streaming`（`:11-15`）和 streaming adapter，再由 CLI 进程管理层启动。
- 没有按 ProviderName 参数化的 aggregate driver 构造。`RoutingProviderAdapter::new(claude,codex)`（`src/task_run/provider_factory.rs:13-16,35-60`）仅两个 provider，不是 aggregate 通用构造。

## Q3 gateway factory

`src/web/gateway_factory.rs:104-113` 中 `build_for_lc_material_prep` 委托 `build_scoped(..., true)`；`:125-151` 解析 LC scope、加载 manifest、构造 policy/capability stores；`:175-190` 组装 capability source/resolver。未按 provider 配置选 provider。manifest 使用 `provider_context_root`（`:130-132,152-155`）。LC capability store 存在（`:144-150`）；trust 是信任事实而非偏好。未发现 LC provider candidates/preference 模型。operation input 当前仅一个 provider String（Q1）。

## Q4 admission 错码

- `src/product/logical_codebase/provider_admission_check.inc.rs:533-549` 注释说明 ProviderUnavailable 与 drift 维度归并为 `spawn_revalidation_drift`；trust missing 另处理。故 detail 保留 command_missing，外层码为 drift。
- `src/cross_cutting/process_manager.rs:214-216` 检查命令可解析性；`:301-303`：
  ```rust
  fn command_missing(command: &str) -> ProviderAdapterError {
      ProviderAdapterError::command_missing(format!("provider command not found: {command}"))
  }
  ```
- `missing_actions:[I]` 构造源未能在源码中定位。`cadence/notes/2026-11-01_待办_平台已知未修问题清单_v1.0.md:429-433` 仅称其为占位符。

## Q5 capability 与核验

- `src/cross_cutting/provider_capabilities.rs:9-24` 实际三态：Confirmed、Denied { reason }、Unknown。
- Planning launch 错误构造：`src/product/logical_codebase/production_policy_resolvers.rs:301-316`，只放行 Confirmed，其他状态进入 `... launch cell is {other:?}`。
- POST 路由 `src/web/app.rs:175-177`；handler `src/web/handlers/logical_codebase_capabilities.rs:106-110` `post_lc_capability_revalidate`。
- 服务注释 `src/product/logical_codebase/provider_capability_revalidate.rs:1-10`：现场 boundary probe 并导入 durable Confirmed；失败不伪造 Confirmed，全已确认时秒回。action 为 Coding/Planning/Review（`:35-39`），四家 CLI 映射、Fake 无通道（`:46-53`）。按单个 provider 执行；部分 rows 只探缺失 actions 的测试名见 `:790`。注释称同步等待，但没有固定耗时证据，实际取决于 CLI/probe timeout。snapshot 有 provider_type（`:143-151`）。

## Q6 改造边界及复用能力

已有：LC capability store/重验证（按 provider/action）；provider health 全局逐家 CLI probe（`src/cross_cutting/provider_health.rs:174-210,230-251`）；ProcessManager 启动时检查命令并返回 command_missing（`:214-216`）。health refresh 是全局快照，不是 candidate-first 选择器。

缺失：LC provider candidates/preference 模型；候选优先选择；operation 内最终实际 provider 的独立 receipt 字段。operation record `src/product/logical_codebase/aggregate_initialization.rs:368-402` 有 input、steps、output refs、errors/actions，没有 actual provider receipt；input.provider 是 String。

### 改造点（按文件）

- `src/web/handlers/aggregate_initialization/handlers.inc.rs`：operation provider 写入、trust 清单。
- `src/web/handlers/aggregate_initialization/production_dependencies.inc.rs`：Claude 专属 driver 两分支。
- `src/product/logical_codebase/coordinator_provider_turn.inc.rs`：ProviderRef 固定构造、turn provider 绑定。
- `src/product/logical_codebase/aggregate_initialization.rs`：单 provider input、缺候选/实际 provider receipt。
- `src/web/gateway_factory.rs`：网关/store 组装边界；不含偏好选择。
- `src/product/logical_codebase/provider_admission_check.inc.rs`：availability/drift reason code 折叠。
- `src/cross_cutting/process_manager.rs`：command_missing 来源。
- `src/cross_cutting/provider_health.rs`：本机 CLI health probe 复用面。
- `src/product/logical_codebase/provider_capability_revalidate.rs`、`provider_capability_store.rs`：capability 核验与 LC 状态复用面。

### 检索范围与命令

范围：aggregate handlers/dependencies、driver、gateway factory、capability/admission/provider health、HTTP handler、notes。命令/工具：`pwd`；`codegraph init`；CodeGraph explore（operation input、driver、gateway factory）；文本检索 `input.provider`、`build_for_lc_material_prep`、`capability-revalidate`、`spawn_revalidatability_drift`、`missing_actions`、`ProviderCapabilityEvidence`；定向读取实现文件。
