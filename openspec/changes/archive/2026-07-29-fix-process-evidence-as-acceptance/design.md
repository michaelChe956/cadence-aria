## 背景

问题不是「reviewer 判断错了」，而是**平台把不可判定的东西交给 reviewer 判定**。语义漂移链路：

| 环节 | 位置 | 平台约束 | 漂移空间 |
|---|---|---|---|
| splitter 生成验收标准 | `work_item_split_engine/prompts.rs:651` | `criterion_id` 非空、`required_evidence` 取四种 kind | `criterion_id` 与 `statement` 是自由文本，语义无约束 |
| canonical 校验 | `work_item_contract/validation.rs:86-95、216-225` | 非空、不重复、引用完整 | 不区分结果状态与过程事实 |
| reviewer projection 编译 | `work_item_projection/reviewer.rs:11-27` | 原样克隆 `required_evidence`，`failure_route` 恒为 `CoderRework` | 过程性 criterion 也被标成可返修 |
| `criterion_id` 注入 prompt | `work_item_projection/render.rs:379-383` | 序列化 `criterion_refs` 与 `requirement_matrix` | provider 撰写的 ID 原样进入，且是 **mandatory section**（`render.rs:72-81`），无法被截断规则去掉 |
| `statement` 注入 prompt | `coding_evaluation_context/builder.rs:230-233` | 整个 canonical contract JSON pretty-print 进 `EvaluationContextPack.work_item.raw_markdown_or_sections` | provider 撰写的 statement 原样进入 |
| reviewer 判断 | `coding_workspace_engine/prompts.rs:317-340、342-361` | 要求覆盖「TDD/测试要求」，缺证据必须记 finding | 没有过程证据边界 |

各环节都没有把关，reviewer 是链路末端的受害者：它收到一条写着「存在先失败的测试提交」的验收标准，标注 `failure_route=coder_rework`，协议又要求「缺少 required 证据必须记 finding、必要时 request_changes」。按协议行事的结果就是不可修复的否决。

**注意注入点是两条独立路径，不是一条。** `ReviewerRequirementCheck`（`work_item_projection/model.rs:68-73`）只有 `criterion_id`、`requirement_refs`、`required_evidence`、`failure_route`——**没有 `statement` 字段**。所以 reviewer projection 给 reviewer 的只有 criterion ID 字符串加 evidence kind 枚举；statement 走的是 `EvaluationContextPack` 那条路。

这让论证更强而非更弱：`AcceptanceCriteriaRequirementMatrix` 在 `REVIEWER_MANDATORY_SECTIONS` 里（`render.rs:72-81`），意味着 `ac_tdd_red_evidence` 这类 ID 是**强制注入、无法被截断规则去掉**的。

## 决策一：把边界写在 prompt 与校验两处，不改数据结构

`AcceptanceCriterion`、`ReviewerRequirementCheck`、`EvidenceKind` 的结构本身没有缺陷。`EvidenceKind` 的四种 kind 恰好都是可观测的：`SourceDiff`（最终代码状态）、`NonZeroTestExecution`（验证命令执行结果）、`ManualCheck`（人工检查结果）、`HandoffField`（交接字段存在性）。**没有一种 kind 表达提交历史或时序。**

缺陷在于这个「都可观测」的性质从未被说明，也从未被强制。因此：

- 不新增 kind、不新增字段、不新增标记位。
- 在 splitter prompt 里禁止生成过程性 criterion（源头拦截）。
- 在 canonical 校验里检出过程性 criterion（兜底检出）。
- 在 reviewer 协议里声明过程事实不可否决（末端兜底）。

三层都做，因为任何单层都不足：prompt 约束是软的、provider 可以违背；校验是启发式的、可能漏检；reviewer 边界只在漏检发生后起作用。

## 决策二：过程证据的判定标准是「可观测性 + 可追补性」

不列举「TDD」「red commit」这类词就完事——列举会漏。判定标准两条，同时满足即为过程事实：

