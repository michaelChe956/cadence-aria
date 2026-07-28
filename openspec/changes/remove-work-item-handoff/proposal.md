## Why

`WorkItemHandoff`（`src/product/coding_models/plan.rs:49`）是由 provider 生成的自然语言交接摘要，与承担运行时契约职责的 `HandoffRevision`（`src/product/models/work_item_revision.rs:187`）并存。它既无价值又阻塞流程。

**它的 provider 生成路径必然失败。** `generate_work_item_handoff_from_provider`（`src/product/coding_workspace_engine/handoffs.rs:401`）要求 provider 输出 `files_changed`、`diff_summary`、`tests_run`，但 prompt 不提供任何这些信息，provider 必须自行查 git 才能回答；而 `default_compatibility_matrix` 给 claude 的 run_command 带 `--tools ""`，禁用了全部工具。实测在真实 worktree 中以相同命令与 prompt 调用，provider 返回 exit 0 但输出中不含 `<ARIA_STRUCTURED_OUTPUT>` sentinel（尝试调用工具查 git 后中断），随即被 `parse_last_structured_output` 判为 parse error。这不是偶发失败：任务要求信息，执行环境不提供获取信息的手段。

**失败被静默降级。** `handoffs.rs:337-349` 捕获该错误后仅 `tracing::warn!`，回退到 `generate_placeholder_work_item_handoff`。该路径不创建 role run、不保存 provider 原始输出、不发 chat entry、不写 timeline 节点，因此三套记录机制中均无 handoff 阶段痕迹，真实失败原因被丢弃。

**它的字段绝大部分冗余。** group final review 的 prompt 已包含完整 git diff 与 `EvaluationContextPack`（内含 `HandoffRevision` 的 `provided_contracts` / `provided_capabilities`）。`files_changed`、`diff_summary` 可由 diff 得到且更准确；`api_or_contract_changes`、`commit_sha` 由 `HandoffRevision` 以结构化形式提供且受运行时权威校验；`tests_run`、`test_result_summary` 依赖已废弃的 testing 阶段。

**用它做验收造成假阳性。** 契约交接的正确性由 `HandoffRevision` 的权威校验保证（`runtime_handoff_authority.rs` 比对 commit、revision、status），而非由一段摘要的字段完整度保证。真实案例：某 WorkItemGroup 两个 unit 的 `WorkItemHandoff` 全为占位，但 `HandoffRevision` 契约齐全、产品代码经复核正确，group final review 仍因摘要字段缺失判 request_changes。

## What Changes

- 移除 `WorkItemHandoff` 模型及其全部读写路径、存储位置与文件产物。
- 移除 provider 生成交接摘要的能力：不再为交接摘要调用 provider，也不再有占位降级路径。
- 移除 `HandoffRevision` 的 `tests` 与 `artifacts` 字段：二者当前唯一数据源是 `WorkItemHandoff` 的 `tests_run` 与 `files_changed`。`HandoffRevision` 仅保留契约与能力语义。
- 移除 lifecycle 层的交接摘要引用与 legacy 前置校验：`handoff_summary_ref`、`required_handoff_from`、`planned_handoff_summary`，以及启动 coding 时基于它们的 `work_item_handoff_missing` 校验。
- 移除 WebSocket 协议与前端对交接摘要的暴露与消费。
- 移除 `WorkItemHandoffMissing` 错误类型及其触发点。
- 不改变 `HandoffRevision` 的契约语义：`provided_contracts`、`provided_capabilities`、`contract_hash`、`commit_sha` 及其运行时权威校验保持不变。
- 不改变 schema v2 契约体系中的 `handoff_contract` 与 `handoff_field` 证据类型：它们是 `HandoffRevision` 的来源，与被移除的交接摘要无关。
- 不改变 group completion 的 unit 完成判定、commit 绑定与 `HandoffRevision` 发布路径（除不再读取交接摘要）。
- 不自动清理历史遗留的 `work-item-handoff.json` 文件。

## Capabilities

### New Capabilities

- `work-item-handoff-removal`: 交接职责单一归属 `HandoffRevision` 的语义，包括交接摘要的移除范围、契约语义的保持约束与验收依据的归属。

### Modified Capabilities

（无。现有 specs 未覆盖交接摘要与契约凭证的职责划分。）

## Impact

约 84 个文件涉及交接摘要引用，其中：

- `src/product/coding_models/plan.rs`：移除 `WorkItemHandoff` 模型。
- `src/product/coding_workspace_engine/handoffs.rs`：移除交接摘要生成、provider 调用与占位降级。
- `src/product/coding_workspace_engine/group_completion.rs`：`build_group_handoff_revision` 不再读取交接摘要；移除 `tests` / `artifacts` 组装。
- `src/product/models/work_item_revision.rs`：`HandoffRevision` 移除 `tests` 与 `artifacts` 字段。该结构持久化于 issue 级 lineage，既有记录含这两个字段，需保证反序列化不因多余字段失败。
- `src/product/coding_attempt_store/attempt.rs`、`paths.rs`：移除交接摘要的存取 API 与路径解析。
- `src/product/lifecycle_store/work_item.rs`、`src/product/models/lifecycle.rs`：移除 `handoff_summary_ref` 与相关更新入口。
- `src/product/work_item_split_engine/`：移除 legacy 的 `required_handoff_from`、`max_handoff_chars`、`max_dependency_handoffs`；保留 `handoff_contract` 与 `handoff_field`。
- `src/web/handlers/coding.rs`：移除 `work_item_handoff_missing` 前置校验。
- `src/web/coding_ws_handler/protocol.rs`、`src/web/types.rs`：移除协议字段。
- `src/web/error.rs`、`src/product/coding_workspace_engine/types.rs`：移除 `WorkItemHandoffMissing`。
- `web/src/`：移除前端类型、状态与组件对交接摘要的消费。
- 受影响的用户可见行为：不再生成交接摘要产物；group final review 不再因摘要字段缺失而误判 request_changes；启动 coding 不再因上游缺少摘要引用而被拒绝。
- 不影响跨 unit 契约交接：下游仍通过 `HandoffRevision` 获得上游契约与能力，运行时权威校验不变。

## 依赖与顺序

本 change 必须先于 `remove-testing-stage`：`HandoffRevision.tests` 的数据源是交接摘要的 `tests_run`，先移除交接摘要可使 testing 移除不再触及 lineage 数据结构。
