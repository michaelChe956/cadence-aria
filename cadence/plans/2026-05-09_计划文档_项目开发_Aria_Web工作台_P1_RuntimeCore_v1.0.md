# Aria Web 工作台 P1 Runtime Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立 Web 工作台所需的 runtime core：projection、selected node context、checkpoint、pending provider step、turn/node/provider/artifact/event 持久化、policy preset 和 interactive runner。

**Architecture:** P1 只做 Rust runtime core，不启动 HTTP 服务、不做前端。实现应严格复用并扩展 `src/interactive`、`src/task_run` 和现有 runtime unit，不通过 shell 包装 `aria task run`。

**Tech Stack:** Rust 1.95、serde/serde_json、tokio、tempfile、现有 `interactive`、`task_run`、`runtime_units` 模块。

---

## Design Coverage

P1 覆盖 design 中这些要求：

- `WorkspaceProjection`、`InteractionTurn`、`RuntimeCheckpoint`、`ArtifactIndexEntry`
- selected node 的 Overview、Inputs、Run、Outputs、Diff
- provider 暂停前创建 checkpoint
- confirm 后写入 provider run、turn、node run、artifacts、reports、events
- policy preset：`manual-all`、`manual-write`、`auto-review`、`non-interactive`
- 内部节点自动执行并写入 node run/event/artifact
- `aria task run --non-interactive` 不回归

## Source Tasks From Master Plan

执行以下总计划任务，保持代码片段和测试断言不变：

| Master Task | Scope |
|------|------|
| Task 1 | Web API contract DTO 中 runtime 共享类型和 `PendingProviderStepDto` |
| Task 2 | Web projection 字段 |
| Task 2.5 | selected node IO context 和 OpenSpec evidence |
| Task 3 | rollback preview 和 checkpoint boundary |
| Task 4 | pending provider step metadata |
| Task 5 | fake WebRuntime closed loop |
| Task 5.5 | runtime persistence boundaries |
| Task 14 | interactive runner seam |
| Task 14.5 | policy presets 和 automatic internal steps |

## Files

| Path | Responsibility |
|------|------|
| `src/web/types.rs` | Runtime DTO 共享类型 |
| `src/web/runtime.rs` | Web task create/advance/confirm/projection runtime |
| `src/web/runtime_store.rs` | turn/node-run/provider-run/checkpoint/pending/event 持久化 |
| `src/interactive/models.rs` | Web projection 和 pending provider step model |
| `src/interactive/web_projection.rs` | selected node context、git summary、pending step context |
| `src/interactive/projection.rs` | artifact/OpenSpec evidence index |
| `src/interactive/checkpoint.rs` | rollback preview |
| `src/interactive/controller.rs` | pending confirmation semantics |
| `src/task_run/step_runner.rs` | adapter input 到 pending provider step |
| `src/task_run/interactive_runner.rs` | incremental execution runner |
| `src/runtime_units/clarification.rs` | planning provider input seam |
| `src/runtime_units/coding.rs` | execution provider input seam |
| `src/runtime_units/final_review.rs` | final provider input seam |
| `src/lib.rs` | module export |
| `src/task_run/mod.rs` | module export |

## Tasks

### Task P1.1: Contract And Projection Models

- [ ] **Step 1: Execute master Task 1**

Use the exact test and implementation snippets from master Task 1.

Run:

```bash
cargo test --test web_types --locked
```

Expected: PASS after implementation.

- [ ] **Step 2: Execute master Task 2**

Use the exact test and implementation snippets from master Task 2.

Run:

```bash
cargo test --test web_projection --locked
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/lib.rs src/web/mod.rs src/web/types.rs src/web/error.rs src/interactive/models.rs src/interactive/mod.rs src/interactive/web_projection.rs tests/web_types.rs tests/web_projection.rs
git commit -m "feat: add aria web runtime projection contracts"
```

### Task P1.2: Selected Node IO And OpenSpec Evidence

- [ ] **Step 1: Execute master Task 2.5**

Use the exact `tests/web_node_context.rs` and implementation snippets from master Task 2.5.

Run:

```bash
cargo test --test web_node_context --test web_projection --test interactive_projection --locked
```

Expected: PASS, with selected node context containing Overview、Inputs、Run、Outputs、Diff and OpenSpec refs.

- [ ] **Step 2: Commit**

```bash
git add src/interactive/web_projection.rs src/interactive/projection.rs tests/web_node_context.rs
git commit -m "feat: add aria web selected node context"
```

### Task P1.3: Checkpoint And Pending Provider Step

- [ ] **Step 1: Execute master Task 3**

Run:

```bash
cargo test --test interactive_checkpoint --test interactive_checkpoint_preview --locked
```

Expected: PASS.

- [ ] **Step 2: Execute master Task 4**

Run:

```bash
cargo test --test interactive_controller --test task_run_step_runner --locked
cargo check --locked
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/interactive/checkpoint.rs src/interactive/controller.rs src/task_run/step_runner.rs tests/interactive_checkpoint_preview.rs tests/interactive_controller.rs tests/task_run_step_runner.rs
git commit -m "feat: add aria web checkpoint and provider metadata"
```

### Task P1.4: WebRuntime Fake Loop And Persistence

- [ ] **Step 1: Execute master Task 5**

Run:

```bash
cargo test --test web_runtime_fake --locked
```

Expected: PASS.

- [ ] **Step 2: Execute master Task 5.5**

Run:

```bash
cargo test --test web_runtime_persistence --test web_runtime_fake --test web_projection --locked
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/web/runtime.rs src/web/runtime_store.rs src/web/state.rs src/web/types.rs src/web/mod.rs tests/web_runtime_fake.rs tests/web_runtime_persistence.rs
git commit -m "feat: persist aria web runtime records"
```

### Task P1.5: Interactive Runner And Policy Presets

- [ ] **Step 1: Execute master Task 14**

Run:

```bash
cargo test --test task_run_interactive_runner --test task_run_orchestrator --locked
```

Expected: PASS and `task run --non-interactive` behavior remains compatible.

- [ ] **Step 2: Execute master Task 14.5**

Run:

```bash
cargo test --test web_policy_runtime --test interactive_policy --test interactive_controller --locked
```

Expected: PASS, matching design policy table.

- [ ] **Step 3: Commit**

```bash
git add src/task_run/interactive_runner.rs src/task_run/mod.rs src/runtime_units/clarification.rs src/runtime_units/coding.rs src/runtime_units/final_review.rs src/web/runtime.rs src/interactive/controller.rs tests/task_run_interactive_runner.rs tests/web_policy_runtime.rs
git commit -m "feat: add aria web interactive runner policy"
```

## P1 Exit Criteria

Run:

```bash
cargo test --test web_types --test web_projection --test web_node_context --test interactive_checkpoint_preview --test web_runtime_fake --test web_runtime_persistence --test task_run_interactive_runner --test web_policy_runtime --locked
```

Expected: all listed tests PASS.

## Self-Review

- [x] P1 does not implement HTTP or frontend UI.
- [x] P1 covers runtime state needed by design before P2/P3/P4/P5.
- [x] P1 keeps non-interactive task run regression in scope.
