## Why

reviewer 把「过程事实」当成「验收标准」否决实现：典型现象是以「git 历史里没有 TDD red commit」「没有先失败后通过的提交序列」为由给出 `request_changes`，即使功能实现正确、验证命令有真实非零测试执行证据。这类否决不可修复——历史提交序列在实现完成后无法追补，Coder 返修也无法产出该证据，于是流程进入无出路的返修循环。

**根因是验收标准的内容完全由 splitter provider 自由撰写，并原样注入 reviewer prompt。**

- `src/product/work_item_split_engine/prompts.rs:651` 规定 `acceptance_criteria` 为 `[obj{criterion_id: str+, statement: string, required_evidence: [source_diff|non_zero_test_execution|manual_check|handoff_field]}]`。`criterion_id` 与 `statement` 都是自由文本，平台不约束其语义，因此 provider 可以写出 `ac_tdd_red_evidence`、`statement=存在先失败的测试提交` 这类过程性条目。
- `src/product/work_item_contract/validation.rs:86-95、216-225、237-241` 只校验 `criterion_id` 非空、不重复、被 `done_when_refs` / `reviewer_check_refs` 正确引用。没有任何校验区分「可验证的结果状态」与「不可追补的过程事实」。
- `src/product/work_item_projection/reviewer.rs:11-27` 把每条 criterion 编译成 `ReviewerRequirementCheck`，`required_evidence` 原样克隆，`failure_route` 一律 `BlockerRoute::CoderRework`。
- 注入 reviewer prompt 有两条独立路径：`criterion_id` 经 `work_item_projection/render.rs:379-383` 的 typed section 进入，且该 section 在 `REVIEWER_MANDATORY_SECTIONS` 中（`render.rs:72-81`），**无法被截断规则去掉**；`statement` 经 `coding_evaluation_context/builder.rs:230-233` 把整个 canonical contract JSON 放进 `EvaluationContextPack.work_item.raw_markdown_or_sections` 进入。（`ReviewerRequirementCheck` 本身没有 `statement` 字段。）

**`EvidenceKind::NonZeroTestExecution` 的语义被误读。** `src/product/work_item_contract/model.rs:111-116` 的四种 kind 中，`NonZeroTestExecution` 与 `VerificationCheck.non_zero_test_execution_required`（`model.rs:69`）对应，语义是「测试命令实际执行了非零数量的测试」，即一个**当前可观测的执行结果**。它不表达「测试曾经先失败过」，也不表达任何提交顺序。但 reviewer prompt 里没有任何一句说明这一点，reviewer 因此把它读成完整的 TDD 纪律要求。

**reviewer 协议没有过程证据边界。** `src/product/coding_workspace_engine/prompts.rs:317-340` 的 `code_review_material_protocol` 与 `:342-361` 的 `group_final_review_material_protocol` 都要求「审查清单必须覆盖 TDD/测试要求」，且规定「缺少 required 验证命令的执行证据必须作为 finding，必要时 request_changes 或 blocked」。二者都没有说明哪些「TDD 要求」在只读审查阶段属于可否决证据、哪些属于不可追补的过程事实。同一文件 `:269-276` 的 `reviewer_test_scope_contract` 已经为 E2E/浏览器环境建立了这类「不得成为 finding」边界，说明该模式在本仓库已有先例，只是没有覆盖过程证据。

## What Changes

