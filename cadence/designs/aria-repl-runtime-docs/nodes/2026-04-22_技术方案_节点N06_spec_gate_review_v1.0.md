# 节点文档：N06 spec_gate_review

## 1. 节点标识

- 节点 ID：`N06`
- 节点类型：Aria 内部
- 主执行者：Aria daemon
- 所属链路：规格闸门
- 版本：v1.0

## 2. 节点目的

判断 spec 是否可以进入设计阶段，或是否必须回到澄清阶段补足缺口。

## 3. 进入条件

- `spec` 已生成

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `spec` | `N05` | 是 | spec ref | 回退 `N05` |
| `clarification_record` | `N04` | 是 | record ref | 回退 `N04` |

## 5. Aria 驱动动作

1. 校验 spec 完整性。
2. 判断 `open_items` 是否阻塞设计。
3. 生成 gate decision。
4. 必要时挂人工闸门。

## 6. Provider 执行契约

无。该节点主要由 Aria 内部规则执行。

## 7. 输出产物

- `spec_gate_decision`
- `runtime_snapshot:N06.spec_gate_review`

## 8. 输出产物最小格式

`spec_gate_decision` 至少包含：

- `decision`
- `blocking_items`
- `next_node`
- `required_user_action`

## 9. 完成判定

已有明确 decision，且下游节点已确定。

## 10. 失败与回流

- 阻塞项存在：回流 `N04 clarification`
- 需要用户确认：进入 `X01 approval_gate_open`

## 11. 交接到下一节点

- 通过：交给 `N07 design_authoring`
- 未通过：回到 `N04 clarification`

## 12. 关联横切能力

- `approval_gate`
- `artifact_validate`
- `checkpoint_and_recovery`

