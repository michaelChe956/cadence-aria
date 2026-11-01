# WorkItem 阶段 2 单候选 C′ MVP 实施计划 v1.0

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在不改变阶段 1 策略契约、legacy compile 事务语义和 coding 消费边界的前提下，交付单仓 `work-item-plan.md → PlanCandidateIr → InitialPlanCompileInput → immutable publication` 的单候选计划事务，并使 `auto_if_valid` campaign 无需旧 workitem 决策消息即可到达明确终态。

**Architecture:** markdown/EARS 是唯一可编辑源，`work_item_plan_compiler` 负责失败关闭的 grammar、确定性 lowering、既有 validator 适配与 publish 前 freshness；顶层 IR 只保存一次 source hash/compiler version，逐 item IR 继续使用强类型 canonical contract、verification plan 和 trusted commands。`workspace_engine` 复用阶段 1 已落地的 `PlanOutcome`、`ReviewInvocationScope`、`RunHistory`、CAS、provider ledger、唯一人工门快照和终态矩阵；compile 重构先冻结 legacy journal 行为，再让 legacy/IR 两个 adapter 汇入同一个 prepare/execute 核心。阶段 2 不实现阶段 3 的聊天流人工门或 `advance` 接口，也不删除 legacy WS 协议。

**Tech Stack:** Rust 2024、Serde、SHA-256、现有文件型 lifecycle/revision/compile journal stores、Node.js `.mjs` campaign driver、TypeScript/React/Vitest。

**Spec（唯一范围、架构与验收来源，执行者必须同时阅读）：**

- `openspec/changes/rearch-workitem-plan-pipeline/proposal.md`
- `openspec/changes/rearch-workitem-plan-pipeline/design.md`（D1～D7 与 D-A～D-E）
- `openspec/changes/rearch-workitem-plan-pipeline/tasks.md`（工作包 1～6）
- `openspec/changes/rearch-workitem-plan-pipeline/specs/work-item-plan-single-candidate/spec.md`（REQ-WSC-01、02、03、05、06、07；契约未定义 REQ-WSC-04，计划不臆造）

## Global Constraints

- **契约不可改写：** 本计划只展开上述四件套，不得扩大或修改其范围、架构裁决、验收阈值。实施中若发现必须改变 D1～D7、D-A～D-E 或 requirement，立即停止，先更新并重新确认 OpenSpec，再更新本计划。
- **阶段边界：** 阶段 2 复用阶段 1 `workitem-typed-outcome-policy` 的 classifier、fingerprint、`ReviewPhase`、`ReviewInvocationScope`/digest、`RunHistory.review_cycles`、`RoutingAction`、`HumanGateSnapshot`、CAS 与 `provider_start_ledger`；不得复制或重新定义四类 outcome、终态矩阵、预算含义和 14 条 classifier golden 的分类边界。
- **明确非目标：** 不重构 coding engine/WS；coding 运行期不解析 markdown；不改变 story/design 流程；不实现阶段 3 的聊天流人工门、长文本反馈、行内批准/终止或 `advance`；不做多仓新路径；不删除 generation-mode、outline/draft/batch 确认、两套 review decision legacy 协议；不改变 `work_item_split_validator` 规则。
- **compile 语义：** `src/product/workspace_engine/compile.rs:302-408` 与 `compile/finalizer.rs:122-327` 展示的是多个 journal write，不是单一数据库事务。等价性比较规范化语义产物、`WorkItemPlanCompileTransaction.status/step_cursor/plan_commit_state` 状态序列和恢复结果；动态 ID 必须使用保持跨字段引用关系的稳定占位符映射归一化，不得删除 ID；timestamp 先断言 RFC3339 合法且 `created_at` 全程稳定，再忽略具体值，不比较原始 JSON 字节。初始 compile 的 projection/publication/journal 全链路只使用外层注入的唯一 `InitialPlanCompileInput.now`，共享 prepare/execute/finalizer 禁止直接调用 `Utc::now()`；Abort/HumanTriage 等后续人工动作可读取实时钟。projection/contract hash 不得忽略，必须断言与对应产物一致。
- **测试纪律：** 每个实现任务依次完成失败测试、执行并看见预期失败、最小实现、执行并看见通过、建议原子提交。测试若因意外原因失败，先查明原因，不能把“编译不过”当作目标断言通过。
- **Cargo 命令：** 宿主机在 worktree 根目录直接运行；禁止给任何 cargo 命令添加 `-j`；定向单元测试只能使用 `cargo test --locked --lib 过滤名`。新增测试沿用 `tests.rs → include!("tests/part_03.rs") → include!("part_03/*.rs")` 的扁平模块树，函数名必须带任务约定前缀；执行定向测试前先以同一过滤名加 `-- --list`，统计 `: test` 行并记录“已验证匹配 N 项”，N=0 立即失败。本文对现有 WP3 过滤名给出 2026-06-30 实测数量；新增测试完成后的数量门写在对应步骤，禁止 0-test 假绿。全量门禁固定为：

  ```bash
  cargo fmt --check
  cargo clippy --all-targets --all-features --locked -- -D warnings
  cargo check --locked
  cargo test --locked
  ```

- **前端命令：** 只能使用 pnpm；最终执行 `(cd web && pnpm tsc -b)` 与 `(cd web && pnpm test)`。
- **文档路径：** 计划、设计、报告等 Cadence 产物必须在 `cadence/` 对应子目录；本计划固定在 `cadence/plans/2026-08-27_计划文档_WorkItem阶段2单候选C′MVP_v1.0.md`。
- **提交开关：** 当前 `CLAUDE.md` 的产物自动提交为关闭。下文每个任务都给出提交边界与 message 草案；实施者仅在操作者明确允许后执行 `git commit`，否则报告 changed files 和建议 message。
- **真实 Provider 授权：** 修改 Work Item Draft Prompt/Canonical 投影后，交付前必须提示：

  > 本次改动涉及 Work Item Draft Prompt 或其结构化契约。建议按 `cadence/designs/2026-07-24_技术方案_WorkItemDraftPrompt测试基线_v1.0.md` 执行 Case A 与 Case B 各 10 个有效首次输出的 Claude Code 验证；是否授权执行？

  未获授权不得调用 Provider，不得把完整 Prompt、Provider Draft、认证信息或目标仓内容写入报告。
- **变更前 HEAD 基线：** 开始 Task 1.1 前保存并验证干净基线：

  ```bash
  git rev-parse HEAD | tee /tmp/cadence-aria-workitem-phase2-baseline-sha
  git status --short
  git diff --cached --quiet
  ```

  预期：第一条输出 40 位 SHA；`git status --short` 只允许操作者已知的本计划文件；`git diff --cached --quiet` 退出码为 0。最终门禁按“最终验收门禁”重放该 SHA 以区分新增回归与 HEAD 既有失败。

## 关键代码事实与文件边界

以下事实来自四份 2026-08-27 新鲜勘察报告，执行者不需要重新做无目标的全仓搜索；实际编辑前仍应以结构化 outline 定向确认目标符号区间。

1. **compile 入口与 journal：** `src/product/workspace_engine/compile.rs:54-113` 是进入 compile 的 wrapper；`:257-443` 的 `run_work_item_plan_compile` 混合 store/lifecycle 读取、ID/时钟、projection、validator、journal put、publication 与 finalizer；`:448-595` 是 Continue/Abort recovery。`src/product/workspace_engine/compile/finalizer.rs:9-330` 是恢复/finalizer 状态机。当前 revision **不存在** `CompileStores`、`PreparedInitialPlanCompile`、`prepare_*` 或 `execute_*`；Task 3.2 必须新建，字段和 ownership 按契约授权“实施时以 `compile.rs:257-443` 现状为准”。
2. **legacy compile 输入：** `compile.rs:268-278` 读取 previous plan、active index、latest outline 与 accepted active drafts；`:283-284` 分配 compile ID/时间；`:330-336` 经 `draft_batch/compile_support.rs:31-178` 解析 `Option<BTreeMap<LogicalRepositoryId, String>>`；`:349-352` 经 `compile_support.rs:184-198` 读取 confirmed-design change order；已用 `ast-grep outline src/product/workspace_engine/types.rs --match WorkItemPlanCompileProjectionContext --view expanded` 与定向阅读核实 projection context 完整区间为 `types.rs:417-429`。
3. **持久恢复契约：** `WorkItemPlanCompileFinalizerCheckpoint` 只是 `#[cfg(test)]` failpoint key；真正落盘的是 `src/product/models/outline.rs:272-298` 的 `WorkItemPlanCompileTransaction`，当前尚无 `flow_kind` 或 source/IR/report/provenance refs。正常路径 cursor 依次覆盖 `preparing`、`validating`、`committing`、`plan_summary_prepared`、`child_session_N_ensured`、`child_session_N_binding_ensured`、`child_session_N_context_prepared`、`child_workspaces_prepared`、`plan_confirmed`、`compile_report_persisted`、`committed`；recovery 路径另有 `publication_resumed`（已由 `src/product/workspace_engine/compile/finalizer.rs:74-78` 当前实现核实），它只会在真正恢复 initial publication 后写入。五个 finalizer checkpoint 与 `src/product/work_item_revision_store/initial_publication.rs:89-95` 的五个 initial-publication checkpoint 是两个独立中断面；3.3/3.4/5.4 必须分别穿过后者才能断言 `publication_resumed`。
4. **prompt 删除/保留边界：** 2026-06-30 已用 `rg -n -C 1 'using-superpowers|writing-plans|test-driven-development|50k|单 session|Skill|TDD|拆分' src/product/work_item_split_engine/prompts.rs` 与定向源码阅读核实全部 B 层句子。除两个 `[superpowers_contract]` 外，`work_item_plan_runtime_contract` 当前 `prompts.rs:49` 与 `work_item_draft_runtime_contract` 当前 `:83` 也含“必调 Skill”；`build_work_item_draft_prompt` 当前 `:744` 是 B 层。当前 `:721`“不得输出 writing-plans 的 Markdown Plan”、`:745`“不得提前执行 writing-plans 的落盘步骤”和 `target_retain_instruction` 当前 `:764-765` 是 C 层输出/写回/target 安全边界，必须保留。Task 4.2 以逐句清单和正反精确文本断言编辑，不做会误伤 C 层的全 prompt token 禁止。
5. **canonical 与 validator：** `src/product/work_item_contract/model.rs:7-179` 定义 canonical 全字段；当前 `CanonicalWorkItemContract` 尚无契约 D1 要求的 `depends_on`，Task 1.1/2.2 需以兼容字段加入。`src/product/models/outline.rs:105-112` 是 `TrustedDraftVerificationCommand`，`:128-131` 是 `WorkItemDraftVerificationPlan`。已用 outline/定向阅读核实三个既有机械 validator 的实现入口为 `src/product/work_item_split_validator/types.rs:18-83`，`mod.rs:20-23` 仅 re-export；规则不改。`WorkItemPlanCompileProjectionContext` 的完整当前区间为 `src/product/workspace_engine/types.rs:417-429`。
6. **source linter 基础：** `src/product/workspace_engine/artifact_constraints.rs:1-75,222-307` 只有 heading/token/ID 检查，没有行号、字段路径、未知字段或 EARS grammar。新 compiler 可复用其 normalization primitive，但不得改变兼容 validator 的现有语义。
7. **阶段 1 复用资产：** `src/product/work_item_plan_policy/evaluate.rs:31-47` 的 `PlanOutcome`、`review/policy_routing.rs:36-59` 的终态映射、`models/workspace.rs:62-110` 的 durable 字段、`lifecycle_store/workspace.rs:519-569` 的 CAS 和 `review/routing.rs:80-310` 的策略接线均已存在。新路径必须替换 verification 分支现用的 `legacy_mechanical_report`，但不得复制策略类型或借用 legacy repair compatibility 分支充当 compiler 语义。
8. **campaign/WS：** `cadence/reports/workitem-coding-campaign/workitem_run_campaign.mjs:25-36` 解析 flow/policy/base/data root，`:296-323` 已按真实字段 `initial_count`/`verification_count` 规范化 cycle，`:923-956` 按 `ARIA_DATA_ROOT` 回读 durable session；当前 result schema 尚无 acceptance 所需的 `confirmed_count`、`duration_ms`、完整 `provider_start_ledger` 和 `legacy_decision_messages`。2026-06-30 已执行 Node 测试并验证匹配 21 项。WS raw text 在 `src/web/workspace_ws_handler/socket.rs:465-481` 先反序列化成 `WsInMessage`，未知字段随后不可见；Task 4.3 必须在这里保留提交字段 marker。
9. **创建、执行与恢复：** `src/web/handlers/lifecycle.rs:642-688` 以 `WebAppState.work_item_plan_single_candidate` 创建 plan/session 并快照 flow；该字段在 `src/web/state.rs:133,204` 当前默认 `false`。`src/product/workspace_engine/types.rs:189-214` 定义 `ProviderRunKind`；`src/web/workspace_ws_handler/run/provider_run.rs:160-303` 对当前 `WorkItemPlanAuthor` 固定调用 `build_outline_invocation`，legacy Start 现状不是 `build_split_prompt`。服务器数据根固定为 `src/web/app.rs:515-534`、`socket.rs:132-140` 的 `<workspace>/.aria`，不读取 `ARIA_DATA_ROOT`。Task 4.2a 必须覆盖真实 provider runner；Task 6.1a 必须把显式 web 启动参数接到 state；Task 6.2 让 driver 指向 worktree 实际 `.aria`。既有恢复范式在 `workspace_engine/tests/part_03/part_09.rs:132-440,572-669` 与 `work_item_revision_store/tests/initial_publication.rs:82-120`。

## 目标文件结构

- Create: `src/product/work_item_plan_compiler/mod.rs` — compiler 公共入口与版本常量。
- Create: `src/product/work_item_plan_compiler/types.rs` — AST、诊断、source context、顶层/逐 item IR、mechanical report 与 publication provenance 类型。
- Create: `src/product/work_item_plan_compiler/grammar.rs` — 稳定 section、结构化行与 EARS 规则。
- Create: `src/product/work_item_plan_compiler/parse.rs` — markdown → AST，逐行诊断。
- Create: `src/product/work_item_plan_compiler/lower.rs` — AST/context → typed IR 和 verification/trusted-command projection。
- Create: `src/product/work_item_plan_compiler/freshness.rs` — hash/version/publish 检查。
- Create: `src/product/work_item_plan_compiler/tests.rs` — grammar/lowering/freshness 单元测试。
- Create: `src/product/work_item_plan_source_store.rs` — immutable source revision、IR artifact、mechanical report 的文件型存储。
- Modify: `src/product/mod.rs` — 导出 compiler/source store。
- Modify: `src/product/work_item_contract/model.rs` — 兼容新增 `CanonicalWorkItemContract.depends_on: Vec<String>`。
- Modify: `src/product/models/outline.rs`、`src/product/workspace_engine/compile.rs`、`src/product/workspace_engine/compile/finalizer.rs` 与 `src/product/workspace_engine/plan_projection.rs` — 向后兼容的 durable compile context、输入抽取、输入式 publication preparation、prepare/execute 与既有 journal/finalizer 复用；`compile_initial_plan_revision` 的 legacy wrapper 负责 store 读取，IR adapter 负责 IR 投影，二者汇入同一 publication prepare/execute。
- Create: `src/product/workspace_engine/compile/ir_adapter.rs` — validated IR → `InitialPlanCompileInput`，只传递已分配 ID/时间及 durable refs。
- Create: `src/product/workspace_engine/single_candidate.rs` — 单候选内部状态机、generation strategy 与副作用屏障。
- Modify: `src/product/workspace_engine/mod.rs`、`types.rs`、`session_state.rs`、`review/routing.rs`、`prompts/review.rs` — 新路径接线，复用阶段 1 scope/outcome。
- Modify: `src/product/models/workspace.rs`、`src/product/lifecycle_store/workspace.rs`、`src/web/handlers/lifecycle.rs`、`src/web/workspace_ws_types/out.rs` — phase/source/IR/report/provenance 引用的 durable 与兼容输出。
- Modify: `src/product/work_item_split_engine/prompts.rs`、`src/product/work_item_split_engine/parse.rs` 与 prompt tests — markdown author contract、删除 B 层、保留 C 层；按符号/精确文本编辑，不按未核实行号删除。
- Modify: `src/product/work_item_revision_store/initial_publication.rs`、`src/product/models/work_item_revision.rs` — source store 所有的 immutable publication provenance 与 initial publication 身份校验；legacy 为 `None`，新路径为 `Some(ref)`，coding reader 不加 markdown 专属 gate。
- Modify: `src/product/workspace_engine/types.rs`、`src/web/workspace_ws_handler/run/provider_run.rs`、`socket.rs`、`workspace_ws_types/in_.rs` — durable flow 分发穿过真实 provider builder/parser，raw inbound 保留提交字段 marker。
- Modify: `src/cli.rs`、`src/web/app.rs`、`src/web/state.rs` — 显式 web 启动参数接到 session 创建时读取的 rollout state。
- Modify: `cadence/reports/workitem-coding-campaign/workitem_run_campaign.mjs` 与 `campaign_driver_policies.test.mjs` — single-candidate driver 与 fixture 测试。
- Create: `openspec/changes/rearch-workitem-plan-pipeline/field-source-matrix.md` 与 `fixtures/` 下 markdown/compiler fixtures — 契约 tasks 明确要求的实施产物。

---

## 工作包 1：字段来源矩阵与 grammar

### Task 1.1：逐字段唯一来源矩阵

**可追溯性：** 契约 Task 1.1；REQ-WSC-02、REQ-WSC-07；D1、D-D。

**Files:**

- Create: `openspec/changes/rearch-workitem-plan-pipeline/field-source-matrix.md`
- Create: `src/product/work_item_plan_compiler/tests.rs`（先放矩阵一致性测试，后续任务继续扩展）
- Modify: `src/product/mod.rs`
- Create: `src/product/work_item_plan_compiler/mod.rs`

**Interfaces / exact assertions:**

矩阵每行固定六列：`field_path | source | missing_behavior | forbidden_second_source | lowering_rule | test_id`。`source` 只能是 `markdown`、`session_confirmed_context`、`compiler_derived`、`compile_runtime` 之一。至少逐字段列出：

- `contract.schema_version`、`contract.identity.logical_work_item_id/title/kind`、`contract.goal.summary`、`contract.non_goals[]`、`contract.depends_on[]`；
- `contract.input_contracts[].contract_id/provider_logical_work_item_id/required_capabilities[]/compatibility_policy`；
- `contract.output_contracts[].contract_id/capabilities[]`；
- `contract.tasks[].task_id/statement/requirement_refs[]/done_when_refs[]`；
- `contract.write_policy.exclusive_scopes[]/forbidden_scopes[]`；
- `contract.acceptance_criteria[].criterion_id/statement/required_evidence[]`；
- `contract.verification_checks[].check_id/command/manual_instruction/required/non_zero_test_execution_required`；
- `contract.handoff_contract.required_fields[]/provided_contract_refs[]/reviewer_check_refs[]`；
- `contract.blocker_rules[].reason_code/route/target_contract_refs[]`；
- `contract.design_traceability[].source_type/source_id/requirement_id`；
- `verification_plan.checks[]`、`trusted_commands[].command/cwd/purpose/source_ref`、`target_repository_id`；
- `ir.source_revision_hash`、`ir.compiler_version`、`publication_provenance.id/plan_id/plan_revision_id/source_revision_ref/plan_candidate_ir_ref/mechanical_report_ref/source_revision_hash/compiler_version/published_at/content_hash`、`compile_id/now`。

唯一来源固定：作者语义字段来自 markdown；`target_repository_id` 与 trusted command catalog 来自 session/confirmed context；`verification_plan.checks` 从 canonical verification checks 确定性投影；source hash/compiler version 由 compiler 派生；publication ID/time、compile ID/time 由事务外层运行时注入。handoff 只声明 schema，coding 完成后的值仍由 `src/product/models/work_item_revision.rs:187-197` 的 `HandoffRevision` 产生。

- [ ] **Step 1 — 写失败测试。** 在 `tests.rs` 用 `include_str!("../../../openspec/changes/rearch-workitem-plan-pipeline/field-source-matrix.md")`，断言上述全部 field path 恰好出现一次、每行 source 属四值集合、`target_repository_id` 不标为 markdown、`trusted_commands` 不标为 prompt、`handoff runtime values` 不出现在 lowering 列。首次运行：

  ```bash
  cargo test --locked --lib work_item_plan_compiler::tests::field_source_matrix
  ```

  预期失败：模块或 matrix 文件不存在；不能以其他编译错误代替该失败。
