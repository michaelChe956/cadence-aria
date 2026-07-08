# Coding Workspace 加载 Final Compile Work Item 上下文 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 Coding Workspace 首次 coding prompt 丢失 Work Item 内容的问题，确保 provider 优先拿到 Final Compile 产生的正式 Work Item、验证计划与写入边界。

**Architecture:** `coding_execution_context` 以 Web 层注入的 `ProductAppPaths` 作为唯一 `.aria` 数据根目录，再用 `CodingExecutionAttempt.current_work_item_id` 读取对应的 Final Compile 正式 `LifecycleWorkItemRecord` 与 `verification_plan_ref` 对应的 `VerificationPlan`，生成 prompt markdown 与验证命令。Work Item workspace artifact 只作为人工确认产物快照追加，不能再作为唯一数据源；draft 只在正式记录缺关键上下文字段时通过 `source_work_item_plan_id/source_draft_id` 受控补充。

**Tech Stack:** Rust 2024、serde JSON store、`LifecycleStore`、`WorkItemPlanStore`、Coding Workspace Engine、宿主机 Cargo。遵守 `cadence/project-rules/build-test-commands.md`，禁止给 `cargo test` 携带 `-j 1`。

---

## 文档信息

- 文档类型：计划文档
- 日期：2026-07-02
- 版本：v1.0
- 适用分支：`feat-b-0630`
- 适用开发 worktree：`.worktrees/feat-b-0630`
- 问题案例：`Coding Attempt #coding_attempt_0001`
- 关联 Work Item：`work_item_compile_20260702063721302_001`
- 关联 Work Item Plan：`Work Item Plan #workspace_session_0003`

## 背景与根因

用户抓到的 provider prompt 只有 Coding Workspace header、worktree、依赖初始化诊断与执行要求，没有“已确认 Work Item”段落。provider 日志进一步证明 coder 没拿到 Work Item 内容，因此才在 coding worktree 里尝试找 `.aria` 运行态数据，并报告“当前 worktree 没有 .aria 运行态数据”。

已确认本次开发环境后端服务读取的是 `feat-b-0630/.aria`，不是主仓库 `.aria`，也不是 coding worktree `.aria`。这只说明开发环境的 app root 当前由 `aria web --workspace .` 映射到 worktree 下；正式发布环境应映射到用户 `~/.aria`。所以本问题的直接根因不是 provider 应该去哪个物理目录找 `.aria`，而是 Coding prompt context 构建逻辑在已注入的 app root 内选错了唯一数据源。

当前代码路径：

- `src/web/coding_ws_handler/context.rs::coding_execution_context`
  - 先找当前 Work Item 的 `WorkspaceSessionRecord`
  - 再从 `list_artifact_versions(session.id)` 或 session message artifact 读取 markdown
  - 当前 `workspace_session_0004` 是 Work Item workspace，`entity_id=work_item_compile_20260702063721302_001`，但 `current_version=null` 且没有 `workspace-timelines/workspace_session_0004/artifact_versions.json`
  - 结果 `work_item_markdown=None`
- `src/product/coding_workspace_engine/prompts.rs::build_coding_prompt`
  - 只有 `context.work_item_markdown.is_some()` 时才插入“已确认 Work Item”
  - 因此 provider 首次 prompt 丢失正式 Work Item 内容

正确的数据事实：

- Final Compile 已经在当前运行环境的 app root 下产生正式 Work Item record：
  - `{ProductAppPaths::root()}/projects/project_0001/issues/issue_0001/work-items/work_item_compile_20260702063721302_001.json`
- 该 record 已包含：
  - `title`
  - `source_work_item_plan_id`
  - `source_outline_id`
  - `source_draft_id`
  - `planned_implementation_context`
  - `planned_handoff_summary`
  - `exclusive_write_scopes`
  - `forbidden_write_scopes`
  - `depends_on`
  - `required_handoff_from`
  - `verification_plan_ref`
- Final Compile 也在当前运行环境的 app root 下产生正式 verification plan：
  - `{ProductAppPaths::root()}/projects/project_0001/issues/issue_0001/verification-plans/verification_plan_compile_20260702063721302_001.json`

## 运行环境与 `.aria` root 约束

`.aria` 的物理位置是启动层配置，不是 Coding prompt context 自己推导出来的路径：

- 开发环境：当前使用 `aria web --workspace .` 在 `.worktrees/feat-b-0630` 启动，因此 Web state 的 `workspace_root` 是该 worktree，`ProductAppPaths::new(state.workspace_root.join(".aria"))` 解析为 `.worktrees/feat-b-0630/.aria`。
- 当前 `npx @cadence-aria/cli` 无参启动：launcher 只注入 `web --port <p> --host 127.0.0.1`，没有传 `--workspace`；Rust 端 `parse_workspace` 会使用进程 cwd，因此当前实际落点是 `$(pwd)/.aria`。
- 目标正式发布行为：默认运行 `npx @cadence-aria/cli` 时，产品数据必须落在用户 `~/.aria`。实现上应让 launcher 默认 web mode 显式传入用户 home 作为 workspace root，即 `--workspace <os.homedir()>`；Rust 端继续按现有 `workspace_root/.aria` 得到 `~/.aria`。
- 显式 workspace 行为：如果用户主动运行 `npx @cadence-aria/cli web --workspace /some/path`，继续尊重用户参数，产品数据落在 `/some/path/.aria`。
- 单元测试：继续使用 `TempDir/.aria`，验证逻辑不依赖真实 home 目录，也不依赖当前开发 worktree。

