# 产物规范：readiness_check

## 1. 产物标识
- 类型 ID：`ART-READINESS-CHECK`
- 类别：校验记录
- 默认节点：`N10`

## 2. 产物目的
判定 spec 和 design 是否足够支撑后续 plan/coding。

## 3. 产出时机
design review 通过后产出。

## 4. 产物存储位置
默认写入 `cadence/reports/`。

## 5. 最小字段清单
| 字段 | 必填 |
|------|------|
| `is_ready` | 是 |
| `blocking_items` | 是 |
| `recommended_backtrack_node` | 是 |
| `notes` | 否 |

## 6. 推荐结构
ready 结论、阻塞项、回退节点、备注。

## 7. 固定格式示例
```json
{"is_ready":true,"blocking_items":[],"recommended_backtrack_node":"N11"}
```

## 8. 校验规则
`recommended_backtrack_node` 必须是合法节点 ID。

## 9. 交接规则
供 `N11` 或 `N07` 消费。

## 10. 版本与修订规则
每次 readiness 判断新建记录。

## 11. 失败与缺失处理
无回退节点则停留 `N10`。

