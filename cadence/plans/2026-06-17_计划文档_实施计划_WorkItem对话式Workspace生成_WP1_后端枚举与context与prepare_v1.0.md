# WorkItem 对话式 Workspace 生成 WP1：后端枚举 + context + prepare + WS 契约 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地 `WorkspaceType::WorkItemPlan` 枚举变体、`workspace_context.rs` 的全部 WorkItemPlan 分支、`prepare_work_item_plan` handler 与 `POST /work-item-plans:prepare` 路由、以及 WS 层的 artifact payload union 契约骨架；使 prepare 能创建空 Draft `IssueWorkItemPlan`、创建 `WorkItemPlan` session（`entity_id = plan_id`）、注入上下文消息，且 `WorkspaceType` 与 `WorkItemPlanCandidateDto` 经 serde 往返不丢字段。

**Architecture:** 本 WP 是对话式 WorkItem 生成流程的地基层。`WorkspaceType::WorkItemPlan` 是后续 WP2–WP5 状态机分支的入口；prepare 阶段创建的空 Draft `IssueWorkItemPlan` 是 author/revision 阶段读取结构化参数（source ids/options）的唯一来源；WS 层的 `ArtifactPayload` enum 与 `WorkItemPlanCandidateDto` 定义了 WP2 candidate 推送、WP7 前端展示、WP4 revert 标记的契约。本 WP 不实现 engine 分支、不调 Provider、不删 REST 路由、不改前端。

**Tech Stack:** Rust 1.95.0（edition 2024）、Cargo、Axum、tokio、serde（`#[serde(rename_all = "snake_case")]`）。后端单测内联在源文件 `#[cfg(test)] mod tests`，集成测试在 `tests/it_web.rs`（通过 `#[path]` mod 聚合）。前端在本 WP 不涉及。

**版本：** v1.0
**创建日期：** 2026-06-17
**目标分支：** `feat-b-0616`
**工作区：** `.worktrees/feat-b-0616`
**依赖总览：** `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_拆分总览_v1.0.md`
**设计方案：** `cadence/designs/2026-06-17_技术方案_WorkItem对话式Workspace生成_v1.0.md`

---

## 全局约束（Global Constraints）

复制自拆分总览与设计方案，每个 Task 的需求隐式包含本节：

- **运行命令固定**：Rust 工具链锁 `rust-toolchain.toml` 的 1.95.0；所有 cargo 命令必须带 `--locked`；🔴 **禁止 `-j 1`**（详见 `cadence/project-rules/build-test-commands.md`）。
- **定向快反馈**：开发期用 `cargo check --locked` / `cargo test --locked --lib <过滤名>` / `cargo test --locked --test it_web <过滤名>`；完整验证链放在每个 Task 末尾与 WP 收口。
- **强制检查链**：`cargo fmt --check` + `cargo clippy --all-targets --all-features --locked -- -D warnings` + `cargo check --locked` + 定向测试，**四者缺一不可**，不允许只跑 `fmt + check`。
- **TDD**：每个 Task 先写失败测试，再写最小实现，再跑定向验证；提交保持原子性（小步提交）。
- **serde 约定**：所有新增 enum/struct 默认 `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]` + `#[serde(rename_all = "snake_case")]`，与现有 `WorkspaceType`、`IssueWorkItemPlanStatus` 等保持一致。
- **中文文案**：`workspace_context.rs` 的 system_prompt / output_schema / constraint_summary 等面向 Provider/用户的文案使用中文，与现有 Story/Design/WorkItem 分支一致。
- **写入范围严格**：本 WP 只改「File Structure」声明的文件；实现时若发现需越界（例如必须改 `workspace_engine.rs` 才能编译），**停止扩大范围**，先记录原因并与维护者确认（很可能应调整 WP 边界）。
- **当前代码行号是参考**：方案与本 plan 给出的行号基于 2026-06-17 的 `feat-b-0616` HEAD（`8a2eee4`）；实现时以实际为准，先用 `grep -n` 定位再改。

---

## 当前前置状态（前置交付摘要）

本 WP 是首个 WP，无前置 WP 依赖。实现者需读取以下两份文档对应章节：

- 设计方案 `cadence/designs/2026-06-17_技术方案_WorkItem对话式Workspace生成_v1.0.md`：第 2 节（产物路线 B+）、第 2.1 节（prepare 唯一结构化来源：空 Draft `IssueWorkItemPlan`）、第 12 节（prepare 请求体复用现有 provider 配置解析）、第 13 节（删除 3 条废弃 REST 路由——**删除在 WP5，本 WP 只新增 prepare 路由，不删旧路由**）。
- 拆分总览：WP1 章节（目标/写入范围/验证/不做）。

**已就绪的可复用内核（本 WP 直接调用，不改）：**

| 内核 | 位置 | 本 WP 用法 |
|---|---|---|
| `IssueWorkItemPlan` 结构体 | `src/product/models.rs:562-581` | prepare 创建空 Draft plan 的实体（字段已齐全，**本 WP 不改其定义**） |
| `IssueWorkItemPlanStatus::{Draft, Confirmed, ChangeRequested}` | `src/product/models.rs:400-406` | prepare 写 `Draft` |
| `IssueWorkItemPlanOptions`（4 个布尔字段） | `src/product/models.rs:425-432` | prepare 从请求体构造 |
| `LifecycleStore::create_issue_work_item_plan(input)` | `src/product/lifecycle_store.rs:407-445` | prepare 持久化空 Draft plan（`CreateIssueWorkItemPlanInput` 已支持 `status: Draft` + 空 `work_item_ids`） |
| `LifecycleStore::create_workspace_session(input)` | `src/product/lifecycle_store.rs:868-901` | 创建 `WorkspaceType::WorkItemPlan` session |
| `LifecycleStore::get_issue_work_item_plan` | `src/product/lifecycle_store.rs`（pub fn，见 `impl` 列表） | `workspace_entity_context` 的 WorkItemPlan 分支按 `session.entity_id` 取 plan |
| `ensure_workspace_context_message(&app_paths, &lifecycle, session)` | `src/web/workspace_context.rs:13`（pub fn） | prepare 注入上下文消息 |
| `provider_workspace_config(...)` | `src/web/handlers.rs:3113-3146` | prepare 请求体解析 provider 配置 |
| `generate_story_specs` handler | `src/web/handlers.rs:406-458` | prepare handler 的骨架模板 |
| `app_with_confirmed_story_and_design` + `MockSplitProviderAdapter` + `request_json` | `tests/it_web/web_work_item_generation.rs`（均 `pub(crate)`） | prepare 集成测试夹具 |

**关键既有事实（避免重新探查）：**

- `WorkspaceType` 当前定义在 `src/product/models.rs:237-243`，仅 `Story / Design / WorkItem` 三个变体，`#[serde(rename_all = "snake_case")]`，**无 impl、无 Hash、无 Default**。新增 `WorkItemPlan` 会自动序列化为 `"work_item_plan"`。
- `WorkspaceSessionRecord`（`src/product/models.rs:598-617`）**无 metadata/context 字段**；会话上下文由 `workspace_context.rs` 运行期组装进 `messages`，不存于 record。因此 prepare 的结构化参数必须落进 Draft `IssueWorkItemPlan`，不能只塞进消息文本。
- `WorkspaceSessionRecord.entity_id: String` 是外键；WorkItemPlan session 的 `entity_id = plan.id`。
- `WsOutMessage::ArtifactUpdate` 当前是 `{ version: u32, markdown: String, diff: Option<String> }`（`src/web/workspace_ws_types.rs:50-54`），`SessionState.artifact: Option<String>`（同文件 :110）。**两者当前都是纯 markdown String**。
- `WsInMessage`（同文件 :135-190）当前**无** `RevertWorkItem` 变体。
- `tests/it_web.rs` 用 `#[path = "..."] mod xxx;` 把多个子文件聚合进同一测试二进制；子文件间用 `pub(crate)` + `use super::super::...` 共享夹具。

---

## File Structure

本 WP 涉及的文件及其职责（新增 N，修改 M）：

