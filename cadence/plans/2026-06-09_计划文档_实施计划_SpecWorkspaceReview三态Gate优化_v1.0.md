# Spec Workspace Review 三态 Gate 优化实施计划

> **执行约束**：本计划只适用于 `/Users/michaelche/Documents/git-folder/github-folder/cadence-aria/.worktrees/fix_author_confirm_followup`。执行前确认 `git status --short`；不得回退用户已有改动。Rust 命令直接使用宿主机 Cargo，禁止 Docker，禁止 `-j 1`。前端使用 `pnpm`。

## 目标

将 Story Spec、Design Spec、Work Item 共用的 review gate 从二态扩展为三态：

- `requires_revision`：强阻断，进入 `review_decision`。
- `user_confirm_allowed`：可确认但可采纳建议，进入 `human_confirm`。
- `user_triage_required`：reviewer 返修意图不结构化或输出异常，进入 `human_confirm` 由用户裁决。

同时补齐实时 `review_complete` 的 `findings/review_gate` 字段，避免 UI 在刷新前缺少完整 review 分组。

## 文件范围

- Modify: `src/web/workspace_ws_types.rs`
  - `ReviewGate` 增加 `UserTriageRequired`。
  - `ReviewComplete` WS 输出增加 `findings`、`review_gate`。
- Modify: `src/product/workspace_engine.rs`
  - 调整 review 解析分类。
  - 调整 `complete_review()` 分流和 artifact verdict 映射。
  - 调整 reviewer prompt。
  - 补 Story/Design/Work Item 三类测试。
- Modify: `src/web/workspace_ws_handler.rs`
  - 如 WS type 变更需要同步序列化/测试。
- Modify: `web/src/api/types.ts`
  - `ReviewGate` 增加 `user_triage_required`。
  - `review_complete` 消息增加 `findings/review_gate`。
- Modify: `web/src/hooks/useWorkspaceWs.ts`
  - 实时 `review_complete` 写入完整 `findings/review_gate`。
- Modify: `web/src/state/workspace-ws-store.ts`
  - gate prompt metadata 透传 `findings/comments/review_gate`。
  - 恢复态兼容 `user_triage_required`。
- Modify: `web/src/components/chat-workspace/entries/ReviewVerdictEntry.tsx`
  - 新增 `user_triage_required` 标题。
- Modify: `web/src/components/chat-workspace/entries/GatePromptEntry.tsx`
  - 增加“采纳建议并返修”和“按 reviewer 意见返修”按钮。
  - 扩展 `onDecision` 支持 `request-change` payload。
- Modify: `web/src/components/chat-workspace/ChatEntryRenderer.tsx`
  - 透传新增 GatePromptEntry 回调参数。
- Modify: `web/src/components/chat-workspace/ChatEntryList.tsx`
  - 透传 human confirm request-change payload。
- Modify: `web/src/pages/ChatWorkspacePage.tsx`
  - `handleHumanConfirm` 支持 payload。
- Tests:
  - `src/product/workspace_engine.rs`
  - `src/web/workspace_ws_types.rs`
  - `web/src/components/chat-workspace/entries/p1-entries.test.tsx`
  - `web/src/state/workspace-ws-store.test.ts`
  - `web/src/hooks/useWorkspaceWs.test.tsx`
  - `web/src/pages/ChatWorkspacePage.test.tsx`

## Task 1: 后端 ReviewGate 三态合约

- [x] 在 `ReviewGate` 增加 `UserTriageRequired`，序列化为 `user_triage_required`。
- [x] 为旧数据保留 default：缺失 `review_gate` 时仍默认为 `UserConfirmAllowed`。
- [x] 扩展 `ReviewComplete` 事件结构，携带：
  - `findings: Vec<ReviewFinding>`
  - `review_gate: ReviewGate`
- [x] 更新 `workspace_ws_types` 相关序列化测试。

建议先写失败测试：

```bash
cargo test --locked --lib workspace_stage_supports_review_decision_and_revision
```

必要时新增定向测试名：

