# Kimi 审核与 Work Item Plan 契约修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Kimi reviewer 在仅遗漏结构化输出包装时安全恢复审核结果，并让 Work Item Plan 的 author/revision prompt 直接获得 validator 的固定 Markdown 契约。

**Architecture:** 继续由严格的 structured-output parser 负责 nonce 与 JSON 验证；Kimi 只在已有 recoverable JSON 的包装错误上复用现有一次 repair，并以 JSON 逐值相等作为接受条件。artifact schema 不新增手写规则，而是移除 Work Item Plan 从同一 validator-derived schema projection 的排除，使 generation、retry 与 revision 共享同一来源。

**Tech Stack:** Rust、Tokio、serde_json、Cargo test、OpenSpec `add-kimi-code-provider`。

## Global Constraints

- 追溯 Change：`openspec/changes/add-kimi-code-provider/`；本轮只实现已更新的 tasks `5.2`、`5.3`，不标记无关历史任务。
- 不修改、删除或回写 `.aria` 中的历史 workspace/timeline 数据，也不调用真实 Provider。
- 保留 `parse_structured_output` 的严格 nonce 校验；不得接受裸 `</ARIA_STRUCTURED_OUTPUT>`，不得降低跨请求防串包保护。
- Kimi repair 最多一次，且只接受 `missing_end_tag`、`missing_end_nonce`、`nonce_mismatch` 三类、具有 recoverable JSON 的错误；repair JSON 必须和首轮 JSON 逐值相等。Pi 继续排除 review repair；Kimi artifact retry 继续排除。
- repair provider 启动/运行失败、JSON 改变或仍不可解析必须 fail-closed 到 `needs_human` / `user_triage_required`；用户主动 Abort 保持既有 aborted/PrepareContext 取消语义，不 retry、不 fallback。无可信 reviewer finding 时仅 `source="human"` 的非空修改说明可启动 author revision。
- 通用 Markdown Work Item Plan author/retry/delta-full revision schema 必须从 `artifact_constraint_spec_for` 投影，提供 validator-required Markdown heading 与 `[TASK-001]` 示例；JSON Outline/Draft 主链由独立 JSON schema 约束，不要求注入 Markdown heading。
- 共享 reviewer 链路至少覆盖 Story、Design、Work Item、Work Item Plan；不得因本次修复回归前三者。
- 遵循 TDD：先提交并运行失败的回归测试，再写最小实现；定向测试使用 `cargo test --locked --lib <过滤名>`，禁止给 Cargo 传 `-j 1`。
- 每个任务独立原子提交；完成后不 push，除非用户另行要求。

---

### Task 1: Kimi 安全审核包装修复与无目标返修保护

**Files:**

- Modify: `src/product/workspace_engine/review/drive.rs:52-186,639-652`
- Modify: `src/product/workspace_engine/decisions.rs:369-457`
- Modify: `src/product/workspace_engine/tests/part_03/part_03.rs:496-780`
- Modify: `src/product/workspace_engine/tests/part_03/part_06.rs:204-360`
- Modify: `src/product/workspace_engine/tests/part_06.rs:176-225`

**Interfaces:**

- Consumes: `ReviewCompletionError::is_repairable()`, `repair_payload_is_compatible`, `build_review_repair_input`, `ReviewGate::UserTriageRequired`, `HumanConfirmDecision::RequestChange`。
- Produces: Kimi 可进入既有的一次 review repair 分支；无可信 fallback review 只能在携带 `source="human"` 的非空人工说明时进入 `ReviewDecisionOutcome::StartRevision`；主动 Abort 保持既有取消语义。

