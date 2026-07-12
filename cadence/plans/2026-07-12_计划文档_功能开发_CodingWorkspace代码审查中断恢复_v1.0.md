# Coding Workspace 代码审查中断恢复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Code Review Provider 中断进入可恢复阻塞状态，并让历史 `failed + code_review` Attempt 在保留共享 worktree 和 Unit 进度的前提下手动重试 Reviewer。

**Architecture:** 新发生的 Code Review Provider 失败在 Workspace Engine 共享失败入口中转换为 `blocked` Gate；历史失败 Attempt 由 SessionState 派生出只读恢复 Gate，并在 WebSocket 收到 `retry_review` 后通过严格校验原地恢复。普通 terminal Attempt 仍保持不可恢复，前端复用现有 GatePanel 和提交防重逻辑。

**Tech Stack:** Rust 2024、Tokio、Axum WebSocket、文件型 JSON Store、React、TypeScript、Zustand、Vitest。

## Global Constraints

- 必须保留 `coding_attempt_0001` 的 Work Item 1 提交 `b0373a0`、Work Item 2 未提交修改和 Work Item 3 至 10 的 pending 状态。
- 不自动重试 Provider，不自动批准权限，不自动提交、reset、stash 或清理共享 worktree。
- 历史恢复只允许 `failed + code_review` 且最新失败 Node、active Unit、Role Run 和 worktree 相互一致的 Attempt。
- 新失败路径必须保留 active work item lock，不创建 `shared_worktree_dirty_manual_gate`。
- Rust 验证必须使用仓库标准命令，禁止给 Cargo 命令添加 `-j 1`。
- 前端必须使用 `pnpm`。
- 所有生产代码修改必须先有失败测试并观察到预期 RED。

---

### Task 1: 增加 Failed Code Review 的窄范围重开原语

**Files:**
- Modify: `src/product/coding_attempt_store/attempt.rs`
- Modify: `src/product/coding_attempt_store/tests.rs`

**Interfaces:**
- Produces: `CodingAttemptStore::reopen_failed_code_review_attempt(project_id, issue_id, attempt_id) -> Result<CodingExecutionAttempt, ProductStoreError>`
- Guarantees: 仅允许 `Failed + CodeReview` 转为 `Blocked + CodeReview`，清除 `completed_at`；其他终态不变化。

- [ ] **Step 1: 写失败测试**

在 `src/product/coding_attempt_store/tests.rs` 新增：

```rust
#[test]
fn reopen_failed_code_review_attempt_is_narrow_and_clears_completed_at() {
    let (_tmp, store, attempt) = setup();
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("enter code review");
    let failed = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Failed,
        )
        .expect("fail attempt");
    assert!(failed.completed_at.is_some());

    let reopened = store
        .reopen_failed_code_review_attempt(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )
        .expect("reopen failed code review");

    assert_eq!(reopened.status, CodingAttemptStatus::Blocked);
    assert_eq!(reopened.stage, CodingExecutionStage::CodeReview);
    assert_eq!(reopened.completed_at, None);
}
```

同时增加 `Completed`、`Aborted`、`Failed + Testing` 被拒绝且持久化内容不变的表驱动断言。

- [ ] **Step 2: 验证 RED**

Run:

```bash
cargo test --locked --lib coding_attempt_store::tests::reopen_failed_code_review_attempt_is_narrow_and_clears_completed_at
```

Expected: FAIL，提示 `reopen_failed_code_review_attempt` 不存在。

- [ ] **Step 3: 写最小实现**

在 `src/product/coding_attempt_store/attempt.rs` 增加：

```rust
pub fn reopen_failed_code_review_attempt(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> Result<CodingExecutionAttempt, ProductStoreError> {
    let path = self.attempt_path(project_id, issue_id, attempt_id);
    let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
    if attempt.status != CodingAttemptStatus::Failed
        || attempt.stage != CodingExecutionStage::CodeReview
    {
        return Err(ProductStoreError::Io(
            "coding_failed_review_not_recoverable".to_string(),
        ));
    }
    attempt.status = CodingAttemptStatus::Blocked;
    attempt.completed_at = None;
    attempt.updated_at = Utc::now().to_rfc3339();
    write_json(&path, &attempt)?;
    Ok(attempt)
}
```

