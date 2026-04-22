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
{"overallDecision":"pass|followup","completedScope":[],"remainingGaps":[],"followupNeeded":false}
```

## 8. 校验规则
`followupNeeded` 为 true 时，`remainingGaps` 不能为空。

## 9. 交接规则
供 `N26` 或 `N27` 消费。

## 10. 版本与修订规则
每次最终评审新建记录。

## 11. 失败与缺失处理
缺总体结论则回流 `N25`。

