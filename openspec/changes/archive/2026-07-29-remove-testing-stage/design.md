## 背景

Testing 阶段处于"半死"状态：生产 pipeline 不编排它，但类型系统、存储、协议、前端和两条恢复路径仍完整承认它。

| 维度 | 现状 |
|---|---|
| 生产编排入口 | 无。`execute_testing_with_provider` 及变体只有测试调用；`runner.rs:634` 仅在 `continue 'pipeline` 分支列出 Testing |
| 实际运行行为 | attempt 落到 Testing 后被**静默跳过**（不是停机）：`runner.rs:446` 按序号比较 `order() <= CodeReview.order()` 分派，Testing(3) ≤ CodeReview(4) 成立，直接进入 Code Review |
| 阶段枚举 | `CodingExecutionStage::Testing`，`order() = 3`（`src/product/coding_models/execution.rs`） |
| 角色枚举 | `CodingProviderRole::Tester`（同文件），`coding_workspace_runner.rs:35` 映射 Testing→Tester |
| 恢复路径 | `amendment_recovery.rs:82`（Revalidate→Testing）、`gates.rs:570`（三个 testing 门禁动作→Testing） |
| 执行实现 | `testing.rs`、`testing_parser.rs`、`testing_provider.rs` + `testing_provider/`（6 文件）、`tester_agent_loop/`（9 文件） |
| 模型 | `coding_models/testing.rs`：`TestPlan`、`TestingReport` 及 10 个从属类型 |
| 存储 | `coding_attempt_store/report.rs` 六个 API、`paths.rs:209` `test_plans_root`、`testing-reports/` 目录 |
| 协议与前端 | `protocol.rs:48` `testing_report`、`state.rs:29`、前端 6 个源文件 |
| 校验 | `ArtifactKind::TestingReport` + `artifact_validate` 三处条目 |
| 执行链 | `runtime_units/coding.rs:134` N17 节点及其返修判定 |

## 根因

"阶段存在但无编排入口"留下一个**语义为真、行为为空**的合法状态值。

准确的行为描述是**静默跳过**，不是静默停机：
- `runner.rs:446` 的分派是序号比较 `current.stage.order() <= CodingExecutionStage::CodeReview.order()`，Testing(3) ≤ CodeReview(4) 成立，attempt 直接进入 Code Review。
- `code_review.rs:26` 随后把 stage 改写为 CodeReview，`valid_stage_transition`（`coding_attempt_store/attempt.rs:709`，`next.order() >= current.order()`）允许该跃迁。
- `runner.rs:634` 的 `continue 'pipeline` 同样是通路而非死路。

也就是说恢复路径可以把 attempt 送进 Testing，pipeline 会把它当作"已经过"直接带出来——不停机，但阶段语义完全失真：记录显示 attempt 经过了 Testing，实际什么都没执行。评审侧读 testing 派生字段恒空，进而被判为证据缺失。

这不改变移除的正当性（Testing 是不可达的合法状态值，用户已决定移除），但它决定了本 change 的验收方式：**回归测试必须断言阶段字面值，而不是断言"流程不停机"**——后者在改动前就已成立，写成测试不会失败，抓不住任何回归。

`relax-completion-testing-report-gate` 选择了保留基础设施、只放宽门禁。这留下了第二层缺口：评审侧仍读 testing 派生字段，恒空即被判为证据缺失。用户已明确"所有 testing report 相关的内容我都不需要，我要跳过这个阶段，这是基准"，且选择彻底移除，因此不再采用保留基础设施的路线。

`relax-completion-testing-report-gate` 选择了保留基础设施、只放宽门禁。这留下了第二层缺口：评审侧仍读 testing 派生字段，恒空即被判为证据缺失。用户已明确"所有 testing report 相关的内容我都不需要，我要跳过这个阶段，这是基准"，且选择彻底移除，因此不再采用保留基础设施的路线。

## 决策

### 决策一：移除枚举成员本身，不保留占位

替代方案是保留 `CodingExecutionStage::Testing` 但删除实现。被否决：只要枚举成员存在，恢复路径与反序列化就能产生该值，阶段语义失真的风险不消除；且编译器无法帮助定位残留引用。