- [ ] **Step 4: 验证 GREEN**

Run:

```bash
cargo test --locked --lib coding_attempt_store::tests::reopen_failed_code_review_attempt
```

Expected: 新增测试全部 PASS。

- [ ] **Step 5: 提交原子变更**

```bash
git add src/product/coding_attempt_store/attempt.rs src/product/coding_attempt_store/tests.rs
git commit -m "feat: add narrow failed code review reopen primitive"
```

---

### Task 2: 新 Code Review Provider 失败进入恢复 Gate

**Files:**
- Modify: `src/product/coding_workspace_engine/testing_parser.rs`
- Modify: `src/product/coding_workspace_engine/gates.rs`
- Create: `src/product/coding_workspace_engine/tests/provider_failure_recovery.rs`
- Modify: `src/product/coding_workspace_engine/tests.rs`

**Interfaces:**
- Consumes: `CodingAttemptStore::update_role_run_status`、`create_review_blocked_gate`
- Produces: `CodingWorkspaceEngine::fail_provider_stream` 在 `CodeReview` 阶段的可恢复分支。

- [ ] **Step 1: 写 Code Review 失败测试**

新增 `provider_failure_recovery.rs`，构造 `Running + CodeReview` Attempt、running active Unit、Code Review Timeline Node、Reviewer Role Run 和已加锁的共享 worktree，调用：

```rust
let error = engine
    .fail_provider_stream::<()>(
        &attempt,
        "coding_node_0009",
        "Permission request permission_1 timed out".to_string(),
    )
    .await
    .expect_err("provider timeout remains surfaced");

assert!(matches!(error, CodingWorkspaceEngineError::ProviderStream(_)));
let persisted = store.get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id).unwrap();
assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
assert_eq!(persisted.stage, CodingExecutionStage::CodeReview);
assert_eq!(persisted.completed_at, None);
```

继续断言：

- 最新 Reviewer Role Run 为 `Failed`，reason code 为 `code_review_provider_interrupted`；
- open Gate 标题为“代码审查中断”，action IDs 为 `retry_review`、`send_to_coder`、`abort`；
- active Unit 仍为 running；
- Issue shared worktree lock 仍指向当前 Work Item；
- 不存在 `shared_worktree_dirty_manual_gate`。

- [ ] **Step 2: 验证 RED**

Run:

```bash
cargo test --locked --lib provider_failure_recovery
```

Expected: FAIL，实际状态仍为 `Failed` 或生成 dirty cleanup Gate。

- [ ] **Step 3: 实现 Code Review 特化失败路径**

先把 `coding_gate_action_for_id("retry_review")` 的 label 从“重试审查”调整为“重试代码审查”，保持 action ID 和 action type 不变。

在 `fail_provider_stream` 最前面增加 Code Review 分支：

```rust
if attempt.stage == CodingExecutionStage::CodeReview {
    self.complete_timeline_node(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        node_id,
        CodingTimelineNodeStatus::Failed,
        Some(message.clone()),
    )
    .await?;
    if let Some(role_run) = self.store.latest_role_run(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        CodingExecutionStage::CodeReview,
        CodingProviderRole::CodeReviewer,
    )? && role_run.status == CodingRoleRunStatus::Running
    {
        self.store.update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            CodingRoleRunStatus::Failed,
            Some("code_review_provider_interrupted".to_string()),
        )?;
    }
    self.create_review_blocked_gate(ReviewBlockedGateInput {
        attempt,
        node_id,
        stage: CodingExecutionStage::CodeReview,
        role: CodingProviderRole::CodeReviewer,
        title: "代码审查中断".to_string(),
        description: message.clone(),
        reason_code: "code_review_provider_interrupted",
        evidence_refs: Vec::new(),
        raw_provider_output_ref: None,
    })
    .await?;
    return Err(CodingWorkspaceEngineError::ProviderStream(message));
}
```

