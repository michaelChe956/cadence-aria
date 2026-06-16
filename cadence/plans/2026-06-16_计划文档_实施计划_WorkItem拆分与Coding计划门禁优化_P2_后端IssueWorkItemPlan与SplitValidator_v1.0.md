# WorkItem 拆分 P2 后端 IssueWorkItemPlan 与 SplitValidator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增 Issue 级 Work Item 拆分计划模型与纯函数 SplitValidator，让后续生成流在创建可执行 Work Item 前能校验 DAG、写入范围、跨端拆分、测试选项、上下文预算与 traceability。

**Architecture:** 本计划只做后端模型与纯函数校验，不调用 provider、不创建真实 Work Item、不修改前端。`IssueWorkItemPlan` 是 Aria 内部数据模型；`WorkItemSplitValidator` 消费 P1 已扩展的 `LifecycleWorkItemRecord` 字段并返回结构化 findings，便于 P3 接入 `generate_work_items`。

**Tech Stack:** Rust 1.95.0、Serde JSON、Cargo integration tests、TDD、OpenSpec、Superpowers。

---

## 前置交付摘要

执行本计划前，先阅读 P1 的提交摘要并确认以下事实已经成立：

- `LifecycleWorkItemRecord` 已包含 `work_item_set_id`、`kind`、`depends_on`、`exclusive_write_scopes`、`forbidden_write_scopes`、`context_budget`、`required_handoff_from`、`require_execution_plan_confirm`、`execution_plan_status`、`handoff_summary_ref`、`completion_commit`、`completion_diff_summary_ref`。
- 旧 `WorkItemRecord` 和 `WorkItemStore` 已删除，`worktree_scheduler::ready_work_items()` 已迁移到 `LifecycleWorkItemRecord`。
- `src/product/mod.rs` 已导出仍在使用的 product 模块。

## 计划大小边界

本计划必须保持为纯后端模型与 validator：

- 不修改 `src/web/handlers.rs` 的 `generate_work_items` 行为。
- 不新增 workspace session 或 artifact version 创建逻辑。
- 不修改 Coding Workspace 启动门禁。
- 不修改 Issue 共享 worktree。
- 不修改前端。

如果实现时发现必须接入 HTTP handler 或 provider run，停止并把接入工作留给 P3。

## 文件结构

- Modify: `src/product/models.rs`
  - 新增 `IssueWorkItemPlan`、`IssueWorkItemPlanStatus`、`IssueWorkItemPlanOptions`、`IssueWorkItemDependencyEdge`、`WorkItemSplitFinding`、`WorkItemSplitFindingSeverity`。
- Create: `src/product/work_item_split_validator.rs`
  - 提供 `WorkItemSplitValidator::validate(&IssueWorkItemPlan, &[LifecycleWorkItemRecord]) -> WorkItemSplitValidationReport`。
  - 校验 DAG、scope overlap、跨端拆分、Integration/E2E 选项、上下文预算和 traceability。
- Modify: `src/product/mod.rs`
  - 导出 `work_item_split_validator`。
- Modify: `tests/it_product.rs`
  - 引入 `product_work_item_split_validator`。
- Create: `tests/it_product/product_work_item_split_validator.rs`
  - 覆盖所有生成期结构校验。

## 任务 1：Add IssueWorkItemPlan Model

**文件：**

- Modify: `src/product/models.rs`
- Create: `tests/it_product/product_work_item_split_validator.rs`
- Modify: `tests/it_product.rs`

- [ ] **步骤 1：编写失败态 serde model tests**

创建 `tests/it_product/product_work_item_split_validator.rs` with these initial tests:

```rust
use cadence_aria::product::models::{
    IssueWorkItemDependencyEdge, IssueWorkItemPlan, IssueWorkItemPlanOptions,
    IssueWorkItemPlanStatus, WorkItemSplitFindingSeverity,
};

#[test]
fn issue_work_item_plan_serializes_options_and_dependency_graph_as_snake_case() {
    let plan = IssueWorkItemPlan {
        id: "work_item_set_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        source_story_spec_ids: vec!["story_spec_0001".to_string()],
        source_design_spec_ids: vec!["design_spec_0001".to_string()],
        options: IssueWorkItemPlanOptions {
            include_integration_tests: true,
            include_e2e_tests: false,
            force_frontend_backend_split: true,
            require_execution_plan_confirm: false,
        },
        status: IssueWorkItemPlanStatus::Draft,
        work_item_ids: vec![
            "work_item_0001".to_string(),
            "work_item_0002".to_string(),
        ],
        dependency_graph: vec![IssueWorkItemDependencyEdge {
            from_work_item_id: "work_item_0001".to_string(),
            to_work_item_id: "work_item_0002".to_string(),
        }],
        created_from_provider_run: Some("provider_run_0001".to_string()),
        validator_findings: Vec::new(),
        review_summary: Some("backend first, frontend second".to_string()),
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    };

    let value = serde_json::to_value(plan).expect("serialize plan");

    assert_eq!(value["status"], "draft");
    assert_eq!(value["options"]["include_integration_tests"], true);
    assert_eq!(value["options"]["include_e2e_tests"], false);
    assert_eq!(value["dependency_graph"][0]["from_work_item_id"], "work_item_0001");
    assert_eq!(value["dependency_graph"][0]["to_work_item_id"], "work_item_0002");
}

#[test]
fn split_finding_severity_serializes_as_snake_case() {
    assert_eq!(
        serde_json::to_value(WorkItemSplitFindingSeverity::Error).unwrap(),
        serde_json::json!("error")
    );
    assert_eq!(
        serde_json::to_value(WorkItemSplitFindingSeverity::Warning).unwrap(),
        serde_json::json!("warning")
    );
}
```

