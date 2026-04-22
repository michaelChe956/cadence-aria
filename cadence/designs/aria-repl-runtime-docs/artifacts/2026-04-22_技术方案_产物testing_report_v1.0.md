# 产物规范：testing_report

## 1. 产物标识
- 类型 ID：`ART-TESTING-REPORT`
- 类别：运行记录
- 默认节点：`N17/N19`

## 2. 产物目的
记录测试执行结果，决定进入 review 还是 rework。

## 3. 产出时机
testing 完成后产出。

## 4. 产物存储位置
默认写入 `cadence/reports/`。

## 5. 最小字段清单
| 字段 | 必填 |
|------|------|
| `tests_run` | 是 |
| `pass_fail_status` | 是 |
| `failures` | 是 |
| `next_recommendation` | 是 |
| `coverage_summary` | 否 |
| `test_types` | 否 |

## 6. 推荐结构
测试集合、通过状态、失败详情、建议下一步。

## 7. 固定格式示例
```json
{"tests_run":[{"test_id":"t1","name":"test case 1","status":"pass"}],"pass_fail_status":"pass|fail|partial|skip","failures":[],"next_recommendation":"N18","coverage_summary":{"line_coverage":85.2},"test_types":["unit"]}
```

## 8. 校验规则

### L1 存在性校验
- `tests_run` 存在且为非空数组（至少 1 条）
- `pass_fail_status` 存在且为 `pass` / `fail` / `partial` / `skip` 之一
- `failures` 存在（允许为空数组 `[]`）
- `next_recommendation` 存在且非空

### L2 结构性校验
- `tests_run` 为数组，每个元素为对象（含 `test_id`、`name`、`status` 字段）
- `pass_fail_status` 为字符串，取值范围：`pass`, `fail`, `partial`, `skip`
- `failures` 为数组，每个元素为对象（含 `test_id`、`error_message` 字段）
- `next_recommendation` 为字符串，必须是合法节点 ID
- `coverage_summary`（若存在）为对象（含 `line_coverage`、`branch_coverage` 字段）
- `test_types`（若存在）为数组，每个元素为字符串（如 `unit`, `integration`, `e2e`）

### L3 语义性校验（二期增强）
- `pass_fail_status` 为 `fail` 时，`failures` 不应为空
- `pass_fail_status` 为 `skip` 时，`tests_run` 应为空
- `coverage_summary` 中的百分比值应在 0-100 范围内

## 9. 交接规则
供 `N18` 或 `N19` 消费。

## 10. 版本与修订规则
每次测试执行新建记录。

## 11. 失败与缺失处理
未给出下一步建议则回流 `N17`。