**但"编译器保证覆盖完整"只对一部分落点成立，不能作为主要控制手段。** `Stage::Testing` 在 `src/` 非测试路径共 28 处，扣除整体删除的文件后剩 14 处会成为编译错误。以下落点编译器与 `pnpm tsc -b` 都不报：

- 字符串键：`coding_workspace_runner.rs:48,:80-87` 的 `"tester"|"tester_plan"|"tester_execute"` 解析与快照应用、`web/coding_ws_handler/runner.rs:162-180` 的 gate action_id 白名单、整个 N17 协议面（`protocol/contracts.rs`、`prompt_manifest.rs:95`、`cross_cutting/provider_context_builder.rs`、`runtime_units/prompt_template_registry.rs`、`task_run/*`、`web/runtime/utils.rs:37`）
- `cross_cutting/streaming_provider/fake.rs` 的 JSON 字面量
- 前端 TS 的结构类型与字符串联合：`CodingArtifactTab` 的 `"tests"` 成员、switch case 与 union 成员不同步时均不报错

因此覆盖完整性靠模块清单逐项核对，`cargo check` 只是辅助。

`order()` 序号随之收紧为：PrepareContext(0)、WorktreePrepare(1)、Coding(2)、CodeReview(3)、ReviewRequest(4)、InternalPrReview(5)、FinalConfirm(6)。序号只用于阶段先后比较，绝对值无外部契约。

### 决策二：不为历史持久化记录做兼容

用户明确按全新系统处置。既有记录中若含 `"stage":"testing"` 或 testing report 文件，移除枚举成员后反序列化会失败。**不添加迁移、不添加 `#[serde(other)]` 兜底成员、不添加忽略未知值的兼容层、不写历史数据兼容测试。**

代价是本变更前的 attempt 记录可能不可读。这是已接受的取舍。同一原则适用于 change A 的 `HandoffRevision`。

### 决策三：`AmendmentResumeMode::Revalidate` 恢复目标改为 Code Review

`Revalidate` 的语义是"实现未变，需要重新验证"。Testing 移除后，唯一仍执行验证的阶段是 Code Review。

`amendment_recovery.rs:82` 改为 `CodingExecutionStage::CodeReview`。注意 `Revalidate` 在别处还派生 `CodingExecutionUnitStatus::NeedsRevalidation` 与 `CodingUnitRunStatus::NeedsRevalidation`（`unit_run_amendment.rs:631,639`）与 `group.rs:44` 的映射——这些是 unit 状态语义，不随阶段变化，保持不变。

被否决的替代：移除 `Revalidate` 模式本身。它由 `plan_repair/engine.rs:704` 产生，语义独立于 testing，移除会超出本 change 范围。

### 决策四：testing 门禁动作整体移除，而非重定向

`gates.rs:557-579` 的 `RetryTestPlan` / `RerunMissingSteps` / `RerunTesting` 三个动作与 `AcceptTestingResult` 都只在 testing 阶段有意义。重定向到 Code Review 会造出语义错位的动作（"重试测试计划"却重跑代码审查）。因此整体移除动作、对应的 `CodingRoleRunTrigger` 成员，以及产生这些动作的门禁构造点。

Code Review 阶段的重试入口由 `RetryReview` 与 `open-code-review-triage-gate` 引入的分诊动作承担，无需补位。

### 决策五：`test_executor.rs` 按符号处置，不整体保留

原判断是"整体保留"，与决策七要删除 `coding_models/testing.rs` **直接冲突**：`test_executor.rs:13-15` 正是 import 该模块的 `TestCommand`、`TestCommandStatus`、`TestingOverallStatus`、`TestingReport`。`run_all_tests`（`:331-377`）的返回类型就是 `TestingReport`。

按实际消费者逐符号处置：

