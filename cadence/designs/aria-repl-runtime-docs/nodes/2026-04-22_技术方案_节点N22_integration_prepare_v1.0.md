# 节点文档：N22 integration_prepare

## 1. 节点标识

- 节点 ID：`N22`
- 节点类型：Aria 内部
- 主执行者：Aria daemon
- 所属链路：集成准备
- 版本：v1.0

## 2. 节点目的

为当前出队任务执行集成前同步、冲突预判和环境检查。

## 3. 进入条件

- WorkTask 已从集成队列出队

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| queue record | `N21` | 是 | ref | 回退 `N21` |
| worktree ref | `N14` | 是 | ref | 回退 `N14` |
| current base ref | repo | 是 | git ref | 中止集成 |

## 5. Aria 驱动动作

1. 更新集成基线。
2. 检查冲突风险。
3. 执行必要预检。
4. 写 snapshot。

## 6. Provider 执行契约

无。

## 7. 输出产物

- `runtime_snapshot:N22.integration_prepare`

## 8. 输出产物最小格式

snapshot 至少包含：

- `workTaskId`
- `baseRefBeforeIntegration`
- `precheckStatus`
- `conflictRisk`

## 9. 完成判定

已明确可执行集成，或已明确必须回流。

## 10. 失败与回流

- 冲突明显：回流 `N19 rework`
- 预检失败：gate 或人工介入

## 11. 交接到下一节点

- 通过：进入 `N23 integration_execute`
- 不通过：回流 `N19`

## 12. 关联横切能力

- `integration_queue`
- `worktree_lifecycle`
- `approval_gate`