| 文件 | 操作 | 职责 / 本 WP 改动 |
|---|---|---|
| `src/product/models.rs` | M | `WorkspaceType` 枚举新增 `WorkItemPlan` 变体（约 :237-243） |
| `src/web/workspace_context.rs` | M | 10 处 match / early-return 补 `WorkItemPlan` 分支；新增 `find_issue_work_item_plan` helper；补 WorkItemPlan context 单测 |
| `src/web/workspace_ws_types.rs` | M | **纯新增** `ArtifactPayload` enum（`untagged`）、`WorkItemPlanCandidateDto` 及子 DTO、`WsInMessage::RevertWorkItem` 变体；补 serde 往返单测。**不改 `WsOutMessage::ArtifactUpdate` 与 `SessionState.artifact`**（union 挂载在 WP2） |
| `src/web/handlers.rs` | M | 新增 `prepare_work_item_plan` handler；`workspace_type_text`（:3063-3069）补 WorkItemPlan 分支 |
| `src/web/types.rs` | M | 新增 `PrepareWorkItemPlanRequest` / `PrepareWorkItemPlanResponse` DTO |
| `src/web/app.rs` | M | 注册 `POST .../work-item-plans:prepare` 路由（邻近 :79-82 的 `work-items:generate`） |
| `tests/it_web/web_work_item_generation.rs` 或新增 `tests/it_web/web_work_item_plan_prepare.rs` | M/N | prepare 集成测试（复用 `app_with_confirmed_story_and_design` 夹具） |

**不改（重要边界）：**

- ❌ `src/product/workspace_engine.rs`（WP2–WP5 共享，本 WP 不碰）
- ❌ `src/web/workspace_ws_handler.rs`（WP2/WP4/WP5 共享）
- ❌ `src/product/lifecycle_store.rs`（WP2/WP4/WP5 共享——本 WP 只**调用**其现有 pub fn，不改其实现）
- ❌ `src/product/work_item_split_engine.rs`（WP4）
- ❌ 任何前端文件（WP6/WP7）
- ❌ 不删除任何 REST 路由（WP5）

> ⚠️ **artifact payload union 的阶段性策略（选项 B，已与维护者确认）**：设计方案第 204-209 行要求把 `WsOutMessage::ArtifactUpdate` 与 `SessionState.artifact` 升级为 `ArtifactPayload::Markdown | WorkItemPlanCandidate` 互斥 enum。但构造 `ArtifactUpdate` 的 `workspace_ws_handler.rs:270` 与消费它的 `workspace_engine.rs` 不在 WP1 写入范围（WP2 才改）。若 WP1 改 `ArtifactUpdate` 字段形态，会让这两个文件编译失败。
>
> **WP1 的处理（选项 B）**：在 `workspace_ws_types.rs` 中**纯新增** `ArtifactPayload` enum（serde `untagged`，JSON 表现为方案要求的扁平 `markdown?/diff?/candidate?` 形态）、`WorkItemPlanCandidateDto` 及子 DTO、`WsInMessage::RevertWorkItem` 变体。**不修改任何现有类型**——`WsOutMessage::ArtifactUpdate`（`{ version, markdown, diff }`）与 `SessionState.artifact`（`Option<String>`）保持原样。因此 WP1 **完全不触及** `workspace_ws_handler.rs` / `workspace_engine.rs`，`cargo check` 直接通过。`ArtifactPayload` 的真正挂载（`ArtifactUpdate` / `SessionState.artifact` 切换到此 enum）与所有构造/消费点改造在 **WP2**（WP2 本就要改这两个文件，届时一并完成 union 切换）。Task 2 落实此策略。

---

## Task 1：`WorkspaceType::WorkItemPlan` 变体 + `workspace_context.rs` 全部分支

**目标**：加枚举变体；让 `workspace_context.rs` 的 10 处分支（9 个 helper 函数的 match + `work_item_context_summary` 的 early-return）对 WorkItemPlan 给出合理文案与 entity 解析；新增 `find_issue_work_item_plan` helper；补齐 WorkItemPlan 的 context message 单测；保证 `all_workspace_artifact_outputs_require_artifact_fence` 回归对新变体通过。

**Files:**
- Modify: `src/product/models.rs`（`enum WorkspaceType`，约 :237-243）
- Modify: `src/web/workspace_context.rs`（:146-185 `work_item_context_summary`、:187-191 `is_workspace_generation_brief`、:207-250 `workspace_entity_context`、:390-396 `workspace_type_label`、:398-404 `node_id_for`、:406-412 `workspace_runtime_role`、:414-426 `system_prompt_for`、:428-447 `constraint_summary_for`、:449-477 `workflow_discipline_for`、:479-506 `output_schema_for`；新增 `find_issue_work_item_plan` helper，邻近 :252-317 的现有 `find_*` helper）
- Modify: `src/web/handlers.rs`（`workspace_type_text`，:3063-3069）
- Test: `src/web/workspace_context.rs` 的 `#[cfg(test)] mod tests`（:523+）

**Interfaces:**
- Consumes: `LifecycleStore::get_issue_work_item_plan(&self, project_id, issue_id, plan_id) -> Result<IssueWorkItemPlan, ProductStoreError>`（现有 pub fn）；`linked_story_context` / `linked_design_context`（现有 private helper，:297/:317）；`issue_repo_id(issue)`（:380）。
- Produces: `WorkspaceType::WorkItemPlan` 变体（被 WP2–WP5 的 engine/handler 分支依赖）；`find_issue_work_item_plan` helper（本文件内部使用）。

- [ ] **Step 1.1：写失败测试 —— WorkItemPlan context message 含预期文案**

在 `src/web/workspace_context.rs` 的 `#[cfg(test)] mod tests`（:524+）末尾追加。测试夹具参考现有 `claude_code_story_context_requires_structured_ask_user_question`（:566-621）：建 repository/issue → 建 story_spec + design_spec（用于 linked_context） → 建 Draft `IssueWorkItemPlan` → 建 WorkItemPlan session → 调 `ensure_workspace_context_message` → 断言消息文案。

```rust
    #[test]
    fn work_item_plan_context_message_includes_plan_brief_and_workspace_type() {
        let root = tempdir().expect("root");
        let repo = tempdir().expect("repo");
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        let repository = RepositoryStore::new(app_paths.clone())
            .create(CreateRepositoryInput {
                project_id: "project_0001".to_string(),
                name: "Repo".to_string(),
                path: repo.path().to_path_buf(),
                default_policy_preset: None,
                default_provider_mode: None,
            })
            .expect("repository");
        IssueStore::new(app_paths.clone())
            .create(CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some(repository.id.clone()),
                title: "登录会话拆分".to_string(),
                description: Some("把登录模块拆成可并行的 Work Item".to_string()),
                change_id: None,
            })
            .expect("issue");

        let lifecycle = LifecycleStore::new(app_paths.clone());
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: repository.id.clone(),
                title: "登录 Story Spec".to_string(),
            })
            .expect("story");
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "登录 Design Spec".to_string(),
                design_kind: DesignKind::Backend,
            })
            .expect("design");
        // 追加 spec 版本并确认（confirm 逻辑参考现有测试夹具；若无 helper，直接调 lifecycle 的确认接口）
        // 注：design_spec_ids / story_spec_ids 用于 linked_context；prepare 阶段不要求 spec 已确认，
        // 但本测试为了让 linked_context 非空，直接传入已创建的 spec id。
        let plan = lifecycle
            .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
                id: None,
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                source_story_spec_ids: vec![story.id.clone()],
                source_design_spec_ids: vec![design.id.clone()],
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: true,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Draft,
                work_item_ids: Vec::new(),
                repository_profile_ref: None,
                verification_plan_ids: Vec::new(),
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            })
            .expect("plan");
        let session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: plan.id.clone(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .expect("session");

        let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
            .expect("workspace context");
        let context = &session.messages[0].content;

        assert!(context.contains("候选 work item plan 生成器"));
        assert!(context.contains("Workspace 类型: Work Item Plan"));
        assert!(context.contains("runtime_role=workspace_work_item_plan"));
        assert!(context.contains("node_id=WORK_ITEM_PLAN"));
        assert!(context.contains(plan.id.as_str()));
        assert!(context.contains("```artifact fenced block"));
        assert!(context.contains("登录 Story Spec"));
        assert!(context.contains("登录 Design Spec"));
    }