- [ ] **Step 2 — 创建矩阵与最小模块。** 写完每一行的缺失行为（required→diagnostic；optional→显式 `None`/空集合）、禁止第二来源和具体 test ID；不允许“prompt 补齐”“runtime 猜测 path”。
- [ ] **Step 3 — 运行转绿。** 执行同一 cargo 命令，预期 `field_source_matrix_* ... ok` 且过滤集 0 failed。
- [ ] **Step 4 — 人工审阅。** 将 `CanonicalWorkItemContract`、verification/trusted/target/provenance 字段与矩阵逐项打勾，确认没有把 handoff runtime value 变成第二来源。

**提交建议：** `docs(workitem): define single-candidate field source matrix`

### Task 1.2：稳定 markdown/EARS grammar

**可追溯性：** 契约 Task 1.2；REQ-WSC-02、REQ-WSC-06；D1、D6、D-D。

**Files:**

- Create: `src/product/work_item_plan_compiler/grammar.rs`
- Create: `src/product/work_item_plan_compiler/types.rs`
- Modify: `src/product/work_item_plan_compiler/mod.rs`
- Modify: `src/product/work_item_plan_compiler/tests.rs`

**Interfaces / metadata only:**

```rust
pub const WORK_ITEM_PLAN_COMPILER_VERSION: &str = "work_item_plan_compiler/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemPlanAst {
    pub items: Vec<WorkItemPlanItemAst>,
    pub notes: Vec<String>,
    pub rationale: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerDiagnostic {
    pub code: String,
    pub line: usize,
    pub field: String,
    pub message: String,
    pub repair_example: String,
}
```

本任务只定义稳定 section/key/EARS 元数据、AST 容器及 diagnostic 类型，不实现 parser/linter，也不声称能够产生真实诊断。稳定文档 section 固定为 `# Work Item Plan`，每项 `## Work Item WI-<digits>`，结构化子节固定为 `Identity`、`Goal`、`Non Goals`、`Dependencies`、`Inputs`、`Outputs`、`Tasks`、`Write Policy`、`Acceptance Criteria`、`Verification`、`Handoff Schema`、`Blockers`、`Traceability`；文档/项目尾部仅 `Notes`、`Rationale` 容忍自由文本。结构化行统一 `- key: value` 或带显式 ID 的 `- TASK-001 | ...`，未知 key 的 fail-closed 规则只作为 grammar metadata。任务与验收 statement 的 EARS metadata 固定为 `WHEN <condition> THE SYSTEM SHALL <observable outcome>`；`required_evidence`、compatibility、route 的允许值只登记现有契约集合。

- [ ] **Step 1 — 写失败测试。** 只断言 grammar 常量、section/key 元数据、AST 类型字段/derive、允许值集合和 compiler version；不测试 `missing_section`、`invalid_ears` 的行号、字段或修复示例，因为 parser/linter 尚未存在。执行：

  ```bash
  cargo test --locked --lib work_item_plan_compiler::tests::grammar_contract
  ```

  预期失败：grammar 常量/类型尚未实现；不能以其他编译错误代替该失败。
- [ ] **Step 2 — 实现元数据与类型。** 只定义稳定语法、AST 容器和公开 diagnostic 形状；不解析输入、不扫描行号、不构造 fixture 诊断。
- [ ] **Step 3 — 运行转绿。** 同一命令预期 grammar contract tests 全部 `ok`；真实诊断断言留给 Task 1.4。

**提交建议：** `feat(workitem): define deterministic plan markdown grammar`

### Task 1.3：rep4 完整 source fixture 与 diagnostic fixtures

**可追溯性：** 契约 Task 1.3；REQ-WSC-02、REQ-WSC-06、REQ-WSC-07；D3、D6、D-D。

**Files:**

- Create: `openspec/changes/rearch-workitem-plan-pipeline/fixtures/work-item-plan-rep4.md`
- Create: `openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/missing-verification.md`
- Create: `openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/unknown-field.md`
- Create: `openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/invalid-id.md`
- Create: `openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/invalid-ears.md`
- Create: `openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/expected.json`
- Modify: `src/product/work_item_plan_compiler/tests.rs`

**静态 fixture/schema 边界：** `work-item-plan-rep4.md` 必须静态包含恰好 backend/frontend/integration 三个 item，integration 的 `Non Goals` 只禁止产品实现和上游测试并明确允许 `tests/integration/**`；HTML 验收只断言容器与 `level-select.js`，另一个脚本证据断言 `/api/levels`，不得出现 rep4 两个矛盾。三个 item 覆盖矩阵里的全部作者语义字段，trusted commands/target repo 不写入 markdown。`expected.json` 每项严格为 `{fixture, code, line, field, repair_example}` 的 schema；四个错误 fixture 只含一个目标错误，且静态检查 `code` 属 grammar/lowering 集合。

本任务只测试 fixture 的静态结构与 `expected.json` schema，不调用 parser/linter，不断言实际诊断行号、字段或示例；实际诊断断言统一在 Task 1.4 parser/linter 实现后执行。四例仅对应缺 section、未知 field、非法 ID、非法 EARS，不把 rep2/3/4 reviewer finding 伪装为 compiler diagnostic。

- [ ] **Step 1 — 写失败测试。** `include_str!` 读取确切 fixture，断言 item 标题/数量/冲突字符串、每例只有一个目标错误；解析 `expected.json` 后只断言字段集合、类型、非空 schema 和 fixture 一一对应，不读取实际 parser 输出。执行：

  ```bash
  cargo test --locked --lib work_item_plan_compiler::tests::fixtures
  ```

  预期失败：fixture 文件不存在。
- [ ] **Step 2 — 写完整 fixtures。** `work-item-plan-rep4.md` 不使用省略号，所有 ID/ref 具备可解析形状；错误 fixture 保持单一目标错误，避免在 parser 尚未实现时预设诊断。
- [ ] **Step 3 — 运行转绿。** 同一命令预期静态 fixture/schema tests 全部通过；实际 line/field/example 比对由 Task 1.4 接管。

**提交建议：** `test(workitem): add rep4 markdown and compiler diagnostic fixtures`

### Task 1.4：source linter 失败关闭

**可追溯性：** 契约 Task 1.4；REQ-WSC-02；D1、D-D。

**Files:**

- Create: `src/product/work_item_plan_compiler/parse.rs`
- Modify: `src/product/work_item_plan_compiler/mod.rs`
- Modify: `src/product/work_item_plan_compiler/tests.rs`
- Read-only reuse: `src/product/workspace_engine/artifact_constraints.rs:222-307`

**Interfaces:**

```rust
pub fn lint_work_item_plan_source(source: &str) -> Vec<CompilerDiagnostic>;
pub fn parse_work_item_plan(source: &str) -> Result<WorkItemPlanAst, Vec<CompilerDiagnostic>>;
```

- [ ] **Step 1 — 写失败测试。** 覆盖未知结构化 heading/key、缺 required section/field、重复 WI/TASK/AC/CHECK ID、非法依赖 ID、依赖不存在、自依赖、dependency cycle、非法 EARS、Notes/Rationale 任意 Unicode/冒号/表格文本允许；断言 diagnostics 按 `(line, field, code)` 稳定排序，每条 line 为 1-based、field 非空、message 非空且 `repair_example` 恰好一个。这里同时读取 Task 1.3 的四个错误 fixture，逐项断言实测 `line/field/repair_example` 与 `expected.json` 一致；这是首次允许写诊断行号/字段/示例断言的任务。执行：

  ```bash
  cargo test --locked --lib work_item_plan_compiler::tests::source_linter
  ```

  预期失败：`lint_work_item_plan_source` 未定义。
- [ ] **Step 2 — 实现最小 linter/parser。** 可复制/抽出 heading normalization primitive，但不得改 `validate_workspace_artifact_constraints` 的现有返回或兼容行为。结构化区域一律 fail-closed；自由文本区不做 token 污染检查；诊断必须来自实际解析输入，不能由 fixture expected 反向生成。
- [ ] **Step 3 — 运行转绿。** 同一命令预期 source linter tests 全绿；再运行：

  ```bash
  cargo test --locked --lib workspace_engine::artifact_constraints
  ```

  预期既有 artifact constraint tests 无回归。

**提交建议：** `feat(workitem): lint plan markdown with line diagnostics`

### Task 2.1：markdown → AST 与可回喂诊断

**可追溯性：** 契约 Task 2.1；REQ-WSC-02；D1、D-D。

**Files:**

- Modify: `src/product/work_item_plan_compiler/parse.rs`
- Modify: `src/product/work_item_plan_compiler/types.rs`
- Modify: `src/product/work_item_plan_compiler/tests.rs`

**Interfaces:** `parse_work_item_plan` 必须保留原始行号到每个 AST field；内部使用 `Spanned<T> { value: T, line: usize }`。公开 diagnostic 不暴露 parser 内部 token 类型。

- [ ] **Step 1 — 写失败测试。** 给一份同时含非法 WI ID、缺 Goal、非法 EARS 的源，断言返回三个稳定 diagnostics；每项精确断言 `code`、1-based `line`、canonical field path、中文 message 非空、repair example 中只出现一个 `- key:` 或 EARS 例。执行：

  ```bash
  cargo test --locked --lib work_item_plan_compiler::tests::parse_diagnostics
  ```

  预期失败：现有 parser 未携带完整 span 或未聚合文档级诊断。
- [ ] **Step 2 — 实现 span 与聚合。** parser 不做 typed lowering，不读取 session/store，不调用 Provider；同源多次 parse 产生相同 AST/diagnostic 顺序。
- [ ] **Step 3 — 运行转绿。** 同一命令两次，预期输出和 snapshot 完全相同、0 failed。

**提交建议：** `feat(workitem): parse plan markdown into spanned ast`

### Task 2.2：AST → 顶层/逐 item typed IR

**可追溯性：** 契约 Task 2.2；REQ-WSC-02；D1、D-D。

**Files:**

- Create: `src/product/work_item_plan_compiler/lower.rs`
- Modify: `src/product/work_item_plan_compiler/types.rs`
- Modify: `src/product/work_item_plan_compiler/mod.rs`
- Modify: `src/product/work_item_contract/model.rs`
- Modify: canonical model constructors/tests affected by compatible `depends_on`
- Modify: `src/product/work_item_plan_compiler/tests.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanCandidateIr {
    pub source_revision_hash: String,
    pub compiler_version: String,
    pub items: Vec<PlanCandidateItemIr>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanCandidateItemIr {
    pub target_repository_id: String,
    pub contract: CanonicalWorkItemContract,
    pub verification_plan: WorkItemDraftVerificationPlan,
    pub trusted_commands: Vec<TrustedDraftVerificationCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemPlanSourceContext {
    pub target_repository_id: String,
    pub trusted_command_catalog: Vec<TrustedDraftVerificationCommand>,
}

pub fn lower_work_item_plan(
    source: &str,
    ast: WorkItemPlanAst,
    context: &WorkItemPlanSourceContext,
) -> Result<PlanCandidateIr, Vec<CompilerDiagnostic>>;

pub fn compile_work_item_plan(
    source: &str,
    context: &WorkItemPlanSourceContext,
) -> Result<PlanCandidateIr, Vec<CompilerDiagnostic>>;
```

`CanonicalWorkItemContract` 兼容新增：

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub depends_on: Vec<String>;
```

`source_revision_hash` 固定为原始 markdown UTF-8 bytes 的 lowercase SHA-256 hex；`compiler_version == WORK_ITEM_PLAN_COMPILER_VERSION`；provenance 不得复制到 item。verification plan 必须是 `contract.verification_checks` 的同序 clone；trusted command 只能按 markdown verification command ref 从 `context.trusted_command_catalog` 选择，找不到或重复 ref 产生 diagnostic，绝不采信 markdown 自带 command/cwd。

- [ ] **Step 1 — 写失败测试。** 断言 rep4 fixture lower 后 `items.len()==3`；顶层 JSON 恰含 `source_revision_hash/compiler_version/items`；item JSON 不含这两个键；三个 target repo 都等于 context；`depends_on` 与 input provider refs 一致；verification checks 同序相等；trusted commands 与 catalog 中被引用记录逐字段相等。再断言未知 command ref、markdown 试图写 `command:` 或 `target_repository_id:` 均失败。执行：

  ```bash
  cargo test --locked --lib work_item_plan_compiler::tests::lower_typed_ir
  ```

  预期失败：IR/lower/`depends_on` 尚未实现。
- [ ] **Step 2 — 实现兼容 model 与 lowering。** 更新所有 canonical fixture 构造器显式填 `depends_on` 或依赖 serde default；不能新增临时 JSON 中间协议，不能从自由文本推断缺字段。
- [ ] **Step 3 — 运行转绿。** 同一命令通过；再执行：

  ```bash
  cargo test --locked --lib work_item_contract
  ```

  预期 canonical 旧 JSON 兼容、所有 model tests 通过。

**提交建议：** `feat(workitem): lower markdown into typed plan candidate ir`

### Task 2.3：classifier golden 与 compiler diagnostic 边界

**可追溯性：** 契约 Task 2.3；REQ-WSC-03、REQ-WSC-06、REQ-WSC-07；D3、D6。

**Files:**

- Reuse unchanged: `src/product/work_item_plan_policy/fixtures/golden_findings.json`
- Create: `openspec/changes/rearch-workitem-plan-pipeline/fixtures/reviewer-finding-channel-map.json`
- Modify: `src/product/work_item_plan_compiler/tests.rs`
- Modify only if a regression is found: stage 1 classifier tests; classification rule changes require stopping for contract review

**Exact boundary:** stage 1 fixture 固定 14 条 = 11 原始（rep2/3/4 九条 + rep1 round-1 两条 Advisory）+ 3 条人工 `class_hint` 变体。`reviewer-finding-channel-map.json` 恰有 rep2/3/4 九条映射；基于当前原文它们都是契约完备度/跨 item 语义/自相矛盾 evidence，不是 markdown grammar/lowering 错误，因此每条固定 `channel: "prompt_few_shot"`、`compiler_fixture: null`。若实施证据要求把某条改为 compiler diagnostic，必须先停下更新契约裁决，不能在代码中自行升级。

- [ ] **Step 1 — 写失败测试。** 断言 golden 数量 14、provider_raw=11、annotated_variant=3、分类结果逐条等于 `expected_class`；channel map 九个 ID 与 rep2/3/4 原始 finding 一一对应且无 diagnostic fixture；Task 1.3 的四个 compiler diagnostics 不冒充 reviewer finding。执行：

  ```bash
  cargo test --locked --lib work_item_plan_policy::tests_classify
  cargo test --locked --lib work_item_plan_compiler::tests::reviewer_finding_channel_boundary
  ```

  预期：第一条保持绿；第二条因 channel map 尚不存在而失败。若第一条失败，视为阶段 1 回归，先修复而非改变 expected class。
- [ ] **Step 2 — 写 channel map 与边界校验。** prompt few-shot 保留完整 finding/message/evidence/action，但 provider 原始字段不得被人工 class_hint 覆盖。
- [ ] **Step 3 — 运行转绿。** 两条命令均通过，classifier 14/14 正确，compiler boundary tests 通过。

**提交建议：** `test(workitem): separate classifier and compiler golden channels`

### Task 2.4：完整 lowering 通过既有 validator

**可追溯性：** 契约 Task 2.4；REQ-WSC-02、REQ-WSC-03；D1、D2、D-D。

**Files:**

- Modify: `src/product/work_item_plan_compiler/lower.rs`
- Modify: `src/product/work_item_plan_compiler/types.rs`
- Modify: `src/product/work_item_plan_compiler/tests.rs`
- Reuse unchanged: `src/product/work_item_split_validator/types.rs:18-83`（实际三层 validator 实现；`mod.rs:20-23` 仅 re-export）

**Interfaces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanCandidateValidationContext<'a> {
    pub project_id: &'a str,
    pub issue_id: &'a str,
    pub plan_id: &'a str,
    pub source_story_spec_ids: &'a [String],
    pub source_design_spec_ids: &'a [String],
    pub repository_profile: Option<&'a RepositoryProfile>,
    pub now: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanCandidateMechanicalReport {
    pub source_revision_hash: String,
    pub compiler_version: String,
    pub findings: Vec<WorkItemSplitFinding>,
}

pub fn validate_plan_candidate_ir(
    ir: &PlanCandidateIr,
    context: &PlanCandidateValidationContext<'_>,
) -> Result<PlanCandidateMechanicalReport, Vec<CompilerDiagnostic>>;
```

adapter 必须用 session/confirmed context 中的 project/issue/plan、source story/design refs、repository profile 和外层注入时间构造既有 validator 所需的 outline/draft/plan 视图；只转换输入形状，不改 membership/dependency/scope/semantics/verification 规则。Error finding 导致 `Err` 或报告 `has_errors` 后禁止 publish；Warning/Info 原样进入策略 evidence。

- [ ] **Step 1 — 写失败测试。** rep4 IR 经过 `WorkItemPlanOutlineValidator`、每项 `WorkItemDraftLocalValidator`、最终 `WorkItemSplitValidator` 后 Error 数为 0；故意移除 integration input contract 后返回既有 validator code；stage 1 14 条 fixture 分类结果不变。执行：

  ```bash
  cargo test --locked --lib work_item_plan_compiler::tests::full_lowering_validator
  ```

  预期失败：IR validator adapter 尚未实现。
- [ ] **Step 2 — 实现 adapter。** `verification_plan.checks` 与 canonical checks 同序；依赖图只从 `contract.depends_on` 派生，不新增 DependencyGraph 类型；target/trusted 强类型保持不降级。
- [ ] **Step 3 — 运行转绿。** 同一命令通过；再执行：

  ```bash
  cargo test --locked --lib work_item_split_validator
  ```

  预期既有 validator 全绿且规则文件无语义改动。

**提交建议：** `feat(workitem): validate compiled ir with existing validators`

### Task 2.5：publish freshness 与 immutable provenance

**可追溯性：** 契约 Task 2.5；REQ-WSC-02、REQ-WSC-07；D1、D-C。

**Files:**

- Create: `src/product/work_item_plan_compiler/freshness.rs`
- Modify: `src/product/work_item_plan_compiler/types.rs`
- Create: `src/product/work_item_plan_source_store.rs`
- Modify: `src/product/mod.rs`
- Modify: `src/product/models/work_item_revision.rs`
- Modify: `src/product/work_item_revision_store/initial_publication.rs`
- Modify: `src/product/work_item_revision_store/tests/initial_publication.rs`
- Modify: `src/product/work_item_plan_compiler/tests.rs`
- Modify: `src/product/models/outline.rs`（供 compile transaction durable context 复用；具体字段在 Task 3.2/3.4 落地）
- Modify: `src/product/models/workspace.rs`、`src/product/lifecycle_store/workspace.rs`（typed compile reservation 与 CAS）

**Interfaces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanCandidatePublicationProvenance {
    pub id: String,
    pub plan_id: String,
    pub plan_revision_id: String,
    pub source_revision_ref: String,
    pub plan_candidate_ir_ref: String,
    pub mechanical_report_ref: String,
    pub source_revision_hash: String,
    pub compiler_version: String,
    pub published_at: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FreshnessError {
    SourceRevisionMismatch,
    CompilerVersionMismatch,
    MechanicalValidationFailed,
}

pub fn verify_publish_freshness(
    current_source: &str,
    ir: &PlanCandidateIr,
    mechanical_report: &PlanCandidateMechanicalReport,
) -> Result<(), FreshnessError>;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SingleCandidateCompileReservation {
    pub compile_id: String,
    pub now: String,
    pub publication_provenance_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileReservationError {
    Conflict,
    InvalidSession,
    PersistenceFailure {
        diagnostic: crate::product::work_item_plan_policy::PolicyDiagnostic,
    },
}

impl CompileReservationError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Conflict => "SINGLE_CANDIDATE_COMPILE_RESERVATION_CONFLICT",
            Self::InvalidSession => "SINGLE_CANDIDATE_COMPILE_RESERVATION_INVALID_SESSION",
            Self::PersistenceFailure { .. } => "persistence_failure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourceStoreScope {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceStoreError {
    MalformedRef,
    WrongKind,
    ScopeMismatch,
    DanglingRef,
    IdentityMismatch,
    ContentHashMismatch,
    SourceHashMismatch,
    CompilerVersionMismatch,
    Io(String),
    Json(String),
    Serialize(String),
}

impl SourceStoreError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MalformedRef => "SOURCE_STORE_MALFORMED_REF",
            Self::WrongKind => "SOURCE_STORE_WRONG_KIND",
            Self::ScopeMismatch => "SOURCE_STORE_SCOPE_MISMATCH",
            Self::DanglingRef => "SOURCE_STORE_DANGLING_REF",
            Self::IdentityMismatch => "SOURCE_STORE_IDENTITY_MISMATCH",
            Self::ContentHashMismatch => "SOURCE_STORE_CONTENT_HASH_MISMATCH",
            Self::SourceHashMismatch => "SOURCE_STORE_SOURCE_HASH_MISMATCH",
            Self::CompilerVersionMismatch => "SOURCE_STORE_COMPILER_VERSION_MISMATCH",
            Self::Io(_) | Self::Json(_) | Self::Serialize(_) => "persistence_failure",
        }
    }
}

