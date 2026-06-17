# WorkItem 对话式 Workspace 生成 WP5：后端 confirm 落盘 + 子 session + 删废弃路由 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `handle_confirm` WorkItemPlan 分支根据 `session.entity_id` 调 `LifecycleStore::confirm_issue_work_item_plan`（plan/work_items `Draft→Confirmed`）+ 为每个 WorkItem 幂等创建 `WorkspaceType::WorkItem` 子 Coding session；删除 3 条废弃 REST 路由（`/work-items:generate`、`/work-item-plans/{plan_id}/confirm`、`/work-item-plans/{plan_id}/change-request`）与对应 handler + DTO。candidate 的 Draft 持久化已在 WP2b/WP4 完成，本 WP 不再生成/替换 candidate。

**Architecture:** `HumanConfirm::Confirm` 在 WorkItemPlan 分支：① 从 `session.entity_id` 找 Draft `IssueWorkItemPlan`；② 调 `confirm_issue_work_item_plan`（现有，`lifecycle_store.rs:473`）—— plan.status `Draft→Confirmed`，关联 WorkItem `plan_status → Confirmed`；③ 此时才建子 Coding session：为每个 WorkItem 调 `create_workspace_session(WorkspaceType::WorkItem, entity_id=work_item.id)`；若 session 已存在则跳过（幂等，保证重试安全）；④ engine 返回本次新建子 session 列表，WS handler 层持有 `app_paths`，负责对这些子 session 调 `ensure_workspace_context_message` 注入 WorkItem 上下文。废弃路由删除：3 条路由 + 3 个 handler + `build_generate_work_items_response` 及相关 DTO；底层 `persist_work_item_split_provider_output`/`validate_work_item_generation_candidates`/`confirm_issue_work_item_plan` 的逻辑保留（迁入新 WS 流程，仅删 REST 包装）。

**Tech Stack:** Rust 1.95.0、Cargo、Axum、tokio、serde。本 WP 不涉及前端。

**版本：** v1.0
**创建日期：** 2026-06-17
**目标分支：** `feat-b-0616`
**工作区：** `.worktrees/feat-b-0616`
**依赖总览：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_拆分总览_v1.0.md`（v1.1，WP5 章节）
**设计方案：** `cadence/designs/2026-06-17_技术方案_WorkItem对话式Workspace生成_v1.0.md`（第 306-316 行 Confirm 与子 session、第 433-444 行废弃项删除）
**前置 WP：** WP1、WP2a、WP2b、WP3、WP4

---

## 全局约束（Global Constraints）

- **运行命令固定**：Rust 1.95.0；cargo 命令带 `--locked`；🔴 **禁止 `-j 1`**。
- **强制检查链**：`cargo fmt --check` + `cargo clippy --all-targets --all-features --locked -- -D warnings` + `cargo check --locked` + 定向测试，四者缺一不可。
- **TDD**：每个 Task 先写失败测试，再写实现。
- **写入范围严格**：只改「File Structure」声明的文件。本 WP 共享 `workspace_engine.rs` / `workspace_ws_handler.rs` / `lifecycle_store.rs` / `handlers.rs` / `app.rs`——须在 WP4 之后串行执行。
- **行号是参考**：基于 `feat-b-0616` HEAD `8a2eee4`；实现时以 `grep -n` 实际为准。
- **删除前搜索**：删除废弃路由前，全局搜 `work-items:generate` / `work-item-plans.*confirm` / `change-request` 确认前端无残留调用（前端入口在 WP6 改，本 WP 删后端路由；WP6 会在同一分支内落地，本 WP 删除时前端若仍有调用会暂时编译/运行失败——这是预期的，WP6 收口）。

---

## 前置交付摘要（来自 WP1 + WP2a + WP2b + WP3 + WP4）

### 来自 WP1
- `WorkspaceType::WorkItemPlan` 变体（serde `"work_item_plan"`）；`prepare_work_item_plan` handler 创建空 Draft `IssueWorkItemPlan` + `WorkItemPlan` session（`entity_id = plan_id`）。
- `workspace_context.rs` 全分支（含 WorkItem 的 `workspace_entity_context`，建子 session 时注入上下文用）。

### 来自 WP2a + WP2b
- Draft candidate 已落盘：plan（Draft）+ work_items（Draft）+ verification_plans + repository_profile，**无子 WorkItem session**（confirm 前只存 candidate）。
- `session.entity_id = plan_id`；`session.workspace_type = WorkItemPlan`。
- `WorkspaceEngine::handle_confirm` 的 `match workspace_type`（:2694）未加 WorkItemPlan 分支——本 WP 加。

### 来自 WP3 + WP4
- review/revert/revision 链路已就位；confirm 前 candidate 已最终化（Draft 状态）。
- WP4 的 revision 回 AuthorConfirm；用户点"确认计划" → `AuthorDecision::Accept` →（review_rounds>0 则 CrossReview，否则）HumanConfirm → `HumanConfirm::Confirm` 触发本 WP。

### 现有可复用内核（本 WP 调用，不改或最小改）
- `LifecycleStore::confirm_issue_work_item_plan`（`lifecycle_store.rs:473`）：plan.status `Draft→Confirmed`，关联 WorkItem `plan_status → Confirmed`。**本 WP 直接调用**。
- `LifecycleStore::create_workspace_session`（:868-901）：建子 WorkItem session。
- `ensure_workspace_context_message`（`workspace_context.rs:13`）：注入 WorkItem 上下文。
- `persist_work_item_split_provider_output`（`handlers.rs:589-703`）：**逻辑保留**（已迁入 WP2b 的 `replace_issue_work_item_plan_candidate`），本 WP 删其 REST 包装。

---

## 关键既有事实（避免重新探查）

所有行号基于 `feat-b-0616` HEAD `8a2eee4`，实现时用 `grep -n` 确认。

### `src/product/workspace_engine.rs`（8362 行）
- `handle_confirm`（:2683-2731）：处理 `HumanConfirm::Confirm`/`RequestChange`/`Terminate`。`match workspace_type`（:2694）当前有 Story/Design/WorkItem 分支，无 WorkItemPlan——WP5 加。
- `WorkspaceEngine` struct（:333-347）：持有 `lifecycle_store: Option<LifecycleStore>`。
- `transition_stage` / `create_timeline_node` / `append_completed_timeline_event` / `complete_active_node`：现有方法。
- `WorkspaceStage::Completed`、`WorkspaceSessionStatus`：现有。
- 文件顶部 `use`（:1-33）：已导入 `WorkspaceType`、`LifecycleStore`、`ArtifactPayload` 等。WP5 需补 `IssueStore`（若 engine 层建 session 需读 issue）、`WorkspaceType::WorkItem`（已导入）。

### `src/product/lifecycle_store.rs`（2054 行）
- `confirm_issue_work_item_plan(project_id, issue_id, plan_id) -> Result<IssueWorkItemPlan, ProductStoreError>`（:473-529）：read-modify-write，plan `Draft→Confirmed`，关联 work_items `plan_status → Confirmed`。
- `create_workspace_session(input: CreateWorkspaceSessionInput) -> Result<WorkspaceSessionRecord, _>`（:868-901）。
- `list_workspace_sessions(project_id, issue_id) -> Vec<WorkspaceSessionRecord>`：现有，用于幂等检查（已存在则跳过）。
- `get_issue_work_item_plan` / `list_work_items`：现有。
- **无** 幂等建子 session 的 helper——WP5 新增 `ensure_work_item_sessions_for_plan`（或直接在 engine 层循环调 `create_workspace_session` + 跳过已存在）。

### `src/web/handlers.rs`
- `generate_work_items`（:512-561）：废弃 REST handler，WP5 删。
- `confirm_issue_work_item_plan` handler（:845 附近）：废弃 REST handler（与 lifecycle_store 的同名方法不同——这是 HTTP handler），WP5 删。
- `request_issue_work_item_plan_change`（:860 附近）：废弃 REST handler，WP5 删。
- `build_generate_work_items_response` + 相关 DTO（:705-843）：WP5 删。
- `persist_work_item_split_provider_output`（:589-703）：**逻辑保留**（WP2b 已迁），本 WP 不删其本体（若仅被废弃 handler 调用，删 handler 后此函数成为 dead code——`cargo check` 会警告，届时删除或保留为 `#[allow(dead_code)]` 待 WP8 确认无其他引用）。**实现时先 `grep -rn "persist_work_item_split_provider_output" src/` 确认调用方**。
- `validate_work_item_generation_candidates`：**逻辑保留**，同上处理。
- `workspace_type_text`（:3063-3069）：WP1 已加 WorkItemPlan 分支，本 WP 不改。

