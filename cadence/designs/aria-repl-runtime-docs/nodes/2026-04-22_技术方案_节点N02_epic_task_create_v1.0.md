# 节点文档：N02 epic_task_create

## 1. 节点标识

- 节点 ID：`N02`
- 节点类型：Aria 内部
- 主执行者：Aria daemon
- 所属链路：任务注册
- 版本：v1.0

## 2. 节点目的

把入口 brief 转换成系统内部可调度的 `EpicTask`，作为后续所有阶段的主任务容器。

## 3. 进入条件

- `intake_brief` 已存在
- session 可用

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `intake_brief` | `N01` | 是 | brief ref | 回退 `N01` |
| sessionId | `N00` | 是 | session ref | 回退 `N00` |

## 5. Aria 驱动动作

1. 生成 `EpicTask` ID。
2. 绑定 session 与 repo。
3. 初始化 phase 为 `intake`。
4. 写 task record 和 snapshot。

## 6. Provider 执行契约

无。该节点不调用外部 provider。

## 7. 输出产物

- `runtime_snapshot:N02.epic_task_create`
- `EpicTask` 引用

## 8. 输出产物最小格式

snapshot 至少包含：

- `epicTaskId`
- `sessionId`
- `initialPhase`
- `sourceIntakeBriefRef`

## 9. 完成判定

`EpicTask` 已注册且可由后续节点引用。

## 10. 失败与回流

- task record 写入失败：终止并保持 intake 状态

## 11. 交接到下一节点

把 `epicTaskId`、`intake_brief`、session refs 交给 `N03 policy_resolve`。

## 12. 关联横切能力

- `checkpoint_and_recovery`

