# WorkItem 对话式 Workspace 生成拆分总览 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement each detailed WP plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking in detailed plans.

**Goal:** 将 `cadence/designs/2026-06-17_技术方案_WorkItem对话式Workspace生成_v1.0.md` 拆成多个单 session 可完成、前后端分离、可独立验证且最终能合成同一对话式 WorkItem 生成流程的实施计划。

**Architecture:** 先落 `WorkspaceType::WorkItemPlan` 枚举、空 Draft `IssueWorkItemPlan` prepare 入口、artifact payload union 与 WS 契约骨架，再实现非流式 WorkItemPlan author/review/revert+revision/confirm 各阶段，最后前端入口与候选面板，贯通测试收尾。LifecycleStore Draft candidate 是事实来源；artifact payload 是展示/恢复镜像，AuthorConfirm 的 revert meta 写回当前 `ArtifactVersion.payload`（不新增 version）以保证重连不丢；每个 WP 只改自己声明的写入范围，共享 `src/product/workspace_engine.rs` / `src/web/workspace_ws_handler.rs` 的 WP 必须严格串行。

**Tech Stack:** Rust 1.95.0、Cargo、Axum、tokio、React、TypeScript、Zustand、Vitest、OpenSpec、Superpowers。本计划不做 Playwright 浏览器 E2E。

**版本：** v1.3（v1.0 → v1.1 修订：将 WP2 拆分为 WP2a「artifact payload union 挂载」+ WP2b「WorkItemPlan author run + Draft candidate 持久化」，同步更新串行约束表与推荐执行顺序；文件名保留 v1.0 以维持既有引用路径稳定。v1.1 → v1.2 修订：① WP3 验证条目 `work_item_plan_review_revision_loop` 改为 `work_item_plan_review_returns_decision_response`（WP3 边界只到 ReviewDecision 响应，revision 重做在 WP4）；② WP4 新增 Task 4「迁移 WP2b AutoRevision 到 generate_revision」以对齐 design 第 269 行 validate 失败语义；③ WP3-WP8 详细计划文档已生成，更新引用标注。v1.2 → v1.3 修订：① prepare 响应统一为 `IssueWorkItemPlanDetailDto` + `WorkspaceSession.workspace_session_id`；② revert meta 写回当前 artifact version，重连恢复；③ revision 改为 provider 只生成 redo 项、后端合并 retained）

---

## 当前前置状态

- 工作目录：`.worktrees/feat-b-0616`
- 当前分支：`feat-b-0616`
- 设计方案：`cadence/designs/2026-06-17_技术方案_WorkItem对话式Workspace生成_v1.0.md`
- 取代：`cadence/plans/2026-06-17_实施计划_WorkItem对话式Workspace生成_方案B_v1.0.md`（方案 B v1.0，已废弃）
- 关键约束：
  - 本方案是 P3（WorkItem 拆分 REST 流程）的对话式演进，**替代而非并存**；3 条废弃 REST 路由在 WP5 删除。
  - 新增 `WorkspaceType::WorkItemPlan`，**复用** Story/Design 的 8 阶段状态机、WS 消息、Timeline 节点、恢复链路。
  - 唯一新增 WS 消息 `WsInMessage::RevertWorkItem`；`ArtifactUpdate` 加 `candidate` 字段；**不新增** `TimelineNodeType`、`WorkspaceStage`、`AdapterRole`。
  - prepare 阶段先创建空 Draft `IssueWorkItemPlan`，`session.entity_id = plan_id`；author/revision 从该 plan 读取 source ids/options。
  - author/revision 用 `WorkItemSplitter`（`spawn_blocking` 非流式），review 用 `Reviewer`（流式）；WorkItemPlan 的 author/revision 不走普通 `drive_provider_session` 流式路径。
  - LifecycleStore Draft candidate 是唯一事实来源；`ArtifactUpdate` / `SessionState.artifact` 扩展为 markdown 或 candidate 的 union。
  - AuthorConfirm 阶段逐个 revert，批量触发 Revision；revert = 重做（删一个重生补上，DAG 自动重连）；revert meta 持久化到当前 artifact version，断线重连后仍可继续批量 Revision。
  - `confirm` 后才建子 Coding session（draft 阶段只存 candidate）。
  - 运行时验证命令不硬编码（本计划不涉及目标项目验证命令，仅复用 P3 的 provider-based VerificationPlan）。

