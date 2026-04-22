# 节点文档：N21 integration_enqueue

## 1. 节点标识

- 节点 ID：`N21`
- 节点类型：Aria 内部
- 主执行者：Aria daemon
- 所属链路：集成排队
- 版本：v1.0

## 2. 节点目的

把可集成任务放入串行集成队列，确保主线合入顺序可控。

## 3. 进入条件

- WorkTask 状态为 `ready_for_integration`

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| ready snapshot | `N20` | 是 | ref | 回退 `N20` |
| queue state | runtime | 是 | current queue | 恢复 queue 后重试 |

## 5. Aria 驱动动作

1. 将任务入队。
2. 记录入队顺序。
3. 写 queue snapshot。

## 6. Provider 执行契约

无。

## 7. 输出产物

- `runtime_snapshot:N21.integration_enqueue`

## 8. 输出产物最小格式

snapshot 至少包含：

- `workTaskId`
- `queuePosition`
- `enqueueTime`

## 9. 完成判定

队列中存在可追踪的入队记录。

## 10. 失败与回流

- 队列写入失败：重试或进入人工介入

## 11. 交接到下一节点

出队时进入 `N22 integration_prepare`。

## 12. 关联横切能力

- `integration_queue`
- `checkpoint_and_recovery`

