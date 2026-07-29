# 计划文档：移除 Testing 阶段与 Tester 角色

- **OpenSpec Change**：`remove-testing-stage`
- **Capability**：`testing-stage-removal`
- **前置 Change**：`remove-work-item-handoff`（必须先完成）
- **日期**：2026-07-28
- **版本**：v1.1（评审后修订：根因改为静默跳过、`test_executor.rs` 改逐符号处置、移除 `mutation_test_pause.rs` 误删项、补齐字符串键与前端落点、新增执行链契约与审计数据源两项）

## 目标

彻底移除 Coding Workspace 的 Testing 阶段与 Tester 角色，使阶段集合收敛为准备上下文 → 准备 worktree → Coding → Code Review → 请求评审 → 最终确认，并消除"测试证据缺失即阻塞"与"恢复路径送入无编排入口阶段"两类失败模式。

## 前置条件

- `remove-work-item-handoff` 已完成。硬依赖是 `handoffs.rs:357-368` 的 `generate_placeholder_work_item_handoff` 在 `:360` 调用 `list_testing_reports`——前者整体删除该函数使这个消费者消失；若本 change 先做，会撞上一个 Impact 清单外的调用点。
- 归属划分：`list_testing_reports` 的 `handoffs.rs:360` 归 A，其余五处（`reports.rs:11`、`reviewer_context.rs:18`、`plan_defect.rs:427`、`web/coding_ws_handler/state.rs:30`、`web/handlers/coding.rs:552`）归本 change；`handoff_tests_run` / `handoff_test_result_summary` 归 A，本 change 不重复处理。
- 工作树可编译、`cargo fmt --check` 与 `clippy -D warnings` 干净。
- 已知既有失败基线：`large_file_guard`（会使 `cargo test --locked` 提前终止，因此 `it_web` / `it_provider` / `it_task_run` / `it_product` 需单独运行）。

## 关键约束

### 历史持久化数据不做兼容

按用户决定按全新系统处置：**不写迁移、不加 `#[serde(other)]` 兜底成员、不加忽略未知阶段值的兼容层、不为历史记录写兼容测试**。移除枚举成员后，含 `"stage":"testing"` 的历史 attempt 记录可能不可读，这是已接受的取舍。

实施中遇到任何"旧记录含 testing"的场景，一律不加兼容分支。

### 两套 TestingReport 必须区分

| 标识 | 位置 | 用途 | 处置 |
|---|---|---|---|
| `coding_models::TestingReport` | `src/product/coding_models/testing.rs:115` | Coding Workspace 强类型模型 | 移除 |
| `ArtifactKind::TestingReport` | `src/protocol/artifacts.rs:19` | task_run 执行链 N17 的 JSON artifact 种类 | 移除 |

两者独立，都属于被废弃职责，一并移除。

### 🔴 `test_executor.rs` 按符号处置，不整体保留

**原判断「整体保留」与删除 `coding_models/testing.rs` 直接冲突**：`test_executor.rs:13-15` 正是 import 该模块的 `TestCommand`、`TestCommandStatus`、`TestingOverallStatus`、`TestingReport`；`run_all_tests`（`:331-377`）的返回类型就是 `TestingReport`。

按实际生产消费者逐符号处置：

| 符号 | 生产消费者 | 处置 |
|---|---|---|
| `planned_test_commands_from_markdown`（`:81`）、`TestCommandSpec` | `coding_work_item_context.rs:378,:431` | **保留**（只返回 `Vec<TestCommandSpec>`，不依赖被删模型） |
| `run_all_tests`（`:331`） | 仅 `testing.rs:30`（被删） | 移除 |
| `remove_test_command_artifacts`（`:379`） | 仅 `testing_provider/execution_tool.rs:60,:90,:105`（被删） | 移除（保留必触发 `dead_code` 撞 `-D warnings`） |
| `execute_test_command_with_cancellation`（`:154`） | 仅 `tester_agent_loop/executor.rs:95`（被删） | 移除 |
| `infer_test_commands`（`:60`） | 仅 `tester_agent_loop/prompts.rs:26`（被删） | 移除 |
| `discover_test_commands`（`:37`） | **无生产调用方**（仅 `tests/it_product/product_test_executor.rs:23,:37`） | 移除 |

