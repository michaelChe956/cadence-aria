## Why

group final review 给出 `request_changes` 结论时，流程静默退出：attempt 停在 `running` / `internal_pr_review`，既没有 blocked gate，也没有任何可操作入口，用户只看到「GroupFinalReview 要求修改」但无法继续、返修或终止。

**门禁只在 `Blocked` 下落地。** `src/product/coding_workspace_engine/internal_pr_review.rs:385` 的 `blocked_gate_reason` 以 `review.verdict == ReviewVerdict::Blocked` 为前置条件，`RequestChanges` 分支（`:334-346`）只把 timeline 节点置 Failed、role run 置 Completed、`reason_code` 置 `None`，不落门禁、不改 attempt 状态。

**runner 侧也不兜底。** `src/web/coding_ws_handler/runner.rs:233-238` 把 `RunCoderFix`、`RetryVerification`、`StartStoryAmendment`、`StartDesignAmendment`、`OpenOperationalGate`、`StopForHumanTriage` 六个决策统一处理为 `current.clone()`——即原样返回、不做任何状态流转。attempt 因此留在 `running`，而 pipeline 已经返回，没有任何后续动作会推进它。

**缺口的准确边界是 `RequestChanges`，不是全部结论。** `Blocked` 结论按决策仍会落门禁：actionable finding 时 `plan_defect.rs:215-217` 推出 `RunCoderFix` → `internal_review_blocked` / `group_final_review_blocked`；非 actionable 时推出 `StopForHumanTriage` → `internal_review_human_triage`。因此 `internal_pr_review.rs:8-10` 的 `RunCoderFix if is_group_final_review` 分支是**可达的**（现存单测 `tests/plan_defect_entrypoints.rs:319-322` 直接断言它），本变更不移除。

Code Review 阶段的同类缺陷已由 `open-code-review-triage-gate` 修复（`code_review.rs:202-253`）。但其互斥判定依赖一条 internal review 没有的独立落地分支（`code_review.rs:202-203` 的 `lands_code_review_blocked`，条件 `Blocked && !actionable`），因此形式可参考、语义不可照搬。

## What Changes

- group final review 与 internal PR review 在 `request_changes` 结论下必须落地 blocked gate，并把 attempt 从 `running` 置为 `blocked`。
- 对 `RunCoderFix`、`RetryVerification`、`StopForHumanTriage`、`OpenOperationalGate` 四个流程决策分别使用互不相同的 reason code，便于 UI 与排查区分停机原因。
- 门禁提供可操作动作：重试评审、人工继续、终止。
- 落门禁判定收敛为「流程决策 → reason code」的单一映射，一个决策对应一个门禁，天然互斥，不再以 verdict 为前置。
- 保留 `internal_pr_review.rs:8-10` 的 `RunCoderFix if is_group_final_review` 分支：该分支在「阻塞结论 + actionable finding」下可达，不是死代码。
- **不提供 internal PR review 阶段的送回 Coder 返修**：该动作有两层结构性阻断（`valid_stage_transition` 只白名单 `CodeReview → Coding`；group final review 时 `active_unit_id` 已清空，退回 Coding 会以 `unit_run_projection_binding_missing` 失败），需要新增阶段回退许可与重开 unit 语义，超出本变更边界。记入已知缺口。
- 顺带移除既有 `internal_review_blocked` 门禁上必定失败的 `send_to_coder` 动作（`provider_failure.rs:65-70`）。
- 不改变 `internal_review_flow_decision` 与 `internal_review_flow_decision_with_bindings` 的决策判定逻辑。
- 不改变 plan repair 的唤起条件：分诊门禁的任何动作都不触发 plan repair。
- 不改变 `Approve` 结论的完成路径与 `StartPlanRepair`、`ContinueAfterApprove` 两个决策的既有处理。
- 不改变评审提示词与 reviewer 的判断口径（属 `fix-process-evidence-as-acceptance`）。

## Capabilities

### New Capabilities

- `group-final-review-triage`: group final review 与 internal PR review 阶段人工路由决策的停机语义，包括 `request_changes` 结论落地 blocked gate、attempt 状态流转、reason code 区分、门禁互斥，以及手动送回 Coder 通道在该阶段的可用条件。

### Modified Capabilities

（无。现有 specs 未覆盖 internal PR review 阶段停机语义。）

## Impact

- `src/product/coding_workspace_engine/internal_pr_review.rs`：落门禁判定去掉 `verdict == Blocked` 前置，改为流程决策的单一映射；新增两个 reason code（`internal_review_change_requested`、`internal_review_verification_incomplete`）。
- `src/product/coding_workspace_engine/provider_failure.rs`：`create_review_blocked_gate` 的动作集合增加新 reason code 分支，提供重试评审、人工继续、终止三个动作；移除既有 internal review 门禁上必定失败的 `send_to_coder`。
- `src/product/coding_workspace_engine/tests/plan_defect_entrypoints.rs`：改写与新 spec 冲突的两处断言（`:330-474` 的「保持 Running、无 gate」、`:310-327` 的 `RetryVerification → None`）；`:132-137`（human triage 不含 send_to_coder）与 `:319-322`（`RunCoderFix(group)` reason code）**保持不变**。
- 受影响的用户可见行为：group final review 给出 `request_changes` 时不再静默停机，出现带三个动作的阻塞门禁；不同停机原因以不同 reason code 区分。
- 不影响 Code Review 阶段的分诊门禁行为。
- 不改 `gates.rs` 的 `is_code_review_feedback_gate`、`rework.rs`、`web/coding_ws_handler/runner.rs`：送回 Coder 移出范围后无需扩展前者；runner 状态读回已确认无需改动（`provider_failure.rs:72-77` 先写 `Blocked` 再建 gate，`runner.rs:423-424`/`:761-762` 引擎返回后即 `get_attempt`，`state.rs:293` 自身重新读库）。

## 依赖与顺序

本 change 与 `remove-work-item-handoff`、`remove-testing-stage` 独立，可并行或任意顺序实施。但若在其后实施，需注意评审证据字段已变化，测试夹具需相应调整。

`open-code-review-triage-gate` 提供了本 change 的实现样板（`code_review.rs:202-253`、`provider_failure.rs:53-64`）。该 change 尚未归档，实施时需确认不回退其结论。
