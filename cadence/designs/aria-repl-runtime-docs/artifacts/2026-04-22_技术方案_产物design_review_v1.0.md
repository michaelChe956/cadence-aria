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
{"review_decision":"pass|revise","findings":[],"required_changes":[],"allow_next_step":false}
```

## 8. 校验规则
必须显式写出 `allow_next_step`。

## 9. 交接规则
供 `N09` 或 `N10` 消费。

## 10. 版本与修订规则
每轮评审独立版本。

## 11. 失败与缺失处理
缺结论则回流 `N08`。

