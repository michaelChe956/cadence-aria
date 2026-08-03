# Task 6 回归验证报告

- Change：`add-pi-provider`
- 工作包：6.1 / 6.2 / 6.3
- 验证日期：2026-06-30
- 结论：通过。新增边界断言、修复两项因合法默认值变更而失效的 Supervised 集成测试前置条件，并消除了本分支引入的 Clippy warning；未改变产品行为。

## 覆盖矩阵

| Brief 要求 | 覆盖测试 | 状态 | 说明 |
|---|---|---|---|
| Pi 健康检查（`pi --version`） | `cross_cutting::provider_health::tests::pi_version_command_uses_pi_binary` | 既有 | Task 1 已覆盖。 |
| Pi 目录展示 / 状态 API | `web::handlers::providers::tests::providers_status_includes_pi_when_available`、`web::provider_availability::tests::parse_provider_name_accepts_pi`、`web::provider_availability::tests::provider_name_key_pi` | 既有 | Task 1 已覆盖。 |
| Pi RPC 文本流、工具事件、完成和错误映射 | `cross_cutting::pi_provider::tests::{parse_text_delta_from_message_update, parse_tool_execution_events, parse_agent_settled_as_terminal, session_sends_prompt_and_emits_text_until_settled, session_failure_is_terminal_no_retry, session_demultiplexes_response_by_id}` | 既有 | Task 2 的 15 项 `pi_provider` 测试覆盖。 |
| Pi 取消：`abort` 后会话终止、无完成事件 | `cross_cutting::pi_provider::tests::{session_aborts_on_provider_command_abort, session_suppresses_output_after_abort, session_aborts_when_token_cancels_during_get_state_handshake, session_aborts_when_abort_command_and_token_race_during_prompt_handshake}` | 既有 | Task 2 已覆盖。 |
| Pi 恢复：`--session-id` | `cross_cutting::pi_provider::tests::{build_args_resume_includes_session_id, session_resumes_with_existing_session_id}` | 既有 | Task 2 已覆盖。 |
| Pi Auto 运行、审计事件 | `cross_cutting::pi_provider::tests::build_args_rpc_mode_auto_only`、`product::workspace_engine::tests::author_run_with_pi_uses_pi_provider_in_auto_mode` | 既有 | Task 2/3 已覆盖；前者锁定无 Aria 授权扩展，后者验证 Workspace 实际输入为 Auto。 |
| Story / Design / Work Item 三入口的 Provider 选择 | `product::workspace_engine::tests::{author_run_with_pi_uses_pi_provider_in_auto_mode, initial_author_inputs_directly_route_every_workspace_artifact_type}` | 既有 | 共享 `workspace_engine` 覆盖三种入口。 |
| Workspace 权限持久化和默认 Auto | `product::models::tests::{workspace_role_permission_modes_default_is_auto, old_workspace_session_record_without_permission_modes_deserializes_to_auto}`、`product::workspace_engine::tests::start_generation_locks_selected_modes_into_store` | 既有 | Task 3 已覆盖。 |
| Claude/Codex Supervised 保留且授权链路可用 | `workspace_ws_integration::workspace_ws_supervised_permission_allows_real_stream_to_complete` | 更新 | 测试现在显式持久化 Author = `Supervised`，断言收到 `Bash` permission request，批准后完成。 |
| Supervised 下错误 permission id 仍被协议层拒绝 | `workspace_ws_integration::workspace_ws_unmatched_permission_response_returns_protocol_error` | 更新 | 显式 Supervised 后断言 `PERMISSION_ID_UNMATCHED`，再批准正确 id 并完成。 |
| Pi Supervised 服务端归一为 Auto | `product::workspace_engine::tests::start_generation_normalizes_pi_role_to_auto_and_keeps_disabled_reviewer_mode`、`web::workspace_ws_handler::tests::provider_select_then_user_message_forces_pi_to_auto_from_stale_supervised_mode`、`product_coding_workspace_engine::pi_role_with_supervised_mode_normalized_to_auto` | 既有 | Task 3/4/5 已覆盖。 |
| Workspace 启动失败 fail-fast、无替代 Provider | `product::workspace_engine::tests::pi_start_failure_does_not_retry_selected_provider`、`web::workspace_ws_handler::tests::pi_failures_do_not_start_registered_alternate_provider` | 既有 | Task 3 已覆盖。 |
| Coding Provider / 权限 / fail-fast | `product_coding_workspace_engine::{pi_role_with_supervised_mode_normalized_to_auto, pi_failure_does_not_trigger_fresh_retry}`、`web::coding_ws_handler::tests::coding_pi_start_failure_does_not_start_registered_alternate_provider`、`product::coding_workspace_engine::tests::coder_rework_normalizes_pi_supervised_mode_to_auto` | 既有 | Task 4 已覆盖。 |
| 前端 Pi Auto-only 两面板及状态可见性 | `ProviderConfigPanel` 与 `CodingProviderConfigPanel` 的 Pi Auto-only 测试，`coding_ws_permission_mode_select_normalizes_pi_to_auto`，`providerConfigFor` Pi 保留 | 既有 | Task 5 已覆盖。 |
| 仓库初始化 UI 不展示 Pi | `CreateRepositoryDialog > 仓库初始化不显示 Pi 即使 Pi 可用` | 既有 | Task 1 已覆盖。 |
| 仓库初始化后端执行路径不会输入 Pi | `product::repository_store::initializer::tests::repository_initializer_runs_four_independent_claude_turns_in_strict_order` | 更新 | 新增遍历全部四个实际 `StreamingProviderInput` 的断言，仅允许 `ProviderType::ClaudeCode`。 |
| Task Runner HTTP 入口拒绝 `pi` | `web::provider_availability::tests::parse_provider_type_still_rejects_pi` | 既有 | `web_runtime_provider_type` 错误文本含 `pi`。 |
| Task Runner router 拒绝 Pi 且不调用 adapter | `task_run::provider_factory::tests::routing_provider_rejects_pi_without_calling_real_providers` | 既有 | Task 1 已覆盖。 |
| 兼容性矩阵不包含 Pi | `cli_adapter_baseline::default_matrix_contains_claude_code_and_codex_cli_entries` | 更新 | 新增 `default_compatibility_matrix().entry_for(ProviderType::Pi).is_none()`。 |
| 静态节点契约不产生 Pi | `context_builder::contract_workflow_and_prompt_registries_cover_p4_execution_and_closure_nodes` | 更新 | 新增对实际 `phase1_node_contract_table()` 返回数据的遍历断言：所有 `provider_type != Some(ProviderType::Pi)`；不是源码文本扫描。 |
| Task 1 新增测试的 Clippy 布局质量 | `product::work_item_split_engine::types::tests::provider_name_to_type_maps_pi` + `cargo clippy -p cadence-aria --all-targets` | 更新 | 仅移动已有 test module 到文件末尾，消除 `items_after_test_module`，测试逻辑不变。 |