本修复的硬约束：

- `coding_execution_context` 不允许从 `attempt.worktree_path` 拼 `.aria`。
- `coding_execution_context` 不允许从当前进程 cwd 拼 `.aria`。
- `coding_execution_context` 不允许硬编码 `feat-b-0630/.aria` 或 `~/.aria`。
- 所有 lifecycle / work item / verification / draft 读取都必须从入参 `app_paths: &ProductAppPaths` 出发。

如果执行时发现正式发布版当前没有把 app root 映射到 `~/.aria`，应单独立一个启动层路径修复任务，优先检查 `src/cli.rs::parse_workspace`、`src/web/handlers/support.rs::product_app_paths`、`src/web/coding_ws_handler/socket.rs`、`src/web/coding_ws_handler/runner.rs`。这属于发布路径配置问题，不应混入本次 Coding prompt context 数据源修复。

本计划已把默认 `npx` 入口的 `~/.aria` 目标纳入 Task 7。该任务只改 npm launcher 默认 web mode，不改 Rust `ProductAppPaths` 的语义。

## 数据源优先级

本次修复必须按以下顺序组织 Coding prompt context：

1. `CodingExecutionAttempt.current_work_item_id`，不存在时使用 `attempt.work_item_id`
2. `LifecycleStore::list_work_items(project_id, issue_id)` 中匹配 id 的 `LifecycleWorkItemRecord`
3. `work_item.verification_plan_ref` 对应的 `LifecycleStore::get_verification_plan(...)`
4. `source_work_item_plan_id/source_outline_id/source_draft_id` 对应的 draft 内容，仅当正式 Work Item 记录缺关键字段时补充
5. Work Item workspace artifact version 或 session message artifact，作为“Workspace Artifact Snapshot”追加，不能覆盖 Final Compile canonical context

明确不采用的口径：

- 不再把 Work Item workspace artifact 当作唯一来源。
- 不再把 draft 当作首选来源。
- 不要求 provider 自己在 coding worktree 里查 `.aria`；`.aria` 必须由后端按 `ProductAppPaths` 读取后注入 prompt。
- 不把 `handoff_summary_ref` 与计划阶段的 `planned_handoff_summary` 混用。

## 文件结构

### 需要修改

- `src/web/coding_ws_handler/context.rs`
  - 新增 Final Compile Work Item context 构建 helper
  - 新增 verification plan 命令提取 helper
  - 修改 `coding_execution_context` 的数据源优先级
  - 保留 `select_work_item_markdown` 的 workspace artifact 兼容行为

- `src/web/coding_ws_handler/tests.rs`
  - 新增回归测试：无 workspace artifact version 时仍能从 Final Compile Work Item record 生成 `work_item_markdown`
  - 新增回归测试：workspace artifact 存在时 Final Compile canonical context 仍在 prompt 前半段
  - 新增回归测试：正式记录缺 planned context 时可从 source draft 受控补充

- `src/product/coding_workspace_engine/tests/parser_prompt.rs`
  - 新增 prompt guard：`build_coding_prompt` 输出包含“已确认 Work Item”、Final Compile context 与验证命令

- `npm/cli/bin/aria.js`
  - 默认 web mode 下显式传入 `--workspace <用户 home 目录>`，让 Rust 端现有 `workspace_root/.aria` 规则落到 `~/.aria`

- `npm/cli/test/launch.test.mjs`
  - 增加 launcher 默认 web mode 的回归断言：无参或只带 `--no-open` 时转发参数必须包含 `--workspace`，且值为 `homedir()`

### 可能只读参考

- `src/product/models/lifecycle.rs`
  - `LifecycleWorkItemRecord` 已有 `source_*`、`planned_*`、write scopes、verification ref 字段，本次不需要新增模型字段

- `src/product/models/verification.rs`
  - `VerificationPlan`、`VerificationCommand`

- `src/product/work_item_plan_store.rs`
  - `WorkItemPlanStore::list_draft_records`
  - `WorkItemPlanStore::get_draft_record`

- `src/web/handlers/support.rs`
  - `product_app_paths(state)` 当前把 Web `workspace_root` 映射为 `workspace_root/.aria`；只作为理解开发环境 app root 的参考，本次不直接修改

- `src/cli.rs`
  - `parse_workspace(args)` 当前决定 `aria web --workspace` 的 workspace root；Task 7 不修改它，而是让 npm launcher 显式传入用户 home

### 不建议修改

- 不修改 `.claude/rules/`
- 不修改 Final Compile 投影字段，除非执行 Task 1 时发现现有正式记录字段实际为空
- 不调整 Story/Design/Work Item workspace artifact 共同链路，因为本问题落点是 Coding prompt context，不是三类 artifact 展示或确认流程
- 不把 `--workspace` 设为 `~/.aria`，否则 Rust 端会再拼 `.aria`，变成 `~/.aria/.aria`
- 不改变显式 `web --workspace /some/path` 的语义

---

## Task 1: 写失败测试，复现无 workspace artifact 时 prompt context 丢失

**Files:**
- Modify: `src/web/coding_ws_handler/tests.rs`

- [ ] **Step 1: 补充测试 imports**

在 `src/web/coding_ws_handler/tests.rs` 顶部补充以下 imports：

