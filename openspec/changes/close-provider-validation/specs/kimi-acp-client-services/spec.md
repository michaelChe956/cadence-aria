# kimi-acp-client-services Delta

## MODIFIED Requirements

### Requirement: 终端生命周期与资源边界（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

terminal 状态机 SHALL 为：created→running→(exited|killed)→released；重复 kill/release 幂等成功；未知 terminalId 返回可区分错误。执行方式 SHALL 默认 pipe + 独立 stdout/stderr + 进程组 + 每命令超时；仅当 fixture 证明 kimi 依赖 TTY 时引入 pty（`TERM=dumb`、窗口与 EOF 行为显式设定），运行时模式记入 capability/审计，不做静默降级。资源上限（默认值，常量可配置）：每会话最大并发终端数 `MAX_TERMINALS=4`（第 1~4 个成功，第 5 个拒绝）、单终端最大输出字节 `MAX_TERMINAL_OUTPUT_BYTES=1048576`（stdout+stderr 合计；恰为 1048576 完整返回，1048577 截断并只标记一次）、命令超时 `TERMINAL_COMMAND_TIMEOUT_SECS=120`（超时后进程组终止且 RPC 队列继续服务）；输出回传 backpressure 不阻塞 RPC 队列；会话结束/取消时按进程组清理全部终端，无残留进程。终端执行作为 execution event 记录供用户审计。

**确定性验证条款（DEF-7 桶根治）**：cap 与截断标记的边界行为——恰为上限完整返回且不标记截断、超限截断且只标记一次、并发双流共享预算不超额——SHALL 以不依赖真实进程计时预算的确定性测试锁定（受控内存流/可复现字节供给直接驱动输出预算逻辑，无墙钟预算依赖）；真实进程管道端到端行为 SHALL 保留至少一条 smoke 测试覆盖（create→输出流→wait_for_exit→release 全链路与 cap 生效的最小断言）。验证方式改造 MUST NOT 改变本 requirement 上述任何行为语义（上限常量、合计口径、截断标记语义、并发预算原子性均不变）；MUST NOT 以放宽断言、加大超时预算或标记 ignore 等症状压制手段替代根治。

#### Scenario: 正常链路

- **WHEN** create→输出流→wait_for_exit→release
- **THEN** 全链路成功，事件时间线可见命令与输出摘要

#### Scenario: 清理

- **WHEN** 会话取消时存在运行中终端
- **THEN** 进程组被终止，无残留子进程

#### Scenario: 恰为上限不截断的确定性锁定

- **WHEN** 以受控内存流确定性供给恰好 `MAX_TERMINAL_OUTPUT_BYTES` 字节
- **THEN** 全部字节完整返回且截断不被标记，断言无计时预算依赖（机器负载不敏感）

#### Scenario: 超限截断只标记一次的确定性锁定

- **WHEN** 以受控内存流确定性供给 `MAX_TERMINAL_OUTPUT_BYTES + 1` 字节
- **THEN** 恰好保留前 `MAX_TERMINAL_OUTPUT_BYTES` 字节、截断被标记且仅一次，断言无计时预算依赖

#### Scenario: 并发流共享预算不超额的确定性锁定

- **WHEN** 以受控内存流确定性并发供给双流（stdout 与 stderr 各自至多 `MAX_TERMINAL_OUTPUT_BYTES` 字节、总量足以击穿上限）
- **THEN** 合计保留恰好 `MAX_TERMINAL_OUTPUT_BYTES` 字节、截断被标记，任何交错下不超额，断言无计时预算依赖

#### Scenario: 真实进程管道 smoke 保留

- **WHEN** 运行保留的真实进程 smoke 用例（真实子进程经 pipe 泵输出）
- **THEN** create→输出流→wait_for_exit→release 全链路成功且输出 cap 生效，该用例在隔离过滤跑下稳定通过（唯一保留的真实进程端到端覆盖）

#### Scenario: 根治不改变行为语义

- **WHEN** 确定性改造完成后的全量回归运行
- **THEN** 本 requirement 的全部行为约束（上限常量、合计口径、截断标记语义、并发预算原子性、幂等清理）与改造前一致，无断言放宽或用例删除导致的保护面缩水
