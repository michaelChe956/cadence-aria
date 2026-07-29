## Why

CLI provider adapter 把 provider 进程的 stdout/stderr 流日志写到 provider 子进程的工作目录下，即被开发的目标代码库 worktree。`provider_stream_path`（`src/cross_cutting/cli_adapter.rs:414`）直接用 `current_dir`（来源为 `AdapterInput.worktree_path`）拼接 `.aria/runtime/provider-streams`，把「provider 进程工作目录」与「Aria 状态根」当作同一个概念。

真实现象：开发 naruto 仓库时，在目标 worktree 下生成 `/home/michaelche/workspace/github/naruto/.worktrees/aria-issues/issue_0001/.aria/runtime/provider-streams`。

后果：

- Aria 在用户仓库中凭空创建 `.aria/` 目录，污染被开发的代码库。
- 目标仓库若未忽略 `.aria/`，流日志会进入 `git status`，可能被 Coder 连带提交。
- 流日志脱离 attempt 生命周期，排查时与同一次执行的 `provider-raw` 产物分处两地。

同一次执行的 provider 原始输出（`provider-raw/<stage>/<purpose>_NNNN.txt`）已正确落在 Aria 侧 attempt 目录下，由 engine 通过 attempt store 写入。只有流日志这一条路径走偏。

## What Changes

- `AdapterInput` 新增可选的流日志目录字段，由调用方在构造 input 时提供 Aria 侧目录；adapter 只向给定目录写入，不再自行推导路径。
- CLI provider adapter 移除基于 provider 进程工作目录的流日志路径推导。未提供流日志目录时不写流日志文件，MUST NOT 退化为写入 provider 工作目录。
- 全部持有 coding attempt 上下文的 provider 执行路径提供该 attempt 的流日志目录，使流日志与 `provider-raw` 同处一个 attempt 目录下，并随 attempt 删除一并清理。这既覆盖实际调用 `ProviderAdapter::run` 的 handoff 生成路径，也覆盖当前经 streaming fallback 路由、暂不写流日志的 coder / code review / rework / internal review / testing 路径：填值而非留空，避免将来 fallback 路由变化时静默退化为写入目标仓库。
- 无 attempt 上下文的执行路径（task run 各运行单元、provider workspace session、work item split）不提供流日志目录，按缺省行为不写流日志文件。
- 流日志目录必须为绝对路径。传入非绝对路径（含空串）时按未提供处理，避免退化为写入 Aria 进程的当前工作目录。
- 不改变流日志的文件命名规则、写入模式（追加）与内容。
- 不改变 `provider-raw` 原始输出的落盘路径与写入方式。
- 不改变 provider 子进程的工作目录：provider 仍在目标 worktree 中执行。
- 不自动清理历史遗留在目标仓库中的流日志目录。

## Capabilities

### New Capabilities

- `provider-stream-log-placement`: provider 进程流日志的落盘位置语义，包括目录来源、缺省行为与目标仓库隔离约束。

### Modified Capabilities

（无。现有 specs 未覆盖 provider 流日志落盘位置。）

## Impact

- `src/protocol/contracts.rs`：`AdapterInput` 新增可选流日志目录字段。该结构为协议契约，全部构造点需要补齐字段（可用结构体更新语法或默认值收敛）。
- `src/cross_cutting/cli_adapter.rs`：`provider_stream_path` 改为消费传入目录，移除 `current_dir` 推导。
- `src/product/coding_attempt_store/paths.rs`：新增 attempt 级流日志目录解析。
- `src/product/coding_workspace_engine/handoffs.rs`：handoff 生成路径填入 attempt 流日志目录。
- 受影响的用户可见行为：被开发的目标代码库不再出现 Aria 生成的 `.aria/runtime/provider-streams` 目录；流日志改为与该次 attempt 的 `provider-raw` 同处一处。
- 不影响 provider 的执行结果、结构化输出解析、plan defect 判定与任何门禁行为。
- 不影响 streaming provider 路径：该路径不写流日志文件。

### 已知可观测性缺口（需后续 change 处理）

无 attempt 上下文的执行路径此前把流日志写进用户仓库，本变更后完全不写。这些路径的 `ProviderRunRecord` 只保存 `stdout_ref` / `stderr_ref` 这类引用字符串（`src/cross_cutting/provider_run.rs`），仓库中没有代码把这些引用物化成文件，因此流日志曾是这些路径在磁盘上唯一的 provider 原始输出。

停止污染用户仓库优先于保留该输出，故本变更不在此处补落点。但这不是零成本，需后续 change 为 task run 各运行单元、provider workspace session 与 work item split 定义 Aria 侧的原始输出落点。

### 已知同型缺陷（本变更范围外）

`src/product/coding_workspace_engine/testing_parser.rs` 的 `parse_test_execution_payload_from_provider_output` 仍使用单候选 `extract_json_object`，与 plan defect 解析修复前同型：provider 正文中的花括号片段可能被误判为结构化结论。经确认本轮不处理，需后续单独评估与修复。
