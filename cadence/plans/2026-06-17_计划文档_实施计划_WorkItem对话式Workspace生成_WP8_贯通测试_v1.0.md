# WorkItem 对话式 Workspace 生成 WP8：贯通测试 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 验证 prepare → author → revert → review → revision → confirm 的端到端关系；Fake provider 下全流程可走通；Story/Design/WorkItem/WorkItemPlan 四种 workspace type 的恢复链路影响评估完成；废弃路由 404。本 WP 不改生产代码（除非测试暴露真实缺陷，届时先新增修复计划）。

**Architecture:** WP1-WP7 已分别落地各层。本 WP 用 Fake provider（`MockSplitProviderAdapter`）+ 现有 `app_with_confirmed_story_and_design` 夹具，写贯通测试覆盖 WorkItemPlan 全流程（HTTP prepare + WS start_generation/author/revert/revision/review/confirm）。恢复链路评估：`SessionState.workspace_type = work_item_plan` 经 serde 往返不丢 candidate；Story/Design/WorkItemPlan 三种 ChatWorkspace type 的 timeline/chat/provider 恢复逐一验证；WorkItem 既有 Coding Workspace 链路若不受 artifact union 影响，给出排除说明。废弃路由 404 已在 WP5 验证，本 WP 复测。

**Tech Stack:** Rust 1.95.0、Cargo、tokio、axum（后端集成测试）；React、Vitest（前端）；`pnpm`（前端）。本 WP 不做 Playwright 浏览器 E2E。

**版本：** v1.0
**创建日期：** 2026-06-17
**目标分支：** `feat-b-0616`
**工作区：** `.worktrees/feat-b-0616`
**依赖总览：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_拆分总览_v1.0.md`（v1.1，WP8 章节）
**设计方案：** `cadence/designs/2026-06-17_技术方案_WorkItem对话式Workspace生成_v1.0.md`（第 445-502 行测试策略与验收标准）
**前置 WP：** WP1-WP7

---

## 全局约束（Global Constraints）

- **运行命令固定**：Rust 1.95.0；cargo 命令带 `--locked`；🔴 **禁止 `-j 1``。前端用 `pnpm`。
- **强制检查链**：`cargo fmt --check` + `cargo clippy --all-targets --all-features --locked -- -D warnings` + `cargo check --locked` + 定向测试 + `pnpm -C web test` + `pnpm -C web build`，全绿。
- **TDD**：本 WP 是测试 WP，每个 Task 先写失败测试（若测试暴露生产缺陷，停下新增修复计划，不在本 WP 改生产代码）。
- **写入范围严格**：只改「File Structure」声明的文件。**不改生产后端/前端代码**，除非测试暴露真实缺陷（届时先新增修复计划）。
- **行号是参考**：基于 `feat-b-0616` HEAD `8a2eee4`；实现时以 `grep -n` 实际为准。

---

## 前置交付摘要（来自 WP1-WP7）

### 后端（WP1-WP5）
- `POST /work-item-plans:prepare`：创建空 Draft `IssueWorkItemPlan` + `WorkItemPlan` session（`entity_id = plan_id`），注入上下文，不调 provider。
- WS 链路：`StartGeneration`（WorkItemPlan）→ `WorkItemSplitEngine::generate`（非流式）→ `ArtifactUpdate`（candidate）→ `StageChange`（author_confirm）。
- `RevertWorkItem`（AuthorConfirm）→ candidate meta 更新 + 同 version `ArtifactUpdate`。
- `RequestRevision`/review `StartRevision`（WorkItemPlan）→ `WorkItemPlanRevision`（`generate_revision` + `repatch_dependencies`）→ 新 version `ArtifactUpdate` → `StageChange`（author_confirm）。
- review：`build_work_item_plan_review_input` 流式审整组 → `ReviewDecisionResponse`。
- `HumanConfirm::Confirm` → `confirm_issue_work_item_plan` + `ensure_work_item_sessions_for_plan`（幂等建子 WorkItem session）→ `StageChange`（completed）。
- 3 条废弃 REST 路由已删（404）。
- artifact payload union：`ArtifactUpdate`/`SessionState.artifact` 为 `{ markdown, diff? }` 或 `{ candidate }` 扁平形态。

