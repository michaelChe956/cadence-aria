# Coding Workspace 重复代码审查中断恢复 Journal 轮换实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让同一 Coding Attempt 可以安全完成多次 Code Review Provider 中断恢复，并修正当前 `coding_attempt_0001`，使其保留 Work Item 7 Coder 内容后重新进入 Code Review。

**Architecture:** 保留单一 `failed-code-review-recovery.json` 作为当前恢复事务；当下一次不同 identity 的恢复开始时，把旧 completed journal 原子移动到 `failed-code-review-recoveries/completed/<gate_id>.json`。恢复识别对未完成 journal 保持排他，对已完成且结束 Runner 交接的 journal继续扫描当前 interrupted Gate；所有持久化轮换仍在 Attempt 级 recovery reservation 之后执行。

**Tech Stack:** Rust 2024、Tokio、Serde JSON、现有 `CodingAttemptStore`、`CodingWorkspaceEngine`、Coding WebSocket 测试夹具、Git 文件系统 worktree。

## Global Constraints

- 所有代码、测试、文档和数据检查均在 `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0709` 中进行。
- 严格遵循 TDD：先添加可稳定复现“第二次 Review 中断”的失败测试，再修改实现。
- 不新增、不运行 Playwright、浏览器或任何 E2E 测试。
- 不修改前端代码；若调查证明前端必须变化，先停止并向用户说明，不在本计划中扩大范围。
- 不修改 Reviewer Prompt、Work Item Draft、Story Spec、Design Spec 或 Work Item 产物 Workspace Engine。
- 不修改 `/home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001` 中的 Coder 文件、提交或未提交 diff。
- 不使用 `rollback-coding-attempt` skill：当前操作是 blocked Review Gate 的窄范围 journal 修复，不是 Unit 边界回退；该 skill 明确规定此场景应使用 Gate 重试。
- 当前 Attempt 数据只在平台代码通过定向与全量验证后修复，修复前必须完整备份并记录业务 worktree 指纹。
- 当前 Attempt 数据修复只归档旧 completed journal，不修改 Attempt、Unit、Timeline Node、Role Run 或 Gate JSON 内容。
- Rust 定向测试必须使用 `cargo test --locked --lib <过滤名>`，禁止 `cargo test --locked <过滤名>`，禁止任何 Cargo 命令使用 `-j 1`。
- 保留用户未提交文件 `.superpowers/sdd/final-review-fix-report.md`，不得暂存、提交或覆盖。
- 每个源码任务使用独立原子提交，最终推送 `origin/feat-b-0709`。

---

## 文件结构与职责

- Modify: `src/product/coding_attempt_store/recovery.rs`
  - 负责当前 journal 路径、completed 历史路径、归档冲突校验和 prepare-or-rotate 语义。
- Modify: `src/product/coding_attempt_store/tests.rs`
  - 注册新的 Store 级 journal 轮换测试子模块。
- Create: `src/product/coding_attempt_store/tests/failed_review_recovery.rs`
  - 独立验证 completed 轮换、未完成排他、归档冲突和崩溃窗口。
- Modify: `src/product/coding_workspace_engine/failed_review_recovery.rs`
  - 让只读恢复识别跳过已结束交接的 completed 历史，并让 Engine 始终按精确 recovery identity 调用 Store prepare-or-rotate。
- Modify: `src/web/coding_ws_handler/tests/failed_review_recovery/support.rs`
  - 构造“第一次恢复完成后，Retry Reviewer 再次中断”的可复用 fixture。
- Modify: `src/web/coding_ws_handler/tests/failed_review_recovery.rs`
  - 验证第二次 Gate 可展示、旧 journal 被归档、Role Run supersede 链正确。
- Modify: `src/web/coding_ws_handler/tests/failed_review_recovery/runner.rs`
  - 验证第二次中断时两个 socket 仍只产生一个 reservation、当前 Retry Run 和 Runner。