impl LifecycleStore {
    pub fn put_compile_reservation_cas(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        session_id: &str,
        expected: &WorkspaceSessionRecord,
        reservation: &SingleCandidateCompileReservation,
    ) -> Result<WorkspaceSessionRecord, CompileReservationError>;
}
```

`SourceStoreError::Io`、`Json`、`Serialize`（或等价包装 `ProductStoreError`）是独立的持久化错误域，必须保留底层错误信息；它们不得被压成 `DanglingRef` 或上述四类 canonical-ref 语义码，调用方必须映射为 fatal（`FatalReason::PersistenceFailure` / `PolicyDiagnostic.code == "persistence_failure"`）。错误形状与 `src/product/json_store.rs:8-25` 的 `ProductStoreError::{Io, Json, ...}` 对齐。

freshness 校验的调用边界固定在 adapter 外层/新路径 engine（该层允许读取 source store），不得放进纯 prepare，也不得让 coding reader 读取 markdown。新路径在任何 provenance 或 compile transaction 写入前读取 immutable source revision，核对 current source bytes、IR `source_revision_hash`、compiler version、mechanical report hash/version 与 zero Error；不匹配即 fail-closed，已发布 binding 不受 source 后续修改影响。写入 reservation 前，先由唯一 Approval CAS 把 Approval record 原子落在 `WorkspaceSessionRecord`：新增 `approval_attempt_id: Option<String>` 与 `approved_at: Option<String>`（逐字段 `#[serde(default, skip_serializing_if = "Option::is_none")]`），并固定接口 `compare_and_save_single_candidate_approval(expected: &WorkspaceSessionRecord, approval_attempt_id: &str, approved_at: &str) -> Result<WorkspaceSessionRecord, ProductStoreError>`；该 CAS 必须完全对齐 `src/product/lifecycle_store/workspace.rs:519-555` 的边界：先取得 session 排他锁，再在锁内重读 stored record，对 stored 与完整 expected（包括全部字段及 `updated_at` 版本 token）做全 record equality；不相等的 stale expected 返回 `ProductStoreError::Conflict`，不得部分合并；首次保存与后续重试都必须以该次操作开始时的完整 durable record 作为 expected（后续重试须以首次返回/重读的 stored record 作为 expected）。只有完整 expected 相等时，若锁内重读后已有相同 Approval 二元组 `(approval_attempt_id, approved_at)`，才原样幂等返回 stored；已有任一不同值时禁止覆盖并返回 `ProductStoreError::Conflict`。只有尚未落盘二元组时才写入；锁、读取、JSON 解析/序列化或 write 错误必须原样保留其 `ProductStoreError` 语义，不得伪装成 `Conflict`。在该持久化边界还必须根据 expected 的 `id`（session_id）、`entity_id`（WorkItemPlan 的 plan_id）及 `work_item_plan_source_revision_ref`、`plan_candidate_ir_ref`、`mechanical_report_ref` 三 refs 重算 `approval_attempt_id`，与调用者传入值逐字比较；任意传入值（包括任意伪造字符串、大小写变化或字段错位）不一致即拒绝且不得写入。该 CAS 要求 expected 为完整 durable session、phase=`Approval` 且 source/IR/report refs 已在同一 session。`approval_attempt_id` 固定为当前三个 durable refs 所标识的首次获批版本的 `sha256("single_candidate_approval" + NUL + session_id + NUL + plan_id + NUL + source_ref + NUL + ir_ref + NUL + report_ref)` lowercase hex；`approved_at` 仅在调用首次 Approval CAS 前读取一次时钟并要求 RFC3339。CAS 把二者原子保存；冲突或进程重试必须先重读已保存字段，同一三 refs 已有值时逐字复用，不得重新取时钟。只有 Approval CAS 成功后才允许 reservation CAS。此时 durable tuple `(session_id, plan_id, approval_attempt_id, approved_at)` 有真实持久化落点；`compile_id` 的输入字节必须固定为 `b"single_candidate_compile\0" || session_id.as_bytes() || b"\0" || plan_id.as_bytes() || b"\0" || approval_attempt_id.as_bytes() || b"\0" || approved_at.as_bytes()`，即 domain separator、session_id、plan_id、approval_attempt_id、approved_at 逐字段 NUL 分隔后取 lowercase SHA-256；不得使用 JSON tuple、显示文本或未定义拼接。固定测试向量：`(session-001, plan-001, approval-001, 2026-08-27T12:34:56Z)` 的 canonical bytes hex 为 `73696e676c655f63616e6469646174655f636f6d70696c650073657373696f6e2d30303100706c616e2d30303100617070726f76616c2d30303100323032362d30382d32375431323a33343a35365a`，预期 `compile_id` 为 `5a16e570210838318554c17b3ebd0c433c3001ce00adb7b8e9726d79aecf788e`。`now = approved_at`，reservation 前重试不得重新取时钟或随机分配 ID。reservation 前 crash 重启只调用现有精确 `LifecycleStore::get_workspace_session(session_id)` 读取并校验同一 project/issue/plan scope 下的 durable Approval record，按同一 tuple 重算 `compile_id`、`now` 与 `publication_provenance_ref`，不会依赖重启前内存；`publication_provenance_ref` 固定为 `project/{project_id}/issue/{issue_id}/plan/{plan_id}/publication_provenance/{compile_id}`。reservation 成功后才分配并持久化唯一 publication provenance（含 `plan_id`），并绑定待分配的 `plan_revision_id`；所有 publication IDs（plan revision、child artifacts）均由 `sha256(compile_id + NUL + object_kind + NUL + logical_id)` 确定性重算。CAS 冲突只有同一三元组可幂等重放，其他值返回稳定码 `SINGLE_CANDIDATE_COMPILE_RESERVATION_CONFLICT`；legacy provenance 为 `None`。`InitialPlanCompileInput` 不增删字段，也不承载这些 refs。

`SingleCandidateCompileReservation` 单独持久化在 `WorkspaceSessionRecord.compile_reservation`，并以 `#[serde(default, skip_serializing_if = "Option::is_none")]` 保持旧 session JSON 兼容。`put_compile_reservation_cas(project_id, issue_id, plan_id, session_id, expected, reservation)` 是该字段写入的唯一 CAS API，`expected: &WorkspaceSessionRecord` 必须是完整 durable record（包括现有 `updated_at` 版本 token），实现先在锁内重读并做全 record equality，拒绝任何 stale session；同时校验参数与 expected 的 project/issue/session/plan(entity) scope，`workspace_type=WorkItemPlan`、`flow_kind=SingleCandidate`、`single_candidate_phase=Approval`、`work_item_plan_source_revision_ref`/`plan_candidate_ir_ref`/`mechanical_report_ref` 三个 canonical refs 均 `Some` 且分别通过当前 plan scope 与正确 object kind 校验，durable `approval_attempt_id`/`approved_at` 均 `Some`，并校验 reservation 的 `compile_id`/`now`/`publication_provenance_ref` 与该 Approval tuple 的确定性结果逐字相等。它必须位于任何 `put_publication_provenance` 或 `put_compile_transaction` 之前（Generate/Evaluate 的 source/IR/report immutable records 可在 Approval reservation 前按各自 typed API 写入）；expected 中 reservation 为 None 时只创建，已有 reservation 仅允许完全相同三元组幂等重放；成功返回更新后的 `WorkspaceSessionRecord`。完整 record 冲突返回 `Conflict`/`SINGLE_CANDIDATE_COMPILE_RESERVATION_CONFLICT`，session/phase/scope/ref 不合法返回 `InvalidSession`/`SINGLE_CANDIDATE_COMPILE_RESERVATION_INVALID_SESSION`；锁、读取、序列化或写盘错误不得压成 Conflict，统一转换为 `PersistenceFailure`，由调用方按阶段 1 语义记录 `PolicyDiagnostic`、以 `FatalReason::PersistenceFailure` durable 收敛为 Failed。

`WorkItemPlanRevision` 新增向后兼容的 `publication_provenance_ref: Option<String>`；single-candidate publication 必须逐字写入 provenance canonical ref，legacy publication 为 `None`。本轮固定 **`WorkItemPlanSourceStore` 为 `PlanCandidatePublicationProvenance` 与三类输入对象的唯一 durable owner**。定义 `SourceStoreScope { project_id: String, issue_id: String, plan_id: String }` 作为显式 expected scope。三类对象使用带完整 scope 的 typed record 与精确 API：`SourceRevisionRecord { id, source, source_revision_hash, content_hash }` 对应 `put_source_revision(&self, project_id: &str, issue_id: &str, plan_id: &str, revision: &SourceRevisionRecord) -> Result<String, SourceStoreError>` / `get_source_revision(&self, expected_scope: &SourceStoreScope, canonical_ref: &str) -> Result<SourceRevisionRecord, SourceStoreError>`；`PlanCandidateIrRecord { id, source_revision_id, ir: PlanCandidateIr, content_hash }` 对应 `put_plan_candidate_ir(&self, project_id: &str, issue_id: &str, plan_id: &str, ir: &PlanCandidateIrRecord) -> Result<String, SourceStoreError>` / `get_plan_candidate_ir(&self, expected_scope: &SourceStoreScope, canonical_ref: &str) -> Result<PlanCandidateIrRecord, SourceStoreError>`；`PlanCandidateMechanicalReportRecord { id, source_revision_id, ir_id, report: PlanCandidateMechanicalReport, content_hash }` 对应 `put_mechanical_report(&self, project_id: &str, issue_id: &str, plan_id: &str, report: &PlanCandidateMechanicalReportRecord) -> Result<String, SourceStoreError>` / `get_mechanical_report(&self, expected_scope: &SourceStoreScope, canonical_ref: &str) -> Result<PlanCandidateMechanicalReportRecord, SourceStoreError>`。这些方法均属于 `impl WorkItemPlanSourceStore`，不得在其他 store 或模块复制；put 成功返回 canonical ref path `String`。本仓库写入前已 grep 确认 `WorkItemPlanSourceStore` 及上述 API 当前均不存在；实现不得依赖隐含 API。

每个 put 返回 ref 的 canonical path 固定为 `project/{project_id}/issue/{issue_id}/plan/{plan_id}/{object_kind}/{object_id}`（object_kind 为 `source_revision`、`plan_candidate_ir`、`mechanical_report` 或 `publication_provenance`），put 将完整 scope 写入该路径；get 只接收 `expected_scope` 与该 canonical ref，不接收裸 object ID，也不允许调用方自行 split/猜 ID。各 get API 内部以同一固定 grammar 解析 canonical ref：segment 数量/名称/空 ID 任一不符先返回 `SOURCE_STORE_MALFORMED_REF`；grammar 正确但 object kind 与目标 get API 不符返回 `SOURCE_STORE_WRONG_KIND`；kind 正确但三个业务 scope 字段（`project_id`、`issue_id`、`plan_id`）与 expected scope 不符返回 `SOURCE_STORE_SCOPE_MISMATCH`；object kind 已在前一步单独校验；scope/kind/ID 全正确但对象不存在返回 `SOURCE_STORE_DANGLING_REF`。该顺序是四类失败的稳定优先级。record 的 immutable identity 是 `(object_kind, project_id, issue_id, plan_id, id)` 及其关联 `source_revision_id`/`source_revision_hash`/`compiler_version`，同 ID 同 canonical 内容与 hash 重复写成功且幂等，任何同 ID 不同内容、source hash 或 compiler version 漂移均拒绝。每个 record 的 `content_hash` 固定为排除自身后的 canonical JSON bytes lowercase SHA-256，get 与 put 都重算校验；稳定失败码另固定为 `SOURCE_STORE_IDENTITY_MISMATCH`（同 ID 不同内容）、`SOURCE_STORE_CONTENT_HASH_MISMATCH`（篡改或 hash 不符）、`SOURCE_STORE_SOURCE_HASH_MISMATCH`（source hash 漂移）、`SOURCE_STORE_COMPILER_VERSION_MISMATCH`（compiler version 漂移）。`put_publication_provenance` 与 `get_publication_provenance` 同样属于 `impl WorkItemPlanSourceStore`，put 显式带完整 scope：`put_publication_provenance(&self, project_id: &str, issue_id: &str, plan_id: &str, provenance: &PlanCandidatePublicationProvenance) -> Result<String, SourceStoreError>`；get 固定为 `get_publication_provenance(&self, expected_scope: &SourceStoreScope, canonical_ref: &str) -> Result<PlanCandidatePublicationProvenance, SourceStoreError>`，并使用同一 canonical-ref 四类解析失败。put 写入并校验与 get 完全相同的 durable scope，且 `provenance.plan_id == plan_id`，不存在 constructor 绑定或隐含 scope。对象文件保存完整 refs、hash、compiler version 与 `content_hash`，其中 provenance `content_hash` 固定为除 `content_hash` 自身外其余字段的 canonical JSON bytes lowercase SHA-256，禁止递归自哈希或只哈希 ID。`InitialPlanPublicationArtifacts` 当前仅持有产物，不能假定它拥有 provenance；publication prepare 必须在 journal/artifacts 中保存 `publication_provenance_ref` 与 `publication_provenance_content_hash`（均为 `Option` legacy-safe），publication fingerprint 覆盖这两个值，recovery 再按 expected scope+canonical ref 调 `get_publication_provenance` 并以完整内容 hash 做 identity 校验。coding runtime reader 和 `WorkItemRuntimeBinding` 不加 markdown/compiler freshness gate。

- [ ] **Step 1 — 写失败测试。** 覆盖 source bytes 修改→`SourceRevisionMismatch`；旧 compiler version→`CompilerVersionMismatch`；mechanical report hash/version 不匹配或含 Error→拒绝；分别构造 malformed ref、wrong kind、scope mismatch、dangling ref，精确断言 `SOURCE_STORE_MALFORMED_REF`、`SOURCE_STORE_WRONG_KIND`、`SOURCE_STORE_SCOPE_MISMATCH`、`SOURCE_STORE_DANGLING_REF`。全部匹配时先断言 reservation CAS 已持久化，再在其后完成 provenance 与首个 transaction put。分别在 **reservation 前 / reservation 后且 provenance 前 / provenance 后且首个 transaction put 前 / 首个 transaction put 后** 注入 crash；每次重启都断言复用同一 `compile_id`、`now`、`publication_provenance_ref` 与由 compile ID 重算的 publication IDs，且四例均不重新取时钟、不新分配 reservation。重放后 provenance 字节语义不变；篡改同 ID 被拒绝；断言 provenance 的 `plan_id`、source hash、compiler version 与 publication plan revision 绑定；coding runtime reader 只读取 immutable binding，不打开 source store。执行：

  ```bash
  cargo test --locked --lib work_item_plan_compiler::tests::publish_freshness  # 已验证匹配 0 项（当前测试尚未创建；实现后先用同一过滤名加 -- --list，必须 >=1）
  cargo test --locked --lib work_item_revision_store::tests::initial_publication  # 已验证匹配 10 项
  ```

  预期失败：freshness/provenance 尚未实现。
- [ ] **Step 2 — 实现 freshness/store/publication。** source edit 每次写新的 immutable revision ID 与 hash；按上述 typed record/API 实现三字段业务 scope path（object kind 单独校验）、重复同内容幂等与稳定 failure code；由 adapter/新路径 engine 完成读取、校验、reservation CAS、provenance 分配和首个 transaction put 前的失败关闭。`put_source_revision`、`put_plan_candidate_ir`、`put_mechanical_report` 均须在各自 Generate/Evaluate 步骤写入并重复时复用同一 immutable identity；Approval 校验通过后先 CAS reservation，再调用精确 `put_publication_provenance`，最后才允许 `put_compile_transaction`。恢复读取只能调用精确 `get_source_revision`、`get_plan_candidate_ir`、`get_mechanical_report`、`get_publication_provenance` API，Continue 不得调用未列出的读取 API。publication execute 只消费已校验引用，不在纯核心重读 source/IR/report store；不得在 coding 段重新解析 markdown 或解释 compiler version。
- [ ] **Step 3 — 运行转绿。** 两条定向命令均通过；先用同一四边界 fixture 断言 reservation/provenance/transaction 的写入顺序和 compile/publication ID 复用，再运行：

  ```bash
  cargo test --locked --lib work_item_runtime_reader
  ```

  预期 runtime reader 回归全绿。

**提交建议：** `feat(workitem): enforce publish freshness and immutable provenance`

## 工作包 3：【高风险·独立审查门】compile 事务输入抽取

> 顺序硬门：必须严格完成 3.1 legacy parity → 3.2 `InitialPlanCompileInput`/prepare-execute → 3.3 五 failpoint recovery parity；完成独立审查并获准后才能开始 3.4 IR adapter。不得先写 IR adapter 再补 characterization。

### Task 3.1：legacy 三层 parity/characterization

**可追溯性：** 契约 Task 3.1；REQ-WSC-01、REQ-WSC-07；D7、D-E。

**Files:**

- Modify: `src/product/workspace_engine/tests/part_03.rs`（登记新 test module）
- Create: `src/product/workspace_engine/tests/part_03/part_15.rs`（`part_14.rs` 已存在，不得覆盖或按 Create 使用）
- Reuse: `src/product/workspace_engine/tests/part_03/part_09.rs`
- Modify test-only instrumentation: `src/product/work_item_plan_store.rs`（`put_compile_transaction` 每次调用的 recording observer/hook；不改变生产持久化语义）
- Test integration: `tests/it_web/web_work_item_plan_compile/part_01.rs`

**Characterization output：** 新增 `NormalizedInitialCompileObservation`，精确包含 normalized plan/work-items/verification plans/runtime bindings/projection hashes；每次 `put_compile_transaction` 的完整 snapshot（至少 `status`、`step_cursor`、`plan_commit_state`、全部业务字段和 durable refs）；created record 数与稳定引用关系；finalizer 后 Confirmed plan/report/child session bindings。observer 必须在每次 `put_compile_transaction` 调用时 clone snapshot 并按测试 scope 收集，不能从覆盖写后的最终 JSON 推导中间状态；使用 thread-local/guard 或等价锁隔离并在每个 case 清理，防并发测试串数据。

归一化规则：动态 ID 使用稳定占位符映射（同一原 ID 在所有字段映射为同一占位符，保留跨字段引用关系），不得删除所有 ID；timestamp 先断言所有值为 RFC3339、同一 transaction 的 `created_at` 全程稳定，再忽略具体时间；projection/contract hash 不得忽略，必须由测试重新序列化对应产物并断言 hash 一致。正常与 recovery cursor 分开保存：正常路径必须含 `preparing → validating → committing → ... → committed`；recovery 路径必须额外含 `publication_resumed`，且其位置在 initial publication 恢复与后续 finalizer cursor 之间。provider ledger 不是空总数断言，而是保存前后字节快照、断言新增 `started=true` 数为 0，并断言事件序列没有 `EngineEvent::ProviderRunRequested`。

- [ ] **Step 1 — 先写 characterization。** 只增加 test-only observer/hook 与测试，不改 compile 流程；驱动当前 `run_work_item_plan_compile` 正常成功、validator failed、recovery required 三路径，采集真实多次 put snapshot 并保存 normalized observation。新增函数统一命名为 `work_item_plan_initial_compile_phase2_*`，先列举、再执行：

  ```bash
  cargo test --locked --lib work_item_plan_initial_compile -- --list  # 已验证匹配 4 项（当前基线；新增 characterization 后必须 >=8）
  cargo test --locked --lib work_item_plan_initial_compile
  ```

  预期通过当前 legacy 行为且非零执行；若 list 少于 8 或执行失败，先修正测试夹具对现状的误解，不能动生产流程迎合测试。
- [ ] **Step 2 — 在 3.2 前证明 transient `updated_at` 不参与选择/恢复。** 从同一 durable fixture 复制两组 transaction，仅把所有合法 RFC3339 `updated_at` 改为相反先后顺序，保持 `created_at`、compile ID、status、cursor 和内容不变；分别调用当前 `mark_latest_compile_transaction_recovery_required`、`latest_work_item_plan_recovery_transaction` 与 Continue，断言选中的 compile ID、matching 结果、recovery outcome、cursor snapshot 和最终产物完全相同。另断言改变 `created_at` 会按当前 `compile.rs:169-199,603-617` 排序基线改变选择，以防测试错误地证明“所有时间都无关”。Abort/HumanTriage 的实时 `updated_at` 只验证合法性，不混入 pure prepare。该 characterization 未绿禁止进入 3.2。
- [ ] **Step 3 — 固化语义断言。** 正常路径依序包含 `preparing→validating→committing→...→committed`，validation failure 终止于 Failed，recovery 使用同 compile ID/transaction，不新分配业务对象；动态 ID 占位符关系、RFC3339/created_at 稳定性、所有 hash 与产物一致性均显式断言。
- [ ] **Step 4 — 跑 integration characterization。**

  ```bash
  cargo test --locked --test it_web web_work_item_plan_compile -- --list  # 已验证匹配 7 项
  cargo test --locked --test it_web web_work_item_plan_compile
  ```

  预期现有 7 项 web compile tests 全绿。该命令是 integration target，不属于 `src/lib.rs` 定向单测例外。

