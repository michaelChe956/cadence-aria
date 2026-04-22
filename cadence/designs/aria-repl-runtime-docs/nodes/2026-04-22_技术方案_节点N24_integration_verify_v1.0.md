# 节点文档：N24 integration_verify

## 1. 节点标识

- 节点 ID：`N24`
- 节点类型：混合
- 主执行者：Aria daemon + Codex/本地测试
- 所属链路：集成验证
- 版本：v1.0

## 2. 节点目的

验证刚完成的集成是否保持主线稳定，并决定是否进入最终评审。

## 3. 进入条件

- `integration_execute` 成功

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `integration_report` | `N23` | 是 | ref | 回退 `N23` |
| verification target set | runtime config | 是 | checks | 中止验证 |

## 5. Aria 驱动动作

1. 执行集成后必要回归。
2. 需要时调用 Codex 分析失败。
3. 更新 integration report。

## 6. Provider 执行契约

- 可选角色：`reviewer`
- 默认 provider：Codex，仅在验证失败分析时调用

## 7. 输出产物

- 更新后的 `integration_report`
- `runtime_snapshot:N24.integration_verify`

## 8. 输出产物最小格式

report 至少新增：

- `verificationStatus`
- `postIntegrationFailures`
- `rollbackNeeded`

## 9. 完成判定

已明确主线稳定或需要回流返工。

## 10. 失败与回流

- 回归失败：回流 `N19 rework`
- 无法判定：人工介入

## 11. 交接到下一节点

- 通过：进入 `N25 final_review`
- 失败：回流 `N19`

## 12. 关联横切能力

- `integration_queue`
- `provider_run_lifecycle`
- `manual_intervention`

