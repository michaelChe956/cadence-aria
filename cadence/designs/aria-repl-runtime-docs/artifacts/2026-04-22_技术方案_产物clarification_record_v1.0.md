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

### L1 存在性校验
- `goal_summary` 存在且非空
- `constraints` 存在（允许为空数组 `[]`）
- `assumptions` 存在（允许为空数组 `[]`）
- `open_questions` 存在（即使为空也必须写 `[]`）
- `suggested_scope` 存在且非空

### L2 结构性校验
- `goal_summary` 为字符串类型，长度 >= 20 字符
- `constraints` 为数组类型，每个元素为对象（含 `description` 字段）
- `assumptions` 为数组类型，每个元素为字符串
- `open_questions` 为数组类型，每个元素为字符串；空值必须写 `[]`，不允许 `null`
- `suggested_scope` 为字符串类型

### L3 语义性校验（二期增强）
- `goal_summary` 应覆盖用户原始需求的核心意图
- `open_questions` 中每个问题应可由用户直接回答

## 9. 交接规则
供 `N05` 和 `N06` 消费。

## 10. 版本与修订规则
每轮澄清生成新版本。

## 11. 失败与缺失处理
缺关键字段则回流 `N04`。