在 `tests/it_product.rs`, add:

```rust
#[path = "it_product/product_work_item_split_validator.rs"]
mod product_work_item_split_validator;
```

- [ ] **步骤 2：运行 model tests 并确认失败**

运行:

```bash
cargo test --locked --test it_product issue_work_item_plan_serializes_options_and_dependency_graph_as_snake_case
cargo test --locked --test it_product split_finding_severity_serializes_as_snake_case
```

预期：编译失败，因为 the new model types do not exist.

- [ ] **步骤 3：添加 model types**

在 `src/product/models.rs`, add the model types near the P1 Work Item split types:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueWorkItemPlanStatus {
    Draft,
    Confirmed,
    ChangeRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemPlanOptions {
    pub include_integration_tests: bool,
    pub include_e2e_tests: bool,
    pub force_frontend_backend_split: bool,
    pub require_execution_plan_confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemDependencyEdge {
    pub from_work_item_id: String,
    pub to_work_item_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemSplitFindingSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemSplitFinding {
    pub severity: WorkItemSplitFindingSeverity,
    pub code: String,
    pub message: String,
    pub work_item_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueWorkItemPlan {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub source_story_spec_ids: Vec<String>,
    pub source_design_spec_ids: Vec<String>,
    pub options: IssueWorkItemPlanOptions,
    pub status: IssueWorkItemPlanStatus,
    pub work_item_ids: Vec<String>,
    pub dependency_graph: Vec<IssueWorkItemDependencyEdge>,
    pub created_from_provider_run: Option<String>,
    pub validator_findings: Vec<WorkItemSplitFinding>,
    pub review_summary: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
```

- [ ] **步骤 4：运行 model tests 并确认通过**

运行:

```bash
cargo test --locked --test it_product issue_work_item_plan_serializes_options_and_dependency_graph_as_snake_case
cargo test --locked --test it_product split_finding_severity_serializes_as_snake_case
```

预期: both tests pass.

## 任务 2：Implement DAG And Same-Issue Validation

**文件：**

- Create: `src/product/work_item_split_validator.rs`
- Modify: `src/product/mod.rs`
- Modify: `tests/it_product/product_work_item_split_validator.rs`

- [ ] **步骤 1：编写失败态 DAG tests**

Append these tests:

```rust
use cadence_aria::product::models::{
    LifecycleWorkItemRecord, WorkItemContextBudget, WorkItemExecutionPlanStatus, WorkItemKind,
    WorkItemPlanStatus, WorkItemStatus,
};
use cadence_aria::product::work_item_split_validator::WorkItemSplitValidator;

#[test]
fn validator_rejects_dependency_cycles() {
    let plan = split_plan(
        vec!["work_item_0001", "work_item_0002"],
        vec![("work_item_0001", "work_item_0002"), ("work_item_0002", "work_item_0001")],
    );
    let items = vec![
        work_item("work_item_0001", WorkItemKind::Backend, vec!["work_item_0002"], vec!["src/**"]),
        work_item("work_item_0002", WorkItemKind::Frontend, vec!["work_item_0001"], vec!["web/src/**"]),
    ];

    let report = WorkItemSplitValidator::validate(&plan, &items);

    assert!(report.has_errors());
    assert!(report.findings.iter().any(|finding| finding.code == "dependency_cycle"));
}

#[test]
fn validator_rejects_dependency_outside_same_issue() {
    let plan = split_plan(vec!["work_item_0001"], vec![("work_item_0001", "work_item_9999")]);
    let items = vec![work_item(
        "work_item_0001",
        WorkItemKind::Backend,
        vec!["work_item_9999"],
        vec!["src/**"],
    )];

    let report = WorkItemSplitValidator::validate(&plan, &items);

    assert!(report.has_errors());
    assert!(report.findings.iter().any(|finding| finding.code == "dependency_not_in_plan"));
}
```

- [ ] **步骤 2：运行 DAG tests 并确认失败**

运行:

```bash
cargo test --locked --test it_product validator_rejects_dependency_cycles
cargo test --locked --test it_product validator_rejects_dependency_outside_same_issue
```

预期：编译失败，因为 `WorkItemSplitValidator` does not exist.

- [ ] **步骤 3：添加 validator skeleton and DAG checks**

创建 `src/product/work_item_split_validator.rs`:

```rust
use std::collections::{HashMap, HashSet};

use crate::product::models::{
    IssueWorkItemPlan, LifecycleWorkItemRecord, WorkItemSplitFinding,
    WorkItemSplitFindingSeverity,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemSplitValidationReport {
    pub findings: Vec<WorkItemSplitFinding>,
}

impl WorkItemSplitValidationReport {
    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == WorkItemSplitFindingSeverity::Error)
    }
}

pub struct WorkItemSplitValidator;

impl WorkItemSplitValidator {
    pub fn validate(
        plan: &IssueWorkItemPlan,
        work_items: &[LifecycleWorkItemRecord],
    ) -> WorkItemSplitValidationReport {
        let mut findings = Vec::new();
        validate_plan_membership(plan, work_items, &mut findings);
        validate_dependencies(plan, work_items, &mut findings);
        WorkItemSplitValidationReport { findings }
    }
}
```

添加 helper functions in the same file:

```rust
fn error(code: &str, message: impl Into<String>, work_item_ids: Vec<String>) -> WorkItemSplitFinding {
    WorkItemSplitFinding {
        severity: WorkItemSplitFindingSeverity::Error,
        code: code.to_string(),
        message: message.into(),
        work_item_ids,
    }
}
```

Implement membership and cycle detection with `HashSet`/DFS. Use the `plan.work_item_ids` list as the allowed node set and verify every `depends_on` and every `dependency_graph` endpoint belongs to it.

在 `src/product/mod.rs`, add:

```rust
pub mod work_item_split_validator;
```

- [ ] **步骤 4：运行 DAG tests 并确认通过**

运行:

```bash
cargo test --locked --test it_product validator_rejects_dependency_cycles
cargo test --locked --test it_product validator_rejects_dependency_outside_same_issue
```

预期: both tests pass.

## 任务 3：Validate Scope Conflicts And Context Budgets

**文件：**

- Modify: `src/product/work_item_split_validator.rs`
- Modify: `tests/it_product/product_work_item_split_validator.rs`

- [ ] **步骤 1：编写失败态 scope and budget tests**

追加:

```rust
#[test]
fn validator_rejects_parallel_overlapping_write_scopes() {
    let plan = split_plan(vec!["work_item_0001", "work_item_0002"], vec![]);
    let items = vec![
        work_item("work_item_0001", WorkItemKind::Backend, vec![], vec!["src/product/**"]),
        work_item("work_item_0002", WorkItemKind::Backend, vec![], vec!["src/**"]),
    ];

    let report = WorkItemSplitValidator::validate(&plan, &items);

    assert!(report.has_errors());
    assert!(report.findings.iter().any(|finding| finding.code == "parallel_scope_overlap"));
}

#[test]
fn validator_allows_overlapping_write_scopes_when_dependency_orders_items() {
    let plan = split_plan(
        vec!["work_item_0001", "work_item_0002"],
        vec![("work_item_0001", "work_item_0002")],
    );
    let items = vec![
        work_item("work_item_0001", WorkItemKind::Backend, vec![], vec!["src/product/**"]),
        work_item("work_item_0002", WorkItemKind::Backend, vec!["work_item_0001"], vec!["src/**"]),
    ];

    let report = WorkItemSplitValidator::validate(&plan, &items);

    assert!(!report.has_errors());
}

#[test]
fn validator_rejects_context_budget_over_proxy_limits() {
    let plan = split_plan(vec!["work_item_0001"], vec![]);
    let mut item = work_item("work_item_0001", WorkItemKind::Backend, vec![], vec!["src/**"]);
    item.context_budget.max_summary_chars = 100_001;
    item.context_budget.max_context_file_refs = 500;

    let report = WorkItemSplitValidator::validate(&plan, &[item]);

    assert!(report.has_errors());
    assert!(report.findings.iter().any(|finding| finding.code == "context_budget_over_limit"));
}
```

- [ ] **步骤 2：运行 scope and budget tests 并确认失败**

运行:

```bash
cargo test --locked --test it_product validator_rejects_parallel_overlapping_write_scopes
cargo test --locked --test it_product validator_allows_overlapping_write_scopes_when_dependency_orders_items
cargo test --locked --test it_product validator_rejects_context_budget_over_proxy_limits
```

预期: tests fail because scope and budget checks are not implemented.

- [ ] **步骤 3：实现 scope and budget checks**

使用 `crate::cross_cutting::worktree::scopes_may_overlap(&left, &right, true)` for overlap checks. Treat two Work Items as ordered if either item can reach the other through the dependency graph.

Budget limits for first version:

```rust
const MAX_SUMMARY_CHARS: usize = 50_000;
const MAX_HANDOFF_CHARS: usize = 20_000;
const MAX_CODE_CONTEXT_CHARS: usize = 50_000;
const MAX_CONTEXT_FILE_REFS: usize = 120;
const MAX_TRACEABILITY_REFS: usize = 80;
const MAX_DEPENDENCY_HANDOFFS: usize = 5;
```

Return `context_budget_over_limit` if any Work Item exceeds those proxy limits.

- [ ] **步骤 4：运行 scope and budget tests 并确认通过**

Run the three commands from Step 2 again.

预期：全部通过。

## 任务 4：Validate Cross-End Split, Integration/E2E Options, And Traceability

**文件：**

- Modify: `src/product/work_item_split_validator.rs`
- Modify: `tests/it_product/product_work_item_split_validator.rs`

- [ ] **步骤 1：编写失败态 semantic validation tests**

追加:

```rust
#[test]
fn validator_requires_backend_and_frontend_when_force_split_is_enabled() {
    let mut plan = split_plan(vec!["work_item_0001"], vec![]);
    plan.options.force_frontend_backend_split = true;
    let items = vec![work_item("work_item_0001", WorkItemKind::Backend, vec![], vec!["src/**"])];

    let report = WorkItemSplitValidator::validate(&plan, &items);

    assert!(report.has_errors());
    assert!(report.findings.iter().any(|finding| finding.code == "frontend_backend_split_required"));
}

#[test]
fn validator_requires_integration_item_when_option_enabled() {
    let mut plan = split_plan(vec!["work_item_0001", "work_item_0002"], vec![]);
    plan.options.include_integration_tests = true;
    let items = vec![
        work_item("work_item_0001", WorkItemKind::Backend, vec![], vec!["src/**"]),
        work_item("work_item_0002", WorkItemKind::Frontend, vec!["work_item_0001"], vec!["web/src/**"]),
    ];

    let report = WorkItemSplitValidator::validate(&plan, &items);

    assert!(report.has_errors());
    assert!(report.findings.iter().any(|finding| finding.code == "integration_work_item_required"));
}

#[test]
fn validator_requires_traceability_refs_on_every_work_item() {
    let plan = split_plan(vec!["work_item_0001"], vec![]);
    let mut item = work_item("work_item_0001", WorkItemKind::Backend, vec![], vec!["src/**"]);
    item.story_spec_ids.clear();
    item.design_spec_ids.clear();

    let report = WorkItemSplitValidator::validate(&plan, &[item]);

    assert!(report.has_errors());
    assert!(report.findings.iter().any(|finding| finding.code == "traceability_refs_required"));
}
```

- [ ] **步骤 2：运行 semantic tests 并确认失败**

运行:

```bash
cargo test --locked --test it_product validator_requires_backend_and_frontend_when_force_split_is_enabled
cargo test --locked --test it_product validator_requires_integration_item_when_option_enabled
cargo test --locked --test it_product validator_requires_traceability_refs_on_every_work_item
```

预期: tests fail because semantic checks are not implemented.

- [ ] **步骤 3：实现 semantic checks**

Rules:

- `force_frontend_backend_split=true` requires at least one `WorkItemKind::Backend` and at least one `WorkItemKind::Frontend`.
- `include_integration_tests=true` requires at least one `WorkItemKind::Integration`.
- `include_e2e_tests=true` requires at least one `WorkItemKind::E2e`.
- Every Work Item requires at least one `story_spec_id` and at least one `design_spec_id`.
- Empty `exclusive_write_scopes` is an error with code `write_scope_required`.

- [ ] **步骤 4：运行 semantic tests 并确认通过**

Run the three commands from Step 2 again.

预期：全部通过。

## 最终验证

运行:

```bash
cargo test --locked --test it_product work_item_split_validator
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

预期:

- Validator tests pass.
- Formatting passes.
- Clippy passes with warnings denied.
- `cargo check --locked` passes.

## 提交

```bash
git add src/product/models.rs src/product/work_item_split_validator.rs src/product/mod.rs tests/it_product.rs tests/it_product/product_work_item_split_validator.rs
git commit -m "feat: add work item split validator"
```
