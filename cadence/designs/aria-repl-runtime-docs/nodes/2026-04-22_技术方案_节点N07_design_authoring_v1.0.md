# 节点文档：N07 design_authoring

## 1. 节点标识

- 节点 ID：`N07`
- 节点类型：Agent 业务
- 主执行者：Claude Code
- 所属链路：方案设计
- 版本：v1.0

## 2. 节点目的

把 spec 转成可评审、可落地的技术设计，覆盖前端、后端、数据模型、API、JSON Schema、公共组件、风险点和技术点。

## 3. 进入条件

- `spec_gate_decision.decision = pass`

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `spec` | `N05` | 是 | spec ref | 回退 `N05` |
| `spec_gate_decision` | `N06` | 是 | decision ref | 回退 `N06` |

## 5. Aria 驱动动作

1. 组装 design package。
2. 将 spec 与 repo 事实传给 Claude Code。
3. 产出 design 文档并落盘。
4. 校验 design 最小结构。

## 6. Provider 执行契约

- 角色：`orchestrator`
- 默认 provider：Claude Code
- 必做动作：
  - 数据模型
  - API 规范
  - JSON Schema
  - 前后端落点
  - 风险点

## 7. 输出产物

- `design`
- `runtime_snapshot:N07.design_authoring`

## 8. 输出产物最小格式

`design` 至少包含：

- `architecture_summary`
- `frontend_design`
- `backend_design`
- `data_model`
- `api_contracts`
- `risk_list`

## 9. 完成判定

design 通过最小结构校验且可交给评审节点。

## 10. 失败与回流

- provider 失败：retry 或 gate
- design 不完整：回流 `N07`

## 11. 交接到下一节点

将 `design`、`spec` 交给 `N08 design_review`。

## 12. 关联横切能力

- `provider_run_lifecycle`
- `artifact_validate`
- `checkpoint_and_recovery`