### 前端（WP6-WP7）
- `prepareWorkItemPlan` API client + `ChatWorkspacePage`（`work_item_plan` 分支）→ `WorkItemPlanCandidatePanel`。
- `workspace-ws-store`：`workItemPlanCandidate` 状态 + union 分流 + `sendRevertWorkItem`。

---

## 关键既有事实（避免重新探查）

- `tests/it_web/web_work_item_generation.rs`：`app_with_confirmed_story_and_design(valid_split_output)` + `MockSplitProviderAdapter` + `request_json` + WP2b 共享 WS helper（`connect_ws` / `recv_ws_messages_with_timeout` / `recv_until_stage`）。
- `tests/it_web.rs`：`#[path] mod xxx;` 聚合子模块。
- WP2b/WP3/WP4/WP5 已建的定向测试：`prepare_work_item_plan_creates_draft_plan_and_session_without_generating`、`work_item_plan_start_generation_returns_candidate_artifact`、`review_returns_verdict_for_whole_candidate`、`revert_work_item_triggers_local_redo_in_revision`、`confirm_creates_child_work_item_sessions`、`delete_legacy_rest_routes_returns_404`。
- 前端测试：`IssueLifecycleWorkbench.test.tsx`、`ChatWorkspacePage.test.tsx`（WP6/WP7 已建 work_item_plan 分支测试）。

---

## File Structure

| 文件 | 操作 | 职责 / 本 WP 改动 |
|---|---|---|
| `tests/it_web/web_work_item_split_flow.rs` | N | WorkItemPlan 全流程贯通测试（prepare→author→revert→review→revision→confirm） |
| `tests/it_web.rs` | M | 注册 `web_work_item_split_flow` mod |
| `tests/it_web/web_work_item_generation.rs` | M | 补充恢复链路一致性测试（若有空间），或新增 `web_workspace_recovery_consistency.rs` |
| `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx` | M | 贯通入口测试（WP6 已建，本 WP 补全端到端断言） |
| `web/src/pages/ChatWorkspacePage.test.tsx` | M | work_item_plan 全流程 UI 测试（WP7 已建分支，本 WP 补 review/confirm 交互） |

**不改：**
- ❌ 任何生产后端代码（`src/`）
- ❌ 任何生产前端代码（`web/src/` 的非 test 文件）
- ❌ 不新增 Playwright 浏览器 E2E

> ⚠️ 若贯通测试暴露真实生产缺陷，**停下**，新增修复计划（不在本 WP 改生产代码）。

---

## Task 1：WorkItemPlan 全流程贯通测试（后端）

**目标**：用 Fake provider 写一个贯通测试：prepare（HTTP）→ WS connect → start_generation → 收 candidate → revert 标记 → request_revision → 收新 candidate → author_decision accept →（review）→ human_confirm confirm → 验证 plan Confirmed + 子 WorkItem session 建立。

**Files:**
- Create: `tests/it_web/web_work_item_split_flow.rs`
- Modify: `tests/it_web.rs`（注册 mod）

**Interfaces:**
- Consumes: WP1-WP5 的后端链路 + WP2b 共享 WS helper + `MockSplitProviderAdapter`。
- Produces: `work_item_plan_full_flow` 贯通测试。

- [ ] **Step 1.1：写贯通测试**

`tests/it_web/web_work_item_split_flow.rs`：

