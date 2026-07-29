# 计划文档：GroupFinalReview 要求修改落地门禁

- **OpenSpec Change**：`open-group-final-review-change-gate`
- **Capability**：`group-final-review-triage`
- **日期**：2026-07-28
- **版本**：v1.1（评审后修订：修正「不可达分支」误判、重建互斥方案、送回 Coder 移出范围、关闭三处实施期待定项）

## 目标

消除 internal PR review（含 group final review）在 `request_changes` 结论下的静默停机：四个需人工介入的决策各落地一个 reason code 互不相同的阻塞门禁，并提供重试评审、人工继续、终止三个动作。

## 前置条件

- 工作树可编译、`cargo fmt --check` 与 `clippy -D warnings` 干净。
- 已知既有失败基线：`large_file_guard`（会使 `cargo test --locked` 提前终止，因此 `it_web` / `it_provider` / `it_task_run` / `it_product` 需单独运行）。
- 本 change 与 `remove-work-item-handoff`、`remove-testing-stage` 独立。若在其后实施，评审证据字段已变化，测试夹具需相应调整。

## 实现样板

`open-code-review-triage-gate` 已为 Code Review 阶段实现同类语义，但**只能参考形式，不能照搬语义**（见「互斥方案」）：

| 内容 | 样板位置 | 可照搬程度 |
|---|---|---|
| 分诊门禁落地 | `code_review.rs:202-253` | 形式可参考 |
| 互斥判定 `lands_code_review_blocked` | `code_review.rs:202-203`（**注意：不是 `:218-222`，那里是注释**） | 语义**不可**照搬 |
| reason code → 动作集合映射 | `provider_failure.rs:53-64` | 可照搬 |
| 门禁判定用于动作路由 | `gates.rs:756-766` | 本变更不需要（送回 Coder 已移出范围） |

**该 change 尚未归档**，实施时需确认改动不回退其结论。

## 关键落点

### 缺陷位置

| 位置 | 现状 | 处置 |
|---|---|---|
| `internal_pr_review.rs:385` | `blocked_gate_reason` 以 `verdict == Blocked` 为前置 | 去掉该前置，改由流程决策决定 |
| `internal_pr_review.rs:334-346` | `RequestChanges` 分支不落门禁、`reason_code` 为 `None` | 保留 timeline / role run 处理，门禁由新判定路径落地 |
| `internal_pr_review.rs:8-10` | `RunCoderFix if is_group_final_review` | 🔴 **保留**（原判断「不可达」是错的，见下） |
| `runner.rs:233-238` | 六个决策统一 `current.clone()` | **不改**（状态读回已确认无问题） |

### 🔴 缺口边界：只覆盖 `RequestChanges`，不覆盖 `Blocked`

原 Plan 把 `internal_pr_review.rs:8-10` 判为不可达死代码并计划移除，**这是错的**。`plan_defect.rs:215-217`：

```rust
ReviewVerdict::Blocked if review_findings_have_actionable_findings(findings) => {
    CodeReviewFlowDecision::RunCoderFix
}
```

「阻塞结论 + 至少一条 actionable finding」（`mod.rs:135-150`：severity 为 error/warning 且有 message）就推出 `RunCoderFix`，此时 `verdict == Blocked` 前置成立，分支正常生效。这是 reviewer 给阻塞结论时最常见的形态。现存单测 `tests/plan_defect_entrypoints.rs:319-322` 直接断言它。

原设计的论证「`RunCoderFix` 由 actionable finding 推出、通常伴随 `RequestChanges`」本身自相矛盾——actionable finding + Blocked 恰恰就是它可达的路径，「通常」被当成了「只能」。

实际现状：

| 结论 | 决策 | 当前落地 |
|---|---|---|
| `Blocked` + actionable | `RunCoderFix` | `internal_review_blocked` / `group_final_review_blocked` |
| `Blocked` + 非 actionable | `StopForHumanTriage` | `internal_review_human_triage` |
| `RequestChanges` | 任意 | **无** ← 这才是缺口 |

### 落门禁的决策集合

| 决策 | 落门禁 | reason code |
|---|---|---|
| `RunCoderFix` | 是 | `internal_review_change_requested` |
| `RetryVerification` | 是 | `internal_review_verification_incomplete` |
| `StopForHumanTriage` | 是 | `internal_review_human_triage`（已存在） |
| `OpenOperationalGate` | 是 | `internal_review_operational_blocker`（已存在） |
| `StartPlanRepair` | 否 | 已有 `start_plan_repair_from_internal_review` 编排 |
| `ContinueAfterApprove` | 否 | 已有完成编排 |
| `StartStoryAmendment` / `StartDesignAmendment` | 否 | 见「已知缺口」 |

