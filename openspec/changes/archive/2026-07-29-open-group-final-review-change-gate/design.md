## 背景

internal PR review（含 group final review）阶段的停机处理与 Code Review 阶段不对称。

| 维度 | Code Review（已修） | internal PR review（本 change） |
|---|---|---|
| `Blocked` 结论 | 落 `code_review_blocked` 门禁 | 落 `internal_review_blocked` / `group_final_review_blocked` 门禁 |
| `RequestChanges` 结论 | 由分诊门禁覆盖 | **不落任何门禁** |
| 分诊决策落地 | `code_review.rs:222-253`，三个决策各一 reason code | **无** |
| 门禁前置条件 | 不以 verdict 为前置 | `internal_pr_review.rs:385` 以 `verdict == Blocked` 为前置 |
| runner 兜底 | 门禁已落地，读回 blocked 状态 | `runner.rs:233-238` 六个决策统一 `current.clone()` |

## 根因

两处共同导致静默停机：

1. `internal_pr_review.rs:385`：
   ```rust
   let blocked_gate_reason = (review.verdict == ReviewVerdict::Blocked)
       .then(|| internal_review_blocked_gate_reason(review_flow_decision, is_group_final_review))
       .flatten();
   ```
   `RequestChanges` 直接被 `verdict` 前置条件挡掉，`internal_review_blocked_gate_reason` 根本不被调用。

2. `runner.rs:233-238`：六个决策（含 `RunCoderFix`、`StopForHumanTriage`）统一返回 `current.clone()`，不做状态流转。若门禁未落地，attempt 就停在 `running` 且无人推进。

缺口只覆盖 `RequestChanges`，不覆盖 `Blocked`。`internal_pr_review.rs:8-10` 的 `RunCoderFix if is_group_final_review => Some("group_final_review_blocked")` **是可达的、不是死代码**：`plan_defect.rs:215-217` 有

```rust
ReviewVerdict::Blocked if review_findings_have_actionable_findings(findings) => {
    CodeReviewFlowDecision::RunCoderFix
}
```

即「阻塞结论 + 至少一条 actionable finding」（`mod.rs:135-150`：severity 为 error/warning 且有 message）就推出 `RunCoderFix`，此时 `verdict == Blocked` 前置成立，该分支正常生效。这是 reviewer 给阻塞结论时最常见的形态。现存单测 `tests/plan_defect_entrypoints.rs:319-322` 直接断言 `internal_review_blocked_gate_reason(RunCoderFix, true) == Some("group_final_review_blocked")`。

因此本变更**不移除该分支**。缺口的准确边界是：`RequestChanges` 结论（无论决策为何）完全不落门禁，而 `Blocked` 结论按决策落 `internal_review_blocked` / `group_final_review_blocked` / `internal_review_human_triage`。

## 决策

### 决策一：以流程决策而非 verdict 决定是否落门禁

去掉 `verdict == Blocked` 前置条件，改为由 `CodeReviewFlowDecision` 决定。落门禁的决策集合：

| 决策 | 落门禁 | 理由 |
|---|---|---|
| `RunCoderFix` | 是 | 该阶段确无自动返修编排（已确认，见下） |
| `RetryVerification` | 是 | 验证证据不完整，需人工决定重试或继续 |
| `StopForHumanTriage` | 是 | 定义即人工分诊 |
| `OpenOperationalGate` | 是 | 运维阻塞，需人工处置 |
| `StartPlanRepair` | 否 | 已有 `start_plan_repair_from_internal_review` 编排 |
| `StartStoryAmendment` / `StartDesignAmendment` | 否 | 需实测确认是否有编排；若无，属独立缺陷，不在本 change 扩大范围 |
| `ContinueAfterApprove` | 否 | 已有完成编排 |