```rust
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::{CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage};
use crate::product::lifecycle_store::{
    CreateVerificationPlanInput, CreateWorkItemInput, LifecycleStore,
};
use crate::product::models::{
    RepositoryProfileConfidence, VerificationCommand, VerificationCommandSafety,
    VerificationCommandSource, VerificationFallbackPolicy, VerificationScope, WorkItemKind,
    WorkItemPlanStatus,
};
use tempfile::TempDir;
```

同时把现有 `super::{ ... }` import 补上：

```rust
use super::{
    CodingExecutionAttempt, CodingWsInMessage, ProviderConfigSnapshot, coding_execution_context,
    is_coding_ws_message_allowed, select_work_item_markdown,
    should_resume_runner_after_gate_response,
};
```

- [ ] **Step 2: 新增失败测试**

在 `falls_back_to_assistant_artifact_when_persisted_markdown_lacks_commands` 后新增：

```rust
#[test]
fn coding_execution_context_uses_final_compile_work_item_when_workspace_artifact_missing() {
    let (_tmp, app_paths, attempt) = seed_final_compile_work_item_without_workspace_artifact();

    let context = coding_execution_context(&app_paths, &attempt).expect("coding context");

    let markdown = context.work_item_markdown.expect("work item markdown");
    assert!(markdown.contains("# Final Compile Work Item"));
    assert!(markdown.contains("work_item_compile_20260702063721302_001"));
    assert!(markdown.contains("Final Compile title"));
    assert!(markdown.contains("source_work_item_plan_id: issue_work_item_plan_0001"));
    assert!(markdown.contains("source_outline_id: outline_backend"));
    assert!(markdown.contains("source_draft_id: draft_backend"));
    assert!(markdown.contains("planned implementation context for coder"));
    assert!(markdown.contains("src/web/coding_ws_handler/context.rs"));
    assert!(markdown.contains("forbidden/path"));
    assert!(markdown.contains("verification_plan_compile_20260702063721302_001"));
    assert!(markdown.contains("cargo test --locked --lib coding_execution_context"));
    assert_eq!(
        context.verification_commands,
        vec!["cargo test --locked --lib coding_execution_context".to_string()]
    );
}
```

- [ ] **Step 3: 新增测试 fixture**

在测试文件底部新增：

```rust
fn seed_final_compile_work_item_without_workspace_artifact() -> (TempDir, ProductAppPaths, CodingExecutionAttempt) {
    let tmp = TempDir::new().expect("temp dir");
    let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let work_item_id = "work_item_compile_20260702063721302_001";
    let verification_plan_id = "verification_plan_compile_20260702063721302_001";

    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some(work_item_id.to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: vec!["story_spec_0001".to_string()],
            design_spec_ids: vec!["design_spec_0001".to_string()],
            title: "Final Compile title".to_string(),
            source_work_item_plan_id: Some("issue_work_item_plan_0001".to_string()),
            source_outline_id: Some("outline_backend".to_string()),
            source_draft_id: Some("draft_backend".to_string()),
            planned_implementation_context: Some(
                "planned implementation context for coder\n- touch src/web/coding_ws_handler/context.rs".to_string(),
            ),
            planned_handoff_summary: Some("planned handoff summary for dependent work items".to_string()),
            kind: WorkItemKind::Backend,
            sequence_hint: Some(1),
            depends_on: vec!["work_item_compile_dependency_001".to_string()],
            exclusive_write_scopes: vec!["src/web/coding_ws_handler/context.rs".to_string()],
            forbidden_write_scopes: vec!["forbidden/path".to_string()],
            required_handoff_from: vec!["work_item_compile_dependency_001".to_string()],
            verification_plan_ref: Some(verification_plan_id.to_string()),
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create final compile work item");

    lifecycle
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some(verification_plan_id.to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            repository_profile_ref: None,
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "cmd_001".to_string(),
                label: "context unit test".to_string(),
                command: "cargo test --locked --lib coding_execution_context".to_string(),
                cwd: ".".to_string(),
                purpose: "verify coding context uses final compile work item".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: Vec::new(),
            required_gates: vec!["cargo fmt --check".to_string()],
            risk_notes: vec!["provider prompt must include final compile context".to_string()],
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .expect("create verification plan");

    let attempt = CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: work_item_id.to_string(),
        attempt_no: 1,
        scope: CodingAttemptScope::WorkItemGroup,
        status: CodingAttemptStatus::Running,
        stage: CodingExecutionStage::Coding,
        base_branch: "main".to_string(),
        branch_name: "aria/issues/issue_0001".to_string(),
        worktree_path: Some(tmp.path().join("coding-worktree")),
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Fake),
            review_rounds: 1,
        },
        provider_conversations: Vec::new(),
        rework_count: 0,
        max_auto_rework: 2,
        work_item_group_id: None,
        current_work_item_id: Some(work_item_id.to_string()),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        created_at: "2026-07-02T00:00:00Z".to_string(),
        updated_at: "2026-07-02T00:00:00Z".to_string(),
        completed_at: None,
    };

    (tmp, app_paths, attempt)
}
```

- [ ] **Step 4: 运行定向测试，确认失败**

Run:

```bash
cargo test --locked --lib coding_execution_context_uses_final_compile_work_item_when_workspace_artifact_missing
```

Expected: 测试失败，`context.work_item_markdown` 为 `None` 或缺少 `# Final Compile Work Item`。

---

## Task 2: 实现 Final Compile canonical Work Item markdown

**Files:**
- Modify: `src/web/coding_ws_handler/context.rs`

- [ ] **Step 1: 补充 imports**

