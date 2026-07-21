# Coding Workspace Analyst 重跑闭环实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 P1 已完成的 Tester role run 基础上，让 Analyst 的 human gate 可以一键重跑，并且重跑后的 Analyst 计划、执行、结果、证据和历史 run 都能恢复与追踪。

**Architecture:** 复用当前 `CodingRoleRun` store 和 `CodingRoleRunTrigger::RetryAnalyst`，补齐 `retry_analyst` gate action、Analyst role run 绑定、evidence snapshot 持久化，以及 blocked gate 回到 Rework 后的 runner 恢复路径。Provider 仍只负责输出 JSON；后端负责把 Analyst 输入证据、原始输出、决策记录、chat entry 和 role run 状态串起来。

**Tech Stack:** Rust 1.95、serde JSON store、Tokio async、Axum WebSocket、React、Zustand、Vitest。

---

## 当前基线

本计划基于 `bugfix_branch` 当前提交 `1bad0be`，也就是 P1 已完成后的代码。

已存在能力：

- `CodingRoleRun`、`CodingRoleRunStatus`、`CodingRoleRunTrigger`、`CodingProviderRole` 已存在。
- `CodingRoleRunTrigger` 已包含 `RetryAnalyst`。
- Store 已支持 `create_role_run`、`latest_role_run`、`attach_role_run_node`、`update_role_run_status`、`supersede_latest_role_run_and_create`。
- Tester 已有 role run、blocked gate retry 和 readable chat。
- `execute_rework_with_commands` 已负责 Analyst prompt、provider run、raw output 保存、JSON parse、`AnalystDecisionRecord` 保存、Analyst verdict chat entry、human gate。

当前缺口：

- `CodingGateActionType` 与前端 gate action type 还没有 `retry_analyst`。
- `coding_gate_action_for_id` 不能把 Analyst 推荐的 `retry_analyst` 渲染成可点击动作。
- `AnalystDecisionRecord` 没有 `role_run_id/run_no`。
- Analyst 执行没有创建/绑定/完成/阻塞 `CodingRoleRun`。
- Analyst 输入 evidence 没有持久化到 role run，blocked gate 后无法可靠重跑同一份 evidence。
- `should_resume_runner_after_gate_response` 不包含 `retry_analyst`。
- `execute_start_coding_flow` 没有“当前 stage 已是 Rework，但没有内存中的 testing/review report”时的恢复路径。

## Design Readiness Review

当前 design 符合实施落地条件。

- 改动集中在 Coding Workspace 专属链路，不触发 Story/Design/Work Item 三模块共享 workspace 规则。
- P1 已提供 role run 模型和前端 snapshot 基础，本计划只补 Analyst 角色闭环。
- 计划内所有行为都能用现有 fake/test provider、store fixture 和 WebSocket integration test 验证。
- 单 session 可完成：后端 contract + store helper + engine 绑定 + WS 恢复 + 少量前端 type/UI 测试。

不做范围：

- 不实现 Code Reviewer/Internal Reviewer role run，P3 处理。
- 不实现历史 run UI，P4 处理。
- 不改变 Analyst provider JSON schema，只新增后端持久化和 gate action。

## File Structure

- Modify: `src/product/coding_models.rs`
  - `CodingGateActionType` 增加 `RetryAnalyst`。
  - `AnalystDecisionRecord` 增加可选 `role_run_id`、`run_no`。

- Modify: `src/product/coding_attempt_store.rs`
  - 增加 `update_role_run_refs`，追加并去重 `raw_provider_output_refs` 和 `artifact_refs`。
  - 增加 `read_attempt_artifact_text(path: &str) -> Result<String, ProductStoreError>`，用于 runner 读取 Analyst evidence snapshot（artifact refs 中包含 `analyst_evidence` 的文件）。

