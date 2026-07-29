## Why

`WorkItemHandoff`（`src/product/coding_models/plan.rs:49`）是由 provider 生成的自然语言交接摘要，与承担运行时契约职责的 `HandoffRevision`（`src/product/models/work_item_revision.rs:187`）并存。它既无价值又阻塞流程。

**它的 provider 生成路径必然失败。** `generate_work_item_handoff_from_provider`（`src/product/coding_workspace_engine/handoffs.rs:401`）要求 provider 输出 `files_changed`、`diff_summary`、`tests_run`，但 prompt 不提供任何这些信息，provider 必须自行查 git 才能回答；而 `default_compatibility_matrix` 给 claude 的 run_command 带 `--tools ""`，禁用了全部工具。实测在真实 worktree 中以相同命令与 prompt 调用，provider 返回 exit 0 但输出中不含 `<ARIA_STRUCTURED_OUTPUT>` sentinel（尝试调用工具查 git 后中断），随即被 `parse_last_structured_output` 判为 parse error。这不是偶发失败：任务要求信息，执行环境不提供获取信息的手段。

**失败被静默降级。** `handoffs.rs:337-349` 捕获该错误后仅 `tracing::warn!`，回退到 `generate_placeholder_work_item_handoff`。该路径不创建 role run、不保存 provider 原始输出、不发 chat entry、不写 timeline 节点，因此三套记录机制中均无 handoff 阶段痕迹，真实失败原因被丢弃。

**它的字段绝大部分冗余。** group final review 的 prompt 已包含完整 git diff 与 `EvaluationContextPack`（内含 `HandoffRevision` 的 `provided_contracts` / `provided_capabilities`）。`files_changed`、`diff_summary` 可由 diff 得到且更准确；`api_or_contract_changes`、`commit_sha` 由 `HandoffRevision` 以结构化形式提供且受运行时权威校验；`tests_run`、`test_result_summary` 依赖已废弃的 testing 阶段。

**用它做验收造成假阳性。** 契约交接的正确性由 `HandoffRevision` 的权威校验保证（`runtime_handoff_authority.rs` 比对 commit、revision、status），而非由一段摘要的字段完整度保证。真实案例：某 WorkItemGroup 两个 unit 的 `WorkItemHandoff` 全为占位，但 `HandoffRevision` 契约齐全、产品代码经复核正确，group final review 仍因摘要字段缺失判 request_changes。

## What Changes

- 移除 `WorkItemHandoff` 模型及其全部读写路径、存储位置与文件产物。
- 移除 provider 生成交接摘要的能力：不再为交接摘要调用 provider，也不再有占位降级路径。
- 移除 `HandoffRevision` 的 `tests` 与 `artifacts` 字段：二者当前唯一**生产者**是 `WorkItemHandoff` 的 `tests_run` 与 `files_changed`。`HandoffRevision` 仅保留契约与能力语义。
- 组完成门禁的写入范围校验改用 git 事实作为 changed_files 数据源。`artifacts` 有一个生产消费者（`gates.rs:281-287` → `gates/schema_v2.rs:110-140`），legacy 路径用 `files_changed`（`gates.rs:289-305`），两者都在移除范围内；不迁移数据源会让越界写入静默放行。
- 改写 reviewer 提示词中以交接摘要为审查对象的指令（`prompts.rs:325`、`:346-353`），切换为 `HandoffRevision` 的契约与能力语义。不改写会让假阳性换形式保留，本变更的核心动机落空。
- 保留 work item 的 `completion_commit` 写入：`update_work_item_handoff_summary`（`lifecycle_store/work_item.rs:177-192`）同时写 `handoff_summary_ref` 与 `completion_commit`，后者是该函数在全仓的唯一写入点且有独立消费者，不能随函数一并删除。
- 移除 lifecycle 层的交接摘要引用与 legacy 前置校验：`handoff_summary_ref`、`required_handoff_from`、`planned_handoff_summary`，以及启动 coding 时基于它们的 `work_item_handoff_missing` 校验。
- 移除 WebSocket 协议与前端对交接摘要的暴露与消费。
- 移除 `WorkItemHandoffMissing` 的四个交接摘要触发点；该错误变体本身保留，因为另有四个触发点属 `HandoffRevision` 体系、语义必须保留。
- 不改变 schema v2 outline 层的 `handoff_notes` 与 `handoff_strategy`：名字带 handoff 但与交接摘要无关，且都是 splitter schema 的 required 项。
- 不改变 `HandoffRevision` 的契约语义：`provided_contracts`、`provided_capabilities`、`contract_hash`、`commit_sha` 及其运行时权威校验保持不变。
- 不改变 schema v2 契约体系中的 `handoff_contract` 与 `handoff_field` 证据类型：它们是 `HandoffRevision` 的来源，与被移除的交接摘要无关。
- 不改变 group completion 的 unit 完成判定、commit 绑定与 `HandoffRevision` 发布路径（除不再读取交接摘要）。
- 不自动清理历史遗留的 `work-item-handoff.json` 文件。
- 不为历史持久化数据提供迁移或兼容层：按全新系统处置。

