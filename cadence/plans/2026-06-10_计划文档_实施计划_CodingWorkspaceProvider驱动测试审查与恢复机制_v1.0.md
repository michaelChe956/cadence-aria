# CodingWorkspace Provider 驱动测试审查与恢复机制实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Coding Workspace 的 Tester、Analyst、Code Reviewer、Internal Reviewer 全部基于 OpenSpec 与 Superpowers 契约工作，并由 Provider 根据 Story Spec、Design Spec、Work Item、diff 和项目规则生成测试或审查结论，Aria 只负责上下文、流程契约、证据、持久化、恢复 gate 与 UI 展示。

**Architecture:** 新增 `EvaluationContextPack` 作为四类评估节点的统一输入；Testing 节点改为 `plan_tests` -> `execute_test_plan` 两段式，并用 TestPlan 约束 required step 完整性。Review/Analyst/Internal Review 保持 Provider 决策，但后端保存 raw output、容错解析、生成持久化 blocked gate，前端通过现有 WebSocket pending gate 链路展示与恢复。

**Tech Stack:** Rust 1.95、serde/serde_json、tokio、Axum WebSocket、React 19、TypeScript、Zustand、Vitest、Cargo。

---

## 关联文档

- 问题记录：`cadence/analysis-docs/2026-06-10_状态记录_CodingWorkspace测试与代码审查节点问题_v1.0.md`
- 技术方案：`cadence/designs/2026-06-10_技术方案_CodingWorkspaceProvider驱动测试审查与恢复机制_v1.0.md`

## 文件结构

### 后端模型与上下文

- Modify: `src/product/mod.rs`
  - 暴露 `coding_evaluation_context` 新模块。
- Create: `src/product/coding_evaluation_context.rs`
  - 构建 `EvaluationContextPack`。
  - 从 `LifecycleStore`、attempt、worktree diff、项目规则中收集 Story/Design/WorkItem/OpenSpec/Superpowers 上下文。
- Modify: `src/product/coding_models.rs`
  - 新增 TestPlan、TestPlanStep、TestingStepResult 与 blocked gate metadata。
  - 扩展 TestingReport、ReviewFinding、CodeReviewReport、InternalPrReview、CodingGateActionType、CodingGateRequired。
- Modify: `src/product/coding_attempt_store.rs`
  - 新增保存和读取 test plan、raw provider output、blocked gate 的方法。

### 后端流程与 Provider 契约

- Modify: `src/product/tester_agent_loop.rs`
  - 新增 plan prompt、plan parser、step-bound tool 调用、plan-based report builder。
  - 保留现有 command executor 作为 provider 不支持 tool calls 时的 blocked/fallback 证据，不再声明 plan-based passed。
- Modify: `src/product/coding_workspace_engine.rs`
  - Testing 改成先生成 TestPlan，再执行计划。
  - Review/Analyst/Internal Review prompt 注入 EvaluationContextPack、OpenSpec contract、Superpowers contract。
  - Review raw output 保存，解析失败创建 blocked gate。
  - failed/request_changes 与 blocked 分流。
- Modify: `src/web/coding_ws_handler.rs`
  - `build_coding_session_state` 合并 stage gate 与 blocked gate。
  - 处理 `GateResponse`。
  - blocked 状态允许 gate response 恢复。

### 前端类型、状态与 UI

- Modify: `web/src/api/types.ts`
  - 同步 TestPlan、TestingReport v2、blocked gate metadata、gate action type。
- Modify: `web/src/state/coding-workspace-store.ts`
  - 支持 plan-based testing report 与 blocked gate 合并状态。
- Modify: `web/src/hooks/useCodingWorkspaceWs.ts`
  - gate response 成功后移除本地 pending gate。
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
  - Testing 面板展示 TestPlan、step 状态、missing required steps、raw output 引用。
  - GatePanel 展示 blocked reason、evidence/raw refs 和恢复动作。

### 测试

- Modify: `src/product/tester_agent_loop.rs` tests
- Modify: `src/product/coding_workspace_engine.rs` tests
- Modify: `src/product/coding_attempt_store.rs` tests
- Modify: `src/web/coding_ws_handler.rs` tests
- Modify: `web/src/api/types.test.ts`
- Modify: `web/src/state/coding-workspace-store.test.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.test.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`

## 验证命令

Rust 标准命令必须使用宿主机 cargo，禁止给 cargo test 增加 `-j 1`：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

前端命令：

```bash
pnpm -C web test
pnpm -C web build
```

---

### Task 1: 扩展后端模型

**Files:**
- Modify: `src/product/coding_models.rs`
- Test: `src/product/coding_models.rs`

- [ ] **Step 1: 写模型序列化测试**

在 `src/product/coding_models.rs` 的 `#[cfg(test)] mod tests` 中新增测试；如果文件还没有测试模块，放到文件末尾。

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_and_testing_report_round_trip_preserve_step_evidence() {
        let plan = TestPlan {
            id: "test_plan_0001".to_string(),
            attempt_id: "coding_attempt_0001".to_string(),
            summary: "验证 API、前端和安全风险".to_string(),
            context_warnings: vec!["missing_design_spec".to_string()],
            assumptions: vec!["使用项目已有验证命令".to_string()],
            steps: vec![TestPlanStep {
                id: "step_api_smoke".to_string(),
                title: "API smoke".to_string(),
                intent: "确认核心 API 返回成功状态".to_string(),
                required: true,
                tool: TestPlanTool::RunCommand,
                risk_level: TestPlanRiskLevel::Low,
                command_or_tool_input: serde_json::json!({"command": ["cargo", "test", "--locked"]}),
                evidence_expectation: "exit_code=0 且 stdout/stderr artifact 可读取".to_string(),
                related_requirements: vec!["story:REQ-1".to_string()],
                related_design_constraints: vec!["design:API contract".to_string()],
                related_work_item_tasks: vec!["work_item:验证命令".to_string()],
            }],
            created_at: "2026-06-10T00:00:00Z".to_string(),
            raw_provider_output_ref: Some("raw/test-plan-0001.txt".to_string()),
        };

        let report = TestingReport {
            id: "testing_report_0001".to_string(),
            attempt_id: "coding_attempt_0001".to_string(),
            plan_id: Some(plan.id.clone()),
            plan_summary: Some(plan.summary.clone()),
            commands: Vec::new(),
            steps: vec![TestingStepResult {
                step_id: "step_api_smoke".to_string(),
                title: "API smoke".to_string(),
                required: true,
                status: TestCommandStatus::Passed,
                evidence_refs: vec!["test-output/step_api_smoke.stdout.log".to_string()],
                provider_analysis: Some("API smoke passed".to_string()),
                started_at: "2026-06-10T00:00:00Z".to_string(),
                completed_at: Some("2026-06-10T00:00:01Z".to_string()),
            }],
            unplanned_commands: Vec::new(),
            missing_required_steps: Vec::new(),
            skipped_required_steps: Vec::new(),
            context_warnings: plan.context_warnings.clone(),
            overall_status: TestingOverallStatus::PassedWithWarnings,
            provider_claim: Some(serde_json::json!({"summary": "passed"})),
            raw_provider_output_ref: Some("raw/testing-0001.txt".to_string()),
            backend_verified: true,
            started_at: "2026-06-10T00:00:00Z".to_string(),
            completed_at: Some("2026-06-10T00:00:01Z".to_string()),
        };

        let value = serde_json::to_value((&plan, &report)).expect("serialize");
        assert_eq!(value[0]["steps"][0]["tool"], "run_command");
        assert_eq!(value[1]["steps"][0]["step_id"], "step_api_smoke");
        assert_eq!(value[1]["overall_status"], "passed_with_warnings");
    }
}
```

- [ ] **Step 2: 运行模型测试并确认失败**

Run:

```bash
cargo test --locked --lib test_plan_and_testing_report_round_trip_preserve_step_evidence
```

Expected: FAIL，错误包含缺少 `TestPlan`、`TestPlanStep`、`TestPlanTool`、`TestingStepResult` 或 `PassedWithWarnings`。

- [ ] **Step 3: 新增模型定义**

在 `src/product/coding_models.rs` 中 `TestCommand` 后、`TestingOverallStatus` 前加入：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestPlanTool {
    RunCommand,
    ReadFile,
    ListFiles,
    SearchCode,
    ProviderManaged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestPlanRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPlanStep {
    pub id: String,
    pub title: String,
    pub intent: String,
    pub required: bool,
    pub tool: TestPlanTool,
    pub risk_level: TestPlanRiskLevel,
    pub command_or_tool_input: serde_json::Value,
    pub evidence_expectation: String,
    pub related_requirements: Vec<String>,
    pub related_design_constraints: Vec<String>,
    pub related_work_item_tasks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPlan {
    pub id: String,
    pub attempt_id: String,
    pub summary: String,
    pub context_warnings: Vec<String>,
    pub assumptions: Vec<String>,
    pub steps: Vec<TestPlanStep>,
    pub created_at: String,
    pub raw_provider_output_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestingStepResult {
    pub step_id: String,
    pub title: String,
    pub required: bool,
    pub status: TestCommandStatus,
    pub evidence_refs: Vec<String>,
    pub provider_analysis: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}
```