1. **不可从当前证据观测**：无法从最终 diff、验证命令输出、handoff 字段或人工检查结果中读出。
2. **实现完成后不可追补**：即使 Coder 返修，也无法产出该证据。

据此，以下属于过程事实：red commit 的存在、失败→通过的提交序列、开发时序、提交拆分粒度、分支创建与 rebase 历史、Coder 会话内的操作顺序。

以下**不属于**过程事实，仍必须审查：测试文件是否存在、测试是否覆盖需求场景、验证命令是否真实执行且非零、测试输出是否与实现自相矛盾、Forbidden Write Scopes 是否被越过。

这条区分必须在 prompt 里以标准形式给出，而不是给一份词表。同时给若干典型例子，帮助 provider 落地判定。

## 决策三：沿用 `reviewer_test_scope_contract` 的表述模式

`prompts.rs:269-276` 的 E2E 边界已经确立了一套完整表述，且经过实战：

- 明确不得创建以某类目标为目的的 finding；
- 明确该类事实的缺失、失败或缺少证据均不得成为 finding；
- 明确不得作为 verdict 或 summary 中的否决理由；
- 明确不得成为 Coder `required_action` 或任何返修要求；
- 明确即使 Work Item、Design Spec、Verification Plan、handoff 或 EvaluationContextPack 提到该类事实，也不得转换成上述任何一种。

最后一条是关键：它处理的正是「上游材料里写了、reviewer 该不该听」的冲突。过程证据边界必须包含等价条款，否则漏检的过程性 criterion 仍会被 reviewer 采纳。

新增 `reviewer_process_evidence_boundary_contract()` 与之并列，而不是塞进 `reviewer_test_scope_contract`——两者约束的是不同类别，混在一起会让后续修改互相牵连。

## 🔴 决策四：必须覆盖三条 prompt 构造路径，不是两条

原设计只考虑 `code_review_material_protocol`（`prompts.rs:317`）与 `group_final_review_material_protocol`（`:342`）两个函数，照 `reviewer_test_scope_contract` 的方式引入两处（`prompts.rs:54`、`internal_pr_review.rs:120`）。

**这样做在 Code Review 阶段完全不生效**，而那正是本缺陷最常触发的阶段。`code_review.rs:68-74`：

```rust
let prompt = match self.render_reviewer_unit_run_context(&attempt, &reviewer)? {
    Some(rendered) => rendered.text,
    None => { self.build_code_review_prompt(...).await? }
};
```

`render_reviewer_unit_run_context`（`plan_defect.rs:413-455`）在 `attempt.scope == WorkItemGroup` 且有 active unit + projection bundle 时返回 `Some`。此时 prompt **完全**是 `render.rs` 的 projection 文本，`code_review_material_protocol()`、`reviewer_test_scope_contract()`、`no_default_stack_assumption_contract()` 全部不出现。projection 渲染路径唯一的 reviewer 侧行为契约是 `render.rs:471-484` 的 `role_structured_output_contract`，只讲 JSON 格式。

也就是说三层拦截的第三层在主路径上是空的。

三条路径都必须覆盖：

| 路径 | 构造位置 | 触发条件 | 当前是否含 `reviewer_test_scope_contract` |
|---|---|---|---|
| Code Review — projection 渲染 | `render.rs` 的 reviewer 渲染 | group scope + active unit + projection bundle（**主路径**） | 否 |
| Code Review — 传统 prompt | `prompts.rs:52-55` | 上述条件不满足时 | 是（`:54`） |
| GroupFinalReview — group | `internal_pr_review.rs:119-122` | group scope | 是（`:120`） |
| GroupFinalReview — 非 group | `prompts.rs:121-123` | 非 group scope | **否** |

projection 路径的注入方式：加进 `role_structured_output_contract(Reviewer)`，或作为一个新 section。实施时选后者更清晰——`role_structured_output_contract` 的名字是讲输出格式的，塞行为契约进去会让命名失真。

第四条路径（`prompts.rs:121-123` 的 `build_internal_pr_review_prompt`，非 group scope 分支）原设计也漏了：它用了 `group_final_review_material_protocol` 但**没有** `reviewer_test_scope_contract`。