### `src/web/app.rs`
- 路由 `POST /work-items:generate`（:79-82 附近）。
- 路由 `POST /work-item-plans/{plan_id}/confirm`（:83-86 附近）。
- 路由 `POST /work-item-plans/{plan_id}/change-request`（:87-90 附近）。
- 路由 `POST /work-item-plans:prepare`（WP1 新增，本 WP 保留）。

### `src/web/workspace_ws_handler.rs`
- `WsInMessage::HumanConfirm` 处理（`grep -n "WsInMessage::HumanConfirm\|handle_confirm" src/web/workspace_ws_handler.rs`）：调 `engine.handle_confirm`。WP5 若需在 handler 层做 WorkItemPlan 专用状态刷新，在此收口；一般 `handle_confirm` engine 层处理足够，handler 无需改。

### `src/web/types.rs`
- `GenerateWorkItemsRequest` / `GenerateWorkItemsResponse` / 相关 DTO（:564-594 区间）：若仅被废弃 handler 使用，删 handler 后成为 dead code，WP5 删除。**实现时 `grep -rn "GenerateWorkItemsRequest\|GenerateWorkItemsResponse" src/` 确认是否被 WP2b 的 `build_work_item_plan_generate_request` 复用**——WP2b 复用了 `GenerateWorkItemsRequest`（组装 author 输入），**不能删 `GenerateWorkItemsRequest`**；只删 `GenerateWorkItemsResponse` 与 `build_generate_work_items_response` 相关 DTO。

---

## File Structure