在 `context.rs` 顶部补充模型和 store imports：

```rust
use crate::product::models::{
    LifecycleWorkItemRecord, ProviderName, VerificationPlan, WorkItemExecutionPlanStatus,
    WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_plan_store::WorkItemPlanStore;
```

保留已有 `ProviderName`、`WorkItemExecutionPlanStatus`、`WorkspaceSessionRecord` 等 imports，避免重复 import。

- [ ] **Step 2: 新增 context 结构体**

在 `current_work_item_id_for_attempt` 后新增：

```rust
#[derive(Debug, Clone, Default)]
struct CompiledWorkItemContext {
    markdown: Option<String>,
    verification_commands: Vec<String>,
}
```

- [ ] **Step 3: 新增 Final Compile context 构建函数**

在 `coding_execution_context` 前新增：

```rust
fn compiled_work_item_context(
    lifecycle: &LifecycleStore,
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    current_work_item_id: &str,
) -> Result<CompiledWorkItemContext, ProductStoreError> {
    let Some(work_item) = lifecycle
        .list_work_items(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .find(|item| item.id == current_work_item_id)
    else {
        return Ok(CompiledWorkItemContext::default());
    };

    let verification_plan = match work_item.verification_plan_ref.as_deref() {
        Some(plan_id) => Some(lifecycle.get_verification_plan(
            &attempt.project_id,
            &attempt.issue_id,
            plan_id,
        )?),
        None => None,
    };

    let draft_supplement = final_compile_draft_supplement(app_paths, attempt, &work_item)?;
    let markdown = compiled_work_item_markdown(&work_item, verification_plan.as_ref(), draft_supplement.as_deref());
    let verification_commands = verification_plan
        .as_ref()
        .map(verification_command_lines)
        .unwrap_or_default();

    Ok(CompiledWorkItemContext {
        markdown: Some(markdown),
        verification_commands,
    })
}
```

- [ ] **Step 4: 新增 markdown formatter**

在 `select_work_item_markdown` 前新增：

```rust
fn compiled_work_item_markdown(
    work_item: &LifecycleWorkItemRecord,
    verification_plan: Option<&VerificationPlan>,
    draft_supplement: Option<&str>,
) -> String {
    let mut markdown = String::new();
    markdown.push_str("# Final Compile Work Item\n\n");
    markdown.push_str(&format!("- Work Item ID: {}\n", work_item.id));
    markdown.push_str(&format!("- Title: {}\n", work_item.title));
    markdown.push_str(&format!("- Kind: {}\n", work_item.kind.as_str()));
    push_optional_line(&mut markdown, "source_work_item_plan_id", work_item.source_work_item_plan_id.as_deref());
    push_optional_line(&mut markdown, "source_outline_id", work_item.source_outline_id.as_deref());
    push_optional_line(&mut markdown, "source_draft_id", work_item.source_draft_id.as_deref());
    push_optional_line(&mut markdown, "verification_plan_ref", work_item.verification_plan_ref.as_deref());

    push_markdown_section(
        &mut markdown,
        "Planned Implementation Context",
        work_item.planned_implementation_context.as_deref(),
    );
    push_markdown_section(
        &mut markdown,
        "Planned Handoff Summary",
        work_item.planned_handoff_summary.as_deref(),
    );
    push_string_list(&mut markdown, "Story Spec IDs", &work_item.story_spec_ids);
    push_string_list(&mut markdown, "Design Spec IDs", &work_item.design_spec_ids);
    push_string_list(&mut markdown, "Depends On", &work_item.depends_on);
    push_string_list(&mut markdown, "Required Handoff From", &work_item.required_handoff_from);
    push_string_list(&mut markdown, "Exclusive Write Scopes", &work_item.exclusive_write_scopes);
    push_string_list(&mut markdown, "Forbidden Write Scopes", &work_item.forbidden_write_scopes);

    if let Some(plan) = verification_plan {
        markdown.push_str("\n## Verification Plan\n\n");
        markdown.push_str(&format!("- Verification Plan ID: {}\n", plan.id));
        markdown.push_str(&format!("- Scope: {}\n", plan.scope.as_str()));
        if !plan.commands.is_empty() {
            markdown.push_str("\n### 验证命令\n\n");
            for command in &plan.commands {
                markdown.push_str(&format!(
                    "- `{}`: {} (cwd: {}, required: {})\n",
                    command.label,
                    command.command,
                    command.cwd,
                    command.required
                ));
            }
        }
        push_string_list(&mut markdown, "Required Gates", &plan.required_gates);
        push_string_list(&mut markdown, "Risk Notes", &plan.risk_notes);
    }

    if let Some(supplement) = draft_supplement {
        push_markdown_section(&mut markdown, "Source Draft Supplement", Some(supplement));
    }

    markdown
}
```

- [ ] **Step 5: 新增小 helper**

在 `compiled_work_item_markdown` 后新增：

```rust
fn verification_command_lines(plan: &VerificationPlan) -> Vec<String> {
    plan.commands
        .iter()
        .map(|command| command.command.trim())
        .filter(|command| !command.is_empty())
        .map(str::to_string)
        .collect()
}

fn push_optional_line(markdown: &mut String, label: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        markdown.push_str(&format!("- {label}: {}\n", value.trim()));
    }
}

fn push_markdown_section(markdown: &mut String, heading: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        markdown.push_str(&format!("\n## {heading}\n\n{}\n", value.trim()));
    }
}

fn push_string_list(markdown: &mut String, heading: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    markdown.push_str(&format!("\n## {heading}\n\n"));
    for value in values {
        markdown.push_str("- ");
        markdown.push_str(value);
        markdown.push('\n');
    }
}
```

