# provider-stream-log-placement Specification

## Purpose
TBD - created by archiving change fix-provider-stream-log-location. Update Purpose after archive.
## Requirements
### Requirement: 流日志目录由调用方提供

provider adapter MUST NOT 从 provider 子进程的工作目录推导流日志路径。流日志目录 MUST 由调用方通过 adapter 输入显式提供，且 MUST 为绝对路径。

#### Scenario: 非绝对路径按未提供处理

- **WHEN** 调用方提供的流日志目录为空串或相对路径
- **THEN** adapter MUST NOT 写入流日志文件，且 MUST NOT 退化为写入 provider 工作目录或 Aria 进程的当前工作目录

#### Scenario: adapter 使用调用方提供的目录

- **WHEN** 调用方在 adapter 输入中提供流日志目录，且 provider 在某个 worktree 中执行
- **THEN** 流日志文件 MUST 写入所提供的目录，MUST NOT 写入 provider 的工作目录

#### Scenario: 未提供目录时不写流日志

- **WHEN** 调用方未在 adapter 输入中提供流日志目录
- **THEN** adapter MUST NOT 写入任何流日志文件，且 MUST NOT 回退到 provider 的工作目录

### Requirement: 目标代码库不得被写入流日志

Aria MUST NOT 在被开发的目标代码库中创建流日志目录或文件。provider 子进程的工作目录 MUST 保持为目标 worktree，但该目录 MUST NOT 被用作流日志落盘位置。

#### Scenario: provider 在目标 worktree 中执行后目标库无新增目录

- **WHEN** provider 以某目标代码库 worktree 作为工作目录完成一次执行
- **THEN** 该 worktree 下 MUST NOT 出现由流日志写入产生的 `.aria/runtime/provider-streams` 目录

### Requirement: coding attempt 的流日志落在该 attempt 目录下

持有 coding attempt 上下文的 provider 执行路径 MUST 提供该 attempt 的流日志目录，使流日志与同一次 attempt 的 provider 原始输出同处一个 attempt 目录下。该要求不区分该路径当前是否实际写入流日志：即使经 streaming 路由暂不写入，也 MUST 填入目录，以免路由变化时退化为写入目标仓库。无 attempt 上下文的执行路径 MUST NOT 提供流日志目录。

#### Scenario: 流日志与原始输出同处一个 attempt 目录

- **WHEN** 某 coding attempt 的 handoff 生成完成一次 provider 执行
- **THEN** 该次执行的流日志 MUST 位于该 attempt 的目录下，与该 attempt 的 `provider-raw` 产物同属一个 attempt 根

#### Scenario: 无 attempt 上下文时不写流日志

- **WHEN** task run 运行单元、provider workspace session 或 work item split 执行 provider
- **THEN** MUST NOT 写入流日志文件，且 MUST NOT 向该次执行的工作目录写入任何流日志

#### Scenario: 流日志随 attempt 删除一并清理

- **WHEN** 用户删除一个已产生流日志的 coding attempt
- **THEN** 该 attempt 的流日志 MUST 随 attempt 目录一并被清理，MUST NOT 残留于其他位置

### Requirement: 流日志的命名与写入语义保持不变

流日志的文件命名规则、追加写入模式与内容 MUST NOT 改变。目录不存在时 MUST 先创建再写入。

#### Scenario: 命名规则未改变

- **WHEN** 同一 provider 进程分别写入 stdout 与 stderr 流日志
- **THEN** 两个文件 MUST 仍按 provider 名、子进程标识与流名区分，且 MUST 仍以追加模式写入

### Requirement: provider 原始输出落盘不受影响

本变更 MUST NOT 改变 `provider-raw` 原始输出的落盘路径、命名与写入方式，也 MUST NOT 改变 provider 执行结果、结构化输出解析与任何门禁判定。

#### Scenario: 原始输出路径未改变

- **WHEN** engine 在某 attempt 中保存一次 provider 原始输出
- **THEN** 该产物 MUST 仍落在该 attempt 的 `provider-raw/<stage>/` 下，命名规则 MUST 保持不变