| 文件 | 操作 | 职责 / 本 WP 改动 |
|---|---|---|
| `src/product/workspace_engine.rs` | M | `handle_confirm` 加 WorkItemPlan 分支：`confirm_issue_work_item_plan` + 幂等建子 WorkItem session + 进 Completed；返回 confirm outcome，包含本次新建子 session |
| `src/product/lifecycle_store.rs` | M | 新增 `ensure_work_item_sessions_for_plan`（幂等建子 session helper，抽取自 `persist_work_item_split_provider_output` 的 session 创建部分） |
| `src/web/workspace_ws_handler.rs` | M | HumanConfirm Confirm 后读取 engine outcome，对本次新建 WorkItem 子 session 调 `ensure_workspace_context_message(&app_paths, &lifecycle, session)` 注入上下文 |
| `src/web/handlers.rs` | M | 删 `generate_work_items` / `confirm_issue_work_item_plan` handler / `request_issue_work_item_plan_change` handler / `build_generate_work_items_response` + 相关 DTO；处理 `persist_work_item_split_provider_output`/`validate_work_item_generation_candidates` 的 dead code |
| `src/web/app.rs` | M | 删 3 条废弃路由 |
| `src/web/types.rs` | M | 删 `GenerateWorkItemsResponse` 及仅被废弃 handler 使用的 DTO（**保留 `GenerateWorkItemsRequest`**——WP2b 复用） |
| `tests/it_web.rs` | M | 删除/迁移废弃路由的现有测试（`generate_work_items` / `confirm_issue_work_item_plan` REST 测试） |
| `tests/it_web/web_work_item_generation.rs` | M | 废弃 REST 测试迁移为 WS confirm 测试；保留夹具 `app_with_confirmed_story_and_design`/`valid_split_output`/`MockSplitProviderAdapter` |
| `tests/it_web/web_work_item_plan_confirm.rs` | N | confirm 集成测试（新增） |

**不改：**
- ❌ `src/web/workspace_context.rs`（WP1 已完成）
- ❌ `src/web/workspace_ws_types.rs`（WP1/WP2a 已完成）
- ❌ `src/product/work_item_split_engine.rs` / `work_item_split_validator.rs`（WP2b/WP4）
- ❌ 前端（WP6/WP7）

---

## Task 1：`handle_confirm` WorkItemPlan 分支 + 幂等建子 session

**目标**：`handle_confirm` 的 `match workspace_type` 加 WorkItemPlan 分支：① `confirm_issue_work_item_plan`（plan/work_items `Draft→Confirmed`）；② 为每个 WorkItem 幂等建 `WorkspaceType::WorkItem` 子 session（已存在则跳过）；③ 进 `Completed`。`RequestChange` → 进 Revision（WP4 已实现）；`Terminate` → session `Terminated`，draft candidate 保留可追溯但不 promote。

**Files:**
- Modify: `src/product/lifecycle_store.rs`（新 `ensure_work_item_sessions_for_plan`）
- Modify: `src/product/workspace_engine.rs`（`handle_confirm` WorkItemPlan 分支 + confirm outcome）
- Modify: `src/web/workspace_ws_handler.rs`（对 outcome 中的新建子 session 注入上下文）
- Test: `tests/it_web/web_work_item_plan_confirm.rs`（新增）+ `tests/it_web.rs` 注册

**Interfaces:**
- Consumes: `confirm_issue_work_item_plan`、`create_workspace_session`、`list_workspace_sessions`、`list_work_items`、`ensure_workspace_context_message`、`IssueStore::get`。
- Produces: `LifecycleStore::ensure_work_item_sessions_for_plan(project_id, issue_id, plan_id, session_config) -> Result<Vec<WorkspaceSessionRecord>, _>`；`handle_confirm` WorkItemPlan 分支；`WorkspaceConfirmOutcome::WorkItemPlan { child_sessions }`（或等价 enum/struct）。

- [ ] **Step 1.1：写失败测试 —— confirm 推进 plan Confirmed + 建子 WorkItem session**

在 `tests/it_web/web_work_item_plan_confirm.rs`（新增，`tests/it_web.rs` 加 `#[path]` mod 注册）。复用 `app_with_confirmed_story_and_design` + prepare + start_generation 夹具，推进到 HumanConfirm（author + 可选 review + AuthorDecision::Accept → HumanConfirm）。

```rust
#[tokio::test]
async fn confirm_creates_child_work_item_sessions() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;
    let session_id = prepare_and_start_generation(&app).await; // 推进到 author_confirm
    let ws = connect_ws(&app, &session_id).await;
    // AuthorDecision::Accept →（review_rounds=0 时不进 review）→ HumanConfirm
    ws.send(json!({"type":"author_decision","decision":"accept"}).to_string()).await;
    // 收到 stage_change → human_confirm
    recv_until_stage(&ws, "human_confirm", timeout).await;
    // HumanConfirm::Confirm
    ws.send(json!({"type":"human_confirm","action":"confirm"}).to_string()).await;
    let messages = recv_ws_messages_with_timeout(&ws, timeout).await;
    // 收到 stage_change → completed
    assert!(messages.iter().any(|m| m["type"] == "stage_change" && m["stage"] == "completed"));

    // 断言：plan Confirmed，每个 WorkItem 有子 WorkItem session
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(_repo.path().join(".aria")));
    let plan = lifecycle.get_issue_work_item_plan("project_0001", "issue_0001", &plan_id).unwrap();
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);
    let work_items = lifecycle.list_work_items("project_0001", "issue_0001").unwrap();
    let sessions = lifecycle.list_workspace_sessions("project_0001", "issue_0001").unwrap();
    for wi in &work_items {
        let has_session = sessions.iter().any(|s|
            s.workspace_type == WorkspaceType::WorkItem && s.entity_id == wi.id
        );
        assert!(has_session, "work_item {} should have a child WorkItem session", wi.id);
    }
}

#[tokio::test]
async fn confirm_uses_session_entity_plan_id() {
    // 断言 confirm 时 engine 从 session.entity_id 读 plan_id（而非从其他来源）
    // ... prepare + start_generation，记录 session.entity_id == plan_id ...
    // ... confirm 后断言被 confirm 的 plan.id == session.entity_id ...
}

#[tokio::test]
async fn confirm_is_idempotent_on_retry() {
    // 连续发两次 HumanConfirm::Confirm，断言不重复建子 session
    // （第二次 confirm 应跳过已建 session，或返回错误——取决于 engine 语义）
}
```