- [x] **Step 1: 写出 Kimi repair 的失败回归测试**

  在 `part_03/part_03.rs` 将“Kimi 不尝试 repair”的旧断言改为 table-driven 成功用例：首轮输出 `missing_end_nonce_output(review_json)`，第二轮输出相同 `review_json` 的 `valid_structured_output`。用例必须包含 `Story`、`Design`、`WorkItem`、`WorkItemPlan`，并分别断言：provider 恰好启动两次、第二次携带首轮 provider session id、诊断 `repair_attempted=true` 且 `repair_succeeded=true`、仅产生 Started/Completed repair 事件。

  为 Work Item Plan 使用能被 `queued_review_engine_for` 正常审核的候选 artifact；若其审核路径与普通 Markdown 不同，保留既有专用 fixture，不改变生产路由来迁就测试。

  同时保留或新增 Kimi repair 改变 `verdict`/`summary`/`findings` 的测试，断言其只能 `NeedsHuman`、diagnostic code 为 `repair_payload_changed`、不会产生第三次启动；Pi 的断言必须保持 repair 禁用。

- [x] **Step 2: 运行测试，确认 RED**

  Run:

  ```bash
  cargo test --locked --lib kimi_review_repairs_missing_end_nonce_once_for_all_workspace_types
  ```

  Expected: FAIL，因为 `provider_allows_review_repair` 当前排除 `ProviderName::KimiCode`，Kimi 只启动一次并进入 fallback。

- [x] **Step 3: 写出无可信返修目标的失败回归测试**

  在 `part_06.rs` 创建处于 `HumanConfirm` 的普通 Workspace，设置 `latest_review_verdict` 为 `NeedsHuman`、`UserTriageRequired`、空 findings、`structured_output_diagnostic.repair_succeeded=false`。调用：

  ```rust
  engine
      .handle_human_confirm(HumanConfirmDecision::RequestChange, None)
      .await
  ```

  断言 `None`、`{"description":"补充失败路径"}` 与 `{"description":"补充失败路径","source":"review_findings"}` 均返回含 source 的错误，stage 仍为 `WorkspaceStage::HumanConfirm`，且没有 Active `TimelineNodeType::Revision`。再用 `{"description":"补充失败路径","source":"human"}` 覆盖正向路径，断言仍能开始 revision，避免拒绝真实人工目标；矩阵须覆盖 Story、Design、WorkItem 与 WorkItemPlan Outline。

- [x] **Step 4: 运行测试，确认 RED**

  Run:

  ```bash
  cargo test --locked --lib human_confirm_request_change_requires_context_after_untrusted_review
  ```

  Expected: FAIL，因为当前实现会把空 fallback summary 直接转换为 author revision。

- [x] **Step 5: 写最小实现**

  在 `provider_allows_review_repair` 中只排除 `ProviderName::Pi`，并把过期的“resume 未实证”注释改为说明 Kimi 仅复用现有的一次 JSON 等值 repair。不得改动 `ReviewCompletionError::is_repairable`、`repair_payload_is_compatible` 或 parser 的 nonce 规则；它们已经分别限定错误类别和 JSON 等值。

  在 `handle_human_confirm` 的普通 `RequestChange` 分支，先规范化 `human_confirm_payload_description(payload)` 与 payload source。当最新 reviewer verdict 是 `ReviewGate::UserTriageRequired` 且没有可信 findings 时，只有 `source="human"` 且说明非空才允许继续；此检查必须发生在 `complete_active_node`、stage transition 和创建 Revision timeline node 之前。正常人工说明与现有 `UserConfirmAllowed` 路径保持行为不变。

- [x] **Step 6: 运行定向测试，确认 GREEN**

  Run:

  ```bash
  cargo test --locked --lib "kimi_review_repairs_missing_end_nonce_once_for_all_workspace_types|review_structured_output_repair_rejects_payload_change|human_confirm_request_change_requires_context_after_untrusted_review|handle_human_confirm_request_change_starts_revision"
  ```

  Expected: PASS；覆盖 Kimi 成功、变更 payload fail-closed、无目标不启动、带人工目标仍启动。

- [x] **Step 7: 自审并提交**

  Run:

  ```bash
  cargo fmt --check
  git diff --check
  git status --short
  ```

  Commit:

  ```bash
  git add src/product/workspace_engine/review/drive.rs src/product/workspace_engine/decisions.rs src/product/workspace_engine/tests/part_03/part_03.rs src/product/workspace_engine/tests/part_03/part_06.rs src/product/workspace_engine/tests/part_06.rs
  git commit -m "fix(kimi): safely repair reviewer output wrappers"
  ```

