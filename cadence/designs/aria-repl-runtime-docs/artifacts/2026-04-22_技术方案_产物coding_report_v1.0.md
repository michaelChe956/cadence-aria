# 产物规范：coding_report

## 1. 产物标识
- 类型 ID：`ART-CODING-REPORT`
- 类别：运行记录
- 默认节点：`N16/N19`

## 2. 产物目的
记录某个 WorkTask 的实现结果与变更范围。

## 3. 产出时机
coding 完成或 rework 后产出。

## 4. 产物存储位置
默认写入 `cadence/reports/`。

## 5. 最小字段清单
| 字段 | 必填 |
|------|------|
| `workTaskId` | 是 |
| `changed_files` | 是 |
| `implementation_summary` | 是 |
| `known_risks` | 是 |

## 6. 推荐结构
任务 ID、改动文件、实现摘要、已知风险。

## 7. 固定格式示例
```json
{"workTaskId":"work_001","changed_files":[],"implementation_summary":"","known_risks":[]}
```

## 8. 校验规则
`changed_files` 必须至少 1 项或显式声明无代码变更原因。

## 9. 交接规则
供 `N17`、`N18`、`N20` 消费。

## 10. 版本与修订规则
每次 rework 生成新版本。

## 11. 失败与缺失处理
缺任务 ID 则回流 `N16/N19`。