> 实现者注意：
> 1. `prepare_and_start_generation`/`connect_ws`/`recv_until_stage`/`recv_ws_messages_with_timeout` helper：复用 WP2b 共享 WS helper；若缺能力先补 helper。本 Task 需要覆盖 HumanConfirm WS 路由与 handler 层子 session 上下文注入，不能用 engine 层测试替代。
> 2. `review_rounds=0` 时 AuthorDecision::Accept 直接进 HumanConfirm（不进 CrossReview）。prepare 时设 `review_rounds: 0` 跳过 review，简化测试。
> 3. `plan_id` 从 prepare 响应取。
> 4. `IssueWorkItemPlanStatus`/`WorkspaceType`/`LifecycleStore`/`ProductAppPaths` 在 test mod `use`。

- [ ] **Step 1.2：运行测试，确认失败**

Run: `cargo test --locked --test it_web confirm_creates_child_work_item_sessions`
Expected: 失败——`handle_confirm` 无 WorkItemPlan 分支，confirm 不推进 plan、不建子 session。

- [ ] **Step 1.3：实现 `ensure_work_item_sessions_for_plan`**

`src/product/lifecycle_store.rs`，在 `confirm_issue_work_item_plan`（:473-529）附近新增。幂等：已存在 session 则跳过。

```rust
/// 为 plan 关联的每个 WorkItem 幂等创建 WorkspaceType::WorkItem 子 session。
///
/// 在 HumanConfirm::Confirm 时调用。若 WorkItem 已有子 session（重试场景），跳过。
/// 返回新建的 session 列表（已存在的跳过不计入）。
pub fn ensure_work_item_sessions_for_plan(
    &self,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    author_provider: ProviderName,
    reviewer_provider: Option<ProviderName>,
    review_rounds: u32,
    superpowers_enabled: bool,
    openspec_enabled: bool,
) -> Result<Vec<WorkspaceSessionRecord>, ProductStoreError> {
    let plan = self.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
    let existing_sessions = self.list_workspace_sessions(project_id, issue_id)?;
    let mut created = Vec::new();
    for wi_id in &plan.work_item_ids {
        // 幂等：已存在 WorkItem session 则跳过
        let already_exists = existing_sessions.iter().any(|s|
            s.workspace_type == WorkspaceType::WorkItem && s.entity_id == *wi_id
        );
        if already_exists {
            continue;
        }
        let session = self.create_workspace_session(CreateWorkspaceSessionInput {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            entity_id: wi_id.clone(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider,
            reviewer_provider,
            review_rounds,
            superpowers_enabled,
            openspec_enabled,
        })?;
        created.push(session);
    }
    Ok(created)
}
```

> 实现者注意：
> 1. `CreateWorkspaceSessionInput` 字段以 `lifecycle_store.rs:868-901` 实际定义为准——`grep -n "struct CreateWorkspaceSessionInput" src/product/lifecycle_store.rs`。上面字段参考 WP1 prepare handler 的调用。
> 2. 子 session 的 provider 配置（author_provider/reviewer_provider/review_rounds/superpowers/openspec）从 WorkItemPlan session 继承——`handle_confirm` 调用时从 `self.session` 读这些字段传入。
> 3. **建子 session 后注入 WorkItem 上下文消息**：`ensure_workspace_context_message`（`workspace_context.rs:13`）需要 `ProductAppPaths`。当前 `WorkspaceEngine` 不持有 `app_paths`，因此本 WP 选定方案：`LifecycleStore::ensure_work_item_sessions_for_plan` 只建 session；`WorkspaceEngine::handle_confirm` 返回本次新建子 session；`workspace_ws_handler.rs` 在 HumanConfirm Confirm 返回后用已有 `app_paths` 调 `ensure_workspace_context_message`。不要给 `WorkspaceEngine` 新增 `app_paths` 字段。

- [ ] **Step 1.4：实现 `handle_confirm` WorkItemPlan 分支**

`src/product/workspace_engine.rs:2683-2731`，`match workspace_type`（:2694）加 WorkItemPlan 分支。参考现有 WorkItem 分支的结构（`grep -n "WorkspaceType::WorkItem =>" src/product/workspace_engine.rs` 看 WorkItem 的 confirm 分支模式）。

