# Coding Workspace 精简 Plan 1：摘除 Testing 阶段与 analyst 调用

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 从 coding workspace 执行编排中删除 Testing 阶段和所有 analyst rework 调用，使新链路为 `Coding → CodeReview → ReviewRequest → InternalPrReview → FinalConfirm`，编译通过，现有测试全绿。

**Architecture:** 主编排在 `runner.rs` 的 `execute_start_coding_flow` 中，一个 `'pipeline` 循环串行推进各阶段。本 plan 只做删除：去掉 Testing 分支（约 226–364 行）以及每个执行阶段后插入的 `execute_rework_with_commands`（analyst）调用，并清理因此产生的孤儿辅助函数。阶段枚举和前端类型字段保留，不破坏历史数据兼容性。

**Tech Stack:** Rust（edition 2024，cargo check/clippy/test），前端暂不涉及。

## Global Constraints

- 禁止 `cargo test -j 1`；单测定向用 `cargo test --locked --lib <过滤名>`
- `CodingExecutionStage::Testing` 和 `CodingExecutionStage::Rework` 枚举变体**保留不删**（向后兼容）
- `tester_plan_provider`/`tester_execute_provider` 字段保留不删
- analyst 相关存储函数（`save_analyst_decision`、`save_analyst_evidence` 等）保留不删（其他代码路径可能引用）
- 本 plan 不改 `rework.rs` 内部实现，只删除对它的调用入口

---

### Task 1：删除 runner.rs 中的 Testing 分支

**Files:**
- Modify: `src/web/coding_ws_handler/runner.rs:226-364`

**Interfaces:**
- Consumes: 当前 `execute_start_coding_flow` 函数（`runner.rs:93`）
- Produces: 同函数，Testing 分支不再存在；Coding 完成后（`runner.rs:224` 之后）直接落到 CodeReview 段

- [x] **Step 1: 确认当前 Testing 分支范围**

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0630
sed -n '220,370p' src/web/coding_ws_handler/runner.rs
```

预期：能看到 `if current.stage.order() <= CodingExecutionStage::Testing.order()` 开始的整块（约 226–364）。

- [x] **Step 2: 删除 Testing 分支**

删除 `src/web/coding_ws_handler/runner.rs` 中以下完整代码块（行范围以 Step 1 确认为准）：

```rust
        if current.stage.order() <= CodingExecutionStage::Testing.order() {
            // ... 整块约 139 行，含 await_stage_gate、execute_testing_with_distinct_provider_commands、
            //     create_testing_result_review_gate、testing_report_should_enter_analyst、
            //     execute_rework_with_commands（testing→analyst）等
        }