- 明确 `EvidenceKind` 四种 kind 的语义边界，并把 `NonZeroTestExecution` 的语义（当前执行结果，非提交历史）写入 reviewer prompt 与 splitter prompt。
- 在 reviewer 协议中加入过程证据边界：red commit、提交顺序、开发时序、分支操作历史等不可从当前 diff 与验证输出观测、且实现完成后不可追补的过程事实，不得成为 finding、不得作为 `request_changes` 或 `blocked` 的否决理由、不得成为 Coder `required_action`。
- 该边界必须覆盖**四条** reviewer 提示词构造路径，而非两个协议函数：Code Review 的 projection 渲染路径（`code_review.rs:68-74` 在有 unit run projection 时完全走 `render.rs`，**这是最常触发本缺陷的主路径**）、Code Review 的传统提示词（`prompts.rs:52-55`）、GroupFinalReview 的 group 分支（`internal_pr_review.rs:119-122`）、GroupFinalReview 的非 group 分支（`prompts.rs:121-123`）。只改两个协议函数会让本变更在主路径上完全不生效。
- 在 splitter prompt 中禁止把过程事实写成 acceptance criterion：acceptance criterion 必须描述可从最终代码状态、验证命令输出或 handoff 字段观测的结果状态。
- 在 canonical contract 校验中加入过程性 acceptance criterion 的检出，作为 `Warning` 级 finding：可见于候选的校验结论，但不阻断候选。既有 acceptance criterion finding 都是 `Error`，本项有意不一致——检出只能做关键词匹配，误报不应让合法候选被整体拒绝。
- 不改变 `EvidenceKind` 枚举成员本身。
- 不改变 `AcceptanceCriterion`、`ReviewerRequirementCheck`、`VerificationCheck` 的结构。
- 不改变 `reviewer_check_refs` 与 acceptance criterion ID 集合必须完全一致的既有约束。
- 不改变 TDD 在 Coder 侧的要求：`coding_execution_protocol`（`prompts.rs:278-290`）仍要求写代码前调用 `test-driven-development`。本变更只约束 reviewer 能以什么为由否决。
- 不为历史持久化数据提供迁移或兼容层：按全新系统处置。

## Capabilities

### New Capabilities

- `process-evidence-acceptance-boundary`: 过程事实与验收标准的边界，包括 `EvidenceKind` 语义口径、reviewer 不可否决的过程证据类别、acceptance criterion 必须为可观测结果状态的约束，以及过程性 criterion 的检出与路由。

### Modified Capabilities

（无。现有 specs 未覆盖验收标准的语义边界。）

## Impact

- `src/product/coding_workspace_engine/prompts.rs`：新增过程证据边界契约；`code_review_material_protocol` 与 `group_final_review_material_protocol` 引入该契约；`build_internal_pr_review_prompt`（非 group 分支）补入该契约与 `reviewer_test_scope_contract`；`EvidenceKind` 语义说明写入 reviewer 材料协议。
- `src/product/work_item_projection/render.rs`：reviewer 渲染路径注入过程证据边界（Code Review 主路径）。
- `src/product/coding_workspace_engine/internal_pr_review.rs`：group 分支引入该契约。
- `src/product/work_item_split_engine/prompts.rs`：`[canonical_field_contract]` 与 `[hard_rules]` 增加 acceptance criterion 必须为可观测结果状态的约束，并说明 `non_zero_test_execution` 的语义。
- `src/product/work_item_contract/validation.rs`：新增过程性 acceptance criterion 的检出与 finding。
- `src/product/coding_workspace_engine/tests/parser_prompt.rs`：新增 prompt 内容断言。
- `src/product/work_item_split_engine/tests/prompt_contract.rs`：新增 splitter prompt 断言。
- `src/product/work_item_contract/tests.rs`：新增校验用例。
- 受影响的用户可见行为：reviewer 不再以「缺少 TDD red commit」「提交顺序不对」为由否决；splitter 生成的验收标准不再包含不可追补的过程条目。
- 不影响 Coder 侧 TDD 要求与验证命令执行要求。

## 依赖与顺序

本 change 与 `remove-work-item-handoff`、`remove-testing-stage`、`open-group-final-review-change-gate` 独立。

与 `remove-testing-stage` 的真实交集在 `render.rs`，不在 `prompts.rs`：两个协议函数（`prompts.rs:317-340`、`:342-361`）没有任何 Testing / TestingReport / tester 字样，而 `remove-testing-stage` 要移除的 `test_evidence_refs` 经 `render.rs:449` 的 `ReviewExecutionEvidence` section 进 reviewer prompt 且是 mandatory section（`render.rs:80`）——与本 change 要注入契约的是同一条渲染路径。顺序无强制要求，但谁后做谁负责确认 mandatory section 集合完整、渲染不失败。

`open-group-final-review-change-gate` 明确把「评审提示词与 reviewer 判断口径」划归本 change，两者不重叠。
