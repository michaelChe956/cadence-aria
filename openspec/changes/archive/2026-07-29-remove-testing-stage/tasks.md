## 1. 移除后行为回归测试

- [x] 1.1 为 attempt 全流程编写测试，断言阶段序列不含 Testing、不创建 Tester role run、不为测试产物调用 provider。
- [x] 1.2 为阶段先后比较编写测试，断言移除后相对顺序与移除前一致。
- [x] 1.3 为 attempt 完成编写测试，断言 attempt 目录下不出现测试计划与测试报告目录。
- [x] 1.4 为会话状态编写测试，断言不再暴露测试报告字段。
- [x] 1.5 为计划修订重校验恢复编写测试，断言 attempt 阶段**字面值恰为 Code Review**。MUST NOT 断言「可继续推进、不停机」——该性质在移除前已成立（阶段按序号比较分派，Testing 会被直接跳过），断言它不会失败，抓不住回归。
- [x] 1.6 为门禁动作集合编写测试，断言不含重试测试计划、重跑缺失步骤、重跑测试、接受测试结果；并断言 Code Review 重试与分诊动作仍可用。
- [x] 1.7 为 group final review 编写测试，断言无测试报告时不因测试证据缺失判要求修改或阻塞。
- [x] 1.8 为评估上下文编写测试，断言不含测试执行清单与测试结论字段。
- [x] 1.9 为完成判定编写测试，断言 Coding 与 Code Review 通过且其他非测试门禁满足时可完成。
- [x] 1.10 为 `planned_test_commands_from_markdown` 与 `TestCommandSpec` 编写回归测试，断言 work item 上下文侧的测试命令规划行为与移除前一致。
- [x] 1.11 为保留节点的执行链契约编写测试：断言各保留节点（N18/N19/N20）的必需产物集合变化符合预期，且不存在指向已移除节点的失败路由。
- [x] 1.12 为质量绕过审计编写测试，断言其原先依赖测试报告的字段已被移除或改为明确空语义，不存在恒空而语义仍在的字段。

## 2. 生产实现

