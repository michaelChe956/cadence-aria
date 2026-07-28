## Why

Coding Workspace 的 Testing 阶段与 Tester 角色已确定不再使用，但相关代码仍完整存在于仓库中，并以三种方式持续造成损害。

**它在生产 pipeline 中已无编排入口，却仍是可被恢复路径抵达的合法阶段值。** `execute_testing_with_provider` 及其变体没有任何生产调用点，只有测试调用；`src/web/coding_ws_handler/runner.rs:634` 的 `CodingExecutionStage::Testing` 仅出现在 `continue 'pipeline` 分支中。但两条恢复路径仍会把 attempt 置到该阶段：`src/product/coding_attempt_store/amendment_recovery.rs:82` 在 `AmendmentResumeMode::Revalidate` 下置为 Testing，`src/product/coding_workspace_engine/gates.rs:570` 的 `RetryTestPlan` / `RerunMissingSteps` / `RerunTesting` 三个门禁动作也置为 Testing。

后果是**静默跳过**，不是静默停机：`runner.rs:446` 的分派按序号比较 `order() <= CodeReview.order()`，Testing(3) ≤ CodeReview(4) 成立，attempt 直接进入 Code Review；`code_review.rs:26` 再把 stage 改写为 CodeReview，`coding_attempt_store/attempt.rs:709` 的 `next.order() >= current.order()` 允许该跃迁。也就是说记录显示 attempt 经过了 Testing，实际什么都没执行——阶段语义完全失真，而没有任何错误暴露出来。

这决定了本变更的验收方式：回归测试必须断言阶段字面值，而非断言「流程不停机」。后者在变更前已成立，写成测试不会失败。

**它让"测试证据缺失"变成阻塞理由。** group final review 与完成门禁读取 testing report 派生的字段（`handoff_tests_run`、`test_evidence_refs`、`tests_run`）；testing 阶段不运行，这些字段恒空，reviewer 因此判要求修改。`relax-completion-testing-report-gate` 已放宽完成门禁对 testing report 的强制要求，但保留了全部 testing 基础设施，评审侧的证据缺口仍在。

**它持续消耗维护成本。** `src/product/tester_agent_loop/`（9 个文件）、`src/product/coding_workspace_engine/testing_provider/`（6 个文件）、`testing.rs`、`testing_parser.rs`、`src/product/coding_models/testing.rs` 及配套存储、协议、前端组件全部需要随重构同步修改，却不产出任何被消费的结果。

## What Changes

