# 节点文档：N18 code_review

## 1. 节点标识

- 节点 ID：`N18`
- 节点类型：Agent 业务
- 主执行者：Codex
- 所属链路：代码评审
- 版本：v1.0

## 2. 节点目的

对 WorkTask 当前实现进行代码级评审，判断是否可以进入集成队列。

## 3. 进入条件

- testing 已通过

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `coding_report` | `N16`/`N19` | 是 | ref | 回退上游 |
| `testing_report` | `N17` | 是 | ref | 回退 `N17` |
| worktree ref | `N14` | 是 | path | 回退 `N14` |

## 5. Aria 驱动动作

1. 组装 code review package。
2. 调用 Codex 评审当前改动。
3. 根据 review 结论决定进入 `N20` 或 `N19`。

## 6. Provider 执行契约

- 角色：`reviewer`
- 默认 provider：Codex
- 必做动作：
  - 找功能回归风险
  - 找实现遗漏
  - 给出 allow / revise 结论

## 7. 输出产物

- `code_review_report`
- `runtime_snapshot:N18.code_review`

## 8. 输出产物最小格式

`code_review_report` 至少包含：

- `review_decision`
- `findings`
- `required_changes`
- `allow_integration`

## 9. 完成判定

review 结果已明确且可决定是否进入集成队列。

## 10. 失败与回流

- review 不通过：进入 `N19 rework`
- provider 失败：retry/gate

## 11. 交接到下一节点

- 通过：进入 `N20 ready_for_integration`
- 不通过：进入 `N19 rework`

## 12. 关联横切能力

- `provider_run_lifecycle`
- `artifact_validate`
- `approval_gate`

