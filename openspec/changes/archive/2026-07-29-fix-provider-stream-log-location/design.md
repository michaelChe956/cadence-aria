## 背景

`provider_stream_path`（`src/cross_cutting/cli_adapter.rs:414`）签名的第一个参数是 `current_dir: Option<&Path>`，与传给 `Command::current_dir` 的是同一个值。该值来源为 `AdapterInput.worktree_path`（`src/cross_cutting/cli_adapter.rs:71`、`:88`），即被开发的目标仓库 worktree。因此流日志落在目标仓库而非 Aria 状态目录。

仓库已有 `AriaStatePaths`（`src/cross_cutting/aria_state_paths.rs`）承载 Aria 状态根解析，但 adapter 完全拿不到 Aria 侧任何路径。

## 关键约束

adapter 的生命周期长于单次执行：`CliProviderAdapter` 在 `real_routing_provider_with_output_sink`（`src/task_run/provider_factory.rs:94-121`）构造一次，经 `install_provider_gate`（`src/web/runtime/provider.rs:154-180`）安装后跨所有 attempt 复用。而 attempt 目录只在单次执行时确定。

因此流日志目录 MUST 走**单次执行的输入**（`AdapterInput`），不能走 adapter 构造期配置（`CliAdapterConfig`）。这决定了本变更必须触及协议契约。

## 决策

### 决策一：流日志目录经 `AdapterInput` 传入

在 `AdapterInput`（`src/protocol/contracts.rs:195`）新增可选字段承载流日志目录。adapter 只消费该字段，不做任何路径推导。

理由：这是唯一能把「单次执行的 attempt 上下文」送达 adapter 的通道。代价是协议结构新增字段，全部构造点需补齐（非测试约 12 处，含测试共 36 处）。

被否决的替代方案：

- **放入 `CliAdapterConfig`**：adapter 构造期无 attempt 上下文，落点只能退化为全局 runtime 根，无法满足 attempt 级归属。
- **改由 `output_sink` 上送、adapter 不写文件**：职责更干净，但需要改造 sink 的消费端语义并确认其承载完整原始流，改动面与风险大于本方案，且与「保持流日志内容和命名不变」冲突。

### 决策二：缺省时不写流日志，不回退

字段为空时 adapter 不写流日志文件。这与现状一致：现有实现在 `current_dir` 为 `None` 时同样返回 `None` 而不写。

关键是 MUST NOT 把「未提供目录」实现成「回退到工作目录」，否则缺陷原样保留。

### 决策三：coding attempt 的落点与 `provider-raw` 同根

流日志目录取该 attempt 的 artifact 根下一级，与 `provider-raw`（`src/product/coding_attempt_store/paths.rs:358` 的 `save_provider_raw_output`）同属一个 attempt 根。

`cli_adapter` 实现的是 `ProviderAdapter` trait（`src/cross_cutting/cli_adapter.rs:62`），与 coding 流式路径所用的 `StreamingProviderAdapter` 是两条不同的 trait。因此填入点是全部实际调用 `ProviderAdapter::run` 且提供 `worktree_path` 的生产路径：

| 调用点 | 传入的工作目录 | 是否有 attempt 上下文 |
|---|---|---|
| `src/product/coding_workspace_engine/handoffs.rs:447` | attempt worktree | 有 |
| `src/runtime_units/coding.rs:410` | 运行单元 worktree | 无（task run 上下文） |
| `src/runtime_units/final_review.rs:277` | 运行单元 worktree | 无（task run 上下文） |
| `src/runtime_units/clarification/provider.rs:40` | 运行单元 worktree | 无（task run 上下文） |
| `src/product/provider_workspace_runner.rs:53` | 仓库根 `repository.path` | 无（workspace session） |
| `src/product/work_item_split_engine/engine.rs:134` | 仓库根 `repository.path` | 无（split 阶段） |

其中 `provider_workspace_runner` 与 `work_item_split_engine` 传入的是仓库根而非 worktree，这与观察到的现象形态一致。

只有 `handoffs.rs` 持有 `attempt`，可落到 attempt 目录。其余调用点无 attempt 上下文，按决策二不写流日志——这已经消除了污染目标仓库的行为，是本变更的主要收益。

收益：有 attempt 上下文时，流日志与同次执行的原始输出相邻，便于排查，且随 attempt 目录删除自动清理；无 attempt 上下文时不再向用户仓库写入任何内容。

### 决策四：不改变 provider 工作目录

provider 子进程仍以目标 worktree 为工作目录。本变更只解耦「工作目录」与「流日志落点」两个概念，不触碰 provider 的执行语义。

## 边界

- 不改流日志文件命名（`{provider}-{child_id}-{stream}.log`）、追加写入模式与内容。
- 不改 `provider-raw` 落盘路径与写入方式。
- 不改 streaming provider 路径：该路径不写流日志文件，`provider-streams` 全仓仅 `cli_adapter.rs` 一处写入。
- 不自动清理历史遗留在目标仓库中的流日志目录。
- 非 coding 场景（task run、work item split、provider workspace runner 等）的 `AdapterInput` 构造点可不提供流日志目录，按决策二不写流日志；本变更不为这些场景新增落点。