集成测试 `tests/it_product/product_test_executor.rs:287,:320` 仍在用 `run_all_tests`，需同步处置。

**这个判断不能推给实施阶段实测**：`TestingReport` 的存废是删除 `coding_models/testing.rs`（阶段三）的前置条件。

### 🔴 `mutation_test_pause.rs` 不在移除范围

原 Plan 误将其列入删除清单。`CodingMutationTestPoint`（`mutation_test_pause.rs:8-13`）只有 `GroupCompletionRunning`、`GroupCompletionCompletedRetry`、`ProviderFailure` 三个点，**与 Testing 无关**——大概是与 `testing_provider/test_pause.rs`（`TesterToolCommitTestPoint`，确实在移除范围）混淆。

其生产调用方是 `group_completion.rs:52-56` 与 `provider_failure.rs:108-110`，测试消费者是 `tests/group_completion_recovery.rs:263,:303` 与 `tests/provider_failure_recovery.rs:471`。删除会破坏 group completion 与 provider failure 的并发恢复测试脚手架。

### 🔴 「编译器兜底」不成立，覆盖靠清单

原 Plan 把「编译器定位 + `pnpm tsc -b` 兜底」作为主要控制手段。实测：`Stage::Testing` 在 `src/` 非测试路径 28 处，扣除整体删除的文件后只剩 14 处会成为编译错误。以下落点两个工具都不报：

- **字符串键**：`coding_workspace_runner.rs:48,:80-87` 的 `"tester"|"tester_plan"|"tester_execute"`、`web/coding_ws_handler/runner.rs:162-180` 的 gate action_id 白名单、`cross_cutting/streaming_provider/fake.rs` 的 JSON 字面量、N17 协议面的多处匹配
- **TS 结构类型与字符串联合**：`CodingArtifactTab` 的 `"tests"` 成员、switch case 与 union 成员不同步时均不报错

因此每个阶段都必须按清单逐项核对，`cargo check` 只是辅助。

### `AmendmentResumeMode::Revalidate` 只改阶段落点

| 位置 | 内容 | 处置 |
|---|---|---|
| `amendment_recovery.rs:82` | `Revalidate => CodingExecutionStage::Testing` | 改为 `CodeReview` |
| `unit_run_amendment.rs:631,639` | `Revalidate => NeedsRevalidation`（unit / unit run 状态） | 不变 |
| `group.rs:44` | `Revalidate` 映射 | 不变 |
| `plan_repair/engine.rs:704` | 产生 `Revalidate` | 不变 |
| `unit_run.rs:525,528,537`、`unit_run_handoff.rs:249,286,304`、`coding_models/group.rs:14,29`、`plan_repair.rs:22,36`、`runtime_impact.rs:428`、`web/handlers/dto.rs:455` | 只碰 unit/run status 枚举 | 不变 |

枚举成员本身不移除：其语义独立于 testing，移除会超出本 change 范围。

**需要裁定的口径分歧**：改为 CodeReview 后，`amendment_recovery.rs:82`（attempt stage=CodeReview）与 `group.rs:44`（unit status=NeedsRevalidation）会并存——attempt 说「去做代码审查」，unit 说「待重新验证」。这个组合在移除前不存在（Testing 阶段对应 Tester 验证）。1.5 需断言该组合可推进。

### 根因表述：静默跳过，不是静默停机

原 Plan 与 proposal 把后果写成「attempt 落到无编排入口的阶段就静默停机」——**这是错的**：