**`RunCoderFix` 在该阶段无自动返修编排（已确认）**：`runner.rs:233-238` 是该决策在 internal review 后的唯一处理点，直接 `current.clone()`。自动返修只存在于 Code Review 分支（`runner.rs:491-573` 调 `execute_coder_fix_from_review_outcome`），而 `execute_coder_fix_from_review_outcome` 全仓仅 3 处调用（`rework.rs:14`、`runner.rs:505`、plan_repair 测试），无 internal review 调用者；`rework.rs:426` 的阶段前置也把手动通道挡在 CodeReview。因此落门禁不会打断任何既有自动流程。

**`StartStoryAmendment` / `StartDesignAmendment` 也无编排（已确认）**：两个决策在全仓只有 `runner.rs:235-236`（internal review，`current.clone()`）与 `runner.rs:610-611`（code review，仅 emit state）两处处理，`plan_defect_routing.rs:287-288` 只有 label，没有任何编排入口。即它们同样静默停机。本 change 只记录为已知缺口，不修——正确处置是唤起 story / design 修订流程，不是落分诊门禁。

### 决策二：四个 reason code 互不相同

沿用 Code Review 的命名形态：

| 决策 | reason code | 门禁标题 |
|---|---|---|
| `RunCoderFix` | `internal_review_change_requested` | internal PR review 要求修改 |
| `RetryVerification` | `internal_review_verification_incomplete` | internal PR review 验证证据不完整 |
| `StopForHumanTriage` | `internal_review_human_triage` | internal PR review 需人工分诊 |
| `OpenOperationalGate` | `internal_review_operational_blocker` | internal PR review 命中运维阻塞 |

group final review 使用同一组 reason code，标题前缀改为 GroupFinalReview。后两个 reason code 已存在（`internal_pr_review.rs:12-13`），语义不变，只是不再受 `verdict == Blocked` 限制。

命名一致性说明：Code Review 侧是 `code_review_output_human_triage`（`gates.rs:760-765`），internal 侧是 `internal_review_human_triage`，少了 `output_`。这是既有的家族不齐，本变更不改名（改名会牵动既有测试与前端查表），只保证四个 reason code 互不相同。

### 决策三：门禁互斥必须新建判定结构，不能照搬

原判断「`Blocked` 结论已由既有分支落门禁，新分诊门禁在该情形下跳过」**前提不成立**——internal review 里没有这个「既有分支」。`internal_pr_review.rs:385-414` 只有一条落门禁路径，正是本次要改的那条。

实际现状：

| 结论 | 决策 | 当前落地 |
|---|---|---|
| `Blocked` + 非 actionable | `StopForHumanTriage` | `internal_review_human_triage` |
| `Blocked` + actionable | `RunCoderFix` | `internal_review_blocked` / `group_final_review_blocked` |
| `RequestChanges` | 任意 | **无** |

Code Review 侧能做互斥，是因为 `code_review.rs:202-203` 有一条独立的字面量 `code_review_blocked` 落地分支（条件 `Blocked && !actionable`）：

```rust
let lands_code_review_blocked = report.verdict == ReviewVerdict::Blocked
    && !code_review_report_has_actionable_findings(&report);
```

internal review 没有对等物，因此 `lands_code_review_blocked` 的形式可参考、语义不可照搬。

本变更的做法：把落门禁判定**统一收敛到流程决策的单一映射**，一个决策对应一个 reason code，天然互斥，不需要额外的排除条件。`Blocked` 结论不再单独走一条落地路径——它推出的决策已经落在四类之中（actionable → `RunCoderFix`，非 actionable → `StopForHumanTriage`），由同一映射处理。这样既消除重复落地的可能，也不必回答「结论本身是 Blocked 该落哪个 gate」这个在原设计里悬空的问题。

`group_final_review_blocked` 保留为 group 场景下 `RunCoderFix` 的 reason code（即现状），不新增含义。

### 决策四：送回 Coder 移出本变更范围

原计划让分诊门禁提供「送回 Coder 返修」动作。**该动作在 internal PR review 阶段存在结构性阻断，不是放宽前置条件能解决的**，因此从本变更移出。

两层阻断：

