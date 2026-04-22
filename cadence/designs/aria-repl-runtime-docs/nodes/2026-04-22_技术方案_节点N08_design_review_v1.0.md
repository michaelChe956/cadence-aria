# 节点文档：N08 design_review

## 1. 节点标识

- 节点 ID：`N08`
- 节点类型：Agent 业务
- 主执行者：Codex
- 所属链路：设计评审
- 版本：v1.0

## 2. 节点目的

从实现可行性、风险、缺失项和一致性角度评审 design，判断是否允许进入下游节点。

## 3. 进入条件

- `design` 已生成

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `design` | `N07` | 是 | design ref | 回退 `N07` |
| `spec` | `N05` | 是 | spec ref | 回退 `N05` |

## 5. Aria 驱动动作

1. 组装 design review package。
2. 调用 Codex 作为 reviewer。
3. 收集 review 结果。
4. 根据 review 结论路由到 `N09` 或 `N10`。

## 6. Provider 执行契约

- 角色：`reviewer`
- 默认 provider：Codex
- 必做动作：
  - 找缺失
  - 找不一致
  - 找高风险点
  - 给出修改建议

## 7. 输出产物

- `design_review`
- `runtime_snapshot:N08.design_review`

## 8. 输出产物最小格式

`design_review` 至少包含：

- `review_decision`
- `findings`
- `required_changes`
- `allow_next_step`

## 9. 完成判定

review 结果已明确允许通过或要求修订。

## 10. 失败与回流

- review 失败：retry 或 gate
- 需要修订：回流 `N09 design_revision`

## 11. 交接到下一节点

- 通过：进入 `N10 plan_readiness_check`
- 不通过：进入 `N09 design_revision`

## 12. 关联横切能力

- `provider_run_lifecycle`
- `artifact_validate`
- `approval_gate`
- `checkpoint_and_recovery`