```rust
use super::{app_with_confirmed_story_and_design, request_json, valid_split_output};
use axum::http::{Method, StatusCode};
use serde_json::json;
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{WorkspaceType, IssueWorkItemPlanStatus};

#[tokio::test]
async fn work_item_plan_full_flow() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    // 1. prepare
    let (_, prepare_resp) = request_json(app.clone(), Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({ "title":"登录拆分", "story_spec_ids":["story_spec_0001"], "design_spec_ids":["design_spec_0001"],
                "include_integration_tests":true, "include_e2e_tests":false,
                "force_frontend_backend_split":true, "require_execution_plan_confirm":false, "review_rounds":1 })).await;
    let session_id = prepare_resp["workspace_session"]["workspace_session_id"].as_str().unwrap().to_string();
    let plan_id = prepare_resp["work_item_plan"]["id"].as_str().unwrap().to_string();

    // 2. WS connect + start_generation
    let ws = connect_ws(&app, &session_id).await;
    ws.send(json!({"type":"start_generation","provider_config":{/* minimal */},"reviewer_enabled":true}).to_string()).await;
    let messages = recv_ws_messages_with_timeout(&ws, timeout).await;

    // 3. 收 candidate artifact + stage=author_confirm
    let artifact = messages.iter().find(|m| m["type"] == "artifact_update").expect("artifact_update");
    assert!(artifact["candidate"]["work_items"].is_array());
    assert!(artifact["candidate"]["work_items"].as_array().unwrap().len() >= 1);
    let first_wi_id = artifact["candidate"]["work_items"][0]["id"].as_str().unwrap().to_string();
    assert!(messages.iter().any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm"));

    // 4. revert 标记 first_wi
    ws.send(json!({"type":"revert_work_item","work_item_id":first_wi_id,"feedback":"拆得太粗","clear":false}).to_string()).await;
    let after_revert = recv_ws_messages_with_timeout(&ws, timeout).await;
    let reverted_artifact = after_revert.iter().filter(|m| m["type"] == "artifact_update").last().expect("artifact_update after revert");
    let reverted_wi = reverted_artifact["candidate"]["work_items"].as_array().unwrap().iter()
        .find(|w| w["id"] == first_wi_id).unwrap();
    assert_eq!(reverted_wi["meta"]["reverted"], true);

    // 5. request_revision
    ws.send(json!({"type":"request_revision","feedback":"重做被标记的"}).to_string()).await;
    let after_revision = recv_ws_messages_with_timeout(&ws, timeout).await;
    let new_artifact = after_revision.iter().filter(|m| m["type"] == "artifact_update").last().expect("artifact_update after revision");
    // 旧 wi_id 不在，有新 id 顶替，整组数量不变
    assert!(new_artifact["candidate"]["work_items"].as_array().unwrap().iter().all(|w| w["id"] != first_wi_id));
    assert_eq!(new_artifact["candidate"]["work_items"].as_array().unwrap().len(), artifact["candidate"]["work_items"].as_array().unwrap().len());
    // 回 author_confirm
    assert!(after_revision.iter().any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm"));

    // 6. author_decision accept → review（review_rounds=1）→ review_decision_response continue → human_confirm
    ws.send(json!({"type":"author_decision","decision":"accept"}).to_string()).await;
    // 收 review 相关消息（ReviewDecisionRequired/StreamChunk/ReviewComplete）→ 发 ReviewDecisionResponse continue
    let review_msgs = recv_ws_messages_with_timeout(&ws, timeout).await;
    // 断言 review 触发（若有 reviewer）——具体消息时序以 WP3 实现为准
    ws.send(json!({"type":"review_decision_response","decision":"continue"}).to_string()).await;
    recv_until_stage(&ws, "human_confirm", timeout).await;

    // 7. human_confirm confirm → completed
    ws.send(json!({"type":"human_confirm","action":"confirm"}).to_string()).await;
    let final_msgs = recv_ws_messages_with_timeout(&ws, timeout).await;
    assert!(final_msgs.iter().any(|m| m["type"] == "stage_change" && m["stage"] == "completed"));

    // 8. 验证 plan Confirmed + 子 WorkItem session 建立
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(_repo.path().join(".aria")));
    let plan = lifecycle.get_issue_work_item_plan("project_0001", "issue_0001", &plan_id).unwrap();
    assert_eq!(plan.status, IssueWorkItemPlanStatus::Confirmed);
    let work_items = lifecycle.list_work_items("project_0001", "issue_0001").unwrap();
    let sessions = lifecycle.list_workspace_sessions("project_0001", "issue_0001").unwrap();
    for wi in &work_items {
        assert!(sessions.iter().any(|s| s.workspace_type == WorkspaceType::WorkItem && s.entity_id == wi.id));
    }
}
```

> 实现者注意：
> 1. WS helper（`connect_ws`/`recv_ws_messages_with_timeout`/`recv_until_stage`/`timeout`）：复用 WP2b 共享 helper。若 helper 不完整，本 Task 先补全 helper（作为测试基础设施，属本 WP 范围），但不要降级为非 WS 测试。
> 2. `provider_config`/`reviewer_enabled` 最小值参考现有 StartGeneration 测试夹具。
> 3. review 消息时序（`ReviewDecisionRequired`/`StreamChunk`/`ReviewComplete`）：以 WP3 实现为准。若 review 在 `review_rounds=1` 时自动跑完不要求用户响应，调整测试时序。`recv_until_stage("human_confirm")` 兜底。
> 4. `MockSplitProviderAdapter` 需支持 revision 调用返回不同 JSON（保留项 + 重做项）——`grep -n "MockSplitProviderAdapter" tests/it_web/web_work_item_generation.rs` 看是否支持调用序列，不支持则扩展。
> 5. WS 端到端测试是本 WP 核心价值；如果多轮消息不稳定，收紧 helper 的超时/过滤逻辑或拆分为多个 WS 场景测试，不用 engine/store 层测试替代。

