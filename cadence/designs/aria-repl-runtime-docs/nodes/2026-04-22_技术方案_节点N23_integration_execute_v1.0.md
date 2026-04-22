# 节点文档：N23 integration_execute

## 1. 节点标识

- 节点 ID：`N23`
- 节点类型：混合
- 主执行者：Aria daemon + git/test toolchain
- 所属链路：执行集成
- 版本：v1.0

## 2. 节点目的

把当前 WorkTask 的变更串行合入主线或集成分支，并生成集成执行结果。

## 3. 进入条件

- 集成准备通过

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| prepare snapshot | `N22` | 是 | ref | 回退 `N22` |
| worktree ref | `N14` | 是 | ref | 回退 `N14` |
| target branch | runtime config | 是 | branch | 中止集成 |

## 5. Aria 驱动动作

1. 在受控模式下执行合入。
2. 记录 merge/rebase 结果。
3. 写 integration report 初版。

## 6. Provider 执行契约

本节点默认不依赖 Claude/Codex，主要依赖 git/test toolchain。

## 7. 输出产物

- `integration_report`
- `runtime_snapshot:N23.integration_execute`

## 8. 输出产物最小格式

`integration_report` 至少包含：

- `attemptId`
- `integrationMode`
- `gitResult`
- `conflictFiles`
- `nextDecision`

## 9. 完成判定

已明确合入成功或冲突失败，且结果已固化。

## 10. 失败与回流

- 合入冲突：回流 `N19 rework`
- 工具链异常：gate 或人工介入

## 11. 交接到下一节点

- 成功：进入 `N24 integration_verify`
- 失败：回流 `N19`

## 12. 关联横切能力

- `integration_queue`
- `worktree_lifecycle`
- `approval_gate`
- `manual_intervention`

