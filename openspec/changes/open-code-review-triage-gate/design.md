## Context

`code_review_flow_decision`（`src/product/coding_workspace_engine/plan_defect.rs`）把 Code Review 结论映射为流程决策。其中 `RunCoderFix`、`StartPlanRepair`、`ContinueAfterApprove` 有明确后继动作，而 `StopForHumanTriage`、`RetryVerification`、`OpenOperationalGate` 在 `src/web/coding_ws_handler/runner.rs` 共用同一个「推送会话状态后 return」分支，不落任何门禁、不改 attempt 状态。

真实案例 `coding_attempt_45c6a9317c954be4ac359071b47e39f6`：Reviewer 以 `defect_class=implementation_defect` 携带非空 `plan_defect_evidence`，`validate_plan_defect_finding` 判定契约失败，决策兜底为 `StopForHumanTriage`，流程静默退出。落地状态为 attempt `running` / `code_review`、`coding_node_0005` 为 `failed`、stage-gates 目录无任何人工分诊门禁、`coding_unit_0002` 仍 `running`。

Coding 阶段的同类缺陷已在提交 `e462fbc1` 修复（`open_coding_output_human_triage_gate`），Code Review 阶段未做对应处理。

约束：Testing 与 tester 角色已退出流程，本 change MUST NOT 引入或恢复任何 Testing 相关内容。

## Goals / Non-Goals

**Goals:**

- Code Review 的三个人工路由决策都具备可操作 blocked gate，attempt 不再停留 `running`。
- 三个决策使用可区分的 reason code。
- 单次审查结论只落地一个 blocked gate。
- Reviewer 结构化输出契约明确 implementation defect 的字段边界与证据出口。

**Non-Goals:**

- 不修改 `validate_plan_defect_finding` 的校验判定，不放宽 plan defect 契约。
- 不为 `RetryVerification` 引入自动化验证补跑能力。
- 不引入、不恢复 Testing 或 tester 角色相关流程内容。
- 不改动 coding 阶段 `runner.rs` 中同类静默返回路径（另案处理）。
- 不修改 completion gate、plan repair、internal PR review 的既有判定。
- 不自动迁移、重放或改写历史停滞 attempt。

## Decisions

### 门禁落地在引擎内，而非 runner 内

在 `execute_code_review_with_commands`（`src/product/coding_workspace_engine/code_review.rs`）内复用已计算的流程决策落地门禁。

理由：与 coding 阶段 `open_coding_output_human_triage_gate` 的位置一致（引擎落地状态、runner 只读取并推送），且覆盖所有调用该方法的入口而非仅主 runner 分支。

备选方案：在 `runner.rs:609` 分支内落地。放弃原因是会把状态写入职责下移到 Web 层，并且只覆盖单一调用点。

### 复用 `create_review_blocked_gate`

三个门禁通过既有 `create_review_blocked_gate`（`src/product/coding_workspace_engine/provider_failure.rs`）落地，stage 为 `CodeReview`、role 为 `CodeReviewer`。该函数已按 reason code 分派动作集合，internal review 的 `internal_review_human_triage` / `internal_review_operational_blocker` 已是「重试 + 终止」形态，新增三个 reason code 归入同一形态即可。

reason code 映射：

| 决策 | reason code |
|---|---|
| `StopForHumanTriage` | `code_review_output_human_triage` |
| `RetryVerification` | `code_review_verification_incomplete` |
| `OpenOperationalGate` | `code_review_operational_blocker` |

备选方案：新增独立门禁函数。放弃原因是会复制状态流转与动作装配逻辑。

### 动作集合限定为重试审查与终止

三个门禁均只提供「重试代码审查」与「终止」。

理由：`retry_coding` 在 Code Review 阶段语义混乱——当前 unit 已产出 completion commit，返修应由 `RunCoderFix` 决策经 `execute_coder_fix_from_review_outcome` 走既有路径，不应由门禁旁路触发。`send_to_coder` 同理排除。

备选方案：额外提供 `retry_coding`。放弃原因是引入与返修路径重复的旁路状态机。

### 与既有 `code_review_blocked` 门禁互斥

`verdict=blocked` 且报告无可执行 finding 时，既落入既有 `code_review_blocked` 分支，也会被判为 `StopForHumanTriage`。实现 MUST 保证两者互斥，只落地既有 `code_review_blocked` 门禁。

理由：避免同一次审查产生两个 blocked gate，导致 UI 出现重复门禁与状态歧义。

### Reviewer 契约强化落在 projection 渲染路径

契约文案增补在 `role_structured_output_contract(Reviewer)`（`src/product/work_item_projection/render.rs`），不改 `src/product/coding_workspace_engine/prompts.rs`。

理由：schema v2 运行时优先走 `render_reviewer_unit_run_context`，是本次案例的真实 prompt 路径；`prompts.rs` 属 legacy 回退路径。既有 reviewer 契约段只声明了 `severity`、`file_path`、`line`、`message`、`required_action`、`source_stage`，未提及 `defect_class` 与 plan defect 字段族，Reviewer 从同一 prompt 中拼接的通用 plan defect 契约段获知字段名后误填。

契约同时给出禁令与证据出口，避免 Reviewer 在下一轮改用其他字段重犯或直接丢弃证据。

## Risks / Trade-offs

- [门禁互斥判断遗漏导致重复 gate] → 为 `verdict=blocked` 且无可执行 finding 的组合编写专门回归测试，断言只有一个 blocked gate。
- [`RetryVerification` 无自动补验证能力，用户只能重试审查或终止] → 这是当前流程不含 Testing 的既有事实，门禁描述明确说明停机原因，由用户决策。
- [契约文案变更影响 Reviewer 输出稳定性] → 只增补字段边界与证据出口，不改动既有 verdict/findings 结构约束；以契约渲染测试锁定文案存在。
- [历史停滞 attempt 不会自动恢复] → 部署后通过既有恢复入口重试，或使用新 attempt 验证。

## Migration Plan

- 无数据结构变更，无存储迁移。
- 部署后需重启后端服务方可生效。
- 历史停滞 attempt 不自动迁移；用户可经既有恢复入口重试，或以新 attempt 验证。
- 回滚方式为回退提交并重启服务；回滚后恢复为原静默退出行为，不产生残留数据。

## Open Questions

无。设计边界已确认。