- [ ] **Step 1.2：运行贯通测试，修复测试基础设施**

Run: `cargo test --locked --test it_web work_item_plan_full_flow`
Expected: 通过（若 WP1-WP5 实现正确）；若失败，分析是测试基础设施问题（修测试）还是生产缺陷（停下新增修复计划）。

> 若测试暴露生产缺陷（如 review 时序错误、revision DAG 重连错误、confirm 幂等性错误），**停下**，记录缺陷，新增修复计划。不在本 WP 改生产代码。

- [ ] **Step 1.3：提交**

```bash
git add tests/it_web/web_work_item_split_flow.rs tests/it_web.rs
git commit -m "test(WP8): WorkItemPlan 全流程贯通测试（prepare→author→revert→revision→review→confirm）"
```

---

## Task 2：四种 workspace type 恢复链路一致性

**目标**：验证 Story/Design/WorkItemPlan 三种 ChatWorkspace type 的 timeline/chat/provider 恢复一致；WorkItem 既有 Coding Workspace 链路若不受 artifact union 影响，给出排除说明。`SessionState.workspace_type = work_item_plan` 经 serde 往返不丢 candidate。

**Files:**
- Create: `tests/it_web/web_workspace_recovery_consistency.rs`（或追加到 `web_work_item_generation.rs`）
- Modify: `tests/it_web.rs`（注册 mod，若新增）

**Interfaces:**
- Consumes: WP2a 的 artifact payload union + `SessionState` serde。
- Produces: 恢复链路一致性测试 + WorkItem 排除说明。

- [ ] **Step 2.1：写恢复链路一致性测试**

```rust
#[tokio::test]
async fn story_design_work_item_plan_recovery_consistency() {
    // 对 Story/Design/WorkItemPlan 三种 type：
    // 1. prepare/generate 到某个中间 stage（如 author_confirm）
    // 2. 新建 engine（new_persistent）从 lifecycle 恢复
    // 3. 断言 session.artifact、stage、timeline_nodes、messages 恢复正确
    //    - Story/Design：artifact 是 Markdown payload
    //    - WorkItemPlan：artifact 是 WorkItemPlanCandidate payload
    // 4. SessionState 经 serde 往返不丢字段
}

#[tokio::test]
async fn work_item_workspace_recovery_unaffected_or_covered() {
    // WorkItem（Coding Workspace）的恢复链路：
    // - artifact union 改造后，WorkItem session 的 artifact 是否受影响？
    // - WorkItem 的 Coding Workspace 链路（P5/P6 已落地）是否走 artifact payload union？
    // - 若 WorkItem 不走 ChatWorkspacePage 的 artifact 链路（Coding Workspace 独立），给出排除说明：
    //   "WorkItem Coding Workspace 不使用 ArtifactUpdate/SessionState.artifact，
    //    走独立的 CodingAttempt/CodingSession 协议，不受 WP2a artifact union 影响。"
    // - 断言：WorkItem session 恢复后，其 Coding 链路字段（attempts等）未受 union 改造影响。
    // - 若 WorkItem 也用 artifact（若有），验证其 Markdown payload 恢复正确。
}

#[tokio::test]
async fn session_state_serde_roundtrip_preserves_work_item_plan_candidate() {
    // 构造一个 workspace_type=work_item_plan 的 SessionState，artifact={candidate: {...}}
    // serde 序列化 → 反序列化，断言 candidate 完整保留（work_items/meta/dependency_graph 等）
}

#[tokio::test]
async fn reconnect_preserves_revert_marks_from_current_artifact_version() {
    // prepare → start_generation → RevertWorkItem(work_item_0001, feedback)
    // 断开后重新连接同一 session。
    // 断言 SessionState.artifact.candidate.work_items[work_item_0001].meta.reverted == true，
    // 且 revert_feedback 保留。该测试锁定 WP4 的策略：revert meta 写回当前 ArtifactVersion.payload，
    // 不新增 version，但必须跨重连恢复。
}
```