```

> 实现者注意：`CreateDesignSpecInput`、`DesignKind`、`IssueWorkItemPlanOptions`、`IssueWorkItemPlanStatus`、`CreateIssueWorkItemPlanInput` 需在 test mod 的 `use` 语句中导入（参考 :525-537 现有导入，补 `use crate::product::models::{DesignKind, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus};` 与 `use crate::product::lifecycle_store::CreateIssueWorkItemPlanInput;`）。若 `CreateDesignSpecInput` 字段与上述不符，以 `grep -n "struct CreateDesignSpecInput" src/product/lifecycle_store.rs` 实际定义为准。

- [ ] **Step 1.2：运行测试，确认失败（编译错误）**

Run: `cargo test --locked --lib workspace_context`
Expected: 编译失败——`WorkspaceType::WorkItemPlan` 不存在；以及 `match session.workspace_type` 非穷尽（现有 6 个 match 无 `_ =>` 兜底）。这是预期的红灯。

- [ ] **Step 1.3：加 `WorkspaceType::WorkItemPlan` 变体**

`src/product/models.rs:237-243`，在 `WorkItem,` 后追加 `WorkItemPlan,`：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceType {
    Story,
    Design,
    WorkItem,
    WorkItemPlan,
}
```

- [ ] **Step 1.4：新增 `find_issue_work_item_plan` helper**

在 `src/web/workspace_context.rs` 现有 `find_story_spec`（:252）/`find_design_spec`/`find_work_item`（:282）附近，按同样模式新增。先 `grep -n "fn find_work_item" src/web/workspace_context.rs` 定位插入点。

```rust
fn find_issue_work_item_plan(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
    plan_id: &str,
) -> Result<IssueWorkItemPlan, ProductStoreError> {
    lifecycle
        .get_issue_work_item_plan(&session.project_id, &session.issue_id, plan_id)
        .map_err(|err| {
            ProductStoreError::not_found_value(
                "issue_work_item_plan",
                plan_id,
                &format!("project={} issue={}", session.project_id, session.issue_id),
                err,
            )
        })
}
```

> 实现者注意：
> 1. 顶部 `use crate::product::models::{...}` 需补 `IssueWorkItemPlan`。
> 2. `ProductStoreError::not_found_value` 的构造签名以现有 `find_story_spec` / `find_work_item` 里的错误包装为准——先读 :252-317 任一现有 `find_*` 函数，**照抄其错误包装方式**（可能是 `ProductStoreError::NotFound` 直接构造，或某个 helper）。若现有 helper 用的是 `.map_err(...)` 包装成别的 variant，照搬即可。**关键是错误能被 `ensure_workspace_context_message` 的调用方 `product_store_api_error` 正确转换成 404。**

- [ ] **Step 1.5：`workspace_entity_context` 加 WorkItemPlan 分支**

`src/web/workspace_context.rs:207-250`，在 `WorkspaceType::WorkItem => { ... }` 之后、match 闭合前追加：

```rust
        WorkspaceType::WorkItemPlan => {
            let plan = find_issue_work_item_plan(lifecycle, session, &session.entity_id)?;
            let mut linked_context =
                linked_story_context(lifecycle, session, &plan.source_story_spec_ids)?;
            linked_context.extend(linked_design_context(
                lifecycle,
                session,
                &plan.source_design_spec_ids,
            )?);
            Ok(WorkspaceEntityContext {
                title: format!("Work Item Plan ({})", plan.id),
                repository_id: issue_repo_id(issue)?,
                linked_context,
            })
        }
```

> 说明：`IssueWorkItemPlan` 无 `title` 字段，title 用 `"Work Item Plan ({plan_id})"` 形式（与 `build_workspace_context_message` 的 `目标产物: {} ({})` 拼接配合，最终显示 `目标产物: Work Item Plan (issue_work_item_plan_0001) (issue_work_item_plan_0001)`——entity_id 重复但可读，可接受；后续 WP 若需优化可改）。`repository_id` 从 issue 取（与 Design 分支一致）。`linked_context` 拼接 source story + design 的摘要。

- [ ] **Step 1.6：`work_item_context_summary` 的 early-return 不变**

`src/web/workspace_context.rs:150` 当前是 `if session.workspace_type != WorkspaceType::WorkItem { return Ok(String::new()); }`。**WorkItemPlan 自然命中此分支（返回空），不产生 `[work_item_context]` block**——这是期望行为（plan 级 workspace 不展示单个 work_item 的 context）。**本步不改代码**，仅确认：WorkItemPlan session 的 context message 不含 `[work_item_context]`（测试 Step 1.1 不断言该 block）。

- [ ] **Step 1.7：`is_workspace_generation_brief` 加关键词**

`src/web/workspace_context.rs:187-191`，追加一条 `contains`（与 `system_prompt_for` 的 WorkItemPlan 文案联动，见 Step 1.12）：

```rust
fn is_workspace_generation_brief(content: &str) -> bool {
    content.contains("候选 spec 生成器")
        || content.contains("候选 design 生成器")
        || content.contains("候选 work item 生成器")
        || content.contains("候选 work item plan 生成器")
}
```

- [ ] **Step 1.8：`workspace_type_label` / `node_id_for` / `workspace_runtime_role` 加分支**

`src/web/workspace_context.rs:390-412`，三个 match 各加一行：

```rust
fn workspace_type_label(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "Story Spec",
        WorkspaceType::Design => "Design Spec",
        WorkspaceType::WorkItem => "Work Item",
        WorkspaceType::WorkItemPlan => "Work Item Plan",
    }
}

fn node_id_for(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "N05",
        WorkspaceType::Design => "N07",
        WorkspaceType::WorkItem => "WORK_ITEM",
        WorkspaceType::WorkItemPlan => "WORK_ITEM_PLAN",
    }
}

fn workspace_runtime_role(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "story_spec",
        WorkspaceType::Design => "design_spec",
        WorkspaceType::WorkItem => "work_item",
        WorkspaceType::WorkItemPlan => "work_item_plan",
    }
}
```

- [ ] **Step 1.9：`handlers.rs::workspace_type_text` 加分支**

`src/web/handlers.rs:3063-3069`：

```rust
fn workspace_type_text(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "story",
        WorkspaceType::Design => "design",
        WorkspaceType::WorkItem => "work_item",
        WorkspaceType::WorkItemPlan => "work_item_plan",
    }
}
```

- [ ] **Step 1.10：`system_prompt_for` 加 WorkItemPlan 文案**

`src/web/workspace_context.rs:414-426`。文案须含字面量「候选 work item plan 生成器」（被 Step 1.7 的 brief 识别依赖）。注意：WorkItemPlan 的 author 实际由 `WorkItemSplitEngine::generate` 驱动（WP2），本 system prompt 只作为 session 上下文消息展示任务背景，**不直接作为 provider system prompt**（与设计方案 :271 一致）。

```rust
        WorkspaceType::WorkItemPlan => {
            "你是 Aria 的候选 work item plan 生成器。你负责基于已确认 Story Spec、Design Spec、Repository 代码上下文和项目规则，整组生成可并行/可调度的 Work Item 候选计划（WorkItem 列表 + 依赖 DAG + 写入范围 + 验证计划 + 仓库画像）；daemon 负责校验、落盘 Draft candidate、逐个 revert/批量 revision 重做与最终 confirm 建立子 Coding session。不要逐个生成或流式逐个产出，必须一次性规划整组。"
        }
```

- [ ] **Step 1.11：`constraint_summary_for` 加 WorkItemPlan 分支**

`src/web/workspace_context.rs:428-447`。该函数结构是 `if session.openspec_enabled { match ... } else { ... }`；在 OpenSpec 分支的 match 内追加 WorkItemPlan：

```rust
            WorkspaceType::WorkItemPlan => {
                "OpenSpec 已启用。Work Item 拆分必须覆盖已确认 Story/Design 的 requirement constraints；每个 WorkItem 必须绑定来源 spec、声明互斥写入范围与可追踪的验证计划，供 daemon 写回 OpenSpec tasks constraints。不要把 OpenSpec 当作 runtime truth。".to_string()
            }
```

