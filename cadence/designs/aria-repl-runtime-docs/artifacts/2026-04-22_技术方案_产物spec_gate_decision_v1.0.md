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

### L1 存在性校验
- `decision` 存在且为 `pass` / `backtrack` / `hold` 之一
- `blocking_items` 存在（允许为空数组 `[]`）
- `next_node` 存在且非空

### L2 结构性校验
- `decision` 为字符串，取值范围：`pass`, `backtrack`, `hold`
- `blocking_items` 为数组，每个元素为对象（含 `description` 字段）
- `next_node` 为字符串，必须是合法节点 ID（Nxx 或 Xxx）
- `required_user_action`（若存在）为字符串类型

### L3 语义性校验（二期增强）
- `decision` 为 `backtrack` 时，`blocking_items` 不应为空
- `decision` 为 `hold` 时，`required_user_action` 应存在且非空
- `next_node` 应与 `decision` 一致（pass → N07, backtrack → N04, hold → X01）

## 9. 交接规则
供 `N07` 或回退 `N04` 消费。

## 10. 版本与修订规则
每次 gate review 产出新记录。

## 11. 失败与缺失处理
字段缺失则停留 `N06`。