将 `TestingOverallStatus` 扩展为：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestingOverallStatus {
    Passed,
    PassedWithWarnings,
    Failed,
    SkippedByUserDecision,
    Blocked,
}
```

将 `TestingReport` 替换为：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestingReport {
    pub id: String,
    pub attempt_id: String,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub plan_summary: Option<String>,
    #[serde(default)]
    pub commands: Vec<TestCommand>,
    #[serde(default)]
    pub steps: Vec<TestingStepResult>,
    #[serde(default)]
    pub unplanned_commands: Vec<TestCommand>,
    #[serde(default)]
    pub missing_required_steps: Vec<String>,
    #[serde(default)]
    pub skipped_required_steps: Vec<String>,
    #[serde(default)]
    pub context_warnings: Vec<String>,
    pub overall_status: TestingOverallStatus,
    pub provider_claim: Option<serde_json::Value>,
    #[serde(default)]
    pub raw_provider_output_ref: Option<String>,
    pub backend_verified: bool,
    pub started_at: String,
    pub completed_at: Option<String>,
}
```

在 `ReviewFinding` 中追加追踪字段：

```rust
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub related_requirements: Vec<String>,
    #[serde(default)]
    pub related_design_constraints: Vec<String>,
    #[serde(default)]
    pub related_work_item_tasks: Vec<String>,
```

在 `CodeReviewReport` 与 `InternalPrReview` 中追加：

```rust
    #[serde(default)]
    pub raw_provider_output_ref: Option<String>,
```

在 `CodingGateActionType` 中追加：

```rust
    RetryTestPlan,
    RerunMissingSteps,
    ProvideContext,
    ManualContinue,
    RetryReview,
    SendRawOutputToAnalyst,
```

在 `CodingGateRequired` 中追加：

```rust
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub raw_provider_output_ref: Option<String>,
```

- [ ] **Step 4: 运行模型测试并确认通过**

Run:

```bash
cargo test --locked --lib test_plan_and_testing_report_round_trip_preserve_step_evidence
```

Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/product/coding_models.rs
git commit -m "feat: extend coding QA models"
```

---

### Task 2: 增加 EvaluationContextPack 构建器

**Files:**
- Modify: `src/product/mod.rs`
- Create: `src/product/coding_evaluation_context.rs`
- Test: `src/product/coding_evaluation_context.rs`

- [ ] **Step 1: 写上下文构建测试**

创建 `src/product/coding_evaluation_context.rs`，先放入测试和最小类型引用。测试要证明 Story、Design、Work Item 都进入上下文，且 OpenSpec/Superpowers 对 Tester、Analyst、Code Reviewer、Internal Reviewer 都启用。

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_models::{CodingExecutionAttempt, CodingExecutionStage, CodingAttemptStatus};
    use crate::product::lifecycle_store::{
        AppendSpecVersionInput, CreateDesignSpecInput, CreateStorySpecInput, CreateWorkItemInput,
        CreateWorkspaceSessionInput, LifecycleStore,
    };
    use crate::product::models::{DesignKind, ProviderName, WorkspaceType};
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;
    use tempfile::tempdir;

    #[test]
    fn evaluation_context_pack_includes_story_design_work_item_and_contracts() {
        let temp = tempdir().expect("tempdir");
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());

        let story = lifecycle.create_story_spec(CreateStorySpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repo_0001".to_string(),
            title: "Story".to_string(),
        }).expect("story");
        lifecycle.append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id.clone(),
            markdown: "# Story\n\n## Acceptance Criteria\n\n- API smoke passes".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: Some("tester".to_string()),
        }).expect("story version");
        lifecycle.create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: story.id.clone(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        }).expect("story session");

        let design = lifecycle.create_design_spec(CreateDesignSpecInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            design_kind: DesignKind::Backend,
            title: "Design".to_string(),
        }).expect("design");
        lifecycle.append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: design.id.clone(),
            markdown: "# Design\n\n## Security\n\n- Preserve provider contract".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: Some("tester".to_string()),
        }).expect("design version");
        lifecycle.create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: design.id.clone(),
            workspace_type: WorkspaceType::Design,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        }).expect("design session");

        let work_item = lifecycle.create_work_item(CreateWorkItemInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repo_0001".to_string(),
            story_spec_ids: vec![story.id.clone()],
            design_spec_ids: vec![design.id.clone()],
            title: "Work Item".to_string(),
        }).expect("work item");
        lifecycle.append_version(AppendSpecVersionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: work_item.id.clone(),
            markdown: "# Work Item\n\n## 验证命令\n\n```bash\ncargo test --locked\n```".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: Some("tester".to_string()),
        }).expect("work item version");
        lifecycle.create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: work_item.id.clone(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        }).expect("session");

        let attempt = CodingExecutionAttempt {
            id: "coding_attempt_0001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item.id.clone(),
            attempt_no: 1,
            status: CodingAttemptStatus::Running,
            stage: CodingExecutionStage::Testing,
            base_branch: "main".to_string(),
            branch_name: "bugfix_test_branch".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            rework_count: 0,
            max_auto_rework: 2,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: "2026-06-10T00:00:00Z".to_string(),
            updated_at: "2026-06-10T00:00:00Z".to_string(),
            completed_at: None,
        };

        let pack = build_evaluation_context_pack(&paths, &attempt, EvaluationContextRole::Tester)
            .expect("pack");

        assert!(pack.story_spec.raw_markdown_or_sections.contains("Acceptance Criteria"));
        assert!(pack.design_spec.raw_markdown_or_sections.contains("Security"));
        assert!(pack.work_item.raw_markdown_or_sections.contains("验证命令"));
        assert!(pack.openspec_context.enabled);
        assert!(pack.superpowers_context.enabled);
        assert!(pack.superpowers_context.required_methods_by_role.contains_key("tester"));
        assert!(pack.superpowers_context.required_methods_by_role.contains_key("analyst"));
        assert!(pack.superpowers_context.required_methods_by_role.contains_key("code_reviewer"));
        assert!(pack.superpowers_context.required_methods_by_role.contains_key("internal_reviewer"));
    }
}
```

- [ ] **Step 2: 运行上下文测试并确认失败**

Run:

```bash
cargo test --locked --lib evaluation_context_pack_includes_story_design_work_item_and_contracts
```

Expected: FAIL，错误包含缺少 `build_evaluation_context_pack` 或相关类型。

- [ ] **Step 3: 实现上下文类型和构建器**

在 `src/product/mod.rs` 加入：

```rust
pub mod coding_evaluation_context;
```

在 `src/product/coding_evaluation_context.rs` 加入：

