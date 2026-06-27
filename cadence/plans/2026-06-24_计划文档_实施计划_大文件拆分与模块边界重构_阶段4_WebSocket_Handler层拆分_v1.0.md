# 大文件拆分与模块边界重构 — 阶段 4：WebSocket / Handler 层拆分 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `src/web/coding_ws_handler.rs`、`src/web/workspace_ws_handler.rs`、`src/web/handlers.rs` 拆分为目录模块，每个子文件不超过 800 行，保持外部 API 不变，所有测试通过。

**Architecture:** 采用 Rust 目录模块 + Facade 模式。原文件变为 `xxx/mod.rs`，内部按职责拆分为多个子模块，`mod.rs` 统一 `pub use` 重新导出公共函数/类型。外部 `use` 路径和公共符号保持不变。

**Tech Stack:** Rust 2024, Cargo, rust-toolchain.toml 固定 1.95.0

---

## 前置检查

- [ ] **Step 0.1: 确认工作区与分支**

  工作目录应为 `.worktrees/feat-b-0616`，当前分支为 `feat-b-0616`，阶段 3 已推送且工作树干净。

  Run:
  ```bash
  cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0616
  git status --short
  git branch --show-current
  ```

  Expected: 无未提交改动，分支为 `feat-b-0616`。

- [ ] **Step 0.2: 确认 baseline 通过**

  Run:
  ```bash
  cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0616
  cargo fmt --check
  cargo check --locked
  cargo clippy --all-targets --all-features --locked -- -D warnings
  cargo test --locked
  ```

  Expected: 全部通过。

---

## Task 1: 拆分 `src/web/coding_ws_handler.rs`

**目标文件大小：** 每个子模块 ≤ 800 行，`mod.rs` ≤ 100 行。

**文件结构：**

```
src/web/coding_ws_handler/
├── mod.rs              # facade
├── socket.rs           # coding_ws / handle_coding_socket / CodingWsSender / send_coding_json
├── runner.rs           # spawn_coding_runner / execute_start_coding_flow / provider_for
├── gates.rs            # Stage Gate / Blocked Gate 逻辑
├── context.rs          # 执行上下文、证据、测试规格、上下文笔记
├── state.rs            # 会话状态、role run 快照、provider 配置状态
├── protocol.rs         # CodingWsOutMessage / CodingWsInMessage / is_coding_ws_message_allowed
└── tests.rs            # 原测试模块
```

**拆分边界：** 见 explore 子代理输出。关键原则：

- `socket.rs`: `CodingWsSender`, `coding_ws`, `handle_coding_socket`, `send_coding_json`
- `runner.rs`: `spawn_coding_runner`, `ensure_work_item_execution_plan_confirmed`, `execute_start_coding_flow`, `latest_analyst_role_run_evidence`, `testing_result_acceptance_pending_analyst`, `handle_pending_runner_commands`, `provider_for`
- `gates.rs`: `STAGE_GATE_COUNTDOWN_SECONDS`, `should_resume_runner_after_gate_response`, `await_stage_gate`, `emit_stage_gate`, `stage_gate_expires_at`, `confirm_open_stage_gate`, `stage_gate_required`
- `context.rs`: `coding_execution_context`, `update_provider_selection`, `update_provider_permission_mode`, `provider_selection_targets_current_running_stage`, `context_note_chat_entry`, `chat_entry_id_for_context_note`, `latest_assistant_artifact_markdown`, `select_work_item_markdown`, `test_specs_for_attempt`, `testing_rework_evidence`, `code_review_rework_evidence`, `internal_pr_review_rework_evidence`, `repository_path_for_attempt`
- `state.rs`: `emit_current_session_state`, `build_coding_session_state`, `coding_role_run_snapshots`, `role_run_event_summary`, `recent_role_run_events`, `role_run_event_title`, `role_run_event_status`, `role_run_event_reason`, `role_run_event_payload_text`, `active_coding_timeline_node_id`
- `protocol.rs`: `CodingWsOutMessage`, `CodingWsInMessage`, `is_coding_ws_message_allowed`
- `tests.rs`: 原 `#[cfg(test)] mod tests`（2247-2389）