1. **阶段机不允许回退。** `coding_attempt_store/attempt.rs:696-710` 的 `valid_stage_transition` 只白名单了 `CodeReview → Coding`，其余要求 `next.order() >= current.order()`。`InternalPrReview` order=6、`Coding` order=2，所以 `rework.rs:499-504` 的 `update_attempt_stage(..., Coding)` 会直接报 `invalid_coding_attempt_stage_transition`。需要新增一条阶段回退许可。
2. **group 场景没有可返修的 unit。** group final review 时全部 unit 已 Completed、`active_unit_id` 被清空（`coding_attempt_store/group.rs:358-367`），`group_validation.rs:330-352` 只在 `stage.order() >= ReviewRequest` 时才允许空指针。一旦把 stage 退回 `Coding`，`plan_defect.rs:530-537` 的 `get_active_coding_unit` 返回 None，`execute_coding_with_commands_outcome → render_coder_unit_run_context` 会以 `unit_run_projection_binding_missing: active unit` 失败。

也就是说用户报告的正是 group final review 场景，而该场景下「回到 Coding」需要**重开 unit** 的语义——这是独立的设计问题，超出「让停机可见、可操作」的变更边界。

顺带记录：原设计的方案 A（参数化报告来源）在类型层面也做不到。`rework.rs:514` `review_findings_summary`、`:526` `review_findings_fix_hints`、`:555` `review_report_evidence_refs` 全部签名绑定 `CodeReviewReport`；且 `InternalPrReview`（`coding_models/review.rs:169-188`）没有 `round` 字段，而 `rework.rs:476-480` 的 summary 兜底正好用 `review_report.round`。真要做需先抽一个 `{summary, fix_hints, evidence_refs, raw_ref}` 描述结构、由两个来源分别构造——形态接近方案 B，且工作量远超本变更。

因此本变更的门禁提供三个动作：重试评审、人工继续、终止。用户在 group final review 需要返修时，路径是终止 attempt 后重新发起——不理想，但比静默停机可见得多，且不引入半成品的阶段回退。送回 Coder 记入已知缺口。

相应地，`gates.rs:756` 的 `is_code_review_feedback_gate` **不需要扩展**。同时注意 `provider_failure.rs:65-70` 的 else 分支目前给既有 `internal_review_blocked` 门禁配了 `[retry, send_to_coder, abort]`，而该 `send_to_coder` 必定失败（`gates.rs:756-766` 要求 `stage == CodeReview && role == CodeReviewer`，不匹配则落到 `gates.rs:654` 的 `send_review_limit_feedback_to_coder`，后者在 `rework.rs:336-346` 同样要求 `stage == CodeReview` → 必定 `send_to_coder_not_available`）。这是既有 bug，本变更顺带移除该动作即可，不新增能力。

### 决策五：runner 状态读回无需改动（已确认）

`create_review_blocked_gate` 在 `provider_failure.rs:72-77` 先写 `Blocked` 再建 gate；runner 在 `runner.rs:423-424` 与 `:761-762` 引擎调用返回后立即 `get_attempt`，且 `emit_current_session_state`（`web/coding_ws_handler/state.rs:293`）自身会重新读库。推送过期状态在结构上不可能。

原设计把这条列为「需实测确认」的风险，实际静态可判定，**不需要改动 runner，也不需要在实施时验证**。

### 决策六：不改判定逻辑

`internal_review_flow_decision` 与 `internal_review_flow_decision_with_bindings` 的决策推导保持不变。本 change 只处理"决策已产生但无落地"的缺口。

reviewer 判断口径的问题（要求红灯提交、把进程证据当验收）属 `fix-process-evidence-as-acceptance`，不在此处理。

### 决策七：与既有测试的语义冲突需按新 spec 改写

三处既有断言与本变更正面冲突，属语义分歧而非夹具调整，必须显式改写：

