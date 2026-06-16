# WorkItem 拆分 P1 后端模型收敛与调度器迁移 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 Work Item 调度基础收敛到活跃模型 `LifecycleWorkItemRecord`，删除孤立旧模型和空壳 store，为后续 Work Item Set 与 SplitValidator 提供单一事实源。

**Architecture:** 只改后端模型与纯函数调度器，不接入生成流、不改 Coding Workspace、不改前端。先给 `LifecycleWorkItemRecord` 增加拆分所需的兼容字段，再迁移 `ready_work_items()` 到活跃模型并保留原依赖/写入范围判定算法，最后删除旧 `WorkItemRecord` 与 `WorkItemStore`。

**Tech Stack:** Rust 1.95.0、Serde JSON、Cargo integration tests、TDD、OpenSpec、Superpowers。

**版本：** v1.1

> **v1.1 修订摘要：** 1) 将 `src/product/lifecycle_store.rs` 补进 File Structure 写入范围与 Task 4 的 `git add` 提交清单（给 `LifecycleWorkItemRecord` 加字段必须同步改 `lifecycle_store.rs:254` 的字面量构造点，否则编译失败却漏提交）；2) 在前置上下文与 Task 4 记录两项已评估风险：迁移后 `product::models::ExecutionMode` 成为无使用者的孤儿 `pub enum`（不触发 `dead_code`），以及新 scheduler 保留的 `ReadyDecision::NotAgentExecutable` variant 无构造分支，二者均确认 `-D warnings` 仍可通过。

---

## Plan Size Guard

本计划必须在一个 session 内完成，范围故意限制为模型收敛和调度器迁移。

本计划不做以下内容：

- 不新增 `IssueWorkItemPlan` 持久化。
- 不实现 `WorkItemSplitValidator`。
- 不修改 `generate_work_items` 多 Work Item 创建流程。
- 不修改 Coding Workspace 启动门禁。
- 不修改 Issue 共享 worktree。
- 不修改前端。
- 不写贯通/E2E 测试。

如果执行时发现必须修改 `src/web/handlers.rs`、`src/product/coding_workspace_engine.rs` 或 `web/**` 才能继续，应停止并拆出新计划。

## 前置上下文

- 设计方案：`cadence/designs/2026-06-16_技术方案_WorkItem拆分与Coding计划门禁优化_v1.1.md`
- 设计评审：`cadence/designs-reviews/2026-06-16_设计评审_WorkItem拆分与Coding计划门禁优化_v1.0.md`
- 拆分总览：`cadence/plans/2026-06-16_计划文档_实施计划_WorkItem拆分与Coding计划门禁优化_拆分总览_v1.1.md`

当前代码事实：

- `src/product/models.rs` 中 `WorkItemRecord` 是旧模型，只被 `worktree_scheduler.rs` 和 `tests/it_core/work_item_scheduler.rs` 使用。
- `src/product/work_item_store.rs` 只有空壳 `WorkItemStore`，没有业务消费者。
- 活跃链路使用 `LifecycleWorkItemRecord`。
- `worktree_scheduler.rs` 已有 `ReadyDecision::{Ready, WaitingForDependency, WaitingForScope, NotAgentExecutable, NotPending}` 和 scope overlap 判定，应迁移保留。

## 已评估风险

- **`product::models::ExecutionMode` 迁移后成为孤儿 `pub enum`**：本计划删除 `WorkItemRecord` 后，`ExecutionMode` 的唯一字段使用者（`WorkItemRecord::execution_mode`，`models.rs:187`）随之消失。由于它是 `pub` 项，Rust 的 `dead_code` lint 不会对其报警，因此 `-D warnings` 仍可通过。本计划按 Task 3 Step 1 的说明保留 `ExecutionMode`，把清理评估留给后续计划（注意它与 `protocol::projections::ExecutionMode` 是不同类型）。
- **`ReadyDecision::NotAgentExecutable` variant 在新 scheduler 中无构造分支**：迁移后的 `ready_work_items()` 不再产出该 variant（旧 `ExecutionMode` 判定已移除）。该 variant 作为 `pub enum` 成员同样不触发 `dead_code`，`-D warnings` 仍可通过；保留它是为了与旧 scheduler 的 API 兼容，留待 P3 接入 agent 可执行性判定时复用。

