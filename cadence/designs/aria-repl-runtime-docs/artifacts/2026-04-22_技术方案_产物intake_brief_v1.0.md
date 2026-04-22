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

### L1 存在性校验
- `request_summary` 存在且非空
- `raw_user_request` 存在且非空
- `repo_context` 存在（允许为空对象 `{}`）
- `initial_constraints` 存在（允许为空数组 `[]`）
- `requested_goal` 存在且非空

### L2 结构性校验
- `request_summary` 为字符串类型，长度 >= 10 字符
- `raw_user_request` 为字符串类型
- `repo_context` 为对象类型（包含可选子字段：branch, language, framework）
- `initial_constraints` 为数组类型
- `requested_goal` 为字符串类型，长度 >= 5 字符

### L3 语义性校验（二期增强）
- `request_summary` 应包含动词和目标（如"实现"、"修复"、"优化"等）
- `requested_goal` 应可映射到具体交付物

## 9. 交接规则
供 `N02`、`N04`、`N05` 消费。

## 10. 版本与修订规则
大补充升小版本；换主题时新建产物。

## 11. 失败与缺失处理
缺字段则回退 `N01`。