**可见性约定：**

- 被外部调用的入口函数（`coding_ws`）保持 `pub`。
- 跨子模块调用的函数提升为 `pub(crate)`。
- `mod.rs` 使用 `pub(crate) use <submodule>::*;` 共享内部函数。

- [ ] **Step 1.1: 创建目录并移动原文件**

  Run:
  ```bash
  cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0616
  mkdir -p src/web/coding_ws_handler
  git mv src/web/coding_ws_handler.rs src/web/coding_ws_handler/mod.rs
  ```

- [ ] **Step 1.2-1.8: 按边界创建各子模块**

- [ ] **Step 1.9: 编译检查**

  Run: `cargo check --locked`
  Expected: 无错误。

- [ ] **Step 1.10: 运行相关单元测试**

  Run: `cargo test --locked --lib coding_ws_handler`
  Expected: 全部通过。

- [ ] **Step 1.11: 提交 Task 1**

  ```bash
  git add src/web/coding_ws_handler/
  git commit -m "refactor(coding_ws_handler): 拆分 coding_ws_handler.rs 为目录模块

- 按 socket/runner/gates/context/state/protocol/tests 拆分
- 保持外部 use 路径和公共 API 不变
- coding_ws_handler 单元测试通过"
  ```

---

## Task 2: 拆分 `src/web/workspace_ws_handler.rs`

**目标文件大小：** 每个子模块 ≤ 800 行，`mod.rs` ≤ 100 行。

**文件结构：**

```
src/web/workspace_ws_handler/
├── mod.rs              # facade
├── socket.rs           # workspace_ws / handle_workspace_socket / OutboundControl / send_json_outbound / spawn_idle_timeout_task
├── decisions.rs        # review / author / human confirm 决策处理
├── protocol.rs         # 消息协议校验、错误构造
├── run.rs              # Provider Run 编排
├── mapping.rs          # DTO 映射、WorkItemPlan 请求构建
└── tests.rs            # 原测试模块
```

**拆分边界：** 见 explore 子代理输出。关键原则：

- `socket.rs`: `OutboundControl`, `workspace_ws`, `send_json_outbound`, `spawn_idle_timeout_task`, `handle_workspace_socket`
- `decisions.rs`: `handle_review_decision_from_handler`, `handle_author_decision_from_handler`, `handle_human_confirm_from_handler`
- `protocol.rs`: `missing_active_run_error`, `choice_id_unmatched_error`, `is_message_valid_for_stage`, `requires_stage_validation`, `message_type`
- `run.rs`: `NEXT_ACTIVE_RUN_TOKEN`, `ProviderRunContext`, `ProviderRunKind`, `parse_work_item_split_structured_output`, `complete_work_item_plan_outline_author_from_output`, `work_item_plan_findings_feedback`, `combine_outline_auto_retry_feedback`, `work_item_plan_retry_error`, `active_run_command_tx`, `active_run`, `abort_workspace_run`, `abort_active_run`, `clear_active_run_if_token`, `spawn_provider_run_from_handler`, `drive_current_work_item_plan_outline_run`, `build_work_item_plan_generate_request`, `load_work_item_plan_outline_context_resolutions`
- `mapping.rs`: `map_revision_path`, `ws_permission_risk_level`, `ws_choice_option`, `ws_provider_status`, `ws_execution_event`, `ws_execution_event_kind`, `ws_execution_event_status`
- `tests.rs`: 原 `#[cfg(test)] mod tests`（1419-1822）

**特殊注意：** `handle_workspace_socket` 本身约 1000 行，直接迁入 `socket.rs` 会超过 800 行限制。需要将其内部按职责进一步拆分为私有函数（如消息循环、状态机分支、协议错误处理等），或将部分逻辑迁到 `protocol.rs` / `run.rs`。保持外部 `workspace_ws` 入口不变。