**提交建议：** `test(workitem): characterize legacy compile transaction parity`

### Task 3.2：唯一 `InitialPlanCompileInput` 与 prepare/execute 核心

**可追溯性：** 契约 Task 3.2；REQ-WSC-01、REQ-WSC-02、REQ-WSC-07；D1、D-E。

**Files:**

- Modify: `src/product/models/outline.rs:272-298`（`WorkItemPlanCompileTransaction` durable context schema）
- Modify: `src/product/models/workspace.rs`、`src/product/lifecycle_store/workspace.rs`（复用 Task 2.5 typed compile reservation CAS）
- Modify: `src/product/workspace_engine/compile.rs`
- Modify: `src/product/workspace_engine/compile/finalizer.rs`
- Modify: `src/product/workspace_engine/plan_projection.rs`（当前 `compile_initial_plan_revision` 为 `plan_projection.rs:329-539`，必须纳入本任务）
- Modify: `src/product/workspace_engine/tests/part_03/part_15.rs`
- Modify: `tests/it_product/product_work_item_plan_store/part_01.rs`（legacy transaction JSON 兼容）

**Required exact input（契约精确 struct，不增删字段）：**

```rust
pub struct InitialPlanCompileInput {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub previous_plan: IssueWorkItemPlan,
    pub active_index: WorkItemPlanDraftActiveIndex,
    pub outline_candidate: WorkItemPlanOutlineCandidateDto,
    pub outline_order: Vec<String>,
    pub draft_records: Vec<WorkItemDraftRecord>,
    pub logical_targets: Option<BTreeMap<LogicalRepositoryId, String>>,
    pub repository_id: String,
    pub change_order: Vec<LogicalRepositoryId>,
    pub compile_id: String,
    pub now: String,
}
```

**Durable schema（不改上面的 13 字段 input）：** 在 `outline.rs` 为 `WorkItemPlanCompileTransaction` 增加下列字段；所有新增字段必须是 `Option<…>` 且逐字段 `#[serde(default, skip_serializing_if = "Option::is_none")]`，旧 transaction JSON 反序列化时全部为 `None`。`effective_flow_kind()` 将缺失/`None` 解释为 `WorkItemPlanFlowKind::Legacy`，不得把 default 填成 SingleCandidate。

```rust
pub flow_kind: Option<WorkItemPlanFlowKind>,
pub source_revision_id: Option<String>,
pub source_revision_ref: Option<String>,
pub plan_candidate_ir_ref: Option<String>,
pub mechanical_report_ref: Option<String>,
pub publication_provenance_ref: Option<String>,
pub publication_provenance_content_hash: Option<String>,
```

`prepare_initial_plan_compile(input, durable_context)` 只做确定性 projection、validator input 和 transaction draft 构造；`durable_context` 是独立参数，Legacy 为全 `None`，SingleCandidate 必须为 `flow_kind=Some(SingleCandidate)` 且其余六个 durable 字段全 `Some`。`execute_initial_plan_compile(stores, prepared)` 才持有 put/commit/finalizer 所需 writer。`CompileStores`、`PreparedInitialPlanCompile` 的其余 fields/ownership 以实施时 `compile.rs` 当前数据流为准。SingleCandidate execute 入口的第一个 durable 写入必须是 Task 2.5 的 `put_compile_reservation_cas`，只有 CAS 成功或同三元组幂等命中后，才允许任何 provenance/transaction put；reservation 的 `compile_id`、`now`、`publication_provenance_ref` 不得由 prepare 重算或覆盖。首个 `put_compile_transaction` 必须已经包含完整 durable context；字段只允许从已验证的外层 context 一次性复制，此后每次 journal put 原样保留。四个 crash 边界（reservation 前、reservation 后/provenance 前、provenance 后/transaction 前、首个 transaction put 后）均必须重启复用该 reservation 三元组及确定性 publication IDs。

必须同时重构 `src/product/workspace_engine/plan_projection.rs:329-539`：从 `compile_initial_plan_revision` 提取输入式 publication preparation，消费且只消费 `previous_plan`、`outline`、`outline_order`、accepted draft revisions、compile ID、注入的 `now`、外层已分配 publication IDs 和独立 provenance snapshot；该 snapshot 来自 Task 2.5 唯一 owner API，并包含 ref+content hash。纯 preparation 不重新读取 lifecycle、latest outline、matching transaction 或任何 store。legacy wrapper 负责当前 store/lifecycle 读取、accepted draft 排序及 ID 分配；IR adapter 负责 IR 投影为相同 publication preparation input；二者汇入同一 publication prepare/execute 路径。`CompileStores` 只能承载 execute 所需 writer，不能成为重新读取 legacy input 的后门。共享 prepare/execute/finalizer 不得直接调用 `Utc::now()`；所有 initial compile 时间来自 `input.now`，Abort/HumanTriage 等后续人工动作保留实时钟。

- [ ] **Step 1 — 写失败测试。** 从同一 fixture 分别调用旧 wrapper 与新纯 prepare，断言 projection/validator input/initial transaction normalized 相等；同一 input 两次 prepare 完全相等；无 store 句柄的 pure test 可运行；另断言 `compile_initial_plan_revision` 的 store-coupled wrapper 与 input式 publication preparation 共享同一 projection/execute 结果。执行：

  ```bash
  cargo test --locked --lib work_item_plan_initial_compile -- --list  # 已验证匹配 4 项（当前基线；新增 3.2 tests 后必须 >=11）
  cargo test --locked --lib work_item_plan_initial_compile
  ```

  预期失败：类型/函数或 `plan_projection.rs:329-539` 的输入式 preparation 尚不存在。
- [ ] **Step 2 — 先锁 durable JSON 兼容。** 反序列化一份缺少全部新字段的真实 legacy transaction fixture，断言七字段均为 `None`、`effective_flow_kind()==Legacy`；SingleCandidate fixture roundtrip 后字段逐字不变；任何缺少一项 ref/hash 的 SingleCandidate context 在首个 put 前失败；另断言 session 的 typed `compile_reservation` 先经 CAS 落盘，随后 provenance/transaction 才允许写入，四个 crash 边界重启均复用同一三元组。
- [ ] **Step 3 — 提取 legacy adapter。** 严格按当前 `compile.rs` 的输入读取顺序组装 input，完成外层 ID/时间注入；外层 durable context 全 `None`。不得把 store 读取塞回 pure core。
- [ ] **Step 4 — 实现 prepare/execute 与 publication preparation。** `run_work_item_plan_compile` 只做 legacy input assembly 后调用共享 core；`compile_initial_plan_revision` 只保留 legacy wrapper 读取职责；IR projection 和 legacy accepted revisions 调用同一 publication prepare/execute；将 initial compile 共享路径中的直接 `Utc::now()` 全部改为 `input.now`。SingleCandidate execute 先重放 durable reservation CAS，再按确定性 ref 写 provenance、首个 transaction 和后续 journal；四个 crash boundary 的恢复不得重新分配 compile/publication IDs。3.1 已负责证明 transient `updated_at` 不影响选择/恢复，本步只重跑，不得把证明后移到此。
- [ ] **Step 5 — 运行转绿。** 先用 `-- --list` 核实新增后匹配至少 11 项，再执行同一过滤名；同时重跑 3.1 characterization。预期 normalized semantic artifacts、hash、durable context、状态序列和恢复结果无差异。

**提交建议：** `refactor(workitem): extract initial compile input and publication core`

### Task 3.3：五 checkpoint failpoint/recovery parity

**可追溯性：** 契约 Task 3.3；REQ-WSC-01、REQ-WSC-07；D-E。

**Files:**

- Modify: `src/product/workspace_engine/tests/part_03/part_09.rs`
- Modify: `src/product/workspace_engine/tests/part_03/part_15.rs`
- Modify: `src/product/work_item_revision_store/tests/initial_publication.rs`
- Modify only if parity defect is found: `src/product/workspace_engine/compile.rs`、`src/product/workspace_engine/compile/finalizer.rs`

- [ ] **Step 1 — 锁定五个 finalizer checkpoint。** 对 `PlanSummaryPrepared`、`FirstChildSessionEnsured`、`FirstChildBindingEnsured`、`FirstChildContextPrepared`、`CompileReportPersisted` 逐个注入 failpoint→捕获 RecoveryRequired transaction→销毁 engine→`WorkspaceEngine::new_persistent`→Continue→比较 3.1 observation；断言同 compile ID、child IDs/bindings/context/report、一个 committed transaction、provider ledger 前后字节不变、新增 started 数 0、无 `EngineEvent::ProviderRunRequested`。
- [ ] **Step 2 — 增加真正穿过 initial publication 的 case。** 另对 `InitialPlanPublicationCheckpoint::{LineageWritten, FirstWorkItemArtifactsWritten, PlanArtifactsWritten, FirstWorkItemActivated, PlanActivated}` 五项逐个中断 initial publication，确保 transaction 尚未得到 outcome；销毁 engine 后 Continue 必须进入 `resume_initial_plan_compile_transaction`。每例在 recording observer 中精确断言 `publication_resumed` 出现在该 initial-publication 恢复后、首个后续 finalizer cursor 前；断言同 publication journal/fingerprint/IDs、完整 provenance 内容 hash、refs、ledger/event 前缀及不可变产物无重复。五个 finalizer checkpoint 自身不得被用来替代本 case。
- [ ] **Step 3 — 先列举再执行。** 以下现有过滤名已于 2026-06-30 用 `-- --list` 实测，均匹配 1 项；新增 `work_item_plan_initial_compile_phase2_*` 完成后，`work_item_plan_initial_compile` 必须至少匹配 13 项：

  ```bash
  cargo test --locked --lib compile_recovery_continue_replays_each_partial_finalizer_checkpoint_after_restart -- --list  # 已验证匹配 1 项
  cargo test --locked --lib compile_recovery_continue_replays_pre_active_publication_with_same_tx_after_restart -- --list  # 已验证匹配 1 项
  cargo test --locked --lib initial_plan_publication_resumes_each_store_write_failure_after_restart -- --list  # 已验证匹配 1 项
  cargo test --locked --lib work_item_plan_initial_compile -- --list  # 已验证匹配 4 项（当前基线；新增后必须 >=13）
  cargo test --locked --lib compile_recovery_continue
  cargo test --locked --lib initial_plan_publication
  cargo test --locked --lib work_item_plan_initial_compile
  ```

  预期新增 initial-publication/observer parity helper 在接线前失败，现有三个 recovery case 保持绿。
- [ ] **Step 4 — 修复提取造成的差异并转绿。** 只修 ownership/调用顺序/重放幂等差异；不得减少 journal put、跳过 finalizer cursor、吞掉 `publication_resumed` 或把 journal 改成新事务模型。最终用 `cargo test --locked --lib workspace_engine::tests -- --list` 核实当前基线已匹配 881 项且新增后更多，再运行 `cargo test --locked --lib workspace_engine::tests`；provider ledger 继续断言“前后字节不变 + 新增启动 0 + 无 ProviderRunRequested”，不得改为总数等于 0。
- [ ] **Step 5 — 独立审查门。** 审查者核对 3.1 observer baseline、精确 `InitialPlanCompileInput` 字段、pure prepare 无 store read、`plan_projection.rs:329-539` 已拆出输入式 publication preparation、正常/recovery cursor（含 `publication_resumed`）、五 checkpoint 重放、唯一注入 now、hash/ID/timestamp normalization 未过度忽略字段。审查结论通过前禁止开始 Task 3.4。

**提交建议：** `test(workitem): prove compile failpoint recovery parity`

### Task 3.4：validated IR adapter 汇入共享核心

**可追溯性：** 契约 Task 3.4；REQ-WSC-01、REQ-WSC-02、REQ-WSC-07；D1、D-E。

**Files:**

- Create: `src/product/workspace_engine/compile/ir_adapter.rs`
- Modify: `src/product/models/outline.rs:272-298`（复用/验证 Task 3.2 durable schema）
- Modify: `src/product/workspace_engine/compile.rs`
- Modify: `src/product/workspace_engine/compile/finalizer.rs`
- Modify: `src/product/workspace_engine/plan_projection.rs`
- Modify: `src/product/work_item_revision_store/initial_publication.rs`
- Modify: `src/product/workspace_engine/tests/part_03/part_15.rs`
- Modify: `src/product/work_item_revision_store/tests/initial_publication.rs`
- Reuse: `src/product/work_item_plan_source_store.rs`（唯一 durable owner；精确 `put_source_revision/get_source_revision`、`put_plan_candidate_ir/get_plan_candidate_ir`、`put_mechanical_report/get_mechanical_report`、`put_publication_provenance/get_publication_provenance` API）

**Adapter context（不改 `InitialPlanCompileInput`）：**

```rust
pub(crate) struct IrCompileAdapterContext {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub previous_plan: IssueWorkItemPlan,
    pub source_revision_id: String,
    pub source_revision_ref: String,
    pub plan_candidate_ir_ref: String,
    pub mechanical_report_ref: String,
    pub publication_provenance_ref: String,
    pub logical_targets: Option<BTreeMap<LogicalRepositoryId, String>>,
    pub repository_id: String,
    pub change_order: Vec<LogicalRepositoryId>,
    pub compile_id: String,
    pub now: String,
}
```

`initial_plan_compile_input_from_ir(context, ir, mechanical_report)` 只组装契约精确的 `InitialPlanCompileInput`；相邻的 `durable_compile_context_from_ir(&context, &provenance)` 精确返回 Task 3.2 七个 optional 字段，首个 put 前随 `prepare_initial_plan_compile(input, durable_context)` 写入 transaction。这些 refs 不进入 input struct。freshness 校验发生在 adapter 外层/SingleCandidate engine：唯一 source store 的 get API **直接接收 canonical ref**，即 `get_source_revision(expected_scope: &SourceStoreScope, canonical_ref: &str)`、`get_plan_candidate_ir(expected_scope: &SourceStoreScope, canonical_ref: &str)`、`get_mechanical_report(expected_scope: &SourceStoreScope, canonical_ref: &str)`；调用方按完整 `(project_id, issue_id, plan_id)` expected scope 依次传入 session/transaction 中持久化的 refs，store 先解析并核对 canonical path/object kind/object ID，再校验 bytes/content hash、IR source hash/compiler version、mechanical report source hash/compiler version 与零 Error。解析/读取失败码固定为 `SOURCE_STORE_MALFORMED_REF`（无法匹配 canonical grammar）、`SOURCE_STORE_WRONG_KIND`（ref 的 object kind 与目标 API 不符）、`SOURCE_STORE_SCOPE_MISMATCH`（三个业务 scope 字段不符；object kind 已单独校验）、`SOURCE_STORE_DANGLING_REF`（格式正确但对象不存在）；不得由调用方自行 split/猜 ID。若 transaction 已存在，**Continue 只能从 durable transaction 的 `source_revision_ref`/`plan_candidate_ir_ref`/`mechanical_report_ref` 读取这些 refs，在三对象全部重载并校验成功后才进入共享 pure prepare**，不依赖重启前 `IrCompileAdapterContext` 的任何内存 ID；随后以同一 canonical-ref 约定的 `get_publication_provenance(expected_scope: &SourceStoreScope, canonical_ref: &str)` 重载并核对完整 provenance content hash，且只允许后续 journal put/resume，不得再次创建或声称“首个 transaction put”。若 reservation/provenance 已 durable 但 transaction 尚不存在，则这是 session recovery：从 durable session 的 source/IR/report refs 重载并校验三对象，再按 provenance ref 重载完整 provenance，随后才创建首个 transaction；该分支同样不得依赖重启前 `IrCompileAdapterContext` 的任何内存 ID。identity/hash/version 漂移使用 Task 2.5 稳定失败码；不得调用未定义的通配读取 API 或另造 ref→ID 隐含 API。publication execute 只消费已校验 snapshot，`WorkItemPlanRevision.publication_provenance_ref`、journal ref/hash 与 transaction ref/hash 必须逐字相等，legacy 全为 `None`。

adapter 必须确定性构造 active index、outline candidate/order 与 accepted draft revisions；这些派生值只能来自已重载的 typed objects，不能读 active index/draft store、猜 path、重跑 Provider或改变 transaction/finalizer。Continue 先调用 `tx.effective_flow_kind()`：Legacy 才允许当前 `finalizer.rs:14-41` 的 active index/outline/draft 读取；若 reservation/provenance 已 durable 但 transaction 尚不存在，SingleCandidate session recovery 先从 durable session 重载并校验 source/IR/report/provenance，再创建首个 transaction；已有 transaction 的 SingleCandidate Continue 必须先验证 durable transaction 的四个 canonical refs 全为 `Some`，绑定为 `source_ref`、`ir_ref`、`report_ref`、`provenance_ref` 后，按完整 expected scope 依次调用 `WorkItemPlanSourceStore::get_source_revision(&scope, source_ref)`、`get_plan_candidate_ir(&scope, ir_ref)`、`get_mechanical_report(&scope, report_ref)` 重载 source revision、IR、mechanical report，API 内部完成 canonical ref 解析与 object kind/ID 校验、immutable identity、content hash、source hash 与 compiler version 校验，三对象全部成功后才进入共享 pure prepare；随后按同一 scope 调用 `get_publication_provenance(&scope, provenance_ref)` 校验完整 provenance。不得调用未列出的读取 API、依赖 `IrCompileAdapterContext` 重启前内存 ID，或实现隐含 API。SingleCandidate 分支对任何 legacy active-index/outline/draft read 注入 panic spy，证明该读路径从未调用；malformed/wrong-kind/scope-mismatch/dangling ref 及 identity/content hash/source hash/compiler version 漂移一律按 Task 2.5 稳定 failure code durable fail-closed，并继续同一 compile ID/now/reservation、journal/provenance 与确定性 publication IDs。

- [ ] **Step 1 — 写失败测试。** legacy fixture 与同 canonical 数据构造的 IR fixture 分别进入共享 prepare/execute；断言 normalized plan/work items/dependencies/verification/trusted commands/projection hashes、journal status/cursor、finalizer 结果一致。另断言 stale IR、mechanical Error、target map 缺 logical ID 都在首个 transaction put 前失败；先断言 Approval compile-publication 边界内 reservation CAS 是唯一首个 durable 写入，再分别覆盖 reservation 前、reservation 后/provenance 前、provenance 后/transaction 前、首个 transaction put 后四个 crash 边界，销毁 engine 后 Continue 仍复用同一 compile ID/now/reservation/provenance/publication IDs；断言 provenance ref 逐字落在 `WorkItemPlanRevision.publication_provenance_ref`，且无重复 publication/provider start。执行：

  ```bash
  cargo test --locked --lib work_item_plan_initial_compile -- --list  # 已验证匹配 4 项（当前基线；3.1～3.4 完成后必须 >=18）
  cargo test --locked --lib work_item_plan_initial_compile
  ```

  预期失败：IR adapter 或 durable reference/recovery 分支不存在。
- [ ] **Step 2 — 实现 adapter 与 publish 边界。** `logical_targets` 类型严格保持 `Option<BTreeMap<LogicalRepositoryId, String>>`；无 map 时使用注入 physical repository ID；change order 只用注入值。Generate/Evaluate 先通过精确 typed API 持久化 source revision、IR、mechanical report；Approval 在 adapter/新路径 engine 读取并完成 freshness 校验后，先由 durable Approval tuple 计算并 CAS 持久化 `SingleCandidateCompileReservation`，再按固定顺序完成 provenance put/get 与首个 transaction put，任何前置写入失败均不得越过 reservation 或 fallback legacy。
- [ ] **Step 3 — 增加重启测试再转绿。** 对五个 `InitialPlanPublicationCheckpoint` 至少选择 `PlanArtifactsWritten` 中断 IR initial publication，销毁 engine，再由 persistent engine Continue；observer 必须捕获 `publication_resumed`，并断言同 compile ID、now、reservation 三元组、child/publication IDs、transaction 七字段、完整 provenance 对象/content hash、无重复 immutable 产物且 provider ledger 无新增。另构造旧 transaction JSON 缺 `flow_kind`，证明只走 Legacy。
- [ ] **Step 4 — 运行转绿。** 先确认 `work_item_plan_initial_compile` 匹配至少 18 项并执行；再运行 `cargo test --locked --lib work_item_revision_store::tests::initial_publication -- --list`（已验证匹配 10 项）及不带 `-- --list` 的同命令。预期 legacy/IR publication parity、revision-store initial publication recovery、durable provenance reload 全绿。

**提交建议：** `feat(workitem): adapt validated ir to shared compile core`