```rust
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::{CodingExecutionAttempt, CodingProviderRole};
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{LifecycleWorkItemRecord, SpecVersionRecord, WorkspaceType};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationContextRole {
    Tester,
    Analyst,
    CodeReviewer,
    InternalReviewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationContextPack {
    pub issue_id: String,
    pub attempt_id: String,
    pub stage: String,
    pub provider_role: EvaluationContextRole,
    pub story_spec: EvaluationSpecContext,
    pub design_spec: EvaluationSpecContext,
    pub work_item: EvaluationSpecContext,
    pub repo_context: EvaluationRepoContext,
    pub openspec_context: OpenSpecContext,
    pub superpowers_context: SuperpowersContext,
    pub context_warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationSpecContext {
    pub artifact_id: Option<String>,
    pub version_id: Option<String>,
    pub title: Option<String>,
    pub raw_markdown_or_sections: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationRepoContext {
    pub branch_name: String,
    pub base_branch: String,
    pub worktree_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenSpecContext {
    pub enabled: bool,
    pub active_change_id: String,
    pub traceability_notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuperpowersContext {
    pub enabled: bool,
    pub required_methods_by_role: BTreeMap<String, Vec<String>>,
}

pub fn build_evaluation_context_pack(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    provider_role: EvaluationContextRole,
) -> Result<EvaluationContextPack, ProductStoreError> {
    let lifecycle = LifecycleStore::new(paths.clone());
    let work_items = lifecycle.list_work_items(&attempt.project_id, &attempt.issue_id)?;
    let work_item = work_items
        .into_iter()
        .find(|item| item.id == attempt.work_item_id);
    let mut warnings = Vec::new();

    let work_item_context = match work_item.as_ref() {
        Some(item) => spec_context_for_entity(&lifecycle, &attempt.project_id, &attempt.issue_id, &item.id, Some(item.title.clone()))?,
        None => {
            warnings.push("missing_work_item".to_string());
            EvaluationSpecContext::missing()
        }
    };

    let story_context = first_story_context(&lifecycle, attempt, work_item.as_ref(), &mut warnings)?;
    let design_context = first_design_context(&lifecycle, attempt, work_item.as_ref(), &mut warnings)?;

    Ok(EvaluationContextPack {
        issue_id: attempt.issue_id.clone(),
        attempt_id: attempt.id.clone(),
        stage: serde_json::to_value(&attempt.stage)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| format!("{:?}", attempt.stage)),
        provider_role,
        story_spec: story_context,
        design_spec: design_context,
        work_item: work_item_context,
        repo_context: EvaluationRepoContext {
            branch_name: attempt.branch_name.clone(),
            base_branch: attempt.base_branch.clone(),
            worktree_path: attempt.worktree_path.as_ref().map(|path| path.display().to_string()),
        },
        openspec_context: OpenSpecContext {
            enabled: true,
            active_change_id: attempt.issue_id.clone(),
            traceability_notes: vec![
                "Use Story Spec, Design Spec, and Work Item relationships as OpenSpec constraints.".to_string(),
            ],
        },
        superpowers_context: SuperpowersContext {
            enabled: true,
            required_methods_by_role: required_superpowers_methods(),
        },
        context_warnings: warnings,
    })
}

impl EvaluationSpecContext {
    fn missing() -> Self {
        Self {
            artifact_id: None,
            version_id: None,
            title: None,
            raw_markdown_or_sections: String::new(),
        }
    }
}

fn first_story_context(
    lifecycle: &LifecycleStore,
    attempt: &CodingExecutionAttempt,
    work_item: Option<&LifecycleWorkItemRecord>,
    warnings: &mut Vec<String>,
) -> Result<EvaluationSpecContext, ProductStoreError> {
    let Some(story_id) = work_item.and_then(|item| item.story_spec_ids.first()) else {
        warnings.push("missing_story_spec".to_string());
        return Ok(EvaluationSpecContext::missing());
    };
    let title = lifecycle
        .list_story_specs(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .find(|story| story.id == *story_id)
        .map(|story| story.title);
    spec_context_for_entity(lifecycle, &attempt.project_id, &attempt.issue_id, story_id, title)
}

fn first_design_context(
    lifecycle: &LifecycleStore,
    attempt: &CodingExecutionAttempt,
    work_item: Option<&LifecycleWorkItemRecord>,
    warnings: &mut Vec<String>,
) -> Result<EvaluationSpecContext, ProductStoreError> {
    let Some(design_id) = work_item.and_then(|item| item.design_spec_ids.first()) else {
        warnings.push("missing_design_spec".to_string());
        return Ok(EvaluationSpecContext::missing());
    };
    let title = lifecycle
        .list_design_specs(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .find(|design| design.id == *design_id)
        .map(|design| design.title);
    spec_context_for_entity(lifecycle, &attempt.project_id, &attempt.issue_id, design_id, title)
}

fn spec_context_for_entity(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    entity_id: &str,
    title: Option<String>,
) -> Result<EvaluationSpecContext, ProductStoreError> {
    let versions = latest_versions_for_entity(lifecycle, project_id, issue_id, entity_id)?;
    let latest = versions.last();
    Ok(EvaluationSpecContext {
        artifact_id: Some(entity_id.to_string()),
        version_id: latest.map(|version| version.id.clone()),
        title,
        raw_markdown_or_sections: latest
            .map(|version| version.markdown.clone())
            .unwrap_or_default(),
    })
}

fn latest_versions_for_entity(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    entity_id: &str,
) -> Result<Vec<SpecVersionRecord>, ProductStoreError> {
    let sessions = lifecycle.list_workspace_sessions(project_id, issue_id)?;
    let Some(session) = sessions
        .into_iter()
        .rev()
        .find(|session| session.entity_id == entity_id && matches!(session.workspace_type, WorkspaceType::Story | WorkspaceType::Design | WorkspaceType::WorkItem))
    else {
        return Ok(Vec::new());
    };
    lifecycle.list_artifact_versions(&session.id)
}

fn required_superpowers_methods() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([
        ("tester".to_string(), vec!["systematic-debugging".to_string(), "test-driven-development".to_string(), "verification-before-completion".to_string()]),
        ("analyst".to_string(), vec!["systematic-debugging".to_string(), "receiving-code-review".to_string()]),
        ("code_reviewer".to_string(), vec!["requesting-code-review".to_string(), "verification-before-completion".to_string()]),
        ("internal_reviewer".to_string(), vec!["requesting-code-review".to_string(), "verification-before-completion".to_string()]),
    ])
}

impl From<EvaluationContextRole> for CodingProviderRole {
    fn from(role: EvaluationContextRole) -> Self {
        match role {
            EvaluationContextRole::Tester => Self::Tester,
            EvaluationContextRole::Analyst => Self::Analyst,
            EvaluationContextRole::CodeReviewer => Self::CodeReviewer,
            EvaluationContextRole::InternalReviewer => Self::InternalReviewer,
        }
    }
}
```

- [ ] **Step 4: 运行上下文测试并确认通过**

Run:

```bash
cargo test --locked --lib evaluation_context_pack_includes_story_design_work_item_and_contracts
```

Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/product/mod.rs src/product/coding_evaluation_context.rs
git commit -m "feat: build coding evaluation context pack"
```

---

### Task 3: 持久化 TestPlan、raw output 与 blocked gate

**Files:**
- Modify: `src/product/coding_attempt_store.rs`
- Test: `src/product/coding_attempt_store.rs`

- [ ] **Step 1: 写 store 持久化测试**

在 `src/product/coding_attempt_store.rs` 的测试模块中新增测试：

```rust
#[test]
fn persists_test_plan_raw_output_and_blocked_gate() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    let attempt = store.create_attempt(CreateCodingAttemptInput {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        base_branch: "main".to_string(),
        branch_name: "bugfix_test_branch".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
        },
        max_auto_rework: 2,
    }).expect("attempt");

    let raw_ref = store.save_provider_raw_output(
        &attempt.id,
        CodingExecutionStage::Testing,
        "plan_tests",
        "raw plan output",
    ).expect("raw");
    assert_eq!(raw_ref, "provider-raw/testing/plan_tests_0001.txt");

    let plan = TestPlan {
        id: "test_plan_0001".to_string(),
        attempt_id: attempt.id.clone(),
        summary: "plan".to_string(),
        context_warnings: Vec::new(),
        assumptions: Vec::new(),
        steps: Vec::new(),
        created_at: "2026-06-10T00:00:00Z".to_string(),
        raw_provider_output_ref: Some(raw_ref.clone()),
    };
    store.save_test_plan(&plan).expect("save plan");
    assert_eq!(
        store.list_test_plans(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("plans")[0].raw_provider_output_ref,
        Some(raw_ref.clone())
    );

    let gate = store.create_blocked_gate(CreateBlockedGateInput {
        attempt_id: attempt.id.clone(),
        stage: CodingExecutionStage::Testing,
        role: CodingProviderRole::Tester,
        reason_code: "missing_required_steps".to_string(),
        title: "测试计划未完整执行".to_string(),
        description: "required step 未执行".to_string(),
        evidence_refs: vec!["testing_report_0001.json".to_string()],
        raw_provider_output_ref: Some(raw_ref),
        available_actions: vec![CodingGateAction {
            action_id: "rerun_missing_steps".to_string(),
            label: "重跑缺失步骤".to_string(),
            action_type: CodingGateActionType::RerunMissingSteps,
        }],
    }).expect("gate");

    let gates = store.list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("blocked gates");
    assert_eq!(gates[0].gate_id, gate.gate_id);
    assert_eq!(gates[0].reason_code.as_deref(), Some("missing_required_steps"));
    assert_eq!(gates[0].available_actions[0].action_type, CodingGateActionType::RerunMissingSteps);
}
```

- [ ] **Step 2: 运行 store 测试并确认失败**

Run:

```bash
cargo test --locked --lib persists_test_plan_raw_output_and_blocked_gate
```

Expected: FAIL，错误包含缺少 `save_provider_raw_output`、`save_test_plan`、`CreateBlockedGateInput` 或 `list_open_blocked_gates`。

- [ ] **Step 3: 实现 store API**

在 `src/product/coding_attempt_store.rs` imports 中补充：

```rust
    CodingGateAction, CodingGateKind, CodingGateRequired, TestPlan,
```

新增输入结构：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateBlockedGateInput {
    pub attempt_id: String,
    pub stage: CodingExecutionStage,
    pub role: CodingProviderRole,
    pub reason_code: String,
    pub title: String,
    pub description: String,
    pub evidence_refs: Vec<String>,
    pub raw_provider_output_ref: Option<String>,
    pub available_actions: Vec<CodingGateAction>,
}
```

在 `impl CodingAttemptStore` 中加入：

```rust
pub fn save_test_plan(&self, plan: &TestPlan) -> Result<(), ProductStoreError> {
    let attempt = self.find_attempt_by_id(&plan.attempt_id)?;
    write_json(
        &self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("test-plans")
            .join(format!("{}.json", plan.id)),
        plan,
    )
}

pub fn list_test_plans(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> Result<Vec<TestPlan>, ProductStoreError> {
    list_json_records(
        &self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join("test-plans"),
    )
}

pub fn save_provider_raw_output(
    &self,
    attempt_id: &str,
    stage: CodingExecutionStage,
    purpose: &str,
    output: &str,
) -> Result<String, ProductStoreError> {
    validate_relative_id(purpose)?;
    let attempt = self.find_attempt_by_id(attempt_id)?;
    let stage_dir = serde_json::to_value(&stage)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{:?}", stage).to_ascii_lowercase());
    validate_relative_id(&stage_dir)?;
    let root = self
        .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .join("provider-raw")
        .join(&stage_dir);
    fs::create_dir_all(&root)?;
    let id = next_sequential_id(purpose, count_text_files(&root)?);
    let file_name = format!("{id}.txt");
    fs::write(root.join(&file_name), output)?;
    Ok(format!("provider-raw/{stage_dir}/{file_name}"))
}

pub fn create_blocked_gate(
    &self,
    input: CreateBlockedGateInput,
) -> Result<CodingGateRequired, ProductStoreError> {
    let attempt = self.find_attempt_by_id(&input.attempt_id)?;
    let gates_root = self
        .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .join("blocked-gates");
    let gate_id = next_sequential_id("coding_blocked_gate", count_json_files(&gates_root)?);
    let gate = CodingGateRequired {
        gate_id: gate_id.clone(),
        kind: CodingGateKind::Blocked,
        title: input.title,
        description: input.description,
        stage: Some(input.stage),
        role: Some(input.role),
        expires_at: None,
        provider_snapshot: None,
        available_actions: input.available_actions,
        reason_code: Some(input.reason_code),
        evidence_refs: input.evidence_refs,
        raw_provider_output_ref: input.raw_provider_output_ref,
    };
    write_json(&gates_root.join(format!("{gate_id}.json")), &gate)?;
    Ok(gate)
}

pub fn list_open_blocked_gates(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> Result<Vec<CodingGateRequired>, ProductStoreError> {
    list_json_records(
        &self
            .attempt_dir(project_id, issue_id, attempt_id)
            .join("blocked-gates"),
    )
}

pub fn resolve_blocked_gate(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
    gate_id: &str,
) -> Result<(), ProductStoreError> {
    validate_relative_id(gate_id)?;
    let root = self
        .attempt_dir(project_id, issue_id, attempt_id)
        .join("blocked-gates");
    fs::create_dir_all(root.join("resolved"))?;
    let source = root.join(format!("{gate_id}.json"));
    if !path_is_regular_file(&source)? {
        return Err(ProductStoreError::NotFound {
            kind: "coding_blocked_gate",
            id: gate_id.to_string(),
        });
    }
    fs::rename(source, root.join("resolved").join(format!("{gate_id}.json")))?;
    Ok(())
}
```

