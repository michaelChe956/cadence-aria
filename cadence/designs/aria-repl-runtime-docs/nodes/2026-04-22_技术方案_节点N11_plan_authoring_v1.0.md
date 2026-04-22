# 节点文档：N11 plan_authoring

## 1. 节点标识

- 节点 ID：`N11`
- 节点类型：Agent 业务
- 主执行者：Claude Code
- 所属链路：计划生成
- 版本：v1.0

## 2. 节点目的

把已确认的 spec 与 design 转换成可执行 plan，作为 dispatch 的上游输入。

## 3. 进入条件

- `readiness_check.is_ready = true`

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `spec` | `N05` | 是 | ref | 回退 `N05` |
| `design` | `N07`/`N09` | 是 | ref | 回退 `N07` |
| `readiness_check` | `N10` | 是 | ref | 回退 `N10` |

## 5. Aria 驱动动作

1. 组装 plan package。
2. 调用 Claude Code 生成 plan。
3. 校验 plan 是否可拆分、可验收。

## 6. Provider 执行契约

- 角色：`orchestrator`
- 默认 provider：Claude Code
- 必做动作：
  - 拆分实施步骤
  - 标出依赖关系
  - 标出验收标准

## 7. 输出产物

- `plan`
- `runtime_snapshot:N11.plan_authoring`

## 8. 输出产物最小格式

`plan` 至少包含：

- `work_packages`
- `dependencies`
- `acceptance_checks`
- `parallelism_hints`

## 9. 完成判定

plan 已通过结构校验并可进入 dispatch。

## 10. 失败与回流

- 计划不可拆：回流 `N07`
- 缺失验收标准：停留本节点重做

## 11. 交接到下一节点

将 `plan` 交给 `N12 dispatch_authoring`。

## 12. 关联横切能力

- `provider_run_lifecycle`
- `artifact_validate`