- `runner.rs:446` 的分派是序号比较 `current.stage.order() <= CodeReview.order()`，Testing(3) ≤ CodeReview(4) 成立，attempt 直接进入 Code Review
- `code_review.rs:26` 再把 stage 改写为 CodeReview，`coding_attempt_store/attempt.rs:709` 的 `next.order() >= current.order()` 允许该跃迁
- `runner.rs:634` 的 `continue 'pipeline` 同样是通路

实际行为是**静默跳过**：记录显示经过了 Testing，实际什么都没执行，阶段语义失真而无错误暴露。

这不改变移除的正当性，但**决定了验收方式**：回归测试必须断言阶段字面值，不能断言「不停机」——后者在改动前已成立，写成测试不会失败。

## 实施步骤

### 阶段一：失败测试（工作包 1.1–1.10）

先写测试，此时应全部失败或无法编译。

**🔴 1.5 恢复路径（最高优先，且必须改写断言）**：在 `coding_attempt_store` 的 amendment recovery 测试中断言 `Revalidate` 恢复后 attempt 阶段**字面值恰为 Code Review**，并断言 attempt stage=CodeReview 与 unit status=NeedsRevalidation 并存时可推进。

**不得断言「可继续推进、不停机」**：该性质在改动前已成立（Testing 会被序号比较直接跳过），断言它不会失败，抓不住任何回归。这是本 change 最容易写出假绿灯测试的地方。

**1.6 门禁动作集合**：断言门禁不再提供四个 testing 动作，且 Code Review 的 `RetryReview` 与分诊动作仍可用。参照 `coding_workspace_engine/tests/gate_rework.rs` 的夹具。

**1.7、1.8 评审侧**：断言无测试报告时 group final review 不判要求修改；断言评估上下文不含测试派生字段。参照 `coding_evaluation_context` 与 `tests/parser_prompt.rs` 系列。

**1.1、1.2、1.3、1.4、1.9 阶段与产物**：断言阶段序列、阶段序号相对顺序、attempt 目录无测试产物、会话状态无测试报告字段、完成判定不要求测试报告。

**1.10 保留能力**：断言 `planned_test_commands_from_markdown` 与 `TestCommandSpec` 在 work item 上下文侧的行为不变（不是「Coding 侧发现与执行」——按新的逐符号处置，执行类函数会被移除）。

**1.11 保留节点执行链契约**：断言 N18/N19/N20 的必需产物集合变化符合预期、不存在指向已移除 N17 的失败路由。参照 `tests/it_core/artifact_schema_min_fields.rs:144,:197`、`tests/it_core/phase1_profile.rs:104`、`tests/it_provider/execution_chain_fake_provider/part_01.rs:130,:226,:260`（后者断言节点序列 `["N16","N17","N18"]` 与 `["N16","N17","N19","N17","N18"]`，必然要改）。

**1.12 质量绕过审计**：断言 `latest_missing_required_steps` 原先依赖测试报告的字段已被移除或改为明确空语义。

阶段一提交建议：`test: 为移除 Testing 阶段补充回归测试`（允许红灯，作为 TDD 起点）。

### 阶段二：移除枚举成员，让编译器定位残留（工作包 2.1）

改 `src/product/coding_models/execution.rs`：移除 `CodingExecutionStage::Testing`、`CodingProviderRole::Tester`，收紧 `order()`。

此时全仓编译报错。**先只跑 `cargo check --locked` 收集完整错误清单**，据此确认下面各阶段的实际改动面，不要边改边猜。

移除 `coding_workspace_runner.rs:35` 的 Testing→Tester 映射。

提交建议：与阶段三合并（单独提交无法编译）。

### 阶段三：删除执行实现（工作包 2.2、2.3）

删除文件与目录：

- `src/product/coding_workspace_engine/testing.rs`
- `src/product/coding_workspace_engine/testing_parser.rs`
- `src/product/coding_workspace_engine/testing_provider.rs`
- `src/product/coding_workspace_engine/testing_provider/`（execution.rs、execution_tool.rs、execution_types.rs、plan.rs、report.rs、test_pause.rs）
- `src/product/tester_agent_loop/`（9 文件）
- `src/product/coding_evaluation_context/tester_execution.rs`（298 行；唯一生产消费者是 `testing_provider/execution.rs:22`）
- `src/runtime_units/testing.rs`（`TestingUnit`，`covered_protocol_nodes = ["N17"]`）
- `src/product/coding_models/testing.rs`