后两个 reason code 已存在于 `internal_pr_review.rs:12-13`。

命名家族不齐（Code Review 侧是 `code_review_output_human_triage`，多个 `output_`）是既有状况，**不改名**——改名会牵动既有测试与前端查表。

### 🔴 互斥方案：单一映射，不是排除条件

原设计写「`Blocked` 结论已由既有分支落 `internal_review_blocked` / `group_final_review_blocked`；新分诊门禁在该情形下跳过」——**前提不成立**。internal review 里没有这个「既有分支」：`internal_pr_review.rs:385-414` 只有一条落门禁路径，正是本次要改的那条。

Code Review 侧能做互斥，是因为 `code_review.rs:202-203` 有一条独立的字面量 `code_review_blocked` 落地分支（条件 `Blocked && !actionable`）。internal review 没有对等物。

**本变更做法**：把落门禁判定收敛为「流程决策 → reason code」的**单一映射**。一个决策对应一个 reason code，天然互斥，不需要额外排除条件。`Blocked` 结论不再单独走一条落地路径——它推出的决策已经落在四类之中，由同一映射处理。

这样既消除重复落地的可能，也避免回答「结论本身是 Blocked 该落哪个 gate」这个在原设计里悬空的问题。`group_final_review_blocked` 保留为 group 场景下 `RunCoderFix` 的 reason code（即现状），不新增含义。

### 🔴 送回 Coder：移出本变更范围

原计划让门禁提供「送回 Coder 返修」。该动作在 internal PR review 阶段有**两层结构性阻断**，不是放宽前置条件能解决的：

1. **阶段机不允许回退**。`coding_attempt_store/attempt.rs:696-710` 的 `valid_stage_transition` 只白名单 `CodeReview → Coding`，其余要求 `next.order() >= current.order()`。`InternalPrReview` order=6、`Coding` order=2，`rework.rs:499-504` 的 `update_attempt_stage(..., Coding)` 会直接报 `invalid_coding_attempt_stage_transition`。
2. **group 场景没有可返修的 unit**。group final review 时全部 unit 已 Completed、`active_unit_id` 被清空（`coding_attempt_store/group.rs:358-367`），`group_validation.rs:330-352` 只在 `stage.order() >= ReviewRequest` 时允许空指针。退回 `Coding` 后 `plan_defect.rs:530-537` 的 `get_active_coding_unit` 返回 None，`execute_coding_with_commands_outcome → render_coder_unit_run_context` 会以 `unit_run_projection_binding_missing: active unit` 失败。

用户报告的正是 group final review 场景，而该场景下「回到 Coding」需要**重开 unit** 的语义——独立设计问题，超出「让停机可见、可操作」的边界。

原设计的方案 A（参数化报告来源）在类型层面也做不到：`rework.rs:514` `review_findings_summary`、`:526` `review_findings_fix_hints`、`:555` `review_report_evidence_refs` 全部签名绑定 `CodeReviewReport`；且 `InternalPrReview`（`coding_models/review.rs:169-188`）没有 `round` 字段，而 `rework.rs:476-480` 的 summary 兜底正好用 `review_report.round`。真要做需先抽 `{summary, fix_hints, evidence_refs, raw_ref}` 描述结构、由两个来源分别构造——形态接近方案 B，工作量远超本变更。所以原来「倾向 A，分支超过三层改 B」这个判据从一开始就无意义。

**结论**：门禁提供三个动作（重试评审、人工继续、终止）。需返修时的替代路径是终止 attempt 后重新发起——不理想，但比静默停机可见得多，且不引入半成品的阶段回退。

附带收益：`tests/plan_defect_entrypoints.rs:132-137` 断言 human triage 门禁**不含** `send_to_coder`，原本是本变更的冲突项，现在与新设计一致，保留不动。

同时顺带修一个既有 bug：`provider_failure.rs:65-70` 的 else 分支给既有 `internal_review_blocked` 门禁配了 `[retry, send_to_coder, abort]`，但该 `send_to_coder` **必定失败**（`gates.rs:756-766` 要求 `stage == CodeReview && role == CodeReviewer`，不匹配则落到 `gates.rs:654` 的 `send_review_limit_feedback_to_coder`，后者在 `rework.rs:336-346` 同样要求 `stage == CodeReview`）。移除该动作，同步调整 `tests/provider_failure_recovery.rs:590-609`。