在文件底部加入 helper：

```rust
fn count_text_files(root: &Path) -> Result<usize, ProductStoreError> {
    match fs::read_dir(root) {
        Ok(entries) => Ok(entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("txt"))
            .count()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(0),
        Err(error) => Err(ProductStoreError::Io(error.to_string())),
    }
}
```

- [ ] **Step 4: 运行 store 测试并确认通过**

Run:

```bash
cargo test --locked --lib persists_test_plan_raw_output_and_blocked_gate
```

Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/product/coding_attempt_store.rs
git commit -m "feat: persist coding QA plans and blocked gates"
```

---

### Task 4: 实现 TestPlan parser 与 plan-based TestingReport builder

**Files:**
- Modify: `src/product/tester_agent_loop.rs`
- Test: `src/product/tester_agent_loop.rs`

- [ ] **Step 1: 写 TestPlan parser 和 report builder 测试**

在 `src/product/tester_agent_loop.rs` 的测试模块中新增：

```rust
#[test]
fn parses_test_plan_from_provider_json_and_blocks_missing_required_step() {
    let raw = r#"
    ```json
    {
      "summary": "Plan",
      "context_warnings": [],
      "assumptions": [],
      "steps": [
        {
          "id": "unit",
          "title": "unit tests",
          "intent": "run unit tests",
          "required": true,
          "tool": "run_command",
          "risk_level": "low",
          "command_or_tool_input": {"command": ["cargo", "test", "--locked"]},
          "evidence_expectation": "exit_code=0",
          "related_requirements": ["story:REQ-1"],
          "related_design_constraints": [],
          "related_work_item_tasks": ["work_item:验证命令"]
        },
        {
          "id": "security",
          "title": "security review",
          "intent": "provider inspects security risk",
          "required": true,
          "tool": "provider_managed",
          "risk_level": "medium",
          "command_or_tool_input": {"instructions": "check command injection risks"},
          "evidence_expectation": "provider analysis",
          "related_requirements": [],
          "related_design_constraints": ["design:security"],
          "related_work_item_tasks": []
        }
      ]
    }
    ```
    "#;

    let plan = parse_test_plan_payload(
        "coding_attempt_0001",
        "test_plan_0001",
        raw,
        Some("provider-raw/testing/plan_tests_0001.txt".to_string()),
    ).expect("plan");

    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[0].tool, TestPlanTool::RunCommand);

    let report = build_plan_based_testing_report(
        "testing_report_0001",
        "coding_attempt_0001",
        &plan,
        vec![TestingStepResult {
            step_id: "unit".to_string(),
            title: "unit tests".to_string(),
            required: true,
            status: TestCommandStatus::Passed,
            evidence_refs: vec!["test-output/unit.stdout.log".to_string()],
            provider_analysis: None,
            started_at: "2026-06-10T00:00:00Z".to_string(),
            completed_at: Some("2026-06-10T00:00:01Z".to_string()),
        }],
        Vec::new(),
        Some(serde_json::json!({"summary": "passed"})),
        Some("provider-raw/testing/execute_test_plan_0001.txt".to_string()),
    );

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert_eq!(report.missing_required_steps, vec!["security".to_string()]);
}
```

- [ ] **Step 2: 运行测试并确认失败**

Run:

```bash
cargo test --locked --lib parses_test_plan_from_provider_json_and_blocks_missing_required_step
```

Expected: FAIL，错误包含缺少 `parse_test_plan_payload` 或 `build_plan_based_testing_report`。

- [ ] **Step 3: 实现 parser 与 builder**

在 `src/product/tester_agent_loop.rs` imports 中增加：

```rust
    TestPlan, TestPlanStep, TestPlanTool, TestingStepResult,
```

加入 payload 类型和函数：

```rust
#[derive(Debug, serde::Deserialize)]
struct RawTestPlanProviderPayload {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    context_warnings: Vec<String>,
    #[serde(default)]
    assumptions: Vec<String>,
    #[serde(default)]
    steps: Vec<TestPlanStep>,
}

pub fn parse_test_plan_payload(
    attempt_id: &str,
    plan_id: &str,
    raw_output: &str,
    raw_provider_output_ref: Option<String>,
) -> Result<TestPlan, String> {
    let json = extract_json_object(raw_output).ok_or_else(|| "missing_json_object".to_string())?;
    let payload: RawTestPlanProviderPayload =
        serde_json::from_str(json).map_err(|error| error.to_string())?;
    if payload.steps.is_empty() {
        return Err("test_plan_steps_empty".to_string());
    }
    let mut ids = std::collections::BTreeSet::new();
    for step in &payload.steps {
        if step.id.trim().is_empty() {
            return Err("test_plan_step_id_empty".to_string());
        }
        if !ids.insert(step.id.clone()) {
            return Err(format!("duplicate_test_plan_step_id: {}", step.id));
        }
        if step.title.trim().is_empty() || step.intent.trim().is_empty() || step.evidence_expectation.trim().is_empty() {
            return Err(format!("invalid_test_plan_step: {}", step.id));
        }
    }
    Ok(TestPlan {
        id: plan_id.to_string(),
        attempt_id: attempt_id.to_string(),
        summary: non_empty_trimmed(&payload.summary).unwrap_or_else(|| "Tester provider generated a test plan".to_string()),
        context_warnings: payload.context_warnings,
        assumptions: payload.assumptions,
        steps: payload.steps,
        created_at: Utc::now().to_rfc3339(),
        raw_provider_output_ref,
    })
}

