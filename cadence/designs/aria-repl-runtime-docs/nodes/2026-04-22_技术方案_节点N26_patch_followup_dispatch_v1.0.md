# 节点文档：N26 patch_followup_dispatch

## 1. 节点标识

- 节点 ID：`N26`
- 节点类型：混合
- 主执行者：Claude Code + Aria daemon
- 所属链路：补丁任务派生
- 版本：v1.0

## 2. 节点目的

在最终评审发现系统级缺口时，派生新的补丁 WorkTask，并重新回到执行链路。

## 3. 进入条件

- `final_review.followupNeeded = true`

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `final_review` | `N25` | 是 | ref | 回退 `N25` |
| all relevant reports | runtime | 是 | refs | 回退收集阶段 |

## 5. Aria 驱动动作

1. 组装 patch dispatch package。
2. 调用 Claude Code 输出补丁任务。
3. 注册新的 WorkTask。

## 6. Provider 执行契约

- 角色：`orchestrator`
- 默认 provider：Claude Code
- 必做动作：
  - 只针对最终评审缺口派生补丁任务
  - 输出明确的补丁边界与验收标准

## 7. 输出产物

- 更新后的 `dispatch_package`
- `runtime_snapshot:N26.patch_followup_dispatch`

## 8. 输出产物最小格式

补丁 dispatch 至少包含：

- `followupWorktasks`
- `originFinalReviewRef`
- `acceptanceTargets`

## 9. 完成判定

补丁任务已注册为新的 WorkTask 输入。

## 10. 失败与回流

- 派生边界不清：人工介入

## 11. 交接到下一节点

重新进入 `N13 worktask_register`。

## 12. 关联横切能力

- `provider_run_lifecycle`
- `manual_intervention`