## 工作包 4：prompt 重构与复评 invocation

### Task 4.1：author prompt 改为内联 markdown 规格与真实判例

**可追溯性：** 契约 Task 4.1；REQ-WSC-02、REQ-WSC-06；D3、D6。

**Files:**

- Modify: `src/product/work_item_split_engine/prompts.rs`（当前 `build_split_prompt`、`build_work_item_draft_prompt` 及其 runtime contract；写前用 `ast-grep outline src/product/work_item_split_engine/prompts.rs --items structure --view signatures` 核实）
- Modify: `src/product/work_item_split_engine/parse.rs`
- Modify: `src/product/work_item_split_engine/tests/prompt_contract.rs`
- Reuse: `src/product/work_item_plan_compiler/grammar.rs`
- Reuse: `src/product/work_item_plan_policy/fixtures/golden_findings.json`

**Interfaces / generation boundary:**

```rust
pub(crate) fn build_work_item_plan_markdown_prompt(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    story_context: &str,
    design_context: &str,
    repository_structure: &str,
    routing_context: &RoutingReferenceContext,
) -> Result<String, String>;
```

provider 输出直接作为 markdown source revision 交 compiler parse；不得先转私有 JSON 再转 markdown。prompt 内联 grammar 全文、最小合法 source 和真实判例，不能只给 Aria 仓路径；few-shot 使用九条 reviewer 原始 finding + 两条 rep1 Advisory 的“错误模式→修正原则”，不加入三个 classifier-only 人工变体。旧 `build_split_prompt`/`build_work_item_draft_prompt` 的精确借用/返回形状以当前源码为准，不在计划中虚构不存在的参数。

内部 generation 子流程在此任务与 Task 5.2 共同固定为：先调用轻量 markdown outline invocation；对 outline 做本地、无 Provider 的机械解析，得到 `candidate_item_count`；再以 provider capability/budget 和该 count 调用 `select_internal_generation_mode`，选择 batch/serial；最后调用完整 markdown author invocation，并把 source revision 写入 durable source store。若轻量 outline 本身无法解析，走 compiler diagnostic/fatal，禁止猜测 count 或回到客户端选择；legacy flow 不走该子流程。

- [ ] **Step 1 — 写失败测试。** 精确断言 prompt 含 13 个结构化 section 名、EARS 模板、unknown-field fail-closed、rep4 两个矛盾的修正判例；不含“请读取 aria 仓 fixture”作为唯一 grammar；不要求深层 JSON/nonce sentinel；prompt bytes 小于既有质量预算。补充 invocation 顺序 spy：SingleCandidate 为 `outline → mechanical count → mode selector → full markdown author → parse/source revision`，旧 flow 保持原 builder/parser。执行：

  ```bash
  cargo test --locked --lib work_item_split_engine::tests::prompt_contract::work_item_plan_markdown
  ```

  预期失败：builder 尚未改为 markdown 或顺序/计数尚未接线。
- [ ] **Step 2 — 实现 markdown author 与内部 outline。** grammar、最小 source 和 few-shot 以内联内容提供；provider 输出只进入 source revision/compiler；不得以深层 JSON、nonce 或客户端输入补齐 markdown。Legacy 分支由新增 flow-kind 接线任务继续使用旧 builder/parser。
- [ ] **Step 3 — 运行转绿。** 同一命令通过，字节上限和 invocation 顺序断言稳定；不调用真实 Provider。

**提交建议：** `feat(workitem): prompt authors for markdown plan source`

### Task 4.2：删除 B 层，保留 C 层单一职责

**可追溯性：** 契约 Task 4.2；REQ-WSC-06；D6。

**Files:**

- Modify: `src/product/work_item_split_engine/prompts.rs`
- Modify: `src/product/work_item_split_engine/tests/prompt_contract.rs`
- Modify: `src/product/work_item_split_engine/tests/routing_reference_contract.rs`

**已核实的逐句删除清单（以当前源码精确文本为准，实施时再核对漂移）：**

1. `work_item_plan_runtime_contract` 当前 `prompts.rs:49`，位于 `[superpowers_contract]` **之前**：`必调 Skill：using-superpowers → writing-plans。`
2. 同函数当前 `:58-68`：删除 `[superpowers_contract]` marker 及其中全部 10 句行为教学（using-superpowers/writing-plans、TDD、单会话/最少拆分、40k/50k context、session 拆分理由等）。保留前面的 `[openspec_contract]` traceability/blocker/writeback 边界和后面的 allowed/forbidden outputs。
3. `work_item_draft_runtime_contract` 当前 `:83`，位于 section **之前**：从阶段句中仅删除分号后的 `必调 Skill：using-superpowers → writing-plans。`，保留“当前阶段”本身。
4. 同函数当前 `:87-88`：删除 `[superpowers_contract]` marker 及整句行为教学（using-superpowers/writing-plans/TDD/单会话/50k）。
5. `build_work_item_draft_prompt` 当前 `:744`：删除“必须读取并遵守 writing-plans 的拆分、TDD、验证与交接质量纪律”整条 B 层 hard rule。

**逐句保留清单：** 当前 `:721`“不得输出 writing-plans 的 Markdown Plan 或新增 JSON 字段”、`:745`“不得提前执行 writing-plans 的落盘步骤”是 C 层输出/写回边界，必须逐字保留；`:764-765` `target_retain_instruction`、`OUTLINE_WRITE_SCOPE_RULES`、traceability/scope/canonical/verification/trusted command/allowed/forbidden outputs 同样保留。因此测试不得全局禁止 token `writing-plans`；只禁止上述 B 层精确句和 `[superpowers_contract]` section，并正向断言两个 C 层 writing-plans 句仍各恰好一次。

- [ ] **Step 1 — 写失败测试。** 修改现有 `single_item_prompt_scopes_writing_plans_to_pre_confirmation_candidate` 等与旧 B 层绑定的 tests，新增 `behavior_layer_removed_but_output_boundaries_retained`：逐条断言 1～5 精确文本不存在、`[superpowers_contract]` 不存在；断言两个 C 层 writing-plans 句、`traceability`、`forbidden_outputs`、`exclusive_scopes`、`verification`、`handoff schema`、候选不可写回边界及 `target_repository_id` 保留规则存在。先列举后执行：

  ```bash
  cargo test --locked --lib single_item_prompt -- --list  # 已验证匹配 19 项；新增后必须 >=20
  cargo test --locked --lib single_item_prompt
  ```

  预期基线因 B 层仍存在而失败；若 Task 4.1 已按同清单移除则允许直接通过并记录覆盖来源，不人为制造失败。
- [ ] **Step 2 — 按清单最小删除并再次全文件扫描。** 执行 `rg -n -C 1 'using-superpowers|writing-plans|test-driven-development|50k|单 session|Skill|TDD|拆分' src/product/work_item_split_engine/prompts.rs`，逐条与删除/保留清单核对；不得只搜索 section 内文本，也不得删除 C 层安全/产物边界。
- [ ] **Step 3 — 运行转绿。** `single_item_prompt` 必须非零执行且全绿；再以 `cargo test --locked --lib work_item_split_engine::tests -- --list` 核实整个扁平 tests 模块有非零匹配后运行 `cargo test --locked --lib work_item_split_engine::tests`。预期 routing context、target repository 保留和 C 层约束全绿。

**提交建议：** `refactor(workitem): remove duplicated behavior teaching from prompts`

### Task 4.2a：按 durable `flow_kind` 接线 legacy 与 SingleCandidate

**可追溯性：** 契约 Task 4.1、4.2；REQ-WSC-01、REQ-WSC-06、REQ-WSC-07；D6、D7。

**Files:**

- Modify: `src/product/workspace_engine/mod.rs`
- Modify: `src/product/workspace_engine/single_candidate.rs`
- Modify: `src/product/workspace_engine/types.rs:189-214`（`ProviderRunKind` 定义）
- Modify: `src/product/work_item_split_engine/prompts.rs`
- Modify: `src/product/work_item_split_engine/parse.rs`
- Modify: `src/web/workspace_ws_handler/decisions/inbound.rs`（`StartGeneration` 当前 `:510-559`）
- Modify: `src/web/workspace_ws_handler/socket.rs`（outline resume 当前 `:383-463`）
- Modify: `src/web/workspace_ws_handler/run/provider_run.rs:160-303`（真实 builder/provider 执行面）
- Create: `src/web/workspace_ws_handler/tests/single_candidate_provider_run.rs`
- Modify: `src/web/workspace_ws_handler/tests.rs`（登记真实 handler/provider-run tests）
- Create: `src/product/workspace_engine/tests/single_candidate_flow_dispatch.rs`
- Modify: `src/product/workspace_engine/tests.rs`

**接线契约：** `StartGeneration`/retry/reconnect 从 durable `WorkspaceSessionRecord.flow_kind` 构造显式 run kind，例如 `ProviderRunKind::WorkItemPlanLegacyAuthor` 或 `ProviderRunKind::WorkItemPlanSingleCandidateAuthor`；run kind 自身携带已判定 flow，provider runner 禁止再次读取 rollout flag。必须纠正现状描述：当前 Start 生成 `ProviderRunKind::WorkItemPlanAuthor`，随后 `provider_run.rs:285-303` 调 `build_outline_invocation`，输出经 `complete_work_item_plan_outline_author_from_output`/旧 outline parser 完成；当前 Start **不调用 `build_split_prompt`**。Legacy 新分支保持这条 outline→旧 JSON outline/draft/batch parser 链路逐字等价；SingleCandidate 分支调用 markdown outline/full author builders、compiler parse/lower/source revision 和 `single_candidate` engine。preflight 后不允许依据 feature flag 再猜测或静默切换。

- [ ] **Step 1 — 写 engine 分发失败测试。** 使用 legacy（含旧 JSON 缺 `flow_kind`）与 SingleCandidate 两个持久 session fixture，断言 Start/Retry/reconnect 生成不同显式 run kind；SingleCandidate 不产生 generation-mode/draft/batch 决策。该层只证明 durable state→run kind，不声称 provider 执行端到端。
- [ ] **Step 2 — 写真实执行链集成测试。** 在 `workspace_ws_handler/tests/single_candidate_provider_run.rs` 用真实 `handle_workspace_inbound_message`、`spawn_provider_run_from_handler` 和 recording fake provider：legacy provider 返回 outline sentinel JSON，断言链路 `handler → WorkItemPlanLegacyAuthor → provider_run → build_outline_invocation → complete_work_item_plan_outline_author_from_output`；SingleCandidate provider 返回最小合法 markdown，断言链路 `handler → WorkItemPlanSingleCandidateAuthor → provider_run → markdown builder → compiler parser/lower → durable source revision`。同时覆盖 reconnect/retry run kind，断言两条 parser spy 互斥，不能只在 `workspace_engine::tests` 断言。
- [ ] **Step 3 — 实现单一分发。** 在 handler/reconnect/retry 入口读取持久值并传入显式 run kind；provider runner 对两个 variant 分开 builder/parser arm；legacy 与新路径不得调用对方 parser、repair compatibility 或 fallback。
- [ ] **Step 4 — 先列举再转绿。** 当前 `cargo test --locked --lib workspace_ws_handler::tests -- --list` 已验证匹配 44 项；新增两条集成 case 后必须 >=46，再执行 `cargo test --locked --lib workspace_ws_handler::tests`。另用新增函数前缀的 `-- --list` 确认恰好命中 legacy/SingleCandidate 两案。预期真实 handler→runner→parser 两条链路全绿。

**提交建议：** `feat(workitem): dispatch generation by durable flow kind`

### Task 4.3：服务端生成 Initial/Verification invocation scope

**可追溯性：** 契约 Task 4.3；REQ-WSC-03、REQ-WSC-06；D2、D3、D-B。

**Files:**

- Modify: `src/product/workspace_engine/prompts/review.rs`
- Modify: `src/product/workspace_engine/review/routing.rs`
- Modify: `src/web/workspace_ws_types/in_.rs:8-113`（`WsInMessage`）
- Modify: `src/web/workspace_ws_handler/socket.rs:465-526`（raw text 解析与 envelope 透传；阻断项真实落点）
- Modify: `src/web/workspace_ws_handler/protocol.rs`（阶段验证和 message type 当前约 22-133）
- Modify: `src/web/workspace_ws_handler/decisions/inbound.rs:4-45`（context/envelope 接口）与消息 dispatch
- Create: `src/web/workspace_ws_handler/tests/single_candidate_scope_rejection.rs`
- Modify: `src/web/workspace_ws_handler/tests.rs`
- Create: `src/product/workspace_engine/tests/single_candidate_prompt.rs`
- Modify: `src/product/workspace_engine/tests.rs`（登记 test module）

**Interfaces / protocol boundary:**

```rust
pub(crate) fn review_scope_instructions(
    scope: &ReviewInvocationScope,
) -> Result<String, String>;
```

Initial 只允许一次全候选评估；Verification 只允许原 fingerprints 重现检查，并要求 invocation 指向 immutable mechanical report。prompt 要求 must_fix 仅限机械漏网硬错误/明确自相矛盾，完备度意见 advisory，每个 finding 提供 category/class_hint 建议；最终分类仍由阶段 1 策略层决定。scope 与 digest 只由服务端构造并持久化，Provider/campaign 不得提交 scope。

**Raw-object 通路与定向拒绝：** 不能只修改 Rust enum或在 `decisions/inbound.rs` 检查 enum；当前 raw text 在 `socket.rs:472` 已先 `serde_json::from_str::<WsInMessage>`，未知字段会丢失。新增 `parse_workspace_inbound_text(text) -> Result<WorkspaceInboundEnvelope, serde_json::Error>`：先解析 `serde_json::Value::Object` 并收集顶层 `submitted_fields: BTreeSet<String>`，再把同一 Value `serde_json::from_value::<WsInMessage>`；socket 将完整 envelope 传给 `handle_workspace_inbound_message`。当 durable `flow_kind=SingleCandidate` 且 `submitted_fields` 含 `scope`、`review_invocation_scope` 或 `review_scope` 时，handler 在 stage validation/engine dispatch/任何 session/history/CAS 写入前返回精确 `SINGLE_CANDIDATE_SCOPE_FORBIDDEN` `ProtocolError`；legacy 保持未知字段兼容。marker 只记录字段名，不持久化/打印客户端值。

- [ ] **Step 1 — 写失败测试。** Initial/Verification prompt 与 digest 测试照旧；另通过真实 socket parser 输入三种带 scope 字段的 JSON，先证明 `envelope.message` 可正常反序列化、`submitted_fields` 保留字段，再把 envelope 交 handler，精确返回 `SINGLE_CANDIDATE_SCOPE_FORBIDDEN`。对每案比较拒绝前后 session JSON、history、timeline/event bytes 完全相同；legacy 同 payload 继续进入原处理。直接构造 `WsInMessage` 的测试不算 raw 通路覆盖。
- [ ] **Step 2 — 实现 scope builder、durable write、raw envelope 与 handler 检查。** 复用阶段 1 `ReviewPhase` / `ReviewInvocationScope::{initial,verification}` 与 canonical digest，不重定义算法；在任何 stage/CAS/engine 写入前检查 marker；Initial/Verification count 只在 invocation 成功 durable 后增加。
- [ ] **Step 3 — 先列举再转绿。** 新增 scope rejection 函数名前缀完成后，用 `cargo test --locked --lib workspace_ws_handler::tests::single_candidate_scope_rejection -- --list` 确认匹配至少 4 项，再执行同一过滤名；scope builder 同理先列举、确保非零再执行。然后运行：

  ```bash
  cargo test --locked --lib work_item_plan_policy::tests_types
  ```

  预期 digest/serde 边界保持绿。

**提交建议：** `feat(workitem): derive reviewer prompt from durable scope`

### Task 4.4：复评 parser/classifier 失败关闭

**可追溯性：** 契约 Task 4.4；REQ-WSC-03、REQ-WSC-07；D2、D-B。

**Files:**

- Modify: `src/product/workspace_engine/review/routing.rs`
- Modify: `src/product/workspace_engine/review/structured_output.rs`
- Modify: `src/product/work_item_plan_policy/classify.rs`（只接真实 mechanical report ref，不改变分类规则）
- Modify: `src/product/workspace_engine/tests/single_candidate_prompt.rs`

**Exact behavior:** Verification finding 只能重现 `original_fingerprints`；本 invocation 对应的 `PlanCandidateMechanicalReport` 必须存在、source hash/version 与 repaired IR 相等、ref 与 scope 相等。缺报告、ref/digest 不符为 `Fatal(ProtocolViolation)` 并 durable failed；新 fingerprint 沿用阶段 1 已裁决的 `HumanRequired(VerificationNewFindings)`；不做 changed-path/region 归因；复评后 repairable 不再自动返修。

- [ ] **Step 1 — 写失败测试。** 分别构造：报告缺失、report ref 错、hash 错、scope digest 错→`failed` + protocol diagnostic；原指纹重现→按预算进入 human/stop，不二次 repair；新 fingerprint→HumanRequired；`provider_start_ledger` 无第二个 repair start。执行：

  ```bash
  cargo test --locked --lib workspace_engine::tests::single_candidate_prompt::verification_scope
  ```

  预期失败：新路径仍可能使用 `legacy_mechanical_report` 或缺真实 report 检查。
- [ ] **Step 2 — 实现真实 report 解析/校验。** parser error 不得经 fallback 降为 NeedsHuman；保持 UnknownCategory/UnknownClassHint 的阶段 1 fatal 链路。
- [ ] **Step 3 — 运行转绿。** 同一命令通过，再执行：

  ```bash
  cargo test --locked --lib work_item_plan_policy::tests_classify
  cargo test --locked --lib workspace_engine::tests::severity_three_tier
  ```

  预期 classifier 14/14 与 severity 既有 tests 全绿。

**提交建议：** `fix(workitem): fail closed on invalid verification scope`

### Task 4.5：prompt/scope 合并门禁与重连

**可追溯性：** 契约 Task 4.5；REQ-WSC-03、REQ-WSC-06、REQ-WSC-07；D2、D3、D-B。

**Files:**

- Modify: `src/product/work_item_split_engine/tests/prompt_contract.rs`
- Modify: `src/product/workspace_engine/tests/single_candidate_prompt.rs`
- Modify only for discovered defects: Task 4.1～4.3 production files（含补充 Task 4.2a；不存在 Task 4.3a）

- [ ] **Step 1 — 写合并测试。** 覆盖 required markdown/C layer 内容存在、B layer/JSON schema 不存在、prompt byte limit、review calibration、scope durable JSON roundtrip、engine 重建后 scope/digest 不漂移、Initial/Verification 范围违例失败关闭和 flow_kind 分发。若前置任务已覆盖某断言并直接通过，记录其覆盖来源，不人为制造失败或放宽契约。执行：

  ```bash
  cargo test --locked --lib work_item_split_engine::tests::prompt_contract
  cargo test --locked --lib workspace_engine::tests::single_candidate_prompt
  ```

  预期缺少对应接线时失败；若前置任务已提供完整覆盖则允许直接通过，并在验收记录中列明覆盖来源。
- [ ] **Step 2 — 只修合并缺口。** 不通过扩大字节上限、删除 C 层字段或放宽 scope 校验让测试变绿。
- [ ] **Step 3 — 运行转绿。** 两条命令均 0 failed；保存测试输出到验收记录，不调用真实 Provider。

**提交建议：** `test(workitem): lock prompt and review scope contracts`

---

## 工作包 5：单候选 engine 端到端（flag 内，单仓）

### Task 5.1：持久化五阶段 engine 与阶段 1 策略接线

**可追溯性：** 契约 Task 5.1；REQ-WSC-01、REQ-WSC-03、REQ-WSC-05、REQ-WSC-07；D2、D4、D5、D7、D-A、D-B。

**Files:**

- Create: `src/product/workspace_engine/single_candidate.rs`
- Modify: `src/product/workspace_engine/mod.rs`
- Modify: `src/product/models/workspace.rs`
- Modify: `src/product/lifecycle_store/workspace.rs`
- Modify: `src/product/workspace_engine/session_state.rs`
- Modify: `src/web/workspace_ws_types/out.rs`
- Modify: `web/src/api/types/workspace.ts`
- Modify: `web/src/state/workspace-ws-store-types.ts`
- Create: `src/product/workspace_engine/tests/single_candidate.rs`
- Modify: `src/product/workspace_engine/tests.rs`
- Reuse: `src/product/work_item_plan_source_store.rs`、`src/product/work_item_revision_store/initial_publication.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanSingleCandidatePhase {
    Prepare,
    Generate,
    Evaluate,
    Approval,
    Completed,
    Failed,
}
```