## 新增 / 更新测试清单与理由

1. `tests/it_core/cli_adapter_baseline.rs`
   - 在既有默认矩阵测试中新增 Pi 查询为 `None`。
   - 理由：防止 Pi 被误加入 Task Runner 的 CLI compatibility matrix。
2. `tests/it_core/context_builder.rs`
   - 对 `phase1_node_contract_table()` 的实际行集合新增全量遍历断言。
   - 理由：锁定静态 Task Runner node contract 数据不能调度 Pi。
3. `src/product/repository_store/initializer/tests.rs`
   - 对真实仓库初始化的四个 `StreamingProviderInput` 新增全量断言，只能是 Claude Code。
   - 理由：前端下拉过滤不足以证明后端执行路径未扩张。
4. `tests/it_core/workspace_ws_integration/{part_03.rs,part_04.rs,part_05.rs}`
   - 为两个 Supervised 授权集成测试通过 `LifecycleStore::update_workspace_session_permission_modes` 显式持久化 Author = `Supervised`。
   - 理由：Task 3 合法地将默认模式改为 Auto；测试意图是验证 Claude Code 的 Supervised 授权链路，不能再隐式依赖旧默认值。两个测试仍分别验证 permission request → 批准 → 完成，及错误 id → `PERMISSION_ID_UNMATCHED` → 正确批准 → 完成。
