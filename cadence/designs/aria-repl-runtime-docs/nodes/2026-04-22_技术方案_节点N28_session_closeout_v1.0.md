# 节点文档：N28 session_closeout

## 1. 节点标识

- 节点 ID：`N28`
- 节点类型：Aria 内部
- 主执行者：Aria daemon
- 所属链路：会话收口
- 版本：v1.0

## 2. 节点目的

把当前 session 置为已完成状态，整理最终产物索引，释放可回收资源。

## 3. 进入条件

- `final_summary` 已生成

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `final_summary` | `N27` | 是 | ref | 回退 `N27` |
| session refs | runtime | 是 | session id | 回退 `N00` |

## 5. Aria 驱动动作

1. 更新 session 状态为 `completed`。
2. 生成最终索引快照。
3. 释放可回收 queue 与 worktree 资源。
4. 写最终 checkpoint。

## 6. Provider 执行契约

无。

## 7. 输出产物

- `runtime_snapshot:N28.session_closeout`

## 8. 输出产物最小格式

snapshot 至少包含：

- `sessionId`
- `completedAt`
- `finalArtifactRefs`
- `releasedResources`

## 9. 完成判定

session 已不再有待执行节点，且最终索引可被重连读取。

## 10. 失败与回流

- closeout 写入失败：保留 session 在 `completed_pending_closeout`

## 11. 交接到下一节点

无，主流程结束。

## 12. 关联横切能力

- `checkpoint_and_recovery`
- `worktree_lifecycle`

