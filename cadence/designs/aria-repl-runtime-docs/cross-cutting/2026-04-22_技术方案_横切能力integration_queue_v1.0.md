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