其他 stage 保留原有 terminal failure 行为。

- [ ] **Step 4: 验证 GREEN 和非 Review 回归**

Run:

```bash
cargo test --locked --lib provider_failure_recovery
cargo test --locked --lib coding_workspace_engine::tests::provider_driven
```

Expected: 新恢复测试 PASS，其他 Provider failure 测试不变。

- [ ] **Step 5: 提交原子变更**

```bash
git add src/product/coding_workspace_engine/testing_parser.rs src/product/coding_workspace_engine/gates.rs src/product/coding_workspace_engine/tests.rs src/product/coding_workspace_engine/tests/provider_failure_recovery.rs
git commit -m "fix: block recoverably on code review provider failure"
```

---

### Task 3: 从历史 Failed Attempt 派生恢复 Gate

**Files:**
- Create: `src/product/coding_workspace_engine/failed_review_recovery.rs`
- Modify: `src/product/coding_workspace_engine/mod.rs`
- Modify: `src/web/coding_ws_handler/state.rs`
- Create: `src/web/coding_ws_handler/tests/failed_review_recovery.rs`
- Modify: `src/web/coding_ws_handler/tests.rs`

**Interfaces:**
- Produces: `recoverable_failed_code_review(coding_store, attempt) -> Result<Option<FailedCodeReviewRecovery>, CodingWorkspaceEngineError>`，定义在 product 层并供 Engine 与 Web SessionState 复用。
- Produces: SessionState 中复用原 `shared_worktree_dirty_manual_gate.gate_id` 的派生 Gate，action 为 `retry_review`，label 为“重试代码审查”。

- [ ] **Step 1: 写真实历史形态失败测试**

用与 `coding_attempt_0001` 一致的 fixture：

```rust
attempt.status = CodingAttemptStatus::Failed;
attempt.stage = CodingExecutionStage::CodeReview;
attempt.completed_at = Some("2026-07-12T04:30:59Z".to_string());
```

创建：

- active Unit 状态 `Running`；
- 最新 `coding_node_0009` 为 failed CodeReview；
- 最新 Reviewer Role Run 仍为 running，但 `node_id=coding_node_0009`；
- open `shared_worktree_dirty_manual_gate`；
- 存在共享 worktree 目录。

断言 `build_coding_session_state` 返回：

```rust
assert_eq!(gate.gate_id, dirty_gate.gate_id);
assert_eq!(gate.title, "代码审查中断");
assert_eq!(gate.stage, Some(CodingExecutionStage::CodeReview));
assert_eq!(gate.role, Some(CodingProviderRole::CodeReviewer));
assert_eq!(gate.available_actions[0].action_id, "retry_review");
assert_eq!(gate.available_actions[0].label, "重试代码审查");
```

增加负例表驱动测试：Completed、Aborted、Testing、无 active Unit、Unit 非 Running、最新 Review 已完成、Role Run 的 node ID 不匹配、worktree 不存在时不输出 Gate。

- [ ] **Step 2: 验证 RED**

Run:

```bash
cargo test --locked --lib failed_review_recovery
```

Expected: FAIL，现有 `blocked_gate_is_actionable_for_attempt` 会过滤 failed Attempt 的 Gate。

- [ ] **Step 3: 在 product 层实现纯识别模型**

