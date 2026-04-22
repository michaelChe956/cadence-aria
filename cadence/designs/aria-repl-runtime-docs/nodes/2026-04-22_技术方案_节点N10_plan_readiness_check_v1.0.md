# 节点文档：N10 plan_readiness_check

## 1. 节点标识

- 节点 ID：`N10`
- 节点类型：Agent 业务
- 主执行者：Claude Code
- 所属链路：设计就绪检查
- 版本：v1.0

## 2. 节点目的

判断当前 spec 与 design 是否已经具备进入计划阶段的条件，并明确阻塞项是否需要回退。

## 3. 进入条件

- design review 已通过

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `spec` | `N05` | 是 | spec ref | 回退 `N05` |
| `design` | `N07`/`N09` | 是 | design ref | 回退 `N07` |
| `design_review` | `N08` | 是 | review ref | 回退 `N08` |

## 5. Aria 驱动动作

1. 组装 readiness package。
2. 调用 Claude Code 检查阻塞项。
3. 根据结果决定进入 `N11` 还是回到 `N07`。

## 6. Provider 执行契约

- 角色：`orchestrator`
- 默认 provider：Claude Code
- 必做动作：
  - 判断是否仍有阻塞 coding 的未决项
  - 标注 blocking / non-blocking 项

## 7. 输出产物

- `readiness_check`
- `runtime_snapshot:N10.plan_readiness_check`

## 8. 输出产物最小格式

`readiness_check` 至少包含：

- `is_ready`
- `blocking_items`
- `recommended_backtrack_node`
- `notes`

## 9. 完成判定

ready 结论已明确，且下游路由唯一。

## 10. 失败与回流

- 有阻塞项：回流 `N07 design_authoring`
- 需人工确认：走 `approval_gate`

## 11. 交接到下一节点

- ready：进入 `N11 plan_authoring`
- not ready：回到 `N07`

## 12. 关联横切能力

- `provider_run_lifecycle`
- `artifact_validate`
- `approval_gate`