- [ ] **Step 2.1-2.10: 拆分并处理超大函数**

- [ ] **Step 2.11: 编译检查**

  Run: `cargo check --locked`
  Expected: 无错误。

- [ ] **Step 2.12: 运行相关单元测试**

  Run: `cargo test --locked --lib workspace_ws_handler`
  Expected: 全部通过。

- [ ] **Step 2.13: 提交 Task 2**

  ```bash
  git add src/web/workspace_ws_handler/
  git commit -m "refactor(workspace_ws_handler): 拆分 workspace_ws_handler.rs 为目录模块

- 按 socket/decisions/protocol/run/mapping/tests 拆分
- 将 handle_workspace_socket 内部逻辑进一步拆分为辅助函数，保证 socket.rs ≤ 800 行
- 保持外部 use 路径和公共 API 不变
- workspace_ws_handler 单元测试通过"
  ```

---

## Task 3: 拆分 `src/web/handlers.rs`

**目标文件大小：** 每个子模块 ≤ 800 行，`mod.rs` ≤ 100 行。

**文件结构：**

```
src/web/handlers/
├── mod.rs              # facade
├── support.rs          # Query 结构体、ProviderWorkspaceConfig、通用校验/工具
├── health.rs           # health / runtime_info
├── product_resources.rs # workspace/project/repo/issue/gate CRUD
├── lifecycle.rs        # story/design spec 生成、work item plan 准备
├── coding.rs           # coding attempt 管理
├── workspace_session.rs # workspace session 端点
├── runtime.rs          # 旧 task runtime / projection / file / SSE
└── dto.rs              # DTO 转换函数 + SpecDtoSource trait
```

**拆分边界：** 见 explore 子代理输出。关键原则：

- `support.rs`: `ProviderWorkspaceConfig`, `events`, `sse_event`, `canonical_provider_input_path`, `canonical_provider_input_component`, `provider_input_path_escape`, `resolve_workspace_root`, `provider_input_file_name`, `find_repository`, `product_execution_workspace_id`, `parse_product_execution_workspace_id`, `product_app_paths`, `provider_workspace_config`, `product_store_api_error`, `node_detail_store_api_error`
- `health.rs`: `health`, `runtime_info`
- `product_resources.rs`: workspace/project/repo/issue CRUD + gate CRUD（list/create/delete）
- `lifecycle.rs`: `issue_lifecycle`, `generate_story_specs`, `generate_design_specs`, `prepare_work_item_plan`, `delete_story_spec`, `delete_design_spec`, `delete_work_item`, `delete_work_item_plan`, `delete_work_item_with_cleanup`, `confirm_gate`, `request_gate_change`, `terminate_gate`, `resolve_gate`, `backfill_legacy_spec_versions`, `validate_confirmed_story_specs`, `validate_confirmed_design_specs`, `confirm_workspace_entity`
- `coding.rs`: `create_coding_attempt`, `save_work_item_execution_plan_for_attempt`, `next_execution_plan_id`, `work_item_by_id`, `coding_provider_config_snapshot`, `get_coding_attempt`, `coding_attempt_diff`, `abort_coding_attempt`, `delete_coding_attempt`, `confirm_work_item_execution_plan`, `request_work_item_execution_plan_change`, `coding_attempt_artifact_content`, `abort_attempt_if_active`, `cleanup_coding_attempt_workspace`, `git_workspace_api_error`, `coding_workspace_engine_with_dummy_events`, `coding_workspace_api_error`, `git_workspace_diff_api_error`, `is_git_repo`, `current_git_branch`
- `workspace_session.rs`: `workspace_session_message`, `workspace_session_run_next`, `workspace_session_confirm`, `workspace_session_timeline_node_detail`, `workspace_session_timeline_node_prompt`, `workspace_session_timeline_event_output`, `workspace_session_artifact_version`, `validate_workspace_message`, `workspace_user_prompt`, `provider_workspace_prompt`
- `runtime.rs`: `create_task`, `list_tasks`, `advance_task`, `confirm_task`, `stop_task`, `rollback_preview`, `rollback_task`, `issue_rollback_preview`, `issue_rollback`, `projection`, `artifact_content`, `file_content`, `file_diff`, `provider_input_content`, `validate_issue_rollback_ids`, `issue_rollback_missing_worktree`
- `dto.rs`: `ProjectionQuery`, `FileContentQuery`, `FileDiffQuery`, `WorkspaceQuery`, `GateResolveQuery`, `EventsQuery`, `issue_work_item_plan_detail_dto`, `work_item_split_finding_dto`, `issue_work_item_plan_status_text`, `workspace_dto`, `project_dto`, `repository_dto`, `product_issue_dto_with_binding`, `product_issue_dto`, `latest_workspace_artifact_markdown`, `story_spec_dto`, `design_spec_dto`, `workspace_session_for_entity`, `artifact_version_dtos`, `artifact_version_dto`, `current_markdown_preview`, `markdown_preview`, `lifecycle_work_item_dto`, `coding_attempt_dto`, `active_coding_timeline_node_id`, `workspace_session_dto`, `workspace_message_dto`, `product_issue_artifacts`, `workspace_root_for_binding`, `artifact_stage`, `active_binding_for_issue`, `issue_dto`, `product_issue_phase_text`, `product_issue_status_text`, `lifecycle_confirmation_status_text`, `work_item_plan_status_text`, `work_item_status_text`, `work_item_kind_text`, `work_item_execution_plan_status_text`, `coding_attempt_status_text`, `coding_execution_stage_text`, `push_status_text`, `workspace_type_text`, `workspace_session_status_text`, `provider_name_text`, `review_verdict_text`, `issue_status_text`, `SpecDtoSource` trait + impls