🔴 **`mutation_test_pause.rs` 不删**（见关键约束）。

删除 `coding_models/testing.rs` 前必须先按工作包 2.12 处置 `test_executor.rs:13-15` 的 import，否则本阶段无法编译通过。

同步移除模块声明与再导出：`coding_workspace_engine/mod.rs`、`coding_models/mod.rs`、`product/mod.rs`、`coding_evaluation_context/mod.rs:18`（`pub use`）与 `:27`（`EvaluationContextRole::Tester`）、`:97-125`（`TesterExecutionContextPack` / `TesterExecutionSourceArtifacts` / `TesterExecutionWorkItemContext`）、`runtime_units/mod.rs:29`。

提交建议：`refactor: 移除 Testing 阶段与 Tester 角色的执行实现`。

### 阶段四：存储与路径（工作包 2.4）

- `coding_attempt_store/report.rs`：移除 `save_test_plan`、`list_test_plans`、`save_testing_report`、`get_testing_report`、`list_testing_reports` 及 `testing-reports` 目录拼接。
- `paths.rs:209`：移除 `test_plans_root`。
- `utils.rs:222`：移除 `Testing => "testing"` 阶段字符串映射。

提交建议：`refactor: 移除测试计划与测试报告的存储路径`。

### 阶段五：门禁与路由（工作包 2.5、2.6、2.7）

- `coding_models/gate.rs:94,95,101,102`：移除 `RetryTestPlan`、`RerunMissingSteps`、`AcceptTestingResult`、`RerunTesting`。
- `coding_models/role_run.rs:20,21`：移除 `RetryTestPlan`、`RerunMissingSteps` 触发原因。
- `coding_workspace_engine/gates.rs:557-579`：移除该分支。门禁构造侧的唯一产生点已确认是 `testing_parser.rs:286-323`（`build_testing_gate_actions`）与 `:379-412`（`parse_gate_action`），随整文件删除。
- `web/coding_ws_handler/runner.rs:162-180`：`should_resume_runner_after_gate_response` 的**字符串白名单**含四个 testing action_id，须同步移除。**编译器不报错。**
- `coding_models/role_run.rs:24`：移除 `CodingRoleRunTrigger::ManualRerun`——其唯一生产产生点是 `gates.rs:566-567`（RerunTesting 分支与 `_` fallback），随门禁动作移除后失去全部触发点，保留会成为本 change spec 自己禁止的占位成员。
- `plan_defect_routing.rs:311` 与 `plan_defect.rs:19-24`：移除 `PlanDefectSource::Tester` 成员及其 `label()`。第三个消费者 `plan_repair_start.rs:85-92` 的 source 白名单 `Coder | Tester` 收窄为仅 `Coder`——其生产调用方 `runner.rs:204` 的 source 恒为 `Coder`，**行为不变**，但这是判断「移除是否改变 plan repair 唤起条件」（非目标）的关键点，须确认结论成立。
- `coding_attempt_store/unit_run.rs:261`：移除 `CodingProviderRole::Tester` 的 identity_mismatch 分支。
- `coding_models/provider_config.rs`：移除 tester 配置面——`CodingRolePermissionModes.tester`（:20,:29）、`CodingRoleProviderConfigSnapshot.tester_plan`/`tester_execute`（:52-53,:75-76）、`tester_plan_provider`/`tester_execute_provider`/`set_*`（:129-142）、`Display for CodingProviderRole` 的 `Tester` 分支（:40）。该 struct 带 `#[serde(deny_unknown_fields)]`，属持久化契约面。
- `coding_workspace_runner.rs:48,:80-87`：移除 `parse_coding_provider_role` 的 `"tester"|"tester_plan"|"tester_execute"` 与 `apply_provider_selection_to_snapshots` 的两个字符串分支。**字符串键，编译器不报错。**
- `coding_evaluation_context/methods.rs:8,:34`：移除 tester 的 `systematic_debugging`/`verification_before_completion` 注册与 `role_key` 分支。
- `review_parser.rs:93`：移除 `"testing"` 阶段解析。**注意失败模式**：该函数用 `serde::de::Error::unknown_variant`（`:100-113`），删掉 `"testing"` 后 provider 输出 `source_stage: "testing"` 会变成**硬反序列化错误**，而非落回默认值——`:200` 的 `unwrap_or(default_source_stage)` 只覆盖字段缺失（`#[serde(default)]`），不覆盖值非法。reviewer prompt（`prompts.rs:36`、`render.rs:478`）已要求 `source_stage=code_review`，实际风险低，但需明确接受该失败模式。
- `timeline.rs:27,37`：移除 `create_testing_timeline_node`。
- `amendment_recovery.rs:82`：改为 `CodeReview`。

