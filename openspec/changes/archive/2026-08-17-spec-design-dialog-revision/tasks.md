# Tasks: spec-design-dialog-revision

## 1. 协议层：AuthorDecision 变体与阶段守卫

- [x] 1.1 `src/web/workspace_ws_types/in_.rs`：`AuthorDecision` 新增 `Revise{feedback}`、`AcceptWithReview`、`AcceptFinalize` 变体；`Accept`/`Reject` 保留并标注兼容语义（含 serde 兼容测试）
- [x] 1.2 `src/web/workspace_ws_handler/protocol.rs`：AuthorConfirm 阶段守卫放行新变体；Reject 引导性错误由 engine 层 `handle_author_decision` 按 workspace_type 产出（协议守卫仅校验消息类型×阶段，不感知 workspace_type）
- [x] 1.3 WS 协议 roundtrip 与守卫单测（新测试文件，不放入既有 part_*.rs）

## 2. 引擎层：修订循环与确认双出口

- [x] 2.1 `src/product/workspace_engine/types.rs`：`AuthorDecisionOutcome` 追加 `StartRevision{feedback}`、`Finalized` 等新结果（追加，不改既有变体）
- [x] 2.2 `src/product/workspace_engine/decisions.rs`：`handle_author_decision` 追加 `Revise`/`AcceptWithReview`/`AcceptFinalize` 分支；`Revise` 走 `pending_revision_context` → Revision；`AcceptWithReview` 判定用 `reviewer_enabled_at_start == Some(false)`（不可用 `reviewer_provider.is_none()`，重连后失真），命中且 `provisional_reviewer_provider` 存在则恢复并置 `review_rounds=1`，无 provisional 返回引导性错误；旧记录（None）按有效态回退；`Accept` 兼容路由同理按 `reviewer_enabled_at_start`，`Reject` 引导错误在本层产出
- [x] 2.2b provisional 持久化闭环四处：`models/workspace.rs` `WorkspaceSessionRecord` 新增 `provisional_reviewer_provider: Option<ProviderName>` 与 `reviewer_enabled_at_start: Option<bool>`（均 `#[serde(default)]`）；`types.rs` `WorkspaceSession` 新增两字段并 `from_record` 恢复；`lifecycle.rs` `start_generation` 写入并扩展落盘调用（保证 WebSocket 重连后不丢失）；配套 serde 兼容单测（旧 record 无字段可反序列化）
- [x] 2.3 `src/product/workspace_engine/review/routing.rs`：Story/Design 类型下 review 完成路由统一回 AuthorConfirm，verdict 经 `format_review_feedback` 推送为对话消息
- [x] 2.4 `prompts/` 新增 `build_author_revision_prompt`（独立函数）：当前产物 + 用户反馈输入，要求输出修订版与「改动摘要」小节；不修改 `build_revision_full_prompt`/`build_revision_delta_prompt`
- [x] 2.5 `run/provider_run.rs` 接线：AuthorConfirm 反馈触发的修订 run 完成后回 AuthorConfirm，summary 携带改动摘要；接线走**新增 match 臂与独立函数**，不触碰 monorepo 分支已改的 `match run_kind` 既有臂与 ReviewOnly 分支
- [x] 2.6 引擎单测（新文件 `tests/author_revision_loop.rs`）：修订循环、空反馈拒绝、Reject 引导错误、双出口路由、旧变体 Accept 兼容路由（启用/未启用各一例）、review 后回 AuthorConfirm、reviewer pass 不自动定稿、provisional reviewer 恢复与无快照送审报错、重连后 provisional 不丢（from_record 路径）、修订 run 断线重连（完成/未完成两例）

## 3. 存量会话恢复

- [x] 3.1 存量迁移：落点为 `WorkspaceEngine::new_persistent` fallback 链（`src/product/workspace_engine/lifecycle.rs:282-300`，与既有恢复 fallback 并列；注意不是 `interrupted_run_recovery.rs`，其入口仅覆盖 stage==PrepareContext）：Story/Design 会话 stage 为 HumanConfirm/ReviewDecision 时迁移回 AuthorConfirm（保留产物与消息，review verdict 注入对话流）
- [x] 3.2 存量迁移单测（两个阶段的存量会话各一例）
- [x] 3.3 修订 run 断线恢复（扩展现有机制，非复用）：`interrupted_run_recovery.rs` 新增 `Revision` 恢复臂（枚举变体 + 失败修订 AuthorRun 节点检测 + `retry_interrupted_run` 新臂）、`decisions/inbound.rs` `provider_run_kind_for_interrupted_recovery` 映射到既有 `ProviderRunKind::Revision`；完成后重连路径无新机制（`provider_drive.rs:745-791` 已处理）；配套单测两例

## 4. 前端：ChatInputBar 三动作与消息渲染

- [x] 4.1 `ChatInputBar.tsx`：author_confirm 阶段输入框启用 + 「发送反馈/确认并送审/确认定稿」三动作；按 `reviewer_enabled` 默认高亮；移除「重新编写」按钮
- [x] 4.1b `ChatWorkspacePageParts.tsx` `providerConfigFor`：`reviewer_enabled=false` 时仍携带用户已选 reviewer（provisional 机制前置依赖，当前 :346-348 置 null 导致快照空转），且 `review_rounds` 与 `reviewerEnabled` 解耦（未启用时仍传 0），配套单测
- [x] 4.2 修订摘要与 review 报告消息渲染（复用 ReviewVerdictEntry 模式，新增改动摘要 entry）
- [x] 4.3 `ChatWorkspacePage.tsx` 决策发送接线与前端测试更新

## 5. 既有测试迁移与收尾

- [x] 5.1 迁移固化旧路径（Accept→CrossReview→ReviewDecision→HumanConfirm）的引擎/WS/前端用例到新状态机（仅调预期，不删覆盖）
- [x] 5.2 全量验证：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo check --locked`、`cargo test --locked`、`cd web && pnpm tsc -b && pnpm test`（按 cadence/project-rules/build-test-commands.md 标准四条 + 前端）
- [x] 5.3 whats-new 更新（如项目惯例要求）