```rust
            WorkspaceType::WorkItemPlan => {
                let lifecycle = self.lifecycle_store.clone().ok_or("lifecycle_store unavailable")?;
                let project_id = self.session.project_id.clone();
                let issue_id = self.session.issue_id.clone();
                let plan_id = self.session.entity_id.clone();
                match action {
                    HumanConfirmAction::Confirm => {
                        // 1. plan/work_items Draft → Confirmed
                        lifecycle.confirm_issue_work_item_plan(&project_id, &issue_id, &plan_id)
                            .map_err(|e| format!("confirm plan failed: {e}"))?;

                        // 2. 幂等建子 WorkItem session
                        let new_sessions = lifecycle.ensure_work_item_sessions_for_plan(
                            &project_id, &issue_id, &plan_id,
                            self.session.author_provider.clone(),
                            self.session.reviewer_provider.clone(),
                            self.session.review_rounds,
                            self.session.superpowers_enabled,
                            self.session.openspec_enabled,
                        ).map_err(|e| format!("ensure child sessions failed: {e}"))?;

                        // 3. 进 Completed
                        self.transition_stage(WorkspaceStage::Completed).await;
                        if let Some(store) = &self.lifecycle_store {
                            let _ = store.update_workspace_session_status(
                                &self.session.session_id,
                                WorkspaceSessionStatus::Completed,
                            );
                        }
                        let _ = self.create_timeline_node(TimelineNodeDraft {
                            node_type: TimelineNodeType::HumanConfirm,
                            agent: None,
                            stage: WorkspaceStage::Completed,
                            round: None,
                            title: "WorkItemPlan 已确认".to_string(),
                            summary: Some(format!("plan {} confirmed，已建立 {} 个子 WorkItem session", plan_id, new_sessions.len())),
                            status: TimelineNodeStatus::Completed,
                        }).await;

                        return Ok(WorkspaceConfirmOutcome::WorkItemPlan {
                            child_sessions: new_sessions,
                        });
                    }
                    HumanConfirmAction::RequestChange => {
                        // 回 Revision（用户补充反馈）—— WP4 已实现 WorkItemPlanRevision
                        // engine 层触发 revision（参考 handle_review_decision 的 StartRevision 路径）
                        // 实现时复用 WP4 的 revision 触发逻辑
                    }
                    HumanConfirmAction::Terminate => {
                        // session Terminated，draft candidate 保留可追溯但不 promote
                        self.transition_stage(WorkspaceStage::Completed).await; // 或 Terminated 专用 stage
                        if let Some(store) = &self.lifecycle_store {
                            let _ = store.update_workspace_session_status(
                                &self.session.session_id,
                                WorkspaceSessionStatus::Terminated,
                            );
                        }
                    }
                }
            }
```

> 实现者注意：
> 1. `HumanConfirmAction` 枚举名以实际为准——`grep -n "enum HumanConfirmAction\|HumanConfirm" src/product/workspace_engine.rs src/web/workspace_ws_types.rs`。可能是 `HumanConfirmAction::{Confirm, RequestChange, Terminate}` 或 `HumanConfirm::{Confirm, RequestChange, Terminate}`。
> 2. `WorkspaceEngine` 当前不持有 `app_paths`；不要在 engine 内调用 `ensure_workspace_context_message`。把 `handle_confirm` 签名改为返回 `Result<WorkspaceConfirmOutcome, String>`（或等价类型），普通 Story/Design/WorkItem 返回 `WorkspaceConfirmOutcome::None`，WorkItemPlan Confirm 返回新建子 session。
> 3. `WorkspaceSessionStatus::{Completed, Terminated}`、`TimelineNodeDraft`/`TimelineNodeType`/`TimelineNodeStatus` 以现有定义为准。
> 4. `RequestChange` 触发 revision：参考 WP4 的 `RequestRevision` 路由——但 `handle_confirm` 的 `RequestChange` 在 engine 层，可能需返回一个信号让 handler 层 spawn `WorkItemPlanRevision`。**实现时看现有 Story/Design 的 `handle_confirm RequestChange` 如何触发 revision**（`grep -n "RequestChange\|StartRevision" src/product/workspace_engine.rs`），照搬其模式。
> 5. `confirm_issue_work_item_plan` 要求 plan 必须 `Draft`——若 plan 已 Confirmed（重复 confirm），返回错误。`confirm_is_idempotent_on_retry` 测试需处理：第二次 confirm 应在 engine 层被拦（plan 已 Confirmed → 错误）或跳过。**实现时决定语义**：重复 confirm 返回错误（更安全），测试 `confirm_is_idempotent_on_retry` 断言第二次返回错误且不重复建 session。
> 6. `workspace_ws_handler.rs` 的 `WsInMessage::HumanConfirm` 分支在 `engine.handle_confirm(...).await` 后，若 outcome 是 `WorkItemPlan { child_sessions }`，循环调用 `ensure_workspace_context_message(&app_paths, &lifecycle, session)`。失败时返回/发送 protocol error，不要静默吞掉，否则子 Coding session 会缺上下文。

- [ ] **Step 1.5：运行 Task 1 测试 + 收口**

Run:
```
cargo test --locked --test it_web confirm_creates_child_work_item_sessions
cargo test --locked --test it_web confirm_uses_session_entity_plan_id
cargo test --locked --test it_web confirm_is_idempotent_on_retry
cargo test --locked --lib workspace_engine
cargo check --locked
```
Expected: 新测试 PASS；现有 engine 测试全绿；`cargo check` 全绿。

- [ ] **Step 1.6：提交**

```bash
git add src/product/workspace_engine.rs src/product/lifecycle_store.rs tests/it_web/
git commit -m "feat(WP5): handle_confirm WorkItemPlan 分支 + 幂等建子 WorkItem session"
```