pub fn build_plan_based_testing_report(
    report_id: &str,
    attempt_id: &str,
    plan: &TestPlan,
    steps: Vec<TestingStepResult>,
    unplanned_commands: Vec<TestCommand>,
    provider_claim: Option<Value>,
    raw_provider_output_ref: Option<String>,
) -> TestingReport {
    let executed_required = steps
        .iter()
        .filter(|step| step.required)
        .map(|step| step.step_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let missing_required_steps = plan
        .steps
        .iter()
        .filter(|step| step.required && !executed_required.contains(&step.id))
        .map(|step| step.id.clone())
        .collect::<Vec<_>>();
    let skipped_required_steps = steps
        .iter()
        .filter(|step| step.required && step.status == TestCommandStatus::Blocked)
        .map(|step| step.step_id.clone())
        .collect::<Vec<_>>();
    let has_failed_required = steps
        .iter()
        .any(|step| step.required && matches!(step.status, TestCommandStatus::Failed | TestCommandStatus::TimedOut));
    let all_required_executed = missing_required_steps.is_empty() && skipped_required_steps.is_empty();
    let has_warnings = !plan.context_warnings.is_empty()
        || steps.iter().any(|step| !step.required && step.status != TestCommandStatus::Passed);
    let overall_status = if !all_required_executed {
        TestingOverallStatus::Blocked
    } else if has_failed_required {
        TestingOverallStatus::Failed
    } else if has_warnings {
        TestingOverallStatus::PassedWithWarnings
    } else {
        TestingOverallStatus::Passed
    };
    TestingReport {
        id: report_id.to_string(),
        attempt_id: attempt_id.to_string(),
        plan_id: Some(plan.id.clone()),
        plan_summary: Some(plan.summary.clone()),
        commands: Vec::new(),
        steps,
        unplanned_commands,
        missing_required_steps,
        skipped_required_steps,
        context_warnings: plan.context_warnings.clone(),
        overall_status,
        provider_claim,
        raw_provider_output_ref,
        backend_verified: true,
        started_at: Utc::now().to_rfc3339(),
        completed_at: Some(Utc::now().to_rfc3339()),
    }
}
```

加入本地 helper，复用 engine 内同名逻辑：

```rust
fn extract_json_object(value: &str) -> Option<&str> {
    let start = value.find('{')?;
    let end = value.rfind('}')?;
    (start <= end).then(|| &value[start..=end])
}

fn non_empty_trimmed(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
```

同时更新旧 `build_testing_report`，让新增字段有确定值：

```rust
        plan_id: None,
        plan_summary: None,
        steps: Vec::new(),
        unplanned_commands: Vec::new(),
        missing_required_steps: Vec::new(),
        skipped_required_steps: Vec::new(),
        context_warnings: Vec::new(),
        raw_provider_output_ref: None,
```

- [ ] **Step 4: 运行测试并确认通过**

Run:

```bash
cargo test --locked --lib parses_test_plan_from_provider_json_and_blocks_missing_required_step
```

Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/product/tester_agent_loop.rs
git commit -m "feat: parse provider test plans"
```

---

### Task 5: Tester Node 改为 plan_tests -> execute_test_plan

**Files:**
- Modify: `src/product/tester_agent_loop.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `src/product/tester_agent_loop.rs`
- Test: `src/product/coding_workspace_engine.rs`

- [ ] **Step 1: 写 prompt 契约测试**

在 `src/product/tester_agent_loop.rs` 新增：

```rust
#[test]
fn tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools() {
    let attempt = test_attempt("coding_attempt_0001");
    let pack = serde_json::json!({
        "openspec_context": {"enabled": true},
        "superpowers_context": {"enabled": true},
        "work_item": {"raw_markdown_or_sections": "# Work Item"},
        "story_spec": {"raw_markdown_or_sections": "# Story"},
        "design_spec": {"raw_markdown_or_sections": "# Design"}
    });

    let prompt = build_tester_plan_prompt(&attempt, &pack);

    assert!(prompt.contains("plan_tests"));
    assert!(prompt.contains("execute_test_plan"));
    assert!(prompt.contains("[openspec_contract]"));
    assert!(prompt.contains("[superpowers_contract]"));
    assert!(prompt.contains("Story Spec"));
    assert!(prompt.contains("Design Spec"));
    assert!(prompt.contains("Work Item"));
    assert!(prompt.contains("step_id"));
    assert!(prompt.contains("不要硬编码某种语言或包管理器"));
}
```

- [ ] **Step 2: 运行 prompt 测试并确认失败**

Run:

```bash
cargo test --locked --lib tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools
```

Expected: FAIL，错误包含缺少 `build_tester_plan_prompt`。

- [ ] **Step 3: 实现 Tester plan prompt**

在 `src/product/tester_agent_loop.rs` 加入：

```rust
pub fn build_tester_plan_prompt(
    attempt: &CodingExecutionAttempt,
    evaluation_context: &serde_json::Value,
) -> String {
    format!(
        "Coding Workspace Tester plan_tests\n\
         你是 Tester Provider。你必须先制定 TestPlan，再执行 execute_test_plan。\n\
         Aria 是通用项目，不允许你把验证范围限制为 pnpm、cargo、uv、pytest 或任何单一语言生态。\n\
         你必须根据 Story Spec、Design Spec、Work Item、diff、项目规则和可用工具决定验证计划。\n\
         \n[openspec_contract]\n\
         - 你必须使用 Story Spec、Design Spec、Work Item 的追踪关系判断当前任务。\n\
         - 不得忽略 requirement、design decision、task dependency、risk。\n\
         - 如发现 Story/Design/Work Item 冲突，必须输出 blocked 原因。\n\
         \n[superpowers_contract]\n\
         - 你必须遵循系统化调试：先证据，后结论。\n\
         - 你必须遵循验证前置：先定义应验证内容，再执行验证。\n\
         - 你不得用未经执行的推断替代测试证据。\n\
         \n工具调用要求:\n\
         - execute_test_plan 阶段每次 tool call 必须携带 step_id。\n\
         - 未绑定 step_id 的命令只能记录为 unplanned_commands，不能计入 required step 通过。\n\
         \n输出 TestPlan JSON 字段:\n\
         {{\"summary\":\"...\",\"context_warnings\":[],\"assumptions\":[],\"steps\":[{{\"id\":\"...\",\"title\":\"...\",\"intent\":\"...\",\"required\":true,\"tool\":\"run_command|read_file|list_files|search_code|provider_managed\",\"risk_level\":\"low|medium|high\",\"command_or_tool_input\":{{}},\"evidence_expectation\":\"...\",\"related_requirements\":[],\"related_design_constraints\":[],\"related_work_item_tasks\":[]}}]}}\n\
         \nAttempt: {}\n\
         EvaluationContextPack:\n```json\n{}\n```\n",
        attempt.id,
        serde_json::to_string_pretty(evaluation_context).unwrap_or_else(|_| "{}".to_string())
    )
}
```

- [ ] **Step 4: 写 step_id 绑定测试**

在 `src/product/tester_agent_loop.rs` 新增：

```rust
#[test]
fn test_tool_call_without_step_id_is_unplanned_and_does_not_pass_required_step() {
    let plan = TestPlan {
        id: "test_plan_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        summary: "plan".to_string(),
        context_warnings: Vec::new(),
        assumptions: Vec::new(),
        steps: vec![TestPlanStep {
            id: "unit".to_string(),
            title: "unit tests".to_string(),
            intent: "run tests".to_string(),
            required: true,
            tool: TestPlanTool::RunCommand,
            risk_level: TestPlanRiskLevel::Low,
            command_or_tool_input: serde_json::json!({"command": ["cargo", "test", "--locked"]}),
            evidence_expectation: "exit_code=0".to_string(),
            related_requirements: Vec::new(),
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
        }],
        created_at: "2026-06-10T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
    };
    let report = build_plan_based_testing_report(
        "testing_report_0001",
        "coding_attempt_0001",
        &plan,
        Vec::new(),
        vec![TestCommand {
            command: vec!["cargo".to_string(), "test".to_string(), "--locked".to_string()],
            cwd: std::path::PathBuf::from("."),
            exit_code: Some(0),
            duration_ms: 1,
            stdout_ref: "stdout.log".to_string(),
            stderr_ref: "stderr.log".to_string(),
            status: TestCommandStatus::Passed,
        }],
        None,
        None,
    );

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert_eq!(report.missing_required_steps, vec!["unit".to_string()]);
    assert_eq!(report.unplanned_commands.len(), 1);
}
```

- [ ] **Step 5: 运行 step_id 测试并确认通过**

Run:

```bash
cargo test --locked --lib test_tool_call_without_step_id_is_unplanned_and_does_not_pass_required_step
```

Expected: PASS。

- [ ] **Step 6: 改造 engine 调用链**

在 `src/product/coding_workspace_engine.rs` 中：

- 在 `execute_testing_with_provider_commands` 里，provider 支持 tool calls 时先调用 `build_evaluation_context_pack(paths, attempt, EvaluationContextRole::Tester)`。
- 用 `build_tester_plan_prompt` 发起 `plan_tests` provider run。
- 保存 plan raw output：`save_provider_raw_output(attempt.id, Testing, "plan_tests", full_output)`。
- 调用 `parse_test_plan_payload`，保存 `TestPlan`。
- parse 失败时保存 blocked TestingReport，创建 testing blocked gate，stage 设为 blocked。
- parse 成功后再进入当前 tool-call loop，执行 `execute_test_plan`。
- tool call input 没有 `step_id` 时，命令结果放入 `unplanned_commands`。
- tool call input 有 `step_id` 且对应 plan step 时，生成 `TestingStepResult`。
- 最终调用 `build_plan_based_testing_report`。

Testing blocked gate actions 固定为：

```rust
vec![
    CodingGateAction {
        action_id: "retry_test_plan".to_string(),
        label: "重新生成测试计划".to_string(),
        action_type: CodingGateActionType::RetryTestPlan,
    },
    CodingGateAction {
        action_id: "rerun_missing_steps".to_string(),
        label: "重跑缺失步骤".to_string(),
        action_type: CodingGateActionType::RerunMissingSteps,
    },
    CodingGateAction {
        action_id: "provide_context".to_string(),
        label: "补充上下文".to_string(),
        action_type: CodingGateActionType::ProvideContext,
    },
    CodingGateAction {
        action_id: "manual_continue".to_string(),
        label: "人工确认继续".to_string(),
        action_type: CodingGateActionType::ManualContinue,
    },
    CodingGateAction {
        action_id: "abort".to_string(),
        label: "中止 Attempt".to_string(),
        action_type: CodingGateActionType::Abort,
    },
]
```

- [ ] **Step 7: 运行 Tester 相关测试**

Run:

```bash
cargo test --locked --lib tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools
cargo test --locked --lib parses_test_plan_from_provider_json_and_blocks_missing_required_step
cargo test --locked --lib test_tool_call_without_step_id_is_unplanned_and_does_not_pass_required_step
```

Expected: 全部 PASS。

- [ ] **Step 8: 提交**

```bash
git add src/product/tester_agent_loop.rs src/product/coding_workspace_engine.rs
git commit -m "feat: run provider-driven test plans"
```

---

### Task 6: Review raw output、容错解析与 blocked gate

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `src/product/coding_workspace_engine.rs`

- [ ] **Step 1: 写 review parser 容错测试**

在 `src/product/coding_workspace_engine.rs` tests 中新增：

```rust
#[test]
fn review_parser_preserves_findings_with_common_aliases() {
    let output = r#"{
      "verdict": "request_changes",
      "summary": "Need fixes",
      "findings": [
        {
          "file": "src/lib.rs",
          "line": 42,
          "description": "panic on empty input",
          "recommendation": "return an error",
          "failure_scenario": "empty payload"
        }
      ]
    }"#;

    let payload = parse_review_payload(output, CodingExecutionStage::CodeReview);

    assert_eq!(payload.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(payload.findings.len(), 1);
    assert_eq!(payload.findings[0].severity, FindingSeverity::Warning);
    assert_eq!(payload.findings[0].file_path.as_deref(), Some("src/lib.rs"));
    assert_eq!(payload.findings[0].message, "panic on empty input");
    assert_eq!(payload.findings[0].required_action.as_deref(), Some("return an error"));
    assert_eq!(payload.findings[0].source_stage, CodingExecutionStage::CodeReview);
}
```

- [ ] **Step 2: 运行 parser 测试并确认失败**

Run:

```bash
cargo test --locked --lib review_parser_preserves_findings_with_common_aliases
```

Expected: FAIL，当前 `RawReviewFinding` 缺 `severity` 会反序列化失败，最终 findings 为空。

- [ ] **Step 3: 扩展 RawReviewFinding 容错**

把 `RawReviewFinding` 改为：

```rust
#[derive(Debug, Deserialize)]
struct RawReviewFinding {
    #[serde(default)]
    severity: Option<crate::product::coding_models::FindingSeverity>,
    #[serde(default, alias = "file")]
    file_path: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default, alias = "description", alias = "failure_scenario")]
    message: Option<String>,
    #[serde(default, alias = "recommendation", alias = "fix")]
    required_action: Option<String>,
    #[serde(default)]
    source_stage: Option<CodingExecutionStage>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    evidence: Option<String>,
    #[serde(default)]
    related_requirements: Vec<String>,
    #[serde(default)]
    related_design_constraints: Vec<String>,
    #[serde(default)]
    related_work_item_tasks: Vec<String>,
}
```

把 `into_review_finding` 改为：

```rust
fn into_review_finding(self, default_source_stage: CodingExecutionStage) -> ReviewFinding {
    ReviewFinding {
        severity: self.severity.unwrap_or(FindingSeverity::Warning),
        file_path: self.file_path,
        line: self.line,
        message: self
            .message
            .or(self.title)
            .unwrap_or_else(|| "review finding".to_string()),
        required_action: self.required_action,
        source_stage: self.source_stage.unwrap_or(default_source_stage),
        evidence: self.evidence,
        related_requirements: self.related_requirements,
        related_design_constraints: self.related_design_constraints,
        related_work_item_tasks: self.related_work_item_tasks,
    }
}
```

- [ ] **Step 4: 保存 raw output 并创建 Review blocked gate**

在 `build_code_review_report` 和 `build_internal_pr_review` 中保存 raw ref，写入 report：

```rust
let raw_provider_output_ref = Some(self.store.save_provider_raw_output(
    &attempt.id,
    CodingExecutionStage::CodeReview,
    "code_review",
    full_output,
)?);
```

Review blocked gate actions 固定为：

```rust
vec![
    CodingGateAction {
        action_id: "retry_review".to_string(),
        label: "重试审查".to_string(),
        action_type: CodingGateActionType::RetryReview,
    },
    CodingGateAction {
        action_id: "send_raw_output_to_analyst".to_string(),
        label: "交给 Analyst 分析".to_string(),
        action_type: CodingGateActionType::SendRawOutputToAnalyst,
    },
    CodingGateAction {
        action_id: "provide_context".to_string(),
        label: "补充上下文".to_string(),
        action_type: CodingGateActionType::ProvideContext,
    },
    CodingGateAction {
        action_id: "manual_continue".to_string(),
        label: "人工确认继续".to_string(),
        action_type: CodingGateActionType::ManualContinue,
    },
    CodingGateAction {
        action_id: "abort".to_string(),
        label: "中止 Attempt".to_string(),
        action_type: CodingGateActionType::Abort,
    },
]
```

在 `execute_code_review_with_commands` 的 `ReviewVerdict::Blocked` 分支里调用 `create_blocked_gate`，发送 `CodingGateRequired` WebSocket 消息，再更新 attempt status 为 blocked。

- [ ] **Step 5: 运行 review 测试**

Run:

```bash
cargo test --locked --lib review_parser_preserves_findings_with_common_aliases
```

Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add src/product/coding_workspace_engine.rs
git commit -m "feat: recover blocked code reviews"
```