5. `src/product/work_item_split_engine/types.rs`
   - 将 Task 1 已有 `provider_name_to_type_maps_pi` test module 移至文件末尾。
   - 理由：消除本分支引入的 `clippy::items_after_test_module` warning；无生产逻辑或测试逻辑变更。

## 边界验证结果

### 1. 兼容性矩阵无 Pi 条目：通过

- 新增运行时数据断言：`default_compatibility_matrix().entry_for(ProviderType::Pi).is_none()`。
- 定向命令：`cargo test -p cadence-aria --test it_core default_matrix_contains_claude_code_and_codex_cli_entries`。
- 结果：1 passed，0 failed。

### 2. 静态节点契约不产生 Pi：通过

- 实际静态集合为 `phase1_node_contract_table()`；其 `Phase1NodeContractRow.provider_type` 是真实结构化数据。
- 新增运行时遍历：`rows.iter().all(|row| row.provider_type != Some(ProviderType::Pi))`。
- 定向命令：`cargo test -p cadence-aria --test it_core contract_workflow_and_prompt_registries_cover_p4_execution_and_closure_nodes`。
- 结果：1 passed，0 failed。

### 3. 仓库初始化执行路径不能接收 Pi：通过

- 前端：既有 `CreateRepositoryDialog` 测试在 Pi 可用时仍断言无 Pi option；定向运行 12 passed。
- 后端：`ClaudeRepositoryInitializer::initialize` 的既有真实四回合测试现在遍历每一个构造的 `StreamingProviderInput`，断言全部为 `ProviderType::ClaudeCode`；同时既有逐输入断言保留。
- 定向命令：`cargo test -p cadence-aria --lib repository_initializer_runs_four_independent_claude_turns_in_strict_order`。
- 结果：1 passed，0 failed。

## 质量门禁实际输出

| 命令 | 结果 | 实际输出 / 计数 |
|---|---|---|
| `cargo test -p cadence-aria` | 通过 | lib 1450 passed；bin 0；`it_core` 147 passed；`it_interactive` 43 passed；`it_product` 198 passed；`it_provider` 54 passed；`it_task_run` 31 passed；`it_web` 312 passed、12 ignored；doc-test 1 passed。总计 2236 passed、12 ignored、0 failed。 |
| `cargo clippy -p cadence-aria --all-targets` | 通过 | `Finished dev profile`；0 warnings。 |
| `cargo fmt --check` | 通过 | 无输出，退出码 0。 |
| `cd web && npm test && npm run build` | 通过 | Vitest：91 files / 732 tests passed；build：1791 modules transformed，成功构建。Vite 输出一个非失败的 >500 kB chunk-size 建议 warning（主 JS 773.37 kB / gzip 213.13 kB）。 |

所有 Rust 命令均未使用 `-j 1`。

## 与基线的差异

- Rust lib：基线 1450 → 最终 1450，计数不变（本任务主要在既有测试中补断言/修正前置条件）。
- 前端：基线 91 files / 732 tests → 最终 91 files / 732 tests，计数不变。
- 在首次全量 `cargo test -p cadence-aria` 中，`it_core` 有 2 项失败；原因是 Task 3 将默认权限合法改为 Auto 后，两项旧 Supervised 测试仍隐式依赖旧默认值。已以显式持久化 Supervised 修正，最终全量通过。
- 在首次 Clippy 中有 1 项 warning：`src/product/work_item_split_engine/types.rs` 的 `items_after_test_module`。经与 merge-base `735448bc` 对比，test module 是本分支 Task 1 新增，故为本分支引入；已移动到文件末尾，最终 Clippy 为 0 warnings。
- 前端构建的 >500 kB chunk-size 提示在最终构建仍出现；它不导致命令失败，且本任务没有修改前端 bundle 架构。

