# 产物规范：design_revision_record

## 1. 产物标识
- 类型 ID：`ART-DESIGN-REVISION-RECORD`
- 类别：文档产物
- 默认节点：`N09`

## 2. 产物目的
记录设计修订如何响应 design review 的问题。

## 3. 产出时机
每次 design revision 后产出。

## 4. 产物存储位置
默认写入 `cadence/designs-reviews/`。

## 5. 最小字段清单
| 字段 | 必填 |
|------|------|
| `revision_summary` | 是 |
| `resolved_findings` | 是 |
| `remaining_risks` | 是 |
| `updated_design_ref` | 是 |

## 6. 推荐结构
修订摘要、已解决问题、剩余风险、更新 design 引用。

## 7. 固定格式示例
```json
{"revision_summary":"","resolved_findings":[],"remaining_risks":[],"updated_design_ref":"art_design_001"}
```

## 8. 校验规则
`updated_design_ref` 必须存在。

## 9. 交接规则
供 `N08` 再次评审消费。

## 10. 版本与修订规则
跟随对应 design review 轮次。

## 11. 失败与缺失处理
缺引用则回流 `N09`。