```

删除后，Coding 段结束（`handle_pending_runner_commands` 之后）直接跟 `if current.stage == CodingExecutionStage::InternalPrReview` 分支（原 366 行）。

- [x] **Step 3: 删除 runner.rs 顶部 Testing 分支依赖（testing_result_acceptance_pending_analyst）**

`runner.rs:145` 附近有：

```rust
        if current.stage == CodingExecutionStage::Rework
            || testing_result_acceptance_pending_analyst(coding_store, &current)?
        {
```

改为只保留 Rework 分支触发（`testing_result_acceptance_pending_analyst` 的调用删掉即可，后续 Task 2 再整体删掉这段）：

```rust
        if current.stage == CodingExecutionStage::Rework {
```

> 注：`testing_result_acceptance_pending_analyst` 函数本体在 `runner_support.rs`，Task 2 删除。

- [x] **Step 4: 编译确认**

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0630
cargo check --locked 2>&1 | head -60
```

预期：0 errors（可能有 unused import 警告，先不管）。

- [x] **Step 5: 提交**

```bash
git add src/web/coding_ws_handler/runner.rs
git commit -m "refactor(coding): remove Testing stage from pipeline"
```

---

### Task 2：删除 runner.rs 中所有 analyst rework 调用及 runner_support 孤儿函数

**Files:**
- Modify: `src/web/coding_ws_handler/runner.rs`（删除三处 `execute_rework_with_commands` 调用：原 Testing 后、CodeReview 后、InternalPrReview 后）
- Modify: `src/web/coding_ws_handler/runner_support.rs`（删除 `testing_result_acceptance_pending_analyst`、`latest_analyst_role_run_evidence`）

**Interfaces:**
- Consumes: Task 1 完成后的 runner.rs
- Produces: runner.rs 中无任何 `execute_rework_with_commands` 调用；runner_support.rs 只保留 `handle_pending_runner_commands`、`provider_for`

- [x] **Step 1: 定位并删除 CodeReview 后的 analyst rework 段**

在 runner.rs 中，`execute_code_review_with_commands` 之后紧跟：

```rust
            let Some(next) = await_stage_gate(
                &mut command_rx,
                coding_store,
                engine,
                event_tx,
                &current,
                CodingExecutionStage::Rework,
            )
            .await?
            // ... analyst_provider 获取、execute_rework_with_commands 调用 ...
            match current.stage {
                CodingExecutionStage::Coding
                | CodingExecutionStage::Testing
                | CodingExecutionStage::CodeReview => continue 'pipeline,
                CodingExecutionStage::ReviewRequest => {}
                _ => return emit_current_session_state(event_tx, coding_store, &current).await,
            }
```

删除从 `await_stage_gate(... CodingExecutionStage::Rework ...)` 到 `match current.stage { ... }` 这整块，替换为：

```rust
            // reviewer approve → 推进到 ReviewRequest
            match current.stage {
                CodingExecutionStage::Coding
                | CodingExecutionStage::Testing
                | CodingExecutionStage::CodeReview => continue 'pipeline,
                CodingExecutionStage::ReviewRequest => 
                _ => return emit_current_session_state(event_tx, coding_store, &current).await,
            }
```

> 注意：上面的 `match` 保留，只删掉中间的 analyst rework 段。review_report 本身不再需要传给 rework，不用存。

- [x] **Step 2: 删除 InternalPrReview 后的 analyst rework 段（两处）**

runner.rs 中有两处 `InternalPrReview` 后接 analyst rework 的结构（一处在约 `366–462` 行的 early `InternalPrReview` 分支，一处在 `620–715` 行的后段）。

两处均删除 `await_stage_gate(... CodingExecutionStage::Rework ...)` 到 `match current.stage { ... }` 段，替换逻辑：

```rust
            // internal review 完成 → approve 继续；其他停下
            match current.stage {
                CodingExecutionStage::Coding => continue 'pipeline,
                CodingExecutionStage::FinalConfirm => {
                    return emit_current_session_state(event_tx, coding_store, &current).await;
                }
                _ => return emit_current_session_state(event_tx, coding_store, &current).await,
            }
```

- [x] **Step 3: 删除 runner.rs 顶部 Rework 分支整块**

`runner.rs:143` 附近现在只剩：

```rust
        if current.stage == CodingExecutionStage::Rework {
            // execute_rework_with_commands ...
        }
```

Task 1 Step 3 已改成只判断 `Rework` 阶段。这个分支是"恢复中途断掉的 analyst"的逻辑，analyst 已移除，整块删除：

```rust
        // 删除以下整块
        if current.stage == CodingExecutionStage::Rework {
            let analyst_provider_name = ...;
            ...
            continue 'pipeline;
        }
```

- [x] **Step 4: 清理 runner_support.rs 孤儿函数**

删除 `src/web/coding_ws_handler/runner_support.rs` 中的两个函数：

```rust
// 删除整个函数（约 20 行）
pub(super) fn latest_analyst_role_run_evidence(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<String, CodingWorkspaceEngineError> {
    ...
}

// 删除整个函数（约 25 行）
pub(super) fn testing_result_acceptance_pending_analyst(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<bool, CodingWorkspaceEngineError> {
    ...
}
```

同时删除文件顶部不再需要的 import：

```rust
// 删除
use crate::product::coding_models::{
    ..., CodingExecutionStage, CodingProviderRole, CodingRoleRunStatus,  // 这几项如果只给删掉的函数用，则删掉
};
```

（保留 `handle_pending_runner_commands` 和 `provider_for` 所需的 import）

- [x] **Step 5: 编译确认**

```bash
cargo check --locked 2>&1 | head -60
```

预期：0 errors。若有 unused import 警告，用 `cargo fix --allow-dirty` 或手动删除。

- [x] **Step 6: 提交**

```bash
git add src/web/coding_ws_handler/runner.rs \
        src/web/coding_ws_handler/runner_support.rs
git commit -m "refactor(coding): remove analyst rework calls from pipeline"
```

---

### Task 3：清理 analyst_parser.rs 中的 RerunTesting 与 Testing 映射

**Files:**
- Modify: `src/product/coding_workspace_engine/analyst_parser.rs`

**Interfaces:**
- Consumes: 现有 `analyst_parser.rs`；`AnalystProviderVerdict`、`default_next_stage_for_legacy_verdict`
- Produces: `RerunTesting` variant 从 `AnalystProviderVerdict` 中移除；`Testing → CodeReview` 映射移除

> **背景**：analyst_parser.rs 依然会被 `rework.rs` 内部用到（本 plan 不动 rework.rs 内部），但 runner 不再调用 rework。清理这里是为防止将来误用并消除死代码警告。

- [x] **Step 1: 从 AnalystProviderVerdict 中删除 RerunTesting**

```rust
// 修改前
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnalystProviderVerdict {
    NeedsFix,
    NeedsHumanInput,
    NoIssue,
    RerunTesting,   // ← 删除这行
    Proceed,
    HumanRequired,
    Blocked,
}
```

同时删除其 `structured()` 方法中对应 arm：

```rust
// 修改前的 structured() 中删除：
Self::RerunTesting => AnalystDecisionVerdict::RerunTesting,
```

- [x] **Step 2: 从 default_next_stage_for_legacy_verdict 中删除 Testing 映射**

```rust
// 修改前
AnalystDecisionVerdict::Proceed => match source_stage {
    CodingExecutionStage::Testing => AnalystDecisionNextStage::CodeReview,  // ← 删除
    CodingExecutionStage::CodeReview => AnalystDecisionNextStage::ReviewRequest,
    CodingExecutionStage::InternalPrReview => AnalystDecisionNextStage::FinalConfirm,
    _ => AnalystDecisionNextStage::CodeReview,
},
```

删除 `Testing =>` 这一 arm（`_ => CodeReview` 已经是兜底）。

- [x] **Step 3: 检查 AnalystDecisionVerdict::RerunTesting 是否还有其他使用方**

```bash
grep -rn "RerunTesting" src/
```

预期：除了 `analyst_parser.rs` 本身和可能的 `coding_models/analyst.rs` 定义外，不应有其他调用。如有，逐一确认是否可删。

- [x] **Step 4: 从 coding_models/analyst.rs 中删除 RerunTesting variant**

```bash
grep -n "RerunTesting" src/product/coding_models/analyst.rs
```

找到 `AnalystDecisionVerdict::RerunTesting` 定义，删除该 variant（该枚举用于持久化记录，保留其他 variant）。

- [x] **Step 5: 编译确认**

```bash
cargo check --locked 2>&1 | head -60
```

预期：0 errors。

- [x] **Step 6: 提交**

```bash
git add src/product/coding_workspace_engine/analyst_parser.rs \
        src/product/coding_models/analyst.rs
git commit -m "refactor(coding): remove RerunTesting verdict and Testing→CodeReview mapping"
```

---

### Task 4：运行测试确认绿色基线

**Files:**
- 无新增修改；只运行测试

**Interfaces:**
- Consumes: Task 1–3 完成后的代码库

- [x] **Step 1: 运行完整测试套件**

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0630
cargo test --locked 2>&1 | tail -40
```

预期：所有测试通过。若有失败，查看具体 test 名：

```bash
cargo test --locked 2>&1 | grep -E "FAILED|error\[" | head -20
```

- [x] **Step 2: 针对 coding workspace 相关测试定向跑**

```bash
cargo test --locked --test it_web web_coding_ws_handler 2>&1 | tail -30
cargo test --locked --test it_product product_coding_workspace_engine 2>&1 | tail -30
```

- [x] **Step 3: 修复因删除分支导致的测试断言失败**

常见情况：

- 测试中 mock 了 tester provider 并断言它被调用 → 删除相关 mock 注册和断言
- 测试断言执行了 Testing 阶段 timeline node → 删除对 `Testing` stage node 的断言
- 测试断言了 analyst decision 被写入 → 删除对应 `assert` 或改为不断言 analyst

修复原则：测试目标是验证 **Coding → CodeReview** 链路的正确性，不应再验证已不存在的 Testing/analyst 阶段。

- [x] **Step 4: 再次运行确认全绿**

```bash
cargo test --locked 2>&1 | grep -E "test result|FAILED"
```

预期：`test result: ok. N passed; 0 failed`

- [x] **Step 5: 提交修复后的测试**

```bash
git add tests/ src/
git commit -m "test(coding): update tests for Coding→CodeReview pipeline (no Testing/analyst)"
```