## 发现的真实缺陷

1. **已修复的测试回归（非产品缺陷）**：Task 3 的 Auto 默认值变更使两条 Supervised WebSocket 集成测试不再进入授权请求路径。产品默认 Auto 符合批准的 OpenSpec；测试改为显式持久化 Supervised 后，仍验证真实授权请求与批准/错误响应链路。
2. **已修复的质量问题**：Task 1 在 `work_item_split_engine/types.rs` 将 `#[cfg(test)] mod tests` 放在三个生产函数前，触发 Clippy warning。仅移动模块至文件末尾，最终无 warning。
3. 未发现未修复的产品行为缺陷。

## Concerns

- Vite 构建成功但仍报告主 chunk 大于 500 kB 的性能建议；与 Pi 回归范围无关，未扩展范围处理。
- 最终工作树无暂存文件；所有 6.1/6.2/6.3 工作包已在 `openspec/changes/add-pi-provider/tasks.md` 勾选。

---

# Task 6 第 1 轮修复报告

- 修复目标：关闭评审指出的 6.1 / 6.2 三个真实覆盖缺口；仅新增/加强测试与本报告，不改变生产行为。
- 结论：通过。新增 3 组测试（2 个 Rust unit/session 覆盖组、1 个 WebSocket 集成覆盖组），三项均先在隔离的 `d0378419` 基线 worktree 中以定向变异得到 RED，再在本分支得到 GREEN。

## 更正后的诚实覆盖矩阵

