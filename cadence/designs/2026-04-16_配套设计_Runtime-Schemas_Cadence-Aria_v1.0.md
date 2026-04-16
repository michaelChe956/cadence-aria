# Cadence-Aria Runtime Schemas 配套设计

> **版本**：v1.0
> **日期**：2026-04-16
> **关联主文档**：`cadence/designs/2026-04-16_方案设计_Cadence-Aria_v1.3.md`

## 目标

本配套文档只回答一类问题：

1. 运行时工件有哪些结构字段
2. 字段的类型、必填性、枚举和值域是什么
3. 哪些字段必须由系统生成，哪些字段允许为空

本文件不重复解释业务目标、角色边界与状态机原则；这些内容以主设计文档为准。

## 总体约束

1. 所有时间字段统一使用 ISO 8601 字符串
2. 所有路径字段统一使用仓库相对路径
3. 所有 ID 字段必须可回溯到单一任务、单一执行单元或单一问题项
4. 同名字段跨工件保持同一语义
5. schema 是实现约束，不是展示文案

## `state.yaml`

### 顶层字段

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `task_id` | string | 是 | `aria-YYYYMMDD-NNN` |
| `source` | enum | 是 | `vk \| native \| aria-native` |
| `flow_type` | enum | 是 | `formal \| fast-lane` |
| `status` | enum | 是 | 必须属于状态机合法状态 |
| `current_round` | integer | 是 | `>= 1` |
| `active_exec_units` | string[] | 是 | 可为空数组 |
| `review_status` | enum | 是 | `pending \| passed \| failed` |
| `test_status` | enum | 是 | `pending \| passed \| failed` |
| `patch_required_by` | enum | 是 | `none \| review \| test \| both` |
| `patch_round` | integer | 是 | `>= 0` |
| `exec_units` | map | 是 | key 为 `exec-xx` |
| `patch_units` | map | 否 | key 为 `patch-xx` |
| `created_at` | datetime string | 是 | ISO 8601 |
| `updated_at` | datetime string | 是 | ISO 8601 |
| `workspace_ref` | string | 否 | 可空 |
| `worktree_ref` | string | 否 | 可空 |

### `exec_units.<id>`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `status` | enum | 是 | `pending \| running \| succeeded \| failed \| timeout \| cancelled \| blocked` |
| `contract_path` | string | 是 | 指向 `dispatch contract` |
| `worktree_ref` | string | 否 | 并行模式建议非空 |
| `attempt` | integer | 是 | `>= 0` |
| `exit_code` | integer/null | 是 | 未结束可为 `null` |
| `result_path` | string | 是 | 指向 exec result |
| `started_at` | datetime string | 否 | 未开始可空 |
| `finished_at` | datetime string | 否 | 未结束可空 |
| `blocked_by` | string[] | 是 | 可为空数组 |

### `patch_units.<id>`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `status` | enum | 是 | `pending \| running \| succeeded \| failed \| cancelled` |
| `based_on_exec_unit` | string | 是 | 必须引用已存在的 `exec-xx` |
| `contract_path` | string | 是 | 指向 `patch contract` |
| `attempt` | integer | 是 | `>= 0` |
| `started_at` | datetime string | 否 | 未开始可空 |
| `finished_at` | datetime string | 否 | 未结束可空 |

## `task intake card`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `task_id` | string | 是 | 与 `state.yaml.task_id` 一致 |
| `source` | enum | 是 | `vk \| native \| aria-native` |
| `flow_type_suggestion` | enum | 是 | `formal \| fast-lane` |
| `risk_level` | enum | 是 | `low \| medium \| high` |
| `scope_summary` | string | 是 | 非空 |
| `boundary_check` | object | 是 | 包含布尔边界判定字段 |
| `created_at` | datetime string | 是 | ISO 8601 |

## `plan brief`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `plan_id` | string | 是 | `plan-<task-id>` 或其变体 |
| `task_id` | string | 是 | 必须映射现有任务 |
| `quality_gates` | object[] | 是 | 至少 1 项 |
| `exec_unit_count` | integer | 是 | `>= 1` |
| `parallel_candidates` | array | 否 | 每项为 exec unit 组 |
| `acceptance_strategy` | enum/string | 是 | 一期至少支持 `all_units_pass` |
| `generated_at` | datetime string | 是 | ISO 8601 |

### `quality_gates[]`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `type` | string | 是 | 如 `test_coverage`、`format_check` |
| `threshold` | integer | 否 | 数值型门槛时使用 |
| `enabled` | boolean | 否 | 开关型门槛时使用 |
| `command_ref` | string | 否 | 指向验证命令来源 |

