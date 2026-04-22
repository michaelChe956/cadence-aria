# 产物规范：clarification_record

## 1. 产物标识
- 类型 ID：`ART-CLARIFICATION-RECORD`
- 类别：文档产物
- 默认节点：`N04`

## 2. 产物目的
记录澄清后的目标、约束、假设和待确认项，为 spec 编写定边界。

## 3. 产出时机
每次澄清轮次结束后产出，可多版本。

## 4. 产物存储位置
默认写入 `cadence/prds/`。

## 5. 最小字段清单
| 字段 | 必填 |
|------|------|
| `goal_summary` | 是 |
| `constraints` | 是 |
| `assumptions` | 是 |
| `open_questions` | 是 |
| `suggested_scope` | 是 |

## 6. 推荐结构
目标总结、约束、假设、待确认项、建议边界。

## 7. 固定格式示例
```md
# Clarification Record
- goal_summary:
- constraints:
- assumptions:
- open_questions:
- suggested_scope:
```

## 8. 校验规则
必须显式列出 `open_questions`，即使为空也要写。

## 9. 交接规则
供 `N05` 和 `N06` 消费。

## 10. 版本与修订规则
每轮澄清生成新版本。

## 11. 失败与缺失处理
缺关键字段则回流 `N04`。