| Brief 要求 | 实际覆盖测试 | 状态 | 说明 |
|---|---|---|---|
| Pi 健康检查（`pi --version`） | `cross_cutting::provider_health::tests::pi_version_command_uses_pi_binary` | 既有 | Task 1 覆盖。 |
| Pi 目录展示 / 状态 API | `web::handlers::providers::tests::providers_status_includes_pi_when_available`、`web::provider_availability::tests::{parse_provider_name_accepts_pi, provider_name_key_pi}` | 既有 | Task 1 覆盖。 |
| Pi RPC 文本流、会话标识、完成和错误映射 | `cross_cutting::pi_provider::tests::{session_in_auto_mode_emits_tool_audit_events_from_recorded_stream, session_failure_is_terminal_no_retry, session_demultiplexes_response_by_id}` | 既有 + 本轮新增 | 本轮扩展原 text-stream 测试；错误映射/response demux 为既有。 |
| Pi 工具调用在 Auto 下直接执行且保留审计运行事件 | `cross_cutting::pi_provider::tests::session_in_auto_mode_emits_tool_audit_events_from_recorded_stream` | 本轮新增 | 从真实 `auto_text.jsonl` 提取 `tool_execution_start/end`，在真实 session 驱动路径断言 `ProviderEvent::ToolCall/ToolResult`。 |
| Pi 取消：`abort` 后会话终止、无完成事件 | `cross_cutting::pi_provider::tests::{session_aborts_on_provider_command_abort, session_suppresses_output_after_abort, session_aborts_when_token_cancels_during_get_state_handshake, session_aborts_when_abort_command_and_token_race_during_prompt_handshake}` | 既有 | Task 2 adapter/session 覆盖。 |
| Pi 取消到 Workspace 前端既有已取消状态，且停止输出 | `workspace_ws_integration::workspace_ws_abort_with_pi_reaches_cancelled_state_and_stops_output` | 本轮新增 | Pi 注册到真实 WS registry；断言 Aborted 状态、回到 `prepare_context`、无后续输出、重连状态中的 failed timeline node 与“运行已中止”。 |
| Pi 恢复：`--session-id` | `cross_cutting::pi_provider::tests::{build_args_resume_includes_session_id, session_resumes_with_existing_session_id}` | 既有 | Task 2 覆盖。 |
| Story / Design / Work Item 三入口均可选择、启动 Pi，且收到 Auto | `product::workspace_engine::tests::pi_author_runs_from_story_design_and_work_item_entries_in_auto_mode` | 本轮新增 | 每种 `WorkspaceType` 均创建真实 author run；断言 adapter 只启动一次、输入为 `(ProviderType::Pi, Auto)`。 |
| Workspace 权限持久化和默认 Auto | `product::models::tests::{workspace_role_permission_modes_default_is_auto, old_workspace_session_record_without_permission_modes_deserializes_to_auto}`、`product::workspace_engine::tests::start_generation_locks_selected_modes_into_store` | 既有 | Task 3 覆盖。 |
| Claude/Codex Supervised 保留且授权链路可用 | `workspace_ws_integration::workspace_ws_supervised_permission_allows_real_stream_to_complete` | 更新 | 显式持久化 Author = `Supervised`，保持 permission request → 批准 → 完成意图。 |
| Supervised 下错误 permission id 仍被协议层拒绝 | `workspace_ws_integration::workspace_ws_unmatched_permission_response_returns_protocol_error` | 更新 | 显式 Supervised，断言错误 id 被拒绝，正确 id 后完成。 |
| Pi Supervised 服务端归一为 Auto | `product::workspace_engine::tests::start_generation_normalizes_pi_role_to_auto_and_keeps_disabled_reviewer_mode`、`web::workspace_ws_handler::tests::provider_select_then_user_message_forces_pi_to_auto_from_stale_supervised_mode`、`product_coding_workspace_engine::pi_role_with_supervised_mode_normalized_to_auto` | 既有 | Task 3/4/5 覆盖。 |
| Workspace 启动失败 fail-fast、无替代 Provider | `product::workspace_engine::tests::pi_start_failure_does_not_retry_selected_provider`、`web::workspace_ws_handler::tests::pi_failures_do_not_start_registered_alternate_provider` | 既有 | Task 3 覆盖。 |
| Coding Provider / 权限 / fail-fast | `product_coding_workspace_engine::{pi_role_with_supervised_mode_normalized_to_auto, pi_failure_does_not_trigger_fresh_retry}`、`web::coding_ws_handler::tests::coding_pi_start_failure_does_not_start_registered_alternate_provider`、`product::coding_workspace_engine::tests::coder_rework_normalizes_pi_supervised_mode_to_auto` | 既有 | Task 4 覆盖。 |
| 前端 Pi Auto-only 两面板及状态可见性 | `ProviderConfigPanel` 与 `CodingProviderConfigPanel` 的 Pi Auto-only 测试、`coding_ws_permission_mode_select_normalizes_pi_to_auto`、`providerConfigFor` Pi 保留 | 既有 | Task 5 覆盖。 |
| 仓库初始化 UI 不展示 Pi | `CreateRepositoryDialog > 仓库初始化不显示 Pi 即使 Pi 可用` | 既有 | Task 1 覆盖。 |
| 仓库初始化后端执行路径不会输入 Pi | `product::repository_store::initializer::tests::repository_initializer_runs_four_independent_claude_turns_in_strict_order` | 更新 | 遍历四个实际输入，只允许 Claude Code。 |
| Task Runner HTTP 入口拒绝 `pi` | `web::provider_availability::tests::parse_provider_type_still_rejects_pi` | 既有 | 错误文本含 `pi`。 |
| Task Runner router 拒绝 Pi 且不调用 adapter | `task_run::provider_factory::tests::routing_provider_rejects_pi_without_calling_real_providers` | 既有 | Task 1 覆盖。 |
| 兼容性矩阵不包含 Pi | `cli_adapter_baseline::default_matrix_contains_claude_code_and_codex_cli_entries` | 更新 | 断言 `entry_for(ProviderType::Pi).is_none()`。 |
| 静态节点契约不产生 Pi | `context_builder::contract_workflow_and_prompt_registries_cover_p4_execution_and_closure_nodes` | 更新 | 遍历 `phase1_node_contract_table()` 真实结构化数据。 |