- [ ] **Step 6: 运行 Task 1 测试，确认仍因未接线失败**

Run:

```bash
cargo test --locked --lib coding_execution_context_uses_final_compile_work_item_when_workspace_artifact_missing
```

Expected: 若 helper 尚未接入 `coding_execution_context`，测试仍失败；若编译失败，先修正 imports 或 helper 可见性。

---

## Task 3: 接入 coding_execution_context 并保留 workspace artifact 兼容

**Files:**
- Modify: `src/web/coding_ws_handler/context.rs`
- Test: `src/web/coding_ws_handler/tests.rs`

- [ ] **Step 1: 抽出 workspace artifact markdown helper**

把 `coding_execution_context` 内原有的 session artifact 读取逻辑抽为函数：

```rust
fn workspace_artifact_work_item_markdown(
    lifecycle: &LifecycleStore,
    attempt: &CodingExecutionAttempt,
    current_work_item_id: &str,
) -> Result<Option<String>, ProductStoreError> {
    let sessions = lifecycle.list_workspace_sessions(&attempt.project_id, &attempt.issue_id)?;
    let work_item_session = sessions
        .iter()
        .rev()
        .find(|session| {
            session.entity_id == current_work_item_id
                && session.workspace_type == WorkspaceType::WorkItem
                && session.status == WorkspaceSessionStatus::Confirmed
        })
        .or_else(|| {
            sessions.iter().rev().find(|session| {
                session.entity_id == current_work_item_id
                    && session.workspace_type == WorkspaceType::WorkItem
            })
        });

    match work_item_session {
        Some(session) => Ok(lifecycle
            .list_artifact_versions(&session.id)?
            .into_iter()
            .last()
            .map(|version| version.to_markdown_string())
            .and_then(|markdown| select_work_item_markdown(Some(markdown), session))
            .or_else(|| select_work_item_markdown(None, session))),
        None => Ok(None),
    }
}
```

- [ ] **Step 2: 新增 markdown 合并 helper**

```rust
fn merge_work_item_markdown(
    compiled_markdown: Option<String>,
    workspace_markdown: Option<String>,
) -> Option<String> {
    match (compiled_markdown, workspace_markdown) {
        (Some(compiled), Some(workspace)) if !workspace.trim().is_empty() && workspace.trim() != compiled.trim() => {
            Some(format!(
                "{}\n\n---\n\n## Workspace Artifact Snapshot\n\n{}",
                compiled.trim(),
                workspace.trim()
            ))
        }
        (Some(compiled), _) => Some(compiled),
        (None, Some(workspace)) if !workspace.trim().is_empty() => Some(workspace),
        (None, _) => None,
    }
}
```

- [ ] **Step 3: 新增命令合并 helper**

```rust
fn merge_verification_commands(
    compiled_commands: Vec<String>,
    markdown: Option<&str>,
) -> Vec<String> {
    let mut commands = Vec::new();
    for command in compiled_commands {
        push_unique_command(&mut commands, command);
    }
    if commands.is_empty() {
        if let Some(markdown) = markdown {
            for spec in planned_test_commands_from_markdown(markdown) {
                push_unique_command(&mut commands, spec.command.join(" "));
            }
        }
    }
    commands
}

fn push_unique_command(commands: &mut Vec<String>, command: String) {
    let command = command.trim();
    if !command.is_empty() && !commands.iter().any(|existing| existing == command) {
        commands.push(command.to_string());
    }
}
```

- [ ] **Step 4: 修改 `coding_execution_context`**

用以下结构替换现有实现主体：

```rust
pub(crate) fn coding_execution_context(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<CodingExecutionContext, ProductStoreError> {
    let current_work_item_id = current_work_item_id_for_attempt(attempt);
    let lifecycle = LifecycleStore::new(app_paths.clone());

    let compiled_context =
        compiled_work_item_context(&lifecycle, app_paths, attempt, current_work_item_id)?;
    let workspace_markdown =
        workspace_artifact_work_item_markdown(&lifecycle, attempt, current_work_item_id)?;
    let work_item_markdown =
        merge_work_item_markdown(compiled_context.markdown, workspace_markdown);
    let verification_commands = merge_verification_commands(
        compiled_context.verification_commands,
        work_item_markdown.as_deref(),
    );

    Ok(CodingExecutionContext {
        work_item_markdown,
        verification_commands,
    })
}
```

- [ ] **Step 5: 运行 Task 1 测试，确认通过**

Run:

```bash
cargo test --locked --lib coding_execution_context_uses_final_compile_work_item_when_workspace_artifact_missing
```

Expected: PASS。

- [ ] **Step 6: 新增 workspace artifact 不覆盖 Final Compile 的回归测试**

在 `tests.rs` 新增：

