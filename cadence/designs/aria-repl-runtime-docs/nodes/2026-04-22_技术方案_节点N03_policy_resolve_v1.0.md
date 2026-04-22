# 节点文档：N03 policy_resolve

## 1. 节点标识

- 节点 ID：`N03`
- 节点类型：Aria 内部
- 主执行者：Aria daemon
- 所属链路：策略解析
- 版本：v1.0

## 2. 节点目的

在主任务正式进入 agent 阶段前，计算当前 session 的有效策略，决定默认自动化强度与 gate 规则。

## 3. 进入条件

- `EpicTask` 已创建

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `EpicTask` ref | `N02` | 是 | taskId | 回退 `N02` |
| session policy | session config | 是 | global mode | 缺失则用 conservative |
| phase overrides | session config | 否 | override map | 缺失则空 |

## 5. Aria 驱动动作

1. 读取 session 默认策略。
2. 读取当前阶段覆写。
3. 生成 `effectivePolicy`。
4. 写入 snapshot 与交接包。

## 6. Provider 执行契约

无。该节点不调用外部 provider。

## 7. 输出产物

- `runtime_snapshot:N03.policy_resolve`

## 8. 输出产物最小格式

snapshot 至少包含：

- `taskId`
- `effectivePolicy`
- `phaseOverrideApplied`
- `autoAdvanceAllowed`

## 9. 完成判定

有效策略已写入交接包，可供 `N04 clarification` 消费。

## 10. 失败与回流

- 策略解析失败：自动回退为 `conservative`

## 11. 交接到下一节点

将 `effectivePolicy`、`epicTaskId`、`intake_brief` 交给 `N04 clarification`。

## 12. 关联横切能力

- `policy_mode_and_override`
- `checkpoint_and_recovery`