## File Structure

- Modify: `src/product/models.rs`
  - 删除旧 `WorkItemRecord`。
  - 增加 `WorkItemKind`、`WorkItemContextBudget`、`WorkItemExecutionPlanStatus`。
  - 扩展 `LifecycleWorkItemRecord`，新增拆分/调度字段，并使用 serde default 保持旧 JSON 可读。
- Modify: `src/product/lifecycle_store.rs`
  - `create_work_item` 中 `LifecycleWorkItemRecord` 字面量构造点（`lifecycle_store.rs:254`）必须补齐新增字段的显式默认值，否则结构体新增字段后该构造点会编译失败。
  - 补充导入 `WorkItemContextBudget`、`WorkItemExecutionPlanStatus`、`WorkItemKind`。
- Modify: `src/product/worktree_scheduler.rs`
  - `ready_work_items()` 入参改为 `&[LifecycleWorkItemRecord]`。
  - 依赖读取 `depends_on`。
  - 写入范围读取 `exclusive_write_scopes`。
  - agent 可执行性暂时按 `execution_status == Pending` 且非 blocked 处理，不再依赖旧 `ExecutionMode`。
- Delete: `src/product/work_item_store.rs`
  - 删除空壳 store。
- Modify: `src/product/mod.rs`
  - 删除 `pub mod work_item_store;`。
- Modify: `tests/it_core/work_item_scheduler.rs`
  - 测试迁移到 `LifecycleWorkItemRecord`。
- Modify: `tests/it_product.rs`
  - 引入新增 product 模型兼容测试模块。
- Create: `tests/it_product/product_work_item_models.rs`
  - 覆盖旧 JSON 反序列化默认值，保证已有 Work Item 数据不因新增字段损坏。

## Task 1: Add Lifecycle Work Item Split Fields With Serde Defaults

**Files:**

- Modify: `src/product/models.rs`
- Create: `tests/it_product/product_work_item_models.rs`
- Modify: `tests/it_product.rs`

- [ ] **Step 1: Write failing compatibility tests**

Create `tests/it_product/product_work_item_models.rs`:

