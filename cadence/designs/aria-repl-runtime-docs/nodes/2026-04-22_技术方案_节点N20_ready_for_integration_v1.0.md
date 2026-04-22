# 节点文档：N20 ready_for_integration

## 1. 节点标识

- 节点 ID：`N20`
- 节点类型：Aria 内部
- 主执行者：Aria daemon
- 所属链路：集成前就绪
- 版本：v1.0

## 2. 节点目的

把已经通过 testing 和 code review 的 WorkTask 标记为可进入集成队列。

## 3. 进入条件

- `code_review_report.allow_integration = true`

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `coding_report` | `N16`/`N19` | 是 | ref | 回退上游 |
| `testing_report` | `N17` | 是 | ref | 回退 `N17` |
| `code_review_report` | `N18` | 是 | ref | 回退 `N18` |

## 5. Aria 驱动动作

1. 检查当前 WorkTask 是否已具备全部前置通过项。
2. 把任务状态改为 `ready_for_integration`。
3. 写 snapshot。

## 6. Provider 执行契约

无。该节点不调用 provider。

## 7. 输出产物

- `runtime_snapshot:N20.ready_for_integration`

## 8. 输出产物最小格式

snapshot 至少包含：

- `workTaskId`
- `readyAt`
- `validationRefs`
- `worktreeRef`

## 9. 完成判定

任务已被队列系统识别为 ready。

## 10. 失败与回流

- 前置结果不一致：进入 `runtime_recover` 或回流 `N18`

## 11. 交接到下一节点

进入 `N21 integration_enqueue`。

## 12. 关联横切能力

- `integration_queue`
- `checkpoint_and_recovery`

