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

## 6. 推荐结构
测试集合、通过状态、失败详情、建议下一步。

## 7. 固定格式示例
```json
{"tests_run":[],"pass_fail_status":"pass|fail","failures":[],"next_recommendation":"N18"}
```

## 8. 校验规则
失败时 `failures` 不能为空。

## 9. 交接规则
供 `N18` 或 `N19` 消费。

## 10. 版本与修订规则
每次测试执行新建记录。

## 11. 失败与缺失处理
未给出下一步建议则回流 `N17`。