- Runtime data: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/**`
  - 平台验证完成后只移动旧 journal，并校验当前 Attempt 业务状态完全不变。
- Read-only business worktree: `/home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001`
  - 只记录并比对 HEAD、status 和 binary diff 哈希。

---

### Task 1: 为 CodingAttemptStore 增加 completed journal 历史归档与轮换

**Files:**

- Modify: `src/product/coding_attempt_store/recovery.rs:1-112`
- Modify: `src/product/coding_attempt_store/tests.rs:1-18`
- Create: `src/product/coding_attempt_store/tests/failed_review_recovery.rs`

**Interfaces:**

- Consumes: `FailedCodeReviewRecoveryJournal`、`FailedCodeReviewRecoveryPhase`、`write_json` 的原子临时文件写入、`CodingAttemptStore::attempt_dir`。
- Produces: `CodingAttemptStore::get_archived_failed_code_review_recovery_journal(project_id, issue_id, attempt_id, gate_id) -> Result<Option<FailedCodeReviewRecoveryJournal>, ProductStoreError>`。
- Produces: `prepare_failed_code_review_recovery_journal` 的新语义：相同 identity 幂等返回；不同未完成 identity 拒绝；不同 completed identity 先归档再创建。

- [ ] **Step 1: 注册 Store 测试子模块**

在 `src/product/coding_attempt_store/tests.rs` 的常量定义前加入：

```rust
mod failed_review_recovery;
```

- [ ] **Step 2: 写 completed journal 轮换的失败测试**

创建 `src/product/coding_attempt_store/tests/failed_review_recovery.rs`：

```rust
use std::path::PathBuf;

use super::setup;
use crate::product::coding_attempt_store::{
    FAILED_CODE_REVIEW_RECOVERY_JOURNAL_FILE, FailedCodeReviewRecoveryJournal,
    FailedCodeReviewRecoveryPhase,
};
use crate::product::json_store::{ProductStoreError, read_json, write_json};

fn journal(
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    gate_id: &str,
    failed_node_id: &str,
    stale_role_run_id: &str,
    phase: FailedCodeReviewRecoveryPhase,
) -> FailedCodeReviewRecoveryJournal {
    let completed = phase == FailedCodeReviewRecoveryPhase::Completed;
    FailedCodeReviewRecoveryJournal {
        attempt_id: attempt.id.clone(),
        project_id: attempt.project_id.clone(),
        issue_id: attempt.issue_id.clone(),
        expected_gate_id: gate_id.to_string(),
        expected_failed_node_id: failed_node_id.to_string(),
        expected_stale_role_run_id: stale_role_run_id.to_string(),
        recovery_key: format!(
            "failed_code_review_recovery:{}:{gate_id}:{stale_role_run_id}",
            attempt.id
        ),
        retry_role_run_id: Some("coding_role_run_0002".to_string()),
        phase,
        runner_started_at: completed.then(|| "2026-07-12T12:53:57Z".to_string()),
        completed_at: completed.then(|| "2026-07-12T12:53:57Z".to_string()),
        created_at: "2026-07-12T12:50:00Z".to_string(),
        updated_at: "2026-07-12T12:53:57Z".to_string(),
    }
}

fn current_path(
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
) -> PathBuf {
    store
        .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .join(FAILED_CODE_REVIEW_RECOVERY_JOURNAL_FILE)
}

fn archived_path(
    store: &crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    gate_id: &str,
) -> PathBuf {
    store
        .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .join("failed-code-review-recoveries")
        .join("completed")
        .join(format!("{gate_id}.json"))
}

#[test]
fn prepare_archives_completed_journal_before_creating_new_identity() {
    let (_tmp, store, attempt) = setup();
    let old = journal(
        &attempt,
        "coding_blocked_gate_0001",
        "coding_node_0009",
        "coding_role_run_0008",
        FailedCodeReviewRecoveryPhase::Completed,
    );
    write_json(&current_path(&store, &attempt), &old).expect("seed completed journal");

    let current = store
        .prepare_failed_code_review_recovery_journal(
            &attempt,
            "coding_blocked_gate_0007",
            "coding_node_0030",
            "coding_role_run_0029",
        )
        .expect("rotate completed journal");

    assert_eq!(current.expected_gate_id, "coding_blocked_gate_0007");
    assert_eq!(current.phase, FailedCodeReviewRecoveryPhase::Prepared);
    assert_eq!(
        store
            .get_archived_failed_code_review_recovery_journal(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                "coding_blocked_gate_0001",
            )
            .expect("archived journal")
            .expect("completed history"),
        old
    );
}

#[test]
fn prepare_rejects_different_identity_while_current_journal_is_unfinished() {
    let (_tmp, store, attempt) = setup();
    let old = journal(
        &attempt,
        "coding_blocked_gate_0001",
        "coding_node_0009",
        "coding_role_run_0008",
        FailedCodeReviewRecoveryPhase::GateResolved,
    );
    write_json(&current_path(&store, &attempt), &old).expect("seed unfinished journal");

    let rejected = store.prepare_failed_code_review_recovery_journal(
        &attempt,
        "coding_blocked_gate_0007",
        "coding_node_0030",
        "coding_role_run_0029",
    );

    assert!(matches!(
        rejected,
        Err(ProductStoreError::Io(message))
            if message == "coding_failed_review_recovery_state_changed"
    ));
    assert_eq!(
        read_json::<FailedCodeReviewRecoveryJournal>(&current_path(&store, &attempt))
            .expect("unchanged current journal"),
        old
    );
    assert!(!archived_path(&store, &attempt, "coding_blocked_gate_0001").exists());
}

#[test]
fn prepare_reuses_identical_archive_after_rotation_crash() {
    let (_tmp, store, attempt) = setup();
    let old = journal(
        &attempt,
        "coding_blocked_gate_0001",
        "coding_node_0009",
        "coding_role_run_0008",
        FailedCodeReviewRecoveryPhase::Completed,
    );
    write_json(&current_path(&store, &attempt), &old).expect("seed current journal");
    write_json(
        &archived_path(&store, &attempt, "coding_blocked_gate_0001"),
        &old,
    )
    .expect("seed identical archived journal");

    let current = store
        .prepare_failed_code_review_recovery_journal(
            &attempt,
            "coding_blocked_gate_0007",
            "coding_node_0030",
            "coding_role_run_0029",
        )
        .expect("converge duplicate archive prefix");

    assert_eq!(current.expected_gate_id, "coding_blocked_gate_0007");
    assert_eq!(
        read_json::<FailedCodeReviewRecoveryJournal>(&archived_path(
            &store,
            &attempt,
            "coding_blocked_gate_0001",
        ))
        .expect("preserved archive"),
        old
    );
}

#[test]
fn prepare_rejects_conflicting_archive_without_overwriting_audit_history() {
    let (_tmp, store, attempt) = setup();
    let old = journal(
        &attempt,
        "coding_blocked_gate_0001",
        "coding_node_0009",
        "coding_role_run_0008",
        FailedCodeReviewRecoveryPhase::Completed,
    );
    let conflicting = journal(
        &attempt,
        "coding_blocked_gate_0001",
        "coding_node_0999",
        "coding_role_run_0999",
        FailedCodeReviewRecoveryPhase::Completed,
    );
    write_json(&current_path(&store, &attempt), &old).expect("seed current journal");
    write_json(
        &archived_path(&store, &attempt, "coding_blocked_gate_0001"),
        &conflicting,
    )
    .expect("seed conflicting archive");

    let rejected = store.prepare_failed_code_review_recovery_journal(
        &attempt,
        "coding_blocked_gate_0007",
        "coding_node_0030",
        "coding_role_run_0029",
    );

    assert!(matches!(
        rejected,
        Err(ProductStoreError::Io(message))
            if message == "coding_failed_review_recovery_state_changed"
    ));
    assert_eq!(
        read_json::<FailedCodeReviewRecoveryJournal>(&current_path(&store, &attempt))
            .expect("preserved current journal"),
        old
    );
    assert_eq!(
        read_json::<FailedCodeReviewRecoveryJournal>(&archived_path(
            &store,
            &attempt,
            "coding_blocked_gate_0001",
        ))
        .expect("preserved conflicting archive"),
        conflicting
    );
}

#[test]
fn prepare_recreates_current_journal_after_archive_before_write_crash() {
    let (_tmp, store, attempt) = setup();
    let old = journal(
        &attempt,
        "coding_blocked_gate_0001",
        "coding_node_0009",
        "coding_role_run_0008",
        FailedCodeReviewRecoveryPhase::Completed,
    );
    write_json(
        &archived_path(&store, &attempt, "coding_blocked_gate_0001"),
        &old,
    )
    .expect("seed archive-only crash prefix");

    let current = store
        .prepare_failed_code_review_recovery_journal(
            &attempt,
            "coding_blocked_gate_0007",
            "coding_node_0030",
            "coding_role_run_0029",
        )
        .expect("recreate current journal");

    assert_eq!(current.expected_gate_id, "coding_blocked_gate_0007");
    assert_eq!(current.phase, FailedCodeReviewRecoveryPhase::Prepared);
    assert_eq!(
        store
            .get_archived_failed_code_review_recovery_journal(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                "coding_blocked_gate_0001",
            )
            .expect("archived journal")
            .expect("completed history"),
        old
    );
}

#[test]
fn archived_journal_lookup_rejects_gate_path_escape() {
    let (_tmp, store, attempt) = setup();

    let rejected = store.get_archived_failed_code_review_recovery_journal(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        "../coding_blocked_gate_0001",
    );

    assert!(matches!(
        rejected,
        Err(ProductStoreError::PathEscape(value))
            if value == "../coding_blocked_gate_0001"
    ));
}
```

- [ ] **Step 3: 运行 Store 测试并确认 RED**

运行：

```bash
cargo test --locked --lib coding_attempt_store::tests::failed_review_recovery
```

Expected: 编译失败，明确提示 `get_archived_failed_code_review_recovery_journal` 不存在；不得通过删除断言或放宽测试解决。

- [ ] **Step 4: 实现历史路径、读取和归档 helper**

在 `src/product/coding_attempt_store/recovery.rs` 顶部增加：

```rust
use std::fs;
use std::path::PathBuf;
```

在当前 journal 常量之后增加：

```rust
const FAILED_CODE_REVIEW_RECOVERIES_DIR: &str = "failed-code-review-recoveries";
const COMPLETED_FAILED_CODE_REVIEW_RECOVERIES_DIR: &str = "completed";
```

在 `impl CodingAttemptStore` 内加入路径和读取方法，并让现有 current getter 使用路径 helper：

```rust
fn failed_code_review_recovery_journal_path(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> Result<PathBuf, ProductStoreError> {
    validate_relative_id(project_id)?;
    validate_relative_id(issue_id)?;
    validate_relative_id(attempt_id)?;
    Ok(self
        .attempt_dir(project_id, issue_id, attempt_id)
        .join(FAILED_CODE_REVIEW_RECOVERY_JOURNAL_FILE))
}

fn archived_failed_code_review_recovery_journal_path(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
    gate_id: &str,
) -> Result<PathBuf, ProductStoreError> {
    validate_relative_id(project_id)?;
    validate_relative_id(issue_id)?;
    validate_relative_id(attempt_id)?;
    validate_relative_id(gate_id)?;
    Ok(self
        .attempt_dir(project_id, issue_id, attempt_id)
        .join(FAILED_CODE_REVIEW_RECOVERIES_DIR)
        .join(COMPLETED_FAILED_CODE_REVIEW_RECOVERIES_DIR)
        .join(format!("{gate_id}.json")))
}

pub fn get_archived_failed_code_review_recovery_journal(
    &self,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
    gate_id: &str,
) -> Result<Option<FailedCodeReviewRecoveryJournal>, ProductStoreError> {
    let path = self.archived_failed_code_review_recovery_journal_path(
        project_id,
        issue_id,
        attempt_id,
        gate_id,
    )?;
    if !super::path_is_regular_file(&path)? {
        return Ok(None);
    }
    read_json(&path).map(Some)
}

fn archive_completed_failed_code_review_recovery_journal(
    &self,
    journal: &FailedCodeReviewRecoveryJournal,
) -> Result<(), ProductStoreError> {
    if !journal.is_completed()
        || journal.runner_started_at.is_none()
        || journal.completed_at.is_none()
    {
        return Err(recovery_state_changed());
    }
    let current_path = self.failed_code_review_recovery_journal_path(
        &journal.project_id,
        &journal.issue_id,
        &journal.attempt_id,
    )?;
    if !super::path_is_regular_file(&current_path)? {
        return Err(recovery_state_changed());
    }
    let archived_path = self.archived_failed_code_review_recovery_journal_path(
        &journal.project_id,
        &journal.issue_id,
        &journal.attempt_id,
        &journal.expected_gate_id,
    )?;
    if super::path_is_regular_file(&archived_path)? {
        let archived: FailedCodeReviewRecoveryJournal = read_json(&archived_path)?;
        if archived != *journal {
            return Err(recovery_state_changed());
        }
        super::remove_file_if_exists(&current_path)?;
        return Ok(());
    }
    if let Some(parent) = archived_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    fs::rename(&current_path, &archived_path).map_err(|error| {
        ProductStoreError::Io(format!(
            "rename {} to {}: {error}",
            current_path.display(),
            archived_path.display()
        ))
    })
}
```

把 `get_failed_code_review_recovery_journal` 中的手工路径构造替换为：

```rust
let path = self.failed_code_review_recovery_journal_path(project_id, issue_id, attempt_id)?;
```

- [ ] **Step 5: 实现 prepare-or-rotate 语义**

把 `prepare_failed_code_review_recovery_journal` 现有的 journal 冲突分支替换为：

```rust
if let Some(existing) = self.get_failed_code_review_recovery_journal(
    &attempt.project_id,
    &attempt.issue_id,
    &attempt.id,
)? {
    if existing.expected_gate_id == gate_id
        && existing.expected_failed_node_id == failed_node_id
        && existing.expected_stale_role_run_id == stale_role_run_id
    {
        return Ok(existing);
    }
    if !existing.is_completed() {
        return Err(recovery_state_changed());
    }
    self.archive_completed_failed_code_review_recovery_journal(&existing)?;
}
```

把 `save_failed_code_review_recovery_journal` 中的 current 路径构造也替换为同一个 path helper，避免读取和写入路径漂移：

```rust
let path = self.failed_code_review_recovery_journal_path(
    &journal.project_id,
    &journal.issue_id,
    &journal.attempt_id,
)?;
```

- [ ] **Step 6: 运行 Store 测试并确认 GREEN**

运行：

```bash
cargo test --locked --lib coding_attempt_store::tests::failed_review_recovery
```

Expected: 6 个新测试全部通过；现有相同 identity 幂等语义保持不变。

- [ ] **Step 7: 运行现有 journal 前缀回归**

运行：

```bash
cargo test --locked --lib failed_review_recovery_journal
```

Expected: 现有 prepared、attempt reopened、retry run created、attempt running、gate resolved、completed 相关测试全部通过。

- [ ] **Step 8: 提交 Store 轮换实现**

```bash
git add \
  src/product/coding_attempt_store/recovery.rs \
  src/product/coding_attempt_store/tests.rs \
  src/product/coding_attempt_store/tests/failed_review_recovery.rs
git commit -m "fix: rotate completed failed review journals"
```

Expected: 提交仅包含 Store 实现与 Store 测试，不包含 `.superpowers/sdd/final-review-fix-report.md`。

---

### Task 2: 识别并恢复同一 Attempt 的第二次 Review 中断

**Files:**

- Modify: `src/product/coding_workspace_engine/failed_review_recovery.rs:27-49,253-299`
- Modify: `src/web/coding_ws_handler/tests/failed_review_recovery/support.rs:1-18,40-46,210-470`
- Modify: `src/web/coding_ws_handler/tests/failed_review_recovery.rs:1-24,560-690`

**Interfaces:**

- Consumes: Task 1 的 `prepare_failed_code_review_recovery_journal` prepare-or-rotate 语义和 archived journal getter。
- Produces: `seed_repeated_interrupted_review(fixture: &FailedReviewFixture) -> RepeatedInterruptedReview` 异步测试 helper。
- Produces: `recoverable_failed_code_review` 对 completed journal 的三态语义：未完成继续、completed 交接窗口继续、completed 历史则扫描当前 Gate。
- Produces: `recover_failed_code_review_for_attempt` 始终先解析精确 recovery identity，再调用 Store prepare-or-rotate。

- [ ] **Step 1: 增加重复中断 fixture 数据结构**

在 `support.rs` 增加 imports：

```rust
use tokio::sync::mpsc;

use crate::product::coding_attempt_store::{
    CodingAttemptStore, CreateBlockedGateInput, CreateCodingExecutionUnitInput,
    FailedCodeReviewRecoveryJournal,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::git_workspace_service::GitWorkspaceService;
```

用以上 `coding_attempt_store` import 替换原来的同名 import，并在 `FailedReviewFixture` 后加入：

```rust
pub(super) struct RepeatedInterruptedReview {
    pub(super) blocked_attempt: CodingExecutionAttempt,
    pub(super) first_journal: FailedCodeReviewRecoveryJournal,
    pub(super) first_retry_role_run_id: String,
    pub(super) second_gate: CodingGateRequired,
}
```

在 `failed_review_fixture` 之前加入完整 helper：

```rust
pub(super) async fn seed_repeated_interrupted_review(
    fixture: &FailedReviewFixture,
) -> RepeatedInterruptedReview {
    let first_gate_id = fixture
        .dirty_gate
        .as_ref()
        .expect("first provider interrupted gate")
        .gate_id
        .clone();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        event_tx,
    );
    let first_running = engine
        .recover_failed_code_review_for_attempt(&fixture.attempt.id, &first_gate_id)
        .await
        .expect("first interrupted review recovery");
    let first_journal = fixture
        .store
        .complete_failed_code_review_recovery_journal(&first_running.id, &first_gate_id)
        .expect("complete first recovery journal");
    let first_retry_role_run_id = first_journal
        .retry_role_run_id
        .clone()
        .expect("first retry reviewer run");
    let second_failed_node_id = "coding_node_0010";
    fixture
        .store
        .save_timeline_node(CodingTimelineNode {
            id: second_failed_node_id.to_string(),
            attempt_id: first_running.id.clone(),
            stage: CodingExecutionStage::CodeReview,
            title: "代码审查".to_string(),
            status: CodingTimelineNodeStatus::Failed,
            agent_role: Some(CodingAgentRole::Reviewer),
            summary: Some("second review provider interrupted".to_string()),
            started_at: "2026-07-12T04:40:00Z".to_string(),
            completed_at: Some("2026-07-12T04:40:59Z".to_string()),
            artifact_refs: Vec::new(),
        })
        .expect("second failed review node");
    fixture
        .store
        .attach_role_run_node(
            &first_running.project_id,
            &first_running.issue_id,
            &first_running.id,
            &first_retry_role_run_id,
            second_failed_node_id.to_string(),
        )
        .expect("bind first retry reviewer node");
    fixture
        .store
        .update_role_run_status(
            &first_running.project_id,
            &first_running.issue_id,
            &first_running.id,
            &first_retry_role_run_id,
            CodingRoleRunStatus::Failed,
            Some("code_review_provider_interrupted".to_string()),
        )
        .expect("fail first retry reviewer");
    let blocked_attempt = fixture
        .store
        .update_attempt_status(
            &first_running.project_id,
            &first_running.issue_id,
            &first_running.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("block attempt for second interruption");
    let second_gate = fixture
        .store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: blocked_attempt.id.clone(),
            stage: CodingExecutionStage::CodeReview,
            node_id: Some(second_failed_node_id.to_string()),
            role: Some(CodingProviderRole::CodeReviewer),
            title: "代码审查中断".to_string(),
            description: "second review provider interrupted".to_string(),
            reason_code: Some("code_review_provider_interrupted".to_string()),
            evidence_refs: Vec::new(),
            raw_provider_output_ref: None,
            available_actions: vec![
                CodingGateAction {
                    action_id: "retry_review".to_string(),
                    label: "重试代码审查".to_string(),
                    action_type: CodingGateActionType::RetryReview,
                },
                CodingGateAction {
                    action_id: "send_to_coder".to_string(),
                    label: "发送给 Coder".to_string(),
                    action_type: CodingGateActionType::SendToCoder,
                },
                CodingGateAction {
                    action_id: "abort".to_string(),
                    label: "终止".to_string(),
                    action_type: CodingGateActionType::Abort,
                },
            ],
        })
        .expect("second provider interrupted gate");

    RepeatedInterruptedReview {
        blocked_attempt,
        first_journal,
        first_retry_role_run_id,
        second_gate,
    }
}
```

- [ ] **Step 2: 写第二次中断恢复的失败集成测试**

在 `failed_review_recovery.rs` 的 support import 中加入 `seed_repeated_interrupted_review`，并增加：

```rust
#[tokio::test]
async fn completed_journal_rotates_when_later_review_is_interrupted() {
    let fixture = failed_review_fixture(
        CodingAttemptScope::WorkItemGroup,
        FixtureCase::BlockedProviderInterrupted,
    );
    let repeated = seed_repeated_interrupted_review(&fixture).await;
    let second_gate_id = repeated.second_gate.gate_id.clone();
    let recovery = recoverable_failed_code_review(&fixture.store, &repeated.blocked_attempt)
        .expect("recoverable second interruption")
        .expect("second recovery identity");
    assert_eq!(recovery.gate_id, second_gate_id);
    assert_eq!(recovery.failed_node_id, "coding_node_0010");
    assert_eq!(recovery.stale_role_run_id, repeated.first_retry_role_run_id);

    let state = build_coding_session_state(&fixture.store, repeated.blocked_attempt.clone())
        .expect("session state for second interruption");
    let CodingWsOutMessage::CodingSessionState { pending_gates, .. } = state else {
        panic!("expected coding session state");
    };
    assert!(pending_gates.iter().any(|gate| gate.gate_id == second_gate_id));
    assert!(failed_code_review_recovery_request(
        &fixture.store,
        &repeated.blocked_attempt,
        &CodingWsInMessage::GateResponse {
            gate_id: second_gate_id.clone(),
            action_id: "retry_review".to_string(),
            extra_context: None,
        },
    ));

    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        event_tx,
    );
    let running = engine
        .recover_failed_code_review_for_attempt(&fixture.attempt.id, &second_gate_id)
        .await
        .expect("recover second interrupted review");
    assert_eq!(running.status, CodingAttemptStatus::Running);
    assert_eq!(running.stage, CodingExecutionStage::CodeReview);
    assert_eq!(running.active_unit_id, repeated.blocked_attempt.active_unit_id);

    let current = fixture
        .store
        .get_failed_code_review_recovery_journal(
            &running.project_id,
            &running.issue_id,
            &running.id,
        )
        .expect("current journal")
        .expect("second recovery journal");
    assert_eq!(current.expected_gate_id, second_gate_id);
    assert_eq!(current.phase, FailedCodeReviewRecoveryPhase::GateResolved);
    assert_eq!(
        fixture
            .store
            .get_archived_failed_code_review_recovery_journal(
                &running.project_id,
                &running.issue_id,
                &running.id,
                &repeated.first_journal.expected_gate_id,
            )
            .expect("archived first journal")
            .expect("first recovery history"),
        repeated.first_journal
    );

    let runs = fixture
        .store
        .list_role_runs(&running.project_id, &running.issue_id, &running.id)
        .expect("role runs after second recovery");
    let first_retry = runs
        .iter()
        .find(|run| run.id == repeated.first_retry_role_run_id)
        .expect("first retry reviewer run");
    let second_retry = runs
        .iter()
        .find(|run| run.reason_code.as_deref() == Some(current.recovery_key.as_str()))
        .expect("second retry reviewer run");
    assert_eq!(first_retry.status, CodingRoleRunStatus::Superseded);
    assert_eq!(
        first_retry.superseded_by_run_id.as_deref(),
        Some(second_retry.id.as_str())
    );
    assert_eq!(
        second_retry.supersedes_run_id.as_deref(),
        Some(repeated.first_retry_role_run_id.as_str())
    );

    let first_gate_retry = CodingWsInMessage::GateResponse {
        gate_id: repeated.first_journal.expected_gate_id,
        action_id: "retry_review".to_string(),
        extra_context: None,
    };
    assert!(!failed_code_review_recovery_request(
        &fixture.store,
        &running,
        &first_gate_retry,
    ));
}
```

- [ ] **Step 3: 运行集成测试并确认 RED**

运行：

```bash
cargo test --locked --lib completed_journal_rotates_when_later_review_is_interrupted
```

Expected: FAIL 于 `second recovery identity`，证明旧 completed journal 仍然遮蔽当前第二个 Gate；不得先修改 fixture 绕开 current journal。

- [ ] **Step 4: 修改只读恢复识别的 completed 分支**

把 `recoverable_failed_code_review` 当前 journal 分支替换为：

```rust
if let Some(journal) = coding_store.get_failed_code_review_recovery_journal(
    &attempt.project_id,
    &attempt.issue_id,
    &attempt.id,
)? {
    if !journal.is_completed() {
        if !journal_recovery_prefix_is_valid(coding_store, attempt, &journal)? {
            return Ok(None);
        }
        return Ok(Some(FailedCodeReviewRecovery {
            gate_id: journal.expected_gate_id,
            failed_node_id: journal.expected_failed_node_id,
            stale_role_run_id: journal.expected_stale_role_run_id,
        }));
    }
    if completed_journal_waits_for_retry_node(coding_store, attempt, &journal)? {
        return Ok(Some(FailedCodeReviewRecovery {
            gate_id: journal.expected_gate_id,
            failed_node_id: journal.expected_failed_node_id,
            stale_role_run_id: journal.expected_stale_role_run_id,
        }));
    }
}
```

该分支结束后必须继续执行现有 blocked/failed Attempt 严格识别，不允许在只读函数中移动 journal。

- [ ] **Step 5: 让 Engine 始终按精确 identity 调用 prepare-or-rotate**

把 `recover_failed_code_review_for_attempt` 从 `let mut journal = if let Some(existing)` 到对应 `else` 结束的代码替换为：

```rust
let Some(recovery) = recoverable_failed_code_review(&self.store, &current)? else {
    return Err(recovery_state_changed());
};
if recovery.gate_id != gate_id {
    return Err(recovery_state_changed());
}
let mut journal = self.store.prepare_failed_code_review_recovery_journal(
    &current,
    &recovery.gate_id,
    &recovery.failed_node_id,
    &recovery.stale_role_run_id,
)?;
```

后续重新加载 Attempt、验证 journal expected IDs、reopen/blocked-to-running、Role Run supersede 和 Gate resolve 逻辑保持不变。

- [ ] **Step 6: 运行第二次中断测试并确认 GREEN**

运行：

```bash
cargo test --locked --lib completed_journal_rotates_when_later_review_is_interrupted
```

Expected: PASS；旧 journal 可从历史 getter 读取，当前 journal 指向第二个 Gate，第一 Retry Reviewer 被第二 Retry Reviewer supersede。

- [ ] **Step 7: 运行全部 failed review recovery 测试**

运行：

```bash
cargo test --locked --lib failed_review_recovery
```

Expected: 现有历史 failed Attempt、blocked Provider 中断、journal phase、Runner 交接和普通 Gate 防线测试全部通过。

- [ ] **Step 8: 提交 Engine 重复恢复实现**

```bash
git add \
  src/product/coding_workspace_engine/failed_review_recovery.rs \
  src/web/coding_ws_handler/tests/failed_review_recovery.rs \
  src/web/coding_ws_handler/tests/failed_review_recovery/support.rs
git commit -m "fix: recover repeated interrupted code reviews"
```

Expected: 提交不包含 runner 并发测试和用户未提交文件。

---

### Task 3: 覆盖第二次 Review 中断的 WebSocket 并发与重复点击

**Files:**

- Modify: `src/web/coding_ws_handler/tests/failed_review_recovery/runner.rs:329-390`

**Interfaces:**

- Consumes: Task 2 的 `seed_repeated_interrupted_review`、Attempt 级 `CodingRunRegistry::try_reserve_attempt` 和 Store archived journal getter。
- Produces: 两个 socket 对第二次 Gate 并发重试时只能产生一个新 recovery journal、一个第二次 Retry Reviewer Run 和一个 Runner 的回归证据。

- [ ] **Step 1: 导入重复中断 fixture helper**

把 runner 测试的 support import 调整为：

```rust
use super::support::{
    FixtureCase, failed_review_fixture, seed_repeated_interrupted_review,
};
```

- [ ] **Step 2: 添加第二次中断的并发测试**

在现有 `two_blocked_review_retry_sockets_converge_to_one_retry_run_and_runner` 后加入：

```rust
#[tokio::test]
async fn two_repeated_review_retry_sockets_converge_to_one_current_run_and_runner() {
    let fixture = failed_review_fixture(
        CodingAttemptScope::WorkItemGroup,
        FixtureCase::BlockedProviderInterrupted,
    );
    let repeated = seed_repeated_interrupted_review(&fixture).await;
    let gate_id = repeated.second_gate.gate_id.clone();
    let retry = CodingWsInMessage::GateResponse {
        gate_id: gate_id.clone(),
        action_id: "retry_review".to_string(),
        extra_context: None,
    };
    assert!(failed_code_review_recovery_request(
        &fixture.store,
        &repeated.blocked_attempt,
        &retry,
    ));

    let registry = Arc::new(CodingRunRegistry::default());
    let barrier = Arc::new(Barrier::new(3));
    let attempts = (0..2)
        .map(|_| {
            let registry = Arc::clone(&registry);
            let barrier = Arc::clone(&barrier);
            let attempt_id = repeated.blocked_attempt.id.clone();
            thread::spawn(move || {
                barrier.wait();
                registry.try_reserve_attempt(&attempt_id)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let mut reservations = attempts
        .into_iter()
        .filter_map(|attempt| attempt.join().expect("socket reservation"))
        .collect::<Vec<_>>();
    assert_eq!(reservations.len(), 1);

    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        event_tx,
    );
    let updated = engine
        .recover_failed_code_review_for_attempt(&repeated.blocked_attempt.id, &gate_id)
        .await
        .expect("winning socket recovers second interrupted review");
    let (command_tx, _command_rx) = mpsc::channel(1);
    let run_id = reservations
        .pop()
        .expect("winning reservation")
        .activate(command_tx)
        .expect("activate winning runner");
    let current = fixture
        .store
        .complete_failed_code_review_recovery_journal(&updated.id, &gate_id)
        .expect("complete second recovery journal");

    assert_eq!(registry.runner_count(&updated.id), 1);
    assert_eq!(current.expected_gate_id, gate_id);
    assert_eq!(
        fixture
            .store
            .get_archived_failed_code_review_recovery_journal(
                &updated.project_id,
                &updated.issue_id,
                &updated.id,
                &repeated.first_journal.expected_gate_id,
            )
            .expect("archived first journal")
            .expect("first recovery history"),
        repeated.first_journal
    );
    let runs = fixture
        .store
        .list_role_runs(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("role runs after concurrent second retry");
    assert_eq!(
        runs.iter()
            .filter(|run| run.trigger
                == crate::product::coding_models::CodingRoleRunTrigger::RetryReview)
            .count(),
        2
    );
    assert_eq!(
        runs.iter()
            .filter(|run| run.reason_code.as_deref() == Some(current.recovery_key.as_str()))
            .count(),
        1
    );
    registry.remove(&updated.id, run_id);
}
```

- [ ] **Step 3: 运行并发测试**

运行：

```bash
cargo test --locked --lib two_repeated_review_retry_sockets_converge_to_one_current_run_and_runner
```

Expected: PASS；两个 reservation 竞争者中只有一个成功，历史归档存在，第二次 recovery key 只对应一个 Role Run。

- [ ] **Step 4: 运行普通路径和 reservation 回归**

运行：

```bash
cargo test --locked --lib blocked_review_retry
cargo test --locked --lib failed_review_recovery_reservation
```

Expected: 普通 Gate 仍不能绕过 reservation；旧 Runner、重复 socket、abort/context note 竞争测试保持通过。

- [ ] **Step 5: 提交并发回归测试**

```bash
git add src/web/coding_ws_handler/tests/failed_review_recovery/runner.rs
git commit -m "test: cover repeated review retry concurrency"
```

Expected: 提交只包含 runner 测试。

---

### Task 4: 执行源码质量门禁与全量 Rust 验证

**Files:**

- Verify: `src/product/coding_attempt_store/recovery.rs`
- Verify: `src/product/coding_workspace_engine/failed_review_recovery.rs`
- Verify: `src/web/coding_ws_handler/tests/failed_review_recovery/**`
- Preserve: `.superpowers/sdd/final-review-fix-report.md`

**Interfaces:**

- Consumes: Tasks 1–3 的三个原子提交。
- Produces: 代码格式、Clippy、编译和全部 Rust 测试通过的证据；确认无前端、Playwright 或 E2E 内容。

- [ ] **Step 1: 应用并检查 Rust 格式**

```bash
cargo fmt
cargo fmt --check
```

Expected: `cargo fmt --check` 退出码为 0；若 `cargo fmt` 产生改动，只允许出现在 Tasks 1–3 的 Rust 文件。

- [ ] **Step 2: 运行 Clippy**

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: 退出码为 0，无 warning。

- [ ] **Step 3: 运行 Cargo check**

```bash
cargo check --locked
```

Expected: 退出码为 0。

- [ ] **Step 4: 运行全部 Rust 测试**

```bash
cargo test --locked
```

Expected: 全部测试通过，无失败或 ignored-as-failure 项。

- [ ] **Step 5: 确认没有 E2E、Playwright、前端依赖或配置变化**

```bash
git diff --name-only ea3f382..HEAD | rg '(^web/|playwright|e2e|package.json|pnpm-lock.yaml)' || true
git status --short
```

Expected: 第一条无输出；`git status --short` 只允许显示预先存在的 `.superpowers/sdd/final-review-fix-report.md`，或尚未提交的本轮 Rust 格式化修正。

- [ ] **Step 6: 提交剩余的纯格式修正**

若 `cargo fmt` 没有产生未提交改动，跳过本步骤。否则只暂存本轮 Rust 文件：

```bash
git add \
  src/product/coding_attempt_store/recovery.rs \
  src/product/coding_attempt_store/tests.rs \
  src/product/coding_attempt_store/tests/failed_review_recovery.rs \
  src/product/coding_workspace_engine/failed_review_recovery.rs \
  src/web/coding_ws_handler/tests/failed_review_recovery.rs \
  src/web/coding_ws_handler/tests/failed_review_recovery/support.rs \
  src/web/coding_ws_handler/tests/failed_review_recovery/runner.rs
git commit -m "style: format repeated review recovery changes"
```

Expected: 不暂存用户文件；若没有格式差异则没有新提交。

---

### Task 5: 备份并修正当前 coding_attempt_0001 的旧 journal

**Files:**

- Runtime source: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/failed-code-review-recovery.json`
- Runtime destination: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/failed-code-review-recoveries/completed/coding_blocked_gate_0001.json`
- Verify: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json`
- Verify: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/units/coding_unit_0007.json`
- Verify: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/timeline-nodes.json`
- Verify: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/role-runs/coding_role_run_0029.json`
- Verify: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/blocked-gates/coding_blocked_gate_0007.json`
- Read-only: `/home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001`

**Interfaces:**

- Preserves: Attempt `blocked + code_review`、`coding_unit_0007` running、`coding_node_0030` failed、`coding_role_run_0029` failed、`coding_blocked_gate_0007` open、Work Item 7 Coder diff。
- Archives: completed recovery identity `coding_blocked_gate_0001 / coding_node_0009 / coding_role_run_0008`。
- Produces: 当前 journal 槽位为空，等待修复后的平台在用户点击“重试代码审查”时创建 Gate 0007 的新 journal。

- [ ] **Step 1: 重启并确认后端加载 feat-b-0709 最新代码**

先读取当前运行信息：

```bash
EXPECTED_SHA="$(git rev-parse --short=12 HEAD)"
curl -fsS http://127.0.0.1:4317/api/runtime-info | jq \
  --arg expected "$EXPECTED_SHA" \
  '{branch,git_sha,workspace_root,expected_sha:$expected}'
```

若 `git_sha` 不等于 `EXPECTED_SHA`，只停止 4317 后端进程，不停止 5173 前端：

```bash
BACKEND_PID="$(ps -eo pid=,cmd= | rg 'target/debug/aria web --workspace \. --host 127\.0\.0\.1 --port 4317' | awk 'NR==1 {print $1}')"
test -n "$BACKEND_PID"
kill "$BACKEND_PID"
cargo run --locked -- web --workspace . --host 127.0.0.1 --port 4317
```

最后一条命令在持续运行的终端会话中启动后端。等待 `/api/health` 恢复后重新检查：

```bash
EXPECTED_SHA="$(git rev-parse --short=12 HEAD)"
curl -fsS http://127.0.0.1:4317/api/health
curl -fsS http://127.0.0.1:4317/api/runtime-info | jq -e \
  --arg expected "$EXPECTED_SHA" \
  '.branch == "feat-b-0709" and .git_sha == $expected'
```

Expected: 后端 branch 为 `feat-b-0709`，`git_sha` 等于当前 HEAD 的 12 位短 SHA；前端进程未重启。

- [ ] **Step 2: 确认 Attempt 没有活跃 Reviewer/Runner**

```bash
BASE=.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001
ps -eo pid,etime,cmd | rg 'aria|claude|codex' | rg -v 'rg '
jq '{status,stage,current_work_item_id,active_unit_id,updated_at}' \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json
jq -c 'select(.status=="running") | {id,stage,role,node_id,started_at}' \
  "$BASE"/role-runs/*.json
```

Expected: Attempt 为 `blocked + code_review`，active Unit 为 `coding_unit_0007`；没有当前 `running` Reviewer Role Run。若存在真实活跃 Runner 或 Provider 进程，停止数据操作并等待其结束。

- [ ] **Step 3: 校验旧 journal 和当前第二次中断 identity**

```bash
jq -e '
  .attempt_id == "coding_attempt_0001" and
  .expected_gate_id == "coding_blocked_gate_0001" and
  .expected_failed_node_id == "coding_node_0009" and
  .expected_stale_role_run_id == "coding_role_run_0008" and
  .retry_role_run_id == "coding_role_run_0009" and
  .phase == "completed" and
  .runner_started_at != null and
  .completed_at != null
' "$BASE/failed-code-review-recovery.json"
jq -e '
  .status == "open" and
  .node_id == "coding_node_0030" and
  .gate.gate_id == "coding_blocked_gate_0007" and
  .gate.reason_code == "code_review_provider_interrupted" and
  .gate.stage == "code_review" and
  .gate.role == "code_reviewer"
' "$BASE/blocked-gates/coding_blocked_gate_0007.json"
jq -e '
  .id == "coding_role_run_0029" and
  .status == "failed" and
  .stage == "code_review" and
  .role == "code_reviewer" and
  .node_id == "coding_node_0030" and
  .reason_code == "code_review_provider_interrupted"
' "$BASE/role-runs/coding_role_run_0029.json"
```

Expected: 三条 `jq -e` 均退出 0；任何 ID 或状态不匹配都停止，不根据“最新文件”重新猜测 identity。

- [ ] **Step 4: 记录业务 worktree 指纹**

在 `/home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001` 执行：

```bash
git rev-parse HEAD
git status --short -uall
git diff --stat
git diff --binary | sha256sum
```

Expected: HEAD 为 `76428a098fc740d698f705892389cdf075645a14`；保存 status、stat 和 binary diff 哈希。若 HEAD 已变化，停止并重新核对当前 Attempt，而不是覆盖业务 worktree。

- [ ] **Step 5: 完整备份 Attempt 数据**

```bash
BACKUP="/tmp/cadence-aria-coding_attempt_0001-before-journal-rotation-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$BACKUP"
cp -a \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001 \
  "$BACKUP"/
sha256sum \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json \
  "$BASE/failed-code-review-recovery.json" \
  "$BASE/timeline-nodes.json" \
  "$BASE/role-runs/coding_role_run_0029.json" \
  "$BASE/blocked-gates/coding_blocked_gate_0007.json" \
  "$BASE/units/coding_unit_0007.json" \
  > "$BACKUP/pre-rotation-sha256.txt"
printf '%s\n' "$BACKUP"
```

Expected: 输出唯一的 UTC 时间戳备份目录；备份内同时包含顶层 Attempt JSON、完整明细目录和 `pre-rotation-sha256.txt`。

- [ ] **Step 6: 原子归档旧 completed journal**

```bash
ARCHIVE="$BASE/failed-code-review-recoveries/completed/coding_blocked_gate_0001.json"
test ! -e "$ARCHIVE"
mkdir -p "$(dirname "$ARCHIVE")"
mv "$BASE/failed-code-review-recovery.json" "$ARCHIVE"
```

Expected: current journal 路径不存在；archive 路径存在。若 archive 已存在，停止并比较内容，不覆盖或删除任一文件。

- [ ] **Step 7: 校验只发生 journal 路径移动**

```bash
test ! -e "$BASE/failed-code-review-recovery.json"
cmp \
  "$ARCHIVE" \
  "$BACKUP/coding_attempt_0001/failed-code-review-recovery.json"
jq -e '
  .expected_gate_id == "coding_blocked_gate_0001" and
  .expected_failed_node_id == "coding_node_0009" and
  .expected_stale_role_run_id == "coding_role_run_0008" and
  .phase == "completed"
' "$ARCHIVE"
sha256sum -c "$BACKUP/pre-rotation-sha256.txt" \
  --ignore-missing
jq -e '
  .status == "blocked" and
  .stage == "code_review" and
  .current_work_item_id == "work_item_compile_20260712024139064_007" and
  .active_unit_id == "coding_unit_0007" and
  .completed_at == null
' .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json
jq -e '
  .id == "coding_unit_0007" and
  .status == "running" and
  .completed_at == null and
  .handoff_ref == null and
  .completion_commit == null
' "$BASE/units/coding_unit_0007.json"
jq -e '
  any(.[];
    .id == "coding_node_0030" and
    .stage == "code_review" and
    .status == "failed"
  )
' "$BASE/timeline-nodes.json"
```

Expected: 所有非 journal 文件哈希保持一致；Attempt、Unit 和失败 Review Node 状态不变。

- [ ] **Step 8: 重新比对业务 worktree 指纹**

在 `/home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001` 再次执行：

```bash
git rev-parse HEAD
git status --short -uall
git diff --stat
git diff --binary | sha256sum
```

Expected: HEAD、status、stat 和 binary diff 哈希与 Step 4 完全相同；不允许执行 `git restore`、`git stash`、`git reset` 或提交。

- [ ] **Step 9: 只读确认页面恢复前置状态**

```bash
curl -fsS http://127.0.0.1:4317/api/health
jq '{status,stage,current_work_item_id,active_unit_id}' \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json
```

Expected: 后端健康；Attempt 仍是 `blocked + code_review`。此时不代替用户点击 `retry_review`，不提前创建 Role Run 或关闭 Gate。

---

### Task 6: 最终核对、推送并交付页面操作步骤

**Files:**

- Verify: all commits after `ea3f382`
- Preserve: `.superpowers/sdd/final-review-fix-report.md`
- Push: `origin/feat-b-0709`

**Interfaces:**

- Consumes: 通过全部 Rust 门禁的源码、已备份且完成窄范围 journal 归档的当前 Attempt。
- Produces: 远端可拉取的 `feat-b-0709`、数据修复证据和用户刷新后点击“重试代码审查”的明确交付说明。

- [ ] **Step 1: 查看提交与工作区状态**

```bash
git log --oneline ea3f382..HEAD
git status --short
git diff --name-only ea3f382..HEAD
```

Expected: 只包含计划文档、Store、Engine 和 failed review recovery 测试；工作区只保留用户原有 `.superpowers/sdd/final-review-fix-report.md` 修改。

- [ ] **Step 2: 重新运行关键定向测试作为推送前快检**

```bash
cargo test --locked --lib coding_attempt_store::tests::failed_review_recovery
cargo test --locked --lib completed_journal_rotates_when_later_review_is_interrupted
cargo test --locked --lib two_repeated_review_retry_sockets_converge_to_one_current_run_and_runner
```

Expected: 三组测试全部通过。

- [ ] **Step 3: 推送 feat-b-0709**

```bash
git push origin feat-b-0709
```

Expected: `origin/feat-b-0709` 指向本轮最终源码提交。

- [ ] **Step 4: 交付用户页面操作**

向用户明确说明：

```text
1. 后端必须运行 feat-b-0709 最新代码；若自动重载未生效，先重启后端。
2. 刷新 Coding Attempt #coding_attempt_0001 页面。
3. 当前应继续显示 Work Item 7 的“重试代码审查”。
4. 只点击一次“重试代码审查”。
5. 平台应创建新的 Reviewer Run，不重跑 Coder，不修改 Work Item 7 当前代码。
6. 若再次出现错误，立即保留页面错误码，不反复点击；检查 current journal、Gate 0007 和新 Role Run identity。
```

- [ ] **Step 5: 汇报验证与数据安全结果**

最终汇报必须包含：

```text
- 源码提交和远端分支位置；
- Store、Engine、并发定向测试结果；
- fmt、clippy、check、全量 cargo test 结果；
- 当前 Attempt 备份目录；
- 旧 journal 归档路径；
- Attempt/Unit/Gate/Node/Role Run 状态未被改写的证据；
- 业务 worktree HEAD 和 binary diff 哈希前后一致；
- 未新增或运行 Playwright/E2E；
- 用户未提交文件未被暂存或修改。
```
