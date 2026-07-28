## Why

Code Reviewer 的审查结论落到非 `RunCoderFix` 的人工路由决策时，Coding Workspace 流程会静默退出：attempt 停在 `running` / `code_review`，既没有 blocked gate，也没有任何可操作入口，用户只看到「代码审查失败」但无法继续或终止。真实案例 `coding_attempt_45c6a9317c954be4ac359071b47e39f6` 就停在该状态，当前 work item 的 coding unit 永久停留 `running`。

## What Changes

- Code Review 阶段在 `StopForHumanTriage`、`RetryVerification`、`OpenOperationalGate` 三个流程决策下必须落地 blocked gate，并把 attempt 从 `running` 置为 `blocked`，提供「送回 Coder 返修」「重试代码审查」「人工继续」「终止」四个动作。
- 三个决策使用互不相同的 reason code，便于 UI 与后续排查区分停机原因。
- 放宽手动送回 Coder 通道的前置条件，使其在分诊门禁下支持 `verdict=request_changes` 的审查结论，让「审查有问题回到 Coder」在人工分诊时可用。
- 保持与既有 `code_review_blocked` 门禁互斥：同一次审查结论只允许落地一个 blocked gate。
- Reviewer 结构化输出契约补充 implementation defect 的字段边界：禁止在 `defect_class=implementation_defect` 的 finding 上填写 plan defect 路由字段，并明确该类 finding 的证据出口。
- 不改变 plan defect finding 的校验判定逻辑，不放宽既有契约。
- 不改变 plan repair 的唤起条件，分诊门禁的任何动作都不触发 plan repair。
- 不引入、不恢复任何 Testing 或 tester 角色相关流程内容。

## Capabilities

### New Capabilities
- `coding-code-review-triage`: Code Review 阶段人工路由决策的停机语义，包括 blocked gate 落地、attempt 状态流转、reason code 区分、门禁互斥，以及 Reviewer implementation defect 输出契约边界。

### Modified Capabilities

（无。现有 specs 未覆盖 Code Review 阶段停机语义。）

## Impact

- `src/product/coding_workspace_engine/code_review.rs`：审查结论后的门禁落地路径。
- `src/product/coding_workspace_engine/gates.rs`：`SendToCoder` 动作在分诊门禁下的分派条件。
- `src/product/coding_workspace_engine/rework.rs`：手动送回 Coder 的审查结论前置条件。
- `src/product/work_item_projection/render.rs`：Reviewer 结构化输出契约文案。
- `src/web/coding_ws_handler/runner.rs`：Code Review 决策分支的状态推送行为（读取新状态，不新增旁路）。
- 受影响的用户可见行为：Code Review 停机后 UI 具备可操作门禁，且可将审查反馈送回 Coder 返修；已停滞的历史 attempt 不自动迁移。
- 不影响 completion gate、plan repair 唤起条件、internal PR review 既有判定。
