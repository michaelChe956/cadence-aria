# WP5 实施移交笔记（T5 中断交接 · 2026-09-19）

> 状态：**进行中（未完成）**——L2 后端删除核心面已落地且 **lib（产码）编译绿**；测试面 157 errors 待清（退役/重钉）；Step 4 preflight、前端归零、残留断言、门禁、报告未做。工作树未提交代码变更（仅归属表 docs commit）。

## 已完成（产码面，lib `cargo check` 绿）

1. **Step 1 归属判定表**：`wp5-attribution-table.md`（本 commit）——必删集 6 族+共享变体逐符号复核+ContextBlocker oracle 二选一判定=(b) 随 L2 删+SC 可达 review_decision 两处改道处置+flow_kind 收敛落点。
2. **Step 2 wire 层**（红测先行：`tests/it_core/workspace_ws_integration/part_07.rs` 已写并实测红，删除落地后待复跑绿）：
   - `in_.rs`：删 10 变体（ReviewDecisionResponse/AuthorDecision/SelectWorkItemGenerationMode/SelectRevisionPath/RequestOutlineRevision/WorkItemDraftDecision/WorkItemBatchDecision/SaveHumanPresentationRevision/HumanConfirm/RevertWorkItem）+7 DTO；`WorkItemGenerationModeDto` **迁至 artifact.rs**（历史 artifact 载荷字段+SC 内部诊断 `select_internal_generation_mode` 消费，wire 面归零——残留断言符号表不含该 DTO 名）。
   - `protocol.rs`：`RETIRED_INBOUND_MESSAGE_TYPES`+`retired_inbound_message_type`+`retired_message_protocol_error`（code=`LEGACY_MESSAGE_RETIRED`，含 stage 上下文）；stage 白名单删退役臂（ReviewDecision 阶段恒 false；HumanConfirm legacy 臂留 Confirm/amendment/compile-recovery）；删 `single_candidate_generation_decision_error`；新 2 单测。
   - `socket.rs`：parse 错误路径接退役识别（stage-specific protocol error）；删 `single_candidate_generation_decision_bypasses_stage_validation`+其单测。
   - `decisions.rs`(handler)：删 `handle_review_decision_from_handler`/`handle_author_decision_from_handler`/`handle_human_confirm_from_handler`；新增 `handle_confirm_from_handler`（非 SC Confirm 帧直连 `handle_confirm`，错误码沿用 INVALID_HUMAN_CONFIRM_ACTION——前端 GATE_REJECTION_CODES 依赖）。⚠️ 过程中曾误删 `handle_human_gate_feedback/termination_from_handler` 两保留函数，**已原样恢复**（diff 核对过）。
   - `inbound.rs`：删 9 个退役路由臂（各臂留退役注释）；Confirm 臂 else 分支切直连；RequestRevision 非 WorkItemPlan 桥接→protocol error（REQUEST_REVISION_WORKSPACE_INVALID）；删 `single_candidate_generation_decision_error` 调用。
   - `mapping.rs`：删 `map_revision_path`。
3. **Step 3 引擎层**：
   - `workspace_engine/decisions.rs`：删 `handle_human_confirm`/`handle_work_item_plan_context_blocker_decision`/`append_work_item_plan_context_blocker_resolution`/`handle_author_decision`/`handle_work_item_plan_outline_decision`/`handle_review_decision`/`skip_work_item_plan_optional_findings`+payload/context-blocker helpers（enter_* 状态进入函数保留——在途会话读侧）。
   - `draft_batch/decisions.rs`：删 `handle_work_item_batch_decision`/`handle_work_item_draft_decision`（accept/rewrite/downgrade 内部函数保留）；`draft_batch/runs.rs`：删 `select_work_item_generation_mode`。
   - `plan_outline/authoring.rs`：删 `request_work_item_plan_outline_revision`。
   - `human_presentation.rs`：删 `save_human_presentation_revision_command`+自由函数+`SaveHumanPresentationRevision`/`HumanPresentationScope` 类型；`out.rs` 删 `HumanPresentationRevisionSaved/SaveFailed` 两出站变体。
   - `author_confirm.rs`：删 `apply_revert_mark`。
   - **SC 可达面保护**（归属表 §4）：`review/routing.rs` `_` 臂 RequiresRevision 加 SC 守卫→`enter_human_confirm`（非 SC 保留 enter_review_decision=在途限制）；`plan_repair_review.rs` `route_plan_repair_candidate_review` 同款 SC 守卫。
   - `mod.rs`（engine/ws_handler/ws_types）re-exports 收敛。

## 待办（按序）

