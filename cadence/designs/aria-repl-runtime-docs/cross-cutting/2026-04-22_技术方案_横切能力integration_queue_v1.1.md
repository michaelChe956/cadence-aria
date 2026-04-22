# 横切能力文档：integration_queue

## 1. 能力标识

- 能力 ID：`CC07`
- 能力名称：`integration_queue`
- 类型：调度 / 集成
- 适用范围：`N20` 到 `N24`

## 2. 能力目的

在允许多任务并行执行的前提下，保证主线合入保持串行和可审计。

## 3. 触发条件

- WorkTask 进入 `ready_for_integration`

## 4. 前置状态与输入

- ready task ref
- latest base ref
- worktree ref
- validation refs

## 5. Aria 执行动作

1. 将任务加入 integration queue
2. 按顺序出队
3. 准备集成前同步基线
4. 执行冲突检查与回归验证

## 6. 状态变化与副作用

- 更新 queue order
- 任务可能从 `ready` 变为 `integrating`
- 冲突时任务退回 `rework`

## 7. 输出与记录

- integration queue record
- integration attempt record

## 8. 完成判定

任务已成功合入或已明确回流。

## 9. 失败与恢复

- daemon 重启：恢复 queue 顺序
- 集成失败：保留现场并退回上游节点

## 10. 与节点文档的关联规则

`N21` 至 `N24` 必须引用该能力，其他节点只引用其结果。

## 队列排序策略

### 默认排序规则

- 默认采用 FIFO（先到先得）排序
- 入队时间以 N21 节点的 `enqueueTime` 为准
- 多个 WorkTask 同时 ready 时，按入队顺序排列

### 优先级覆写

- 支持手动优先级覆写，用户可通过 REPL 执行 `queue priority <workTaskId> <level>`
- 优先级级别：
  - `critical`：最高优先级，立即出队（如紧急修复）
  - `high`：优先于普通任务
  - `normal`：默认优先级
  - `low`：可延后处理的任务
- 同优先级内保持 FIFO 顺序
- 优先级变更记录写入 event log

### 依赖拓扑感知

- 如果 WorkTask A 依赖 WorkTask B 的产物，即使 A 的优先级更高，也必须等 B 完成集成后才能出队
- 依赖关系来自 N13 注册时的 `dependencyGraph`
- 循环依赖检测在 N13 已完成，此处不再重复

### 队列持久化与恢复

- 队列状态通过 checkpoint 持久化
- 持久化内容包括：
  - 队列中所有 WorkTask 的 ID 和优先级
  - 当前正在集成的 WorkTask ID（如果有）
  - 每个任务的入队时间
- daemon 重启后恢复流程：
  1. 从最新 checkpoint 恢复队列状态
  2. 回放 checkpoint 之后的 event 补充队列变更
  3. 正在集成的任务恢复为 `recovering` 状态
  4. 恢复完成后队列可继续出队

### 出队条件

- 队列中排在前面的任务满足以下条件时才可出队：
  1. 任务状态为 `ready_for_integration`
  2. 所有前置依赖任务已完成集成（`completed` 状态）
  3. 当前无其他任务正在集成（互斥保证）
  4. 集成基线（main/target branch）未被锁定

### 回流次数限制

- 同一 WorkTask 连续集成失败回流超过 2 次后：
  - 不再自动出队
  - 升级到 X08（manual_intervention_hold）
  - intervention record 中注明 `integration_retry_limit_exceeded`