### 已关闭的实施期待定项

原 Plan 有三处「实施时再确认」，现已全部查清：

| 原待定项 | 结论 |
|---|---|
| `RunCoderFix` 在该阶段是否已有自动返修路径 | **没有**。`runner.rs:233-238` 是唯一处理点（`current.clone()`）；自动返修只在 Code Review 分支（`runner.rs:491-573` 调 `execute_coder_fix_from_review_outcome`），该函数全仓 3 处调用无 internal review 调用者。落门禁不打断任何既有流程。 |
| runner 是否会推送过期状态 | **不会**。`create_review_blocked_gate` 在 `provider_failure.rs:72-77` 先写 `Blocked` 再建 gate；runner 在 `runner.rs:423-424`、`:761-762` 引擎返回后立即 `get_attempt`；`emit_current_session_state`（`state.rs:293`）自身重新读库。结构上不可能，**不需要改 runner**。 |
| 前端是否有 reason code 硬编码映射需同步 | **不需要**。`web/src/pages/CodingWorkspaceControls.tsx:33-41` 的 `blockedGateDisplayTitle` 只对 testing 阶段查表，其余一律用 `gate.title`；动作按钮由 `:379-389` 直接遍历 `available_actions` 渲染。新增 reason code 无需前端改动。 |

## 实施步骤

### 阶段一：失败测试（工作包 1.1–1.11）

先写测试，此时应全部失败。参照 `coding_workspace_engine/tests/gate_rework.rs` 与 code review 分诊测试的夹具。

**1.1 最高优先**：group final review 给出 `request_changes` + `RunCoderFix` 决策时落地门禁且 attempt 为 blocked。这是用户报告的确切场景。

**1.4、1.5 回归**：通过结论不落门禁、完成路径不变；`StartPlanRepair` 不落门禁、计划修订编排不变。这两条防止修复过度。

**1.7 互斥**：任意一次评审落地的门禁数量不超过一个；必须覆盖「阻塞结论 + actionable finding」这一既可达组合。

**1.9 可达性保护**：断言 `RunCoderFix` + group 场景的分支仍生效，防止后续有人再把它当死代码删掉。

**🔴 1.11 既有测试改写**：三处需要处置，性质不同——

| 位置 | 处置 |
|---|---|
| `:330-474`（verdict=blocked + `verification_incomplete` 时保持 `Running`、无 open gate） | **改写**，与新 spec 冲突 |
| `:310-327` 的 `RetryVerification → None` | **改写** |
| `:310-327` 的 `RunCoderFix(group) → Some("group_final_review_blocked")` | **保留** |
| `:132-137`（human triage 不含 `send_to_coder`） | **保留**（与新设计一致） |

提交建议：`test: 为 internal PR review 停机语义补充失败测试`（允许红灯，作为 TDD 起点）。

### 阶段二：门禁落地（工作包 2.1–2.4）

改 `internal_pr_review.rs`：

1. **保留** `:8-10` 的 `RunCoderFix if is_group_final_review` 分支。
2. 去掉 `:385` 的 `verdict == Blocked` 前置，改为按 `review_flow_decision` 落地。
3. 新增两个 reason code（`internal_review_change_requested`、`internal_review_verification_incomplete`）与标题映射，group final review 标题前缀用 GroupFinalReview。
4. 把落门禁判定收敛为流程决策的单一映射——**不要**保留「verdict 落地路径 + 决策落地路径」两条并存，那才是重复落地的来源。

提交建议：`fix: internal PR review 人工路由决策落地阻塞门禁`。

### 阶段三：动作集合（工作包 2.5、2.6）

改 `provider_failure.rs:45-70`：

- 为四个 reason code 提供重试评审、人工继续、终止三个动作。
- `:37-40` 的 `retry_action` 在 `InternalPrReview` 阶段已正确解析为 `retry_internal_review`，无需改动。
- 移除 `:65-70` else 分支中必定失败的 `send_to_coder`，同步调整 `tests/provider_failure_recovery.rs:590-609`。

提交建议：`feat: 分诊门禁提供可操作动作并移除失效的送回 Coder`。

### 阶段四：残留确认（工作包 2.7、2.8）

- 确认未改动：流程决策判定逻辑、通过结论完成路径、计划修订唤起条件、Code Review 分诊门禁行为、`valid_stage_transition`、`gates.rs` 的 `is_code_review_feedback_gate`、`rework.rs`、`web/coding_ws_handler/runner.rs`。
- 在 design 已知缺口中记录四项独立立项。

提交建议：`chore: 确认 internal PR review 停机语义的边界不变`。

