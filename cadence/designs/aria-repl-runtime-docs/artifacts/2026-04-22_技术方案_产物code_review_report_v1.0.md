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
| `allow_integration` | 是 |

## 6. 推荐结构
结论、发现、修改项、是否放行。

## 7. 固定格式示例
```json
{"review_decision":"pass|revise","findings":[],"required_changes":[],"allow_integration":false}
```

## 8. 校验规则
`allow_integration` 必须与 `review_decision` 一致。

## 9. 交接规则
供 `N20` 或 `N19` 消费。

## 10. 版本与修订规则
每轮 review 生成新记录。

## 11. 失败与缺失处理
缺结论则回流 `N18`。