1. **测试面清障**（`cargo check --all-targets` 157 errors，分布见下）——处置规则：直接调用已删符号/构造已删变体的 legacy 回归测试→删除+退役注释（T1 矩阵「legacy 路径回归全绿」子项已留档 wp1-gate-retest/evidence-matrix.md:22）；锚保留面（Confirm/compile recovery/amendment/typed 门）的→重钉。
   - 产码内嵌测试：controls.rs(53)、human_presentation.rs(6)、protocol.rs(2)
   - engine tests：part_06(22)/author_revision_loop(15)/part_20(13)/part_03/part_07(8)/part_03/part_02(7)/part_04(6)/part_03/part_05(6)/part_03/part_01(4)/part_05(3)/review/tests_policy_routing(3)
   - handler tests：human_presentation(8)/tests.rs(8)/single_candidate_scope_rejection(5)/conversational_gate_protocol(3)/campaign_stage3_interactive/cases(3)/conversational_gate_stage(2)
   - ws_types tests.rs(6)
   - it_core：workspace_ws_integration part_04(5)/part_02(5)/part_01(2)；it_web：web_work_item_plan_mode/part_04(3)/web_workspace_recovery_consistency/part_03(2)
   - 备注：`update_work_item_plan_outline_generation_metadata`（plan_outline.rs:434）删除 wire 入口后仅剩 tests/part_03/part_04.rs:505 调用——该测试退役时此函数一并处置（当前为 pub(crate) 会有 dead_code 风险，编译警告待清）。
2. **Step 4 preflight 收敛**（未动）：`lifecycle.rs:647-708` 删 `SingleCandidatePreflightDecision::LegacyFallback` 两分支→preflight 失败仍建 SingleCandidate 会话+`mark_single_candidate_prepare_failure` 同款 durable Failed+原因（HTTP 错误带 reason）；`lifecycle_tests.inc.rs:567` 重钉（0/2 仓→SC Failed+原因，无 legacy 回落）；:514/:545 两 rollout 默认测试随「新会话一律 SingleCandidate」重钉/退役；`inputs.rs:232` Default flow_kind 翻 SingleCandidate；`models/workspace.rs:53` serde 默认**保持 Legacy**（历史记录缺字段读兼容，归属表 §5）。
3. **REQ-RET-03 兼容测试**：durable flow_kind=Legacy 历史 record 读取只读呈现+在途决策拒绝——part_07 已覆盖拒绝面，读侧面可并到 it_core 或 lifecycle_tests。
4. **前端归零**（未动）：useWorkspaceWs 删 sendSelectRevisionPath/sendAuthorDecision/sendSelectWorkItemGenerationMode/sendRequestOutlineRevision/sendWorkItemDraftDecision/sendWorkItemBatchDecision/sendReviewDecision；ChatCockpitPage 删 handleAuthorDecision(:463-472)/staged props(:1074-1078)/ReviewDecisionActionBar 块(:1082-1090)/reviewDecisionOptions(:431-436)；ChatWorkspacePageParts 删 ReviewDecisionActionBar 组件；ChatInputBar 删 staged 决策 props+按钮+revise 输入径（compile recovery 回调保留）；workspace.ts union 删 8 分支+RevertWorkItemMessage+SaveHumanPresentationRevisionMessage+相关类型（common.ts AuthorDecision/RevisionPath/WorkItemGenerationMode 等——注意 `WorkItemPlanCompileRecoveryAction` 类型保留）；store 面 human_presentation begin/fail 动作+出站 Saved/Failed 处理器删除；`CockpitOperation` 联合删 request_change/terminate 历史值；前端测试面同批清障（vitest+tsc）。
5. **门禁+断言**：`cargo fmt --check`+clippy -D warnings+`cargo test --locked` 三 target 绿+web `npm test`/tsc；残留断言（9 符号，文件级豁免=负向测试 part_07+报告目录）→ `wp5-residue-scan.md`。
6. **提交**（分批显式列文件）：代码面单批原子 commit（src+web+tests 同批，计划 Step 5 原子收口要求）→ `stage4-c3-t5-report.md`+residue-scan docs commit。

## 已知偏差记录（供报告如实登记）

- `WorkItemGenerationModeDto` 未从仓库消失而是迁至 artifact.rs（历史 artifact 载荷 `selected_generation_mode` 字段+SC 内部诊断消费）；计划必删集字面「+WorkItemGenerationModeDto 删除」按「wire 面归零」执行，残留断言符号表不含该名。
- 非 SC `Confirm` 帧保留直连 `handle_confirm`（T4 §4 登记「approve（confirm 帧）legacy 分支仍接受不受影响」的承接）。
- part_07 负向测试当前应已转绿（parse 面拒收）——**未复跑**（all-targets 未过）。
