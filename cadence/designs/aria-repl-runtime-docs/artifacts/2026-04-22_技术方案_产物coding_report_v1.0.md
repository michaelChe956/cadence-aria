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

### L1 存在性校验
- `workTaskId` 存在且非空
- `changed_files` 存在且为非空数组（至少 1 项），或显式声明无代码变更原因
- `implementation_summary` 存在且非空
- `known_risks` 存在（允许为空数组 `[]`）

### L2 结构性校验
- `workTaskId` 为字符串，格式符合 `work_` 前缀规则
- `changed_files` 为数组，每个元素为相对文件路径字符串
- `implementation_summary` 为字符串类型，长度 >= 20 字符
- `known_risks` 为数组，每个元素为 Risk Registry 中的 `riskId` 引用

### L3 语义性校验（二期增强）
- `changed_files` 中的文件路径应存在于 worktree 中
- `implementation_summary` 应涵盖所有 `changed_files` 的变更意图

## 9. 交接规则
供 `N17`、`N18`、`N20` 消费。

## 10. 版本与修订规则
每次 rework 生成新版本。

## 11. 失败与缺失处理
缺任务 ID 则回流 `N16/N19`。