- 移除 `CodingExecutionStage::Testing` 阶段与 `CodingProviderRole::Tester` 角色，以及其阶段-角色映射。
- 移除 Testing 阶段的全部执行实现：`testing.rs`、`testing_provider.rs` 与 `testing_provider/` 目录、`testing_parser.rs`、`src/product/tester_agent_loop/`。
- 移除 `TestPlan`、`TestingReport` 及其从属模型、存储读写 API、落盘目录（`test-plans/`、`testing-reports/`）与 artifact 校验条目。
- 移除 testing 相关门禁动作与触发原因：`RetryTestPlan`、`RerunMissingSteps`、`RerunTesting`、`AcceptTestingResult` 门禁动作，以及 `CodingRoleRunTrigger` 中的 `RetryTestPlan`、`RerunMissingSteps`。
- 移除 Tester 作为 plan defect 来源：`PlanDefectSource::Tester`。
- 改写两条恢复路径，使其不再指向被移除的阶段：`AmendmentResumeMode::Revalidate` 恢复目标改为 Code Review；`RetryReview` 之外的 testing 门禁动作随动作一并移除。
- 移除 WebSocket 协议中的 testing report 字段与前端对 testing report、test plan 的类型、状态与组件消费。
- 移除评审与完成语义中对 testing 证据的读取：testing 派生的 `test_evidence_refs`，以及 reviewer 提示词中要求 testing 证据的段落。（`handoff_tests_run` / `test_result_summary` 归 `remove-work-item-handoff`，本变更不重复处理。）
- 按实际消费者逐符号处置 `test_executor.rs`：保留 work item 上下文侧的测试命令规划能力，移除失去生产消费者的执行函数。整体保留会与移除 `coding_models/testing.rs` 冲突——该模块正是 `test_executor.rs` 的 import 来源。
- 移除失去全部生产触发点的枚举成员：`CodingRoleRunTrigger::ManualRerun`、`VerificationGateResultMissing`、`CodingWorkspaceEngineError::TestExecutor` 与 `TesterAgent`。其中 `VerificationGateResultMissing` 是 `relax-completion-testing-report-gate` 为「未来恢复 Testing」刻意保留的，与本变更的「不保留恢复占位」冲突，以本变更为准。
- 显式重写保留节点的执行链契约：移除 N17 后，`protocol/contracts.rs:664,:666` 指向/来自 N17 的失败路由与 `:513-522` 的 N18/N19/N20 必需产物集合都会变化，不得静默改变。
- 显式处置 `latest_missing_required_steps`：它读 testing report 但唯一调用方是 quality bypass audit（与 testing 门禁无关），移除存储后失去数据源。
- **不移除 `mutation_test_pause.rs`**：其测试点与 Testing 无关，服务于 group completion 与 provider failure 的并发恢复。
- 移除 `ArtifactKind::TestingReport` 及 task_run 执行链中的 N17 testing report 节点。
- 保留 `src/product/test_executor.rs` 中被 `coding_work_item_context.rs` 消费的测试命令规划能力（`planned_test_commands_from_markdown`、`TestCommandSpec`）。
- 不保留任何"未来恢复 Testing"的兼容开关、占位枚举成员或死代码。
- 不为历史持久化记录提供兼容层：按全新系统处置，既有含 `testing` 阶段值或 testing report 的记录不做迁移。
- 不改变 Coder、Code Reviewer、Internal Reviewer 三个角色的行为与阶段顺序语义（除阶段序号因移除而收紧）。

## Capabilities

### New Capabilities

- `testing-stage-removal`: Coding Workspace 不含 Testing 阶段与 Tester 角色的语义，包括移除范围、恢复路径的重定向约束、测试证据不作为评审与完成依据的归属，以及通用测试命令执行能力的保留边界。

### Modified Capabilities

（无。现有 specs 未覆盖 Coding Workspace 的阶段编排与 Testing 语义。）

## Impact

约 166 个文件涉及 testing 引用（口径：`Stage::Testing|Role::Tester|TestingReport|TestPlan|testing_report|test_plan|tester` 在 `src/` 非测试路径 72 个、含 `src/` 内联测试与 `tests/` 共 137 个、`web/src/` 29 个），其中：