---

### Task 7: Analyst 与 Internal Reviewer 注入 OpenSpec/Superpowers 契约

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `src/product/coding_workspace_engine.rs`

- [ ] **Step 1: 写 prompt contract 测试**

在 `src/product/coding_workspace_engine.rs` tests 中新增：

```rust
#[test]
fn rework_and_internal_review_prompts_require_openspec_and_superpowers() {
    let attempt = test_attempt("coding_attempt_0001");
    let context_notes = ReworkContextNoteInput {
        text: "无".to_string(),
        truncated: false,
    };
    let rework_prompt = build_rework_prompt(
        &attempt,
        "testing failed",
        &CodingExecutionStage::Testing,
        1,
        &context_notes,
        "{\"story_spec\":{\"raw_markdown_or_sections\":\"# Story\"},\"design_spec\":{\"raw_markdown_or_sections\":\"# Design\"},\"work_item\":{\"raw_markdown_or_sections\":\"# Work Item\"}}",
    );

    assert!(rework_prompt.contains("[openspec_contract]"));
    assert!(rework_prompt.contains("[superpowers_contract]"));
    assert!(rework_prompt.contains("Story Spec"));
    assert!(rework_prompt.contains("Design Spec"));
    assert!(rework_prompt.contains("Work Item"));

    let contract = provider_runtime_contract("InternalReviewer");
    assert!(contract.contains("[openspec_contract]"));
    assert!(contract.contains("[superpowers_contract]"));
    assert!(contract.contains("InternalReviewer"));
}
```

- [ ] **Step 2: 运行测试并确认失败**

Run:

```bash
cargo test --locked --lib rework_and_internal_review_prompts_require_openspec_and_superpowers
```

Expected: FAIL，当前 rework prompt 没有 OpenSpec/Superpowers 契约。

- [ ] **Step 3: 新增统一 contract helper**

在 `src/product/coding_workspace_engine.rs` prompt helper 附近加入：

```rust
fn provider_runtime_contract(role: &str) -> String {
    format!(
        "[openspec_contract]\n\
         - {role} 必须使用 Story Spec、Design Spec、Work Item 的追踪关系判断当前任务。\n\
         - 不得忽略已确认的 requirement、design decision、task dependency、risk。\n\
         - 如发现 Story/Design/Work Item 冲突，必须报告 blocked 或请求人工澄清。\n\
         - OpenSpec 是需求、设计和追踪约束来源，不是运行时猜测来源。\n\
         \n[superpowers_contract]\n\
         - {role} 必须遵循系统化调试：先证据，后结论。\n\
         - {role} 必须遵循验证前置：先定义应验证内容，再执行验证或审查。\n\
         - {role} 必须在完成判断前给出验证证据。\n\
         - {role} 不得用未经执行的推断替代测试或审查证据。\n"
    )
}
```

先把 `build_rework_prompt` 签名改为接收 context JSON：

```rust
fn build_rework_prompt(
    attempt: &CodingExecutionAttempt,
    evidence: &str,
    source_stage: &CodingExecutionStage,
    rework_round: u32,
    context_notes: &ReworkContextNoteInput,
    evaluation_context_json: &str,
) -> String
```

在 `execute_rework_with_commands` 调用 `build_rework_prompt` 前构建 Analyst context：

```rust
let evaluation_context = build_evaluation_context_pack(
    &self.store.paths(),
    &attempt,
    EvaluationContextRole::Analyst,
)?;
let evaluation_context_json =
    serde_json::to_string_pretty(&evaluation_context).unwrap_or_else(|_| "{}".to_string());
```

并把 `&evaluation_context_json` 传给 `build_rework_prompt`。在 `build_rework_prompt` 中使用 `provider_runtime_contract("Analyst")` 并拼入 prompt。Code Reviewer 使用 `"CodeReviewer"`，Internal Reviewer 使用 `"InternalReviewer"`。

- [ ] **Step 4: 注入 EvaluationContextPack JSON**

在 `build_code_review_prompt` 和 `build_internal_pr_review_prompt` 中读取 `build_evaluation_context_pack`，把 JSON 追加到 prompt：

```rust
let evaluation_context = build_evaluation_context_pack(
    &self.store.paths(),
    attempt,
    EvaluationContextRole::CodeReviewer,
)?;
let evaluation_context_json =
    serde_json::to_string_pretty(&evaluation_context).unwrap_or_else(|_| "{}".to_string());
```

prompt 中加入以下文本片段：

````text
EvaluationContextPack:
```json
{evaluation_context_json}
```
````

- [ ] **Step 5: 运行 prompt 测试**

Run:

```bash
cargo test --locked --lib rework_and_internal_review_prompts_require_openspec_and_superpowers
```

Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add src/product/coding_workspace_engine.rs
git commit -m "feat: require openspec and superpowers in coding reviews"
```

---

### Task 8: WebSocket session state 合并 blocked gate 并处理 gate response

**Files:**
- Modify: `src/web/coding_ws_handler.rs`
- Test: `src/web/coding_ws_handler.rs`

- [ ] **Step 1: 写 session state blocked gate 测试**

在 `src/web/coding_ws_handler.rs` tests 中新增：

```rust
#[test]
fn blocked_attempt_allows_gate_response_messages() {
    assert!(is_coding_ws_message_allowed(
        &CodingAttemptStatus::Blocked,
        &CodingExecutionStage::CodeReview,
        &CodingWsInMessage::GateResponse {
            gate_id: "coding_blocked_gate_0001".to_string(),
            action_id: "retry_review".to_string(),
            extra_context: None,
        },
    ));
}
```

- [ ] **Step 2: 运行测试**

Run:

```bash
cargo test --locked --lib blocked_attempt_allows_gate_response_messages
```

Expected: PASS。这个测试锁定现有允许规则，防止恢复动作被协议层拦截。

- [ ] **Step 3: 合并 pending gates**

在 `build_coding_session_state` 中把：

```rust
let pending_gates = coding_store
    .list_open_stage_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
    .into_iter()
    .map(stage_gate_required)
    .collect();
```

替换为：

```rust
let mut pending_gates: Vec<CodingGateRequiredModel> = coding_store
    .list_open_stage_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
    .into_iter()
    .map(stage_gate_required)
    .collect();
pending_gates.extend(coding_store.list_open_blocked_gates(
    &attempt.project_id,
    &attempt.issue_id,
    &attempt.id,
)?);
```

- [ ] **Step 4: 处理 GateResponse**

在 inbound 分支中新增：

```rust
} else if let CodingWsInMessage::GateResponse {
    gate_id,
    action_id,
    extra_context,
} = inbound {
    let engine = CodingWorkspaceEngine::new(
        coding_store.clone(),
        GitWorkspaceService::new(),
        event_tx.clone(),
    );
    let updated = match engine
        .handle_blocked_gate_response(&current_attempt, &gate_id, &action_id, extra_context)
        .await
    {
        Ok(updated) => updated,
        Err(error) => {
            let _ = send_coding_json(
                &mut socket_tx,
                &CodingWsOutMessage::CodingProtocolError {
                    code: "coding_gate_response_failed".to_string(),
                    message: error.to_string(),
                },
            )
            .await;
            continue;
        }
    };
    if let Ok(snapshot) = build_coding_session_state(&coding_store, updated) {
        let _ = send_coding_json(&mut socket_tx, &snapshot).await;
    }
```

- [ ] **Step 5: 在 engine 实现 gate response handler**

在 `src/product/coding_workspace_engine.rs` 增加：

```rust
pub async fn handle_blocked_gate_response(
    &self,
    attempt: &CodingExecutionAttempt,
    gate_id: &str,
    action_id: &str,
    extra_context: Option<String>,
) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
    if action_id == "abort" {
        return self
            .handle_abort(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .await;
    }
    if let Some(context) = extra_context.and_then(|value| non_empty_trimmed(&value)) {
        let _ = self.store.create_context_note(&attempt.id, context)?;
    }
    self.store.resolve_blocked_gate(&attempt.project_id, &attempt.issue_id, &attempt.id, gate_id)?;
    match action_id {
        "retry_test_plan" | "rerun_missing_steps" => {
            let updated = self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Running,
            )?;
            self.store.update_attempt_stage(
                &updated.project_id,
                &updated.issue_id,
                &updated.id,
                CodingExecutionStage::Testing,
            )
            .map_err(Into::into)
        }
        "retry_review" | "send_raw_output_to_analyst" => {
            let updated = self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Running,
            )?;
            let stage = if action_id == "send_raw_output_to_analyst" {
                CodingExecutionStage::Rework
            } else {
                CodingExecutionStage::CodeReview
            };
            self.store
                .update_attempt_stage(&updated.project_id, &updated.issue_id, &updated.id, stage)
                .map_err(Into::into)
        }
        "provide_context" => self
            .store
            .update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::WaitingForHuman,
            )
            .map_err(Into::into),
        "manual_continue" | "accept_risk" => {
            let updated = self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Running,
            )?;
            Ok(updated)
        }
        other => Err(CodingWorkspaceEngineError::InvalidInput(format!(
            "unsupported_blocked_gate_action: {other}"
        ))),
    }
}
```

如果 `CodingWorkspaceEngineError` 没有 `InvalidInput` 变体，新增：

```rust
#[error("invalid coding workspace input: {0}")]
InvalidInput(String),
```

- [ ] **Step 6: 运行 WebSocket handler 测试**

Run:

```bash
cargo test --locked --lib blocked_attempt_allows_gate_response_messages
```

Expected: PASS。

- [ ] **Step 7: 提交**

```bash
git add src/web/coding_ws_handler.rs src/product/coding_workspace_engine.rs
git commit -m "feat: handle coding blocked gates"
```

---

### Task 9: 前端类型与 store 支持 TestingReport v2 和 blocked gate metadata

**Files:**
- Modify: `web/src/api/types.ts`
- Modify: `web/src/api/types.test.ts`
- Modify: `web/src/state/coding-workspace-store.ts`
- Modify: `web/src/state/coding-workspace-store.test.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.test.tsx`

- [ ] **Step 1: 写 TypeScript 类型测试**

在 `web/src/api/types.test.ts` 新增：

```ts
it("accepts plan based testing reports and blocked gate metadata", () => {
  const report: TestingReport = {
    id: "testing_report_0001",
    attempt_id: "coding_attempt_0001",
    plan_id: "test_plan_0001",
    plan_summary: "API smoke and security checks",
    commands: [],
    steps: [
      {
        step_id: "api",
        title: "API smoke",
        required: true,
        status: "passed",
        evidence_refs: ["test-output/api.stdout.log"],
        provider_analysis: "passed",
        started_at: "2026-06-10T00:00:00Z",
        completed_at: "2026-06-10T00:00:01Z",
      },
    ],
    unplanned_commands: [],
    missing_required_steps: [],
    skipped_required_steps: [],
    context_warnings: [],
    overall_status: "passed_with_warnings",
    provider_claim: { summary: "passed" },
    raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
    backend_verified: true,
    started_at: "2026-06-10T00:00:00Z",
    completed_at: "2026-06-10T00:00:01Z",
  };
  const gate: CodingGateRequired = {
    gate_id: "coding_blocked_gate_0001",
    kind: "blocked",
    title: "测试被阻塞",
    description: "required step 未执行",
    stage: "testing",
    role: "tester",
    reason_code: "missing_required_steps",
    evidence_refs: ["testing_report_0001.json"],
    raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
    available_actions: [
      {
        action_id: "rerun_missing_steps",
        label: "重跑缺失步骤",
        action_type: "rerun_missing_steps",
      },
    ],
  };

  expect(report.steps[0].step_id).toBe("api");
  expect(gate.reason_code).toBe("missing_required_steps");
});
```

- [ ] **Step 2: 运行前端类型测试并确认失败**

Run:

```bash
pnpm -C web test -- types.test.ts
```

Expected: FAIL，类型不包含 v2 字段和新 action type。

- [ ] **Step 3: 更新 `web/src/api/types.ts`**

加入类型：

```ts
export type TestPlanTool =
  | "run_command"
  | "read_file"
  | "list_files"
  | "search_code"
  | "provider_managed";
export type TestPlanRiskLevel = "low" | "medium" | "high";

export type TestPlanStep = {
  id: string;
  title: string;
  intent: string;
  required: boolean;
  tool: TestPlanTool;
  risk_level: TestPlanRiskLevel;
  command_or_tool_input: unknown;
  evidence_expectation: string;
  related_requirements: string[];
  related_design_constraints: string[];
  related_work_item_tasks: string[];
};

export type TestingStepResult = {
  step_id: string;
  title: string;
  required: boolean;
  status: TestCommandStatus;
  evidence_refs: string[];
  provider_analysis: string | null;
  started_at: string;
  completed_at: string | null;
};
```

更新：

```ts
export type TestingOverallStatus =
  | "passed"
  | "passed_with_warnings"
  | "failed"
  | "skipped_by_user_decision"
  | "blocked";
```

扩展 `TestingReport`：

```ts
  plan_id?: string | null;
  plan_summary?: string | null;
  steps: TestingStepResult[];
  unplanned_commands: TestCommand[];
  missing_required_steps: string[];
  skipped_required_steps: string[];
  context_warnings: string[];
  raw_provider_output_ref?: string | null;
```

扩展 `CodingGateActionType`：

```ts
  | "retry_test_plan"
  | "rerun_missing_steps"
  | "provide_context"
  | "manual_continue"
  | "retry_review"
  | "send_raw_output_to_analyst";
```

扩展 `CodingGateRequired`：

```ts
  reason_code?: string | null;
  evidence_refs?: string[];
  raw_provider_output_ref?: string | null;
```

- [ ] **Step 4: hook 响应 gate 后移除本地 pending gate**

在 `respondGate` 成功发送后追加：

```ts
if (!sendJson({
  type: "gate_response",
  gate_id: gateId,
  action_id: actionId,
  extra_context: extraContext ?? null,
})) {
  return;
}
useCodingWorkspaceStore.getState().resolvePendingGate(gateId);
```

- [ ] **Step 5: 运行前端 store/hook 测试**

Run:

```bash
pnpm -C web test -- types.test.ts coding-workspace-store.test.ts useCodingWorkspaceWs.test.tsx
```

Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add web/src/api/types.ts web/src/api/types.test.ts web/src/state/coding-workspace-store.ts web/src/state/coding-workspace-store.test.ts web/src/hooks/useCodingWorkspaceWs.ts web/src/hooks/useCodingWorkspaceWs.test.tsx
git commit -m "feat: support coding QA gate state in web types"
```

---