### Task 2: Work Item Plan validator schema 投影

**Files:**

- Modify: `src/product/workspace_engine/artifact_constraints.rs:310-370`
- Modify: `src/product/workspace_engine/tests/part_31.rs:185-430`

**Interfaces:**

- Consumes: `artifact_constraint_spec_for(&WorkspaceType)`, `author_artifact_schema_contract_for`, `build_artifact_retry_prompt`, `WorkspaceEngine::build_revision_delta_prompt`, `WorkspaceEngine::build_revision_full_prompt`。
- Produces: 四种通用 Markdown workspace 均可获得 `[artifact_schema_contract]`，Work Item Plan Markdown prompt 含 parser 派生 heading 与 `[TASK-001]` 示例；JSON Outline/Draft 主链不注入 Markdown heading。

- [x] **Step 1: 写出 Work Item Plan schema 的失败回归测试**

  将 `part_31.rs` 中下列测试矩阵从三种类型扩展为四种类型：

  ```rust
  [
      WorkspaceType::Story,
      WorkspaceType::Design,
      WorkspaceType::WorkItem,
      WorkspaceType::WorkItemPlan,
  ]
  ```

  至少覆盖：`initial_author_prompts_render_parser_derived_schema`、`parser_derived_schema_contract_keeps_concrete_heading_and_id_examples`、`retry_and_revision_prompts_render_parser_derived_schema`。对 Work Item Plan 显式断言 schema/prompt 包含 marker、每个 parser-required heading 和 `[TASK-001]`。保留 Story、Design、Work Item 既有断言，证明共享投影没有回归。

- [x] **Step 2: 运行测试，确认 RED**

  Run:

  ```bash
  cargo test --locked --lib parser_derived_schema_contract_keeps_concrete_heading_and_id_examples
  ```

  Expected: FAIL，因为 `markdown_artifact_constraint_spec_for` 对 `WorkspaceType::WorkItemPlan` 返回 `None`。

- [x] **Step 3: 写最小实现**

  移除 `markdown_artifact_constraint_spec_for` 对 `WorkspaceType::WorkItemPlan` 的排除，使它直接返回 `Some(artifact_constraint_spec_for(workspace_type))`。保留 `append_markdown_artifact_schema_items` 作为唯一的 heading、token、ID 示例渲染器；不要复制 Work Item Plan heading 常量，也不要修改 validator 接受条件。

- [x] **Step 4: 运行定向测试，确认 GREEN**

  Run:

  ```bash
  cargo test --locked --lib "initial_author_prompts_render_parser_derived_schema|parser_derived_schema_contract_keeps_concrete_heading_and_id_examples|retry_and_revision_prompts_render_parser_derived_schema|work_item_plan_constraints_allow_task_ids"
  ```

  Expected: PASS；初始 author、artifact retry、delta revision、full revision 均对 Work Item Plan 投影同一 schema，且既有 validator 仍接受合法计划。

- [x] **Step 5: 自审并提交**

  Run:

  ```bash
  cargo fmt --check
  git diff --check
  git status --short
  ```

  Commit:

  ```bash
  git add src/product/workspace_engine/artifact_constraints.rs src/product/workspace_engine/tests/part_31.rs
  git commit -m "fix(workspace): project plan artifact schema"
  ```

## 验证与收尾

- [x] 更新 `openspec/changes/add-kimi-code-provider/tasks.md`：仅将 `5.2`、`5.3` 标记为完成。
- [ ] 执行 `cargo fmt --check`、`cargo check --locked`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked`、`git diff --check`。
- [ ] 使用独立 reviewer 审查任务提交及最终全分支 diff；若发现 Critical/Important，按 SDD fix/re-review loop 修正。
- [ ] 验证后重启当前 worktree 的后端服务并仅检查 `/api/health`、前端 `/`、前端代理 `/api/health`。由用户在 `Work Item Plan #workspace_session_0003` 重新触发审核/返修；不得改写旧节点。