## 决策五：校验层检出用 Warning 级，不与既有 Error 一致

过程性 acceptance criterion 是**契约缺陷**，不是实现缺陷：它出现在 canonical contract 里，返修 Coder 无用，必须回到 Work Item 修订。因此校验层检出后应产出 finding，走既有 canonical 校验通道。不新增路由类型、不新增 `BlockerRoute` 成员。

**严重级别选 `Warning`，不与既有 acceptance criterion finding 一致。** 既有 `blank_acceptance_criterion_id`、`duplicate_acceptance_criterion_id` 都是 `Error`（`validation.rs:447`、`:467` 走 `error_finding`）——原设计写「与它们一致处理路径」是自相矛盾的表述，此处更正。选 `Warning` 的理由是本检出只能做关键词匹配，误报会让合法 criterion 被整体拒绝。

Warning 通道的完整行为已确认：

- `ContractFindingSeverity::Warning` → `work_item_split_validator/draft.rs:19-24` → `utils.rs:16-27` `warning()` → `WorkItemSplitFindingSeverity::Warning`。
- **不阻断候选**：`WorkItemDraftLocalValidator::validate`（`types.rs:57-84`）的三个消费点（`draft_batch/authoring.rs:80`、`:271`、`draft_batch/decisions.rs:400`）全部只读 `report.has_errors()`（`types.rs:9-13`，只看 Error）。Warning 不触发 `RetryOnce`、不置 `ValidationFailed`、不影响 `can_accept`。
- **会进用户可见输出，但仅限一处**：`report.findings` 整体经 `plan_outline/dto.rs:3-15` 进 `WorkItemDraftCandidatePayload.validator_findings`，经 `workspace_engine/controls.rs:216-256` 持久化并推送；前端 `WorkItemPlanArtifactContent.tsx:148` 的 DraftsTab **无条件**渲染 `ValidatorFindings`，`:710-730` 不按 severity 过滤。`draft_candidate` artifact 的默认 tab 就是 `drafts`（`WorkItemPlanArtifactPanel.tsx:351-353`）。
- **chat 确认流上静默**：`DraftValidationFailureNotice`（`ChatInputBar.tsx:188-190`、`WorkItemPlanStagedPanel.tsx:75-77`）只在 `!can_accept` 时渲染；batch 路径的 `failure_summary`（`draft_batch/runs.rs:401-409`）只收 `ValidationFailed` 记录。

因此「让缺陷可见且可路由」这个论证成立，但**成立条件是用户查看 artifact panel 的 Drafts tab**。这个限定必须写进 Plan，不能笼统说「可见」。

`ContractFindingSeverity::Warning` 在整个 `src/` 非测试代码里**零构造点**（只在 `draft.rs:21` 的 match 分支被消费），因此必须新增一个与 `error_finding`（`validation.rs:420-435`）对称的构造函数。这一点已确认，不是待定项。

**匹配集合保守**：要求 statement 同时出现「提交/commit」类词与「先失败/red/顺序/时序」类词，不得对单一关键词命中即报。宁漏不误——漏检由 reviewer 侧边界兜底。

## 决策六：`non_zero_test_execution` 的语义说明写入两侧 prompt

splitter 侧（`work_item_split_engine/prompts.rs:651`）与 reviewer 侧都需要，且措辞必须一致：`non_zero_test_execution` 表示验证命令执行时实际运行了非零数量的测试，是当前可观测的执行结果；它不表达测试曾先失败、不表达提交顺序、不表达任何时序。

这条说明本身就能消掉相当一部分误判——reviewer 之所以把它读成 TDD 要求，是因为 prompt 里从未定义过它。

**`ManualCheck` 与 `HandoffField` 的语义是本变更新建立的，不是核实既有语义。** `model.rs:109-116` 四个成员是裸枚举，无 doc comment、无 `as_str`、无任何消费逻辑区分它们；全仓非测试代码里 `EvidenceKind` 只出现三处（`model.rs:59` 字段、`work_item_projection/model.rs:71` 字段、`reviewer.rs:24` clone），**没有任何行为依赖具体 kind**。

