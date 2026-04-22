# 产物规范：spec_gate_decision

## 1. 产物标识
- 类型 ID：`ART-SPEC-GATE-DECISION`
- 类别：校验记录
- 默认节点：`N06`

## 2. 产物目的
明确 spec 是否允许进入设计阶段，以及阻塞项与回流目标。

## 3. 产出时机
每次 spec 审核后产出。

## 4. 产物存储位置
默认写入 `cadence/reports/`。

## 5. 最小字段清单
| 字段 | 必填 |
|------|------|
| `decision` | 是 |
| `blocking_items` | 是 |
| `next_node` | 是 |
| `required_user_action` | 否 |

## 6. 推荐结构
决策、阻塞项、下一节点、用户动作。

## 7. 固定格式示例
```json
{"decision":"pass|backtrack|gate","blocking_items":[],"next_node":"N07"}
```

## 8. 校验规则
`decision` 必须属于允许枚举。

## 9. 交接规则
供 `N07` 或回退 `N04` 消费。

## 10. 版本与修订规则
每次 gate review 产出新记录。

## 11. 失败与缺失处理
字段缺失则停留 `N06`。