- Modify: `src/product/coding_workspace_engine.rs`
  - `coding_gate_action_for_id` 支持 `retry_analyst`。
  - `analyst_human_gate_actions` 默认包含 `retry_analyst`。
  - `execute_rework_with_commands` 创建/复用 Analyst role run。
  - 保存 Analyst evidence snapshot，并把 evidence/raw output 追加到 role run refs。
  - `AnalystDecisionRecord` 和 Analyst verdict chat entry metadata 绑定 `role_run_id/run_no`。
  - `handle_blocked_gate_response` 处理 `RetryAnalyst`，创建新的 running Analyst role run 并复用上一轮 evidence refs。

- Modify: `src/web/coding_ws_handler.rs`
  - `should_resume_runner_after_gate_response` 支持 `retry_analyst`。
  - `execute_start_coding_flow` 支持从 `CodingExecutionStage::Rework` 直接读取最新 Analyst role run evidence 并重跑 Analyst。

- Modify: `web/src/api/types.ts`
  - `CodingGateActionType` 增加 `"retry_analyst"`。
  - `AnalystDecisionRecord` 增加 `role_run_id?: string | null`、`run_no?: number | null`。

- Tests:
  - `tests/it_product/product_coding_attempt_store.rs`
  - `tests/it_product/product_coding_models.rs`
  - `tests/it_product/product_coding_workspace_engine.rs`
  - `tests/it_web/web_coding_ws_handler.rs`
  - `web/src/api/types.test.ts`
  - `web/src/pages/CodingWorkspacePage.test.tsx`

## Task 1: Analyst Gate Contract

