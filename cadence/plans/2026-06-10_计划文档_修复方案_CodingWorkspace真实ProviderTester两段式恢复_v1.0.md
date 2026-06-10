# CodingWorkspace 真实 Provider Tester 两段式恢复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复真实 Coding Workspace attempt 中 Tester 仍走 legacy 固定命令、没有 TestPlan 和 Tester 气泡的问题，并移除 Coding Workspace Tester 的 legacy fallback。

**Architecture:** 将“是否进入 provider-driven Tester”从 `supports_tool_calls()` 中拆出独立 capability，避免把“Provider 能否让 Aria 接管工具调用”和“Provider 能否按 TestPlan 契约输出测试结果”混为一谈。真实 ClaudeCode/Codex Provider 应进入 `plan_tests -> execute_test_plan` 两段式；若 Provider 不支持、不可用或输出不符合契约，应生成 blocked report/gate。Coding Workspace Tester 不再静默回退 legacy fixed commands，也不得再生成 `plan_id=null && steps=[] && raw_provider_output_ref=null` 的 legacy passed report。

**Tech Stack:** Rust 1.95.0、Cargo、Axum WebSocket、React/Vite、pnpm、Cadence Aria Coding Workspace。

---

## 背景与问题

当前真实 attempt 的 provider 配置示例：

```json
{
  "coder": "codex",
  "tester": "claude_code",
  "analyst": "codex",
  "code_reviewer": "claude_code",
  "internal_reviewer": "codex",
  "review_rounds": 1
}
```

但真实 Tester 没有进入 provider-driven TestPlan 路径。后端当前判断：

```rust
if !provider.supports_tool_calls() {
    return self.execute_testing(attempt, specs).await;
}
```

`ClaudeCodeProvider` 和 `CodexProvider` 没有覆盖 `supports_tool_calls()`，默认返回 `false`，因此真实 Tester 回退到 legacy 固定命令执行器：

- `cargo test --locked`
- `pnpm -C web test`

真实结果表现为：

- `TestingReport.plan_id == null`
- `TestingReport.steps == []`
- `TestingReport.raw_provider_output_ref == null`
- 没有 `provider-raw/testing/plan_tests_*.txt`
- 没有 `provider-raw/testing/execute_test_plan_*.txt`
- 页面没有 Tester provider 气泡。

## 不改范围

- 不调整 Coding Workspace 阶段顺序。
- 不调整 `Coding -> Testing -> Analyst -> CodeReview -> Analyst -> ReviewRequest` 编排。
- 不改 CodeReviewer 的阶段职责。
- 不物理清理历史 `.aria` attempt 中已经生成的 legacy testing report。
- 不以旧固定命令执行器作为 Coding Workspace Tester fallback；本方案完成后，Coding Workspace Tester 运行时路径没有 legacy fallback。

## 文件结构

- Modify: `src/cross_cutting/streaming_provider.rs`
  - 新增 provider-driven testing capability 默认实现。
- Modify: `src/cross_cutting/claude_code_provider.rs`
  - ClaudeCodeProvider 声明支持 provider-driven testing。
- Modify: `src/cross_cutting/codex_provider.rs`
  - CodexProvider 声明支持 provider-driven testing。
- Modify: `src/web/test_controls.rs`
  - TestControlledFakeStreamingProvider 同步声明支持 provider-driven testing。
