# 产物规范：design_review

## 1. 产物标识
- 类型 ID：`ART-DESIGN-REVIEW`
- 类别：文档产物
- 默认节点：`N08`

## 2. 产物目的
承载设计评审结论，决定设计是否通过或必须修订。

## 3. 产出时机
每次 design review 后产出。

## 4. 产物存储位置
默认写入 `cadence/designs-reviews/`。

## 5. 最小字段清单
| 字段 | 必填 |
|------|------|
| `review_decision` | 是 |
| `findings` | 是 |
| `required_changes` | 是 |
| `allow_next_step` | 是 |

## 6. 推荐结构
结论、发现、修改项、是否放行。

## 7. 固定格式示例
```json
{"review_decision":"pass|revise|conditional_pass","findings":[],"required_changes":[],"allow_next_step":false}
```

## 8. 校验规则

### L1 存在性校验
- `review_decision` 存在且为 `pass` / `revise` / `conditional_pass` 之一
- `findings` 存在（允许为空数组 `[]`）
- `required_changes` 存在（允许为空数组 `[]`）
- `allow_next_step` 存在且为布尔值

### L2 结构性校验
- `review_decision` 为字符串，取值范围：`pass`, `revise`, `conditional_pass`
- `findings` 为数组，每个元素为对象（含 `severity`、`description` 字段）
- `required_changes` 为数组，每个元素为对象（含 `description` 字段）
- `allow_next_step` 为布尔类型

### L3 语义性校验（二期增强）
- `review_decision` 为 `revise` 时，`required_changes` 不应为空
- `review_decision` 为 `pass` 时，`allow_next_step` 应为 `true`
- `findings` 中 `severity` 取值应为 `high` / `medium` / `low`

## 9. 交接规则
供 `N09` 或 `N10` 消费。

## 10. 版本与修订规则
每轮评审独立版本。

## 11. 失败与缺失处理
缺结论则回流 `N08`。

