# 节点文档：N13 worktask_register

## 1. 节点标识

- 节点 ID：`N13`
- 节点类型：Aria 内部
- 主执行者：Aria daemon
- 所属链路：WorkTask 注册
- 版本：v1.0

## 2. 节点目的

把 dispatch package 注册成系统内部多个 `WorkTask`，建立依赖图和调度基础。

## 3. 进入条件

- `dispatch_package` 已存在

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `dispatch_package` | `N12` | 是 | ref | 回退 `N12` |
| `epicTaskId` | `N02` | 是 | task ref | 回退 `N02` |

## 5. Aria 驱动动作

1. 为每个 WorkTask 分配 ID。
2. 记录依赖与并行组。
3. 初始化任务状态为 `pending`。
4. 写 snapshot。

## 6. Provider 执行契约

无。该节点由 Aria 内部完成。

## 7. 输出产物

- `runtime_snapshot:N13.worktask_register`

## 8. 输出产物最小格式

snapshot 至少包含：

- `epicTaskId`
- `registeredWorktasks`
- `dependencyGraph`
- `parallelGroups`

## 9. 完成判定

全部 WorkTask 已注册并可调度。

## 10. 失败与回流

- 依赖图非法：回流 `N12`

## 11. 交接到下一节点

将 WorkTask refs 交给 `N14 worktree_prepare`。

## 12. 关联横切能力

- `checkpoint_and_recovery`