### 阶段五：验证（工作包 3.1–3.3）

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked --lib
cargo test --locked --test it_core
cargo test --locked --test it_web
cargo test --locked --test it_provider
cargo test --locked --test it_product
cargo test --locked --test it_task_run
```

🔴 禁止 `-j 1`。`large_file_guard` 是既有失败基线，需与新增失败明确区分。

`openspec validate open-group-final-review-change-gate --strict`，然后代码审查。

本 change 不改前端（已确认无 reason code 硬编码映射），但仍跑 `pnpm tsc -b` 确认。

### 阶段六：用户验证（工作包 3.4）

经用户确认后重启后端，由用户验证 group final review 要求修改时出现带三个动作的阻塞门禁而非静默停机。

**同时须向用户说明两点**：当前无送回 Coder 通道，需返修时的替代路径是终止 attempt 后重新发起；「人工继续」目前会让 attempt 回到 `Running` 且无 runner（既有缺陷，见已知缺口）。

## 验收对照

| 工作包 | Requirement |
|---|---|
| 1.1–1.5、2.1 | internal PR review 的人工路由决策必须落地阻塞门禁 |
| 1.6、2.2 | 停机原因以互不相同的原因码区分 |
| 1.7、2.3 | 同一次评审结论只落地一个阻塞门禁 |
| 1.8、1.10、2.5、2.6 | 分诊门禁提供可操作动作 |
| 1.9、2.4 | 门禁原因码判定不含不可达分支 |
| 1.11 | 既有断言与新 spec 对齐 |

## 非目标

- 不改 `internal_review_flow_decision` 系列的决策判定逻辑
- 不改 `Approve` 结论的完成路径
- 不改 `StartPlanRepair`、`ContinueAfterApprove` 的既有处理
- 不改 plan repair 唤起条件
- 不改 Code Review 阶段的分诊门禁行为
- 不改评审提示词与 reviewer 判断口径（属 `fix-process-evidence-as-acceptance`）
- 不改阶段跃迁规则（`valid_stage_transition`）
- 不提供 internal PR review 阶段的送回 Coder 能力
- 不修复「人工继续」的空转
- 不修复 `StartStoryAmendment` / `StartDesignAmendment` 的同类停机
- 不移除 `internal_pr_review.rs:8-10` 的 `RunCoderFix if is_group_final_review` 分支

## 已知缺口

按严重程度排列，均需独立立项：

1. 🔴 **「人工继续」空转**：`runner.rs:166-176` 的 `should_resume_runner_after_gate_response` 白名单不含 `manual_continue`；`gates.rs:680-712` 的 ManualContinue 只写审计并把状态置回 `Running`，没有后续编排（group attempt 走向完成只在 `runner.rs:219-227` 的 `ContinueAfterApprove` 分支）。**结果是点击后回到与本变更要修的完全相同的静默停机。** 这是 Code Review 分诊门禁引入时的既存缺陷，本变更照搬会把它复制过来。修复需要设计 group attempt 在非 approve 路径下的完成语义。
2. **internal PR review 无送回 Coder 通道**：需新增阶段回退许可 + group 场景重开 unit 语义。
3. **`StartStoryAmendment` / `StartDesignAmendment` 无编排**（已确认）：两个决策全仓只有 `runner.rs:235-236`（`current.clone()`）与 `runner.rs:610-611`（仅 emit state）两处处理，`plan_defect_routing.rs:287-288` 只有 label。同样静默停机。
4. **reason code 命名家族不齐**：Code Review 侧 `code_review_output_human_triage` 有 `output_`，internal 侧无。

## 风险

1. 🔴 **把可达分支当死代码删掉**：`internal_pr_review.rs:8-10` 在「阻塞结论 + actionable finding」下可达，且有现存单测。1.9 是这条的保护。原 Plan 的 tasks 2.4 要删它，照做会造成行为回退。
2. 🔴 **互斥实现留下两条并存的落地路径**：若同时保留 verdict 落地与决策落地，`Blocked` + actionable 会落两个门禁。收敛为单一映射是唯一可靠做法；1.7 必须覆盖该组合。
3. **修复过度**：`Approve` 与 `StartPlanRepair` 不得落门禁。1.4、1.5 是护栏。
4. **与 `open-code-review-triage-gate` 交叠**：该 change 未归档。共享 `create_review_blocked_gate`，改动需确认不回退其结论。
5. **用户预期落差**：门禁只有三个动作，需返修时必须终止重发。阶段六必须向用户说明，否则会被当成新缺陷。