在 `failed_review_recovery.rs` 新增：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FailedCodeReviewRecovery {
    pub(crate) gate_id: String,
    pub(crate) failed_node_id: String,
    pub(crate) stale_role_run_id: String,
}
```

实现 `recoverable_failed_code_review`，严格验证 Attempt、active Unit、最新 CodeReview Node、最新 Reviewer Role Run、dirty Gate 和 worktree 路径。该函数只读取 Store 和文件系统，不依赖 WebSocket 或运行注册表。通过 `mod.rs` 以 `pub(crate)` 重导出。

- [ ] **Step 4: 在 SessionState 中派生 Gate**

`state.rs` 调用 product 层识别函数，识别成功后把原 dirty Gate转换为：

```rust
CodingGateRequiredModel {
    gate_id: recovery.gate_id,
    kind: CodingGateKind::Blocked,
    title: "代码审查中断".to_string(),
    description: "上次代码审查已中断，可保留当前修改并重试 Reviewer。".to_string(),
    stage: Some(CodingExecutionStage::CodeReview),
    role: Some(CodingProviderRole::CodeReviewer),
    expires_at: None,
    provider_snapshot: None,
    available_actions: vec![CodingGateAction {
        action_id: "retry_review".to_string(),
        label: "重试代码审查".to_string(),
        action_type: CodingGateActionType::RetryReview,
    }],
    reason_code: Some("failed_code_review_recoverable".to_string()),
    evidence_refs: vec![recovery.failed_node_id, recovery.stale_role_run_id],
    raw_provider_output_ref: None,
}
```

SessionState 构建保持只读，不修改持久化 Attempt、Gate 或 Role Run。

- [ ] **Step 5: 验证 GREEN**

Run:

```bash
cargo test --locked --lib failed_review_recovery
```

Expected: 正例和全部负例 PASS。

- [ ] **Step 6: 提交原子变更**

```bash
git add src/product/coding_workspace_engine/failed_review_recovery.rs src/product/coding_workspace_engine/mod.rs src/web/coding_ws_handler/state.rs src/web/coding_ws_handler/tests.rs src/web/coding_ws_handler/tests/failed_review_recovery.rs
git commit -m "feat: expose failed code review recovery gate"
```

---

### Task 4: WebSocket 原地恢复并防止重复 Reviewer

**Files:**
- Create: `src/product/coding_attempt_store/recovery.rs`
- Modify: `src/product/coding_attempt_store/mod.rs`
- Modify: `src/product/coding_workspace_engine/failed_review_recovery.rs`
- Modify: `src/web/state.rs`
- Modify: `src/web/coding_ws_handler/socket.rs`
- Modify: `src/web/coding_ws_handler/runner.rs`
- Modify: `src/web/coding_ws_handler/tests/failed_review_recovery.rs`

**Interfaces:**
- Consumes: `CodingAttemptStore::reopen_failed_code_review_attempt`
- Produces: `CodingWorkspaceEngine::recover_failed_code_review(gate_id) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError>`
- Produces: `failed_code_review_recovery_request(...)` WebSocket guard。
- Produces: `CodingRunRegistry::try_reserve_attempt(attempt_id) -> Option<CodingRunReservation>`，同一 Attempt 的 reservation/active Runner 原子互斥。
- Produces: Attempt 级 `FailedCodeReviewRecoveryJournal` 与幂等 phase 继续能力。

- [ ] **Step 1: 写恢复执行失败测试**

覆盖：

```rust
let updated = engine
    .recover_failed_code_review("coding_blocked_gate_0001")
    .await
    .expect("recover failed review");