**Files:**
- Modify: `src/product/coding_models.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `web/src/api/types.ts`
- Test: `tests/it_product/product_coding_models.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`
- Test: `web/src/api/types.test.ts`

- [ ] **Step 1: RED - backend enum round trip**

Add to `tests/it_product/product_coding_models.rs`:

```rust
#[test]
fn coding_gate_action_type_round_trips_retry_analyst() {
    let action = CodingGateAction {
        action_id: "retry_analyst".to_string(),
        label: "重试 Analyst".to_string(),
        action_type: CodingGateActionType::RetryAnalyst,
    };

    let value = serde_json::to_value(&action).expect("serialize action");
    assert_eq!(value["action_type"], "retry_analyst");
    let decoded: CodingGateAction = serde_json::from_value(value).expect("decode action");
    assert_eq!(decoded.action_type, CodingGateActionType::RetryAnalyst);
}
```

Run:

```bash
cargo test --locked --test it_product coding_gate_action_type_round_trips_retry_analyst
```

Expected: compile failure because `CodingGateActionType::RetryAnalyst` is missing.

- [ ] **Step 2: GREEN - add backend action type and mapper**

In `src/product/coding_models.rs`, add `RetryAnalyst` to `CodingGateActionType`.

In `src/product/coding_workspace_engine.rs`, extend `coding_gate_action_for_id`:

```rust
"retry_analyst" => Some(CodingGateAction {
    action_id: "retry_analyst".to_string(),
    label: "重试 Analyst".to_string(),
    action_type: CodingGateActionType::RetryAnalyst,
}),
```

Update `analyst_human_gate_actions` fallback to use:

```rust
actions.push(coding_gate_action_for_id("retry_analyst").expect("retry analyst action"));
actions.push(coding_gate_action_for_id("provide_context").expect("provide context action"));
actions.push(coding_gate_action_for_id("manual_continue").expect("manual continue action"));
actions.push(coding_gate_action_for_id("abort").expect("abort action"));
```

- [ ] **Step 3: RED/GREEN - Analyst gate exposes retry action**

Add to `tests/it_product/product_coding_workspace_engine.rs`:

```rust
#[tokio::test]
async fn analyst_human_gate_offers_retry_analyst_action() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status("project_0001", "issue_0001", &attempt.id, CodingAttemptStatus::Running)
        .expect("running");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: "Analyst prose without JSON".to_string(),
    };

    engine
        .execute_rework(&attempt, "testing blocked evidence", &provider)
        .await
        .expect("execute analyst");

    let gates = store
        .list_open_blocked_gates("project_0001", "issue_0001", &attempt.id)
        .expect("gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].role, Some(CodingProviderRole::Analyst));
    assert!(gates[0].available_actions.iter().any(|action| {
        action.action_id == "retry_analyst"
            && action.action_type == CodingGateActionType::RetryAnalyst
    }));
}
```

Run:

```bash
cargo test --locked --test it_product analyst_human_gate_offers_retry_analyst_action
```

Expected: PASS after Step 2.

- [ ] **Step 4: frontend type coverage**

In `web/src/api/types.ts`, add `"retry_analyst"` to `CodingGateActionType`.

Add to `web/src/api/types.test.ts`:

```ts
it("accepts retry analyst gate actions", () => {
  const action: CodingGateAction = {
    action_id: "retry_analyst",
    label: "重试 Analyst",
    action_type: "retry_analyst",
  };

  expect(action.action_type).toBe("retry_analyst");
});
```

Run:

```bash
pnpm -C web exec vitest --run src/api/types.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/product/coding_models.rs src/product/coding_workspace_engine.rs web/src/api/types.ts web/src/api/types.test.ts tests/it_product/product_coding_models.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: add analyst retry gate action"
```

## Task 2: Store RoleRun Refs And Analyst Metadata

**Files:**
- Modify: `src/product/coding_models.rs`
- Modify: `src/product/coding_attempt_store.rs`
- Modify: `web/src/api/types.ts`
- Test: `tests/it_product/product_coding_attempt_store.rs`
- Test: `tests/it_product/product_coding_models.rs`
- Test: `web/src/api/types.test.ts`

- [ ] **Step 1: RED - store appends role run refs idempotently**

Add to `tests/it_product/product_coding_attempt_store.rs`:

```rust
#[test]
fn updates_coding_role_run_refs_without_duplicates() {
    let root = tempfile::tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: None,
            ..create_input()
        })
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Rework,
            CodingProviderRole::Analyst,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .expect("role run");

    let updated = store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            vec!["provider-raw/rework/analyst_decision_0001.txt".to_string()],
            vec!["provider-raw/rework/analyst_evidence_0001.txt".to_string()],
        )
        .expect("update refs");
    assert_eq!(updated.raw_provider_output_refs.len(), 1);
    assert_eq!(updated.artifact_refs.len(), 1);

    let updated = store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            vec!["provider-raw/rework/analyst_decision_0001.txt".to_string()],
            vec!["provider-raw/rework/analyst_evidence_0001.txt".to_string()],
        )
        .expect("update refs again");
    assert_eq!(updated.raw_provider_output_refs.len(), 1);
    assert_eq!(updated.artifact_refs.len(), 1);
}
```

Run:

```bash
cargo test --locked --test it_product updates_coding_role_run_refs_without_duplicates
```

Expected: compile failure because `update_role_run_refs` is missing.

- [ ] **Step 2: GREEN - implement `update_role_run_refs`**

In `src/product/coding_attempt_store.rs`, add a public method that:

```rust
let mut run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
for reference in raw_provider_output_refs {
    validate_relative_artifact_ref(&reference)?;
    if !run.raw_provider_output_refs.iter().any(|existing| existing == &reference) {
        run.raw_provider_output_refs.push(reference);
    }
}
for reference in artifact_refs {
    validate_relative_artifact_ref(&reference)?;
    if !run.artifact_refs.iter().any(|existing| existing == &reference) {
        run.artifact_refs.push(reference);
    }
}
self.save_role_run(project_id, issue_id, &run)?;
Ok(run)
```

Add `validate_relative_artifact_ref` beside the existing validators if no equivalent helper exists. It must reject empty refs, absolute paths, `..`, and Windows drive-prefix paths.

- [ ] **Step 3: RED - AnalystDecisionRecord metadata round trip**

Add to `tests/it_product/product_coding_models.rs`:

```rust
#[test]
fn analyst_decision_round_trips_role_run_metadata() {
    let decision = AnalystDecisionRecord {
        id: "analyst_decision_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        source_stage: CodingExecutionStage::Testing,
        rework_round: 1,
        verdict: AnalystDecisionVerdict::HumanRequired,
        next_stage: AnalystDecisionNextStage::HumanGate,
        reason: "Analyst 输出不是有效 JSON".to_string(),
        evidence_refs: vec!["testing_report_0001.json".to_string()],
        raw_provider_output_refs: vec!["provider-raw/rework/analyst_decision_0001.txt".to_string()],
        rework_instructions: None,
        human_gate: None,
        created_at: "2026-06-13T00:00:00Z".to_string(),
        parse_error: Some("expected JSON".to_string()),
        role_run_id: Some("coding_role_run_0001".to_string()),
        run_no: Some(1),
    };

    let value = serde_json::to_value(&decision).expect("serialize decision");
    assert_eq!(value["role_run_id"], "coding_role_run_0001");
    assert_eq!(value["run_no"], 1);
    let decoded: AnalystDecisionRecord = serde_json::from_value(value).expect("decode decision");
    assert_eq!(decoded.role_run_id.as_deref(), Some("coding_role_run_0001"));
    assert_eq!(decoded.run_no, Some(1));
}
```

Run:

```bash
cargo test --locked --test it_product analyst_decision_round_trips_role_run_metadata
```

Expected: compile failure because the fields are missing.

- [ ] **Step 4: GREEN - add optional metadata fields**

In `src/product/coding_models.rs`, add to `AnalystDecisionRecord`:

```rust
#[serde(default)]
pub role_run_id: Option<String>,
#[serde(default)]
pub run_no: Option<u32>,
```

Update existing Rust fixtures constructing `AnalystDecisionRecord` with `role_run_id: None` and `run_no: None`.

In `web/src/api/types.ts`, add to `AnalystDecisionRecord`:

```ts
role_run_id?: string | null;
run_no?: number | null;
```

Add to `web/src/api/types.test.ts`:

```ts
it("accepts role run metadata on analyst decisions", () => {
  const decision: AnalystDecisionRecord = {
    id: "analyst_decision_0001",
    attempt_id: "coding_attempt_0001",
    source_stage: "testing",
    rework_round: 1,
    verdict: "human_required",
    next_stage: "human_gate",
    reason: "Analyst 输出不是有效 JSON",
    evidence_refs: [],
    raw_provider_output_refs: [],
    created_at: "2026-06-13T00:00:00Z",
    role_run_id: "coding_role_run_0001",
    run_no: 1,
  };

  expect(decision.role_run_id).toBe("coding_role_run_0001");
  expect(decision.run_no).toBe(1);
});
```

- [ ] **Step 5: Run focused tests**

```bash
cargo test --locked --test it_product updates_coding_role_run_refs_without_duplicates
cargo test --locked --test it_product analyst_decision_round_trips_role_run_metadata
pnpm -C web exec vitest --run src/api/types.test.ts
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/product/coding_models.rs src/product/coding_attempt_store.rs web/src/api/types.ts web/src/api/types.test.ts tests/it_product/product_coding_attempt_store.rs tests/it_product/product_coding_models.rs
git commit -m "feat: persist analyst role run metadata"
```

## Task 3: Bind Analyst Execution To RoleRun

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`
- Test: `web/src/pages/CodingWorkspacePage.test.tsx`