`WorkspaceSessionRecord` 兼容新增 `single_candidate_phase: Option<WorkItemPlanSingleCandidatePhase>`、`work_item_plan_source_revision_ref`、`plan_candidate_ir_ref`、`mechanical_report_ref`、`publication_provenance_ref`、`approval_attempt_id: Option<String>`、`approved_at: Option<String>` 与 `compile_reservation: Option<SingleCandidateCompileReservation>`，均逐字段 `#[serde(default, skip_serializing_if = "Option::is_none")]`；这两个 Approval 字段是 durable session schema 的唯一种子落点，`approved_at` 必须是 RFC3339，`approval_attempt_id` 由首次 Approval CAS 分配并在重试/恢复中只读复用。SingleCandidate flow 必须 Some phase；legacy 为 None。公开 `session_state` 只兼容增加 phase/ref，不增加阶段 3 决策消息；前端只透传、不新增 UI。

**provenance/freshness 数据通路：** Generate 只把 immutable source revision ref 与 IR ref 写入 durable session/source store；Evaluate 写入 immutable mechanical report ref。Approval 在新路径 engine/IR adapter 外层读取 durable source/IR/report，验证 source hash、compiler version 和 report zero Error；在任何 provenance 或 compile transaction 写入前，先以 Approval CAS 中已保存的 durable tuple 按 Task 2.5 规则计算 `compile_id`、`now`、`publication_provenance_ref`，并通过唯一 `put_compile_reservation_cas` 持久化 typed reservation。只有 reservation CAS 成功/同值幂等命中后，才分配 publication IDs、写 publication provenance（绑定 `plan_id`/待发布 `plan_revision_id`）并建立含这些 refs 的 compile transaction；四个 crash 边界重启都复用 reservation 三元组和所有 publication IDs。publication execute 将该 provenance canonical ref 逐字写入 `WorkItemPlanRevision.publication_provenance_ref`；recovery 从 transaction 的 `flow_kind` 与 refs 重建同一输入，禁止走 `legacy_mechanical_report` 或 legacy repair compatibility 分支。

状态规则：Prepare→Generate→Evaluate→Approval→Completed；任意 Fatal→Failed 吸收；auto valid 从 Evaluate 经 durable Approval/compile 到 Completed，不等 WS response；interactive 在 Approval 复用阶段 1 human gate snapshot。每一步用阶段 1 CAS 先持久 next state/provider idempotency key，再启动 provider；相同 completion/event 重放不产生第二 source revision/IR/review/compile。预算耗尽后出现新的（非重复指纹）repairable finding 时，必须按阶段 1 矩阵进入 interactive `awaiting_human` 或 auto `stopped_needs_human`，不 fatal、不空转。

- [ ] **Step 1 — 写失败测试。** auto valid 精确状态序列、repairable 最多一次 repair+一次 verification、repeated fingerprint 的 interactive/auto 终态、repair budget 耗尽后的新 repairable finding 分支、transition budget fatal、run policy 创建后不可改；断言 reviewer verdict/severity 从未直接调用阶段跳转。另断言在任何 provenance/transaction 写入前先完成 reservation CAS，四个 crash 边界重启复用同一 compile ID/now/provenance/publication IDs，并断言 source/IR/report/provenance refs 在 CAS 与 publication revision 中保持逐字一致。

  ```bash
  cargo test --locked --lib workspace_engine::tests::single_candidate::phase_machine
  ```

  预期失败：single candidate phase/engine 不存在。
- [ ] **Step 2 — 实现 durable 类型/store/state projection。** CAS 同时提交 phase、history/scope/gate/diagnostics/provider reservation/ledger 与 source/IR/report/provenance refs；唯一 Approval CAS 原子写入 `approval_attempt_id`/`approved_at` 后，Approval compile 的专用 `put_compile_reservation_cas` 才可成为任何 provenance/transaction 写入前的首个 compile-publication durable 写。reservation CAS 使用 expected 完整 session/`updated_at` 版本并返回更新 session；Conflict 后重读并重评，不复用旧 delta，persistence error 按 `FatalReason::PersistenceFailure` + `PolicyDiagnostic { code: "persistence_failure", ... }` durable 收敛为 Failed；四个 crash boundary 都必须先从 durable Approval record 重算或恢复同一 typed reservation。`flow_kind` 只从 session 创建时快照并从 durable record 读取，不能以当前 rollout flag 覆盖。
- [ ] **Step 3 — 实现 engine happy/terminal paths。** Generate 保存 immutable markdown revision；Evaluate 编译/验证/策略；Approval 在首个 transaction put 前做 freshness/provenance 校验，先 CAS reservation，再按固定顺序写 provenance、首个 transaction 并调用 IR compile adapter；Completed/Failed 吸收。恢复若停在四个边界中的任一处，均复用 reservation 的 compile ID、now、provenance ref 与确定性 publication IDs。对新路径只传递真实 mechanical report ref。
- [ ] **Step 4 — 运行转绿。** 同一命令通过，再执行：

  ```bash
  cargo test --locked --lib lifecycle_store
  cargo test --locked --lib workspace_engine::tests::single_candidate
  ```

  预期 durable 与 engine tests 全绿。

**提交建议：** `feat(workitem): add durable single-candidate phase engine`

### Task 5.2：运行时内部选择 batch/serial，不发决策

**可追溯性：** 契约 Task 5.2；REQ-WSC-01；D7。

**Files:**

- Modify: `src/product/workspace_engine/single_candidate.rs`
- Modify: `src/product/workspace_engine/tests/single_candidate.rs`
- Modify: `src/web/workspace_ws_handler/protocol.rs`
- Modify: `src/web/workspace_ws_handler/decisions/inbound.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleCandidateGenerationDecisionInput {
    pub provider: ProviderName,
    pub candidate_item_count: usize,
    pub prompt_bytes: usize,
    pub provider_input_budget_bytes: usize,
}

pub(crate) fn select_internal_generation_mode(
    input: &SingleCandidateGenerationDecisionInput,
) -> WorkItemGenerationModeDto;
```

generation 子流程和输入时点固定：先由 SingleCandidate author provider 生成轻量 markdown outline；服务端对该 outline 做本地机械解析，取成功解析的 item 数作为 `candidate_item_count`；只有拿到 count 后才读取 provider/model capability budget 并调用 selector；最后按 selector 结果调用完整 markdown author invocation。selector 不接受客户端 count/mode，不能从尚未生成的 draft/index 猜数。outline parse 失败立即形成 compiler diagnostic/fatal，禁止 fallback legacy；invocation 顺序测试必须捕获 `outline → parse/count → selector → full markdown author → parse/source revision`。

确定性规则：`candidate_item_count <= 3` 且 `prompt_bytes * 4 <= provider_input_budget_bytes * 3` 时 Batch，否则 Serial。该阈值是实现自由度和 internal diagnostic，不是 OpenSpec/WS 契约阈值；若 campaign 实测 batch 失败率高，可调整数值而无需回改契约，但同一输入的选择必须保持确定。该 mode 仅写 internal run diagnostic，不进入可应答 WS。provider/model capability 的 budget 读取签名以当前 provider registry/profile 文件现状为准；不能让客户端提交值。

- [ ] **Step 1 — 写失败测试。** 以 outline fixture 的实际解析结果断言 count 来源，覆盖 boundary 3/4 items、75%/超 75% bytes、同输入稳定；spy 断言 selector 发生在 outline parse 之后且 full markdown author 之前。SingleCandidate session 收到 `select_work_item_generation_mode`、draft/batch decision 时返回精确 `SINGLE_CANDIDATE_GENERATION_DECISION_FORBIDDEN` protocol error 且 phase/history 不变；session state/events 不含 generation decision request。

  ```bash
  cargo test --locked --lib workspace_engine::tests::single_candidate::internal_generation_mode
  cargo test --locked --lib workspace_ws_handler::protocol
  ```

  预期失败：选择仍由旧 WS 决策路径承担。
- [ ] **Step 2 — 实现 outline/count/selector 子流程与协议隔离。** legacy flow 保持旧消息可用；只对 SingleCandidate 拒绝客户端 mode/draft/batch decision，内部 diagnostic 不序列化为 WS request。
- [ ] **Step 3 — 运行转绿。** 两条命令通过，legacy protocol tests 不删不改语义。

**提交建议：** `feat(workitem): choose generation mode inside single-candidate engine`

### Task 5.3：多仓 preflight 与副作用后禁止 fallback

**可追溯性：** 契约 Task 5.3；REQ-WSC-07；D7、D-E。

**Files:**

- Modify: `src/web/handlers/lifecycle.rs`
- Modify: `src/product/workspace_engine/single_candidate.rs`
- Modify: `src/product/models/workspace.rs`
- Create: `src/product/workspace_engine/tests/single_candidate_preflight.rs`
- Modify: `src/product/workspace_engine/tests.rs`
- Modify: lifecycle handler tests

**Interfaces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SingleCandidatePreflightDecision {
    Eligible { repository_id: String },
    LegacyFallback { reason: String },
}