| 位置 | 现有断言 | 冲突 |
|---|---|---|
| `tests/plan_defect_entrypoints.rs:132-137` | human triage 门禁**不含** `send_to_coder` | 决策四已把 send_to_coder 移出范围，该断言**与新设计一致**，保留 |
| `tests/plan_defect_entrypoints.rs:330-474` | verdict=blocked + `verification_incomplete` 时 attempt 保持 `Running`、无 open gate | 新 spec 要求 `RetryVerification` 落门禁并置 blocked，必须改写 |
| `tests/plan_defect_entrypoints.rs:310-327` | `RetryVerification → None`；`RunCoderFix(group) → Some("group_final_review_blocked")` | 前者必须改写；**后者保留**（该分支可达，见根因） |

其中 `:132-137` 原本是本变更的冲突项，因决策四把 send_to_coder 移出范围而自动消解——这也是移出范围的一个附带收益。

### 决策八：「人工继续」的空转是既有缺陷，不在本变更修复

`runner.rs:166-176` 的 `should_resume_runner_after_gate_response` 白名单不含 `manual_continue`；`gates.rs:680-712` 的 ManualContinue 只写审计并把状态置回 `Running`，没有后续编排。group attempt 走向完成只在 `runner.rs:219-227` 的 `ContinueAfterApprove` 分支。

结果是用户点「人工继续」后 attempt 变 `Running` 且无 runner——回到与本变更要修的完全相同的静默停机。

这是 Code Review 分诊门禁上的既存缺陷（`open-code-review-triage-gate` 引入时未覆盖），本变更照搬会把它复制过来。裁定：**记入已知缺口，单独立项修复**，不在本变更内扩大范围。本变更的 spec 因此只要求门禁提供该动作，不要求「继续之后流程走到哪」——后者需要设计 group attempt 在非 approve 路径下的完成语义。

## 边界

- 不改 `internal_review_flow_decision` 系列的决策判定逻辑。
- 不改 `Approve` 结论的完成路径。
- 不改 `StartPlanRepair`、`ContinueAfterApprove` 的既有处理。
- 不改 plan repair 唤起条件；分诊门禁的任何动作都不触发 plan repair。
- 不改 Code Review 阶段的分诊门禁行为。
- 不改评审提示词与 reviewer 判断口径。
- 不改阶段跃迁规则（`valid_stage_transition`）。
- 不提供 internal PR review 阶段的送回 Coder 能力（见决策四）。
- 不修复「人工继续」的空转（见决策八）。
- 不修复 `StartStoryAmendment` / `StartDesignAmendment` 的同类停机。
- 不移除 `internal_pr_review.rs:8-10` 的 `RunCoderFix if is_group_final_review` 分支（该分支可达）。

## 已知缺口

按严重程度排列，均为独立立项：

1. **「人工继续」空转**（决策八）：`manual_continue` 不在 runner 恢复白名单，点击后 attempt 变 `Running` 且无 runner，与本变更要修的停机同形。这是 Code Review 分诊门禁的既存缺陷，本变更不复制修复。修复需要设计 group attempt 在非 approve 路径下的完成语义。
2. **internal PR review 无送回 Coder 通道**（决策四）：需要新增阶段回退许可 + group 场景重开 unit 的语义。用户当前的替代路径是终止 attempt 后重新发起。
3. **`StartStoryAmendment` / `StartDesignAmendment` 无编排**（已确认）：两个决策在全仓只有 `current.clone()` 与 emit state 两处处理，`plan_defect_routing.rs:287-288` 只有 label。同样静默停机，正确处置是唤起对应修订流程。
4. **既有 `internal_review_blocked` 门禁的 `send_to_coder` 必定失败**：`provider_failure.rs:65-70` 提供该动作，但 `gates.rs:756-766` 与 `rework.rs:336-346` 的阶段前置都不匹配，必然返回 `send_to_coder_not_available`。本变更顺带移除该动作。
5. **reason code 命名家族不齐**：Code Review 侧 `code_review_output_human_triage` 有 `output_`，internal 侧无。不改名以免牵动既有测试与前端查表。
