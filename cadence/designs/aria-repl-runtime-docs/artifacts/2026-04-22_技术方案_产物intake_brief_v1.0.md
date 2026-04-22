# 产物规范：intake_brief

## 1. 产物标识
- 类型 ID：`ART-INTAKE-BRIEF`
- 类别：文档产物
- 默认节点：`N01`

## 2. 产物目的
承载用户原始需求、初始上下文与入口约束，作为 EpicTask 创建和澄清阶段的起点。

## 3. 产出时机
首次 capture 必产出；用户补充大范围新信息时生成新版本。

## 4. 产物存储位置
默认写入 `cadence/prds/`，文件名建议：`2026-04-22_概要需求_Aria任务入口_epic_login_flow_001_v1.0.md`；文件尾部标识由 Aria 自动使用 `epicTaskId` 或 `sessionId` 填充，不由人工自由命名。

## 5. 最小字段清单
| 字段 | 必填 |
|------|------|
| `request_summary` | 是 |
| `raw_user_request` | 是 |
| `repo_context` | 是 |
| `initial_constraints` | 是 |
| `requested_goal` | 是 |

## 6. 推荐结构
背景、原始诉求、目标、约束、上下文。

## 7. 固定格式示例
```md
# Intake Brief
- request_summary:
- raw_user_request:
- repo_context:
- initial_constraints:
- requested_goal:
```

## 8. 校验规则
5 个必填字段必须存在，`raw_user_request` 不能为空。

## 9. 交接规则
供 `N02`、`N04`、`N05` 消费。

## 10. 版本与修订规则
大补充升小版本；换主题时新建产物。

## 11. 失败与缺失处理
缺字段则回退 `N01`。