- `src/product/coding_models/execution.rs`：移除 `CodingExecutionStage::Testing` 与 `CodingProviderRole::Tester`，`order()` 序号相应收紧。
- `src/product/coding_models/testing.rs`：整文件移除（`TestPlan`、`TestingReport` 及从属类型）。
- `src/product/coding_models/gate.rs`、`role_run.rs`：移除 testing 门禁动作与触发原因。
- `src/product/coding_workspace_engine/`：移除 `testing.rs`、`testing_parser.rs`、`testing_provider.rs`、`testing_provider/`；调整 `gates.rs`、`timeline.rs`、`review_parser.rs`、`plan_defect_routing.rs`、`prompts.rs`、`reports.rs`、`reviewer_context.rs`、`plan_defect.rs`、`mod.rs`。**`mutation_test_pause.rs` 不移除**（其测试点与 Testing 无关）。
- `src/product/coding_models/provider_config.rs`：移除 tester 权限模式、provider 快照字段与访问器（该 struct 带 `#[serde(deny_unknown_fields)]`，属持久化契约面）。
- `src/product/coding_evaluation_context/tester_execution.rs`：整文件移除；`mod.rs` 移除 `TesterExecutionContextPack` 系列类型与 `EvaluationContextRole::Tester`。
- `src/runtime_units/testing.rs`：整文件移除（`TestingUnit`，`covered_protocol_nodes = ["N17"]`）。
- `src/product/test_executor.rs`：按实际消费者逐符号处置（保留规划能力，移除失去消费者的执行函数）。
- `src/web/handlers/coding.rs`、`src/web/types.rs`：移除 REST 快照的 `testing_report` 字段。
- 字符串键落点（编译器不报错，须逐项核对）：`coding_workspace_runner.rs` 的 role 解析、`web/coding_ws_handler/runner.rs:162-180` 的 gate action_id 白名单、`cross_cutting/streaming_provider/fake.rs` 的 JSON 字面量、N17 协议面的多处字符串匹配。
- `src/product/tester_agent_loop/`：整目录移除。
- `src/product/coding_attempt_store/`：移除 `report.rs` 中 test plan / testing report 存取 API、`paths.rs` 的 `test_plans_root`、`utils.rs` 的阶段字符串映射；`amendment_recovery.rs` 恢复目标改为 Code Review。
- `src/product/coding_workspace_runner.rs`：移除 Testing→Tester 阶段角色映射。
- `src/product/coding_evaluation_context/`：移除 testing 派生的评估字段。
- `src/protocol/artifacts.rs`、`src/protocol/contracts.rs`：移除 `ArtifactKind::TestingReport` 与 testing 相关契约类型。
- `src/cross_cutting/artifact_validate/`：移除 TestingReport 的 canonical 字段、profile 与 rules 条目。
- `src/runtime_units/coding.rs`：移除 N17 节点与 testing report 驱动的返修判定。
- `src/web/coding_ws_handler/`：移除 `protocol.rs:48` 的 `testing_report` 字段、`state.rs` 的读取、`runner.rs` 与 `socket.rs` 的阶段分支。
- `src/web/handlers/dto.rs`、`src/web/test_controls/fixtures.rs`：移除阶段序列化与夹具阶段。
- `web/src/`：移除 `api/types/coding.ts`、`state/coding-workspace-store.ts`、`hooks/useCodingWorkspaceWs.ts`、`pages/CodingWorkspaceReports.tsx` 中的 testing report / test plan 类型、状态与展示。
- 受影响的用户可见行为：阶段列表与时间线不再出现 Testing；门禁不再提供 testing 相关动作；报告页不再有 testing report 分区；恢复重校验直接回到 Code Review；group final review 不再因测试证据缺失判要求修改。
- 不影响 Coder 阶段自行运行测试命令：`test_executor` 保留。

## 依赖与顺序

本 change 必须晚于 `remove-work-item-handoff`。硬依赖在 `handoffs.rs:357-368`：`generate_placeholder_work_item_handoff` 在 `:360` 调用 `list_testing_reports`，前者整体删除该函数使这个消费者消失。若本 change 先做，会在移除 `TestingReport` 时撞上一个 Impact 清单外的调用点。

`HandoffRevision.tests` 的数据源关系不构成顺序理由：该字段只在 `group_completion.rs:583` 写入，无论谁先做都是同一个删字段动作。

归属划分（两个 change 均需遵守）：
- `list_testing_reports` 的六个消费者：`handoffs.rs:360` 归 `remove-work-item-handoff`；`reports.rs:11`、`reviewer_context.rs:18`、`plan_defect.rs:427`、`web/coding_ws_handler/state.rs:30`、`web/handlers/coding.rs:552` 归本 change。
- `coding_evaluation_context` 的 `handoff_tests_run` / `handoff_test_result_summary` 归 `remove-work-item-handoff`；本 change 不重复处理。

`relax-completion-testing-report-gate` 已放宽完成门禁对 testing report 的强制要求。本 change 在其基础上移除基础设施本身；若该 change 尚未归档，两者对完成门禁的改动需要在实施时对齐，不得回退其放宽结论。