提交建议：`refactor: 移除 testing 门禁动作并重定向重校验恢复`。

### 阶段六：评审上下文与提示词（工作包 2.8）

- `coding_workspace_engine/prompts.rs`：移除 `build_tester_execute_plan_prompt`（:444-453，本文件唯一的 Testing 相关表述）。**注意 `code_review_material_protocol`（:317-340）与 `group_final_review_material_protocol`（:342-361）中没有任何 Testing / TestingReport / tester 字样**，无需在此改动。
- `test_evidence_refs` 的两个生产者：`reviewer_context.rs:16-23,:42` 与 `plan_defect.rs:425-437`，均调 `list_testing_reports` 填 `ReviewerExecutionEnvelope.test_evidence_refs`；同时移除 `work_item_projection/execution_context.rs:16` 的字段本身。
- ⚠️ **该字段经 `render.rs:449` 的 `ReviewExecutionEvidence` section 进 reviewer prompt，且是 mandatory section（`render.rs:80`）**。移除字段必须同步处理该 section，否则 projection 渲染会失败。这条渲染路径与 `fix-process-evidence-as-acceptance` 要改的是同一处，两个 change 在此有交集。
- **`handoff_tests_run` / `handoff_test_result_summary` 归 `remove-work-item-handoff`**（`builder.rs:383-389`、`coding_evaluation_context/mod.rs:59-60`），本阶段不重复处理。

**注意**：这是本 change 唯一改变 provider 输入的地方。段落删除后 reviewer 判断依据变化，需在工作包 3.5 由用户实际验证。

提交建议：`refactor: 移除评审上下文与提示词的测试证据要求`。

### 阶段七：协议与执行链（工作包 2.9、2.10）

