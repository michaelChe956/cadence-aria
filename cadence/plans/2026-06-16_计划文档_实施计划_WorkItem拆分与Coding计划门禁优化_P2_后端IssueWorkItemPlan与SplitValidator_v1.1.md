# WorkItem 拆分 P2 后端 IssueWorkItemPlan 与 SplitValidator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增 Issue 级 Work Item 拆分计划模型与纯函数 SplitValidator，让后续生成流在创建可执行 Work Item 前能校验 DAG、写入范围、跨端拆分、测试选项、上下文预算与 traceability。

**Architecture:** 本计划只做后端模型与纯函数校验，不调用 provider、不创建真实 Work Item、不修改前端。`IssueWorkItemPlan` 是 Aria 内部数据模型；`WorkItemSplitValidator` 消费 P1 已扩展的 `LifecycleWorkItemRecord` 字段并返回结构化 findings，便于 P3 接入 `generate_work_items`。

**Tech Stack:** Rust 1.95.0、Serde JSON、Cargo integration tests、TDD、OpenSpec、Superpowers。

**版本：** v1.1

> **v1.1 修订摘要：** 1) 在「前置交付摘要」补回 P1 实际新增并测试的 `sequence_hint` 字段；2) 新增「任务 0：测试脚手架」，给出全文 Task2/3/4 都依赖却从未定义的 `work_item(...)` 与 `split_plan(...)` 两个 test helper 的可编译骨架；3) Task 4 为 `write_scope_required` 规则补一个失败测试，并说明 `forbidden_write_scopes` 暂不纳入本计划校验范围；4) 在 validator 实现描述中明确依赖来源以 `work_item.depends_on` 为准、`plan.dependency_graph` 仅做一致性校验；5) Task 4 新增 `integration_or_e2e_skipped_risk` Warning finding，用于 P9 验收跳过 Integration/E2E 时记录风险。

---

## 前置交付摘要

执行本计划前，先阅读 P1 的提交摘要并确认以下事实已经成立：

- `LifecycleWorkItemRecord` 已包含 `work_item_set_id`、`kind`、`sequence_hint`、`depends_on`、`exclusive_write_scopes`、`forbidden_write_scopes`、`context_budget`、`required_handoff_from`、`require_execution_plan_confirm`、`execution_plan_status`、`handoff_summary_ref`、`completion_commit`、`completion_diff_summary_ref`。
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

## 任务 0：测试脚手架（test helpers）

**文件：**

- Modify: `tests/it_product/product_work_item_split_validator.rs`

> **说明：** 任务 2/3/4 的全部测试都依赖 `work_item(...)` 与 `split_plan(...)` 两个 helper，但它们在测试文件中没有内建定义。必须在编写任务 2 的失败测试之前，把这两个 helper 的完整定义加入测试文件，否则任务 2 起的测试无法编译。本任务不新增产品代码，仅补测试脚手架，因此无独立的「失败 → 通过」循环。

- [ ] **步骤 1：在测试文件中加入两个 test helper**

在 `tests/it_product/product_work_item_split_validator.rs`（任务 1 步骤 1 创建）中，把任务 1 步骤 1 顶部的 `use cadence_aria::product::models::{...}` import 块**合并扩展**为下面这份完整 import（新增 `LifecycleWorkItemRecord`、`WorkItemContextBudget`、`WorkItemExecutionPlanStatus`、`WorkItemKind`、`WorkItemPlanStatus`、`WorkItemStatus`），再在其后加入两个 helper。`work_item(...)` 返回的 `LifecycleWorkItemRecord` 必须给 P1 新增的全部字段填默认值，并默认填充 `story_spec_ids` / `design_spec_ids`（任务 4 的 traceability 失败测试通过显式 `clear()` 来构造空值）：

```rust
use cadence_aria::product::models::{
    IssueWorkItemDependencyEdge, IssueWorkItemPlan, IssueWorkItemPlanOptions,
    IssueWorkItemPlanStatus, LifecycleWorkItemRecord, WorkItemContextBudget,
    WorkItemExecutionPlanStatus, WorkItemKind, WorkItemPlanStatus, WorkItemStatus,
};

fn work_item(
    id: &str,
    kind: WorkItemKind,
    depends_on: Vec<&str>,
    scope: Vec<&str>,
) -> LifecycleWorkItemRecord {
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
        kind,
        sequence_hint: None,
        depends_on: depends_on.into_iter().map(str::to_string).collect(),
        exclusive_write_scopes: scope.into_iter().map(str::to_string).collect(),
        forbidden_write_scopes: Vec::new(),
        context_budget: WorkItemContextBudget::default(),
        required_handoff_from: Vec::new(),
        require_execution_plan_confirm: false,
        execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
        handoff_summary_ref: None,
        completion_commit: None,
        completion_diff_summary_ref: None,
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    }
}

fn split_plan(ids: Vec<&str>, edges: Vec<(&str, &str)>) -> IssueWorkItemPlan {
    IssueWorkItemPlan {
        id: "work_item_set_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        source_story_spec_ids: vec!["story_spec_0001".to_string()],
        source_design_spec_ids: vec!["design_spec_0001".to_string()],
        options: IssueWorkItemPlanOptions {
            include_integration_tests: false,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: false,
        },
        status: IssueWorkItemPlanStatus::Draft,
        work_item_ids: ids.into_iter().map(str::to_string).collect(),
        dependency_graph: edges
            .into_iter()
            .map(|(from, to)| IssueWorkItemDependencyEdge {
                from_work_item_id: from.to_string(),
                to_work_item_id: to.to_string(),
            })
            .collect(),
        created_from_provider_run: None,
        validator_findings: Vec::new(),
        review_summary: None,
        created_at: "2026-06-16T00:00:00Z".to_string(),
        updated_at: "2026-06-16T00:00:00Z".to_string(),
    }
}
```