### Task 10: 前端展示 TestPlan、step 证据与 blocked gate 恢复动作

**Files:**
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`

- [ ] **Step 1: 写 Testing UI 测试**

在 `web/src/pages/CodingWorkspacePage.test.tsx` 新增：

```tsx
it("renders plan based testing report details", () => {
  mockCodingWs();
  useCodingWorkspaceStore.setState({
    attemptId: "coding_attempt_0001",
    status: "blocked",
    stage: "testing",
    testingReport: {
      id: "testing_report_0001",
      attempt_id: "coding_attempt_0001",
      plan_id: "test_plan_0001",
      plan_summary: "API smoke and security review",
      commands: [],
      steps: [
        {
          step_id: "api",
          title: "API smoke",
          required: true,
          status: "passed",
          evidence_refs: ["test-output/api.stdout.log"],
          provider_analysis: "passed",
          started_at: "2026-06-10T00:00:00Z",
          completed_at: "2026-06-10T00:00:01Z",
        },
      ],
      unplanned_commands: [],
      missing_required_steps: ["security"],
      skipped_required_steps: [],
      context_warnings: ["missing_design_spec"],
      overall_status: "blocked",
      provider_claim: { summary: "provider claimed passed" },
      raw_provider_output_ref: "provider-raw/testing/execute_test_plan_0001.txt",
      backend_verified: true,
      started_at: "2026-06-10T00:00:00Z",
      completed_at: "2026-06-10T00:00:01Z",
    },
  });

  render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

  expect(screen.getByText("API smoke and security review")).toBeInTheDocument();
  expect(screen.getByText("API smoke")).toBeInTheDocument();
  expect(screen.getByText("missing required: security")).toBeInTheDocument();
  expect(screen.getByText("missing_design_spec")).toBeInTheDocument();
});
```

- [ ] **Step 2: 写 blocked gate UI 测试**

在同文件新增：

```tsx
it("renders blocked gate metadata and sends recovery action", async () => {
  const api = mockCodingWs();
  useCodingWorkspaceStore.setState({
    attemptId: "coding_attempt_0001",
    status: "blocked",
    stage: "code_review",
    pendingGates: [
      {
        gate_id: "coding_blocked_gate_0001",
        kind: "blocked",
        title: "Code Review 被阻塞",
        description: "review 输出不是有效 JSON",
        stage: "code_review",
        role: "code_reviewer",
        reason_code: "review_payload_parse_error",
        evidence_refs: ["code_review_0001.json"],
        raw_provider_output_ref: "provider-raw/code_review/code_review_0001.txt",
        available_actions: [
          {
            action_id: "retry_review",
            label: "重试审查",
            action_type: "retry_review",
          },
        ],
      },
    ],
  });

  render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

  expect(screen.getByText("review_payload_parse_error")).toBeInTheDocument();
  expect(screen.getByText("provider-raw/code_review/code_review_0001.txt")).toBeInTheDocument();

  await userEvent.click(screen.getByRole("button", { name: "重试审查" }));

  expect(api.respondGate).toHaveBeenCalledWith(
    "coding_blocked_gate_0001",
    "retry_review",
    undefined,
  );
});
```

- [ ] **Step 3: 运行 UI 测试并确认失败**

Run:

```bash
pnpm -C web test -- CodingWorkspacePage.test.tsx
```

Expected: FAIL，页面还不展示 v2 字段。

- [ ] **Step 4: 实现 TestingReportCard v2 展示**

在 `CodingWorkspacePage.tsx` 的 Testing 面板组件中追加 v2 内容：

```tsx
{report.plan_summary ? (
  <div className="rounded-md border border-[var(--aria-line)] p-2 text-xs">
    <div className="font-semibold text-[var(--aria-ink)]">Test Plan</div>
    <div className="mt-1 text-[var(--aria-ink-muted)]">{report.plan_summary}</div>
  </div>
) : null}
{report.steps.length > 0 ? (
  <div className="space-y-2">
    {report.steps.map((step) => (
      <div key={step.step_id} className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2 text-xs">
        <div className="flex items-center justify-between gap-2">
          <div className="font-semibold text-[var(--aria-ink)]">{step.title}</div>
          <StatusBadge value={step.status} />
        </div>
        <div className="mt-1 text-[var(--aria-ink-muted)]">
          {step.required ? "required" : "optional"} · {step.step_id}
        </div>
        {step.provider_analysis ? (
          <div className="mt-1 text-[var(--aria-ink-muted)]">{step.provider_analysis}</div>
        ) : null}
        {step.evidence_refs.length > 0 ? (
          <div className="mt-1 font-mono text-[var(--aria-ink-muted)]">
            {step.evidence_refs.join(", ")}
          </div>
        ) : null}
      </div>
    ))}
  </div>
) : null}
{report.missing_required_steps.length > 0 ? (
  <div className="text-xs font-semibold text-amber-700">
    missing required: {report.missing_required_steps.join(", ")}
  </div>
) : null}
{report.context_warnings.length > 0 ? (
  <div className="flex flex-wrap gap-1">
    {report.context_warnings.map((warning) => (
      <span key={warning} className="rounded border border-amber-200 bg-amber-50 px-1.5 py-0.5 text-xs text-amber-700">
        {warning}
      </span>
    ))}
  </div>
) : null}
```

- [ ] **Step 5: 扩展 GatePanel blocked metadata**

在 `GatePanel` 非 stage gate 分支中，在 description 下加入：

```tsx
{gate.reason_code ? (
  <div className="mt-1 font-mono text-xs text-amber-800">{gate.reason_code}</div>
) : null}
{gate.raw_provider_output_ref ? (
  <div className="mt-1 truncate font-mono text-xs text-amber-800">
    {gate.raw_provider_output_ref}
  </div>
) : null}
{gate.evidence_refs?.length ? (
  <div className="mt-1 truncate font-mono text-xs text-amber-800">
    {gate.evidence_refs.join(", ")}
  </div>
) : null}
```

- [ ] **Step 6: 运行 UI 测试并确认通过**

Run:

```bash
pnpm -C web test -- CodingWorkspacePage.test.tsx
```

Expected: PASS。

- [ ] **Step 7: 提交**

```bash
git add web/src/pages/CodingWorkspacePage.tsx web/src/pages/CodingWorkspacePage.test.tsx
git commit -m "feat: show coding QA plans and recovery gates"
```

---

### Task 11: 端到端回归与文档核对

**Files:**
- Verify only

- [ ] **Step 1: 运行 Rust 格式检查**

Run:

```bash
cargo fmt --check
```

Expected: PASS。

- [ ] **Step 2: 运行 Rust clippy**

Run:

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: PASS。

- [ ] **Step 3: 运行 Rust check**

Run:

```bash
cargo check --locked
```

Expected: PASS。

- [ ] **Step 4: 运行 Rust 全量测试**

Run:

```bash
cargo test --locked
```

Expected: PASS。

- [ ] **Step 5: 运行前端测试**

Run:

```bash
pnpm -C web test
```

Expected: PASS。

- [ ] **Step 6: 运行前端 build**

Run:

```bash
pnpm -C web build
```

Expected: PASS。

- [ ] **Step 7: 做真实场景检查**

使用当前 worktree 服务执行一次 Coding Workspace attempt，选择真实 Provider。检查点：

- Testing 节点先输出 TestPlan。
- TestPlan 中至少包含 Provider 根据 Story/Design/WorkItem 判断出的 required steps。
- tool call 结果绑定 `step_id`。
- 未执行 required step 时，TestingReport 为 `blocked`，且前端显示 blocked gate。
- Code Reviewer schema 异常时，raw output 落盘，前端显示 `retry_review`、`send_raw_output_to_analyst`、`provide_context`、`manual_continue`、`abort`。
- Analyst、Code Reviewer、Internal Reviewer 的 prompt 中包含 `[openspec_contract]` 与 `[superpowers_contract]`。

- [ ] **Step 8: 提交验证记录**

如果执行中新增回归记录，保存到 `cadence/reports/`，命名格式：

```text
2026-06-10_进度报告_CodingWorkspaceProvider驱动测试审查与恢复机制验证_v1.0.md
```

提交：

```bash
git add cadence/reports/2026-06-10_进度报告_CodingWorkspaceProvider驱动测试审查与恢复机制验证_v1.0.md
git commit -m "docs: record coding QA recovery verification"
```

---

## 执行顺序

1. Task 1-4 完成后，后端具备 plan/report/store 基础能力。
2. Task 5 完成后，Tester Node 能按 TestPlan 执行并生成 blocked gate。
3. Task 6-8 完成后，Review/Analyst/Internal Review 契约与恢复链路闭环。
4. Task 9-10 完成后，前端能展示计划、证据、缺失项与恢复动作。
5. Task 11 完成后，再进入真实端到端验收。

## 风险控制

- 不把 Rust、Node、Python、Go、Java、pnpm、cargo、pytest、安全扫描命令写入 Aria 核心策略。
- Aria 只校验 Provider 输出结构、step 绑定、required step 完整性、工具权限和证据引用。
- request_changes 进入 Analyst 返修，blocked 进入可恢复 gate；两者不得混用。
- Reviewer 输出即使不合 schema，也必须保存 raw output。
- 新字段必须使用 `#[serde(default)]` 或 optional 类型兼容历史记录。
- 不使用 Docker 作为本地验证路径。
- 不给 `cargo test` 增加 `-j 1`。