```rust
#[test]
fn coding_execution_context_appends_workspace_artifact_without_overriding_final_compile() {
    let (_tmp, app_paths, attempt) = seed_final_compile_work_item_without_workspace_artifact();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let session = lifecycle
        .create_workspace_session(crate::product::lifecycle_store::CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "work_item_compile_20260702063721302_001".to_string(),
            workspace_type: WorkspaceType::WorkItem,
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
        })
        .expect("create workspace session");

    lifecycle
        .append_artifact_version(
            &session.id,
            crate::web::workspace_ws_types::ArtifactVersion {
                version: 1,
                payload: crate::web::workspace_ws_types::ArtifactPayload::Markdown {
                    markdown: "# Workspace Work Item\n\n## 验证命令\n\n```bash\ncargo check --locked\n```".to_string(),
                    diff: None,
                },
                generated_by: ProviderName::Fake,
                reviewed_by: Some(ProviderName::Fake),
                review_verdict: None,
                confirmed_by: Some("user".to_string()),
                is_current: true,
                created_at: "2026-07-02T00:00:00Z".to_string(),
                source_node_id: "node_0001".to_string(),
            },
        )
        .expect("append artifact version");

    let context = coding_execution_context(&app_paths, &attempt).expect("coding context");
    let markdown = context.work_item_markdown.expect("work item markdown");
    let final_compile_index = markdown.find("# Final Compile Work Item").expect("final compile heading");
    let artifact_index = markdown.find("## Workspace Artifact Snapshot").expect("artifact snapshot");

    assert!(final_compile_index < artifact_index);
    assert!(markdown.contains("planned implementation context for coder"));
    assert!(markdown.contains("# Workspace Work Item"));
    assert_eq!(
        context.verification_commands,
        vec!["cargo test --locked --lib coding_execution_context".to_string()]
    );
}
```

- [ ] **Step 7: 运行新增测试**

Run:

```bash
cargo test --locked --lib coding_execution_context_appends_workspace_artifact_without_overriding_final_compile
```

Expected: PASS。

---

## Task 4: 正式记录不足时受控补充 source draft

**Files:**
- Modify: `src/web/coding_ws_handler/context.rs`
- Test: `src/web/coding_ws_handler/tests.rs`

- [ ] **Step 1: 实现 draft 是否需要补充的判断**

在 `context.rs` 新增：

```rust
fn needs_source_draft_supplement(work_item: &LifecycleWorkItemRecord) -> bool {
    work_item
        .planned_implementation_context
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .is_empty()
        || work_item
            .planned_handoff_summary
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
}
```

- [ ] **Step 2: 实现 source draft supplement**

在 `context.rs` 新增：

```rust
fn final_compile_draft_supplement(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    work_item: &LifecycleWorkItemRecord,
) -> Result<Option<String>, ProductStoreError> {
    if !needs_source_draft_supplement(work_item) {
        return Ok(None);
    }
    let Some(plan_id) = work_item.source_work_item_plan_id.as_deref() else {
        return Ok(None);
    };
    let Some(draft_id) = work_item.source_draft_id.as_deref() else {
        return Ok(None);
    };

    let store = WorkItemPlanStore::new(app_paths.clone());
    let draft = store
        .list_draft_records(&attempt.project_id, &attempt.issue_id, plan_id)?
        .into_iter()
        .find(|record| record.draft_id == draft_id);

    let Some(draft) = draft else {
        return Ok(None);
    };

    let mut markdown = String::new();
    markdown.push_str(&format!("- Draft ID: {}\n", draft.draft_id));
    markdown.push_str(&format!("- Outline ID: {}\n", draft.outline_id));
    push_markdown_section(
        &mut markdown,
        "Draft Implementation Context",
        Some(&draft.candidate.implementation_context),
    );
    push_markdown_section(
        &mut markdown,
        "Draft Handoff Summary",
        Some(&draft.candidate.handoff_summary),
    );
    push_string_list(&mut markdown, "Draft Exclusive Write Scopes", &draft.candidate.exclusive_write_scopes);
    push_string_list(&mut markdown, "Draft Forbidden Write Scopes", &draft.candidate.forbidden_write_scopes);
    push_string_list(&mut markdown, "Draft Depends On Outline IDs", &draft.candidate.depends_on_outline_ids);
    push_string_list(
        &mut markdown,
        "Draft Required Handoff From Outline IDs",
        &draft.candidate.required_handoff_from_outline_ids,
    );
    if !draft.candidate.verification_plan.is_null() {
        push_markdown_section(
            &mut markdown,
            "Draft Verification Plan JSON",
            Some(&draft.candidate.verification_plan.to_string()),
        );
    }

    Ok((!markdown.trim().is_empty()).then_some(markdown))
}
```

- [ ] **Step 3: 新增 draft 补充回归测试**

在 `tests.rs` 新增一个最小测试，先用 `WorkItemPlanStore::put_draft_record` 写入 `draft_backend`，再创建一个 `planned_implementation_context=None` 的 Work Item，断言 markdown 包含 `Draft Implementation Context`：