| 符号 | 生产消费者 | 处置 |
|---|---|---|
| `planned_test_commands_from_markdown`（`:81`）、`TestCommandSpec` | `coding_work_item_context.rs:378,:431` | **保留**（只返回 `Vec<TestCommandSpec>`，不依赖被删模型） |
| `run_all_tests`（`:331`） | 仅 `testing.rs:30`（被删） | 移除（它是唯一需要 `TestingReport` / `TestingOverallStatus` 的函数） |
| `remove_test_command_artifacts`（`:379`） | 仅 `testing_provider/execution_tool.rs:60,:90,:105`（被删） | 移除（保留必触发 `dead_code` 撞上 `-D warnings`） |
| `execute_test_command_with_cancellation`（`:154`） | 仅 `tester_agent_loop/executor.rs:95`（被删） | 移除 |
| `infer_test_commands`（`:60`） | 仅 `tester_agent_loop/prompts.rs:26`（被删） | 移除 |
| `discover_test_commands`（`:37`） | 无生产调用方（仅 `tests/it_product/product_test_executor.rs:23,:37`） | 移除 |

集成测试 `tests/it_product/product_test_executor.rs:287,:320` 仍在用 `run_all_tests`，需同步处置。

`TestingReport` 的存废是删除 `coding_models/testing.rs` 的前置条件，因此这一判断**不能推给实施阶段实测**，必须在动手前定。

### 决策六：`ArtifactKind::TestingReport` 与 N17 节点一并移除

`ArtifactKind::TestingReport` 服务于 `runtime_units/coding.rs` 的 task_run 执行链（N17 节点），与 Coding Workspace 的 `coding_models::TestingReport` 是两套独立结构——前者是 JSON `Value`，后者是强类型模型。

二者都实现同一个已废弃的职责，一并移除。`runtime_units/coding.rs:122` `run_worktask_execution_chain` 的循环去掉 N17 及 `testing_report_requires_current_worktask_rework` / `testing_report_has_only_out_of_scope_acceptance_failures` 两个判定函数后，循环仅由 N18 code review 的 `blocking` 驱动。

`loop` 的 `continue` 路径已确认安全，不需要改成非循环结构：`runtime_units/coding.rs:133-169` 有两个 `continue`，`:144`（N17 驱动，随移除消失）与 `:162`（N18 blocking 驱动，保留），后者由 `rework_or_hold`（`:203-241`）返回 true 时触发，即 rework 未超阈值时重跑 N18。

但另有两处必须处理：`rework_or_hold` 内 X08 步骤的 `"trigger_node": "N19"`（`:220`）语义，以及 `node_specific_fields` 的 N17 分支（`:633-641`）。

`ArtifactKind::TestingReport` 移除的横向影响需分类：
- `profile.rs:93` 与 `rules.rs:19` 是多成员 or 模式，删掉 `TestingReport` 后 `CodingReport` / `CodeReviewReport` / `IntegrationReport` 的校验行为不变（安全）。
- `canonical.rs:233-238` 的 `TESTING_REPORT_FIELDS`、`artifacts.rs:41` 的 `all_phase1()`、以及 `protocol/contracts.rs:513-522` 会连带改变 **N18/N19/N20 三个保留节点**的 required artifact 集合。
- `contracts.rs:666` 把 `N19 → N17` 列为 failure route、`:664` 把 `N17 → N19|X08` 列为 failure route——移除 N17 会改变保留节点 N19 的路由图。

此外 `runtime_units/testing.rs` 整文件（`TestingUnit`，`covered_protocol_nodes = ["N17"]`）与 `runtime_units/mod.rs:29` 同批移除。

### 决策七：评审提示词中的 testing 证据要求同批移除

group final review 与 code review 的提示词、`coding_evaluation_context` 的评估字段中含 `handoff_tests_run`、`test_result_summary` 等 testing 派生项。后端不再产生这些数据而提示词仍要求，会直接复现"证据缺失即阻塞"的失败模式。

因此提示词与评估上下文的 testing 段落必须与后端同批移除。至于"进程证据不应作为验收依据"这一更广的语义问题，由 `fix-process-evidence-as-acceptance` 负责，本 change 只移除 testing 来源项。

### 决策七之二：`VerificationGateResultMissing` 的契约冲突裁定

`coding_workspace_engine/types.rs:42` 的 `VerificationGateResultMissing` **当前已无产生点**（全仓仅此一处定义）。它是 `relax-completion-testing-report-gate` 刻意保留的，该 change 的 proposal 明确写了"保留…为未来可能恢复 Testing 门禁保留兼容性"。