pub(crate) fn preflight_single_repository_candidate(
    repository_ids: &[String],
) -> SingleCandidatePreflightDecision;
```

preflight 在创建/持久化 SingleCandidate session、source、IR、run history、transaction 或启动 Provider **之前**运行，只读输入，不调用会写 invalidation 的 resolver。0 或 >1 repository 时可选 legacy 并把创建的 flow_kind 固定为 Legacy；恰 1 个才建 SingleCandidate。建 session/source/IR/run history/transaction 任一记录或 provider ledger `started=true` 后，任何错误都保持 flow_kind=SingleCandidate，durable 收敛为 recoverable gate/stop 或 failed，绝不创建 legacy session/transaction。

- [ ] **Step 1 — 写失败测试。** 对 0/2 repo preflight，比较 data root 前后文件列表：fallback 前没有 SingleCandidate record/provider start；1 repo 后注入 source write、IR write、provider start、compile journal 四种失败，分别断言 flow_kind 不变、无 legacy record、durable terminal/diagnostic 存在。执行：

  ```bash
  cargo test --locked --lib workspace_engine::tests::single_candidate_preflight
  cargo test --locked --lib web::handlers::lifecycle
  ```

  预期失败：当前 lifecycle handler 只按 flag 快照，不具备副作用屏障。
- [ ] **Step 2 — 实现纯 preflight 和 side-effect marker。** prepare HTTP 在 session create 前决定 flow；rollout flag 只读一次。错误路径不得静默重调 legacy prepare。
- [ ] **Step 3 — 运行转绿。** 两条命令通过；断言旧 session JSON serde default 仍是 Legacy。

**提交建议：** `feat(workitem): enforce preflight-only legacy fallback`

### Task 5.4：恢复矩阵与 provider 最多启动一次

**可追溯性：** 契约 Task 5.4；REQ-WSC-01、REQ-WSC-03、REQ-WSC-05、REQ-WSC-07；D2、D4、D5、D-A、D-B、D-E。

**Files:**

- Create: `src/product/workspace_engine/tests/single_candidate_recovery.rs`
- Modify: `src/product/workspace_engine/tests.rs`
- Modify: `src/product/work_item_revision_store/tests/initial_publication.rs`
- Modify only for defects: `src/product/workspace_engine/single_candidate.rs`、`compile.rs`、`compile/finalizer.rs`、`lifecycle_store/workspace.rs`、`workspace_ws_handler/socket.rs`

**Required fixture schema：** 每例按所在 crash 边界显式写初始 session JSON 及该边界之前已经 durable 的 source/IR/mechanical report/provenance object/compile transaction/typed `compile_reservation`，不得预写边界之后的对象；reservation 前例必须让 session 已持久化 `single_candidate_phase=Approval`、source/IR/report refs、`approval_attempt_id`、`approved_at` 而 `compile_reservation=None`，其余边界按实际前缀写入。记录 `initial_events`、transaction observer snapshots 与 `provider_start_ledger`；动作固定为销毁 engine 后新建 persistent engine 或 WS reconnect；断言 final phase/status、transaction 七个 durable 字段、完整 provenance content hash、provider start idempotency key 去重数和 events 前缀不可变。Approval/initial compile 恢复必须额外覆盖四个 crash 边界：Approval record 已 durable 但 reservation 前、reservation 后/provenance 前、provenance 后/transaction 前、首个 transaction put 后；第一例重启必须经 `get_workspace_session(session_id)` 从 durable Approval record 重算，四例均断言同一 compile ID、now、publication provenance ref、provenance content hash 与由 compile ID 确定性重算的 publication IDs。

- [ ] **Step 1 — 写 generate/repair 中断测试。** `Reserved+ledger absent` 重启只释放/重新规划，不计 repair；`ProviderStarted/Committed` 复用同 key；生成/返修 provider 去重启动次数均 ≤1，repair counter 不重复。
- [ ] **Step 2 — 写 approval/completed 测试。** approval pending 重连恢复同 gate、启动 0；Completed/Failed 重放 phase/status/refs 不变、provider 0、无新 compile。
- [ ] **Step 3 — 写两个独立恢复矩阵。** A 矩阵遍历五个 finalizer checkpoint，断言 child/session/report finalization 幂等；B 矩阵遍历 `InitialPlanPublicationCheckpoint::{LineageWritten, FirstWorkItemArtifactsWritten, PlanArtifactsWritten, FirstWorkItemActivated, PlanActivated}`，每例都在 initial publication 尚未完成时销毁 engine，Continue 必须调用 `resume_initial_plan_compile_transaction`，observer 必须捕获 `publication_resumed` 位于 publication 重放后、第一个后续 finalizer cursor 前。B 矩阵的前提是 checkpoint 已存在 durable transaction；B 每例只从 durable transaction 的 canonical refs 调 direct-ref get API 重载 source/IR/report/完整 provenance，不得读取重启前 `IrCompileAdapterContext` ID；断言同 compile/publication/child IDs、journal fingerprint、ref+content hash、ledger/event 前缀且 immutable publication 不重复，并表驱动覆盖 malformed/wrong-kind/scope-mismatch/dangling ref 四个稳定码。B 的 Continue 只允许后续 journal put/resume，不得再次创建或声称“首个 transaction put”。A 矩阵不能替代 B 矩阵。另在 B 之前独立遍历四个 reservation crash 边界：第一例只给 durable Approval record 且 reservation=None，重启后按该 record 重算相同三元组；其余例验证 reservation CAS 先于 provenance/transaction，重启只重放缺失后缀，四例的 compile ID、now、provenance ref/content hash、publication IDs 全部相同；另对 Approval CAS 自身独立断言：(1) 锁内重读后对 stale expected 与 stored 做完整 record equality，若不等则返回 `ProductStoreError::Conflict` 且不写入；(2) 传入非法或任意伪造的 `approval_attempt_id`（包括大小写变化、字段错位）返回 `ProductStoreError::InvalidRecord` 且不写入、不覆盖；(3) 固定 hash 测试向量 `(session-001, plan-001, approval-001, 2026-08-27T12:34:56Z)` 的 canonical bytes hex 与 `compile_id` 分别等于 Task 5.1 定义的 hex 与 `5a16e570210838318554c17b3ebd0c433c3001ce00adb7b8e9726d79aecf788e`。另注入 reservation CAS 的 stale expected full-record Conflict 与 write/serialize persistence error，分别断言冲突重读重评和 `FatalReason::PersistenceFailure` durable diagnostic。
- [ ] **Step 4 — 运行确认失败。** 新增函数统一前缀 `single_candidate_recovery_`，先执行 `cargo test --locked --lib single_candidate_recovery_ -- --list` 并要求至少 10 项，再执行不带 `-- --list` 的同过滤名；未接线前应失败且指出次数/状态/cursor/事件差异，0 项不得继续。
- [ ] **Step 5 — 实现幂等恢复并转绿。** 新增前缀至少 10 项全绿；四个 reservation crash boundary 均须先通过同一 compile ID/now/provenance/publication ID 断言；再执行 `cargo test --locked --lib compile_recovery_continue -- --list`（已验证匹配 3 项）及同过滤名测试，并执行 `cargo test --locked --lib initial_plan_publication -- --list`（已验证匹配 5 项）及同过滤名测试。预期新旧恢复 tests 全绿。

**提交建议：** `test(workitem): cover single-candidate crash recovery matrix`

---

## 工作包 6：【授权门：真实 provider 运行前必须获得用户显式授权，未授权时执行到此暂停】验收

### Task 6.1：campaign driver 适配与授权请求

**可追溯性：** 契约 Task 6.1；REQ-WSC-01、REQ-WSC-05、REQ-WSC-07；D5、D7。

**Files:**

- Modify: `cadence/reports/workitem-coding-campaign/workitem_run_campaign.mjs`
- Modify: `cadence/reports/workitem-coding-campaign/campaign_driver_policies.test.mjs`
- Read-only verification: `cadence/reports/workitem-coding-campaign/coding_run_campaign.mjs`

**Driver contract:** prepare body 显式 `run_policy: "auto_if_valid"`，环境预期 `ARIA_EXPECTED_FLOW_KIND=single_candidate`；首个 session state fail-closed 检查 flow/policy/durable history。SingleCandidate 只发 `start_generation`，不发送 generation-mode、outline/draft/batch/review decision。终态只接受 Confirmed/completed、stopped_needs_human、failed；cycle 只读真实 `run_history.review_cycles.*.{initial_count,verification_count,repairs_used}`；result 必须保存完整 `provider_start_ledger` 和按 idempotency key 去重结果，不能只保存 count。

当前 driver `:671-707` result 尚缺承重字段。本任务新增：`duration_ms=Date.now()-started`；确认 lifecycle plan 为 Confirmed 后 `confirmed_count=1`（失败/未确认保持 0）；`legacy_decision_messages` 收集 SingleCandidate 收到的旧决策消息并立即 fail-closed，成功必须 `[]`；`provider_start_ledger` 从最新 durable session 完整规范化写入。两个案例各成功一次，**每份 result 的 `confirmed_count` 都是 1，合计才是 2**。

proposal 的两个 campaign driver 逐一核对：`workitem_run_campaign.mjs` 与 SingleCandidate 的 flow/policy/session/review 协议有交互，需按本任务适配；`coding_run_campaign.mjs` 只消费 Confirmed handoff 并驱动 coding 段，不读取 workitem `flow_kind`、generation_mode、review scope 或 policy counters，也不发送 WorkItem WS 决策，因此本阶段无需适配，必须在测试/验收记录中保留该“检查过且无适配缺口”的依据，不得误改 coding driver。

- [ ] **Step 1 — 写失败 fixture tests。** 覆盖 flow/policy 不匹配、history 缺失、无旧决策、Confirmed、stopped、failed、unknown diagnostic、完整 provider ledger/去重、真实 cycle 字段、`duration_ms` 与每案 `confirmed_count=1`；同时断言 coding driver 无 SingleCandidate 协议交互。2026-06-30 已实际执行当前 Node 文件，匹配 21 项、21 pass；新增后测试数必须 >21。

  ```bash
  node --test cadence/reports/workitem-coding-campaign/campaign_driver_policies.test.mjs
  ```

  预期在 driver 适配前新增 cases 失败。
- [ ] **Step 2 — 实现 driver。** legacy 分支继续保留原决策处理；single-candidate 发现旧决策请求写入 `legacy_decision_messages` 后立即报告协议回归，不自动应答；finish 时写真实 duration，durable readback 时保留完整 ledger；不修改 coding driver。
- [ ] **Step 3 — 运行转绿/dry-run。** Node 命令显示 tests>21、fail 0；使用服务器真实数据根的无 credential dry-run 验证参数，不连接 Provider（该命令已于 2026-06-30 对当前参数解析实测退出 0）：

  ```bash
  cd /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo && ARIA_DATA_ROOT="$PWD/.aria" ARIA_EXPECTED_FLOW_KIND=single_candidate ARIA_RUN_POLICY=auto_if_valid ARIA_WORKITEM_HARD_TIMEOUT_MS=720000 node cadence/reports/workitem-coding-campaign/workitem_run_campaign.mjs codex 1 /tmp/aria-phase2-dry-run --dry-run
  ```
- [ ] **Step 4 — 授权请求并暂停。** 向操作者逐字给出 Global Constraints 中的 Case A/B 提醒：`本次改动涉及 Work Item Draft Prompt 或其结构化契约。建议按 cadence/designs/2026-07-24_技术方案_WorkItemDraftPrompt测试基线_v1.0.md 执行 Case A 与 Case B 各 10 个有效首次输出的 Claude Code 验证；是否授权执行？`；若操作者授权，按该基线执行 Case A/B 各 10 个有效首次输出，并在验收报告“授权记录”段落记录已授权时间及脱敏的 10/10 结果；若未授权，记录未授权时间和“Case A/B 未执行”，不得调用 Provider。另行请求是否授权 codex+pi naruto 各 1 次真实 campaign；未收到明确授权时到此暂停，不运行 Task 6.2，不创建实测结果。

**提交建议：** `test(campaign): support single-candidate durable protocol`

### Task 6.1a：显式启动参数接通 SingleCandidate rollout state

**可追溯性：** 契约 Task 6.1 与 REQ-WSC-07 的真实 campaign 前置接线；不新增产品语义，只使既有创建时 rollout flag 可显式启用。

**Files:**

- Modify: `src/cli.rs:186-223`（`WebOptions`/`parse_web_options`/`serve_web` 调用）
- Modify: `src/web/app.rs:515-535`（`serve_web` 构造 state）
- Modify: `src/web/state.rs:119-215`（显式 constructor/setter；默认仍 false）
- Modify: CLI/app/state tests（当前 `cli::tests` 已验证匹配 7 项，`web::app::tests` 9 项，`web::state::tests` 11 项）
- Reuse: `src/web/handlers/lifecycle.rs:667-688`（唯一 session flow snapshot 消费点）

**Exact interface:** web CLI 新增 boolean `--work-item-plan-single-candidate`；`WebOptions` 保存该值，`serve_web(workspace, host, port, work_item_plan_single_candidate)` 用它构造 `WebAppState`，最终使 lifecycle prepare 写 `flow_kind=SingleCandidate`、`rollout_snapshot=true`。无 flag 时仍为 false/Legacy，运行中不得由 driver 或 WS 改写。

- [ ] **Step 1 — 写失败测试。** CLI 有/无 flag 解析；app state 有 flag=true、默认=false；经真实 prepare handler 创建的 session 分别为 SingleCandidate/Legacy；旧 session serde default 不变。
- [ ] **Step 2 — 最小接线。** 只增加启动参数→state→既有 lifecycle 快照，不读取 `ARIA_EXPECTED_FLOW_KIND`（它只是 driver 预期），不把 flag 变成运行中控制面。
- [ ] **Step 3 — 列举并转绿。** 分别以 `cargo test --locked --lib cli::tests -- --list`（已验证 7 项）、`web::app::tests`（9 项）、`web::state::tests`（11 项）核实新增后数量增长，再执行三条不带 `-- --list` 的测试；最后跑 lifecycle prepare test，断言新/旧 flow。

**提交建议：** `feat(web): wire single-candidate rollout at startup`

### Task 6.2：授权后 codex + pi naruto 单仓实跑

**可追溯性：** 契约 Task 6.2；REQ-WSC-01、REQ-WSC-03、REQ-WSC-05、REQ-WSC-07；D-A、D-B、D5、D7。

**Files:**（仅授权后创建，内容脱敏）

- Create: `cadence/reports/workitem-coding-campaign/reports/phase2-single-candidate/codex-naruto-result.json`
- Create: `cadence/reports/workitem-coding-campaign/reports/phase2-single-candidate/pi-naruto-result.json`

**逐字可复制的运行前置与命令（不含 credential）：** `src/web/app.rs:520` 与 `socket.rs:139` 已核实服务器固定读写 `<workspace>/.aria`，不读取 `ARIA_DATA_ROOT`；因此 server workspace 固定为本 worktree，driver 的 `ARIA_DATA_ROOT` 也必须逐字指向同一 worktree `.aria`。codex/pi 由不同自动生成 issue/session ID 与不同输出目录隔离，不能伪装成不同 data root。Task 6.1a 的显式 flag 使新 session 确为 SingleCandidate。`ARIA_WORKITEM_HARD_TIMEOUT_MS=720000` 是每案 12 分钟硬上限。

`nohup` 启动铁律：下面启动命令必须**原样单独执行**；不得与 `cd`、`echo $!`、PID 写入、sleep、curl 或任何 `&&`/`;` 子命令夹带。绝对 `--manifest-path`/`--workspace` 使其不依赖调用者 cwd。

```bash
nohup cargo run --locked --manifest-path /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/Cargo.toml -- web --workspace /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo --host 127.0.0.1 --port 4317 --work-item-plan-single-candidate >/tmp/aria-phase2-server.log 2>&1 </dev/null &
```

启动后以下两条也分别执行，不能合成一条；75 秒发生在 health/result 采集前：

```bash
sleep 75
```

```bash
curl --fail --silent --show-error http://127.0.0.1:4317/api/health
```

```bash
cd /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo && ARIA_DATA_ROOT="$PWD/.aria" ARIA_BASE_URL=http://127.0.0.1:4317 ARIA_EXPECTED_FLOW_KIND=single_candidate ARIA_RUN_POLICY=auto_if_valid ARIA_WORKITEM_HARD_TIMEOUT_MS=720000 node cadence/reports/workitem-coding-campaign/workitem_run_campaign.mjs codex 1 /tmp/aria-phase2-results/codex
```

```bash
cd /home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo && ARIA_DATA_ROOT="$PWD/.aria" ARIA_BASE_URL=http://127.0.0.1:4317 ARIA_EXPECTED_FLOW_KIND=single_candidate ARIA_RUN_POLICY=auto_if_valid ARIA_WORKITEM_HARD_TIMEOUT_MS=720000 node cadence/reports/workitem-coding-campaign/workitem_run_campaign.mjs pi 1 /tmp/aria-phase2-results/pi
```

driver 参数解析已与 `workitem_run_campaign.mjs:69-90,1390-1406` 对照，dry-run 已实测退出 0；新增 web flag 必须先通过 Task 6.1a CLI tests 与 `cargo run --locked -- web --workspace . --check --work-item-plan-single-candidate`。输出路径分别为 `/tmp/aria-phase2-results/codex/codex/rep1/result.json` 与 `/tmp/aria-phase2-results/pi/pi/rep1/result.json`；授权后将脱敏字段复制/整理为上列报告文件。每份必须包含 `session_status`、`confirmed_count=1`、`duration_ms`、`run_history.review_cycles.*.{initial_count,verification_count,repairs_used}`、完整 `provider_start_ledger`、`legacy_decision_messages=[]`；缺字段即失败，不以默认值补齐。

- [ ] **Step 1 — 再确认授权状态。** 只有当前对话中的明确允许才有效；没有 codex+pi 实跑授权则保持未运行并报告原因。Case A/B 授权与 campaign 授权分别记录。
- [ ] **Step 2 — 启动并验证服务器。** 单独执行 nohup；单独等待 75 秒；单独 curl；随后从 prepare response/session JSON 断言 `flow_kind=single_candidate`、`rollout_snapshot=true` 后才允许 Provider 运行。
- [ ] **Step 3 — 运行 codex。** 固定 naruto 单仓、SingleCandidate、`auto_if_valid`，author/reviewer=codex；durable session 必须位于 worktree `.aria/projects/.../workspace-sessions/...json`，与 driver 输出的 session path 相同。失败不得补跑挑选样本。
- [ ] **Step 4 — 运行 pi。** 使用同一服务器实际 `.aria`，由不同 issue/session ID 与输出目录隔离；author/reviewer=pi；不得自动尝试旧决策语法。
- [ ] **Step 5 — 精确验收。** 两份 result 各 `confirmed_count==1`，合计 `sum==2`；均为 Confirmed/completed、每案 duration≤720000ms；每 cycle `initial_count<=1`、`verification_count<=1`、`repairs_used<=1`；旧决策数组为空；started ledger key 无重复。任一未达标即失败。

**提交建议：** `test(workitem): record phase2 codex and pi acceptance`

### Task 6.3：golden、diagnostic 与 usage/token 核对

**可追溯性：** 契约 Task 6.3；REQ-WSC-03、REQ-WSC-06、REQ-WSC-07；D3、D6、D-B。

**Files:**

- Reuse unchanged: `src/product/work_item_plan_policy/fixtures/golden_findings.json`
- Reuse: `openspec/changes/rearch-workitem-plan-pipeline/fixtures/reviewer-finding-channel-map.json`
- Modify: `cadence/reports/workitem-coding-campaign/campaign_driver_policies.test.mjs`（usage fixture assertions）

- [ ] **Step 1 — 自动核对 classifier。** 

  ```bash
  cargo test --locked --lib work_item_plan_policy::tests_classify
  ```

  预期 14/14（11 raw + 3 annotated）与阶段 1 expected class 一致。
- [ ] **Step 2 — 自动核对 compiler channel。** 

  ```bash
  cargo test --locked --lib work_item_plan_compiler::tests::reviewer_finding_channel_boundary
  cargo test --locked --lib work_item_plan_compiler::tests::fixtures
  ```

  预期九个 reviewer findings 只走 prompt few-shot；只核对 Task 1.3 明确 grammar/lowering 的 compiler diagnostics。
- [ ] **Step 3 — usage/token 比较。** driver 从两次授权结果读取 provider usage；报告总 input/output tokens、prompt bytes、Cadence-skills 注入 bytes、注入占比 `injected_bytes / total_input_bytes`，并与旧 rep 基线分别展示，不把缺 usage 填 0。缺字段标为 inconclusive，不影响伪造成功。
- [ ] **Step 4 — 运行 driver tests。** 

  ```bash
  node --test cadence/reports/workitem-coding-campaign/campaign_driver_policies.test.mjs
  ```

  预期 usage 缺失/除零/占比越界 fixture 失败关闭，合法 fixture 通过。

**提交建议：** `test(workitem): verify golden channels and token baseline`

### Task 6.4：验收报告与阶段 3 交接

**可追溯性：** 契约 Task 6.4；REQ-WSC-01、REQ-WSC-02、REQ-WSC-03、REQ-WSC-05、REQ-WSC-06、REQ-WSC-07；D4、D7。

**Files:**

- Create: `cadence/reports/2026-08-27_验收报告_WorkItem阶段2单候选C′MVP_v1.0.md`
- Read: Task 6.2 的两个 result JSON（授权后才存在）

**Required report sections:** scope/commit SHA；codex/pi durable terminal and duration；per-cycle counters；compiler/validator results；14 classifier golden；compiler diagnostic channel map；recovery matrix；legacy regression；usage/token 与 injection ratio；full backend/frontend gate；Case A/B 与 campaign 授权记录（只记录“已授权/未授权”和时间，不记录 credential，若 Case A/B 已授权则记录各 10/10 脱敏结果）；residual risks；阶段 3 handoff 仅列“聊天流人工门与 `advance` 接口”，不得在本阶段定义签名。

- [ ] **Step 1 — 写报告验收脚本。** 使用仓库外 Python 读取报告和授权后才存在的两份 result JSON；不得只检查标题。commit SHA、六项实现门禁、Legacy 回归 PASS、授权记录、两个 result path 与阶段 3 handoff 是**授权/未授权共同门禁**。授权分支每案确认 1、合计 2；逐 cycle 使用真实 `initial_count`/`verification_count`/`repairs_used`。只有 usage/token 实际缺失时才要求 `usage_unavailable`/`inconclusive`，数据完整时不得强制假标缺失。

  ```bash
  python3 - <<'PY'
  import json, re
  from pathlib import Path

  report = Path('cadence/reports/2026-08-27_验收报告_WorkItem阶段2单候选C′MVP_v1.0.md')
  result_paths = [
      Path('cadence/reports/workitem-coding-campaign/reports/phase2-single-candidate/codex-naruto-result.json'),
      Path('cadence/reports/workitem-coding-campaign/reports/phase2-single-candidate/pi-naruto-result.json'),
  ]
  assert report.exists()
  text = report.read_text(encoding='utf-8')
  required = ['范围与提交', 'Codex', 'Pi', '持久计数', '14 条 classifier golden', 'Compiler diagnostic', '恢复矩阵', 'Legacy 回归', 'Usage/Token', '全量门禁', '授权记录', '阶段 3 交接']
  assert all(value in text for value in required)

  # 共同实现门禁：未授权只跳过 Provider，不跳过实现质量与 Legacy 证据。
  assert re.search(r'(?i)commit SHA[^\n]*[0-9a-f]{40}', text)
  gates = [
      'cargo fmt --check',
      'cargo clippy --all-targets --all-features --locked -- -D warnings',
      'cargo check --locked',
      'cargo test --locked',
      '(cd web && pnpm tsc -b)',
      '(cd web && pnpm test)',
  ]
  for gate in gates:
      assert re.search(re.escape(gate) + r'[^\n]*(PASS|通过)', text)
  assert re.search(r'Legacy 回归[^\n]*(PASS|通过)', text, re.IGNORECASE)

  authorized = 'campaign：已授权' in text
  unauthorized = 'campaign：未授权' in text
  assert authorized != unauthorized
  usage_missing = unauthorized
  if authorized:
      assert all(path.exists() and str(path) in text for path in result_paths)
      results = [json.loads(path.read_text(encoding='utf-8')) for path in result_paths]
      assert sum(result.get('confirmed_count', 0) for result in results) == 2
      for result in results:
          assert result.get('session_status') in {'confirmed', 'completed'}
          assert result.get('confirmed_count') == 1
          assert 0 < result.get('duration_ms', 0) <= 720000
          assert result.get('legacy_decision_messages') == []
          cycles = result['run_history']['review_cycles']
          for cycle in cycles.values():
              assert cycle['initial_count'] <= 1
              assert cycle['verification_count'] <= 1
              assert cycle['repairs_used'] <= 1
          keys = [entry['provider_start_idempotency_key'] for entry in result['provider_start_ledger'] if entry.get('started')]
          assert len(keys) == len(set(keys))
          usage = result.get('usage_by_role')
          usage_missing = usage_missing or not usage or usage.get('usage_unavailable') is True
      assert 'campaign 结果：2/2' in text
  else:
      assert all(not path.exists() for path in result_paths)
      assert 'Codex result path：未生成（未授权）' in text
      assert 'Pi result path：未生成（未授权）' in text
      assert '2/2' not in text and not re.search(r'Confirmed[^\n]*(达标|通过)', text, re.IGNORECASE)

  if usage_missing:
      assert 'usage_unavailable' in text or 'inconclusive' in text
  PY
  ```

  预期初次因报告不存在或 section/结果断言缺失而失败。
- [ ] **Step 2 — 写验收报告。** 只记录真实命令/输出，不复制完整 prompt/provider draft/目标仓内容；授权与未授权路径都如实记录。授权路径以两份 result JSON 的 durable terminal/counters/duration 为唯一实测证据；未授权路径不得写字面量 `2/2` 或伪造确认结果。
- [ ] **Step 3 — 运行转绿。** 仅在对应授权分支具备真实报告/结果时重跑同一 Python 命令，预期无输出、退出码 0；未授权时仅验证“未授权分支”断言，不创建虚假 result JSON。
- [ ] **Step 4 — 退役裁决。** 只有 Task 6.2～6.3、Case A/B（如已授权）与最终门禁全部达标才标记“可立项阶段 3”；无论达标与否，本 change 都不删除旧协议。未达标明确“旧协议保持可用”。

**提交建议：** `docs(workitem): report phase2 single-candidate acceptance`

---

## 最终验收门禁

### A. 变更文件与范围检查

```bash
git status --short
git diff --name-only "$(cat /tmp/cadence-aria-workitem-phase2-baseline-sha)"
git diff --cached --quiet
```

预期：只出现本计划列明的 compiler/source-store/workspace/compile/prompt/model/store/WS 兼容类型、fixtures、campaign 和报告文件；没有 coding engine/WS、story/design 流程、阶段 3 UI/`advance`、Cadence-skills 项目改造；cached diff 为空。若实施过程按授权做了提交，第三条仍应退出 0。

### B. 当前实现全量四件套

在 worktree 根目录依次执行，任一非零都阻断完成声明：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

预期：四条退出码均 0；不得用过滤、跳过或 `-j` 替代全量门禁。

### C. 前端 TypeScript 与 Vitest

```bash
(cd web && pnpm tsc -b)
(cd web && pnpm test)
```

预期：TypeScript build 退出 0；Vitest 全部通过。阶段 2 不新增人工门 UI，但 SessionState optional fields/type/store 透传必须通过类型检查和现有 tests。

### D. 变更前 HEAD 基线对照

```bash
BASELINE_SHA="$(cat /tmp/cadence-aria-workitem-phase2-baseline-sha)"
BASELINE_ROOT="$(mktemp -d /tmp/cadence-aria-workitem-phase2-head.XXXXXX)"
git worktree add --detach "$BASELINE_ROOT" "$BASELINE_SHA"
(
  cd "$BASELINE_ROOT"
  cargo fmt --check
  cargo clippy --all-targets --all-features --locked -- -D warnings
  cargo check --locked
  cargo test --locked
  (cd web && pnpm tsc -b)
  (cd web && pnpm test)
)
git worktree remove --force "$BASELINE_ROOT"
```

预期：若当前实现门禁失败，必须与基线同命令/同错误逐条比较；只有能证明为变更前 HEAD 已有且与本 diff 无因果的失败才可记录为 baseline debt，不能声称该门禁通过。若基线绿而当前红，必为阻断回归。

### E. 针对性验收摘要

```bash
cargo test --locked --lib work_item_plan_compiler
cargo test --locked --lib work_item_plan_policy
cargo test --locked --lib workspace_engine::tests::single_candidate
WP3_LIST="$(cargo test --locked --lib workspace_engine::tests -- --list)"
printf '%s\n' "$WP3_LIST"
MATCH_COUNT="$(printf '%s\n' "$WP3_LIST" | awk '/: test$/ {n++} END {print n+0}')"
# 2026-06-30 实测 MATCH_COUNT=881；必须保留非零断言，0 项立即失败。
test "$MATCH_COUNT" -gt 0 || exit 1
cargo test --locked --lib workspace_engine::tests
node --test cadence/reports/workitem-coding-campaign/campaign_driver_policies.test.mjs
```

预期：compiler grammar/lowering/freshness、阶段 1 14 golden、single-candidate 状态/恢复、legacy compile parity、campaign driver 全绿。定向命令只用于快速定位；不能替代 B/C 全量门禁。

## 覆盖映射自查表

### Requirement 覆盖

| Requirement | 对应实施任务 | 自查结论 |
|---|---|---|
| REQ-WSC-01 单候选事务 | 3.1～3.4、4.2a、5.1、5.2、5.4、6.1、6.2、6.4 | prepare/generate/evaluate/approval/completed+failed、flow_kind 分发、无旧决策、compile/恢复/验收均有测试 |
| REQ-WSC-02 markdown compiler | 1.1～1.4、2.1～2.5、3.4、4.1、6.4 | 唯一源、typed IR、顶层 hash/version、diagnostic、freshness/provenance、coding 零解析均覆盖 |
| REQ-WSC-03 typed outcome/policy | 2.3、2.4、4.3～4.5、5.1、5.4、6.2～6.4 | 明确复用阶段 1，review evidence 不直跳、预算/重复指纹/终态/计数覆盖 |
| REQ-WSC-04 | 无 | 契约 spec 未定义；经确认不臆造，标记不适用 |
| REQ-WSC-05 运行策略持久化 | 5.1、5.4、6.1、6.2、6.4 | 创建时固定、durable roundtrip、运行中改写拒绝、driver fail-closed 覆盖 |
| REQ-WSC-06 prompt 分层 | 1.2、1.3、2.3、4.1～4.5、4.2a、6.3、6.4 | B 层删除、C 层/grammar/few-shot/reviewer calibration、flow_kind 分支、字节门禁覆盖 |
| REQ-WSC-07 并存与退役门 | 1.1、1.3、2.3、2.5、3.1～3.4、4.2a、4.4～4.5、5.1、5.3、5.4、6.1、6.1a、6.2～6.4 | durable flow_kind 分发、preflight-only fallback、legacy/IR parity、provenance recovery、显式 rollout 启动接线、campaign/golden、未达标不删旧协议覆盖 |

### 契约 Task 覆盖

| 契约 Task | 计划任务 | 验证落点 |
|---|---|---|
| 1.1 | Task 1.1 | matrix 唯一来源 test |
| 1.2 | Task 1.2 | grammar contract test |
| 1.3 | Task 1.3 | rep4/diagnostic fixture tests |
| 1.4 | Task 1.4 | source linter tests |
| 2.1 | Task 2.1 | parse diagnostics tests |
| 2.2 | Task 2.2 | typed IR/lowering tests |
| 2.3 | Task 2.3 | 14 classifier + channel map tests |
| 2.4 | Task 2.4 | existing validator zero Error test |
| 2.5 | Task 2.5 | freshness/immutable publication tests |
| 3.1 | Task 3.1 | legacy normalized characterization |
| 3.2 | Task 3.2 | exact input + pure prepare parity |
| 3.3 | Task 3.3 | five checkpoint recovery parity + independent review |
| 3.4 | Task 3.4 | legacy/IR adapter parity |
| 4.1 | Task 4.1 | markdown prompt contract |
| 4.2 | Task 4.2 | B absent/C present assertions |
| 4.2a | Task 4.2a（补充接线任务，覆盖契约 4.1/4.2） | durable flow_kind legacy/SingleCandidate 端到端分支 |
| 4.3 | Task 4.3 | inbound schema/handler scope rejection + server scope/digest tests |
| 4.4 | Task 4.4 | mechanical report/fingerprint scope fail-closed |
| 4.5 | Task 4.5 | prompt byte/reconnect/invocation suite |
| 5.1 | Task 5.1 | durable five-phase engine tests |
| 5.2 | Task 5.2 | deterministic mode/no WS decision tests |
| 5.3 | Task 5.3 | preflight side-effect barrier tests |
| 5.4 | Task 5.4 | restart/reconnect/provider-once matrix |
| 6.1 | Task 6.1 | campaign driver fixtures + authorization pause |
| 6.1a | Task 6.1a（补充启动接线任务，覆盖契约 6.1） | CLI→serve_web→WebAppState→session rollout snapshot，默认 Legacy |
| 6.2 | Task 6.2 | authorized codex/pi 2/2 metrics |
| 6.3 | Task 6.3 | classifier/compiler/token checks |
| 6.4 | Task 6.4 | acceptance report + no-retirement decision |

## writing-plans 自查结论

- **Spec coverage：** 契约现有 6 条 requirement、契约 1.1～6.4 的 26 个工作包均有映射；另有两个不改变范围的补充接线任务 `4.2a` 与 `6.1a`，因此修订后计划任务总数为 **28**（26 个契约任务 + 2 个补充任务）。REQ-WSC-04 明确为契约未定义、不适用，没有补造语义；覆盖映射表中 `4.2a` 紧随契约 4.2、覆盖契约 4.1/4.2 真实 provider 执行分发，`6.1a` 紧随契约 6.1、覆盖 campaign 前置的 CLI→state→session rollout 快照。
- **空泛步骤扫描：** 28 个任务均有确切文件、断言、失败/验证命令、实现边界与 commit message 草案；`4.2a` 明确 durable session.flow_kind→run kind→builder/parser 的 legacy/SingleCandidate 两条真实执行链，`6.1a` 明确有/无启动 flag、默认 Legacy 与 lifecycle prepare snapshot 测试；没有无代码/无命令/无路径的延后步骤。
- **类型一致性：** `InitialPlanCompileInput` 与契约精确 struct 保持 13 个字段，不加入 provenance/freshness；provenance/freshness 由外层 adapter/engine durable refs 与独立 publication context 承载。后续统一引用 `PlanCandidateIr`、`PlanCandidateItemIr`、`WorkItemPlanSourceContext`、`PlanCandidateMechanicalReport`、`PlanCandidatePublicationProvenance`、`InitialPlanCompileInput`、`IrCompileAdapterContext`、`WorkItemPlanSingleCandidatePhase`；compile 的两个尚不存在内部容器只在契约授权处声明“实施时以当前 compile 数据流为准”，没有锁定虚构字段。validator 实现引用 `types.rs:18-83`，projection context 引用完整 `types.rs:417-429`。
- **高风险顺序：** 3.1→3.2→3.3→独立审查→3.4 已写成硬门；3.3 明确重跑 revision-store initial publication 与现有 compile recovery。
- **占位符/时间/hash 自查：** 3.1 明确 journal observer 采集每次真实 put；动态 ID 使用引用保持型稳定占位符，不删除 ID；timestamp 先验证 RFC3339 与 created_at 稳定；projection/contract hash 重新计算核对，且 recovery 必含 `publication_resumed`。
- **授权顺序：** 6.1 fixture/dry-run 后分别请求 Case A/B 与 campaign 授权；未授权暂停且报告不得出现 `2/2`；6.2 仅在授权后按逐字命令运行 codex/pi。
- **范围结论：** coding 零改动、阶段 3/4 非目标、legacy 协议保留、validator 规则不改，未扩大契约范围。


## 修订记录

> 本节供复审 delta 使用；下表逐条对应三方裁决 finding。重复出现的同一技术问题分别保留来源映射，不以“已在另一裁决处理”省略。

| 来源 / finding | 处理结论 | 修复位置 |
|---|---|---|
| sol B1：Task 1.2/1.3 在 parser 前断言真实诊断，破坏 TDD 顺序 | 修复：1.2 只测 grammar 常量/AST/metadata；1.3 只测 fixture 静态结构与 expected schema；真实 line/field/example 统一移到 1.4 | Task 1.2、Task 1.3、Task 1.4 |
| sol B2：compile transaction 覆盖写，无法观察 journal 序列 | 修复：Task 3.1 增加 test-only `put_compile_transaction` recording observer/hook，采集完整 snapshots 并隔离清理 | Global Constraints、关键事实 3、Task 3.1 |
| sol B3：freshness/provenance 无法进入 immutable publication | 修复：不改 `InitialPlanCompileInput`；外层 adapter/engine 在首个 put 前读取 source/IR/report、校验 freshness、分配 provenance；durable transaction/session 引用并断言 publication ref 逐字一致、recovery 不变 | Task 2.5、Task 3.2、Task 3.4、Task 5.1、Task 5.4 |
| sol M1：prompts.rs 删除行号错误，会误删 C 层 target 保留指令 | 修复：删除行号区间；按 `work_item_plan_runtime_contract`、`work_item_draft_runtime_contract`、`build_work_item_draft_prompt` 符号及精确文本删除 B 层，明确保留 `target_retain_instruction` | 关键事实 4、Task 4.2 |
| sol M2：cursor 清单遗漏 recovery `publication_resumed` | 修复：正常/recovery 清单分列，3.1/3.3/3.4/5.4 都断言 recovery cursor 位置 | 关键事实 3、Task 3.1、Task 3.3、Task 3.4、Task 5.4 |
| sol M3：validator 与 projection context 源码引用不真实 | 修复：validator 改为 `work_item_split_validator/types.rs:18-83`，注明 `mod.rs:20-23` 仅 re-export；context 改为完整 `workspace_engine/types.rs:417-429` | 关键事实 2、5；Task 2.4、Task 3.2；类型自查 |
| sol M4：flow_kind 分发无入口/参数/端到端测试 | 修复：新增 Task 4.2a，指定 `StartGeneration`/reconnect-retry 入口读取 durable session.flow_kind，传入 builder/parser，并分别覆盖 legacy/SingleCandidate | Task 4.2a、REQ 覆盖表、契约任务覆盖表 |
| sol M5：Task 4.3 缺 inbound schema/handler 和定向 scope 拒绝 | 修复：纳入 `src/web/workspace_ws_types/in_.rs`、`workspace_ws_handler/protocol.rs`、`decisions/inbound.rs`；定向 raw-object 拒绝并精确断言 `SINGLE_CANDIDATE_SCOPE_FORBIDDEN`、session/history/timeline 不变 | Task 4.3 |
| sol M6：generation mode 的 candidate count 时点未定义 | 修复：固定 `outline → 本地机械 parse/count → selector → full markdown author → parse/source revision`，补 invocation order spy；失败不得猜 count/fallback | Task 4.1、Task 5.2 |
| sol M7：campaign 缺可复制 dry-run/server/health/codex/pi 命令 | 修复：给出 worktree 显式 cd、`nohup cargo run`、sleep 75 + curl health、隔离 `ARIA_DATA_ROOT`/输出目录、720000ms timeout 和 codex/pi 命令，均无 credential | Task 6.1、Task 6.2 |
| sol M8：6.4 脚本只查标题，不能证明验收 | 修复：脚本读取两份 result JSON，校验终态/确认计数/时长/counters/旧决策/ledger key，解析 commit SHA 与六项门禁；未授权断言不含 `2/2` | Task 6.4 |
| sol minor 1：字段来源矩阵漏 `publication_provenance.plan_id` | 修复：加入矩阵 field path 与唯一来源/禁止第二来源断言 | Task 1.1、Task 2.5 |
| sol minor 2：Task 4.5 强制人为制造失败 | 修复：若前置测试已覆盖则记录覆盖来源，不人为制造失败或放宽契约 | Task 4.5 |
| oracle Major 1：journal observer、引用保持型 ID、RFC3339/created_at/hash 断言不足 | 修复：与 sol B2 合并落实 observer；补稳定占位符映射、时间合法性/稳定性、hash 与产物一致性 | Global Constraints、Task 3.1、Task 3.3 |
| oracle Major 2：`plan_projection.rs:329-539` 未纳入共享 publication core | 修复：Task 3.2 纳入该文件，提取只消费 previous_plan/outline/order/accepted revisions/compile id/now/已分配 IDs 的输入式 preparation；legacy wrapper 读 store，IR adapter 做 IR projection，共用 prepare/execute | 关键事实/目标文件结构、Task 3.2 |
| oracle Major 3：IR publication 中断重启仍走 legacy recovery | 修复：transaction durable 保存 source/IR/report/provenance refs，Continue 按 durable flow_kind 分流；SingleCandidate 从 durable refs 重建同一输入/compile ID/journal，legacy 保持原路径；补重启测试 | Task 2.5、Task 3.4、Task 5.1、Task 5.4 |
| oracle Major 4：初始 compile 时钟边界不确定 | 修复：唯一注入 `InitialPlanCompileInput.now` 覆盖 initial projection/publication/journal；共享核心禁 `Utc::now()`；人工 Abort/HumanTriage 可读实时钟；3.1 characterization 检查 transient timestamp 不参与选择/恢复 | Global Constraints、Task 3.1、Task 3.2 |
| oracle minor：part_14 已存在、应新建 part_15；ledger 断言不得总数为 0 | 修复：Task 3.1/3.3 使用新 `part_15.rs` 并在 `part_03.rs` include；ledger 改为前后字节不变 + 新增 started=0 + 无 ProviderRunRequested | Task 3.1、Task 3.3 |
| reviewer minor 1：Case A/B 获授权后的执行路径与结果落点缺失 | 修复：Task 6.1 Step 4 明确授权分支按基线执行各 10 次并在报告记录脱敏 10/10；未授权记录并停止 | Task 6.1、Task 6.4 |
| reviewer minor 2：REQ-WSC-03 预算耗尽后新 finding 未显式断言 | 修复：Task 5.1 明确“非重复的新 repairable finding → awaiting_human/stopped_needs_human，不 fatal/不空转” | Task 5.1 |
| reviewer minor 3：coding driver 无需适配的依据缺失 | 修复：Task 6.1 逐一核对两个 driver，记录 coding driver 不读 workitem 协议且无需修改 | Task 6.1 |
| reviewer minor 4：5.2 阈值容易被误当契约红线 | 修复：Task 5.2 标注阈值为实现自由度/internal diagnostic，可按实测调整但保持确定性 | Task 5.2 |

### 第 3 轮修订（针对第 2 轮三方复审）

> 下表行号均指本文件第 3 轮正文的稳定行号；同一根因在不同复审来源重复出现时分别保留，便于逐条核验。

| 来源 / finding | 处理结论 | 正文行号区间 |
|---|---|---|
| r2-sol blocker 1：durable compile metadata 只有叙述、没有真实类型/API 落点 | `outline.rs` 已进入 3.2/3.4 Files；transaction 七个字段全为 serde-default `Option`，旧 JSON→Legacy；adapter→durable context→首个 put→Continue 的 API/断言完整 | 正文 551-605、646-691 |
| r2-sol blocker 2：Task 4.2 遗漏 section 外 B 层句子 | 删除清单逐句覆盖 `prompts.rs:49`、`:83`、两个 section 与 hard rule；测试只禁 B 层精确句，并正向保留 C 层 `writing-plans` 边界 | 正文 743-768 |
| r2-sol blocker 3：flow_kind 分发绕开真实 Provider execution surface | Files 纳入 `ProviderRunKind` 与 `provider_run.rs`；纠正当前 Start 调 `build_outline_invocation` 的事实；增加 handler→run kind→provider runner→parser 双 flow 集成链 | 正文 776-796 |
| r2-sol blocker 4：scope raw-object 在 handler 前丢失 | `socket.rs` 纳入修改面；新增 raw `Value::Object` envelope/submitted-fields marker，在任何 durable 写前按 SingleCandidate 定向拒绝并断言字节不变 | 正文 804-831 |
| r2-oracle blocker 1：durable transaction schema 未落在真实模型 | `WorkItemPlanCompileTransaction` 新字段、Legacy default、完整性检查、旧 JSON 与 SingleCandidate roundtrip 均有 Files/测试/实现步骤 | 正文 551-605、646-691 |
| r2-oracle blocker 2：provenance ref 无 immutable durable owner/API | 固定 `WorkItemPlanSourceStore` 为唯一 owner，定义 put/get、完整 refs、非递归 canonical content hash、identity mismatch、journal fingerprint 与 recovery reload | 正文 442-497 |
| r2-oracle blocker 3：WP3 过滤路径会 0-test 假绿 | 改用扁平模块真实函数前缀/现有恢复过滤名；关键命令先 `-- --list`，记录基线匹配数与新增数量下限，最后运行整个 `workspace_engine::tests` | 正文 514-537、609-635、681-691 |
| r2-oracle blocker 4：3.1 未先证明 transient updated_at 与选择/恢复无关 | characterization 在 3.2 前反转合法 `updated_at`，断言 selection/matching/Continue/cursor/产物不变，同时保留 `created_at` 排序对照 | 正文 510-536 |
| r2-oracle major 1：5.4 未覆盖 SingleCandidate initial-publication recovery | 3.3 与 5.4 各自增加五个 `InitialPlanPublicationCheckpoint` 矩阵；重启重载 transaction/source/provenance，并断言 `publication_resumed`、ID/ref/hash/ledger/event 幂等 | 正文 609-635、1049-1062 |
| r2-reviewer minor 1：6.4 正则把换行写成双反斜杠字符类 | heredoc Python 统一使用真实 `r'[^\n]*'` 同行正则 | 正文 1232-1244、1269-1276 |
| r2-reviewer minor 2：脚本正文声称检查 result path、实际未断言 | 已授权逐一断言文件存在且完整 path 出现在报告；未授权断言两条“未生成（未授权）”path 文本 | 正文 1246-1273 |
| r2-reviewer minor 3：usage 标记在数据完整时也被无条件强制 | `usage_missing` 只在未授权、usage 缺失或明确 unavailable 时为真，才要求 `usage_unavailable`/`inconclusive` | 正文 1245-1276 |
| r2-reviewer minor 4：Task 4.5 引用不存在的 Task 4.3a | Files 改为 Task 4.1～4.3 production files，并明确补充任务是 4.2a、不存在 4.3a | 正文 873-888 |

### 第 4 轮修订（针对第 3 轮三方复审）

> 下表为本轮仅处理的 4 个 delta；行号以本轮正文最终版本为准。

| 来源 / finding | 处理结论 | 正文行号区间 |
|---|---|---|
| sol R3-1 + oracle B2：source/IR/mechanical report 仍只有未定义的通配读取表述 | 写前 grep 确认仓库不存在隐含 `WorkItemPlanSourceStore`/typed API；Task 2.5 定义三类 immutable typed record、完整 scope path、精确 put/get、canonical content hash、重复写与 stable failure code；Task 3.4 Continue 按 scope 依次重载 source revision、IR、mechanical report，全部校验成功后才进入共享 pure prepare，并显式禁止未列出的读取 API | 正文 538-550、737-739 |
| sol R3-6：provenance put/get scope 不自洽 | put/get 均显式携带 `project_id`、`issue_id`、`plan_id`；put 写入并校验与 get 相同 durable scope，`provenance.plan_id == plan_id`，不依赖 constructor 隐含绑定 | 正文 538-540 |
| oracle B1：compile reservation 存在 provenance/transaction 前空窗 | 新增 typed `SingleCandidateCompileReservation` 与唯一 CAS API；首个 provenance/transaction 写入前先持久 reservation，compile ID/now/provenance ref/publication IDs 确定性复用；补齐四个 crash boundary 及每次重启同三元组/ID 断言，落入 Task 2.5、3.2、3.4、5.1、5.4 | 正文 481-542、648-662、737-750、993-1007、1116-1122 |
| oracle B3：最终 WP3 门禁 `part_03` 可 0-test 假绿 | 改为 `workspace_engine::tests`，先运行同过滤 `-- --list`，统计 `: test` 并保留 `MATCH_COUNT > 0` 断言；写入前实测匹配 881 项 | 正文 1405-1416 |

### 第 5 轮修订（针对第 4 轮 delta 复审）

> 下表为本轮仅处理的 2 个 finding；行号以本轮正文最终版本为准。

| 来源 / finding | 处理结论 | 正文行号区间 |
|---|---|---|
| r4-sol + r4-oracle B1：reservation 缺 durable Approval 种子，CAS 未比较完整 session/版本且丢失 persistence fatal 语义 | Task 5.1 将 `approval_attempt_id`、`approved_at` 落入兼容 session schema并以唯一 Approval CAS 原子保存；reservation 前重启从 durable Approval record 重算同三元组。`put_compile_reservation_cas` 接受 expected 完整 session、校验 scope/flow/Approval/refs/tuple、返回更新 session，并区分 Conflict、InvalidSession 与 persistence failure→`FatalReason::PersistenceFailure`；5.4 覆盖 seed-only crash、stale record 和写盘失败 | 正文 481-560、1011-1025、1134-1140 |
| r4-oracle B2：transaction 只存 refs，Continue get API 仍要求重启前内存 ID | 选择 get API 直接接收 `SourceStoreScope` + canonical ref；`source_revision_id` 保留并持久化到 transaction，Continue 的 get API 不再以该 ID 为输入，仅消费 transaction refs；固定 malformed/wrong-kind/scope-mismatch/dangling 四类稳定码及优先级，并在 5.4 恢复矩阵表驱动覆盖 | 正文 507-560、656-668、734-759、1134-1138 |

### 第 6 轮修订（针对第 5 轮裁决）

> 下表为本轮仅处理的 5 个 finding；行号以本轮正文最终版本为准。

| 来源 / finding | 处理结论 | 正文行号区间 |
|---|---|---|
| r5-sol blocker 1：Approval CAS 缺锁内完整 CAS 契约、幂等/拒绝覆盖及错误分类 | 对照 `src/product/lifecycle_store/workspace.rs:519-555` 补齐排他锁内重读、stored/expected 全 record equality、stale→`Conflict`、相同二元组幂等、不同值禁止覆盖、IO/JSON/write 错误原样保留；持久化边界根据 session/plan/三 refs 重算并逐字校验 `approval_attempt_id`，任意传入值拒绝 | 正文 560-562、1144 |
| r5-sol blocker 2：compile tuple 编码不具 canonical bytes 定义 | 展开 domain separator、session_id、plan_id、approval_attempt_id、approved_at 的逐字段 NUL 分隔字节拼接，并补固定输入 canonical bytes hex 与预期 `compile_id` 测试向量 | 正文 560、1144 |
| r5-sol blocker 3：SourceStoreError 缺持久化错误域 | 增加 `Io`/`Json`/`Serialize`（或等价 `ProductStoreError` 包装）变体，保留底层信息；明确不得压成 `DanglingRef`/四类语义码，调用方映射 fatal persistence failure | 正文 515-545 |
| r5-sol blocker 4：scope 数量与 Continue 时序矛盾 | 全文改为三个业务 scope 字段（object kind 单独校验）；明确 reservation/provenance 已存在而 transaction 不存在时先做 session recovery 并创建首个 transaction，已有 transaction 的 Continue 只做后续 journal put/resume | 正文 566、576、761-763、1144 |
| r5-sol 非阻断措辞：第 5 轮记录对 adapter ID 的描述与正文不一致，且 5.4 缺三条 Approval CAS 断言 | 修正第 5 轮子表为 `source_revision_id` 保留并持久化到 transaction、Continue get API 不再以其为输入；5.4 B 矩阵补 Approval CAS stale/full-record、非法 `approval_attempt_id`、固定 hash 测试向量三条断言 | 正文 1144、1569-1576 |

**修订后自查结论（writing-plans 三件套）：**

1. **覆盖映射：** 已核对 OpenSpec 四件套的 26 个契约 task、REQ-WSC-01/02/03/05/06/07；计划任务总数为 28（另加 `4.2a`、`6.1a` 两个接线任务）。覆盖表中 `4.2a` 紧随 4.2 并映射契约 4.1/4.2，`6.1a` 紧随 6.1 并映射契约 6.1/REQ-WSC-07；REQ-WSC-04 仍不适用且未臆造。
2. **占位符/时间/hash：** 已核对 Task 3.1/3.3/3.4 的 observer、稳定 ID 占位符引用关系、RFC3339 与 created_at 稳定性、`publication_resumed`、projection/contract hash；无“删除所有 ID”“忽略 hash”“直接依赖最终 JSON 推导 journal”的表述。
3. **类型一致性：** 已核对 `InitialPlanCompileInput` 仍与契约精确 13 字段一致，provenance/freshness 未塞入该 struct；`logical_targets` 保持 `Option<BTreeMap<LogicalRepositoryId, String>>`；validator 与 projection context 使用已由 outline/定向阅读核实的区间；legacy/IR publication context 为独立外层数据。
4. **命令/范围：** 已扫描计划命令，无 `-j 1`；Rust 定向命令均带 `--lib`，Node 命令均指向具体 `.test.mjs`；真实 campaign 仍有授权门，未授权报告不得出现 `2/2`；coding、story/design、阶段 3/4 非目标保持不变。