```bash
cargo test --locked --lib review_complete_serializes_findings_and_review_gate
```

## Task 2: 后端解析与分类

- [x] 把 `parse_review_findings` 改为能区分：
  - findings 字段缺失
  - findings 数组为空
  - findings 有有效弱建议
  - findings 有强阻断
  - findings 字段存在但无法解析
- [x] 调整分类：
  - 强 finding -> `RequiresRevision`
  - `pass` 且无强 finding -> `UserConfirmAllowed`
  - `needs_human` -> `UserTriageRequired`
  - `revise` + 只有弱 finding -> `UserConfirmAllowed`
  - `revise` + 无 finding -> `UserTriageRequired`
  - JSON 不可解析 -> `UserTriageRequired`
- [x] `UserTriageRequired` 的归一化 `verdict` 使用 `NeedsHuman`。
- [x] `ArtifactVersion.review_verdict` 映射：
  - `RequiresRevision` -> `Revise`
  - `UserConfirmAllowed` -> `Pass` 或 `NeedsHuman`
  - `UserTriageRequired` -> `NeedsHuman`
- [x] 更新 reviewer prompt，明确 `verdict=revise` 必须带结构化 finding。

建议测试：

```bash
cargo test --locked --lib parse_review_verdict
cargo test --locked --lib optional_review_findings_enter_human_confirm_for_all_workspace_types
cargo test --locked --lib strong_review_findings_enter_review_decision_for_all_workspace_types
```

新增测试建议：

```bash
cargo test --locked --lib revise_without_findings_enters_user_triage_for_all_workspace_types
cargo test --locked --lib malformed_findings_enter_user_triage_for_all_workspace_types
```

## Task 3: 后端 human_confirm 返修入口复用

- [x] 确认 `handle_human_confirm(RequestChange)` 能消费 `payload.description`。
- [x] 若需要，扩展 `human_confirm_payload_description()` 支持（本轮确认前端统一发送 `{ "description": "..." }`，无需扩展 findings 降级）：
  - 字符串 payload
  - `{ "description": "..." }`
  - `{ "findings": [...] }` 的降级摘要
- [x] 确保 `request-change` 从 `human_confirm` 进入 `Revision`，并使用 `pending_revision_context`。
- [x] 补测试覆盖 `user_confirm_allowed` 下通过 `request-change` 采纳 optional findings 并启动 revision。

建议测试：

```bash
cargo test --locked --lib human_confirm_request_change
```

## Task 4: 前端类型与实时事件

- [x] `web/src/api/types.ts` 增加 `user_triage_required`。
- [x] 扩展 `ReviewComplete` 消息类型，加入 `findings` 与 `review_gate`。
- [x] `useWorkspaceWs.ts` 在 `review_complete` 中写入完整 metadata：
  - `verdict`
  - `comments`
  - `summary`
  - `round`
  - `findings`
  - `review_gate`
- [x] 补 `useWorkspaceWs.test.tsx`：实时 review 完成后无需 hydration 即可显示 findings 和正确 gate。

建议测试：

```bash
pnpm -C web exec vitest --run src/hooks/useWorkspaceWs.test.tsx
```

## Task 5: 前端 review 展示与按钮

- [x] `ReviewVerdictEntry`：
  - `requires_revision` -> “需要解决后再继续”
  - `user_confirm_allowed` -> “可确认当前版本”
  - `user_triage_required` -> “需要判断 reviewer 意图”
- [x] `GatePromptEntry`：
  - `user_confirm_allowed` 显示“确认使用当前版本”“采纳建议并返修”“终止”
  - `user_triage_required` 显示“按 reviewer 意见返修”“确认当前版本”“终止”
  - 组装 request-change payload.description，优先包含 finding 的 `message` 与 `required_action`
- [x] `ChatEntryList`、`ChatEntryRenderer`、`ChatWorkspacePage` 透传 request-change payload。
- [x] 保持现有 ChatInputBar 的人工修改意见入口可用。

建议测试：

