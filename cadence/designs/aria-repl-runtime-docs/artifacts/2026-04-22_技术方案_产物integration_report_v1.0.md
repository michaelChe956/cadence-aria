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

### L1 存在性校验
- `attemptId` 存在且非空
- `integrationMode` 存在且为 `merge` / `rebase` / `cherry_pick` 之一
- `gitResult` 存在且非空
- `conflictFiles` 存在（允许为空数组 `[]`）
- `nextDecision` 存在且为 `proceed` / `rework` / `rollback` / `escalate` 之一
- `verificationStatus` 存在且为 `pass` / `fail` / `rollback_executed` 之一
- `postIntegrationFailures` 存在（允许为空数组 `[]`）
- `rollbackNeeded` 存在且为布尔值

### L2 结构性校验
- `attemptId` 为字符串类型
- `integrationMode` 为字符串，取值范围：`merge`, `rebase`, `cherry_pick`
- `gitResult` 为字符串或对象类型
- `conflictFiles` 为数组，每个元素为对象（含 `path`、`conflict_type` 字段）
- `nextDecision` 为字符串，取值范围：`proceed`, `rework`, `rollback`, `escalate`
- `verificationStatus` 为字符串，取值范围：`pass`, `fail`, `rollback_executed`
- `postIntegrationFailures` 为数组
- `rollbackNeeded` 为布尔类型

### L3 语义性校验（二期增强）
- `verificationStatus` 为 `pass` 时，`postIntegrationFailures` 应为空
- `rollbackNeeded` 为 `true` 时，`nextDecision` 不应为 `proceed`

## 9. 交接规则
供 `N24`、`N25` 或 `N19` 消费。

## 10. 版本与修订规则
同一 attempt 可更新，同一任务多次尝试应新建 attempt。

## 11. 失败与缺失处理
缺关键字段则回流 `N23/N24`。