- Modify: `src/product/coding_workspace_engine.rs`
  - Tester 入口改用新 capability。
  - 移除 Coding Workspace Tester 到固定命令执行器的 fallback。
  - 关闭 Tester `plan_tests` 阶段的 `run_legacy_stream_to_completion` 兼容 fallback；`provider.start` 不可用时进入 blocked。
  - 移除 Tester 相关 `legacy_execute_prompt` / `Legacy tester context` 注入，execute prompt 只依赖 TestPlan、EvaluationContextPack 和明确的 `step_results` 契约。
  - provider-driven testing capability 不支持、Provider 启动失败、TestPlan 解析失败、execute 输出不满足 `step_results` 契约时，统一生成 blocked report/gate。
  - 补 provider-driven Tester 的 chat entry 持久化。
  - 强化 execute 阶段契约失败时 blocked，不伪装 legacy passed。
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
  - 如现有 chat entry 渲染未覆盖 Tester role，则补 Tester 气泡展示。
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`
  - 补 Tester 气泡渲染测试。
- Test: `src/cross_cutting/*_provider.rs`
  - capability 单元测试。
- Test: `src/product/coding_workspace_engine.rs`
  - provider-driven Tester report/gate 单元测试。
- Test: `web/src/pages/CodingWorkspacePage.test.tsx`
  - 前端 Tester 气泡测试。

## Task 1: 拆分 Provider-Driven Testing Capability

**Files:**

- Modify: `src/cross_cutting/streaming_provider.rs`
- Modify: `src/cross_cutting/claude_code_provider.rs`
- Modify: `src/cross_cutting/codex_provider.rs`
- Modify: `src/web/test_controls.rs`

- [ ] **Step 1: 写失败测试，证明真实 Provider 应支持 provider-driven testing**

在 `src/cross_cutting/claude_code_provider.rs` 的 tests 中新增：

```rust
#[test]
fn claude_code_provider_supports_provider_driven_testing() {
    use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;

    let provider = ClaudeCodeProvider::new(std::path::PathBuf::from("claude"));

    assert!(provider.supports_provider_driven_testing());
}
```

在 `src/cross_cutting/codex_provider.rs` 的 tests 中新增：

```rust
#[test]
fn codex_provider_supports_provider_driven_testing() {
    use crate::cross_cutting::streaming_provider::StreamingProviderAdapter;

    let provider = CodexProvider::new(std::path::PathBuf::from("codex"));

    assert!(provider.supports_provider_driven_testing());
}
```

- [ ] **Step 2: 运行测试确认 RED**

```bash
cargo test --locked --lib supports_provider_driven_testing
```

Expected:

- 编译失败，提示 `supports_provider_driven_testing` 方法不存在；或测试失败，返回 `false`。

- [ ] **Step 3: 在 StreamingProviderAdapter 新增默认 capability**

在 `src/cross_cutting/streaming_provider.rs` 的 trait 中新增：

```rust
fn supports_provider_driven_testing(&self) -> bool {
    false
}
```

保留现有：

```rust
fn supports_tool_calls(&self) -> bool {
    false
}
```

语义区分：

- `supports_provider_driven_testing()`：Provider 能按 `plan_tests` / `execute_test_plan` 契约输出 TestPlan 和 step results。
- `supports_tool_calls()`：Provider 能输出 Aria 可接管执行的结构化 `ProviderEvent::ToolCall` / `ToolResult`。

- [ ] **Step 4: 给真实 Provider 打开 provider-driven testing**

在 `ClaudeCodeProvider` 的 `impl StreamingProviderAdapter` 中新增：

```rust
fn supports_provider_driven_testing(&self) -> bool {
    true
}
```

在 `CodexProvider` 的 `impl StreamingProviderAdapter` 中新增：

```rust
fn supports_provider_driven_testing(&self) -> bool {
    true
}
```

在 `TestControlledFakeStreamingProvider` 的 `impl StreamingProviderAdapter` 中新增：

```rust
fn supports_provider_driven_testing(&self) -> bool {
    true
}
```

- [ ] **Step 5: 运行测试确认 GREEN**

```bash
cargo test --locked --lib supports_provider_driven_testing
```

Expected:

- ClaudeCodeProvider 和 CodexProvider capability 测试通过。

## Task 2: Tester 入口改用新 Capability

**Files:**

- Modify: `src/product/coding_workspace_engine.rs`
- Test: `src/product/coding_workspace_engine.rs`

- [ ] **Step 1: 写失败测试，证明不支持 provider-driven testing 时必须 blocked，不得 legacy fallback**

在 `src/product/coding_workspace_engine.rs` 的 tests 中新增 fixture：

```rust
struct NonProviderDrivenTestingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for NonProviderDrivenTestingProvider {}
```

新增测试：

```rust
#[tokio::test]
async fn testing_without_provider_driven_capability_blocks_instead_of_legacy_commands() {
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let specs = vec![TestCommandSpec {
        id: "legacy_true".to_string(),
        command: vec!["true".to_string()],
    }];
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let context = CodingExecutionContext {
        issue_title: "issue".to_string(),
        issue_body: "body".to_string(),
        story_spec_markdown: Some("Story Spec".to_string()),
        design_spec_markdown: Some("Design Spec".to_string()),
        work_item_markdown: Some("Work Item".to_string()),
        verification_commands: Vec::new(),
    };

    let report = engine
        .execute_testing_with_provider(
            &attempt,
            &NonProviderDrivenTestingProvider,
            &context,
            &specs,
            TesterAgentOptions::default(),
        )
        .await
        .expect("blocked testing report");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(report.commands.is_empty());
    assert_eq!(report.plan_id, None);
    assert!(report.steps.is_empty());
    assert_eq!(report.raw_provider_output_ref, None);
    assert!(
        report
            .context_warnings
            .iter()
            .any(|warning| warning.contains("provider_driven_testing_not_supported"))
    );
}
```

- [ ] **Step 2: 运行测试确认 RED**

```bash
cargo test --locked --lib testing_without_provider_driven_capability_blocks_instead_of_legacy_commands
```

Expected:

- 当前会走 legacy fixed command，`true` 命令使 report 通过，测试失败。
- 失败点应证明 `commands` 非空或 `overall_status != Blocked`。

- [ ] **Step 3: 写失败测试，证明无 ToolCall 但有 step_results 的 Provider 应生成 plan-based report**

在 `src/product/coding_workspace_engine.rs` 的 tests 中新增一个 provider fixture：

```rust
struct ProviderDrivenTestingNoToolCallProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ProviderDrivenTestingNoToolCallProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(8);
        let (command_tx, _command_rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            let output = if input.prompt.contains("plan_tests") {
                serde_json::json!({
                    "summary": "provider planned tests",
                    "steps": [{
                        "id": "unit",
                        "title": "Unit tests",
                        "intent": "verify unit behavior",
                        "required": true,
                        "tool": "provider_managed",
                        "risk_level": "low",
                        "command_or_tool_input": {"command": ["cargo", "test", "--locked", "--lib", "some_filter"]},
                        "evidence_expectation": "provider supplies evidence"
                    }]
                })
                .to_string()
            } else {
                serde_json::json!({
                    "step_results": [{
                        "step_id": "unit",
                        "status": "passed",
                        "evidence_refs": ["provider-managed-unit.log"],
                        "provider_analysis": "unit evidence accepted"
                    }]
                })
                .to_string()
            };
            let _ = event_tx
                .send(ProviderEvent::TextDelta {
                    content: output.clone(),
                })
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id: None,
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}
```

新增测试：

```rust
#[tokio::test]
async fn real_provider_driven_testing_accepts_final_step_results_without_tool_calls() {
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let context = CodingExecutionContext {
        issue_title: "issue".to_string(),
        issue_body: "body".to_string(),
        story_spec_markdown: Some("Story Spec".to_string()),
        design_spec_markdown: Some("Design Spec".to_string()),
        work_item_markdown: Some("Work Item".to_string()),
        verification_commands: Vec::new(),
    };

    let report = engine
        .execute_testing_with_provider(
            &attempt,
            &ProviderDrivenTestingNoToolCallProvider,
            &context,
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("provider-driven testing");

    assert_eq!(report.overall_status, TestingOverallStatus::Passed);
    assert!(report.plan_id.is_some());
    assert_eq!(report.steps.len(), 1);
    assert_eq!(report.steps[0].step_id, "unit");
    assert_eq!(report.steps[0].evidence_refs, vec!["provider-managed-unit.log"]);
    assert!(report.commands.is_empty());
    assert!(report.raw_provider_output_ref.is_some());
}
```

- [ ] **Step 4: 运行测试确认 RED**

```bash
cargo test --locked --lib real_provider_driven_testing_accepts_final_step_results_without_tool_calls
```

Expected:

- 当前会走 legacy 或无法编译新 capability，测试失败。

- [ ] **Step 5: 修改 Tester 入口策略，移除 legacy fallback**

在 `src/product/coding_workspace_engine.rs` 中移除如下 fallback：

```rust
if !provider.supports_tool_calls() {
    return self.execute_testing(attempt, specs).await;
}
```

改成 provider-driven testing capability 检查；不支持时创建 blocked report/gate，不调用 `execute_testing`：

```rust
if !provider.supports_provider_driven_testing() {
    return self
        .block_provider_driven_testing(
            &attempt,
            &node,
            "provider_driven_testing_not_supported",
            "Tester provider does not support provider-driven testing",
            None,
        )
        .await;
}
```

同时关闭 Tester `plan_tests` 对旧 streaming API 的兼容 fallback。给 `CodingProviderStreamRun` 增加显式开关：

```rust
struct CodingProviderStreamRun<'a> {
    attempt: &'a CodingExecutionAttempt,
    node_id: &'a str,
    provider: &'a dyn StreamingProviderAdapter,
    legacy_input: &'a AdapterInput,
    input: StreamingProviderInput,
    provider_name: &'a ProviderName,
    provider_role: CodingProviderRole,
    command_rx: &'a mut mpsc::Receiver<CodingRunnerCommand>,
    allow_legacy_stream_fallback: bool,
}
```

在 `run_provider_stream_to_completion` 中只在开关为 `true` 时调用 `run_legacy_stream_to_completion`：

```rust
Err(error)
    if provider_start_is_not_implemented(&error) && allow_legacy_stream_fallback =>
{
    return self
        .run_legacy_stream_to_completion(attempt, node_id, provider, legacy_input)
        .await;
}
Err(error) => {
    return Err(CodingWorkspaceEngineError::ProviderStream(error.details));
}
```

其他角色沿用 `allow_legacy_stream_fallback: true` 以缩小本次改动范围；Tester 的 `plan_tests` 必须传 `false`：

```rust
let plan_output = match self
    .run_provider_stream_to_completion(CodingProviderStreamRun {
        attempt: &attempt,
        node_id: &node.id,
        provider,
        legacy_input: &plan_adapter_input,
        input: plan_input,
        provider_name: &tester_provider,
        provider_role: CodingProviderRole::Tester,
        command_rx,
        allow_legacy_stream_fallback: false,
    })
    .await
{
    Ok(output) => output,
    Err(error) => {
        return self
            .block_provider_driven_testing(
                &attempt,
                &node,
                "provider_start_failed",
                &format!("Tester provider failed during plan_tests: {error}"),
                None,
            )
            .await;
    }
};
```

将 Tester 局部变量 `plan_legacy_input` 重命名为 `plan_adapter_input`。保留 `CodingProviderStreamRun.legacy_input` 字段名只作为其他角色兼容路径的现状，不允许 Tester 使用该 fallback。

新增私有 helper，复用既有 report/gate 持久化与事件发送形态：

```rust
async fn block_provider_driven_testing(
    &self,
    attempt: &CodingExecutionAttempt,
    node: &CodingTimelineNode,
    reason_code: &str,
    description: &str,
    raw_provider_output_ref: Option<String>,
) -> Result<TestingReport, CodingWorkspaceEngineError> {
    let report_id = next_sequential_id(
        "testing_report",
        self.store
            .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .len(),
    );
    let mut report = build_testing_report(
        &attempt.id,
        Vec::new(),
        "",
        Some(description.to_string()),
    );
    report.id = report_id;
    report.overall_status = TestingOverallStatus::Blocked;
    report.raw_provider_output_ref = raw_provider_output_ref.clone();
    report.context_warnings.push(reason_code.to_string());
    self.store.save_testing_report(&report)?;
    let _ = self
        .event_tx
        .send(CodingWsOutMessage::TestingReportUpdate {
            report: Box::new(report.clone()),
        })
        .await;
    self.store.update_attempt_status(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        CodingAttemptStatus::Blocked,
    )?;
    self.complete_timeline_node(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        &node.id,
        CodingTimelineNodeStatus::Blocked,
        Some("测试被阻塞".to_string()),
    )
    .await?;
    let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
        attempt_id: attempt.id.clone(),
        stage: CodingExecutionStage::Testing,
        node_id: Some(node.id.clone()),
        role: Some(CodingProviderRole::Tester),
        title: "Testing blocked".to_string(),
        description: description.to_string(),
        reason_code: Some(reason_code.to_string()),
        evidence_refs: vec![format!("{}.json", report.id)],
        raw_provider_output_ref,
        available_actions: testing_blocked_gate_actions(),
    })?;
    let _ = self
        .event_tx
        .send(CodingWsOutMessage::CodingGateRequired { gate })
        .await;
    Ok(report)
}
```

注意：如果现有类型名或 helper 签名不同，沿用当前文件已有命名；但行为必须保持为 blocked report/gate，且不得调用固定命令执行器。

- [ ] **Step 6: 运行测试确认 GREEN**

```bash
cargo test --locked --lib testing_without_provider_driven_capability_blocks_instead_of_legacy_commands
cargo test --locked --lib real_provider_driven_testing_accepts_final_step_results_without_tool_calls
```

Expected:

- capability 不支持时 `overall_status == blocked`，`commands == []`，不会执行 `true`。
- 生成 plan-based report。
- `plan_id` 非空。
- `steps` 包含 `unit`。
- 不再走 legacy fixed commands。

## Task 3: 强化 execute_test_plan 契约与失败策略

**Files:**

- Modify: `src/product/coding_workspace_engine.rs`
- Test: `src/product/coding_workspace_engine.rs`

- [ ] **Step 1: 写失败测试，证明 Provider 启动失败时必须 blocked，不得向上冒泡为未恢复错误**

新增 provider fixture：

```rust
struct ProviderDrivenTestingStartFailsProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ProviderDrivenTestingStartFailsProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Err(ProviderAdapterError::command_missing(
            "tester provider command not found".to_string(),
        ))
    }
}
```

新增测试：

```rust
#[tokio::test]
async fn provider_driven_testing_blocks_when_provider_start_fails() {
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let context = CodingExecutionContext {
        issue_title: "issue".to_string(),
        issue_body: "body".to_string(),
        story_spec_markdown: Some("Story Spec".to_string()),
        design_spec_markdown: Some("Design Spec".to_string()),
        work_item_markdown: Some("Work Item".to_string()),
        verification_commands: Vec::new(),
    };

    let report = engine
        .execute_testing_with_provider(
            &attempt,
            &ProviderDrivenTestingStartFailsProvider,
            &context,
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("blocked testing report");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert!(report.commands.is_empty());
    assert!(
        report
            .context_warnings
            .iter()
            .any(|warning| warning.contains("provider_start_failed"))
    );
}
```

- [ ] **Step 2: 运行测试确认 RED**

```bash
cargo test --locked --lib provider_driven_testing_blocks_when_provider_start_fails
```

Expected:

- 当前 Provider 启动失败会通过 `?` 冒泡成 `Err`，测试失败。

- [ ] **Step 3: 捕获 plan_tests 和 execute_test_plan Provider 启动失败**

在 `plan_tests` 阶段包住 `run_provider_stream_to_completion`：

```rust
let plan_output = match self
    .run_provider_stream_to_completion(CodingProviderStreamRun {
        attempt: &attempt,
        node_id: &node.id,
        provider,
        legacy_input: &plan_adapter_input,
        input: plan_input,
        provider_name: &tester_provider,
        provider_role: CodingProviderRole::Tester,
        command_rx,
        allow_legacy_stream_fallback: false,
    })
    .await
{
    Ok(output) => output,
    Err(error) => {
        return self
            .block_provider_driven_testing(
                &attempt,
                &node,
                "provider_start_failed",
                &format!("Tester provider failed during plan_tests: {error}"),
                None,
            )
            .await;
    }
};
```

在 `execute_test_plan` 阶段包住 `provider.start(...)`；如果 plan 已保存，则生成 plan-based blocked report，保留 `plan_id` 和 required step 缺失信息：

```rust
let mut session = match provider.start(input, cancel.clone()).await {
    Ok(session) => session,
    Err(error) => {
        let report_id = next_sequential_id(
            "testing_report",
            self.store
                .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                .len(),
        );
        let mut report = build_plan_based_testing_report(
            &report_id,
            &attempt.id,
            &plan,
            Vec::new(),
            Vec::new(),
            None,
            None,
        );
        report.overall_status = TestingOverallStatus::Blocked;
        report
            .context_warnings
            .push(format!("provider_start_failed:{error}"));
        return self
            .save_blocked_testing_report_and_gate(
                &attempt,
                &node,
                report,
                "provider_start_failed",
                "Tester provider failed during execute_test_plan",
                None,
            )
            .await;
    }
};
```

`save_blocked_testing_report_and_gate` 返回保存后的 report：

```rust
async fn save_blocked_testing_report_and_gate(
    &self,
    attempt: &CodingExecutionAttempt,
    node: &CodingTimelineNode,
    report: TestingReport,
    reason_code: &str,
    description: &str,
    raw_provider_output_ref: Option<String>,
) -> Result<TestingReport, CodingWorkspaceEngineError> {
    self.store.save_testing_report(&report)?;
    let _ = self
        .event_tx
        .send(CodingWsOutMessage::TestingReportUpdate {
            report: Box::new(report.clone()),
        })
        .await;
    self.store.update_attempt_status(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        CodingAttemptStatus::Blocked,
    )?;
    self.complete_timeline_node(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        &node.id,
        CodingTimelineNodeStatus::Blocked,
        Some("测试被阻塞".to_string()),
    )
    .await?;
    let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
        attempt_id: attempt.id.clone(),
        stage: CodingExecutionStage::Testing,
        node_id: Some(node.id.clone()),
        role: Some(CodingProviderRole::Tester),
        title: "Testing blocked".to_string(),
        description: description.to_string(),
        reason_code: Some(reason_code.to_string()),
        evidence_refs: vec![format!("{}.json", report.id)],
        raw_provider_output_ref,
        available_actions: testing_blocked_gate_actions(),
    })?;
    let _ = self
        .event_tx
        .send(CodingWsOutMessage::CodingGateRequired { gate })
        .await;
    Ok(report)
}
```

`block_provider_driven_testing` 可用该 helper 组合出无 plan 的 blocked report：

```rust
async fn block_provider_driven_testing(
    &self,
    attempt: &CodingExecutionAttempt,
    node: &CodingTimelineNode,
    reason_code: &str,
    description: &str,
    raw_provider_output_ref: Option<String>,
) -> Result<TestingReport, CodingWorkspaceEngineError> {
    let report_id = next_sequential_id(
        "testing_report",
        self.store
            .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .len(),
    );
    let mut report = build_testing_report(
        &attempt.id,
        Vec::new(),
        "",
        Some(description.to_string()),
    );
    report.id = report_id;
    report.overall_status = TestingOverallStatus::Blocked;
    report.raw_provider_output_ref = raw_provider_output_ref.clone();
    report.context_warnings.push(reason_code.to_string());
    self.save_blocked_testing_report_and_gate(
        attempt,
        node,
        report,
        reason_code,
        description,
        raw_provider_output_ref,
    )
    .await
}
```

注意：如果实际实现中 helper 参数名不同，应以当前文件已有类型为准；但必须保持“保存 report、更新 attempt blocked、完成 timeline blocked、创建 gate、发送事件、返回 `Ok(report)`”这一组行为一致。

- [ ] **Step 4: 运行测试确认 GREEN**

```bash
cargo test --locked --lib provider_driven_testing_blocks_when_provider_start_fails
```

Expected:

- Provider 启动失败返回 `Ok(report)`。
- `overall_status == blocked`。
- attempt status 为 blocked。
- 页面可收到 blocked gate。

- [ ] **Step 5: 写失败测试，证明 execute 阶段缺 step_results 时必须 blocked**

新增 provider fixture：

```rust
struct ProviderDrivenTestingMissingStepResultsProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for ProviderDrivenTestingMissingStepResultsProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(8);
        let (command_tx, _command_rx) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            let output = if input.prompt.contains("plan_tests") {
                serde_json::json!({
                    "summary": "provider planned tests",
                    "steps": [{
                        "id": "unit",
                        "title": "Unit tests",
                        "intent": "verify unit behavior",
                        "required": true,
                        "tool": "provider_managed",
                        "risk_level": "low",
                        "command_or_tool_input": {"command": ["cargo", "test"]},
                        "evidence_expectation": "provider supplies evidence"
                    }]
                })
                .to_string()
            } else {
                "I ran the tests and they passed.".to_string()
            };
            let _ = event_tx
                .send(ProviderEvent::Completed {
                    full_output: output,
                    provider_session_id: None,
                })
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}
```

新增测试：

```rust
#[tokio::test]
async fn provider_driven_testing_blocks_when_execute_output_has_no_step_results() {
    let root = tempfile::tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            base_branch: "HEAD".to_string(),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let (tx, _rx) = tokio::sync::mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), tx);
    let context = CodingExecutionContext {
        issue_title: "issue".to_string(),
        issue_body: "body".to_string(),
        story_spec_markdown: Some("Story Spec".to_string()),
        design_spec_markdown: Some("Design Spec".to_string()),
        work_item_markdown: Some("Work Item".to_string()),
        verification_commands: Vec::new(),
    };

    let report = engine
        .execute_testing_with_provider(
            &attempt,
            &ProviderDrivenTestingMissingStepResultsProvider,
            &context,
            &[],
            TesterAgentOptions::default(),
        )
        .await
        .expect("provider-driven testing");

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert_eq!(report.missing_required_steps, vec!["unit"]);
    assert!(report.raw_provider_output_ref.is_some());
}
```

- [ ] **Step 6: 运行测试确认 RED**

```bash
cargo test --locked --lib provider_driven_testing_blocks_when_execute_output_has_no_step_results
```

Expected:

- 如果当前没有 blocked 语义或缺少 missing required step，测试失败。

- [ ] **Step 7: 移除 legacy execute prompt context，并强化 execute JSON 契约**

删除 Tester execute 阶段的旧 prompt 注入：

```rust
let legacy_execute_prompt = build_tester_system_prompt(&attempt, context, specs);
let prompt = build_tester_execute_plan_prompt(
    &attempt,
    &plan,
    &evaluation_context_json,
    &legacy_execute_prompt,
);
```

改为只传 TestPlan 与 EvaluationContextPack：

```rust
let prompt = build_tester_execute_plan_prompt(&attempt, &plan, &evaluation_context_json);
```

同步修改函数签名，移除 `legacy_prompt_context` 参数和输出中的 `Legacy tester context for command discovery`：

```rust
fn build_tester_execute_plan_prompt(
    attempt: &CodingExecutionAttempt,
    plan: &TestPlan,
    evaluation_context_json: &str,
) -> String {
    let plan_json = serde_json::to_string_pretty(plan).unwrap_or_else(|_| "{}".to_string());
    format!(
        "Tester Provider Runtime\n\
         Phase: execute_test_plan\n\
         Attempt: {}\n\
         Work Item: {}\n\
         \n\
         Execute the following TestPlan. You may execute commands or inspect files yourself.\n\
         Every required TestPlan step must have exactly one corresponding step_results item.\n\
         If you cannot run a required step, emit status=\"blocked\" or status=\"skipped\" with provider_analysis explaining why.\n\
         Do not claim overall success in prose without step_results JSON.\n\
         At the end of execute_test_plan, output a JSON object with:\n\
         {{\"step_results\":[{{\"step_id\":\"...\",\"status\":\"passed|failed|blocked|skipped\",\"evidence_refs\":[\"...\"],\"provider_analysis\":\"...\"}}]}}\n\
         \n\
         TestPlan:\n```json\n{}\n```\n\
         \n\
         Evaluation Context JSON:\n```json\n{}\n```\n",
        attempt.id, attempt.work_item_id, plan_json, evaluation_context_json
    )
}
```

最终 prompt 必须包含明确契约：

```text
You may execute commands or inspect files yourself.
At the end of execute_test_plan, output a JSON object with:
{"step_results":[{"step_id":"...","status":"passed|failed|blocked|skipped","evidence_refs":["..."],"provider_analysis":"..."}]}
Every required TestPlan step must have exactly one corresponding step_results item.
If you cannot run a required step, emit status="blocked" or status="skipped" with provider_analysis explaining why.
Do not claim overall success in prose without step_results JSON.
```

- [ ] **Step 8: 确保缺失 required step 进入 blocked report/gate**

复用已有 `build_plan_based_testing_report` 和 blocked gate 创建逻辑，不新增额外状态机。确保 `step_results` 为空且 plan 有 required step 时：

- `overall_status == blocked`
- `missing_required_steps` 包含 required step id
- `raw_provider_output_ref` 指向 execute raw output
- 创建 `missing_required_steps` blocked gate

- [ ] **Step 9: 运行测试确认 GREEN**

```bash
cargo test --locked --lib provider_driven_testing_blocks_when_provider_start_fails
cargo test --locked --lib provider_driven_testing_blocks_when_execute_output_has_no_step_results
```

Expected:

- 测试通过。

## Task 4: Tester 气泡持久化与前端展示

**Files:**

- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Test: `web/src/pages/CodingWorkspacePage.test.tsx`

- [ ] **Step 1: 写前端失败测试，证明 Tester chat entry 应显示气泡**

在 `web/src/pages/CodingWorkspacePage.test.tsx` 新增测试，构造 `chat_entries`：

```ts
it("renders tester assistant chat entries as bubbles", () => {
  const state = codingSessionStateFixture({
    chat_entries: [
      {
        id: "tester_entry_0001",
        attempt_id: "coding_attempt_0001",
        node_id: "coding_node_0003",
        role: "tester",
        entry_type: { type: "assistant_message" },
        content: "TestPlan: unit checks",
        metadata: {
          phase: "plan_tests",
          test_plan_id: "test_plan_0001",
        },
        created_at: "2026-06-10T00:00:00Z",
      },
    ],
  });

  renderCodingWorkspacePageWithState(state);

  expect(screen.getByText("TestPlan: unit checks")).toBeInTheDocument();
  expect(screen.getByText(/Tester|测试/)).toBeInTheDocument();
});
```

- [ ] **Step 2: 运行前端测试确认 RED**

```bash
pnpm -C web test -- CodingWorkspacePage.test.tsx
```

Expected:

- 当前如果 tester role 没被渲染，测试失败。

- [ ] **Step 3: 后端保存 Tester plan/execute chat entry**

在 `execute_testing_with_provider_commands` 中：

1. `plan_tests` 完成并保存 raw output / test plan 后，保存 chat entry：

```rust
let entry = tester_chat_entry(
    &attempt,
    &node.id,
    &mut chat_entry_sequence,
    CodingEntryType::AssistantMessage,
    Some(plan_output.clone()),
    Some(serde_json::json!({
        "phase": "plan_tests",
        "test_plan_id": plan.id,
        "raw_provider_output_ref": plan.raw_provider_output_ref
    })),
);
self.save_and_emit_chat_entry(entry).await;
```

2. `execute_test_plan` 完成并保存 report 后，保存 chat entry：

```rust
let entry = tester_chat_entry(
    &attempt,
    &node.id,
    &mut chat_entry_sequence,
    CodingEntryType::AssistantMessage,
    Some(full_output.clone()),
    Some(serde_json::json!({
        "phase": "execute_test_plan",
        "testing_report_id": report.id,
        "raw_provider_output_ref": report.raw_provider_output_ref
    })),
);
self.save_and_emit_chat_entry(entry).await;
```

注意：如果当前 `tester_chat_entry` 签名不同，应沿用本文件已有 helper 形态，不新增重复 helper。

- [ ] **Step 4: 前端补 Tester role 展示**

如果 `CodingWorkspacePage.tsx` 的 role label/color 映射缺 Tester，补充：

```ts
tester: {
  label: "Tester",
  tone: "emerald",
}
```

若已有共享 role formatter，则在共享 formatter 中补，不在页面局部重复定义。

- [ ] **Step 5: 运行前端测试确认 GREEN**

```bash
pnpm -C web test -- CodingWorkspacePage.test.tsx
```

Expected:

- Tester chat entry 被渲染。

## Task 5: Controlled E2E 回归

**Files:**

- Modify: `cadence/reports/2026-06-10_进度报告_CodingWorkspaceProvider驱动测试审查与恢复机制验证_v1.0.md`

- [ ] **Step 1: 启动 test controls 服务**

```bash
ARIA_E2E_TEST_CONTROLS=1 cargo watch -w src -w Cargo.toml -w Cargo.lock -x "run --locked -- web --workspace . --host 127.0.0.1 --port 4317"
```

前端：

```bash
pnpm -C web dev --port 5173
```

- [ ] **Step 2: 跑 health check**

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS -I http://127.0.0.1:5173/
curl --noproxy '*' -sS http://127.0.0.1:5173/api/health
```

Expected:

- 后端和前端代理均返回 `{"status":"ok"}`。
- 前端 `/` 返回 `200 OK`。

- [ ] **Step 3: Controlled happy path**

创建新 attempt，注入 fixture：

```json
{
  "plan_output": {
    "summary": "controlled unit and API smoke",
    "steps": [
      {
        "id": "unit",
        "title": "Unit tests",
        "intent": "prove unit behavior",
        "required": true,
        "tool": "run_command",
        "risk_level": "low",
        "command_or_tool_input": {"command": ["true"]},
        "evidence_expectation": "exit 0"
      },
      {
        "id": "api_smoke",
        "title": "API smoke",
        "intent": "prove API health",
        "required": true,
        "tool": "run_command",
        "risk_level": "low",
        "command_or_tool_input": {"command": ["true"]},
        "evidence_expectation": "exit 0"
      }
    ]
  },
  "step_results": [
    {"step_id": "unit", "status": "passed", "evidence_refs": ["unit.stdout.log"], "provider_analysis": "unit ok"},
    {"step_id": "api_smoke", "status": "passed", "evidence_refs": ["api.stdout.log"], "provider_analysis": "api ok"}
  ]
}
```

Expected:

- `overall_status == "passed"`
- `plan_id != null`
- `steps` 包含 `unit`、`api_smoke`
- 页面显示 Tester 气泡。

- [ ] **Step 4: Controlled missing step blocked**

注入 plan steps `unit` + required `security`，只返回 `unit`。

Expected:

- `overall_status == "blocked"`
- `missing_required_steps == ["security"]`
- blocked gate reason 为 `missing_required_steps`
- 页面显示 gate actions。

## Task 6: 真实 Provider E2E 验收

**Files:**

- Modify: `cadence/reports/2026-06-10_进度报告_CodingWorkspaceProvider驱动测试审查与恢复机制验证_v1.0.md`

- [ ] **Step 1: 创建新的真实 coding attempt**

不要复用已经 legacy passed 的旧 attempt。使用当前页面重新创建新的 Coding attempt。

Expected:

- role provider config 中 `tester` 为 `claude_code` 或 `codex`。
- attempt 从 `prepare_context` 进入 `coding`。

- [ ] **Step 2: 进入 Testing 后检查持久化产物**

检查新 attempt 目录：

```bash
find .aria/projects -path '*coding-attempts/<attempt_id>/provider-raw/testing/*' -type f -print
```

Expected:

- 存在 `provider-raw/testing/plan_tests_0001.txt`
- 存在 `provider-raw/testing/execute_test_plan_0001.txt`

- [ ] **Step 3: 检查 TestingReport**

读取：

```bash
sed -n '1,180p' .aria/projects/<project_id>/issues/<issue_id>/coding-attempts/<attempt_id>/testing-reports/testing_report_0001.json
```

Expected:

- `plan_id` 非空。
- `steps` 非空。
- `raw_provider_output_ref` 非空。
- 如果 Provider 未能提供 required step evidence，则 `overall_status=blocked`，不得 legacy passed。

- [ ] **Step 4: 检查页面**

在 Coding Workspace 页面确认：

- Tester 节点有气泡。
- 气泡中能看到 TestPlan 或 execute summary。
- TestingReport 区域显示 plan summary、step evidence、missing/skipped/context warnings。

## Task 7: 最终验证

- [ ] **Step 1: Rust fmt**

```bash
cargo fmt --check
```

Expected: 通过。

- [ ] **Step 2: Rust clippy**

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: 通过。

- [ ] **Step 3: Rust check**

```bash
cargo check --locked
```

Expected: 通过。

- [ ] **Step 4: Rust tests**

```bash
cargo test --locked
```

Expected: 通过。

- [ ] **Step 5: Frontend tests**

```bash
pnpm -C web test
```

Expected: 通过。

- [ ] **Step 6: Frontend build**

```bash
pnpm -C web build
```

Expected: 通过；如仅有 Vite chunk size warning，可记录但不视为失败。

- [ ] **Step 7: Tester legacy path scan**

```bash
if rg -n "return self\\.execute_testing\\(|legacy_execute_prompt|plan_legacy_input|Legacy tester context for command discovery" src/product/coding_workspace_engine.rs; then exit 1; fi
rg -n "allow_legacy_stream_fallback: false" src/product/coding_workspace_engine.rs
```

Expected:

- 第一条命令无输出且退出 0。
- 第二条命令至少命中 Tester `plan_tests` 的 `CodingProviderStreamRun` 构造。

- [ ] **Step 8: Diff check**

```bash
git diff --check
```

Expected: 无输出。

## 验收标准

- 真实 Provider Tester 不再静默走 legacy fixed commands。
- `execute_testing_with_provider_commands` 不再通过 `return self.execute_testing(...)` fallback。
- Tester `plan_tests` 不再允许 `run_legacy_stream_to_completion` fallback；`allow_legacy_stream_fallback` 必须为 `false`。
- Tester execute prompt 不再包含 `legacy_execute_prompt` / `Legacy tester context for command discovery`。
- 新 attempt 的 `TestingReport.plan_id` 非空。
- 新 attempt 的 `TestingReport.steps` 非空。
- `provider-raw/testing/plan_tests_*.txt` 和 `provider-raw/testing/execute_test_plan_*.txt` 落盘。
- 页面显示 Tester 气泡。
- provider-driven testing capability 不支持时，`overall_status=blocked`、`commands=[]`，创建 blocked gate，不能执行固定命令。
- Provider 启动失败时，`overall_status=blocked`，创建 blocked gate，不能向上冒泡为未恢复错误。
- Provider 输出不满足 step evidence 契约时进入 blocked gate，不伪装 passed。
- Provider-driven Tester 不得再生成 `overall_status=passed && plan_id=null && steps=[] && raw_provider_output_ref=null` 的 report。
- CodeReviewer 阶段流程顺序不变。