> **注意：** 任务 2 步骤 1 原本重复列出的 `use cadence_aria::product::models::{...}` 与 `use ...work_item_split_validator::WorkItemSplitValidator;` 中，凡已被本任务 import 覆盖的条目无需重复声明，只补 `WorkItemSplitValidator` 等尚未引入的项即可，避免 `duplicate import` 告警。

- [ ] **步骤 2：确认脚手架编译**

helper 引用的模型类型在任务 1（plan 模型）完成后才齐备。建议本任务的 helper 与任务 1 的 model 测试一起编译验证：

```bash
cargo test --locked --test it_product issue_work_item_plan_serializes_options_and_dependency_graph_as_snake_case
```

预期：测试文件可编译（helper 暂未被任何测试调用时允许出现 `dead_code`，将在任务 2 起被消费）。

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

Append these tests（任务 0 已 import 所需模型类型，此处只补 validator 入口；若你的实现把 helper 与测试分文件组织，请按需补回模型 import）：

```rust
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

Implement membership and cycle detection with `HashSet`/DFS.

**依赖来源约定（唯一事实源）：** 以每个 `LifecycleWorkItemRecord::depends_on` 作为构建依赖图、检测环、判定 scope 排序的**唯一依赖来源**。`plan.dependency_graph` 不作为依赖判定输入，仅用于**一致性校验**：

- 节点集合：以 `plan.work_item_ids` 为允许的节点全集。校验每个 `depends_on` 端点都属于该集合，否则报 `dependency_not_in_plan`。
- `plan.dependency_graph` 的每条边 `(from, to)` 的两个端点也必须属于 `plan.work_item_ids`，否则同样报 `dependency_not_in_plan`。
- 一致性：`depends_on` 推导出的边集（对每个 item 的每个 `dep`，记一条 `dep -> item` 的边）必须与 `plan.dependency_graph` 声明的边集相等。不一致时报 `dependency_graph_mismatch`（severity `error`）。这样可避免「图里画了顺序但 `depends_on` 没写」或反之导致的隐性不一致。
- 环检测：在 `depends_on` 推导出的有向图上做 DFS，发现回边即报 `dependency_cycle`。

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

#[test]
fn validator_requires_non_empty_exclusive_write_scopes() {
    let plan = split_plan(vec!["work_item_0001"], vec![]);
    let item = work_item("work_item_0001", WorkItemKind::Backend, vec![], vec![]);

    let report = WorkItemSplitValidator::validate(&plan, &[item]);

    assert!(report.has_errors());
    assert!(report.findings.iter().any(|finding| finding.code == "write_scope_required"));
}

#[test]
fn validator_records_risk_when_integration_or_e2e_skipped() {
    let mut plan = split_plan(vec!["work_item_0001", "work_item_0002"], vec![]);
    plan.options.include_integration_tests = false;
    plan.options.include_e2e_tests = false;
    let items = vec![
        work_item("work_item_0001", WorkItemKind::Backend, vec![], vec!["src/**"]),
        work_item("work_item_0002", WorkItemKind::Frontend, vec!["work_item_0001"], vec!["web/src/**"]),
    ];

    let report = WorkItemSplitValidator::validate(&plan, &items);

    assert!(!report.has_errors());
    assert!(report.findings.iter().any(|finding| {
        finding.code == "integration_or_e2e_skipped_risk"
            && finding.severity == WorkItemSplitFindingSeverity::Warning
    }));
}
```

- [ ] **步骤 2：运行 semantic tests 并确认失败**

运行:

```bash
cargo test --locked --test it_product validator_requires_backend_and_frontend_when_force_split_is_enabled
cargo test --locked --test it_product validator_requires_integration_item_when_option_enabled
cargo test --locked --test it_product validator_requires_traceability_refs_on_every_work_item
cargo test --locked --test it_product validator_requires_non_empty_exclusive_write_scopes
cargo test --locked --test it_product validator_records_risk_when_integration_or_e2e_skipped
```

预期: tests fail because semantic checks are not implemented.

- [ ] **步骤 3：实现 semantic checks**

Rules:

- `force_frontend_backend_split=true` requires at least one `WorkItemKind::Backend` and at least one `WorkItemKind::Frontend`.
- `include_integration_tests=true` requires at least one `WorkItemKind::Integration`.
- `include_e2e_tests=true` requires at least one `WorkItemKind::E2e`.
- Every Work Item requires at least one `story_spec_id` and at least one `design_spec_id`.
- Empty `exclusive_write_scopes` is an error with code `write_scope_required`（对应步骤 1 的 `validator_requires_non_empty_exclusive_write_scopes` 失败测试）。
- **`forbidden_write_scopes` 不纳入本计划校验范围**：本计划只做「写入范围必须非空」与「并行写入范围不得重叠」两类校验，`forbidden_write_scopes`（禁写范围）与 `exclusive_write_scopes` 的交叉一致性校验留待 P3 接入生成流时设计，本版 validator 不读取该字段。
- 当 `include_integration_tests=false` 或 `include_e2e_tests=false` 时，分别添加 `Warning` 级别 finding，code 为 `integration_or_e2e_skipped_risk`，message 说明跳过的测试类型及建议后续手工验证。该 finding 不阻塞计划确认（`has_errors()` 仍为 false），但会被 P9 验收用例断言。

- [ ] **步骤 4：运行 semantic tests 并确认通过**

Run the five commands from Step 2 again.

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