---

## Task 2：删除 3 条废弃 REST 路由 + handler + DTO

**目标**：删除 `POST /work-items:generate`、`POST /work-item-plans/{plan_id}/confirm`、`POST /work-item-plans/{plan_id}/change-request` 路由及对应 handler + `build_generate_work_items_response` + 相关 DTO。底层 `persist_work_item_split_provider_output`/`validate_work_item_generation_candidates`/`confirm_issue_work_item_plan`（lifecycle_store 方法）逻辑保留。

**Files:**
- Modify: `src/web/app.rs`（删 3 条路由）
- Modify: `src/web/handlers.rs`（删 `generate_work_items` / `confirm_issue_work_item_plan` handler / `request_issue_work_item_plan_change` handler / `build_generate_work_items_response` + 相关 DTO；处理 dead code）
- Modify: `src/web/types.rs`（删 `GenerateWorkItemsResponse` 及仅被废弃 handler 使用的 DTO；**保留 `GenerateWorkItemsRequest`**——WP2b 复用）
- Modify: `tests/it_web.rs` / `tests/it_web/web_work_item_generation.rs`（删除/迁移废弃 REST 测试）

**Interfaces:**
- Consumes: 无。
- Produces: 3 条路由返回 404；废弃 handler/DTO 清理。

- [ ] **Step 2.1：写失败测试 —— 废弃路由返回 404**

在 `tests/it_web/web_work_item_plan_confirm.rs` 末尾：

```rust
#[tokio::test]
async fn delete_legacy_rest_routes_returns_404() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (status, _) = request_json(app.clone(), Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({"title":"x","story_spec_ids":["story_spec_0001"],"design_spec_ids":["design_spec_0001"]})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = request_json(app.clone(), Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/some_plan/confirm",
        json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = request_json(app.clone(), Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/some_plan/change-request",
        json!({"feedback":"x"})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
```

> 实现者注意：路由路径以 `app.rs` 实际注册为准——`grep -n "work-items:generate\|work-item-plans.*confirm\|change-request" src/web/app.rs`。可能不带 `/api` 前缀（取决于 app.rs 的统一前缀）。先确认实际路径再调整测试。

- [ ] **Step 2.2：运行测试，确认失败**

Run: `cargo test --locked --test it_web delete_legacy_rest_routes_returns_404`
Expected: 失败——路由仍存在，返回 200/400 而非 404。

- [ ] **Step 2.3：删 3 条路由**

`src/web/app.rs`，删除：
- `POST .../work-items:generate`（:79-82 附近）
- `POST .../work-item-plans/{plan_id}/confirm`（:83-86 附近）
- `POST .../work-item-plans/{plan_id}/change-request`（:87-90 附近）

```rust
// 删除这 3 条 .route(...) 注册
```

> 保留 `POST .../work-item-plans:prepare`（WP1 新增）。

- [ ] **Step 2.4：删废弃 handler + DTO**

`src/web/handlers.rs`：
- 删 `generate_work_items`（:512-561）
- 删 `confirm_issue_work_item_plan` handler（:845 附近，HTTP handler，与 lifecycle_store 方法同名但不同实体）
- 删 `request_issue_work_item_plan_change`（:860 附近）
- 删 `build_generate_work_items_response`（:705-843）+ 相关 DTO（仅被这些 handler 使用的）

`src/web/types.rs`：
- 删 `GenerateWorkItemsResponse` 及仅被废弃 handler 使用的 DTO。
- **保留 `GenerateWorkItemsRequest`**——`grep -rn "GenerateWorkItemsRequest" src/` 确认 WP2b 的 `build_work_item_plan_generate_request` 复用了它。

> ⚠️ `persist_work_item_split_provider_output`（:589-703）与 `validate_work_item_generation_candidates`：**先 `grep -rn "persist_work_item_split_provider_output\|validate_work_item_generation_candidates" src/ tests/` 确认调用方**。
> - 若仅被废弃 handler 调用 → 删 handler 后成为 dead code → 删除这两个函数（其逻辑已迁入 WP2b 的 `replace_issue_work_item_plan_candidate` + `WorkItemSplitValidator::validate`）。
> - 若被其他地方（如 WP2b/WP4 的 engine/handler）复用 → 保留。
> - **预期**：`persist_work_item_split_provider_output` 的逻辑已迁，应可删除；`validate_work_item_generation_candidates` 若只是 `WorkItemSplitValidator::validate` 的薄包装，也可删。`cargo check` 的 dead_code 警告会指引。

- [ ] **Step 2.5：迁移/删除废弃 REST 的现有测试**

`tests/it_web/web_work_item_generation.rs`：现有 `generate_work_items` / `confirm_issue_work_item_plan` REST 测试（如 `generate_work_items_creates_plan_and_work_items`、`confirm_issue_work_item_plan_marks_work_items_confirmed`）会因路由删除而失败。**处理**：
- 删除这些 REST 测试（其覆盖已被 WP2b 的 author 测试 + WP5 Task 1 的 confirm 测试替代）。
- 保留夹具 `app_with_confirmed_story_and_design`/`valid_split_output`/`MockSplitProviderAdapter`（WP2b/WP3/WP4/WP5 复用）。
- `grep -n "generate_work_items\|confirm_issue_work_item_plan\|change_request" tests/it_web/` 定位所有需删除/迁移的测试。

