# 节点文档：N04 clarification

## 1. 节点标识

- 节点 ID：`N04`
- 节点类型：Agent 业务
- 主执行者：Claude Code
- 所属链路：需求澄清
- 版本：v1.0

## 2. 节点目的

将用户原始需求转成可界定边界的问题集合、约束集合和待确认项，为 spec 编写做准备。

## 3. 进入条件

- `intake_brief` 已存在
- `effectivePolicy` 已确定

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `intake_brief` | `N01` | 是 | brief ref | 回退 `N01` |
| `effectivePolicy` | `N03` | 是 | policy ref | 回退 `N03` |
| prior clarification record | 当前 task 历史 | 否 | latest ref | 无则首次执行 |

## 5. Aria 驱动动作

1. 组装 clarification context package。
2. 选择 orchestrator 角色。
3. 派发 Claude Code。
4. 收集澄清记录。
5. 根据结果决定直接进入 `N05` 或先经 `N06`。

## 6. Provider 执行契约

- 角色：`orchestrator`
- 默认 provider：Claude Code
- 调用模式：`spawn + CLI`
- 必做动作：
  - 提取业务目标
  - 列出约束、假设、风险
  - 明确待确认项
- 禁止动作：
  - 跳过不确定项直接编造边界

## 7. 输出产物

- `clarification_record`
- `runtime_snapshot:N04.clarification`

## 8. 输出产物最小格式

`clarification_record` 至少包含：

- `goal_summary`
- `constraints`
- `assumptions`
- `open_questions`
- `suggested_scope`

## 9. 完成判定

澄清记录通过校验，且能为 spec 提供足够边界信息时完成。

## 10. 失败与回流

- provider 失败：走 `provider_run_lifecycle` 的 retry/gate
- open questions 阻塞：进入 `X01 approval_gate_open`
- 产物不合格：回流当前节点重做

## 11. 交接到下一节点

- 若澄清充分：交给 `N05 spec_authoring`
- 若仍有关键未决项：交给 `N06 spec_gate_review`

## 12. 关联横切能力

- `provider_run_lifecycle`
- `artifact_validate`
- `approval_gate`
- `checkpoint_and_recovery`