- [x] 2.1 移除 `CodingExecutionStage::Testing` 与 `CodingProviderRole::Tester`，收紧阶段序号并移除阶段-角色映射。
- [x] 2.2 移除 Testing 阶段执行实现：`testing.rs`、`testing_parser.rs`、`testing_provider.rs` 与 `testing_provider/`（含 `test_pause.rs`）、`tester_agent_loop/`、`coding_evaluation_context/tester_execution.rs`、`runtime_units/testing.rs`。🔴 **`mutation_test_pause.rs` 不在移除范围**：其 `CodingMutationTestPoint` 只有 `GroupCompletionRunning`/`GroupCompletionCompletedRetry`/`ProviderFailure` 三个点，与 Testing 无关，生产调用方是 `group_completion.rs:52-56` 与 `provider_failure.rs:108-110`，删除会破坏 group completion 与 provider failure 的并发恢复测试。
- [x] 2.3 移除 `TestPlan`、`TestingReport` 及从属模型（`coding_models/testing.rs`）。前置：先按 2.12 处置 `test_executor.rs` 对该模块的 import（`test_executor.rs:13-15`），否则本项无法编译通过。
- [x] 2.4 移除测试计划与测试报告的存储读写 API、落盘路径解析与阶段字符串映射。
- [x] 2.5 移除 testing 门禁动作与触发原因，以及产生这些动作的门禁构造点（`testing_parser.rs:286-323`、`:379-412`）。同步移除 `web/coding_ws_handler/runner.rs:162-180` 的 gate action_id 字符串白名单中的四个 testing 项（**字符串键，编译器不报错**）。
- [x] 2.6 移除 `PlanDefectSource::Tester` 及其阶段路由（`plan_defect.rs:19-24`、`plan_defect_routing.rs:311`）；确认 `plan_repair_start.rs:85-92` 的 source 白名单收窄为仅 `Coder` 后行为不变（其生产调用方 `runner.rs:204` 的 source 恒为 `Coder`）。
- [x] 2.7 将计划修订重校验的恢复目标改为 Code Review；确认 `NeedsRevalidation` 状态语义与 `plan_repair` 唤起条件未变。同时在 design 中裁定或在 1.5 中断言：恢复后「attempt stage=CodeReview」与「unit status=NeedsRevalidation」并存的组合可推进（该组合在移除前不存在）。
- [x] 2.8 移除评估上下文与评审提示词中的测试派生字段与测试证据要求，含 `coding_evaluation_context/methods.rs:8,:34`、`mod.rs:18,:27,:97-125`、`reviewer_context.rs:16-23,:42` 与 `plan_defect.rs:425-437` 两处 `test_evidence_refs` 生产者、`work_item_projection/execution_context.rs:16` 的字段本身。**注意 `handoff_tests_run` / `handoff_test_result_summary` 归 `remove-work-item-handoff`，本项不重复处理。**
- [x] 2.9 移除 `ArtifactKind::TestingReport`、artifact 校验条目（`canonical.rs:233-238` 的 `TESTING_REPORT_FIELDS`、`artifacts.rs:41` 的 `all_phase1()`、`profile.rs:93`、`rules.rs:19`），以及执行链的 N17 节点与测试报告驱动的返修判定。循环结构已确认安全（`runtime_units/coding.rs:162` 的 N18 `continue` 保留），但需处理 `rework_or_hold` 的 X08 `"trigger_node": "N19"`（`:220`）与 `node_specific_fields` 的 N17 分支（`:633-641`）；并显式重写 `protocol/contracts.rs:664,:666` 指向/来自 N17 的失败路由与 `:513-522` 的 N18/N19/N20 必需产物集合。
- [x] 2.10 移除 WebSocket 协议的测试报告字段、其状态读取与阶段分支，以及阶段序列化与测试夹具中的 Testing。含 REST 侧 `web/handlers/coding.rs:551-555` 与 `web/types.rs:550` 的 `CodingAttemptSnapshotResponse.testing_report`。
- [x] 2.11 移除前端测试报告与测试计划的类型、状态、hook 消费与报告页分区。落点约 20 个文件，含 `CodingWorkspaceArtifacts.tsx`、`coding-workspace-store.ts`（`CodingArtifactTab` 的 `"tests"` 成员与 `stageToArtifactTab`）、`CodingTimeline.tsx`、`StageGateEntry.tsx`、`ChatEntryContainer.tsx`、`MessageGroupView.tsx`、`ProviderStreamEntry.tsx`、`RoleRunHistoryPanel.tsx`、`chat-entries.ts`、`coding-chat-entry-mapping.ts`、`plan-repair-session.ts`、`CodingWorkspaceControls.tsx`、`api/types/coding.ts`。**TS 是结构类型 + 字符串联合，`pnpm tsc -b` 对多数不同步情形不报错，须按清单逐项核对。**
- [x] 2.12 按决策五逐符号处置 `test_executor.rs`：保留 `planned_test_commands_from_markdown` 与 `TestCommandSpec`；移除 `run_all_tests`、`remove_test_command_artifacts`、`execute_test_command_with_cancellation`、`infer_test_commands`、`discover_test_commands` 及对 `coding_models::testing` 的 import；同步处置 `tests/it_product/product_test_executor.rs`。
- [x] 2.13 处置失去全部生产触发点的枚举成员：移除 `CodingRoleRunTrigger::ManualRerun`（`role_run.rs:24`）、`VerificationGateResultMissing`（`types.rs:42`，见决策七之二的契约裁定）、`CodingWorkspaceEngineError::TestExecutor`（`types.rs:14`）与 `TesterAgent`（`types.rs:16`）。
- [x] 2.14 处置 `latest_missing_required_steps`（`reports.rs:5-28`）：其唯一调用方 `gates.rs:698` 的 quality bypass audit 与 testing 门禁无关，移除存储后失去数据源。显式决定移除该审计字段或改为明确空语义，不得保留恒空而语义仍在的字段。
- [x] 2.15 移除 `coding_models/provider_config.rs` 的 tester 配置面：`CodingRolePermissionModes.tester`（:20,:29）、`CodingRoleProviderConfigSnapshot.tester_plan`/`tester_execute`（:52-53,:75-76）、`tester_plan_provider`/`tester_execute_provider`/`set_*`（:129-142）、`Display for CodingProviderRole` 的 `Tester` 分支（:40）。该 struct 带 `#[serde(deny_unknown_fields)]`，属持久化契约面。
- [x] 2.16 移除字符串键落点：`coding_workspace_runner.rs:48,:80-87` 的 `"tester"|"tester_plan"|"tester_execute"` 解析与快照应用、`coding_attempt_store/unit_run.rs:261` 的 Tester identity_mismatch 分支、`cross_cutting/streaming_provider/fake.rs` 的 JSON 字面量、N17 协议面的字符串匹配（`protocol/nodes.rs:18`、`prompt_manifest.rs:95`、`cross_cutting/provider_context_builder.rs:202,:207,:313`、`runtime_units/prompt_template_registry.rs:17,:183,:221`、`task_run/interactive_runner.rs:52`、`task_run/step_runner.rs:141`、`web/runtime/utils.rs:37`）。
- [x] 2.17 处置 `review_parser.rs:93` 的 `source_stage` 解析：移除 `"testing"` 后 provider 输出该值会变成**硬反序列化错误**（`unknown_variant`，`:100-113`），而非落回默认值——`:200` 的 `unwrap_or(default_source_stage)` 只覆盖字段缺失。需明确该失败模式是否可接受（reviewer prompt 已要求 `source_stage=code_review`，实际风险低）。
- [x] 2.18 确认未保留任何 Testing 恢复用的开关、占位枚举成员、不可达分支或历史数据兼容层。

## 3. 验证与交付

- [x] 3.1 运行本 change 相关定向测试与 coding workspace、attempt store、plan repair、evaluation context、runtime unit 既有回归，并区分既有失败基线。
- [x] 3.2 运行 `cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings` 与各测试目标（`--lib`、`it_core`、`it_web`、`it_provider`、`it_product`、`it_task_run`、**`it_interactive`**）。`it_interactive` 是独立 target，`interactive_controller.rs:46,:165`、`interactive_policy.rs:8,:54`、`interactive_projection.rs` 均引用 N17，必须纳入。
- [x] 3.3 运行前端检查与测试：`cd web && pnpm tsc -b`、`cd web && pnpm test`。
- [x] 3.4 严格校验 OpenSpec change 并完成代码审查。
- [ ] 3.5 经用户确认后重启后端，由用户验证阶段序列不含 Testing 且 group final review 不因测试证据缺失判要求修改。
