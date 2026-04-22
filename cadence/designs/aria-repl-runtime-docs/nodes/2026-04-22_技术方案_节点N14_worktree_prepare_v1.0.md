# 节点文档：N14 worktree_prepare

## 1. 节点标识

- 节点 ID：`N14`
- 节点类型：Aria 内部
- 主执行者：Aria daemon
- 所属链路：执行环境准备
- 版本：v1.0

## 2. 节点目的

为即将执行的 WorkTask 准备独立 worktree 和分支映射。

## 3. 进入条件

- 至少一个 WorkTask 已注册

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| WorkTask refs | `N13` | 是 | task ids | 回退 `N13` |
| repo root | session | 是 | repo path | 回退 `N00` |

## 5. Aria 驱动动作

1. 为可执行 WorkTask 申请 `WorktreeLease`。
2. 创建独立 worktree。
3. 绑定 branch、base ref、path。
4. 写 snapshot。

## 6. Provider 执行契约

无。该节点不调用外部 provider。

## 7. 输出产物

- `runtime_snapshot:N14.worktree_prepare`

## 8. 输出产物最小格式

snapshot 至少包含：

- `workTaskId`
- `worktreeId`
- `branchName`
- `worktreePath`
- `baseRef`

## 9. 完成判定

任务所需 worktree 均已准备完成并可进入执行节点。

## 10. 失败与回流

- worktree 创建失败：重试或进入人工介入

## 11. 交接到下一节点

把 `WorktreeLease` 与 WorkTask refs 交给 `N15 execution_route_resolve`。

## 12. 关联横切能力

- `worktree_lifecycle`
- `checkpoint_and_recovery`
- `manual_intervention`