> 实现者注意：`confirm_issue_work_item_plan_marks_work_items_confirmed`（若存在于 `tests/it_product/product_lifecycle_store.rs`）是 **lifecycle_store 方法的单测**，不是 REST 测试——**保留**（lifecycle_store 的 `confirm_issue_work_item_plan` 方法未删）。只删 `tests/it_web/` 里走 HTTP 路由的测试。

- [ ] **Step 2.6：运行 Task 2 测试 + 收口**

Run:
```
cargo test --locked --test it_web delete_legacy_rest_routes_returns_404
cargo test --locked --test it_web
cargo test --locked --test it_product product_lifecycle_store
cargo check --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```
Expected: 404 测试 PASS；现有 it_web 测试全绿（废弃 REST 测试已删/迁移）；`cargo check` 无 dead_code 警告（废弃 handler/DTO 已清理）；clippy 全绿。

> 若 `cargo check` 报 dead_code 警告（`persist_work_item_split_provider_output` 等），删除对应函数或加 `#[allow(dead_code)]` 临时保留（优先删除，逻辑已迁）。

- [ ] **Step 2.7：提交**

```bash
git add src/web/app.rs src/web/handlers.rs src/web/types.rs tests/it_web/
git commit -m "chore(WP5): 删除 3 条废弃 REST 路由与 handler/DTO（对话式流程取代）"
```

---

## Task 3：WP5 收口验证（全量回归）

**目标**：跑完整验证链，确保 confirm + 删路由未破坏 Story/Design/WorkItem 既有流程；WorkItemPlan prepare→author→review→revert→revision→confirm 链路通；废弃路由 404。

**Files:** 无新增改动；仅运行验证命令。

- [ ] **Step 3.1：全量验证链**

Run（依次，任一失败即停并修复）:
```
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked --lib workspace_engine
cargo test --locked --test it_product
cargo test --locked --test it_web
```
Expected: 全绿。

> `cargo test --locked --test it_web` 全量覆盖 Story/Design/WorkItem/WorkItemPlan 全流程（prepare/author/review/revert/revision/confirm + 废弃路由 404），是 WP5 最大的回归保障。
> `cargo test --locked --test it_product` 覆盖 lifecycle_store 的 `confirm_issue_work_item_plan`/`ensure_work_item_sessions_for_plan` 单测。

- [ ] **Step 3.2：确认 WP1-WP4 成果未破坏**

Run:
```
cargo test --locked --test it_web prepare_work_item_plan_creates_draft_plan_and_session_without_generating
cargo test --locked --test it_web work_item_plan_start_generation_returns_candidate_artifact
cargo test --locked --test it_web review_returns_verdict_for_whole_candidate
cargo test --locked --test it_web revert_work_item_triggers_local_redo_in_revision
cargo test --locked --test it_web confirm_creates_child_work_item_sessions
cargo test --locked --test it_web delete_legacy_rest_routes_returns_404
```
Expected: 全 PASS。

- [ ] **Step 3.3：交付摘要（供 WP6/WP7/WP8 前置交付摘要使用）**

commit 后，把以下内容写入后续 WP plan 的「前置交付摘要」章节：

- WorkItemPlan confirm 链路：`HumanConfirm::Confirm` → `handle_confirm` WorkItemPlan 分支 → `confirm_issue_work_item_plan`（plan/work_items `Draft→Confirmed`）+ `ensure_work_item_sessions_for_plan`（幂等建子 WorkItem session，继承 provider 配置）→ 返回 child sessions → WS handler 调 `ensure_workspace_context_message` 注入 WorkItem 上下文 → `Completed`。
- 子 WorkItem session：`WorkspaceType::WorkItem`，`entity_id = work_item.id`，provider 配置从 WorkItemPlan session 继承。
- 3 条废弃 REST 路由已删（404）：`/work-items:generate`、`/work-item-plans/{plan_id}/confirm`、`/work-item-plans/{plan_id}/change-request`。废弃 handler/DTO 已清理；`GenerateWorkItemsRequest` 保留（WP2b 复用）；`persist_work_item_split_provider_output`/`validate_work_item_generation_candidates` 若成 dead code 已删。
- WorkItemPlan 全后端链路（WP1-WP5）就绪：prepare → author → AuthorConfirm（revert 标记）→ revision / review → HumanConfirm → confirm 建子 session。
- **WP6 待办**：前端入口从弹窗改为 `prepareWorkItemPlan` + 打开 `ChatWorkspacePage`；移除 `setPendingWorkItemGenerate` 弹窗逻辑；新增 `prepareWorkItemPlan` API client + 类型。
- **WP7 待办**：`WorkItemPlanCandidatePanel` + `ChatWorkspacePage` 分支 + WS store 处理 artifact payload union + `sendRevertWorkItem`。
- **WP8 待办**：贯通测试 prepare→author→revert→review→revision→confirm + 四种 workspace type 恢复链路评估 + 废弃路由 404。

---

## Self-Review（写完后的自查）

