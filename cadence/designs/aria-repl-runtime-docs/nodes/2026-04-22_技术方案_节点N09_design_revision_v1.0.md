# 节点文档：N09 design_revision

## 1. 节点标识

- 节点 ID：`N09`
- 节点类型：Agent 业务
- 主执行者：Claude Code
- 所属链路：设计修订
- 版本：v1.0

## 2. 节点目的

根据 `design_review` 的强制修改项修订 design，并生成修订记录，之后重新回到 design review。

## 3. 进入条件

- `design_review.review_decision = revise`

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `design` | `N07` | 是 | design ref | 回退 `N07` |
| `design_review` | `N08` | 是 | review ref | 回退 `N08` |

## 5. Aria 驱动动作

1. 组装 revision package。
2. 调用 Claude Code 修订。
3. 写修订记录。
4. 重新校验 design 可评审性。

## 6. Provider 执行契约

- 角色：`orchestrator`
- 默认 provider：Claude Code
- 必做动作：
  - 响应全部 required changes
  - 更新设计正文
  - 记录修订原因

## 7. 输出产物

- `design_revision_record`
- 更新后的 `design`

## 8. 输出产物最小格式

`design_revision_record` 至少包含：

- `revision_summary`
- `resolved_findings`
- `remaining_risks`
- `updated_design_ref`

## 9. 完成判定

修订记录与更新后的 design 都已生成，且可再次进入 `N08`。

## 10. 失败与回流

- 修订失败：retry 或 gate
- 设计依旧不完整：停留本节点重做

## 11. 交接到下一节点

将更新后的 `design` 和 `design_revision_record` 交回 `N08 design_review`。

## 12. 关联横切能力

- `provider_run_lifecycle`
- `artifact_validate`
- `approval_gate`
- `checkpoint_and_recovery`