- `src/protocol/artifacts.rs:19`：移除 `ArtifactKind::TestingReport`；`artifacts.rs:41` 的 `all_phase1()` 同步调整。
- `src/cross_cutting/artifact_validate/`：`canonical.rs:233-238` 的 `TESTING_REPORT_FIELDS`、`profile.rs:93`、`rules.rs:19`。后两处是**多成员 or 模式**，删掉 `TestingReport` 后 `CodingReport`/`CodeReviewReport`/`IntegrationReport` 的校验行为不变（安全）。
- 🔴 `src/protocol/contracts.rs`：移除 testing 契约类型，并**显式重写保留节点的契约**——`:513,:516,:519,:522` 的 N17/N18/N19/N20 `required_artifact_kinds` 引用 `ArtifactKind::TestingReport`，移除后 **N18/N19/N20 三个保留节点**的必需产物集合会变；`:664` 的 `N17 → N19|X08` 与 `:666` 的 `N19 → N17` 是失败路由，**移除 N17 会改变保留节点 N19 的路由图**。另有 `:342-348`、`:432`、`:460`、`:469`、`:606` 的 N17 引用。
- `src/protocol/nodes.rs:18`、`prompt_manifest.rs:95`：N17 节点定义与 prompt manifest。
- `src/runtime_units/coding.rs`：移除 N17 节点（:134）、`testing_report_requires_current_worktask_rework`、`testing_report_has_only_out_of_scope_acceptance_failures`、`testing_report_ref`、`node_specific_fields` 的 N17 分支（:633-641）。**循环结构已确认安全**：`:133-169` 有两个 `continue`，`:144`（N17 驱动，随移除消失）与 `:162`（N18 blocking 驱动，由 `rework_or_hold`（:203-241）返回 true 触发），后者保留，`loop` 可原样保留、不需改成非循环结构。但需处理 `rework_or_hold` 内 X08 步骤的 `"trigger_node": "N19"`（:220）语义。
- `src/cross_cutting/provider_context_builder.rs:202,:207,:313`、`src/runtime_units/prompt_template_registry.rs:17,:183,:221`、`src/task_run/interactive_runner.rs:52`、`src/task_run/step_runner.rs:141`、`src/web/runtime/utils.rs:37`：N17 的**字符串匹配**落点，编译器不报错。
- `src/web/coding_ws_handler/protocol.rs:48`、`state.rs:29,127`：移除 `testing_report` 字段与读取。
- `src/web/handlers/coding.rs:551-555`、`src/web/types.rs:550`：移除 REST 快照 `CodingAttemptSnapshotResponse.testing_report`。
- `src/web/coding_ws_handler/runner.rs:634`、`socket.rs:758`：移除阶段分支。
- `src/web/handlers/dto.rs:714`、`src/web/test_controls/fixtures.rs:381,394,400`：移除阶段序列化与夹具阶段。
- `src/cross_cutting/streaming_provider/fake.rs`：JSON 字面量中的 testing 字段，**编译器不报错**。

提交建议：`refactor: 移除 testing 协议字段与执行链节点`。

### 阶段八：前端（工作包 2.11）

原 Plan 只列了 4 个源文件，实际生产落点约 20 个：

- `web/src/api/types/coding.ts:61,:117-128,:139-140,:184,:204-205,:410-418`：`TestingReport`、`TestPlan` 类型
- `web/src/state/coding-workspace-store.ts:47`（`CodingArtifactTab` 的 `"tests"` 成员）、`:504-519`（`stageToArtifactTab`）、`:511,:530,:558-562,:685,:763,:779-780`
- `web/src/pages/CodingWorkspaceArtifacts.tsx:11,:38,:114`（`tests` tab 与 `TestsPanel`）
- `web/src/pages/CodingWorkspaceControls.tsx:21-24,:37`
- `web/src/components/coding-workspace/CodingTimeline.tsx:122`
- `web/src/components/coding-workspace/StageGateEntry.tsx:9,:102-103`
- `web/src/components/coding-workspace/RoleRunHistoryPanel.tsx:185`
- `web/src/components/chat-workspace/ChatEntryContainer.tsx:32`
- `web/src/components/chat-workspace/MessageGroupView.tsx:130`
- `web/src/components/chat-workspace/entries/ProviderStreamEntry.tsx:80,:639,:653`
- `web/src/state/chat-entries.ts:22`、`coding-chat-entry-mapping.ts:43-44`、`plan-repair-session.ts:738`
- 测试文件清理：`api/coding-attempts.test.ts`、`api/types.test.ts`、`components/chat-workspace/MessageGroupView.test.tsx`、`components/chat-workspace/entries/entries.test.tsx`、`components/coding-workspace/RoleRunHistoryPanel.test.tsx`、`hooks/useCodingWorkspaceWs.test-utils.tsx`、`hooks/useCodingWorkspaceWs.test.tsx`、`pages/CodingWorkspacePage*.test.tsx`、`state/coding-workspace-store.test.ts`、`state/plan-repair-session.test.ts`