- [ ] **Step 1: RED - Analyst parse failure creates blocked role run**

Add to `tests/it_product/product_coding_workspace_engine.rs`:

```rust
#[tokio::test]
async fn execute_rework_binds_analyst_decision_chat_and_gate_to_role_run() {
    let root = tempdir().expect("root");
    let worktree = root.path().join("worktree");
    init_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: Some(worktree),
            ..create_input()
        })
        .expect("create attempt");
    store
        .update_attempt_status("project_0001", "issue_0001", &attempt.id, CodingAttemptStatus::Running)
        .expect("running");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = InputCapturingProvider {
        input: Arc::new(Mutex::new(None)),
        output: "not json".to_string(),
    };

    engine
        .execute_rework(&attempt, "testing blocked evidence", &provider)
        .await
        .expect("execute analyst");

    let runs = store
        .list_role_runs("project_0001", "issue_0001", &attempt.id)
        .expect("role runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].stage, CodingExecutionStage::Rework);
    assert_eq!(runs[0].role, CodingProviderRole::Analyst);
    assert_eq!(runs[0].status, CodingRoleRunStatus::Blocked);
    assert!(runs[0].raw_provider_output_refs.iter().any(|value| value.contains("analyst_decision")));
    assert!(runs[0].artifact_refs.iter().any(|value| value.contains("analyst_evidence")));

    let decision = store
        .latest_analyst_decision("project_0001", "issue_0001", &attempt.id)
        .expect("latest decision")
        .expect("decision");
    assert_eq!(decision.role_run_id.as_deref(), Some(runs[0].id.as_str()));
    assert_eq!(decision.run_no, Some(1));

    let entries = store
        .list_chat_entries("project_0001", "issue_0001", &attempt.id)
        .expect("chat entries");
    assert!(entries.iter().any(|entry| {
        entry.metadata.as_ref().is_some_and(|metadata| {
            metadata.get("role_run_id").and_then(|value| value.as_str()) == Some(runs[0].id.as_str())
                && metadata.get("run_no").and_then(|value| value.as_u64()) == Some(1)
        })
    }));
}
```