```rust
#[test]
fn coding_execution_context_supplements_from_source_draft_only_when_final_compile_context_is_missing() {
    let (_tmp, app_paths, mut attempt) = seed_final_compile_work_item_without_workspace_artifact();
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let sparse_work_item_id = "work_item_compile_sparse_001";
    let plan_id = "issue_work_item_plan_0001";
    let draft_id = "draft_sparse_backend";

    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some(sparse_work_item_id.to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            title: "Sparse final compile title".to_string(),
            source_work_item_plan_id: Some(plan_id.to_string()),
            source_outline_id: Some("outline_sparse_backend".to_string()),
            source_draft_id: Some(draft_id.to_string()),
            planned_implementation_context: None,
            planned_handoff_summary: None,
            verification_plan_ref: None,
            plan_status: WorkItemPlanStatus::Confirmed,
            ..Default::default()
        })
        .expect("create sparse work item");

    crate::product::work_item_plan_store::WorkItemPlanStore::new(app_paths.clone())
        .put_draft_record(&crate::product::models::WorkItemDraftRecord {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: plan_id.to_string(),
            draft_id: draft_id.to_string(),
            outline_id: "outline_sparse_backend".to_string(),
            generation_round_id: "round_001".to_string(),
            batch_id: None,
            attempt_index: 1,
            outline_version_ref: "outline_version_001".to_string(),
            generation_mode: crate::product::models::WorkItemGenerationMode::Serial,
            candidate: crate::product::models::WorkItemDraftCandidate {
                outline_id: "outline_sparse_backend".to_string(),
                title: "Draft sparse backend".to_string(),
                kind: WorkItemKind::Backend,
                goal: "restore draft context".to_string(),
                implementation_context: "draft implementation context used only as supplement".to_string(),
                exclusive_write_scopes: vec!["src/web/coding_ws_handler/context.rs".to_string()],
                forbidden_write_scopes: vec!["forbidden/draft/path".to_string()],
                depends_on_outline_ids: Vec::new(),
                required_handoff_from_outline_ids: Vec::new(),
                handoff_summary: "draft handoff summary used only as supplement".to_string(),
                verification_plan: serde_json::json!({
                    "commands": [
                        { "command": "cargo check --locked", "label": "check" }
                    ]
                }),
            },
            status: crate::product::models::WorkItemDraftStatus::Accepted,
            active: true,
            superseded_by_draft_id: None,
            supersede_reason: None,
            copied_from_draft_id: None,
            review_node_id: None,
            review_verdict_ref: None,
            generated_from_node_id: "node_draft_author".to_string(),
            accepted_at: Some("2026-07-02T00:00:00Z".to_string()),
            superseded_at: None,
            created_at: "2026-07-02T00:00:00Z".to_string(),
            updated_at: "2026-07-02T00:00:00Z".to_string(),
        })
        .expect("put draft record");

    attempt.work_item_id = sparse_work_item_id.to_string();
    attempt.current_work_item_id = Some(sparse_work_item_id.to_string());

    let context = coding_execution_context(&app_paths, &attempt).expect("coding context");
    let markdown = context.work_item_markdown.expect("work item markdown");

    assert!(markdown.contains("# Final Compile Work Item"));
    assert!(markdown.contains("Sparse final compile title"));
    assert!(markdown.contains("## Source Draft Supplement"));
    assert!(markdown.contains("draft implementation context used only as supplement"));
    assert!(markdown.contains("draft handoff summary used only as supplement"));
}
```

- [ ] **Step 4: 运行 draft 补充测试**

Run:

```bash
cargo test --locked --lib coding_execution_context_supplements_from_source_draft_only_when_final_compile_context_is_missing
```

Expected: PASS。

---

## Task 5: Prompt guard，确保 provider 首次 prompt 含 Work Item 内容

**Files:**
- Modify: `src/product/coding_workspace_engine/tests/parser_prompt.rs`

- [ ] **Step 1: 新增 prompt guard 测试**

在 `parser_prompt.rs` 新增：

```rust
#[test]
fn coding_prompt_includes_final_compile_work_item_context_and_commands() {
    let attempt = sample_attempt();
    let context = CodingExecutionContext {
        work_item_markdown: Some(
            "# Final Compile Work Item\n\n- Work Item ID: work_item_compile_001\n\n## Planned Implementation Context\n\nuse context.rs".to_string(),
        ),
        verification_commands: vec!["cargo test --locked --lib coding_execution_context".to_string()],
    };

    let prompt = build_coding_prompt(&attempt, &context, None, None);

    assert!(prompt.contains("验证命令:"));
    assert!(prompt.contains("- cargo test --locked --lib coding_execution_context"));
    assert!(prompt.contains("已确认 Work Item:"));
    assert!(prompt.contains("# Final Compile Work Item"));
    assert!(prompt.contains("work_item_compile_001"));
    assert!(prompt.contains("Planned Implementation Context"));
    assert!(prompt.contains("优先按已确认 Work Item 的文件落点、范围和验证命令执行"));
}
```

如果该测试文件没有 `sample_attempt()`，按现有测试风格新增一个局部 helper，字段与现有 `CodingExecutionAttempt` 构造保持一致。

- [ ] **Step 2: 运行 prompt guard**

Run:

```bash
cargo test --locked --lib coding_prompt_includes_final_compile_work_item_context_and_commands
```

Expected: PASS。该测试主要防止未来修改 `build_coding_prompt` 时再次移除“已确认 Work Item”段。

---

## Task 6: 回归验证与手工验收

**Files:**
- No code changes in this task

- [ ] **Step 1: 运行 coding context 定向单测**

Run:

```bash
cargo test --locked --lib coding_execution_context
```

Expected: PASS，包含 Task 1、Task 3、Task 4 新增测试。

- [ ] **Step 2: 运行 prompt 定向单测**

Run:

```bash
cargo test --locked --lib coding_prompt_includes_final_compile_work_item_context_and_commands
```

Expected: PASS。

- [ ] **Step 3: 运行格式检查**

Run:

```bash
cargo fmt --check
```

Expected: PASS。

- [ ] **Step 4: 运行编译检查**

Run:

```bash
cargo check --locked
```

Expected: PASS。

- [ ] **Step 5: 运行相关 lib 测试**

Run:

```bash
cargo test --locked --lib coding_ws_handler
```

Expected: PASS。

