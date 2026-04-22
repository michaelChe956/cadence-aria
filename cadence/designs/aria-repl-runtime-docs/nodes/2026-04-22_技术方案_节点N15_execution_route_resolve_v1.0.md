# 节点文档：N15 execution_route_resolve

## 1. 节点标识

- 节点 ID：`N15`
- 节点类型：Aria 内部
- 主执行者：Aria daemon
- 所属链路：执行路由
- 版本：v1.0

## 2. 节点目的

根据依赖关系、策略与前置条件，决定某个 WorkTask 下一步进入 coding、testing 还是 code_review。

## 3. 进入条件

- WorkTask 已注册
- 所需 worktree 已就绪

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| WorkTask ref | `N13` | 是 | task id | 回退 `N13` |
| worktree ref | `N14` | 是 | worktree id | 回退 `N14` |
| current task state | runtime | 是 | last node/status | 回退恢复 |

## 5. Aria 驱动动作

1. 判断前置依赖是否满足。
2. 判断当前应走哪个执行节点。
3. 生成 execution routing snapshot。

## 6. Provider 执行契约

无。该节点不直接调用 provider。

## 7. 输出产物

- `runtime_snapshot:N15.execution_route_resolve`

## 8. 输出产物最小格式

snapshot 至少包含：

- `workTaskId`
- `nextExecutionNode`
- `routingReason`
- `effectivePolicy`

## 9. 完成判定

后续执行节点已唯一确定。

## 10. 失败与回流

- 状态不一致：进入 `runtime_recover`

## 11. 交接到下一节点

- 首次执行通常进入 `N16 coding`
- 部分补偿路径可直接进入 `N17` 或 `N18`

## 12. 关联横切能力

- `policy_mode_and_override`
- `checkpoint_and_recovery`