## 计划大小控制规则

- 单个详细 WP plan 必须能在一个实现 session 内完成；目标是实现者在读取 plan、读代码、写测试、写实现、验证和提交时仍保留充足上下文。
- 单个详细 WP plan 的实现范围建议控制在 30k-50k tokens 等价上下文内；如果需要同时阅读大量旧实现、跨后端和前端、或需要 6 个以上核心源码文件协同改动，必须继续拆分。
- 详细 WP plan 不允许同时承载后端实现、前端实现和贯通测试；前端、后端、贯通测试必须拆成不同 WP。
- 后序详细 WP plan 必须包含"前置交付摘要"章节，明确依赖哪些已完成 WP、需要读取哪些提交摘要、哪些接口已经稳定。
- 非依赖 WP 只有在写入范围互斥时才可并行；只要会修改同一文件、同一 store、同一 handler 或同一 UI 状态模块，就必须建立顺序依赖。
- 实现过程中如果发现当前 WP plan 实际超出单 session 范围，执行者必须停止扩大范围，先提交已完成的可验证子集，再产出下一份更小的计划。

## 拆分原则

- 后端模型/枚举、后端生成流程、后端 review、后端 revert+revision、后端 confirm+删路由、前端 UI、贯通测试分别成 WP，不混写。
- 每个 WP 都使用 TDD：先写失败测试，再写最小实现，再跑定向验证。
- 每个 WP 都必须说明 OpenSpec、Superpowers、TDD 和验证命令要求。
- 每个 WP 必须只修改自己声明的写入范围；若实现时发现需要越界修改，先更新本总览或新增 WP，不在当前 WP 内临时扩大范围。
- 依赖 WP 的开头必须提供前置交付摘要，避免后序 session 重新吞入前序完整上下文。
- 每个 WP 的验证链必须包含项目强制检查命令：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo check --locked`，外加该 WP 的定向测试；不允许只跑 `fmt + check` 而省略 clippy（详见 `cadence/project-rules/build-test-commands.md`）。

### 写入范围共享与串行约束

多个 WP 共享同一批源码文件，必须按依赖顺序严格串行，禁止并行修改同一文件：

| 共享文件 | 涉及 WP |
|---|---|
| `src/product/workspace_engine.rs` | WP2a、WP2b、WP3、WP4、WP5 |
| `src/web/workspace_ws_handler.rs` | WP2a、WP2b、WP4、WP5 |
| `src/web/workspace_context.rs` | WP1 |
| `src/web/workspace_ws_types.rs` | WP1、WP2a、WP4 |
| `src/web/handlers.rs` | WP1、WP5 |
| `src/web/app.rs` | WP1、WP5 |
| `src/product/work_item_split_engine.rs` | WP4 |
| `src/product/lifecycle_store.rs` | WP2b、WP4、WP5 |
| `src/product/models.rs` | WP1 |
| `web/src/api/types.ts` / `client.ts` | WP6、WP7 |

因此 **WP2a → WP2b → WP3 → WP4 → WP5 必须严格串行**（共享 `workspace_engine.rs` / `workspace_ws_handler.rs` / `lifecycle_store.rs`），不得并行。WP2a 是 WP2b 的前置（union 挂载是 author run 推送 candidate 的基础）。WP6 → WP7 串行（都改前端 API types/client）。

---

## WP1：后端 枚举 + context + prepare + WS 契约

**目标：** 落地 `WorkspaceType::WorkItemPlan` 枚举、`workspace_context.rs` 全部分支、`prepare_work_item_plan` handler 与路由、artifact payload union；使 prepare 能创建空 Draft `IssueWorkItemPlan`、创建 `WorkItemPlan` session（`entity_id = plan_id`）、注入上下文消息，且经 serde 往返不丢 candidate payload。

**依赖：** 无。

**前置交付摘要要求：** 无（首个 WP）。读取设计方案第 2、4、12、13 节。

**写入范围：**

- `src/product/models.rs`
- `src/web/workspace_context.rs`
- `src/web/workspace_ws_types.rs`（加 `ArtifactPayload` union、`WorkItemPlanCandidateDto`；`RevertWorkItem` 变体可在此 WP 预留或留到 WP4）
- `src/web/handlers.rs`（新 `prepare_work_item_plan` handler、`workspace_type_text` 分支；prepare 创建空 Draft `IssueWorkItemPlan` 后创建 session）
- `src/web/app.rs`（新 `POST .../work-item-plans:prepare` 路由）
- `tests/it_web.rs`
- `tests/it_web/web_work_item_generation.rs` 或新增 `web_work_item_plan_prepare.rs`
- `tests/it_product/product_workspace_context.rs`（若存在）或对应单测位置

**不做：**

- 不实现 `WorkspaceEngine` 的 author/review/revision/confirm 分支（WP2–WP5）。
- 不实现 `RevertWorkItem` 的处理逻辑（WP4，本 WP 只定义消息变体）。
- 不删任何 REST 路由（WP5）。
- 不改前端。

**验证：**

- `cargo test --locked --test it_web prepare_work_item_plan_creates_draft_plan_and_session_without_generating`
- `cargo test --locked --lib workspace_ws_types`
- `cargo test --locked --lib workspace_context`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`