```rust
use cadence_aria::product::models::{
    LifecycleWorkItemRecord, WorkItemContextBudget, WorkItemExecutionPlanStatus, WorkItemKind,
    WorkItemPlanStatus, WorkItemStatus,
};

#[test]
fn lifecycle_work_item_deserializes_legacy_json_with_split_defaults() {
    let json = serde_json::json!({
        "id": "work_item_0001",
        "project_id": "project_0001",
        "issue_id": "issue_0001",
        "repository_id": "repo_0001",
        "story_spec_ids": ["story_spec_0001"],
        "design_spec_ids": ["design_spec_0001"],
        "title": "Implement backend API",
        "plan_status": "confirmed",
        "execution_status": "pending",
        "worktree_path": null,
        "created_at": "2026-06-16T00:00:00Z",
        "updated_at": "2026-06-16T00:00:00Z"
    });

    let record: LifecycleWorkItemRecord =
        serde_json::from_value(json).expect("legacy lifecycle work item should deserialize");

    assert_eq!(record.kind, WorkItemKind::Other);
    assert_eq!(record.work_item_set_id, None);
    assert_eq!(record.sequence_hint, None);
    assert!(record.depends_on.is_empty());
    assert!(record.exclusive_write_scopes.is_empty());
    assert!(record.forbidden_write_scopes.is_empty());
    assert_eq!(record.context_budget, WorkItemContextBudget::default());
    assert!(record.required_handoff_from.is_empty());
    assert_eq!(record.verification_plan_ref, None);
    assert!(!record.require_execution_plan_confirm);
    assert_eq!(
        record.execution_plan_status,
        WorkItemExecutionPlanStatus::NotStarted
    );
    assert_eq!(record.handoff_summary_ref, None);
    assert_eq!(record.completion_commit, None);
    assert_eq!(record.completion_diff_summary_ref, None);
}

#[test]
fn work_item_context_budget_defaults_to_single_session_budget_proxy() {
    let budget = WorkItemContextBudget::default();

    assert_eq!(budget.target_context_k, "30-50");
    assert_eq!(budget.max_summary_chars, 20_000);
    assert_eq!(budget.max_handoff_chars, 12_000);
    assert_eq!(budget.max_code_context_chars, 30_000);
    assert_eq!(budget.max_context_file_refs, 80);
    assert_eq!(budget.max_traceability_refs, 40);
    assert_eq!(budget.max_dependency_handoffs, 3);
}

#[test]
fn lifecycle_work_item_serializes_new_split_fields_as_snake_case() {
    let record = LifecycleWorkItemRecord {
        id: "work_item_0002".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        repository_id: "repo_0001".to_string(),
        story_spec_ids: vec!["story_spec_0001".to_string()],
        design_spec_ids: vec!["design_spec_0001".to_string()],
        title: "Backend API".to_string(),
        plan_status: WorkItemPlanStatus::Confirmed,
        execution_status: WorkItemStatus::Pending,
        worktree_path: None,
        work_item_set_id: Some("work_item_set_0001".to_string()),
        kind: WorkItemKind::Backend,
        sequence_hint: Some(10),
        depends_on: vec!["work_item_0001".to_string()],
        exclusive_write_scopes: vec!["src/product/**".to_string()],
        forbidden_write_scopes: vec!["web/**".to_string()],
        context_budget: WorkItemContextBudget::default(),
        required_handoff_from: vec!["work_item_0001".to_string()],
        verification_plan_ref: Some("verification_plan_work_item_0002".to_string()),
        require_execution_plan_confirm: true,
        execution_plan_status: WorkItemExecutionPlanStatus::Draft,
        handoff_summary_ref: Some("handoffs/work_item_0001.json".to_string()),
        completion_commit: Some("abc123".to_string()),
        completion_diff_summary_ref: Some("diffs/work_item_0002.json".to_string()),
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    };

    let value = serde_json::to_value(record).expect("serialize lifecycle work item");

    assert_eq!(value["kind"], "backend");
    assert_eq!(value["execution_plan_status"], "draft");
    assert_eq!(value["verification_plan_ref"], "verification_plan_work_item_0002");
    assert_eq!(value["work_item_set_id"], "work_item_set_0001");
    assert_eq!(value["depends_on"], serde_json::json!(["work_item_0001"]));
    assert_eq!(
        value["exclusive_write_scopes"],
        serde_json::json!(["src/product/**"])
    );
    assert_eq!(value["forbidden_write_scopes"], serde_json::json!(["web/**"]));
    assert_eq!(value["require_execution_plan_confirm"], true);
}
```

In `tests/it_product.rs`, add:

```rust
#[path = "it_product/product_work_item_models.rs"]
mod product_work_item_models;
```

- [ ] **Step 2: Run tests and confirm failure**

Run:

```bash
cargo test --locked --test it_product lifecycle_work_item_deserializes_legacy_json_with_split_defaults
cargo test --locked --test it_product work_item_context_budget_defaults_to_single_session_budget_proxy
cargo test --locked --test it_product lifecycle_work_item_serializes_new_split_fields_as_snake_case
```

Expected:

- First command fails because `LifecycleWorkItemRecord` does not have the new fields.
- Compiler reports unresolved `WorkItemContextBudget`, `WorkItemExecutionPlanStatus`, and `WorkItemKind`.

- [ ] **Step 3: Add new model types**

In `src/product/models.rs`, add these types near `WorkItemPlanStatus`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemKind {
    Backend,
    Frontend,
    Integration,
    E2e,
    Docs,
    Infra,
    Other,
}