## `review report`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `task_id` | string | 是 | 必须映射现有任务 |
| `exec_units_reviewed` | string[] | 是 | 不得为空 |
| `blockers` | object[] | 否 | 每项必须有 `issue_id` 与 `severity` |
| `suggestions` | object[] | 否 | 建议项不得进入 `must_fix` |
| `verdict` | enum | 是 | `passed \| failed \| needs_patch` |
| `reviewed_at` | datetime string | 是 | ISO 8601 |

### `blockers[]` / `suggestions[]`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `issue_id` | string | 是 | 在任务范围内唯一 |
| `severity` | enum | 是 | blocker 或 advisory |
| `exec_unit` | string | 否 | 可定位到单元则填写 |
| `description` | string | 是 | 非空 |
| `file_path` | string | 否 | 仓库相对路径 |
| `line_range` | string | 否 | 文本表示范围 |

## `test report`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `task_id` | string | 是 | 必须映射现有任务 |
| `exec_units_tested` | string[] | 是 | 不得为空 |
| `failures` | object[] | 否 | 失败时必须包含 `test_command` 与 `evidence` |
| `passed_count` | integer | 是 | `>= 0` |
| `failed_count` | integer | 是 | `>= 0` |
| `verdict` | enum | 是 | `passed \| failed` |
| `tested_at` | datetime string | 是 | ISO 8601 |

### `failures[]`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `test_id` | string | 是 | 在任务范围内唯一 |
| `exec_unit` | string | 否 | 可定位到单元则填写 |
| `evidence` | string | 是 | 必须是可审计证据摘要 |
| `severity` | enum | 是 | `blocker \| warning` |
| `file_path` | string | 否 | 仓库相对路径 |
| `test_command` | string | 是 | 必须来自实际执行命令 |

## `dispatch contract`

### 共享字段

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `contract_version` | string | 是 | 一期固定 `1.0` |
| `generated_at` | datetime string | 是 | ISO 8601 |
| `base_revision` | string | 是 | Git revision |
| `input_artifacts` | object | 是 | 路径必须存在于当前任务工件集中 |
| `generated_from_plan` | string | 是 | 指向 `plan_id` |
| `source_task_refs` | string[] | 是 | 至少 1 项 |
| `task_id` | string | 是 | 必须映射现有任务 |
| `timeout_minutes` | integer | 是 | `> 0` 且受配置上限约束 |

### 专属字段

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `exec_unit_id` | string | 是 | `exec-xx` |
| `parent_task` | string | 是 | 映射 OpenSpec task |
| `mode` | enum | 是 | 一期固定 `exec` |
| `scope` | object | 是 | 至少包含 `files_allowed` |
| `goal` | string | 是 | 非空 |
| `acceptance` | string[] | 是 | 至少 1 项 |
| `dependencies` | string[] | 是 | 可为空数组 |
| `worktree_ref` | string | 否 | 并行模式建议非空 |
| `result_path` | string | 是 | 当前任务内唯一 |
| `retry_allowed` | boolean | 是 | 显式指定 |

### `scope`

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `files_allowed` | string[] | 是 | 至少 1 项 |
| `files_blocked` | string[] | 否 | 不得与 `files_allowed` 重叠 |

## `patch contract`

### 共享字段

复用 `dispatch contract` 的共享字段约束。

### 专属字段

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `patch_unit_id` | string | 是 | `patch-xx` |
| `source_exec_unit` | string | 是 | 必须引用已存在 `exec-xx` |
| `based_on_dispatch_contract` | string | 是 | 指向原始 dispatch contract |
| `must_fix` | string[] | 是 | 至少 1 项，且均为 blocker 问题 ID |
| `advisory_only` | string[] | 否 | 可为空 |
| `must_not_change` | string[] | 是 | 至少 1 项 |
| `acceptance` | string[] | 是 | 至少 1 项 |
| `patch_required_by` | enum | 是 | `review \| test \| both` |

## `verification summary`

> 仍以 Markdown 为主，以下为建议 front matter。

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `task_id` | string | 是 | 必须映射现有任务 |
| `review_verdict` | enum | 是 | `passed \| failed` |
| `test_verdict` | enum | 是 | `passed \| failed` |
| `final_patch_round` | integer | 是 | `>= 0` |

## `closure summary`

> 仍以 Markdown 为主，以下为建议 front matter。

| 字段 | 类型 | 必填 | 约束 |
|------|------|------|------|
| `task_id` | string | 是 | 必须映射现有任务 |
| `final_status` | enum | 是 | `done \| cancelled` |
| `completed_at` | datetime string | 是 | ISO 8601 |
| `recovery_actions` | string[] | 否 | 仅在人工恢复发生时建议填写 |

## 使用方式

实现阶段建议按以下顺序消费本文件：

1. 先以本文件生成 schema 校验器或类型定义
2. 再用主文档的状态机与错误码规则补全行为约束
3. 最后用 CLI 交互示例文档校验输出与状态流转