> 非 OpenSpec 分支（`else`）对所有类型共用，无需改。

- [ ] **Step 1.12：`workflow_discipline_for` 加 WorkItemPlan 到第一处 match**

`src/web/workspace_context.rs:449-477`。该函数有两个 match：
1. 第一处（:451-459，`if superpowers_enabled`）选 base 文案；
2. 第二处（:464-476，`match (&workspace_type, &author_provider)`）叠加 provider 条款。

WorkItemPlan 的 author 是 `WorkItemSplitter`（非交互式整组生成，不需要结构化 AskUserQuestion），因此 base 文案与 WorkItem 同类（使用 writing-plans 的计划结构，不执行落盘）。在第一处 match 追加：

```rust
            WorkspaceType::WorkItemPlan => {
                "必须遵守 using-superpowers 与 writing-plans 的计划结构要求来生成候选 Work Item Plan artifact（WorkItem 列表 + DAG + 写入范围 + 验证计划 + 仓库画像），不要执行 writing-plans 默认的落盘与执行交接流程。不得直接输出实现代码，先生成可确认的整组拆分计划与候选。不要创建 docs/superpowers/plans 文件，不要询问 Subagent-Driven 或 Inline Execution；daemon 会负责 Draft candidate 落盘、逐个 revert、批量 revision 与 confirm 后建立子 Coding session。".to_string()
            }
```

> 第二处 match（provider 叠加）**不改**：WorkItemPlan 落到 `_ => base`（与 WorkItem 一致，不强制结构化提问）。确认 :464-476 的 `_ => base` 兜底存在即可。

- [ ] **Step 1.13：`output_schema_for` 加 WorkItemPlan 文案**

`src/web/workspace_context.rs:479-506`。文案**必须**包含字面量 `` ```artifact fenced block ``（否则 `all_workspace_artifact_outputs_require_artifact_fence` 测试失败，该测试遍历所有变体）。

```rust
        WorkspaceType::WorkItemPlan => {
            "Work Item Plan 候选必须用 ```artifact fenced block 包裹，且输出结构化 JSON（由 daemon 解析为 plan + work_items + dependency_graph + verification_plans + repository_profile + validator_findings）；fenced block 内第一行必须是 Work Item Plan 一级标题。每个 WorkItem 必须声明 kind、title、depends_on（引用同组其他 WorkItem id）、exclusive_write_scopes 与 verification_plan_ref；dependency_graph 以 from_work_item_id/to_work_item_id 边数组表达。"
        }