这与本 change 的 spec「不保留任何为恢复 Testing 而设的占位」**直接冲突**，属契约层矛盾，不是实施细节。

裁定：以本 change 为准，移除该变体。理由是用户已明确"跳过这个阶段，这是基准"，不存在"未来恢复 Testing"的路线；保留一个无产生点的错误变体只会让后来者误判 Testing 仍可能回来。`relax-completion-testing-report-gate` 的该项保留意图随本 change 作废，需在其归档说明中记录。

### 决策七之三：会失去全部生产触发点的枚举成员一并移除

`CodingRoleRunTrigger::ManualRerun`（`coding_models/role_run.rs:24`）唯一生产产生点是 `gates.rs:566-567`（RerunTesting 分支与 `_` fallback），两者都随门禁动作移除。保留它会成为本 change 自己在 spec 中禁止的"仅为兼容保留的枚举成员"，因此一并移除。

`CodingWorkspaceEngineError::TestExecutor`（`types.rs:14`）与 `TesterAgent`（`types.rs:16`）同理，随 `tester_agent_loop` 删除一并处置。

### 决策七之四：`PlanDefectSource::Tester` 的移除会收窄一处白名单

`plan_defect.rs:19-24` 的 `PlanDefectSource::Tester`（`label()` 返回 `"tester"`）有三个生产消费者：`plan_defect_routing.rs:311`、`plan_defect.rs:56`（在 `testing_provider/report.rs`，随删）、以及 `plan_repair_start.rs:85-92`。

后者是 `start_plan_repair_from_execution_report` 的 source 白名单 `Coder | Tester`，移除后收窄为仅 `Coder`。其生产调用方是 `runner.rs:204`（coding/rework 路径，source 恒为 `Coder`），因此**行为不变**——这也是判断"移除是否改变 plan repair 唤起条件"（非目标）的关键点，需在实施时确认该结论仍成立。

### 决策七之五：`reports.rs` 的非 testing 消费者需要新数据源

`coding_workspace_engine/reports.rs:5-28` 的 `latest_missing_required_steps` 读 testing report 的 `missing_required_steps`，但**唯一调用方是 `gates.rs:698` 的 quality bypass audit**（ManualContinue / AcceptRisk 路径），与 testing 门禁无关。

移除 testing report 存储后该功能失去数据源。必须显式决定：让它返回空、还是移除该审计字段。不得留下恒空而语义仍在的字段。

### 决策八：前端与协议同批移除

`protocol.rs:48` 的 `testing_report` 经 WebSocket 暴露，被前端 store、hook 与报告页消费。后端移除而前端保留会留下永不赋值的字段与死组件，故同批移除。前端测试文件中的 testing 夹具随之清理。

## 边界

- 不改 Coder、Code Reviewer、Internal Reviewer 三个角色的行为语义。
- 不改阶段先后比较语义（`order()` 绝对值变化，相对顺序不变）。
- 不改 `AmendmentResumeMode` 枚举成员、`NeedsRevalidation` 状态语义与 `plan_repair` 的唤起条件。
- 不改 `test_executor.rs` 的**通用测试命令规划能力**（`planned_test_commands_from_markdown` 与 `TestCommandSpec`）；该模块的其余符号按实际消费者处置（见决策五）。
- 不改 `remove-work-item-handoff` 归属的落点：`coding_evaluation_context` 的 `handoff_tests_run` / `handoff_test_result_summary`、`handoffs.rs:360` 的 `list_testing_reports` 调用由该 change 处理，本 change 不重复。`list_testing_reports` 的另五个消费者（`reports.rs:11`、`reviewer_context.rs:18`、`plan_defect.rs:427`、`web/coding_ws_handler/state.rs:30`、`web/handlers/coding.rs:552`）归本 change。
- 不保留任何"未来恢复 Testing"的开关或占位。
- 不为历史持久化数据提供迁移或兼容层（见决策二）。
- 不清理已落盘的历史 `test-plans/`、`testing-reports/` 目录。
- 不处理"进程证据当验收"的通用语义问题（属 `fix-process-evidence-as-acceptance`）。