🔴 **`pnpm tsc -b` 不能作为兜底**：TS 是结构类型 + 字符串联合，switch 里删了 case 但保留 union 成员（或反过来）都不报错。必须按上面清单逐项核对。

提交建议：`refactor: 移除前端测试报告与测试计划消费`。

### 阶段九：残留确认与失效项处置（工作包 2.12–2.18）

- **2.12** 按「关键约束」的逐符号表处置 `test_executor.rs`（这是阶段三的前置，不是事后确认）。
- **2.13** 移除失去全部生产触发点的枚举成员：`CodingRoleRunTrigger::ManualRerun`、`VerificationGateResultMissing`（`types.rs:42`）、`CodingWorkspaceEngineError::TestExecutor`（`types.rs:14`）与 `TesterAgent`（`types.rs:16`）。

  🔴 **`VerificationGateResultMissing` 是跨 change 契约冲突，需按裁定执行**：它当前已无产生点（全仓仅一处定义），是 `relax-completion-testing-report-gate` 明确为「未来可能恢复 Testing 门禁保留兼容性」而留的，与本 change spec 的「不保留恢复占位」直接矛盾。**裁定以本 change 为准**——用户已明确「跳过这个阶段，这是基准」，不存在恢复路线；保留一个无产生点的错误变体只会让后来者误判。需在 `relax-completion-testing-report-gate` 的归档说明中记录该保留意图作废。

- **2.14** 处置 `latest_missing_required_steps`（`reports.rs:5-28`）：唯一调用方 `gates.rs:698` 的 quality bypass audit（ManualContinue / AcceptRisk 路径）与 testing 门禁无关，移除存储后失去数据源。显式决定移除该审计字段或改为明确空语义，**不得保留恒空而语义仍在的字段**。
- **2.18** 全仓搜索确认无 Testing 恢复用的开关、占位枚举成员、不可达分支或历史数据兼容层。

提交建议：`chore: 清理 Testing 移除后的残留引用`。

