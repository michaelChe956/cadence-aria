# 产物规范：dispatch_package

## 1. 产物标识
- 类型 ID：`ART-DISPATCH-PACKAGE`
- 类别：交接包
- 默认节点：`N12`

## 2. 产物目的
把 plan 转成可注册、可调度的 WorkTask 集合。

## 3. 产出时机
plan 编写完成后产出；最终评审补丁任务也会复用。

## 4. 产物存储位置
默认写入 `cadence/plans/`。

## 5. 最小字段清单
| 字段 | 必填 |
|------|------|
| `worktasks` | 是 |
| `dependencies` | 是 |
| `parallel_groups` | 是 |
| `acceptance_targets` | 是 |

## 6. 推荐结构
任务列表、依赖图、并行组、验收目标。

## 7. 固定格式示例
```json
{"worktasks":[],"dependencies":[],"parallel_groups":[],"acceptance_targets":[]}
```

## 8. 校验规则
每个 worktask 必须有目标和验收标准。

## 9. 交接规则
供 `N13` 和 `N26` 消费。

## 10. 版本与修订规则
每次 dispatch 产出独立版本。

## 11. 失败与缺失处理
缺任务边界则回流 `N12`。

