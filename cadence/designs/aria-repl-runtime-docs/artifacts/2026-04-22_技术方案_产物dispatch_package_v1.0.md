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

### L1 存在性校验
- `worktasks` 存在且为非空数组（至少 1 个）
- `dependencies` 存在（允许为空数组 `[]`）
- `parallel_groups` 存在（允许为空数组 `[]`）
- `acceptance_targets` 存在且为非空数组（至少 1 个）

### L2 结构性校验
- `worktasks` 为数组，每个元素为对象（含 `id`、`goal`、`acceptance_criteria`、`dependencies` 字段）
- `dependencies` 为数组，每个元素为对象（含 `from`、`to` 字段）
- `parallel_groups` 为数组，每个元素为对象（含 `group_id`、`worktask_ids` 字段）
- `acceptance_targets` 为数组，每个元素为对象（含 `worktask_id`、`targets` 字段）

### L3 语义性校验（二期增强）
- 每个 worktask 的 `goal` 应可追溯 to plan 中的 work package
- `dependencies` 不应包含环路

## 9. 交接规则
供 `N13` 和 `N26` 消费。

## 10. 版本与修订规则
每次 dispatch 产出独立版本。

## 11. 失败与缺失处理
缺任务边界则回流 `N12`。