> 实现者注意：
> 1. `new_persistent`（`workspace_engine.rs:470-524`）恢复逻辑：WP2a 已适配 union（从 `ArtifactVersion.payload` 恢复 `session.artifact`）。本测试验证 WorkItemPlan 的 candidate payload 恢复正确，并额外验证 WP4 的 revert meta 已写回当前 `ArtifactVersion.payload`，重连不丢。
> 2. WorkItem Coding Workspace 是否受影响：`grep -rn "ArtifactUpdate\|session.artifact\|WorkspaceType::WorkItem" src/product/workspace_engine.rs src/web/workspace_ws_handler.rs` 确认 WorkItem 是否走 artifact 链路。若 WorkItem 的 Coding Workspace 走独立协议（`CodingAttempt`/`CodingSession`），不受 union 影响——在测试注释/文档说明原因。
> 3. `SessionState` serde 往返：构造 `WsOutMessage::SessionState { workspace_type: WorkItemPlan, artifact: Some(ArtifactPayload::WorkItemPlanCandidate { candidate }), ... }`，`serde_json::to_string` → `from_str`，断言 `artifact` 的 candidate 完整。

- [ ] **Step 2.2：运行测试 + 修复**

Run:
```
cargo test --locked --test it_web story_design_work_item_plan_recovery_consistency
cargo test --locked --test it_web work_item_workspace_recovery_unaffected_or_covered
cargo test --locked --test it_web session_state_serde_roundtrip_preserves_work_item_plan_candidate
cargo test --locked --test it_web reconnect_preserves_revert_marks_from_current_artifact_version
```
Expected: 通过。若失败，分析是测试问题还是生产缺陷（union 恢复遗漏 WorkItemPlan 分支等），停下新增修复计划。

- [ ] **Step 2.3：提交**

```bash
git add tests/it_web/web_workspace_recovery_consistency.rs tests/it_web.rs
git commit -m "test(WP8): 四种 workspace type 恢复链路一致性 + WorkItem 排除说明"
```

---

## Task 3：前端贯通 + 废弃路由复测 + WP8 收口

**目标**：前端 `IssueLifecycleWorkbench` → `ChatWorkspacePage` 贯通测试补全；废弃路由 404 复测；全量回归。

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`
- Modify: `web/src/pages/ChatWorkspacePage.test.tsx`

**Interfaces:**
- Consumes: WP6/WP7 前端链路。
- Produces: 前端贯通测试 + 全量回归。

- [ ] **Step 3.1：前端贯通测试补全**

`web/src/pages/ChatWorkspacePage.test.tsx` 补 work_item_plan 的 review/confirm 交互测试：

```typescript
  it("work_item_plan full UI flow: revert → regenerate → confirm", async () => {
    // mock store: workItemPlanCandidate + stage=author_confirm
    // 渲染 ChatWorkspacePage（work_item_plan）
    // 点 revert → 输入反馈 → 提交 → 断言 sendRevertWorkItem 调用
    // candidate 更新（mock reverted=true）→ 点"重新生成被标记的 1 项" → 断言 sendRequestRevision
    // 点"确认计划" → 断言 sendAuthorDecision("accept")
  });
