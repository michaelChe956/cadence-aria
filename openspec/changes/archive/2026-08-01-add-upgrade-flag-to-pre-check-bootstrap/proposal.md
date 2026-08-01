# Proposal: add-upgrade-flag-to-pre-check-bootstrap

## Why

Cadence-skills 的 `/pre-check` 命令新增 `--upgrade` 参数。代码库初始化链路第二步 `pre_check` 由 `RepositoryInitializationStepKind::command()` 提供提示词，经 Claude Code Provider 在目标 Git 根作为工作目录执行；当前提示词为 `/pre-check --no-interrupt 用大陆镜像`，未携带 `--upgrade`，与 Cadence-skills 最新命令契约不一致。

## What Changes

- `pre_check` 步骤的 Claude Code 提示词改为 `/pre-check --no-interrupt --upgrade 用大陆镜像`；`--upgrade` 位于 `--no-interrupt` 之后、`用大陆镜像` 之前。
- 其余三条命令（`/rule-config`、`/mcp-configuration`、`/project-rules-examples`）保持不变，SHALL NOT 附带 `--upgrade`。
- 本仓库仅负责发送命令字符串，不解释 `--upgrade` 语义；`--upgrade` 的行为由 Cadence-skills 的 `/pre-check` 命令定义。

## Non-goals

- 不改变六步固定步骤的数量、顺序与状态机语义。
- 不为其余三条初始化命令追加 `--upgrade`。
- 不引入"新建初始化 vs 升级初始化"模式概念；`command()` 仍返回静态字符串，所有初始化共用同一条 `pre_check` 命令。
- 不改变命令生成/发送机制；`command()` 仍为 `Option<&'static str>`。
- 不新增配置项；命令字符串仍为代码内常量。

## Capabilities

### Modified Capabilities

- `non-interrupt-repository-bootstrap`：`pre_check` 命令提示词追加 `--upgrade` 参数。

### New Capabilities

- 无

## Impact

- 后端：`src/product/repository_store/types.rs`（`PreCheck` 命令字符串）。
- 测试：引用该字符串的 Rust 单元/集成测试与前端测试断言同步更新。
- 前端：无功能变化；仅测试断言中的命令字符串同步。
- 不新增第三方依赖。