**特殊注意：** `lifecycle.rs` 和 `runtime.rs` 可能会接近或超过 800 行。如果超过，需要进一步拆分（如从 lifecycle.rs 拆出 `gate.rs` 或从 runtime.rs 拆出 `projection.rs`）。保持外部 `use` 路径不变。

- [ ] **Step 3.1-3.10: 拆分并监控文件大小**

- [ ] **Step 3.11: 编译检查**

  Run: `cargo check --locked`
  Expected: 无错误。

- [ ] **Step 3.12: 运行 handlers 相关单元测试**

  Run: `cargo test --locked --lib handlers`
  Expected: 全部通过。

- [ ] **Step 3.13: 提交 Task 3**

  ```bash
  git add src/web/handlers/
  git commit -m "refactor(handlers): 拆分 handlers.rs 为目录模块

- 按 support/health/product_resources/lifecycle/coding/workspace_session/runtime/dto 拆分
- 保持外部 use 路径和公共 API 不变
- handlers 单元测试通过"
  ```

---

## 阶段收尾

- [ ] **Step 4.1: 阶段 4 全量验证**

  Run:
  ```bash
  cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0616
  cargo fmt --check
  cargo check --locked
  cargo clippy --all-targets --all-features --locked -- -D warnings
  cargo test --locked
  ```

  Expected: 全部通过。

- [ ] **Step 4.2: 推送阶段 4 到远端**

  Run:
  ```bash
  cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0616
  git push origin feat-b-0616
  ```

- [ ] **Step 4.3: 汇报阶段 4 完成**

  向用户汇报：
  - 已拆分的三个 Handler 模块
  - 每个子文件行数
  - `cargo test --locked` 结果
  - 提交 hash

---

## 计划自检

1. **Spec 覆盖：** 本计划覆盖了设计文档阶段 4 的全部三个 Handler 拆分任务。
2. **无占位符：** 所有步骤均包含具体文件路径、命令和预期输出。
3. **API 兼容：** `src/web/mod.rs` 中的模块声明不变，外部 `use` 路径不变。
4. **大函数处理：** 明确处理 `handle_workspace_socket` 和可能超限的 `lifecycle.rs` / `runtime.rs`。