```

`web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`：确认 WP6 的入口贯通测试覆盖 prepare → 打开 ChatWorkspacePage（本 WP 复测，不新增）。

- [ ] **Step 3.2：废弃路由 404 复测**

Run: `cargo test --locked --test it_web delete_legacy_rest_routes_returns_404`
Expected: PASS（WP5 已验证，本 WP 复测确认未被后续 WP 破坏）。

- [ ] **Step 3.3：全量验证链（最终回归）**

Run（依次，任一失败即停）:
```
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
pnpm -C web test -- --run
pnpm -C web build
```
Expected: 全绿。

> `cargo test --locked`（全量）是最终回归，覆盖 WP1-WP8 所有后端单测 + 集成测试。`pnpm -C web test -- --run` + `pnpm -C web build` 覆盖前端。

- [ ] **Step 3.4：交付摘要（验收）**

commit 后，对照设计方案第 493-502 行验收标准逐项确认：

- ✅ Design Spec 点"生成下一阶段" → `ChatWorkspacePage`（不再弹窗）—— WP6
- ✅ 点"开始生成" → `AuthorRun` → 一次性返回 candidate —— WP2b/WP7
- ✅ AuthorConfirm 每个 WorkItem 可 revert（附反馈），连续标记多个；点"重新生成被标记的" → Revision 新 version，重做 + 保留 + DAG 重连 —— WP4/WP7
- ✅ review_rounds>0：reviewer 流式审整组，verdict + review decision —— WP3/WP7
- ✅ HumanConfirm::Confirm → plan Confirmed + 子 Coding session —— WP5
- ✅ 3 条废弃路由 404 —— WP5（本 WP 复测）
- ✅ 四种 workspace type 恢复影响评估完成 —— Task 2
- ✅ `cargo fmt/clippy/check/test` + `pnpm test/build` 全绿 —— Task 3 Step 3.3

---

## Self-Review（写完后的自查）

**1. Spec 覆盖**（对照总览 v1.1 WP8 目标/写入范围/验证 + 设计方案 :445-502）：
- ✅ prepare→author→revert→review→revision→confirm 端到端 → Task 1
- ✅ Fake provider 全流程走通 → Task 1
- ✅ 四种 workspace type 恢复链路评估 → Task 2
- ✅ WorkItem 既有 Coding Workspace 链路排除说明 → Task 2 Step 2.1
- ✅ 废弃路由 404 → Task 3 Step 3.2（WP5 已验证，复测）
- ✅ 前端贯通 → Task 3 Step 3.1
- ✅ 验证命令链 → Task 3 Step 3.3
- ✅ 不做项：不改生产代码（除非暴露缺陷）、不新增 Playwright E2E——均在「不做」清单。

**2. Placeholder 扫描**：
- WS helper（Task 1 Step 1.1）：复用/补全 WP2b 共享 helper，不降级为 engine 层测试。属可接受。
- `MockSplitProviderAdapter` 调用序列（Task 1 Step 1.1）：给出 grep 确认 + 扩展指引。属可接受。
- review 消息时序（Task 1 Step 1.1）：以 WP3 实现为准，`recv_until_stage` 兜底。属可接受。
- WorkItem 排除说明（Task 2 Step 2.1）：给出 grep 确认 WorkItem 是否走 artifact 链路的指引。属可接受。

**3. 边界风险**：
- **贯通测试暴露生产缺陷**（Task 1 Step 1.2）：本 WP 不改生产代码，缺陷需新增修复计划。已标注（全局约束 + Task 1 Step 1.2）。
- **WS 端到端测试难度**（Task 1 Step 1.1）：若 WS 握手 + 多轮消息不稳定，补强 helper 或拆成多个 WS 场景测试；不降级为 engine/store 层衔接测试。已标注。
- **WorkItem 恢复链路**（Task 2 Step 2.1）：WorkItem Coding Workspace 是否受 artifact union 影响需确认。若不受影响，排除说明；若受影响，补测试。已标注 grep 确认。
- **review_rounds 时序**（Task 1 Step 1.1）：review_rounds=1 时 review 自动跑还是要求用户响应，以 WP3 实现为准。测试时序需适配。已标注。

**4. 一致性**：
- 贯通测试覆盖设计方案的验收标准（:493-502）——Task 3 Step 3.4 逐项对照。
- 四种 workspace type 恢复测试覆盖 union 改造对 Story/Design/WorkItem/WorkItemPlan 的影响——Task 2。

---

## Execution Handoff

本 WP8 plan 已保存至 `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP8_贯通测试_v1.0.md`。

**执行方式（待用户选择）：**

1. **Subagent-Driven（推荐）** — 每个 Task 派一个 fresh subagent 实现，Task 间做 two-stage review。
2. **Inline Execution** — 在当前 session 按 superpowers:executing-plans 批量执行，带 checkpoint 审查。

**完成后**：WP1-WP8 全部交付，WorkItem 对话式 Workspace 生成流程完整落地。对照设计方案第 493-502 行验收标准逐项确认（Task 3 Step 3.4）。

**⚠️ 实现前注意**：
1. Task 1 的 WS 贯通测试是本 WP 核心，若 WS helper 不完整需先补全（作为测试基础设施）。
2. 若贯通测试暴露生产缺陷，**停下新增修复计划**，不在本 WP 改生产代码。
3. Task 2 的 WorkItem 排除说明需基于 grep 确认 WorkItem Coding Workspace 是否走 artifact 链路。
