# 产物规范：integration_report

## 1. 产物标识
- 类型 ID：`ART-INTEGRATION-REPORT`
- 类别：运行记录
- 默认节点：`N23/N24`

## 2. 产物目的
记录串行集成及其后验证结果，是是否进入最终评审的依据。

## 3. 产出时机
集成执行和集成验证后更新。

## 4. 产物存储位置
默认写入 `cadence/reports/`。

## 5. 最小字段清单
| 字段 | 必填 |
|------|------|
| `attemptId` | 是 |
| `integrationMode` | 是 |
| `gitResult` | 是 |
| `conflictFiles` | 是 |
| `nextDecision` | 是 |
| `verificationStatus` | 是 |
| `postIntegrationFailures` | 是 |
| `rollbackNeeded` | 是 |

## 6. 推荐结构
合入结果、冲突、验证结果、回滚决策。

## 7. 固定格式示例
```json
{"attemptId":"int_001","integrationMode":"merge","gitResult":"success","conflictFiles":[],"nextDecision":"N25","verificationStatus":"pass","postIntegrationFailures":[],"rollbackNeeded":false}
```

## 8. 校验规则
失败时必须写 `nextDecision`。

## 9. 交接规则
供 `N24`、`N25` 或 `N19` 消费。

## 10. 版本与修订规则
同一 attempt 可更新，同一任务多次尝试应新建 attempt。

## 11. 失败与缺失处理
缺关键字段则回流 `N23/N24`。