**1. Spec 覆盖**（对照总览 v1.1 WP5 目标/写入范围/验证 + 设计方案 :306-316、:433-444）：
- ✅ `handle_confirm` WorkItemPlan 分支调 `confirm_issue_work_item_plan` → Task 1 Step 1.4
- ✅ 为每个 WorkItem 幂等建子 Coding session → Task 1 Step 1.3/1.4
- ✅ 删除 3 条废弃 REST 路由 → Task 2 Step 2.3
- ✅ 删除 handler + DTO → Task 2 Step 2.4
- ✅ `session.entity_id = plan_id` 约定 → Task 1 Step 1.4（从 `self.session.entity_id` 读 plan_id）
- ✅ candidate Draft 持久化已在前序 WP 完成，本 WP 不生成/替换 → Task 1 只 confirm + 建 session
- ✅ 验证命令链 → Task 3
- ✅ 不做项：未改前端（WP6）、未改 Coding Workspace 本身——均在「不做」清单。

**2. Placeholder 扫描**：
- `HumanConfirmAction` 枚举名（Task 1 Step 1.4）：给出 grep 定位指引，以实际为准。属可接受。
- `app_paths` 依赖（Task 1 Step 1.4）：已明确不进 `WorkspaceEngine`，由 WS handler 层注入 WorkItem 子 session 上下文。属可接受。
- `RequestChange` 触发 revision（Task 1 Step 1.4）：给出"参考现有 Story/Design 的 RequestChange 模式"指引。属可接受。
- `persist_work_item_split_provider_output`/`validate_work_item_generation_candidates` 的 dead code 处理（Task 2 Step 2.4）：给出 grep 确认调用方 + 删除/保留决策路径。属可接受。
- WS 测试 helper（Task 1 Step 1.1）：复用 WP2b 共享 helper；若能力不足先补 helper。属可接受。

**3. 类型一致性**：
- `ensure_work_item_sessions_for_plan` 签名在 Task 1 Step 1.3 定义，`handle_confirm` 调用一致。
- `confirm_issue_work_item_plan`（lifecycle_store 方法，:473）与本 WP 删除的 `confirm_issue_work_item_plan` handler（HTTP）同名不同实体——Task 2 删 handler，Task 1 调 lifecycle_store 方法，两者不冲突。
- `GenerateWorkItemsRequest` 保留（WP2b 复用），`GenerateWorkItemsResponse` 删除——Task 2 Step 2.4 明确区分。
- 子 WorkItem session 的 `workspace_type = WorkItem`、`entity_id = work_item.id`、provider 配置从 WorkItemPlan session 继承——与 design :312 一致。

**4. 边界风险**：
- **重复 confirm 的幂等性**（Task 1 Step 1.4）：`confirm_issue_work_item_plan` 要求 plan Draft，重复 confirm 会失败。`ensure_work_item_sessions_for_plan` 幂等跳过已存在 session。测试 `confirm_is_idempotent_on_retry` 需明确语义（第二次 confirm 返回错误 vs 跳过）。已标注，实现时决定（建议返回错误，更安全）。
- **`app_paths` 来源**（Task 1 Step 1.4）：已定为 handler 层持有并使用，不给 engine 增加 `app_paths`。风险点变为 handler 忘记处理 `WorkspaceConfirmOutcome::WorkItemPlan`；Task 1 Step 1.4/实现者注意已要求失败时发送 protocol error。
- **dead code 清理**（Task 2 Step 2.4）：`persist_work_item_split_provider_output`/`validate_work_item_generation_candidates` 删除前需确认无其他引用。`cargo check` dead_code 警告兜底。已标注。
- **前端残留调用**（全局约束）：删路由后前端若仍调用会暂时失败。WP6 在同分支收口前端入口。本 WP 删除时前端调用会断——这是预期的（WP6 修复）。**建议执行顺序**：WP5 删路由前先确认 WP6 会紧接着落地，或 WP5/WP6 在同一批次提交。已标注。
- **`RequestChange` 触发 revision 的 engine/handler 协调**（Task 1 Step 1.4）：`handle_confirm` 的 `RequestChange` 在 engine 层，但 revision run 由 handler 层 spawn。需 engine 返回信号让 handler spawn `WorkItemPlanRevision`。参考现有 Story/Design 的 `RequestChange` 模式。已标注。
- **子 session 上下文注入**（Task 1 Step 1.3/1.4）：已定为 handler 层注入，保持 lifecycle_store 不依赖 workspace_context，也避免给 `WorkspaceEngine` 新增 `app_paths`。已标注。

---

## Execution Handoff

本 WP5 plan 已保存至 `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP5_后端confirm与删路由_v1.0.md`。

**执行方式（待用户选择）：**

1. **Subagent-Driven（推荐）** — 每个 Task 派一个 fresh subagent 实现，Task 间做 two-stage review。
2. **Inline Execution** — 在当前 session 按 superpowers:executing-plans 批量执行，带 checkpoint 审查。

**完成后**：交付 WP5 后，后端全链路（WP1-WP5）就绪。继续 WP6（前端入口 + API，依赖 WP1 prepare 契约）+ WP7（前端 candidate 面板 + WS，依赖 WP2-5 WS 契约），最后 WP8 贯通测试。WP6/WP7 的「前置交付摘要」直接引用本 plan Task 3 Step 3.3 的产出。

**⚠️ 实现前注意**：
1. Task 1 Step 1.4 的 `app_paths` 来源需先 grep 确认（engine 是否持有 app_paths）。
2. Task 2 删路由建议与 WP6 前端入口改造同批次落地（删路由后前端旧调用会断，WP6 收口）。
3. Task 2 Step 2.4 的 dead code 清理依赖 `cargo check` 警告指引，逐一确认无引用后删除。