### 阶段十：验证（工作包 3.1–3.4）

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked --lib
cargo test --locked --test it_core
cargo test --locked --test it_web
cargo test --locked --test it_provider
cargo test --locked --test it_product
cargo test --locked --test it_task_run
cargo test --locked --test it_interactive
cd web && pnpm tsc -b && pnpm test
```

🔴 禁止 `-j 1`。`large_file_guard` 是既有失败基线，需与新增失败明确区分。

**`it_interactive` 必须纳入**（原 Plan 漏了）：它是独立 target，`interactive_controller.rs:46,:165`、`interactive_policy.rs:8,:54`、`interactive_projection.rs` 均引用 N17。

**预期必然失败、需同步改写的既有测试**：`tests/it_core/artifact_schema_min_fields.rs:144,:197`、`tests/it_core/phase1_profile.rs:104`、`tests/it_provider/execution_chain_fake_provider/part_01.rs:130,:226,:260`（断言节点序列 `["N16","N17","N18"]` 与 `["N16","N17","N19","N17","N18"]`）、`tests/it_task_run/*`、`tests/it_interactive/*`、`tests/it_product/product_test_executor.rs:23,:37,:287,:320`。

`openspec validate remove-testing-stage --strict`，然后代码审查。

### 阶段十一：用户验证（工作包 3.5）

经用户确认后重启后端，由用户验证阶段序列不含 Testing、group final review 不因测试证据缺失判要求修改。

## 验收对照

| 工作包 | Requirement |
|---|---|
| 1.1、1.2、2.1 | Coding Workspace 不含 Testing 阶段与 Tester 角色 |
| 1.3、1.4、2.2–2.4、2.10、2.11 | 测试计划与测试报告产物不再存在 |
| 1.5、1.6、2.5–2.7 | 恢复路径不指向已移除阶段 |
| 1.7–1.9、2.8、2.9 | 测试证据不作为评审或完成依据 |
| 1.10、2.12 | 通用测试命令规划能力保留 |
| 1.11、2.9 | 保留节点的执行链契约变化必须显式 |
| 1.12、2.14 | 非测试功能不得因移除而失去数据源 |
| 2.13、2.18 | 不保留 Testing 恢复兼容 |
| 2.15、2.16、2.17 | 移除范围完整（含字符串键与持久化契约面） |

## 非目标

- 不改 Coder、Code Reviewer、Internal Reviewer 的行为语义
- 不改 `AmendmentResumeMode` 枚举成员与 `NeedsRevalidation` 状态语义
- 不改 `plan_repair` 唤起条件
- 不改 `test_executor.rs` 的**规划能力**（`planned_test_commands_from_markdown`、`TestCommandSpec`）；执行类符号按实际消费者处置
- 不删 `mutation_test_pause.rs`（与 Testing 无关）
- 不重复处理 `remove-work-item-handoff` 归属的落点（`handoff_tests_run` / `handoff_test_result_summary`、`handoffs.rs:360`）
- 不清理已落盘的历史 `test-plans/`、`testing-reports/` 目录
- 不为历史持久化数据提供迁移或兼容层
- 不处理"进程证据当验收"的通用语义问题（属 `fix-process-evidence-as-acceptance`）

## 风险

1. 🔴 **1.5 容易写成假绿灯测试（最高风险）**：原设计断言「恢复后可继续推进、不停机」，而该性质在改动前就成立（Testing 被序号比较直接跳过）。必须断言阶段字面值，否则整个 change 的核心验收形同虚设。
2. 🔴 **「编译器兜底」不成立**：实际只有 14 处会成为编译错误。字符串键（role 解析、gate action_id 白名单、N17 协议面、`fake.rs` JSON 字面量）与 TS 联合类型都不报。覆盖完整性必须靠清单逐项核对；`cargo check` 只是辅助。
3. 🔴 **删 `coding_models/testing.rs` 会与 `test_executor.rs` 的 import 冲突**：`test_executor.rs:13-15` 就是它的消费者。必须先按逐符号表处置，否则阶段三卡住。这不是实施中再实测的事。
4. 🔴 **`mutation_test_pause.rs` 误删会破坏无关测试**：它与 Testing 无关，服务 group completion 与 provider failure 的并发恢复。
5. **保留节点的执行链契约会静默变化**：移除 `ArtifactKind::TestingReport` 与 N17 后，N18/N19/N20 的必需产物集合与 N19 的失败路由都会变（`contracts.rs:513-522`、`:664`、`:666`）。1.11 是这条的覆盖。
6. **`test_evidence_refs` 与 `fix-process-evidence-as-acceptance` 在 `render.rs` 撞车**：该字段经 `render.rs:449` 进 reviewer prompt 且是 mandatory section（`:80`），两个 change 要改同一条渲染路径。谁先做谁负责保持 section 完整。
7. **`VerificationGateResultMissing` 是跨 change 契约冲突**：`relax-completion-testing-report-gate` 为「未来恢复 Testing」保留它，与本 change spec 矛盾。已裁定以本 change 为准，需在前者归档说明中记录作废。
8. **历史 attempt 记录可能不可读**：移除枚举成员后含 `"stage":"testing"` 的记录反序列化失败。按用户决定不做兼容，本地遗留数据需重建。实施中不得因此临时加兼容分支。
9. **提示词改动是唯一新增行为**：其余均为纯删除。段落删除后 reviewer 判断依据变化，可能改变 group final review 结论倾向。需用户实际验证（3.5）。
10. **`review_parser.rs:93` 的失败模式变化**：删掉 `"testing"` 后该值会触发硬反序列化错误而非落回默认值。实际风险低（prompt 已要求 `code_review`），但需明确接受。
11. **`plan_repair_start.rs` 的白名单收窄**：`Coder | Tester` → 仅 `Coder`。生产调用方 source 恒为 `Coder`，行为不变，但这是「不改 plan repair 唤起条件」这条非目标的关键确认点。
