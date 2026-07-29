## 1. 失败测试

- [x] 1.1 为 `reviewer_process_evidence_boundary_contract`（新增）编写内容断言：包含过程事实判定标准两条，包含不得创建 finding、不得作为 verdict/summary 否决理由、不得成为 Coder required_action 的表述。
- [x] 1.2 编写断言：即使上游材料提到过程性要求，也不得转换为 finding 或返修要求。
- [x] 1.3 编写断言：`code_review_material_protocol` 包含过程证据边界。
- [x] 1.4 编写断言：`group_final_review_material_protocol` 包含过程证据边界。
- [x] 1.4a 🔴 编写断言：**Code Review 的 projection 渲染路径**产出的提示词包含过程证据边界。这是最常触发本缺陷的路径——`code_review.rs:68-74` 在有 unit run projection 时完全走 `render.rs`，不经 `build_code_review_prompt`，因此 1.3 的断言在该路径上不成立。
- [x] 1.4b 编写断言：`prompts.rs:121-123` 的 `build_internal_pr_review_prompt`（非 group scope 分支）产出的提示词包含过程证据边界。该路径目前连 `reviewer_test_scope_contract` 都没有。
- [x] 1.5 编写断言：reviewer 侧提示词说明 `non_zero_test_execution` 的语义为当前执行结果，不表达提交顺序或时序。
- [x] 1.6 编写断言：Code Review 与 GroupFinalReview 提示词仍保留「required 验证命令缺少执行证据必须记 finding」与「测试输出显示没有实际测试被执行不得视为有效覆盖」的既有要求。
- [x] 1.7 编写 splitter prompt 断言：`acceptance_criteria` 约束要求 criterion 描述可观测结果状态，禁止过程事实。
- [x] 1.8 编写 splitter prompt 断言：说明 `non_zero_test_execution` 语义，措辞与 reviewer 侧一致。
- [x] 1.9 编写 canonical 校验用例：含过程性 acceptance criterion 的契约产出指向该 `criterion_id` 的 finding，severity 为 `Warning`。
- [x] 1.10 编写 canonical 校验用例：合法的结果状态类 criterion（含 `required_evidence` 为 `non_zero_test_execution` 的条目）不产出该 finding。可直接用 `work_item_contract/tests.rs:49-53` 的 `AC-001` 夹具（`required_evidence: vec![SourceDiff, NonZeroTestExecution]`，statement 为「Contract survives serde roundtrip」）。
- [x] 1.10a 编写用例：仅含该 finding 的候选仍可接受（`has_errors()` 为 false、`can_accept` 为 true），且 finding 出现在候选的 `validator_findings` 中。
- [x] 1.11 编写断言：Coder 执行协议与增量执行协议的 TDD 与测试要求未被削弱。
- [x] 1.12 确认以上测试全部失败且失败原因是缺少实现，不是断言写错。

## 2. 实现

- [x] 2.1 在 `coding_workspace_engine/prompts.rs` 新增 `reviewer_process_evidence_boundary_contract()`，与 `reviewer_test_scope_contract()` 并列，按其表述模式给出过程证据边界与典型例子。
- [x] 2.2 在 `code_review_material_protocol` 组装处引入新契约（`prompts.rs:52-55` 区域，参照 `:54` 引入 `reviewer_test_scope_contract` 的方式；format 占位在 `:30-33`，加一个 `{}` 与一个参数）。
- [x] 2.3 在 `internal_pr_review.rs:119-122` 的 GroupFinalReview（group scope）prompt 组装处引入新契约；format 占位在 **`internal_pr_review.rs:96-99`**（不是 `prompts.rs:97-99`，那是另一个 builder）。
- [x] 2.3a 🔴 在 `render.rs` 的 reviewer 渲染路径注入新契约。这是 Code Review 的主路径（`code_review.rs:68-74` 有 projection 时完全走它），不注入则本变更在该阶段不生效。建议作为一个新 section 而非塞进 `role_structured_output_contract`（`render.rs:471-484`）——后者命名是讲输出格式的。
- [x] 2.3b 在 `prompts.rs:121-123` 的 `build_internal_pr_review_prompt`（非 group scope 分支）引入新契约与 `reviewer_test_scope_contract`（该路径目前两者都没有）；同步调整 `prompts.rs:97-100` 的占位。
- [x] 2.4 在 reviewer 材料协议中加入 `EvidenceKind` 四种 kind 的语义说明。`non_zero_test_execution` 有实质依据（对应 `VerificationCheck.non_zero_test_execution_required`）；`manual_check` 与 `handoff_field` 的语义**由本变更新建立**（枚举是裸的，无 doc、无行为依赖），措辞须保持在「可观测性」这一共同性质上，不对具体用法过度承诺。
- [x] 2.5 在 `work_item_split_engine/prompts.rs` 的 `[canonical_field_contract]` 中为 `acceptance_criteria` 补充可观测结果状态约束与 `non_zero_test_execution` 语义说明。
- [x] 2.6 在同文件 `[hard_rules]` 中加入禁止把过程事实写成 acceptance criterion 的硬规则。
- [x] 2.7 在 `work_item_contract/validation.rs` 新增过程性 acceptance criterion 检出，产出指向 `criterion_id` 的 finding，severity 为 `Warning`（既有 `blank_` / `duplicate_acceptance_criterion_id` 都是 `Error`，本项**有意不一致**，理由见 design 决策五）。`ContractFindingSeverity::Warning` 在 `src/` 非测试代码零构造点，需新增一个与 `error_finding`（`:420-435`）对称的构造函数。匹配集合保守：要求同时命中「提交/commit」类词与「先失败/red/顺序/时序」类词，不得单一关键词即报。
- [x] 2.8 确认不新增 `EvidenceKind` 成员、不修改 `AcceptanceCriterion` / `ReviewerRequirementCheck` / `VerificationCheck` 结构、不修改 `reviewer.rs` 的 `failure_route` 推导。
- [x] 2.9 确认未削弱 Coder 侧协议与仓库 TDD 规则；不添加豁免名单或宽限期。

## 3. 验证

- [x] 3.1 `cargo fmt --check`
- [x] 3.2 `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [x] 3.3 `cargo test --locked --lib`
- [x] 3.4 `cargo test --locked --test it_product`
- [x] 3.5 全量确认：仓库内不存在把提交历史或提交顺序作为 reviewer 否决依据的提示词表述；且四条 reviewer 提示词构造路径都含过程证据边界。
- [x] 3.5a 确认 `render.rs` 的 mandatory section 集合完整、reviewer projection 渲染不失败。若 `remove-testing-stage` 已实施，`test_evidence_refs` 与 `ReviewExecutionEvidence` section（`render.rs:449`、`:80`）已变化，需在变化后的结构上确认。
- [ ] 3.6 用户实跑验证：确认 reviewer 不再以缺少 TDD red commit 为由否决实现。
