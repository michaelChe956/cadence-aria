# 节点文档：N12 dispatch_authoring

## 1. 节点标识

- 节点 ID：`N12`
- 节点类型：Agent 业务
- 主执行者：Claude Code
- 所属链路：任务分发包生成
- 版本：v1.0

## 2. 节点目的

把 plan 转换成 Aria 可注册的 WorkTask 列表与 dispatch package。

## 3. 进入条件

- `plan` 已通过校验

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `plan` | `N11` | 是 | ref | 回退 `N11` |
| `design` | `N07`/`N09` | 是 | ref | 回退 `N07` |

## 5. Aria 驱动动作

1. 组装 dispatch package。
2. 调用 Claude Code 根据计划拆子任务。
3. 校验每个 WorkTask 是否具备输入、输出、依赖和验收标准。

## 6. Provider 执行契约

- 角色：`orchestrator`
- 默认 provider：Claude Code
- 必做动作：
  - 输出 WorkTask 列表
  - 标注依赖
  - 标注是否可并行
  - 标注建议 provider role

## 7. 输出产物

- `dispatch_package`
- `runtime_snapshot:N12.dispatch_authoring`

## 8. 输出产物最小格式

`dispatch_package` 至少包含：

- `worktasks`
- `dependencies`
- `parallel_groups`
- `acceptance_targets`

## 9. 完成判定

dispatch package 可被 `N13` 注册为内部任务。

## 10. 失败与回流

- 子任务缺交付标准：回流 `N11`
- 子任务边界冲突：回流 `N07`

## 11. 交接到下一节点

把 `dispatch_package` 交给 `N13 worktask_register`。

## 12. 关联横切能力

- `provider_run_lifecycle`
- `artifact_validate`