```bash
pnpm -C web exec vitest --run src/components/chat-workspace/entries/p1-entries.test.tsx src/pages/ChatWorkspacePage.test.tsx
```

## Task 6: Store 恢复与兼容

- [x] `workspace-ws-store.ts` 的 `buildGatePromptEntry()` 透传：
  - `comments`
  - `findings`
  - `review_gate`
- [x] 从 `NodeDetail.verdict` hydration 重建 entry 时兼容三态 gate。
- [x] 旧数据没有 `review_gate` 时继续按 `user_confirm_allowed` 展示。
- [x] 增加恢复态测试：`user_triage_required` node detail 重建后显示人工裁决入口。

建议测试：

```bash
pnpm -C web exec vitest --run src/state/workspace-ws-store.test.ts
```

## Task 7: 集成验证

后端定向验证：

```bash
cargo test --locked --lib parse_review_verdict
cargo test --locked --lib revise_without_findings_enters_user_triage_for_all_workspace_types
cargo test --locked --lib optional_review_findings_enter_human_confirm_for_all_workspace_types
cargo test --locked --lib strong_review_findings_enter_review_decision_for_all_workspace_types
```

前端定向验证：

```bash
pnpm -C web exec vitest --run src/hooks/useWorkspaceWs.test.tsx src/state/workspace-ws-store.test.ts src/components/chat-workspace/entries/p1-entries.test.tsx src/pages/ChatWorkspacePage.test.tsx
```

全量门禁：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
pnpm -C web build
```

验证记录：

- [x] 后端定向验证通过：`parse_review_verdict`、`revise_without_findings_enters_user_triage_for_all_workspace_types`、`malformed_findings_enter_user_triage_for_all_workspace_types`、`optional_review_findings_enter_human_confirm_for_all_workspace_types`、`strong_review_findings_enter_review_decision_for_all_workspace_types`、`review_prompt_limits_revise_to_strong_findings`、`drive_review_session_pass_enters_human_confirm`、`handle_human_confirm_request_change_starts_revision`。
- [x] 前端定向验证通过：`useWorkspaceWs.test.tsx`、`workspace-ws-store.test.ts`、`p1-entries.test.tsx`、`ChatWorkspacePage.test.tsx`。
- [x] `cargo fmt --check` 通过。
- [x] `cargo check --locked` 通过。
- [x] `cargo test --locked` 通过。
- [x] `pnpm -C web build` 通过。
- [x] 开发服务健康检查通过：`http://127.0.0.1:4317/api/health`、`http://127.0.0.1:5173/`、`http://127.0.0.1:5173/api/health`。
- [x] Playwright 轻量页面加载检查通过：`http://127.0.0.1:5173/workbench` 显示 `Issue 生命周期工作台`。
- [x] `cargo clippy --all-targets --all-features --locked -- -D warnings` 通过；已处理 `src/product/test_executor.rs:391` 的 `clippy::question_mark`，并将 `drive_provider_session` 的多参数调用收敛为内部参数对象。

## Task 8: 真实 E2E 检查

至少复验一个真实 Work Item workspace：

- reviewer 输出 `pass + minor/optional` 时：
  - 页面进入 `human_confirm`
  - 显示“可确认当前版本”
  - 显示“采纳建议并返修”
  - 可直接确认当前版本
- 构造或等待 reviewer 输出 `revise` 但无 findings 时：
  - 页面进入 `human_confirm`
  - 显示“需要判断 reviewer 意图”
  - 显示“按 reviewer 意见返修”
  - 点击后进入 `revision`
- 构造强 finding 时：
  - 页面进入 `review_decision`
  - 仍显示现有三条返修决策路径

## 完成标准

- Story、Design、Work Item 三类 workspace 均覆盖三态 review gate。
- 实时 `review_complete` 和刷新恢复展示一致。
- 非结构化 `revise` 不再被文案表达成“可确认当前版本”。
- 非阻塞建议路径有显式采纳返修入口。
- 所有定向验证通过；全量门禁至少完成 Rust/前端关键命令并记录结果。
