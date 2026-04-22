# 产物规范：final_review

## 1. 产物标识
- 类型 ID：`ART-FINAL-REVIEW`
- 类别：文档产物
- 默认节点：`N25`

## 2. 产物目的
从系统级视角判断是否真正完成，或需要派生补丁任务。

## 3. 产出时机
所有集成完成后产出。

## 4. 产物存储位置
默认写入 `cadence/reports/`。

## 5. 最小字段清单
| 字段 | 必填 |
|------|------|
| `overallDecision` | 是 |
| `completedScope` | 是 |
| `remainingGaps` | 是 |
| `followupNeeded` | 是 |

## 6. 推荐结构
总体结论、已完成范围、剩余缺口、补丁需求。

## 7. 固定格式示例
```json
{"overallDecision":"pass|followup|fail","completedScope":[{"item":"...","status":"completed"}],"remainingGaps":[],"followupNeeded":false}
```

## 8. 校验规则

### L1 存在性校验
- `overallDecision` 存在且为 `pass` / `followup` / `fail` 之一
- `completedScope` 存在且非空
- `remainingGaps` 存在（允许为空数组 `[]`）
- `followupNeeded` 存在且为布尔值

### L2 结构性校验
- `overallDecision` 为字符串，取值范围：`pass`, `followup`, `fail`
- `completedScope` 为数组，每个元素为对象（含 `item`、`status` 字段）
- `remainingGaps` 为数组，每个元素为对象（含 `description`、`severity` 字段）
- `followupNeeded` 为布尔类型

### L3 语义性校验（二期增强）
- `overallDecision` 为 `followup` 时，`followupNeeded` 必须为 `true` 且 `remainingGaps` 不应为空
- `overallDecision` 为 `fail` 时，应进入 X08 manual_intervention
- `completedScope` 中的 `status` 取值应为 `completed` / `partial`

## 9. 交接规则
供 `N26` 或 `N27` 消费。

## 10. 版本与修订规则
每次最终评审新建记录。

## 11. 失败与缺失处理
缺总体结论则回流 `N25`。

