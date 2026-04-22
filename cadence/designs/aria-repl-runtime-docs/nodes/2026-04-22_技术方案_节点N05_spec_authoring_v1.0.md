# 节点文档：N05 spec_authoring

## 1. 节点标识

- 节点 ID：`N05`
- 节点类型：Agent 业务
- 主执行者：Claude Code
- 所属链路：规格编写
- 版本：v1.0

## 2. 节点目的

将澄清结果转为可审阅、可作为设计输入的规格文档。

## 3. 进入条件

- `clarification_record` 已存在
- 关键待确认项不再阻塞

## 4. 输入契约

| 输入 | 来源 | 必填 | 最小字段 | 缺失处理 |
|------|------|------|----------|----------|
| `clarification_record` | `N04` | 是 | record ref | 回退 `N04` |
| `intake_brief` | `N01` | 是 | brief ref | 回退 `N01` |

## 5. Aria 驱动动作

1. 组装 spec authoring package。
2. 选择 Claude Code 生成 spec。
3. 将结果写入受管文档路径。
4. 校验 spec 最小结构。

## 6. Provider 执行契约

- 角色：`orchestrator`
- 默认 provider：Claude Code
- 必做动作：
  - 写用户故事
  - 写功能需求
  - 写成功标准
  - 写待确认项

## 7. 输出产物

- `spec`
- `runtime_snapshot:N05.spec_authoring`

## 8. 输出产物最小格式

`spec` 至少包含：

- `scope`
- `user_stories`
- `functional_requirements`
- `success_criteria`
- `open_items`

## 9. 完成判定

spec 通过产物校验且可交给 `N06` 审核。

## 10. 失败与回流

- spec 不完整：回流 `N05`
- 关键待确认项阻塞：交由 `N06` 决定是否回退 `N04`

## 11. 交接到下一节点

将 `spec`、`clarification_record` 交给 `N06 spec_gate_review`。

## 12. 关联横切能力

- `provider_run_lifecycle`
- `artifact_validate`
- `checkpoint_and_recovery`