assert_eq!(updated.status, CodingAttemptStatus::Running);
assert_eq!(updated.stage, CodingExecutionStage::CodeReview);
assert_eq!(updated.completed_at, None);
```

继续断言：

- 原 dirty Gate 已 resolved；
- 旧僵尸 Reviewer Role Run 为 Superseded；
- 新 Role Run 为 Running、trigger 为 RetryReview、`supersedes_run_id` 指向旧 Run；
- active Unit、全部 Unit 顺序和 worktree `git status --porcelain` 前后完全相同；
- stale gate ID、状态变化、node/run 不匹配均返回错误且不创建新 Role Run。
- 同一 Attempt 两次并发 reservation 只有一次成功；释放未激活 reservation 后可再次取得。
- 手工构造 prepared、attempt reopened、retry run prepared、attempt running、gate resolved 等 journal/持久化前缀，重复调用后均收敛为同一个 RetryReview Role Run、Running CodeReview Attempt 和 resolved Gate。

- [ ] **Step 2: 写 WebSocket guard RED 测试**

在 handler 测试中构造 failed Attempt：

```rust
assert!(failed_code_review_recovery_request(
    &coding_store,
    &attempt,
    &CodingWsInMessage::GateResponse {
        gate_id: "coding_blocked_gate_0001".to_string(),
        action_id: "retry_review".to_string(),
        extra_context: None,
    },
));
assert!(!is_coding_ws_message_allowed(
    &CodingAttemptStatus::Failed,
    &CodingExecutionStage::CodeReview,
    &CodingWsInMessage::ContextNote { content: "continue".to_string() },
));
```

Run:

```bash
cargo test --locked --lib failed_review_recovery
```

Expected: FAIL，恢复方法与特殊 guard 尚不存在。

- [ ] **Step 3: 实现 Engine 恢复**

`recover_failed_code_review` 必须：

1. 重新加载 Attempt；
2. 若不存在 journal，调用同文件的 `recoverable_failed_code_review` 确认 gate/node/run，写入包含 expected IDs 的 prepared journal；
3. 若 journal 已存在，只接受相同 attempt/gate，并按 journal expected node/run 继续；
4. 幂等执行 Failed→Blocked；
5. 用 stable recovery key 创建或复用唯一 RetryReview Run，并精确 supersede journal 指定的 stale Run；
6. 幂等执行 Blocked→Running；
7. 幂等 resolve 原 dirty Gate；
8. 返回可启动 Runner 的 Attempt，但此时 journal 保持未完成；
9. Runner 使用 reservation 激活后，再标记 journal runner started/completed。

禁止放宽全局 `valid_status_transition`，禁止让任意 Failed Attempt 通用地转回 Running。

每个 phase 操作必须允许“操作已成功但 phase 写入尚未完成”的重复进入。禁止在校验后再次按“最新 Role Run”选择写入目标；必须使用 journal 中的 expected stale role run ID。Store 新增的幂等 Role Run 方法必须通过 recovery key 复用已创建的 RetryReview Run。

- [ ] **Step 4: 接入 WebSocket 并检查运行注册表**

在普通 `is_coding_ws_message_allowed` 拒绝前单独计算恢复请求。恢复请求既可来自原始 Failed 形态，也可来自匹配未完成 journal 的 Blocked/Running CodeReview 前缀：

```rust
let failed_review_recovery = failed_code_review_recovery_request(
    &coding_store,
    &current_attempt,
    &inbound,
);
if !is_coding_ws_message_allowed(
    &current_attempt.status,
    &current_attempt.stage,
    &inbound,
) && !failed_review_recovery {
    // 保留 coding_message_not_allowed
}
```

执行恢复前原子取得 reservation：

```rust
let Some(reservation) = state
    .coding_runs
    .try_reserve_attempt(&current_attempt.id)