Run:

```bash
cargo test --locked --test it_product execute_rework_binds_analyst_decision_chat_and_gate_to_role_run
```

Expected: FAIL because Analyst does not create role run or bind metadata.

- [ ] **Step 2: GREEN - create/attach Analyst role run**

In `execute_rework_with_commands`, immediately after creating the Rework timeline node, create or attach the Analyst role run:

```rust
let role_run = match self.store.latest_role_run(
    &attempt.project_id,
    &attempt.issue_id,
    &attempt.id,
    CodingExecutionStage::Rework,
    CodingProviderRole::Analyst,
)? {
    Some(run) if run.status == CodingRoleRunStatus::Running && run.node_id.is_none() => {
        self.store.attach_role_run_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &run.id,
            node.id.clone(),
        )?
    }
    _ => self.store.create_role_run(
        &attempt,
        CodingExecutionStage::Rework,
        CodingProviderRole::Analyst,
        CodingRoleRunTrigger::Initial,
        Some(node.id.clone()),
    )?,
};
```

Persist `evidence` with kind `analyst_evidence`, append that ref to `artifact_refs`, append `analyst_decision` raw output to `raw_provider_output_refs`, set `role_run_id/run_no` on `AnalystDecisionRecord`, and include them in Analyst chat metadata.

After `apply_analyst_decision`, update the role run:

```rust
let role_run_status = if node_status == CodingTimelineNodeStatus::Blocked {
    CodingRoleRunStatus::Blocked
} else {
    CodingRoleRunStatus::Completed
};
self.store.update_role_run_status(
    &attempt.project_id,
    &attempt.issue_id,
    &attempt.id,
    &role_run.id,
    role_run_status,
    decision.parse_error.clone().or_else(|| {
        if node_status == CodingTimelineNodeStatus::Blocked {
            Some("analyst_human_gate".to_string())
        } else {
            None
        }
    }),
)?;
```

- [ ] **Step 3: frontend smoke coverage for Analyst run metadata**

Add to `web/src/pages/CodingWorkspacePage.test.tsx`:

```tsx
it("renders analyst chat with role run metadata present", () => {
  mockCodingWs();
  useCodingWorkspaceStore.setState({
    attemptId: "coding_attempt_0001",
    status: "blocked",
    stage: "rework",
    chatEntries: [
      {
        id: "coding_node_0004_analyst_verdict",
        type: "analyst_verdict",
        role: "analyst",
        content: "Analyst 输出不是有效 JSON，已转人工确认。",
        timestamp: "2026-06-13T00:00:01Z",
        node_id: "coding_node_0004",
        metadata: {
          role_run_id: "coding_role_run_0001",
          run_no: 1,
          reason: "Analyst 输出不是有效 JSON，已转人工确认。",
        },
      },
    ],
  });

  render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

  const chatList = screen.getByTestId("chat-entry-list");
  expect(chatList).toHaveTextContent("Analyst");
  expect(chatList).toHaveTextContent("Analyst 输出不是有效 JSON");
});
```

P4 会继续把 `run_no` 做成可见 badge。

- [ ] **Step 4: Run focused tests**

```bash
cargo test --locked --test it_product execute_rework_binds_analyst_decision_chat_and_gate_to_role_run
pnpm -C web exec vitest --run src/pages/CodingWorkspacePage.test.tsx
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/product/coding_workspace_engine.rs tests/it_product/product_coding_workspace_engine.rs web/src/pages/CodingWorkspacePage.test.tsx
git commit -m "feat: bind analyst execution to role runs"
```

## Task 4: Retry Analyst From Blocked Gate

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/web/coding_ws_handler.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`
- Test: `tests/it_web/web_coding_ws_handler.rs`

- [ ] **Step 1: RED - gate response supersedes Analyst role run**

Add `retry_analyst_gate_response_supersedes_latest_analyst_run` to `tests/it_product/product_coding_workspace_engine.rs`. The fixture must:

- create a blocked attempt at `CodingExecutionStage::Rework`;
- create one blocked Analyst role run with `artifact_refs = ["provider-raw/rework/analyst_evidence_0001.txt"]`;
- create a blocked Analyst gate whose only recovery action is `retry_analyst`;
- call `handle_blocked_gate_response(..., "retry_analyst", None)`;
- assert the attempt is `Running/Rework`;
- assert run #1 is `Superseded`;
- assert run #2 has `trigger == CodingRoleRunTrigger::RetryAnalyst`, `supersedes_run_id == run #1`, and copied `analyst_evidence` artifact ref.

Run:

```bash
cargo test --locked --test it_product retry_analyst_gate_response_supersedes_latest_analyst_run
```

Expected: FAIL until `RetryAnalyst` is handled.

- [ ] **Step 2: GREEN - handle `RetryAnalyst`**

In `handle_blocked_gate_response`, add a `CodingGateActionType::RetryAnalyst` branch:

```rust
CodingGateActionType::RetryAnalyst => {
    let previous = self.store.latest_role_run(
        &current.project_id,
        &current.issue_id,
        &current.id,
        CodingExecutionStage::Rework,
        CodingProviderRole::Analyst,
    )?;
    let resumed = self.resume_blocked_attempt_at_stage(&current, CodingExecutionStage::Rework)?;
    let new_run = self.store.supersede_latest_role_run_and_create(
        &resumed,
        CodingExecutionStage::Rework,
        CodingProviderRole::Analyst,
        CodingRoleRunTrigger::RetryAnalyst,
        None,
        gate.reason_code.clone(),
    )?;
    if let Some(previous) = previous {
        self.store.update_role_run_refs(
            &resumed.project_id,
            &resumed.issue_id,
            &resumed.id,
            &new_run.id,
            Vec::new(),
            previous.artifact_refs,
        )?;
    }
    resumed
}
```

- [ ] **Step 3: RED - WS resumes runner after retry analyst**

Add `coding_ws_retry_analyst_resumes_rework_from_persisted_evidence` to `tests/it_web/web_coding_ws_handler.rs`. The fixture must:

- create a confirmed work item and blocked coding attempt at `CodingExecutionStage::Rework`;
- create one blocked Analyst role run with artifact ref `provider-raw/rework/analyst_evidence_0001.txt`;
- write that artifact content as `persisted testing evidence`;
- create open blocked gate `coding_blocked_gate_0001` with `retry_analyst`;
- connect `/ws/coding-attempts/coding_attempt_0001`;
- send `CodingWsInMessage::GateResponse { action_id: "retry_analyst" }`;
- assert a new Rework timeline node is created;
- assert the fake Analyst provider prompt contains `persisted testing evidence`;
- assert two Analyst role runs exist.

