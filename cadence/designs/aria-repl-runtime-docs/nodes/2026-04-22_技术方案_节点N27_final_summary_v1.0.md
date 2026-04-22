# 节点文档：N27 final_summary

## 1. 节点标识

- 节点 ID：`N27`
- 节点类型：Agent 业务
- 主执行者：Claude Code
- 所属链路：最终总结
- 版本：v1.0

## 2. 节点目的

生成用户可读的最终总结，说明完成内容、未做内容、验证结果和风险备注。

## 3. 进入条件

- `final_review.overallDecision = pass`
  或
- 补丁链路全部完成

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `final_review` | `N25` | 是 | ref | 回退 `N25` |
| all key artifacts | runtime | 是 | refs | 回退汇总阶段 |

## 5. Aria 驱动动作

1. 聚合全部关键产物。
2. 调用 Claude Code 生成总结。
3. 落盘总结文档。

## 6. Provider 执行契约

- 角色：`orchestrator`
- 默认 provider：Claude Code
- 必做动作：
  - 概述完成范围
  - 概述验证结果
  - 说明遗留事项

## 7. 输出产物

- `final_summary`
- `runtime_snapshot:N27.final_summary`

## 8. 输出产物最小格式

`final_summary` 至少包含：

- `completed_items`
- `verification_summary`
- `remaining_risks`
- `next_steps`

## 9. 完成判定

总结文档通过校验并可作为 session 收口输入。

## 10. 失败与回流

- 总结不完整：停留本节点重做

## 11. 交接到下一节点

进入 `N28 session_closeout`。

## 12. 关联横切能力

- `provider_run_lifecycle`
- `artifact_validate`