else {
    send coding_recovery_already_active;
    continue;
};
```

Engine 恢复失败时 reservation 通过 Drop/显式 release 释放。恢复成功后使用新增的 reserved spawn 接口激活同一 reservation，禁止再次 insert。激活后标记 recovery journal completed。`should_resume_runner_after_gate_response` 继续只处理普通 Blocked/WaitingForHuman 路径。

- [ ] **Step 5: 验证 GREEN 和重复点击**

Run:

```bash
cargo test --locked --lib failed_review_recovery
cargo test --locked --lib web::coding_ws_handler::tests
```

Expected: 恢复成功；并发请求只有一个 reservation/Runner；各 journal 前缀重试均收敛；第二次请求返回 already active 或复用未完成 journal；普通 failed Attempt 仍返回 `coding_message_not_allowed`。

- [ ] **Step 6: 提交原子变更**

```bash
git add src/product/coding_attempt_store/recovery.rs src/product/coding_attempt_store/mod.rs src/product/coding_workspace_engine/failed_review_recovery.rs src/web/state.rs src/web/coding_ws_handler/socket.rs src/web/coding_ws_handler/runner.rs src/web/coding_ws_handler/tests/failed_review_recovery.rs
git commit -m "feat: retry interrupted code review in place"
```

---

### Task 5: 前端恢复 Gate 与提交状态回归

**Files:**
- Modify: `web/src/pages/CodingWorkspacePage.gates.test.tsx`

**Interfaces:**
- Consumes: 现有 `CodingGateRequired`、`respondGate`、`markGateSubmitting`、ProtocolError 复位逻辑。
- Produces: “重试代码审查”可见、单击发送、pending 防重、错误后复位的 UI 行为。

- [ ] **Step 1: 写前端失败测试**

构造：

```typescript
pendingGates: [{
  gate_id: "coding_blocked_gate_0001",
  kind: "blocked",
  title: "代码审查中断",
  description: "上次代码审查已中断，可保留当前修改并重试 Reviewer。",
  stage: "code_review",
  role: "code_reviewer",
  available_actions: [{
    action_id: "retry_review",
    label: "重试代码审查",
    action_type: "retry_review",
  }],
  reason_code: "failed_code_review_recoverable",
  evidence_refs: [],
}]
```

断言：

```typescript
await userEvent.click(screen.getByRole("button", { name: "重试代码审查" }));
expect(api.respondGate).toHaveBeenCalledWith(
  "coding_blocked_gate_0001",
  "retry_review",
  undefined,
);
```

并验证 submitting 时按钮 disabled，ProtocolError 后重新 enabled，Composer 不发送 Context Note 代替恢复。

- [ ] **Step 2: 验证现有通用 Gate 行为**

Run:

```bash
cd web && pnpm test -- CodingWorkspacePage.gates.test.tsx
```

Expected: 现有通用 Gate UI 直接 PASS；本任务只新增历史恢复 Gate 的前端契约回归，不新增重复 UI。

- [ ] **Step 3: 验证前端完整契约**

Run:

```bash
cd web && pnpm test -- CodingWorkspacePage.gates.test.tsx useCodingWorkspaceWs.actions.test.tsx coding-workspace-store.test.ts
cd web && pnpm tsc -b
```

Expected: 定向测试和 TypeScript 构建 PASS。

- [ ] **Step 4: 提交原子变更**

```bash
git add web/src/pages/CodingWorkspacePage.gates.test.tsx
git commit -m "test: cover interrupted code review recovery gate"
```

---

### Task 6: 当前 Attempt 验收与完整门禁

**Files:**
- Verify only: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json`
- Verify only: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/`
- Verify only: `/home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001`

**Interfaces:**
- Consumes: 完成后的 SessionState 和 GateResponse 恢复路径。
- Produces: 用户可见“重试代码审查”，但不自动点击、不自动启动 Provider。

- [ ] **Step 1: 运行 Rust 定向测试**

```bash
cargo test --locked --lib provider_failure_recovery
cargo test --locked --lib failed_review_recovery
```

Expected: 全部 PASS。

- [ ] **Step 2: 运行完整 Rust 门禁**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

Expected: 全部 exit code 0。

- [ ] **Step 3: 运行完整前端门禁**

```bash
cd web && pnpm tsc -b
cd web && pnpm test
```

Expected: TypeScript 构建和全部 Vitest PASS。

- [ ] **Step 4: 记录恢复前数据指纹**

只读记录：

```bash
git -C /home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001 status --short
git -C /home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001 diff --stat
jq '{status,stage,current_work_item_id,active_unit_id,completed_at}' .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json
```

Expected: Work Item 2 修改仍存在，Attempt 仍为 failed，尚未自动调用 Provider。

- [ ] **Step 5: 重启开发服务并验证恢复入口**

按 `prepare-aria-worktree-dev` 流程重启后端和前端。刷新 Coding Workspace，确认显示“重试代码审查”。此步只验证按钮，不自动点击。

- [ ] **Step 6: 用户手动点击后的数据验收**

用户点击后只读验证：

- Attempt 为 `running + code_review`，`completed_at=null`；
- 新 Reviewer Role Run trigger 为 `retry_review`；
- 旧 `coding_role_run_0008` 被 supersede；
- active Unit 仍是 Work Item 2；
- Work Item 1 completed，Work Item 3 至 10 pending；
- 共享 worktree `git status --short` 与点击前一致。

- [ ] **Step 7: 最终差异检查**

```bash
git diff --check
git status --short
```

Expected: 无空白错误；明确区分本功能、既有结构化审核恢复和 Workspace 中断恢复改动，不混合提交未知文件。