```

> 说明：此处文案描述的是 author 阶段 provider 输出形态（与 `WorkItemSplitEngine` 的 JSON Schema 对齐，见 `src/product/work_item_split_engine.rs:44-54`）。WP2 实现 author run 时会直接调 engine，不走 markdown 解析；本 output_schema 文案主要给上下文消息一致性用。

- [ ] **Step 1.14：回归测试 `all_workspace_artifact_outputs_require_artifact_fence` 自动覆盖新变体**

`src/web/workspace_context.rs:540-552` 的测试遍历 `[Story, Design, WorkItem]`——**需要把 `WorkspaceType::WorkItemPlan` 加入遍历数组**：

```rust
        for workspace_type in [
            WorkspaceType::Story,
            WorkspaceType::Design,
            WorkspaceType::WorkItem,
            WorkspaceType::WorkItemPlan,
        ] {
```

（即便不显式加，由于 Step 1.13 的文案已含 fenced block，测试逻辑也能过；但显式加入数组让新变体被枚举覆盖，更稳。）

- [ ] **Step 1.15：运行 Task 1 测试**

Run:
```
cargo test --locked --lib workspace_context
cargo check --locked
```
Expected:
- `work_item_plan_context_message_includes_plan_brief_and_workspace_type` PASS
- `all_workspace_artifact_outputs_require_artifact_fence` PASS（含 WorkItemPlan）
- `cargo check --locked` 全绿（所有 `match WorkspaceType` 穷尽）

> 若 `cargo check` 报其他文件还有非穷尽 match（例如 `src/web/types.rs` 的 DTO 投影、`src/web/handlers.rs` 其它 `workspace_type_text` 之外的 match），**在本 Task 内补齐**——这些是「加枚举变体让编译通过」的必要最小改动，属于本 Task 范围。用 `grep -rn "match.*workspace_type" src/` 定位每一处。补齐时优先返回合理字符串/分支；若某处需要真实业务逻辑（如 DTO 字段映射），给最小合理实现并加注释 `// WP1: minimal branch for exhaustiveness; enriched in later WP if needed`。

- [ ] **Step 1.16：提交**

```bash
git add src/product/models.rs src/web/workspace_context.rs src/web/handlers.rs
git commit -m "feat(WP1): 新增 WorkspaceType::WorkItemPlan 变体与 workspace_context 全分支"
```

（若 Step 1.15 因 exhaustiveness 改了更多文件，一并 `git add`。）

---

## Task 2：WS 契约类型定义 —— `ArtifactPayload` + `WorkItemPlanCandidateDto` + `RevertWorkItem`（纯新增）

**目标**：在 `workspace_ws_types.rs` **纯新增** `ArtifactPayload` enum（serde `untagged`，JSON 表现为方案要求的扁平 `markdown?/diff?/candidate?` 形态）、`WorkItemPlanCandidateDto` 及子 DTO、`WsInMessage::RevertWorkItem` 变体；补 serde 往返单测（含 `ArtifactPayload` 两变体的扁平 JSON 断言）。**不修改任何现有类型**——`WsOutMessage::ArtifactUpdate`（`{ version, markdown, diff }`）与 `SessionState.artifact`（`Option<String>`）保持原样；**完全不触及** `workspace_ws_handler.rs` / `workspace_engine.rs`，`cargo check` 直接通过。`ArtifactPayload` 的真正挂载与构造点切换在 WP2。

**Files:**
- Modify: `src/web/workspace_ws_types.rs`（纯新增类型 + `WsInMessage` 追加 `RevertWorkItem` 变体；**不改 `WsOutMessage::ArtifactUpdate`、不改 `SessionState`**）
- Test: `src/web/workspace_ws_types.rs` 的 `#[cfg(test)] mod tests`（:473+）

**Interfaces:**
- Consumes: 现有 `ProviderName`、`WorkspaceType`（Task 1 已加 `WorkItemPlan`）。
- Produces:
  - `ArtifactPayload` enum（WP2 把 `ArtifactUpdate` / `SessionState.artifact` 切到此类型）
  - `WorkItemPlanCandidateDto` + `WorkItemPlanDto` + `WorkItemCandidateDto` + `WorkItemCandidateMetaDto` + `WorkItemSplitOptionsDto` + `WorkItemDependencyEdgeDto` + `ValidatorFindingDto`（+ 可能的 `VerificationPlanDto` / `RepositoryProfileDto`，若 `src/web/types.rs` 已有则复用）
  - `WsInMessage::RevertWorkItem { work_item_id, feedback, clear }`（WP4 处理逻辑；纯新增变体不影响现有 `WsInMessage` 反序列化）

- [ ] **Step 2.1：写失败测试 —— DTO 往返 + RevertWorkItem 反序列化 + ArtifactPayload 扁平 JSON 形态**

在 `src/web/workspace_ws_types.rs` 的 `#[cfg(test)] mod tests`（:473+）末尾追加四个测试。参考现有测试的 `serde_json::from_str` / `to_string` 模式（先 `grep -n "fn.*serde\|from_str\|to_string" src/web/workspace_ws_types.rs` 看现有测试风格）。

```rust
    #[test]
    fn work_item_plan_candidate_dto_roundtrips_through_serde() {
        let dto = WorkItemPlanCandidateDto {
            plan: WorkItemPlanDto {
                id: "issue_work_item_plan_0001".to_string(),
                status: "draft".to_string(),
                options: WorkItemSplitOptionsDto {
                    include_integration_tests: true,
                    include_e2e_tests: false,
                    force_frontend_backend_split: true,
                    require_execution_plan_confirm: false,
                },
                dependency_graph: vec![WorkItemDependencyEdgeDto {
                    from_work_item_id: "work_item_0001".to_string(),
                    to_work_item_id: "work_item_0002".to_string(),
                }],
            },
            work_items: vec![WorkItemCandidateDto {
                id: "work_item_0001".to_string(),
                kind: "backend".to_string(),
                title: "后端 API".to_string(),
                depends_on: vec![],
                exclusive_write_scopes: vec!["src/api".to_string()],
                verification_plan_ref: Some("verification_plan_0001".to_string()),
                meta: WorkItemCandidateMetaDto {
                    reverted: false,
                    revert_feedback: None,
                },
            }],
            verification_plans: Vec::new(),
            repository_profile: None,
            validator_findings: Vec::new(),
        };
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: WorkItemPlanCandidateDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
        assert!(json.contains("\"work_items\""));
        assert!(json.contains("\"exclusive_write_scopes\""));
        assert!(json.contains("\"verification_plan_ref\""));
    }

    #[test]
    fn revert_work_item_message_deserializes() {
        let json = r#"{"type":"revert_work_item","work_item_id":"work_item_0001","feedback":"拆得太粗","clear":false}"#;
        let msg: WsInMessage = serde_json::from_str(json).expect("deserialize");
        match msg {
            WsInMessage::RevertWorkItem { work_item_id, feedback, clear } => {
                assert_eq!(work_item_id, "work_item_0001");
                assert_eq!(feedback.as_deref(), Some("拆得太粗"));
                assert!(!clear);
            }
            other => panic!("expected RevertWorkItem, got {other:?}"),
        }
    }

    #[test]
    fn artifact_payload_markdown_variant_serializes_to_flat_json() {
        let payload = ArtifactPayload::Markdown {
            markdown: "# 标题".to_string(),
            diff: None,
        };
        let json = serde_json::to_string(&payload).expect("serialize");
        assert!(json.contains("\"markdown\""));
        assert!(json.contains("# 标题"));
        assert!(!json.contains("\"candidate\""));
        // untagged：不得携带 kind/tag 标签（符合设计方案扁平形态）
        assert!(!json.contains("\"kind\""));
        let back: ArtifactPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(payload, back);
    }

    #[test]
    fn artifact_payload_candidate_variant_serializes_to_flat_json() {
        let candidate = WorkItemPlanCandidateDto {
            plan: WorkItemPlanDto {
                id: "plan_1".to_string(),
                status: "draft".to_string(),
                options: WorkItemSplitOptionsDto {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                dependency_graph: Vec::new(),
            },
            work_items: Vec::new(),
            verification_plans: Vec::new(),
            repository_profile: None,
            validator_findings: Vec::new(),
        };
        let payload = ArtifactPayload::WorkItemPlanCandidate { candidate };
        let json = serde_json::to_string(&payload).expect("serialize");
        assert!(json.contains("\"candidate\""));
        assert!(!json.contains("\"markdown\""));
        assert!(!json.contains("\"kind\""));
        let back: ArtifactPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(payload, back);
    }
```

> test mod 的 `use` 需补：`use super::{ArtifactPayload, WsInMessage, WorkItemPlanCandidateDto, WorkItemPlanDto, WorkItemSplitOptionsDto, WorkItemDependencyEdgeDto, WorkItemCandidateDto, WorkItemCandidateMetaDto};`（以实际定义名为准）。

- [ ] **Step 2.2：运行测试，确认失败（类型未定义）**

Run: `cargo test --locked --lib workspace_ws_types`
Expected: 编译失败——`ArtifactPayload` / `WorkItemPlanCandidateDto` 等类型未定义、`WsInMessage::RevertWorkItem` 不存在。

- [ ] **Step 2.3：定义 `WorkItemPlanCandidateDto` 及子 DTO**

在 `src/web/workspace_ws_types.rs` 类型定义区（建议放在 `ArtifactVersion` :423 之前）新增。字段命名严格对齐设计方案 :173-185 的 `WorkItemPlanCandidateDto` 结构与 `WorkItemSplitProviderOutput`（`src/product/work_item_split_engine.rs:133-139`）。

```rust
/// WorkItemPlan workspace 的候选产物 DTO（artifact payload 的结构化镜像）。
///
/// WP1 阶段：仅定义类型与 serde 契约，供 WP2 把 `ArtifactUpdate` /
/// `SessionState.artifact` 切到 `ArtifactPayload::WorkItemPlanCandidate { candidate }` 时使用。
/// 事实来源是 LifecycleStore 的 Draft `IssueWorkItemPlan` / Draft `LifecycleWorkItemRecord` /
/// `VerificationPlan` / `RepositoryProfile`；本 DTO 由后端在推送前组装。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanCandidateDto {
    pub plan: WorkItemPlanDto,
    pub work_items: Vec<WorkItemCandidateDto>,
    pub verification_plans: Vec<VerificationPlanDto>,
    pub repository_profile: Option<RepositoryProfileDto>,
    pub validator_findings: Vec<ValidatorFindingDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanDto {
    pub id: String,
    pub status: String,
    pub options: WorkItemSplitOptionsDto,
    pub dependency_graph: Vec<WorkItemDependencyEdgeDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemSplitOptionsDto {
    pub include_integration_tests: bool,
    pub include_e2e_tests: bool,
    pub force_frontend_backend_split: bool,
    pub require_execution_plan_confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemDependencyEdgeDto {
    pub from_work_item_id: String,
    pub to_work_item_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemCandidateDto {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub depends_on: Vec<String>,
    pub exclusive_write_scopes: Vec<String>,
    pub verification_plan_ref: Option<String>,
    pub meta: WorkItemCandidateMetaDto,
}

/// AuthorConfirm 阶段的 revert 标记态（标记不产生新 version，只在当前 version 改 meta）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemCandidateMetaDto {
    #[serde(default)]
    pub reverted: bool,
    #[serde(default)]
    pub revert_feedback: Option<String>,
}
```

> `VerificationPlanDto` / `RepositoryProfileDto` / `ValidatorFindingDto` 处理：
> - 先 `grep -n "struct VerificationPlanDto\|struct RepositoryProfileDto\|struct WorkItemSplitFinding" src/web/` 确认是否已有等价 DTO。
> - 若已有（可能在 `src/web/types.rs`），在本文件 `use crate::web::types::{...}` 引入并复用，**不要重复定义**。
> - 若无，定义最小 DTO（字段对齐 `src/product/models.rs` 的 `VerificationPlan` / `RepositoryProfile` / `WorkItemSplitFinding`）。例：
>
> ```rust
> #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
> #[serde(rename_all = "snake_case")]
> pub struct ValidatorFindingDto {
>     pub severity: String,
>     pub code: String,
>     pub message: String,
>     #[serde(default)]
>     pub work_item_ids: Vec<String>,
> }
> ```
>
> `VerificationPlanDto` / `RepositoryProfileDto` 同理取关键字段（review 展示用，不必全字段）。**实现时读 `src/product/models.rs` 对应结构体的字段清单决定 DTO 字段**。

- [ ] **Step 2.4：定义 `ArtifactPayload` enum（`untagged`，WP2 挂载）**

在 `WorkItemPlanCandidateDto` 之后定义。**本 WP 只定义不挂载**（不改 `WsOutMessage::ArtifactUpdate` / `SessionState.artifact`）。

```rust
/// Artifact payload 的互斥 union（设计方案 :204-209）。
///
/// serde `untagged` 让 JSON 表现为扁平字段形态，符合方案要求：
/// - Markdown 变体 → `{"markdown": "...", "diff": "..." | null}`
/// - WorkItemPlanCandidate 变体 → `{"candidate": {...}}`
/// 两变体字段集互不重叠（markdown vs candidate），untagged 能正确区分。
///
/// WP1 只定义类型并测其 serde 形态；WP2 把 `WsOutMessage::ArtifactUpdate`
/// 与 `SessionState.artifact` 切换为携带 `ArtifactPayload` 的形态，
/// 并同步改 `workspace_ws_handler.rs` / `workspace_engine.rs` 的构造/消费点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", untagged)]
pub enum ArtifactPayload {
    Markdown {
        markdown: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
    },
    WorkItemPlanCandidate {
        candidate: WorkItemPlanCandidateDto,
    },
}
```

> `untagged` 反序列化歧义检查：两个变体的必填字段名（`markdown` vs `candidate`）不重叠，`serde_json` 能无歧义匹配。Step 2.1 的两个 `artifact_payload_*` 测试断言 `!json.contains("\"kind\"")` 验证未注入 tag。**若未来新增第三个变体且字段与现有重叠，需改回 `tag` 形态或自定义 Visitor。**

- [ ] **Step 2.5：`WsInMessage` 加 `RevertWorkItem` 变体**

`src/web/workspace_ws_types.rs:135-190`，在 `Abort`（:188）/ `Ping`（:189）之前或之后追加：

```rust
    RevertWorkItem {
        work_item_id: String,
        feedback: Option<String>,
        #[serde(default)]
        clear: bool,
    },
```

> 注意：`WsInMessage` 用 `#[serde(tag = "type", rename_all = "snake_case")]`，故 JSON tag 为 `"revert_work_item"`。新增变体不影响现有变体（Story/Design 的 `StartGeneration` / `AuthorDecision` 等仍正常反序列化）。`feedback` 用 `Option<String>`（用户可只点 revert 不写反馈），`clear: bool` 默认 false（标记），true 表示取消标记。**WP4 才实现该消息的 handler 逻辑；本 WP 仅定义变体。**

- [ ] **Step 2.6：运行 Task 2 测试 + cargo check**

Run:
```
cargo test --locked --lib workspace_ws_types
cargo check --locked
```
Expected:
- 四个新测试 PASS（含两个 `artifact_payload_*` 扁平 JSON 断言）
- `cargo check --locked` 全绿——**纯新增类型，未触及任何现有类型/构造点**，无需改 `workspace_ws_handler.rs` / `workspace_engine.rs`。

- [ ] **Step 2.7：提交**

```bash
git add src/web/workspace_ws_types.rs
git commit -m "feat(WP1): WS 契约类型 ArtifactPayload/WorkItemPlanCandidateDto/RevertWorkItem（纯新增）"
```

> 仅 `workspace_ws_types.rs` 一个文件——这是选项 B 的边界保证：WP1 不碰 engine/handler。若 `git status` 显示其他文件被动到，说明误改了构造点，回退后重做。

---

## Task 3：`prepare_work_item_plan` handler + 路由 + DTO + 集成测试

**目标**：新增 `POST /work-item-plans:prepare` handler，创建空 Draft `IssueWorkItemPlan`（`work_item_ids` / `verification_plan_ids` / `dependency_graph` 初始为空）+ `WorkspaceType::WorkItemPlan` session（`entity_id = plan_id`）+ 注入上下文消息，不调 Provider、不建子 session；新增对应请求/响应 DTO 与路由；集成测试验证 prepare 不触发生成、不建 WorkItem 子 session。

**Files:**
- Modify: `src/web/types.rs`（新增 `PrepareWorkItemPlanRequest` / `PrepareWorkItemPlanResponse`）
- Modify: `src/web/handlers.rs`（新增 `prepare_work_item_plan` handler；`use` 导入新 DTO）
- Modify: `src/web/app.rs`（注册 `POST .../work-item-plans:prepare` 路由）
- Test: `tests/it_web/web_work_item_generation.rs`（追加测试）或新增 `tests/it_web/web_work_item_plan_prepare.rs` + 在 `tests/it_web.rs` 加 `#[path] mod`

**Interfaces:**
- Consumes: Task 1 的 `WorkspaceType::WorkItemPlan`；`provider_workspace_config`（:3113）；`validate_confirmed_design_specs`（现有 private helper，见 `generate_work_items` :512-561）；`IssueStore::get`；`LifecycleStore::create_issue_work_item_plan` + `create_workspace_session` + `ensure_workspace_context_message`；DTO 投影 helper（`workspace_session_dto`，:2710 附近）；新增 `issue_work_item_plan_detail_dto`，显式把 product model 投影为 prepare 专用完整 DTO，避免与现有 web 轻量 `IssueWorkItemPlan { plan_id, ... }` 混淆。
- Produces: `POST /api/projects/{project_id}/issues/{issue_id}/work-item-plans:prepare` → `PrepareWorkItemPlanResponse { work_item_plan, workspace_session }`；被 WP6 前端 `prepareWorkItemPlan` API client 依赖。

- [ ] **Step 3.1：写失败测试 —— prepare 创建 Draft plan + session，不生成 WorkItem**

测试位置：**优先追加到 `tests/it_web/web_work_item_generation.rs`**（复用 `app_with_confirmed_story_and_design` 夹具，该夹具已 `pub(crate)` 暴露）；若该文件已过大或命名不直观，新增 `tests/it_web/web_work_item_plan_prepare.rs` 并在 `tests/it_web.rs` 按 `#[path = "..."] mod xxx;` 模式注册（参考现有 mod 声明）。

```rust
#[tokio::test]
async fn prepare_work_item_plan_creates_draft_plan_and_session_without_generating() {
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (status, response) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false,
            "review_rounds": 1
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 返回 Draft plan，空 work_item_ids / verification_plan_ids / dependency_graph
    let plan = &response["work_item_plan"];
    assert_eq!(plan["status"], "draft");
    assert!(plan["work_item_ids"].as_array().unwrap().is_empty());
    assert!(plan["verification_plan_ids"].as_array().unwrap().is_empty());
    assert!(plan["dependency_graph"].as_array().unwrap().is_empty());
    let plan_id = plan["id"].as_str().unwrap().to_string();

    // 返回 WorkItemPlan session，entity_id == plan_id，workspace_type == work_item_plan
    let session = &response["workspace_session"];
    assert_eq!(session["workspace_type"], "work_item_plan");
    assert_eq!(session["entity_id"], plan_id);

    // 不调用 Provider：无 work item 被创建、无 WorkItem 子 session 被创建
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(_repo.path().join(".aria")));
    let work_items = lifecycle
        .list_work_items("project_0001", "issue_0001")
        .expect("list work items");
    assert!(work_items.is_empty(), "prepare must not create any work items");

    // session 的首条消息是上下文消息（含 WorkItemPlan 文案）
    let sessions = lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("list sessions");
    let plan_session = sessions
        .iter()
        .find(|s| s.workspace_type == WorkspaceType::WorkItemPlan)
        .expect("work item plan session exists");
    assert!(plan_session.messages[0]
        .content
        .contains("候选 work item plan 生成器"));
}
```

> 实现者注意：
> 1. test mod 的 `use`：`use super::{app_with_confirmed_story_and_design, request_json, valid_split_output};` + `use axum::http::{Method, StatusCode};` + `use serde_json::json;` + `use crate::product::app_paths::ProductAppPaths;` + `use crate::product::lifecycle_store::LifecycleStore;` + `use crate::product::models::WorkspaceType;`。
> 2. `app_with_confirmed_story_and_design` 返回的 story id 是 `story_spec_0001`、design id 是 `design_spec_0001`（夹具内固定 id；实现者读 `web_work_item_generation.rs:416-503` 确认实际 id 字面值，若不同则调整测试的 `story_spec_ids`/`design_spec_ids`）。
> 3. `valid_split_output()` 在本测试里**不会被消费**（prepare 不调 Provider）；但夹具要求传入，保持夹具签名不变。
> 4. `review_rounds: 1` 触发 `provider_workspace_config` 的默认校验（1..=5）。

- [ ] **Step 3.2：运行测试，确认失败（404 —— 路由不存在）**

Run: `cargo test --locked --test it_web prepare_work_item_plan_creates_draft_plan_and_session_without_generating`
Expected: `status` 断言失败（404，因路由未注册）或编译失败（handler 不存在）。

- [ ] **Step 3.3：定义 DTO**

`src/web/types.rs`，参考 `GenerateWorkItemsRequest`（:564-579）/ `GenerateStorySpecsResponse`（:537-542）的 derive 与字段风格。先 `grep -n "struct GenerateWorkItemsRequest\|struct GenerateStorySpecsResponse\|struct IssueWorkItemPlan" src/web/types.rs` 定位插入点。

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PrepareWorkItemPlanRequest {
    pub title: String,
    #[serde(default)]
    pub story_spec_ids: Vec<String>,
    #[serde(default)]
    pub design_spec_ids: Vec<String>,
    pub include_integration_tests: Option<bool>,
    pub include_e2e_tests: Option<bool>,
    pub force_frontend_backend_split: Option<bool>,
    pub require_execution_plan_confirm: Option<bool>,
    pub author_provider: Option<String>,
    pub reviewer_provider: Option<String>,
    pub review_rounds: Option<u32>,
    pub superpowers_enabled: Option<bool>,
    pub openspec_enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PrepareWorkItemPlanResponse {
    pub work_item_plan: IssueWorkItemPlanDetailDto,
    pub workspace_session: WorkspaceSessionDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemPlanDetailDto {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub source_story_spec_ids: Vec<String>,
    pub source_design_spec_ids: Vec<String>,
    pub options: IssueWorkItemPlanOptions,
    pub status: IssueWorkItemPlanStatus,
    pub work_item_ids: Vec<String>,
    pub repository_profile_ref: Option<String>,
    pub verification_plan_ids: Vec<String>,
    pub dependency_graph: Vec<IssueWorkItemDependencyEdge>,
    pub created_from_provider_run: Option<String>,
    pub validator_findings: Vec<WorkItemSplitFinding>,
    pub review_summary: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
```

> 不复用现有 web DTO `IssueWorkItemPlan`：`src/web/types.rs` 已有轻量 DTO，字段是 `plan_id/status/options/created_at/updated_at`，不含 `id/work_item_ids/verification_plan_ids/dependency_graph`。prepare 必须返回完整 detail DTO，前端 WP6 也用同名 detail 类型对齐。`WorkspaceSessionDto` 已存在（types.rs:685 附近），其 id 字段名是 `workspace_session_id`，测试和前端 mock 不得写成 `id`。`use` 导入以现有 types.rs 顶部 import 风格为准。

- [ ] **Step 3.4：实现 `prepare_work_item_plan` handler**

`src/web/handlers.rs`，参考 `generate_story_specs`（:406-458）骨架。先 `grep -n "pub async fn generate_story_specs\|fn validate_confirmed_design_specs\|fn product_app_paths\|fn workspace_session_dto" src/web/handlers.rs` 确认 helper 签名。

```rust
pub async fn prepare_work_item_plan(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<PrepareWorkItemPlanRequest>,
) -> ApiResult<Json<PrepareWorkItemPlanResponse>> {
    let workspace_config = provider_workspace_config(
        request.author_provider.as_deref(),
        request.reviewer_provider.as_deref(),
        request.review_rounds,
        request.superpowers_enabled,
        request.openspec_enabled,
        &*state.provider_availability,
    )?;
    let app_paths = product_app_paths(&state);
    let issue = IssueStore::new(app_paths.clone())
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let _repository_id = issue
        .repo_id
        .clone()
        .ok_or_else(|| ApiError::validation("repository_required", "repository_id is required"))?;
    let lifecycle = LifecycleStore::new(app_paths.clone());
    // 校验传入的 story/design spec 存在且已确认（prepare 从 Design Spec 进入）
    validate_confirmed_story_specs(&lifecycle, &project_id, &issue_id, &request.story_spec_ids)?;
    validate_confirmed_design_specs(&lifecycle, &project_id, &issue_id, &request.design_spec_ids)?;

    // 创建空 Draft plan：work_item_ids / verification_plan_ids / dependency_graph 初始为空
    let plan = lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: None,
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            source_story_spec_ids: request.story_spec_ids,
            source_design_spec_ids: request.design_spec_ids,
            options: IssueWorkItemPlanOptions {
                include_integration_tests: request.include_integration_tests.unwrap_or(true),
                include_e2e_tests: request.include_e2e_tests.unwrap_or(false),
                force_frontend_backend_split: request.force_frontend_backend_split.unwrap_or(false),
                require_execution_plan_confirm: request
                    .require_execution_plan_confirm
                    .unwrap_or(false),
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .map_err(product_store_api_error)?;

    // 创建 WorkItemPlan session，entity_id 指向 plan.id（author/revision 阶段据此读 plan）
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id,
            issue_id,
            entity_id: plan.id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: workspace_config.author_provider,
            reviewer_provider: workspace_config.reviewer_provider,
            review_rounds: workspace_config.review_rounds,
            superpowers_enabled: workspace_config.superpowers_enabled,
            openspec_enabled: workspace_config.openspec_enabled,
        })
        .map_err(product_store_api_error)?;

    // 注入上下文消息（含 WorkItemPlan 文案，Task 1 已支持）
    let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
        .map_err(product_store_api_error)?;

    Ok(Json(PrepareWorkItemPlanResponse {
        work_item_plan: issue_work_item_plan_detail_dto(plan),
        workspace_session: workspace_session_dto(session),
    }))
}
```

> 实现者注意：
> 1. `handlers.rs` 顶部 `use` 补：`use crate::web::types::{IssueWorkItemPlanDetailDto, PrepareWorkItemPlanRequest, PrepareWorkItemPlanResponse};` 以及确认 `CreateIssueWorkItemPlanInput`、`IssueWorkItemPlanOptions`、`IssueWorkItemPlanStatus` 已从 `crate::product::{lifecycle_store, models}` 导入（参考 `persist_work_item_split_provider_output` :589-703 的 import 方式——它已经用过这些类型，说明 import 路径已通）。
> 2. **`options` 默认值**：上述 `unwrap_or(...)` 是合理默认。若想与现有 `generate_work_items` 完全一致，先 `grep -n "include_integration_tests\|include_e2e_tests\|force_frontend_backend_split\|require_execution_plan_confirm" src/web/handlers.rs src/product/work_item_split_engine.rs` 看现有代码怎么从 `Option<bool>` 构造 `IssueWorkItemPlanOptions`（很可能在 `persist_work_item_split_provider_output` 或 engine 内），**照抄其 unwrap 默认值**。
> 3. **不调 Provider、不建 WorkItem、不建子 session**：handler 里不出现 `WorkItemSplitEngine`、`create_work_item`、`create_workspace_session(WorkspaceType::WorkItem, ...)`。这是「prepare 不生成」的硬约束。
> 4. `validate_confirmed_design_specs` 的签名以实际为准（`generate_work_items` :512-561 调用了它；若签名是 `fn validate_confirmed_design_specs(&LifecycleStore, &str, &str, &[String]) -> Result<(), ApiError>`，照抄）。
> 5. 新增 `issue_work_item_plan_detail_dto(plan: IssueWorkItemPlan) -> IssueWorkItemPlanDetailDto` helper，逐字段拷贝 product model 字段；不要把 `src/web/types.rs` 现有轻量 `IssueWorkItemPlan` 当作 prepare 响应类型。

- [ ] **Step 3.5：注册路由**

`src/web/app.rs`，在 `work-items:generate`（:79-82）**之前**（新流程入口排在旧流程前面更清晰）插入：

```rust
        .route(
            "/api/projects/{project_id}/issues/{issue_id}/work-item-plans:prepare",
            post(handlers::prepare_work_item_plan),
        )
```

> 注意：`work-items:generate`（:79-82）、`work-item-plans/{plan_id}/confirm`（:83-86）、`work-item-plans/{plan_id}/change-request`（:87-90）**在本 WP 保留不删**（WP5 才删）。

- [ ] **Step 3.6：运行 Task 3 集成测试**

Run:
```
cargo test --locked --test it_web prepare_work_item_plan_creates_draft_plan_and_session_without_generating
cargo test --locked --test it_web web_work_item_generation   # 确保未破坏现有 generate/confirm 测试
```
Expected: 新测试 PASS；现有 `web_work_item_generation` 全部 PASS（路由新增不影响旧路由）。

- [ ] **Step 3.7：提交**

```bash
git add src/web/types.rs src/web/handlers.rs src/web/app.rs tests/it_web/web_work_item_generation.rs tests/it_web.rs
git commit -m "feat(WP1): prepare_work_item_plan handler 与 POST /work-item-plans:prepare 路由"
```

---

## Task 4：WP1 收口验证（全量回归）

**目标**：跑完整验证链，确保本 WP 的改动未破坏 Story/Design/WorkItem 既有流程；确认 `WorkspaceType` serde 往返与所有 WorkItemPlan 分支符合预期。

**Files:** 无新增改动；仅运行验证命令。若验证暴露真实缺陷，回到对应 Task 修复（不扩大范围）。

- [ ] **Step 4.1：全量验证链**

Run（依次，任一失败即停并修复）:
```
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked --lib workspace_context
cargo test --locked --lib workspace_ws_types
cargo test --locked --test it_web prepare_work_item_plan_creates_draft_plan_and_session_without_generating
cargo test --locked --test it_web web_work_item_generation
cargo test --locked --test it_web web_lifecycle_api
```

Expected: 全绿。

> 说明：
> - `cargo clippy --all-targets --all-features --locked -- -D warnings` 是强制项，不能省（见全局约束）。
> - `web_lifecycle_api` 覆盖 Story/Design generate 的 provider fallback，确保新增 `WorkspaceType::WorkItemPlan` 未影响 DTO 序列化与 provider 配置解析。
> - 若 `cargo test --locked`（全量）时间允许，最后跑一次全量 `cargo test --locked` 作为最终回归（可选，定向测试已覆盖核心）。

- [ ] **Step 4.2：确认废弃路由仍可用（WP5 才删）**

Run:
```
cargo test --locked --test it_web confirm_issue_work_item_plan_marks_work_items_confirmed
cargo test --locked --test it_web generate_work_items
```
Expected: PASS（本 WP 不删路由，旧 `generate_work_items` / `confirm` / `change-request` 流程仍工作；WP5 才删除并改为 404）。

- [ ] **Step 4.3：交付摘要（供 WP2 前置交付摘要使用）**

在 commit 后，把以下内容写入 commit notes 或后续 WP2 plan 的「前置交付摘要」章节：

- `WorkspaceType::WorkItemPlan` 变体已就位，serde 序列化为 `"work_item_plan"`。
- `workspace_context.rs` 全部 10 处分支已支持 WorkItemPlan；`ensure_workspace_context_message` 对 WorkItemPlan session 注入含「候选 work item plan 生成器」的上下文消息；entity context 的 title 形如 `Work Item Plan ({plan_id})`，`linked_context` 拼接 source story + design。
- `POST /work-item-plans:prepare` 已可用，返回 `PrepareWorkItemPlanResponse { work_item_plan: IssueWorkItemPlanDetailDto(id=plan_id, status=Draft, 空集合), workspace_session(workspace_session_id, entity_id=plan_id, workspace_type=work_item_plan) }`。注意 `workspace_session` 的 id 字段名是 `workspace_session_id`，不是 `id`。
- WS 契约：`WsOutMessage::ArtifactUpdate` 新增 `candidate: Option<WorkItemPlanCandidateDto>`（Story/Design 调用方填 None）；`WsInMessage::RevertWorkItem { work_item_id, feedback, clear }` 已定义（WP4 处理）；`ArtifactPayload` enum 已定义（WP2 切换）。
- **WP2 待办**：把 `WsOutMessage::ArtifactUpdate` 与 `SessionState.artifact` 切换到 `ArtifactPayload`（即 `ArtifactUpdate` 携带 `payload: ArtifactPayload`、`SessionState.artifact: Option<ArtifactPayload>`），同步改 `workspace_ws_handler.rs:270` 与 `workspace_engine.rs` 的所有构造/消费点。WP1 已定义 `ArtifactPayload`（`untagged` 扁平 JSON）与 `WorkItemPlanCandidateDto`，WP2 直接挂载、无需重新定义。

---

## Self-Review（写完后的自查）

**1. Spec 覆盖**（对照总览 WP1 的目标/写入范围/验证）：
- ✅ `WorkspaceType::WorkItemPlan` 枚举 → Task 1 Step 1.3
- ✅ `workspace_context.rs` 全部分支（10 处）→ Task 1 Step 1.4-1.14
- ✅ `prepare_work_item_plan` handler + 路由 → Task 3
- ✅ artifact payload union（契约类型定义，选项 B：纯新增不挂载）→ Task 2 定义 `ArtifactPayload` + `WorkItemPlanCandidateDto` + `RevertWorkItem`，**不改 `ArtifactUpdate`/`SessionState`，不触及 engine/handler**；真正的 union 挂载在 WP2。
- ✅ prepare 创建空 Draft `IssueWorkItemPlan` + session（`entity_id = plan_id`）+ 注入上下文 → Task 3 Step 3.4
- ✅ serde 往返不丢 candidate payload → Task 2 Step 2.1 测试 + Task 1 Step 1.1 测试
- ✅ 验证命令链（fmt/clippy/check + 定向测试）→ Task 4
- ✅ 不做项：未实现 engine 分支（WP2–WP5）、未实现 RevertWorkItem 处理（WP4）、未删 REST 路由（WP5）、未改前端（WP6/WP7）—— 均在 File Structure 的「不改」清单与各 Task 的「不做」里标注。
- 🔎 总览 WP1 写入范围列了 `tests/it_product/product_workspace_context.rs`（若存在）—— 探查确认该文件**不存在**，workspace_context 单测内联在源文件 `#[cfg(test)] mod tests`（:523+），Task 1 Step 1.1 已落到正确位置。

**2. Placeholder 扫描**：
- 无「TBD/TODO/类似 Task N」占位；每个代码 step 给出真实代码或精确 `grep` 定位指引。
- 文案类分支（system_prompt / constraint_summary / workflow_discipline / output_schema）全部给出最终中文文案。
- `VerificationPlanDto` / `RepositoryProfileDto` 给出「先 grep 确认，若无则按 models.rs 字段定义最小 DTO」的明确决策路径——这不是占位符，是因为这两个 DTO 是否已存在需实现时确认；给出了 fallback 定义模板。

**3. 类型一致性**：
- `WorkspaceType::WorkItemPlan` 在 models.rs、workspace_context.rs、handlers.rs `workspace_type_text`、types.rs DTO 中名称一致。
- `WorkItemPlanCandidateDto` 字段与设计方案 :173-185 + `WorkItemSplitProviderOutput`（engine :133-139）对齐：plan / work_items / verification_plans / repository_profile / validator_findings。
- `WsInMessage::RevertWorkItem { work_item_id, feedback: Option<String>, clear: bool }` 与设计方案 :328-335 一致（设计 :333 的 `feedback` 是示例字符串，本 plan 用 `Option<String>` 允许无反馈标记——与 :281「可取消（clear:true）」语义一致；WP4 处理逻辑时若要求 feedback 必填再收紧）。
- `find_issue_work_item_plan` helper 的错误包装方式明确要求「照抄现有 find_story_spec/find_work_item」——避免臆造 API。

**4. 边界风险**：
- Task 2 选项 B：WP1 **完全不触及** `workspace_ws_handler.rs` / `workspace_engine.rs`（纯新增类型，不改 `ArtifactUpdate`/`SessionState`），故无跨界风险。`ArtifactPayload` 的挂载与构造点切换全部留到 WP2（WP2 写入范围本就含这两个文件）。Step 2.7 的提交只含 `workspace_ws_types.rs` 一个文件，是边界保证。
- Task 1 Step 1.15 的 exhaustiveness 检查可能发现更多 `match workspace_type` 点——已要求在 Task 1 内补齐最小分支（非业务逻辑）。

---

## Execution Handoff

本 WP1 plan 已保存至 `cadence/plans/2026-06-17_计划文档_实施计划_WorkItem对话式Workspace生成_WP1_后端枚举与context与prepare_v1.0.md`。

**执行方式（待用户选择）：**

1. **Subagent-Driven（推荐）** — 每个 Task 派一个 fresh subagent 实现，Task 间做 two-stage review，快速迭代。
2. **Inline Execution** — 在当前 session 按 superpowers:executing-plans 批量执行，带 checkpoint 审查。

**完成后**：交付 WP1 后，按同样模板与标准继续 WP2（后端 author 生成 + Draft candidate 持久化，含 `ArtifactPayload` union 真正切换）。WP2 的「前置交付摘要」直接引用本 plan Task 4 Step 4.3 的产出。