`NonZeroTestExecution` ↔ `VerificationCheck.non_zero_test_execution_required`（`model.rs:69`）的对应有实质依据（同名、同语义域），这条扎实。但 `ManualCheck` ↔ `manual_instruction`（`model.rs:67`）、`HandoffField` ↔ `HandoffContract.required_fields`（`model.rs:96`）只是合理推断，代码里没有连接。

因此写 prompt 语义说明时，这两个 kind 需要自己定义措辞，且**定义一旦与 splitter 实际用法不符就会产生新的漂移**。措辞应保持在「可观测性」这个共同性质上，不对具体用法做过度承诺：`manual_check` 指人工检查的**结果**，`handoff_field` 指交接字段的**存在与内容**——都是当前状态，不含时序。

## 决策七：不改 Coder 侧 TDD 要求

`coding_execution_protocol`（`prompts.rs:278-290`）要求 Coder 提取执行清单并覆盖「TDD/测试要求」，`coding_delta_execution_protocol`（`:292-303`）同理，CLAUDE.md 的仓库规则也要求写代码前调用 `test-driven-development`。这些不动。

本变更的立场是：**TDD 是开发纪律，由 Coder 侧的 Skill 与协议约束；它不是只读审查阶段可判定的验收证据。** 二者不矛盾，边界只在 reviewer 一侧。

## 决策八：不做历史数据兼容

按用户明确的「就当是全新的系统来做」：既有 canonical contract 中若已含过程性 acceptance criterion，实施后会被校验检出为 finding。不添加豁免名单、不添加宽限期、不为历史记录编写兼容测试。

## 决策九：与 `remove-testing-stage` 在 `render.rs` reviewer 路径上的交集

`reviewer_context.rs:16-23` 与 `plan_defect.rs:425-430` 都调 `store.list_testing_reports(...)` 填 `ReviewerExecutionEnvelope.test_evidence_refs`，而该字段经 `render.rs:449` 的 `ReviewExecutionEvidence` section 进 reviewer prompt，且是 mandatory section（`render.rs:80`）。

`remove-testing-stage` 移除 testing report 存储会动这个字段与 section——**与本变更决策四要改的是同一条渲染路径**。

顺序上无强制依赖，但谁后做谁负责确认 mandatory section 集合完整、渲染不失败。原设计的风险 6 只提了 `prompts.rs`，低估了这个交集。

补充确认：`prompts.rs` 中 Testing 相关表述只有 `:444-453` 的 `build_tester_execute_plan_prompt`（整函数会被 `remove-testing-stage` 删除）；`code_review_material_protocol`（`:317-340`）与 `group_final_review_material_protocol`（`:342-361`）**没有任何** Testing / TestingReport / tester 字样。所以两个 change 在 `prompts.rs` 上其实不冲突，冲突在 `render.rs`。

## 边界

- 不新增或删除 `EvidenceKind` 成员。
- 不修改 `AcceptanceCriterion`、`ReviewerRequirementCheck`、`VerificationCheck` 结构。
- 不修改 `reviewer_check_refs` 与 criterion ID 集合完全一致的既有约束（`work_item_split_engine/prompts.rs:642`）。
- 不修改 `failure_route` 的推导（`reviewer.rs:25` 恒为 `CoderRework`）。
- 不修改 Coder 侧协议与仓库 TDD 规则。
- 不为历史持久化数据提供迁移或兼容层（见决策八）。
- 不改变 group final review 的停机语义（属 `open-group-final-review-change-gate`）。

## 已知缺口

`reviewer.rs:25` 把所有 criterion 的 `failure_route` 硬编码为 `CoderRework`，包括契约缺陷类。这意味着即使 reviewer 正确识别出某条 criterion 有问题，projection 也在暗示「让 Coder 修」。这是一个独立的路由缺陷，本 change 只记录，不修——修它需要重新设计 criterion 到路由的映射，范围超出本变更。