- [ ] **Step 6: 手工验收 provider prompt**

重启后端后重新触发 `Coding Attempt #coding_attempt_0001`，检查 provider 首次 prompt 必须包含：

- `已确认 Work Item:`
- `# Final Compile Work Item`
- `work_item_compile_20260702063721302_001`
- `Final Compile title` 或真实 Work Item title
- `planned_implementation_context` 对应内容
- `exclusive_write_scopes`
- `forbidden_write_scopes`
- `verification_plan_compile_20260702063721302_001`
- `cargo ...` 或该 Work Item verification plan 中的真实命令

provider 日志不应再因为 prompt 缺内容而进入“当前 worktree 没有 .aria 运行态数据，无法直接读取 Attempt 产物”的路径。

---

## Task 7: npx 默认启动使用用户 home 作为 workspace root

**Files:**
- Modify: `npm/cli/bin/aria.js`
- Modify: `npm/cli/test/launch.test.mjs`

- [ ] **Step 1: 写失败测试：默认 web mode 必须转发用户 home workspace**

修改 `npm/cli/test/launch.test.mjs` 的 import：

```javascript
import { homedir, tmpdir } from "node:os";
```

在 `default web mode injects --port and forwards to binary` 测试里，`assert.equal(received[0], "web");` 后追加：

```javascript
  const workspaceIndex = received.indexOf("--workspace");
  assert.notEqual(workspaceIndex, -1, "默认 web mode 应注入 --workspace");
  assert.equal(
    received[workspaceIndex + 1],
    homedir(),
    "默认 web mode 应以用户 home 作为 workspace root，使产品数据落到 ~/.aria",
  );
```

- [ ] **Step 2: 运行 launcher 单测，确认失败**

Run:

```bash
node --test npm/cli/test/launch.test.mjs
```

Expected: FAIL，提示默认 web mode 未注入 `--workspace`。

- [ ] **Step 3: 修改 launcher 默认 web mode**

在 `npm/cli/bin/aria.js` 顶部补充：

```javascript
const { homedir } = require("node:os");
```

把默认 web mode 的 forward args 从：

```javascript
forwardArgs = ["web", "--port", String(port), "--host", "127.0.0.1"];
```

改为：

```javascript
forwardArgs = [
  "web",
  "--workspace",
  homedir(),
  "--port",
  String(port),
  "--host",
  "127.0.0.1",
];
```

这样 Rust 端收到的 `workspace_root` 是用户 home，现有 `ProductAppPaths::new(state.workspace_root.join(".aria"))` 会得到 `~/.aria`。

- [ ] **Step 4: 运行 launcher 单测，确认通过**

Run:

```bash
node --test npm/cli/test/launch.test.mjs
```

Expected: PASS。

- [ ] **Step 5: 运行 npm launcher 全量单测**

Run:

```bash
node --test "npm/cli/test/*.test.mjs"
```

Expected: PASS。

- [ ] **Step 6: 手工验收默认 npx 行为**

在任意非 home 目录运行：

```bash
npx @cadence-aria/cli --no-open
```

Expected:

- 后端启动成功。
- 产品数据写入用户 home 下的 `~/.aria`。
- 当前目录不应新建产品级 `.aria/projects/...` 数据。

- [ ] **Step 7: 手工验收显式 workspace 行为不变**

运行：

```bash
npx @cadence-aria/cli web --workspace /tmp/aria-explicit-workspace --host 127.0.0.1 --port 4317
```

Expected:

- launcher 原样透传参数，不自动覆盖 workspace。
- 产品数据写入 `/tmp/aria-explicit-workspace/.aria`。
- 不写入 `~/.aria`，除非该路径同时被用户显式设为 workspace。

## 风险与边界

- 如果正式 Work Item record 存在但 `verification_plan_ref` 指向的文件不存在，本方案默认返回 store error，暴露数据损坏，而不是静默吞掉。
- 如果正式 Work Item record 完整，draft supplement 不会出现，避免 coder 被多个 draft 版本干扰。
- 如果正式 Work Item record 不存在，仍保留旧 workspace artifact 兼容路径，支持历史数据和非 Final Compile 旧流程。
- Coding context 部分只影响 Coding Workspace prompt context，不会改变 Work Item workspace 页面展示、Story/Design workspace artifact 渲染、Final Compile transaction 写入顺序。
- npx 启动层部分只影响默认 web mode。用户显式传入 `web --workspace /some/path`、`task run --workspace /some/path` 或其它子命令时继续原样透传。

## Commit 建议

```bash
git add src/web/coding_ws_handler/context.rs src/web/coding_ws_handler/tests.rs src/product/coding_workspace_engine/tests/parser_prompt.rs npm/cli/bin/aria.js npm/cli/test/launch.test.mjs
git commit -m "fix(coding): load final compile work item context"
```

## Self Review

- Spec coverage：方案覆盖了用户关心的三个核心点：优先 Final Compile 正式输出，draft 只在正式内容不足时补充，默认 `npx @cadence-aria/cli` 正式使用用户 home 作为 workspace root 从而落到 `~/.aria`。
- Placeholder scan：本文没有保留待补充、稍后实现、细节后填等占位内容。
- Type consistency：方案使用现有 `LifecycleWorkItemRecord`、`VerificationPlan`、`WorkItemPlanStore`、`CodingExecutionContext`、`CodingExecutionAttempt` 字段；新增 helper 只位于 `coding_ws_handler/context.rs`，不新增公共 API。