## Capabilities

### New Capabilities

- `work-item-handoff-removal`: 交接职责单一归属 `HandoffRevision` 的语义，包括交接摘要的移除范围、契约语义的保持约束与验收依据的归属。

### Modified Capabilities

（无。现有 specs 未覆盖交接摘要与契约凭证的职责划分。）

## Impact

约 88 个文件涉及交接摘要引用（口径：`WorkItemHandoff|work_item_handoff|handoff_summary|workItemHandoff|handoffSummary|required_handoff_from|planned_handoff_summary|max_handoff_chars|max_dependency_handoffs|work-item-handoff` 在 `src/`、`tests/`、`web/src` 下的文件数），其中：

- `src/product/coding_models/plan.rs`：移除 `WorkItemHandoff` 模型。
- `src/product/coding_workspace_engine/gates.rs`、`gates/schema_v2.rs`：组完成门禁的 changed_files 改用 git 事实。
- `src/product/coding_workspace_engine/prompts.rs`：改写以交接摘要为审查对象的 reviewer 指令。
- `src/product/coding_evaluation_context/builder.rs`、`mod.rs`：移除 `handoff_tests_run` / `handoff_test_result_summary`。
- `src/product/coding_workspace_engine/lifecycle.rs`、`src/web/coding_ws_handler/runner/task.rs`：移除 `provider` 字段与 `with_provider` 构造器。
- `src/web/test_controls/plan_repair/seed.rs`、`recovery.rs`：夹具随字段移除调整（该模块不在 `#[cfg(test)]` 下，属正常编译目标）。
- `src/product/coding_workspace_engine/handoffs.rs`：移除交接摘要生成、provider 调用与占位降级。
- `src/product/coding_workspace_engine/group_completion.rs`：`build_group_handoff_revision` 不再读取交接摘要；移除 `tests` / `artifacts` 组装。
- `src/product/models/work_item_revision.rs`：`HandoffRevision` 移除 `tests` 与 `artifacts` 字段。
- `src/product/coding_attempt_store/attempt.rs`、`paths.rs`：移除交接摘要的存取 API 与路径解析。
- `src/product/lifecycle_store/work_item.rs`、`src/product/models/lifecycle.rs`：移除 `handoff_summary_ref`；保留 `completion_commit` 写入。
- `src/product/work_item_split_engine/`：移除 legacy 的 `required_handoff_from`、`max_handoff_chars`、`max_dependency_handoffs`；保留 `handoff_contract` 与 `handoff_field`。
- `src/web/handlers/coding.rs`：移除 `work_item_handoff_missing` 前置校验。
- `src/web/coding_ws_handler/protocol.rs`、`src/web/types.rs`：移除协议字段。
- `src/web/error.rs`、`src/product/coding_workspace_engine/types.rs`：移除 `WorkItemHandoffMissing`。
- `web/src/`：移除前端类型、状态与组件对交接摘要的消费。
- 受影响的用户可见行为：不再生成交接摘要产物；group final review 不再因摘要字段缺失而误判 request_changes；启动 coding 不再因上游缺少摘要引用而被拒绝。
- 不影响跨 unit 契约交接：下游仍通过 `HandoffRevision` 获得上游契约与能力，运行时权威校验不变。

## 依赖与顺序

本 change 必须先于 `remove-testing-stage`。硬依赖在 `handoffs.rs:357-368`：`generate_placeholder_work_item_handoff` 直接调用 `list_testing_reports`（`:360`）填 `tests_run` 与 `test_result_summary`。本 change 整体删除该函数，该 `list_testing_reports` 消费者随之消失；若先做 `remove-testing-stage`，它会在移除 `TestingReport` 时撞上一个 Impact 清单外的调用点。

`HandoffRevision.tests` 的数据源关系不构成顺序理由：该字段只在 `group_completion.rs:583` 写入，无论谁先做都是同一个删字段动作。

先做 `remove-testing-stage` 不会让本 change 变小：三条动机中只有「`tests_run` / `test_result_summary` 依赖 testing」会被部分吸收，provider 生成路径结构性失败与字段冗余两条与 testing 无关；反而会留下更大的摘要生成问题，并让两个 change 在 `coding_evaluation_context` 与 `reports.rs` 上重复改动。

归属划分（两个 change 均需遵守）：
- 本 change 只消除 `handoffs.rs:360` 一个 `list_testing_reports` 消费者；`reports.rs:11`、`reviewer_context.rs:18`、`plan_defect.rs:427`、`web/coding_ws_handler/state.rs:30`、`web/handlers/coding.rs:552` 五处归 `remove-testing-stage`。
- `coding_evaluation_context` 的 `handoff_tests_run` / `handoff_test_result_summary` 归本 change；`remove-testing-stage` 不重复处理。
