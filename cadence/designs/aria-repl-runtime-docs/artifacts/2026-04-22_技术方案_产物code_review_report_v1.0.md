# 产物规范：code_review_report

## 1. 产物标识
- 类型 ID：`ART-CODE-REVIEW-REPORT`
- 类别：运行记录
- 默认节点：`N18/N19`

## 2. 产物目的
决定当前 WorkTask 是否允许进入集成队列。

## 3. 产出时机
每次 code review 后产出。

## 4. 产物存储位置
默认写入 `cadence/reports/`。

## 5. 最小字段清单
| 字段 | 必填 |
|------|------|
| `review_decision` | 是 |
| `findings` | 是 |
| `required_changes` | 是 |

## 6. 推荐结构
结论、发现、修改项、是否放行。

## 7. 固定格式示例
```json
{"review_decision":"pass|revise|conditional_pass","findings":[{"severity":"medium","description":"..."}],"required_changes":[]}
```

## 8. 校验规则

### L1 存在性校验
- `review_decision` 存在且为 `pass` / `revise` / `conditional_pass` 之一
- `findings` 存在（允许为空数组 `[]`）
- `required_changes` 存在（允许为空数组 `[]`）

### L2 结构性校验
- `review_decision` 为字符串，取值范围：`pass`, `revise`, `conditional_pass`
- `findings` 为数组，每个元素为对象（含 `severity`、`description` 字段）
- `required_changes` 为数组，每个元素为对象（含 `description` 字段）

### L3 语义性校验（二期增强）
- `review_decision` 为 `revise` 时，`required_changes` 不应为空
- `findings` 中 `severity` 取值应为 `high` / `medium` / `low`

## 9. 交接规则
`review_decision` 为 `pass` 或 `conditional_pass` 时供 `N20` 消费；为 `revise` 时供 `N19` 消费。

## 10. 版本与修订规则
每轮 review 生成新记录。

## 11. 失败与缺失处理
缺结论则回流 `N18`。