### 仍未覆盖的 Brief 项

无。上表每个 Task 6 brief 条目均列出真实测试；本轮不再把仅遍历 `WorkspaceType` 但默认 Claude Code 的测试，误报为 Pi 三入口覆盖。

## 三个新增测试组：TDD RED / GREEN 证据

1. **Pi 三 Workspace 入口（6.2）**
   - 新测试：`pi_author_runs_from_story_design_and_work_item_entries_in_auto_mode`。
   - RED：在隔离 `d0378419` worktree 对 `provider_type_for_name(Pi)` 定向变异为 `Fake` 后运行该测试，结果 `0 passed / 1 failed`；首个 Story 断言观测到 `[(Fake, Supervised)]`，期望 `[(Pi, Auto)]`。说明此测试会在 Pi 未被正确选中或未归一为 Auto 时失败。
   - GREEN：本分支定向运行 `1 passed / 0 failed`。

2. **Pi Auto 工具审计（6.1）**
   - 新测试：`session_in_auto_mode_emits_tool_audit_events_from_recorded_stream`。
   - RED：在隔离 `d0378419` worktree 令 `parse_pi_tool_start/end` 返回 `None` 后运行，结果 `0 passed / 1 failed`，缺少期望的 `ProviderEvent::ToolCall`。说明不是孤立 parser 测试，而是 session 到 ProviderEvent 的真实路径断言。
   - GREEN：本分支定向运行 `1 passed / 0 failed`；测试从 `auto_text.jsonl` 驱动 start/end，确认 Pi 在 Auto 中无 Aria 审批 round trip，且仍发出完整工具审计事件。

3. **Pi WebSocket 取消（6.1）**
   - 新测试：`workspace_ws_abort_with_pi_reaches_cancelled_state_and_stops_output`，并复用既有 WS scaffolding/fixture 创建方法。
   - RED：在隔离 `d0378419` worktree 将 `WsInMessage::Abort` 处理临时变为 no-op 后运行，结果 `0 passed / 1 failed`，等待下一 WS 消息在 3 秒超时。说明测试确实验证了前端 Abort 到运行取消的端到端路径。
   - GREEN：本分支定向运行 `1 passed / 0 failed`；Pi adapter 收到 Abort，前端获得 `Aborted`，回到 `prepare_context`，150 ms 内没有 trailing output，重连状态显示既有 failed/“运行已中止”取消结果。

## 本轮质量门禁实际输出

| 命令 | 结果 | 实际计数 / 输出 |
|---|---|---|
| `cargo test -p cadence-aria` | 通过 | lib 1451；bin 0；`it_core` 148；`it_interactive` 43；`it_product` 198；`it_provider` 54；`it_task_run` 31；`it_web` 312 passed、12 ignored；doc-test 1。合计 **2238 passed / 12 ignored / 0 failed**。相对于旧基线 2236，本轮新增两个 unit test functions（Pi 三入口、Pi Auto 工具审计）和一个 WS integration test function；同时将原 session text-stream 测试替换为其加强版本，因此净增 2。 |
| `cargo clippy -p cadence-aria --all-targets` | 通过 | `Finished dev profile`，0 warnings。 |
| `cargo fmt --check` | 通过 | 无输出，退出码 0。 |
| `cd web && npm test` | 通过 | **91 files / 732 tests passed**。 |

所有 Rust 命令均未使用 `-j 1`。

## 产品缺陷

未发现。所有新增测试在既有生产实现上直接转绿；本轮没有修改生产代码或生产行为。