**详细计划文档：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP1_后端枚举与context与prepare_v1.0.md`（已生成）

## WP2a：后端 artifact payload union 挂载

**目标：** 把 WP1 定义的 `ArtifactPayload` union 真正挂载到 artifact 链路：`WsOutMessage::ArtifactUpdate`、`SessionState.artifact`、`EngineEvent::ArtifactUpdate`、`WorkspaceSession.artifact`、`ArtifactVersion`、`ArtifactVersionSummary` 从 markdown `String` 切换到 `ArtifactPayload`（serde `untagged` + `flatten`，JSON 表现为设计方案 :339-348 要求的扁平 `markdown?/diff?/candidate?` 形态）。Story/Design/WorkItem 行为等价（`Markdown` 变体），`WorkItemPlanCandidate` 变体类型就位（WP2b 才产生 candidate 数据）。本 WP 是全 workspace type 基础设施改造，不涉及 WorkItemPlan 业务逻辑。

**依赖：** WP1（`ArtifactPayload` enum、`WorkItemPlanCandidateDto` 及子 DTO 已在 `workspace_ws_types.rs` 纯新增定义）。

**前置交付摘要要求：** 总结 WP1 的 `ArtifactPayload` enum 定义（`untagged`，`Markdown { markdown, diff }` / `WorkItemPlanCandidate { candidate }` 两变体）、`WorkItemPlanCandidateDto` 结构、`WsInMessage::RevertWorkItem` 变体；确认 WP1 未修改 `WsOutMessage::ArtifactUpdate` / `SessionState.artifact` / `ArtifactVersion`（保持 `String`，待本 WP 切换）。

**写入范围：**

- `src/web/workspace_ws_types.rs`（`WsOutMessage::ArtifactUpdate` 切 `{ version, #[serde(flatten)] payload: ArtifactPayload }`；`SessionState.artifact: Option<ArtifactPayload>`；`ArtifactVersion.markdown: String` → `payload: ArtifactPayload`；`ArtifactVersionSummary` 的 `markdown_size`/`markdown_preview` 保留字段名但按 payload 变体派生值，向后兼容前端旧契约）
- `src/product/workspace_engine.rs`（`WorkspaceSession.artifact: Option<ArtifactPayload>`；`EngineEvent::ArtifactUpdate { version, payload }`；`update_artifact` 签名改 `ArtifactPayload`；`build_artifact_version_summary` 按变体派生 size/preview；`build_session_state`；`new_persistent` 恢复；`complete_assistant_message` 把 markdown 包成 `ArtifactPayload::Markdown`；`handle_rollback`；`build_review_input` / `build_revision_input` 读 payload 的 markdown；所有 `session.artifact = Some("...")` 测试夹具迁移为 `Some(ArtifactPayload::Markdown { ... })`）
- `src/web/workspace_ws_handler.rs`（event forwarder `EngineEvent::ArtifactUpdate { version, payload }` → `WsOutMessage::ArtifactUpdate { version, payload }`）
- `tests/it_web.rs` 及受影响集成测试夹具

**不做：**

- 不实现 WorkItemPlan author run（WP2b）。
- 不实现 `replace_issue_work_item_plan_candidate`（WP2b）。
- 不产生 `ArtifactPayload::WorkItemPlanCandidate` 数据（WP2b 才产生）；本 WP 只保证 union 类型挂载 + `Markdown` 变体等价。
- 不改 `workspace_context.rs` / `handlers.rs` / `app.rs`（WP1 已完成）。
- 不改前端（WP6/WP7）。

**验证：**

- `cargo test --locked --lib workspace_engine`（现有 Story/Design/WorkItem 流程测试全绿）
- `cargo test --locked --lib workspace_ws_types`
- `cargo test --locked --test it_web`（现有 web 集成测试全绿）
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`

**详细计划文档：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP2a_后端artifact_union挂载_v1.0.md`（已生成）

## WP2b：后端 author 生成 + Draft candidate 持久化

**目标：** WorkItemPlan 在 `StartGeneration` 后走 dedicated 非流式 author run：从 `session.entity_id` 读取 Draft plan → 组装 `GenerateWorkItemsRequest` 兼容输入 → `WorkItemSplitEngine::generate` → `Validator::validate` → `LifecycleStore::replace_issue_work_item_plan_candidate` 持久化 Draft candidate → 写 `ArtifactPayload::WorkItemPlanCandidate` → 推 `ArtifactUpdate` → 进 AuthorConfirm。

**依赖：** WP1、WP2a。

**前置交付摘要要求：** 总结 WP1 的 `WorkspaceType::WorkItemPlan` 枚举、`workspace_context` 各分支签名、`prepare_work_item_plan` 创建的 Draft plan 字段（source ids/options/provider 配置）、`session.entity_id = plan_id` 约定、`WorkItemPlanCandidateDto` 结构；总结 WP2a 的 `ArtifactPayload` 挂载点（`session.artifact: Option<ArtifactPayload>`、`EngineEvent::ArtifactUpdate { version, payload }`、`update_artifact(payload: ArtifactPayload)`、`ArtifactVersion.payload`），WP2b 直接用 `ArtifactPayload::WorkItemPlanCandidate { candidate }` 推送。

**写入范围：**

- `src/web/workspace_ws_handler.rs`（`StartGeneration` 在 WorkItemPlan 下启动 dedicated non-streaming run，不启动 `ProviderRunKind::Author { content: "" }`；新增 `ProviderRunKind::WorkItemPlanAuthor`；`ProviderRunContext` 加 `provider_adapter` 字段）
- `src/product/workspace_engine.rs`（WorkItemPlan author run 的阶段推进、candidate payload 写入、`workspace_requires_artifact_gate`、`enter_author_confirm` 推进、`workspace_type_title`；不把 WorkItemPlan 走普通 markdown 完成判定）
- `src/product/lifecycle_store.rs`（新增 `replace_issue_work_item_plan_candidate`，替换 Draft plan 关联的 work_items / verification_plans / repository_profile 并更新 plan 引用；新增 `delete_verification_plan` / `delete_repository_profile` helper）
- `tests/it_web.rs`
- `tests/it_web/web_work_item_generation.rs` 或新增 `web_work_item_plan_author.rs`
- `tests/it_product.rs`
- `tests/it_product/product_lifecycle_store.rs`

**不做：**

- 不实现 review（WP3）。
- 不实现 revert / revision 局部重做（WP4）；author 阶段的"validate 失败自动 Revision"可在本 WP 实现最小版本（直接重生），完整 revision 局部重做留 WP4。
- 不实现 confirm 落盘（WP5）。
- 不改 `WorkItemSplitEngine` 内核（本 WP 只调用，不扩展）。
- 不改前端。

**验证：**

- `cargo test --locked --test it_web work_item_plan_start_generation_returns_candidate_artifact`
- `cargo test --locked --test it_web work_item_plan_start_generation_uses_non_streaming_split_run`
- `cargo test --locked --test it_product replace_issue_work_item_plan_candidate_updates_draft_records`
- `cargo test --locked --test it_web work_item_plan_author_persists_draft_candidate_records_without_child_sessions`
- `cargo test --locked --test it_web work_item_plan_validate_errors_auto_revision`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`

**详细计划文档：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP2b_后端author生成_v1.0.md`（已生成）

## WP3：后端 review 整组

**目标：** `build_review_input` 在 WorkItemPlan 分支下调用新 `build_work_item_plan_review_input`，从当前 Draft plan 关联记录组装整组 candidate，序列化后喂给 reviewer provider（流式），复用 verdict 解析 / `review_gate` / `ReviewDecisionResponse`。

**依赖：** WP1、WP2。

**前置交付摘要要求：** 总结 WP2 的 author 完成后 Draft candidate 落盘结构、AuthorConfirm 进入时机、从 `session.entity_id` 加载 candidate DTO 的 helper、artifact payload 推送方式。

**写入范围：**

- `src/product/workspace_engine.rs`（`build_review_input` WorkItemPlan 分支、新 `build_work_item_plan_review_input` 辅助函数；可涉及 `workspace_context.rs` 的 review prompt 片段但优先放 engine）
- `tests/it_web.rs`
- `tests/it_web/web_work_item_generation.rs` 或新增 `web_work_item_plan_review.rs`

**不做：**

- 不实现 revert / revision 局部重做（WP4）。
- 不实现 confirm（WP5）。
- 不改前端。

**验证：**

- `cargo test --locked --test it_web review_returns_verdict_for_whole_candidate`
- `cargo test --locked --test it_web work_item_plan_review_returns_decision_response`（v1.2 修订：原 `work_item_plan_review_revision_loop` 改名——WP3 边界只到 `ReviewDecisionResponse` 响应，完整 revision 重做循环由 WP4 实现、WP8 贯通）
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`

**详细计划文档：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP3_后端review整组_v1.0.md`（已生成）

## WP4：后端 revert + revision 局部重做

**目标：** 实现 `WsInMessage::RevertWorkItem` 标记处理（candidate meta 更新 + 写回当前 ArtifactVersion payload + 推同 version ArtifactUpdate）、批量触发 dedicated 非流式 WorkItemPlan Revision、`WorkItemSplitEngine::generate_revision`（retained + redo_specs）、`repatch_dependencies` DAG 重连；局部 revision 中 provider 只生成 redo 项，后端合并 retained 与 redo 后通过 `replace_issue_work_item_plan_candidate` 替换 Draft candidate，再写新 artifact payload。review 触发的整组 revision 也走本 WP 的 revision 路径。

**依赖：** WP1、WP2。

**前置交付摘要要求：** 总结 WP2 的 Draft candidate + artifact payload 结构（含 `work_items[i].meta`）、AuthorConfirm 阶段 engine 状态、`replace_issue_work_item_plan_candidate` 接口、`WorkItemSplitEngine::generate` 的输入输出签名。

**写入范围：**

- `src/web/workspace_ws_types.rs`（`WsInMessage::RevertWorkItem` 变体）
- `src/web/workspace_ws_handler.rs`（`RevertWorkItem` 阶段白名单、`message_type`、消息分发；`RequestRevision` 在 WorkItemPlan 下启动 dedicated revision run）
- `src/product/workspace_engine.rs`（revert 标记处理、WorkItemPlan Revision 阶段推进、candidate payload 推送）
- `src/product/work_item_split_engine.rs`（新 `generate_revision` 方法、`repatch_dependencies` 纯函数；不改 `generate` 主体）
- `src/product/lifecycle_store.rs`（复用或扩展 `replace_issue_work_item_plan_candidate`，确保 revision 不触碰 Confirmed plan/已有子 session）
- `tests/it_product/product_work_item_split_engine.rs`（`generate_revision` + `repatch_dependencies` 单测）
- `tests/it_web.rs`
- `tests/it_web/web_work_item_generation.rs` 或新增 `web_work_item_plan_revert.rs`

**不做：**

- 不实现 confirm（WP5）。
- 不改前端。

**验证：**

- `cargo test --locked --test it_product generate_revision_keeps_retained_and_redoes_marked`
- `cargo test --locked --test it_product repatch_dependencies_reconnects_dependents`
- `cargo test --locked --test it_web revert_work_item_is_valid_in_author_confirm_only`
- `cargo test --locked --test it_web revert_work_item_triggers_local_redo_in_revision`
- `cargo test --locked --test it_web revision_replaces_draft_candidate_without_touching_confirmed_records`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`

**详细计划文档：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP4_后端revert与revision_v1.0.md`（已生成）

## WP5：后端 confirm 落盘 + 子 session + 删废弃路由

**目标：** `handle_confirm` WorkItemPlan 分支根据 `session.entity_id` 调 `confirm_issue_work_item_plan` + 为每个 WorkItem 幂等创建子 Coding session；删除 3 条废弃 REST 路由与 handler。candidate 的 Draft 持久化已在 WP2/WP4 完成，本 WP 不再负责生成或替换 candidate。

**依赖：** WP1、WP2、WP3、WP4。

**前置交付摘要要求：** 总结 WP2 的 Draft candidate 落盘已建好哪些持久化记录（plan/work_items/verification_plans/repository_profile）、WP4 的 revision 替换语义、confirm 时还需要补建哪些子 session，以及 `session.entity_id = plan_id` 约定。

**写入范围：**

- `src/product/workspace_engine.rs`（`handle_confirm` WorkItemPlan 分支）
- `src/web/workspace_ws_handler.rs`（如 HumanConfirm 后需要触发 WorkItemPlan 专用状态刷新或阻止普通 revision run，必须在本 WP 收口）
- `src/product/lifecycle_store.rs`（confirm 时幂等创建子 session 的 helper，抽取自 `persist_work_item_split_provider_output` 的 session 创建部分）
- `src/web/handlers.rs`（删 `generate_work_items` / `confirm_issue_work_item_plan` / `request_issue_work_item_plan_change` handler、`build_generate_work_items_response` 及相关 DTO）
- `src/web/app.rs`（删 3 条废弃路由）
- `tests/it_web.rs`
- `tests/it_web/web_work_item_generation.rs` 或新增 `web_work_item_plan_confirm.rs`
- `tests/it_web/web_coding_attempt_api.rs`（若删除影响 attempt 流程的兼容性）

**不做：**

- 不改前端（前端入口在 WP6 改）。
- 不改 Coding Workspace 本身（P5/P6 已落地）。

**验证：**

- `cargo test --locked --test it_web confirm_creates_child_work_item_sessions`
- `cargo test --locked --test it_web confirm_uses_session_entity_plan_id`
- `cargo test --locked --test it_web delete_legacy_rest_routes_returns_404`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`

**详细计划文档：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP5_后端confirm与删路由_v1.0.md`（已生成）

## WP6：前端 入口 + API

**目标：** 新增 `prepareWorkItemPlan` API client 与类型；`WorkspaceSession.workspace_type` 支持 `"work_item_plan"`；`IssueLifecycleWorkbench` 入口从弹窗改为调 prepare 并打开 `ChatWorkspacePage`；移除 `setPendingWorkItemGenerate` 弹窗逻辑。

**依赖：** WP1（prepare API 契约）。

**前置交付摘要要求：** 总结 WP1 的 `POST /work-item-plans:prepare` 请求/响应契约、返回的 `work_item_plan.id` / `workspace_session.workspace_session_id`、`WorkspaceType::WorkItemPlan` 的前端序列化值 `"work_item_plan"`，并明确前端新增 `IssueWorkItemPlanDetailDto`，不复用旧 REST 的轻量 `IssueWorkItemPlan`。

**写入范围：**

- `web/src/api/types.ts`
- `web/src/api/types.test.ts`
- `web/src/api/client.ts`
- `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`

**不做：**

- 不实现 `WorkItemPlanCandidatePanel`（WP7）。
- 不改 `ChatWorkspacePage` 分支渲染（WP7）。
- 不改后端。

**验证：**

- `pnpm -C web test -- --run types`
- `pnpm -C web test -- --run IssueLifecycleWorkbench`
- `pnpm -C web build`

**详细计划文档：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP6_前端入口与API_v1.0.md`（已生成）

## WP7：前端 candidate 面板 + WS

**目标：** 新增 `WorkItemPlanCandidatePanel`（WorkItem 列表/DAG/RepositoryProfile/findings + 每 WorkItem revert 按钮 + 批量触发 + 确认按钮）；`ChatWorkspacePage` 按 `work_item_plan` 分支渲染该面板；WS store 处理 artifact payload union 并维护 `workItemPlanCandidate` 状态、新增 `sendRevertWorkItem`、复用 review/confirm/revision 收发。

**依赖：** WP6、以及后端 WP2–WP5 的 WS 契约。

**前置交付摘要要求：** 总结 WP2–WP5 的 WS 消息时序（artifact payload union、`RevertWorkItem`、review/confirm/revision 消息）、candidate DTO 结构、前端 Story/Design 现有 review/confirm 交互复用点。

**写入范围：**

- `web/src/api/types.ts`（`WorkItemPlanCandidateDto`、`RevertWorkItem` 消息类型，若 WP6 未覆盖）
- `web/src/state/workspace-ws-store.ts`（新增 `workItemPlanCandidate`，`artifact` 仅保存 markdown；`SessionState.artifact` 和 `ArtifactUpdate` 按 union 分流）
- `web/src/state/workspace-ws-store.test.ts`
- `web/src/hooks/useWorkspaceWs.ts`
- `web/src/hooks/useWorkspaceWs.test.tsx`
- `web/src/pages/ChatWorkspacePage.tsx`
- `web/src/pages/ChatWorkspacePage.test.tsx`
- `web/src/components/workspace/WorkItemPlanCandidatePanel.tsx`（新增）
- `web/src/components/workspace/WorkItemPlanCandidatePanel.test.tsx`（新增）

**不做：**

- 不改后端。
- 不写 Playwright E2E。

**验证：**

- `pnpm -C web test -- --run ChatWorkspacePage`
- `pnpm -C web test -- --run WorkItemPlanCandidatePanel`
- `pnpm -C web test -- --run workspace-ws-store`
- `pnpm -C web test -- --run useWorkspaceWs`
- `pnpm -C web build`

**详细计划文档：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP7_前端candidate面板_v1.0.md`（已生成）

## WP8：贯通测试

**目标：** 验证 prepare → author → revert → review → revision → confirm 的端到端关系；Fake provider 下全流程可走通；Story/Design/WorkItem/WorkItemPlan 四种 workspace type 的恢复链路影响评估完成；废弃路由 404。

**依赖：** WP1–WP7。

**前置交付摘要要求：** 总结 WP1–WP7 的 API、UI、WS 契约与状态机行为，只引用摘要与关键测试名，不重新吞入所有实现细节；明确 WorkItem 既有 Coding Workspace 链路是否受本次 ChatWorkspace artifact union 影响。

**写入范围：**

- `tests/it_web.rs`
- `tests/it_web/web_work_item_split_flow.rs`（或新增贯通测试文件）
- `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`
- `web/src/pages/ChatWorkspacePage.test.tsx`

**不做：**

- 不改生产后端代码，除非测试暴露真实缺陷；若需要改生产代码，先新增修复计划。
- 不改生产前端代码，除非测试暴露真实缺陷。
- 不新增 Playwright 浏览器 E2E。

**验证：**

- `cargo test --locked --test it_web work_item_plan_full_flow`
- `cargo test --locked --test it_web story_design_work_item_plan_recovery_consistency`
- `cargo test --locked --test it_web work_item_workspace_recovery_unaffected_or_covered`
- `pnpm -C web test -- --run IssueLifecycleWorkbench`
- `pnpm -C web test -- --run ChatWorkspacePage`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features --locked -- -D warnings`
- `cargo check --locked`

**详细计划文档：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP8_贯通测试_v1.0.md`（已生成）

## 推荐执行顺序

1. 执行 WP1，落地枚举、context、prepare、WS 契约类型（`ArtifactPayload` / `WorkItemPlanCandidateDto` / `RevertWorkItem`，纯新增不挂载）。
2. 执行 WP2a，把 artifact payload union 挂载到 `WsOutMessage::ArtifactUpdate` / `SessionState.artifact` / `EngineEvent::ArtifactUpdate` / `WorkspaceSession.artifact` / `ArtifactVersion`，Story/Design/WorkItem 行为等价。
3. 执行 WP2b，接入非流式 author 生成、Draft candidate 持久化（依赖 WP2a 的 union 挂载）。
4. 执行 WP3，接入 review 整组。
5. 执行 WP4，接入 revert 与 revision 局部重做。
6. 执行 WP5，接入 confirm 落盘、建子 session，删除废弃路由。
   - WP2a–WP5 共享 `src/product/workspace_engine.rs`、`src/web/workspace_ws_handler.rs`、`src/product/lifecycle_store.rs`，必须严格串行，不得并行。
7. 执行 WP6，前端入口与 API（依赖 WP1 契约，可与 WP2b–WP5 并行准备，但落地建议在 WP5 后以便端到端联调）。
8. 执行 WP7，前端 candidate 面板与 WS 收发。
   - WP6 → WP7 串行（都改 `web/src/api/types.ts` / `client.ts`）。
9. 最后执行 WP8，贯通测试验收。

## 验收标准

- Design Spec 点"生成下一阶段"后进入 `ChatWorkspacePage`（不再弹窗）。
- prepare 创建空 Draft `IssueWorkItemPlan`，`WorkItemPlan` session 的 `entity_id` 指向 plan id。
- Workspace 内显示上下文消息和"开始生成"入口。
- 点"开始生成" → WorkItemPlan dedicated non-streaming run 调 `WorkItemSplitEngine`，timeline 出现 `AuthorRun` 节点 → 一次性返回 candidate（WorkItem 列表 + DAG + RepositoryProfile + VerificationPlan + findings），并持久化 Draft plan/work_items/verification_plans/profile，不创建子 session。
- 候选以面板形式展示；每个 WorkItem 可 revert（附反馈），支持连续标记；点"重新生成被标记的" → Revision 替换 Draft candidate 并产出新 artifact version，被 revert 的重做、其余保留、DAG 自动重连。
- 若 `review_rounds > 0`：reviewer 流式审整组，返回 verdict + review decision；可在 Workspace 内确认或请求修改。
- `HumanConfirm::Confirm` → plan Confirmed + 为每个 WorkItem 建子 Coding session。
- `POST /work-items:generate`、`/work-item-plans/{id}/confirm`、`/change-request` 返回 404。
- Story / Design / WorkItem / WorkItemPlan 四种 workspace type 的 timeline/chat/provider 恢复影响评估完成；受共享链路影响的类型回归全绿，不受影响的 WorkItem/Coding 链路给出排除说明。
- `cargo fmt --check` / `cargo clippy --all-targets --all-features --locked -- -D warnings` / `cargo check --locked` / `cargo test --locked` / `pnpm -C web test` / `pnpm -C web build` 全绿。