Run:

```bash
cargo test --locked --test it_web coding_ws_retry_analyst_resumes_rework_from_persisted_evidence
```

Expected: FAIL until runner recovery is implemented.

- [ ] **Step 4: GREEN - direct Rework recovery**

In `src/web/coding_ws_handler.rs`, update `should_resume_runner_after_gate_response` to include `"retry_analyst"`.

`execute_start_coding_flow` 当前从 `PrepareContext` / `WorktreePrepare` 后直接进入 `Coding` → `Testing` → `CodeReview` → `ReviewRequest` → `InternalPrReview` 的线性 pipeline。为了能在 attempt 已经处于 `Rework` 阶段时直接恢复 Analyst，需要在 `WorktreePrepare` 处理之后、`Coding` 分支之前增加 stage-based 入口：

1. 如果 `current.stage == CodingExecutionStage::Rework`：
   - 调用 `await_stage_gate(..., CodingExecutionStage::Rework)` 等待 gate 已经被 `handle_blocked_gate_response` 解决；
   - 从 `get_role_provider_config_snapshot` 获取 Analyst provider；
   - 读取 latest Analyst role run 的 `artifact_refs`，找到包含 `analyst_evidence` 的 ref，用 `read_attempt_artifact_text` 加载 evidence 文本；
   - 调用 `execute_rework_with_commands(&current, &evidence, analyst_provider, command_rx)`；
   - 根据返回后的 `current.stage` 决定继续路径：
     - `Coding` 或 `Testing` → `continue 'pipeline` 回到对应分支；
     - `CodeReview`、`ReviewRequest`、`InternalPrReview` → 跳到对应分支；
     - 其他（如 `Blocked`、`FinalConfirm`）→ `emit_current_session_state` 后返回。
2. 如果 `current.stage` 不是 `Rework`，保持现有 pipeline 不变。

证据读取规则：优先使用最新 Analyst role run 的 `artifact_refs` 中最后一个包含 `analyst_evidence` 的 ref；若不存在，则返回 `CodingWorkspaceEngineError::ProviderStream("analyst_retry_missing_evidence".to_string())`。

- [ ] **Step 5: Run focused tests**

```bash
cargo test --locked --test it_product retry_analyst_gate_response_supersedes_latest_analyst_run
cargo test --locked --test it_web coding_ws_retry_analyst_resumes_rework_from_persisted_evidence
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/product/coding_workspace_engine.rs src/web/coding_ws_handler.rs tests/it_product/product_coding_workspace_engine.rs tests/it_web/web_coding_ws_handler.rs
git commit -m "feat: retry analyst from blocked gate"
```

## Verification

Run focused checks first:

```bash
cargo test --locked --test it_product coding_gate_action_type_round_trips_retry_analyst
cargo test --locked --test it_product updates_coding_role_run_refs_without_duplicates
cargo test --locked --test it_product execute_rework_binds_analyst_decision_chat_and_gate_to_role_run
cargo test --locked --test it_product retry_analyst_gate_response_supersedes_latest_analyst_run
cargo test --locked --test it_web coding_ws_retry_analyst_resumes_rework_from_persisted_evidence
pnpm -C web exec vitest --run src/api/types.test.ts src/pages/CodingWorkspacePage.test.tsx
```

Run full local gate:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
pnpm -C web exec vitest --run
```

Do not use Docker. Do not add `-j 1` to any Cargo command.

## Implementation Handoff

This plan should be executed after P1 and before P3/P4.

Recommended execution mode:

1. Use `superpowers:subagent-driven-development` for Task 1-4 with review between tasks.
2. Commit after each task as written above.
3. Stop after verification and report exact commands/results before starting P3.
