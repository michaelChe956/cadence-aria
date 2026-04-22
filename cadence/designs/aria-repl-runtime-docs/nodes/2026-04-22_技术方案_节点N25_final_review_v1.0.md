# 节点文档：N25 final_review

## 1. 节点标识

- 节点 ID：`N25`
- 节点类型：Agent 业务
- 主执行者：Claude Code
- 所属链路：最终评审
- 版本：v1.0

## 2. 节点目的

从全局视角确认所有 WorkTask 集成结果是否满足 spec、design 和计划目标。

## 3. 进入条件

- 全部待集成 WorkTask 已完成集成验证

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| all integration reports | `N23`/`N24` | 是 | refs | 回退集成链 |
| `spec` | `N05` | 是 | ref | 回退 `N05` |
| `design` | `N07`/`N09` | 是 | ref | 回退 `N07` |

## 5. Aria 驱动动作

1. 聚合全部任务结果。
2. 组装 final review package。
3. 调用 Claude Code 做最终评审。
4. 根据结果决定进入补丁分发或总结。

## 6. Provider 执行契约

- 角色：`orchestrator`
- 默认 provider：Claude Code
- 必做动作：
  - 判断系统级完成度
  - 标出剩余缺口
  - 决定是否派生补丁任务

## 7. 输出产物

- `final_review`
- `runtime_snapshot:N25.final_review`

## 8. 输出产物最小格式

`final_review` 至少包含：

- `overallDecision`
- `completedScope`
- `remainingGaps`
- `followupNeeded`

## 9. 完成判定

已明确进入 `N26` 还是 `N27`。

## 10. 失败与回流

- 需补丁：进入 `N26 patch_followup_dispatch`
- provider 失败：retry/gate

## 11. 交接到下一节点

- 需要补丁：进入 `N26`
- 无补丁：进入 `N27`

## 12. 关联横切能力

- `provider_run_lifecycle`
- `artifact_validate`
- `approval_gate`

