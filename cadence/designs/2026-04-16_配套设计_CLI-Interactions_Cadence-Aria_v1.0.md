# Cadence-Aria CLI Interactions 配套设计

> **版本**：v1.0
> **日期**：2026-04-16
> **关联主文档**：`cadence/designs/2026-04-16_方案设计_Cadence-Aria_v1.3.md`

## 目标

本配套文档把主设计中的命令面转换成可执行的交互快照，用于：

1. CLI 输出设计
2. 状态机行为回归测试
3. 用户验收时的对照样例

## 输出约束

1. 命令输出优先展示 `task_id`、`status`、`next`
2. 未完成任务的 `aria:result` 只能返回当前摘要
3. 输出字段名尽量复用 `state.yaml` 与 runtime schema 中的字段名
4. 外部同步失败、能力降级等异常必须带错误码

## 示例 1：native formal flow

```text
$ aria:intake "为 Aria 增加 capability report 结构化输出"

[Aria]
- task_id: aria-20260416-001
- source: native
- flow_type_suggestion: formal
- risk_level: medium
- next: aria:start --task-id aria-20260416-001
```

```text
$ aria:start --task-id aria-20260416-001

[Aria]
- status: planned
- plan_id: plan-aria-20260416-001
- exec_unit_count: 2
- parallel_candidates: [exec-01, exec-02]
- quality_gates: [format_check, contract_validation]
- next: aria:run --task-id aria-20260416-001
```

```text
$ aria:run --task-id aria-20260416-001

[Aria]
- status: executing
- started_exec_units: [exec-01, exec-02]
- review_status: pending
- test_status: pending
```

```text
$ aria:result --task-id aria-20260416-001

[Aria]
- current_status: executing
- summary: "2 个执行单元运行中，暂无最终结果"
```

## 示例 2：计划重开

```text
$ aria:start --task-id aria-20260416-001 --replan

[Aria]
- task_id: aria-20260416-001
- previous_plan_id: plan-aria-20260416-001
- new_plan_id: plan-aria-20260416-002
- superseded: [plan-aria-20260416-001]
- status: planned
```

### 约束

1. `task_id` 不变
2. 原计划不删除
3. 只有 `plan_id` 更新

## 示例 3：patch 闭环

```text
$ aria:run --task-id aria-20260416-002

[Aria]
- status: reviewing/testing
- review_status: failed
- test_status: passed
- patch_required_by: review
- next: generating patch contract
```

```text
$ aria:status --task-id aria-20260416-002

[Aria]
- status: patching
- patch_unit: patch-01
- source_exec_unit: exec-02
- must_fix_count: 2
- advisory_only_count: 1
```

```text
$ aria:result --task-id aria-20260416-002

[Aria]
- final_status: done
- patch_round: 1
- review_status: passed
- test_status: passed
```

## 示例 4：取消、恢复与重试

```text
$ aria:cancel --task-id aria-20260416-003

[Aria]
- status: cancelled
- cancelled_exec_units: [exec-01]
- blocked_exec_units: [exec-02]
- artifacts_preserved: true
```

```text
$ aria:run --task-id aria-20260416-003 --resume

[Aria]
- resumed_from: cancelled
- retryable_units: [exec-01]
- blocked_units_recomputed: true
- next: user may run aria:retry --task-id aria-20260416-003
```

```text
$ aria:retry --task-id aria-20260416-003

[Aria]
- retried_units: [exec-01]
- unblocked_units: [exec-02]
- status: executing
```

## 示例 5：VK 同步重投影

```text
$ aria:status --task-id aria-20260416-004 --sync

[Aria]
- task_id: aria-20260416-004
- internal_status: verified
- vk_sync: attempted
- vk_sync_result: success
```

若同步失败：

```text
[Aria]
- task_id: aria-20260416-004
- internal_status: verified
- vk_sync: failed
- error_code: ARIA-SYNC-002
- note: "主流程状态不受影响"
```

## 示例 6：fast-lane 升级

```text
$ aria:fast "修复跨模块配置读取错误"

[Aria]
- task_id: aria-20260416-005
- flow_type_suggestion: fast-lane
- execution_result: upgrade-required
- reason: "cross_module=true"
- next: aria:start --task-id aria-20260416-005
```

## 示例 7：能力探测失败

```text
$ aria:start --task-id aria-20260416-006

[Aria]
- task_id: aria-20260416-006
- status: blocked
- error_code: ARIA-CAP-101
- reason: "openspec.change.create unavailable"
- next: "修复依赖后重试 aria:start --task-id aria-20260416-006"
```

## 示例 8：状态损坏

```text
$ aria:run --task-id aria-20260416-007 --resume

[Aria]
- task_id: aria-20260416-007
- status: blocked
- error_code: ARIA-STATE-002
- reason: "state.yaml 与 dispatch contract 不一致"
- next: "按恢复规则人工修复或退回 plan/spec"
```

## 快照测试建议

实现 CLI 时，建议把以上示例直接转成快照测试：

1. 每个示例一组输入输出快照
2. 每个异常分支至少包含错误码与 next
3. 输出中的 `task_id`、状态和错误码应保持稳定
