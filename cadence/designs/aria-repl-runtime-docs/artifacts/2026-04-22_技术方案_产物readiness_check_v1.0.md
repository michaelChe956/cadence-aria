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

### L1 存在性校验
- `is_ready` 存在且为布尔值
- `blocking_items` 存在（允许为空数组 `[]`）
- `recommended_backtrack_node` 存在且非空

### L2 结构性校验
- `is_ready` 为布尔类型
- 当 `is_ready` 为 `false` 时，`blocking_items` 不应为空
- `blocking_items` 为数组，每个元素为对象（含 `description` 字段）
- `recommended_backtrack_node` 为字符串，必须是合法节点 ID

### L3 语义性校验（二期增强）
- `recommended_backtrack_node` 应限制在合理回流范围内（N04-N09）

## 9. 交接规则
供 `N11` 或 `N07` 消费。

## 10. 版本与修订规则
每次 readiness 判断新建记录。

## 11. 失败与缺失处理
无回退节点则停留 `N10`。

