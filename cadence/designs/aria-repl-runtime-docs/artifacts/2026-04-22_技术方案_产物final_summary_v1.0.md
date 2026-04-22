# 产物规范：final_summary

## 1. 产物标识
- 类型 ID：`ART-FINAL-SUMMARY`
- 类别：文档产物
- 默认节点：`N27`

## 2. 产物目的
作为用户最终可读的交付总结，说明完成内容、验证结果、剩余风险与下一步。

## 3. 产出时机
最终评审通过后产出。

## 4. 产物存储位置
默认写入 `cadence/reports/`。

## 5. 最小字段清单
| 字段 | 必填 |
|------|------|
| `completed_items` | 是 |
| `verification_summary` | 是 |
| `remaining_risks` | 是 |
| `next_steps` | 是 |

## 6. 推荐结构
完成项、验证摘要、剩余风险、下一步建议。

## 7. 固定格式示例
```md
# Final Summary
## Completed Items
## Verification Summary
## Remaining Risks
## Next Steps
```

## 8. 校验规则

### L1 存在性校验
- `completed_items` 存在且为非空数组（至少 1 项）
- `verification_summary` 存在且非空
- `remaining_risks` 存在（允许为空数组 `[]`）
- `next_steps` 存在（允许为空数组 `[]`）

### L2 结构性校验
- `completed_items` 为数组，每个元素为字符串或对象（含 `description` 字段）
- `verification_summary` 为字符串类型，长度 >= 20 字符
- `remaining_risks` 为数组，每个元素为 Risk Registry 中的 `riskId` 引用
- `next_steps` 为数组，每个元素为字符串

### L3 语义性校验（二期增强）
- `completed_items` 应覆盖 spec 中 `success_criteria` 的核心条目
- `remaining_risks` 中每个风险应在 Risk Registry 中有对应条目且状态为 `open`

## 9. 交接规则
供 `N28 session_closeout` 消费，也作为用户最终输出。

## 10. 版本与修订规则
仅在最终评审重新开启时生成新版本。

## 11. 失败与缺失处理
缺章节则回流 `N27`。

