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
四个主章节必须存在。

## 9. 交接规则
供 `N28 session_closeout` 消费，也作为用户最终输出。

## 10. 版本与修订规则
仅在最终评审重新开启时生成新版本。

## 11. 失败与缺失处理
缺章节则回流 `N27`。