impl Default for WorkItemKind {
    fn default() -> Self {
        Self::Other
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemExecutionPlanStatus {
    NotStarted,
    Draft,
    Confirmed,
    ChangeRequested,
}

impl Default for WorkItemExecutionPlanStatus {
    fn default() -> Self {
        Self::NotStarted
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemContextBudget {
    pub target_context_k: String,
    pub max_summary_chars: usize,
    pub max_handoff_chars: usize,
    pub max_code_context_chars: usize,
    pub max_context_file_refs: usize,
    pub max_traceability_refs: usize,
    pub max_dependency_handoffs: usize,
}

impl Default for WorkItemContextBudget {
    fn default() -> Self {
        Self {
            target_context_k: "30-50".to_string(),
            max_summary_chars: 20_000,
            max_handoff_chars: 12_000,
            max_code_context_chars: 30_000,
            max_context_file_refs: 80,
            max_traceability_refs: 40,
            max_dependency_handoffs: 3,
        }
    }
}
```

- [ ] **Step 4: Extend LifecycleWorkItemRecord**

In `src/product/models.rs`, extend `LifecycleWorkItemRecord` after `worktree_path`:

```rust
    #[serde(default)]
    pub work_item_set_id: Option<String>,
    #[serde(default)]
    pub kind: WorkItemKind,
    #[serde(default)]
    pub sequence_hint: Option<u32>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub exclusive_write_scopes: Vec<String>,
    #[serde(default)]
    pub forbidden_write_scopes: Vec<String>,
    #[serde(default)]
    pub context_budget: WorkItemContextBudget,
    #[serde(default)]
    pub required_handoff_from: Vec<String>,
    #[serde(default)]
    pub verification_plan_ref: Option<String>,
    #[serde(default)]
    pub require_execution_plan_confirm: bool,
    #[serde(default)]
    pub execution_plan_status: WorkItemExecutionPlanStatus,
    #[serde(default)]
    pub handoff_summary_ref: Option<String>,
    #[serde(default)]
    pub completion_commit: Option<String>,
    #[serde(default)]
    pub completion_diff_summary_ref: Option<String>,
```

- [ ] **Step 5: Update lifecycle store creation defaults**

In `src/product/lifecycle_store.rs`, update the `LifecycleWorkItemRecord` literal inside `create_work_item` to set the new fields explicitly:

```rust
            worktree_path: None,
            work_item_set_id: None,
            kind: WorkItemKind::Other,
            sequence_hint: None,
            depends_on: Vec::new(),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: WorkItemContextBudget::default(),
            required_handoff_from: Vec::new(),
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            handoff_summary_ref: None,
            completion_commit: None,
            completion_diff_summary_ref: None,
```

Also update the imports at the top of `src/product/lifecycle_store.rs` to include:

```rust
WorkItemContextBudget, WorkItemExecutionPlanStatus, WorkItemKind,
```

- [ ] **Step 6: Run model tests and confirm pass**

Run:

```bash
cargo test --locked --test it_product lifecycle_work_item_deserializes_legacy_json_with_split_defaults
cargo test --locked --test it_product work_item_context_budget_defaults_to_single_session_budget_proxy
cargo test --locked --test it_product lifecycle_work_item_serializes_new_split_fields_as_snake_case
```

Expected: all three tests pass.

## Task 2: Migrate Worktree Scheduler To LifecycleWorkItemRecord

**Files:**

- Modify: `src/product/worktree_scheduler.rs`
- Modify: `tests/it_core/work_item_scheduler.rs`

- [ ] **Step 1: Rewrite scheduler tests against active model**

Replace `tests/it_core/work_item_scheduler.rs` with:

```rust
use cadence_aria::product::models::{
    LifecycleWorkItemRecord, WorkItemContextBudget, WorkItemExecutionPlanStatus, WorkItemKind,
    WorkItemPlanStatus, WorkItemStatus,
};
use cadence_aria::product::worktree_scheduler::{ReadyDecision, ready_work_items};

fn work_item(id: &str, depends_on: Vec<&str>, scope: Vec<&str>) -> LifecycleWorkItemRecord {
    LifecycleWorkItemRecord {
        id: id.to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        repository_id: "repo_0001".to_string(),
        story_spec_ids: vec!["story_spec_0001".to_string()],
        design_spec_ids: vec!["design_spec_0001".to_string()],
        title: id.to_string(),
        plan_status: WorkItemPlanStatus::Confirmed,
        execution_status: WorkItemStatus::Pending,
        worktree_path: None,
        work_item_set_id: Some("work_item_set_0001".to_string()),
        kind: WorkItemKind::Backend,
        sequence_hint: None,
        depends_on: depends_on.into_iter().map(str::to_string).collect(),
        exclusive_write_scopes: scope.into_iter().map(str::to_string).collect(),
        forbidden_write_scopes: Vec::new(),
        context_budget: WorkItemContextBudget::default(),
        required_handoff_from: Vec::new(),
        verification_plan_ref: None,
        require_execution_plan_confirm: false,
        execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
        handoff_summary_ref: None,
        completion_commit: None,
        completion_diff_summary_ref: None,
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    }
}

#[test]
fn blocks_items_with_unfinished_dependencies_and_overlapping_scope() {
    let items = vec![
        work_item("wi_001", vec![], vec!["src/auth/**"]),
        work_item("wi_002", vec!["wi_001"], vec!["src/api/**"]),
        work_item("wi_003", vec![], vec!["src/auth/login.rs"]),
    ];
    let decisions = ready_work_items(&items, &[], &["src/auth/**".to_string()]);

    assert_eq!(
        decisions.get("wi_001"),
        Some(&ReadyDecision::WaitingForScope)
    );
    assert_eq!(
        decisions.get("wi_002"),
        Some(&ReadyDecision::WaitingForDependency)
    );
    assert_eq!(
        decisions.get("wi_003"),
        Some(&ReadyDecision::WaitingForScope)
    );
}

#[test]
fn marks_pending_items_ready_when_dependencies_complete_and_scope_free() {
    let items = vec![
        work_item("wi_001", vec![], vec!["src/product/models.rs"]),
        work_item("wi_002", vec!["wi_001"], vec!["src/product/worktree_scheduler.rs"]),
    ];
    let decisions = ready_work_items(&items, &["wi_001".to_string()], &[]);

    assert_eq!(decisions.get("wi_001"), Some(&ReadyDecision::Ready));
    assert_eq!(decisions.get("wi_002"), Some(&ReadyDecision::Ready));
}

#[test]
fn non_pending_lifecycle_items_are_not_ready() {
    let mut item = work_item("wi_001", vec![], vec!["src/product/**"]);
    item.execution_status = WorkItemStatus::Coding;

    let decisions = ready_work_items(&[item], &[], &[]);

    assert_eq!(decisions.get("wi_001"), Some(&ReadyDecision::NotPending));
}
```

- [ ] **Step 2: Run scheduler tests and confirm failure**

Run:

```bash
cargo test --locked --test it_core work_item_scheduler
```

Expected:

- Compile fails because `ready_work_items()` still expects `WorkItemRecord`.
- After Task 1, `LifecycleWorkItemRecord` compiles but scheduler type does not match.

- [ ] **Step 3: Migrate scheduler implementation**

Replace `src/product/worktree_scheduler.rs` with:

```rust
use std::collections::{HashMap, HashSet};

use crate::product::models::{LifecycleWorkItemRecord, WorkItemStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyDecision {
    Ready,
    WaitingForDependency,
    WaitingForScope,
    NotAgentExecutable,
    NotPending,
}

pub fn ready_work_items(
    items: &[LifecycleWorkItemRecord],
    completed: &[String],
    active_scopes: &[String],
) -> HashMap<String, ReadyDecision> {
    let completed = completed.iter().cloned().collect::<HashSet<_>>();
    items
        .iter()
        .map(|item| {
            let decision = if item.execution_status != WorkItemStatus::Pending {
                ReadyDecision::NotPending
            } else if item.depends_on.iter().any(|dep| !completed.contains(dep)) {
                ReadyDecision::WaitingForDependency
            } else if item.exclusive_write_scopes.iter().any(|scope| {
                active_scopes
                    .iter()
                    .any(|active| scopes_may_overlap(scope, active))
            }) {
                ReadyDecision::WaitingForScope
            } else {
                ReadyDecision::Ready
            };
            (item.id.clone(), decision)
        })
        .collect()
}

fn scopes_may_overlap(left: &str, right: &str) -> bool {
    let left_scope = vec![left.to_string()];
    let right_scope = vec![right.to_string()];
    crate::cross_cutting::worktree::scopes_may_overlap(&left_scope, &right_scope, true)
}
```

Keep `ReadyDecision::NotAgentExecutable` for API compatibility with the previous scheduler. This P1 no longer has an `ExecutionMode` field on `LifecycleWorkItemRecord`, so no branch currently emits it.

- [ ] **Step 4: Run scheduler tests and confirm pass**

Run:

```bash
cargo test --locked --test it_core work_item_scheduler
```

Expected: scheduler tests pass.

## Task 3: Remove Dead WorkItemRecord And WorkItemStore

**Files:**

- Modify: `src/product/models.rs`
- Delete: `src/product/work_item_store.rs`
- Modify: `src/product/mod.rs`

- [ ] **Step 1: Remove old WorkItemRecord**

In `src/product/models.rs`, delete the entire old struct:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemRecord {
    pub id: String,
    pub issue_id: String,
    pub repo_id: String,
    pub title: String,
    pub allowed_write_scope: Vec<String>,
    pub depends_on: Vec<String>,
    pub execution_mode: ExecutionMode,
    pub status: WorkItemStatus,
    pub worktree_path: Option<PathBuf>,
    pub worktree_branch: Option<String>,
}
```

Do not delete `ExecutionMode` in this plan. It may still be part of older product/protocol concepts and should be evaluated separately.

- [ ] **Step 2: Delete empty store module**

Delete `src/product/work_item_store.rs`.

In `src/product/mod.rs`, remove:

```rust
pub mod work_item_store;
```

- [ ] **Step 3: Verify no references remain**

Run:

```bash
rg -n "WorkItemRecord|WorkItemStore|work_item_store" src tests
```

Expected: no output.

## Task 4: Format, Check, And Commit

**Files:**

- All files touched by Tasks 1-3.

- [ ] **Step 1: Run formatting check**

Run:

```bash
cargo fmt --check
```

Expected: pass. If it fails, run `cargo fmt`, then rerun `cargo fmt --check`.

- [ ] **Step 2: Run targeted tests**

Run:

```bash
cargo test --locked --test it_core work_item_scheduler
cargo test --locked --test it_product lifecycle_work_item_deserializes_legacy_json_with_split_defaults
cargo test --locked --test it_product work_item_context_budget_defaults_to_single_session_budget_proxy
cargo test --locked --test it_product lifecycle_work_item_serializes_new_split_fields_as_snake_case
```

Expected: all pass.

- [ ] **Step 3: Run compile check**

Run:

```bash
cargo check --locked
```

Expected: pass.

- [ ] **Step 4: Run clippy with warnings denied**

项目强制规则要求验证链包含 clippy（`cadence/project-rules/build-test-commands.md`）。删除死代码与迁移调度器后，必须确认未引入新告警。

Run:

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: pass，无任何 warning。若出现 `unused import`（如 `ExecutionMode` 在迁移后不再被 `worktree_scheduler.rs` 使用）或可简化构造的告警，必须在本计划内修复后重跑，不得遗留到后续计划。

- [ ] **Step 5: Inspect git diff**

Run:

```bash
git diff -- src/product/models.rs src/product/lifecycle_store.rs src/product/worktree_scheduler.rs src/product/mod.rs tests/it_core/work_item_scheduler.rs tests/it_product.rs tests/it_product/product_work_item_models.rs
git status --short
```

Expected:

- Diff only contains model defaults, scheduler migration, dead store removal, and tests.
- No `src/web/**`, `src/product/coding_workspace_engine.rs`, or `web/**` changes.

- [ ] **Step 6: Commit**

Run:

```bash
git add src/product/models.rs src/product/lifecycle_store.rs src/product/worktree_scheduler.rs src/product/mod.rs src/product/work_item_store.rs tests/it_core/work_item_scheduler.rs tests/it_product.rs tests/it_product/product_work_item_models.rs
git commit -m "feat: migrate work item scheduler model"
```

Expected: commit succeeds.

## Self-Review Checklist

- [ ] 新字段只挂在 `LifecycleWorkItemRecord` 上。
- [ ] 新字段都有 serde default，旧 Work Item JSON 可读。
- [ ] `ready_work_items()` 不再依赖旧 `WorkItemRecord`。
- [ ] `WorkItemRecord` 与 `WorkItemStore` 已删除，且 `rg` 无引用。
- [ ] 本计划未修改生成流、Coding Workspace、共享 worktree 或前端。
- [ ] 定向测试、`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings` 和 `cargo check --locked` 均已通过。
- [ ] 删除 `WorkItemRecord` 后，确认 `src/product/models.rs:71` 的 `product::models::ExecutionMode` 是否仍有使用者；若已无生产代码引用，在本计划记录该孤儿现状或在后续计划评估清理（注意它与 `protocol::projections::ExecutionMode` 是不同类型）。
