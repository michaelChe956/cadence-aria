# Coding Workspace 完成门禁移除 TestingReport 依赖实施计划

> **面向执行 Agent：** 必须逐任务执行本计划；实施时优先调用 `superpowers:subagent-driven-development`，也可调用 `superpowers:executing-plans`。步骤使用复选框跟踪，严格遵循先失败测试、再最小实现、再验证、再提交的 TDD 顺序。

**目标：** 让 schema v2 group 与 single-attempt 在 Internal PR Review/适用 review 已通过且其他非 testing 门禁满足时完成；让 legacy group completion gate 不再要求 TestingReport，同时保持 group terminal authoritative binding 不变量。

**架构：** 不新增配置、状态或兼容分支；直接从三条 completion gate 路径移除 TestingReport 读取与 required verification result 校验，并删除两个专用私有校验函数。Testing stage、tester 配置、TestingReport 模型/存储及 `VerificationGateResultMissing` 公共错误变体继续保留；文件范围、runtime binding、handoff、unit 状态、completion commit 与共享 worktree 清洁性校验保持原样。

**技术栈：** Rust 2024、Tokio、现有 JSON Product Store、Cargo、OpenSpec CLI。

## 全局约束

- 已批准 OpenSpec：`openspec/changes/relax-completion-testing-report-gate/`。
- Testing 至少未来半年不进入产品完成标准；本 change 不引入 testing gate 开关。
- schema v2 group、legacy group、single-attempt 的 completion gate 均不得依赖 TestingReport；schema v2 group 与 single-attempt 覆盖完整 final-confirm/Completed，legacy group 只覆盖 gate-level 行为。
- 不删除或修改 Testing stage、tester 配置、TestingReport 持久化格式及历史数据。
- 保留 `VerificationGateResultMissing` 公共错误变体。
- 不改变 Internal Reviewer、final confirm、handoff、rework、review request 与状态持久化的其他行为。
- 不放宽 group terminal status 对 `CodingAttemptPlanBinding`、`WorkItemPlanLineage` 和 authoritative plan revision 的完整性要求；无 binding legacy group 继续失败关闭。
- 非 testing 门禁必须继续失败关闭：completion commit、runtime binding、changed-file scope、handoff、unit 完成状态、共享 worktree 清洁性。
- 不自动迁移或直接修改历史停滞 attempt；运行时重试或重启前必须获得用户确认。
- Rust 命令使用宿主机工具链；定向单测必须带 `--lib`，禁止任何 `cargo` 命令使用 `-j 1`。
- 当前验证基线：`it_product` 为 221/224，既有 3 个无关失败；`it_core` 为 143/144，既有 `large_file_guard` 失败。完成时不得新增基线外失败。

---

## 文件与职责

| 文件 | 本 change 的责任 |
|---|---|
| `src/product/coding_workspace_engine/tests.rs` | 为 schema v2 fixture 增加从源头注入 `VerificationCheck` 的测试入口，保持 immutable revision 语义。 |
| `src/product/coding_workspace_engine/tests/schema_v2_runtime.rs` | 覆盖 schema v2 required check 存在但没有 TestingReport 时完整 `handle_final_confirm` 返回 `Completed`、写入 `completed_at`，且 coding units 保持完成。 |
| `tests/it_product/product_coding_workspace_engine/part_08.rs` | 覆盖 single-attempt required legacy verification plan 存在但没有 TestingReport 时 final confirm 成功。 |
| `tests/it_product/product_coding_workspace_engine/part_13.rs` | 保留既有 legacy terminal binding 失败关闭回归；删除错误假设为 legacy 的 final-confirm testing 门禁测试。 |
| `src/product/coding_workspace_engine/tests/legacy_completion_gate.rs`（新增） | crate 内直接覆盖无 lineage legacy group completion gate：Blocked/no report 不阻塞且 report 原样保留。 |
| `tests/it_product/product_coding_workspace_engine/part_14.rs` | 删除改写测试后不再使用的 passed testing report fixture（若 `rg` 确认无调用）。 |
| `src/product/coding_workspace_engine/gates.rs` | 移除 single-attempt 与 legacy group 的 TestingReport 完成校验，保留所有非 testing 门禁。 |
| `src/product/coding_workspace_engine/gates/schema_v2.rs` | 删除 schema v2 required verification report 校验函数。 |
| `openspec/changes/relax-completion-testing-report-gate/tasks.md` | 实施、验证和审查完成后同步工作包状态。 |

---

### Task 1：建立三条 completion gate 的 RED 回归与 terminal invariant 保护

**文件：**

- 修改：`src/product/coding_workspace_engine/tests.rs:18-27,60-105,130-220`
- 修改：`src/product/coding_workspace_engine/tests/schema_v2_runtime.rs:1-168`
- 修改：`tests/it_product/product_coding_workspace_engine/part_08.rs:487-583`
- 修改：`tests/it_product/product_coding_workspace_engine/part_13.rs:608-756`
- 新建：`src/product/coding_workspace_engine/tests/legacy_completion_gate.rs`
- 修改：`src/product/coding_workspace_engine/tests.rs`（注册测试模块与复用所需 import）
- 修改：`tests/it_product/product_coding_workspace_engine/part_14.rs:70-111`（仅在 helper 无调用后删除）

**接口：**

- Consumes：`seed_group_attempt_fixture_with_legacy_work_items`、`VerificationCheck`、`handle_final_confirm`、`run_group_completion_gates`。
- Produces：schema v2 group 与 single-attempt 的 final-confirm RED，以及 legacy group 的 crate 内 gate-level RED；legacy 测试同时覆盖存量 Blocked report 被忽略且数据不变，且不声称无 binding legacy group 可 terminalize。

- [ ] **Step 1：最小扩展 schema v2 fixture，使 required check 在 revision 首次发布时写入**

在 `src/product/coding_workspace_engine/tests.rs` 的 `work_item_contract` import 中加入 `VerificationCheck`：

```rust
use crate::product::work_item_contract::{
    BlockerRoute, BlockerRule, CanonicalWorkItemContract, HandoffContract, PromisedOutputContract,
    VerificationCheck, WorkItemContractIdentity, WorkItemGoal, WorkItemWritePolicy,
    canonical_contract_hash,
};
```

不得新增第三个 wrapper helper。直接给既有 schema v2 fixture 增加 slice 参数：

```rust
fn seed_schema_v2_group_attempt_fixture(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    initialize_attempt: bool,
    with_dependency: bool,
    verification_checks: &[VerificationCheck],
) {
    seed_group_attempt_fixture_with_legacy_work_items(
        store,
        attempt,
        initialize_attempt,
        with_dependency,
        false,
        verification_checks,
    );
}
```

给内部 helper 增加同一参数：

```rust
fn seed_group_attempt_fixture_with_legacy_work_items(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    initialize_attempt: bool,
    with_dependency: bool,
    include_legacy_work_items: bool,
    verification_checks: &[VerificationCheck],
) {
```

`seed_group_attempt_fixture` 调用内部 helper 时追加空 slice：

```rust
seed_group_attempt_fixture_with_legacy_work_items(
    store,
    attempt,
    initialize_attempt,
    with_dependency,
    true,
    &[],
);
```

把 `CanonicalWorkItemContract` 中的空 checks 改为从源头复制：

```rust
verification_checks: verification_checks.to_vec(),
```

不得在 fixture 发布后覆盖 `VerificationPlanRevision`；`put_verification_plan_revision` 使用 immutable write，覆盖不同内容会返回 identity mismatch。修改后运行：

```bash
wc -l src/product/coding_workspace_engine/tests.rs
```

预期：不超过 800 行；若超过，优先压缩本次新增参数排版或删除本次附近冗余空行，不重构无关测试。

- [ ] **Step 2：将既有 schema v2 completion 测试升级为 required-check/no-report 完整 final-confirm 测试**

在 `src/product/coding_workspace_engine/tests/schema_v2_runtime.rs`：

1. 将测试改名：

```rust
async fn schema_v2_group_final_confirm_completes_without_testing_reports()
```

2. 在调用 seed 前创建 required check，并把原调用增加第五个参数：

```rust
let required_checks = [VerificationCheck {
    check_id: "check_unit_tests".to_string(),
    command: Some("node --test".to_string()),
    manual_instruction: None,
    required: true,
    non_zero_test_execution_required: true,
}];
seed_schema_v2_group_attempt_fixture(&store, &attempt, true, false, &required_checks);
```

3. 原测试剩余的 temp git repo、group attempt、completed unit run、projection hashes、HandoffRevision、unit completion commit、FinalConfirm 状态与“无 legacy work item”断言全部原样保留。
4. 在构造 engine 前加入：

```rust
assert!(
    store
        .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("testing reports")
        .is_empty()
);
```

5. 构造 engine 时保留 store clone，调用完整 final-confirm，并断言 attempt 已完成：

```rust
let (tx, _rx) = mpsc::channel(8);
let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
let updated = engine
    .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
    .await
    .expect("required checks must not require testing reports");

assert_eq!(updated.status, CodingAttemptStatus::Completed);
assert!(updated.completed_at.is_some());
assert!(
    store
        .list_coding_units(&updated.project_id, &updated.issue_id, &updated.id)
        .expect("units")
        .iter()
        .all(|unit| unit.status == CodingExecutionUnitStatus::Completed)
);
```

6. 不得退回仅调用 `run_group_completion_gates`；该测试必须覆盖 `handle_final_confirm -> Completed` 的完整状态流转。
7. 同文件 `schema_v2_group_handoff_ignores_legacy_work_item_records` 对 seed helper 的调用追加 `&[]`，保持原测试语义：

```rust
seed_schema_v2_group_attempt_fixture(&store, &attempt, true, false, &[]);
```

- [ ] **Step 3：运行 schema v2 RED 测试**

运行：

```bash
cargo test --locked --lib schema_v2_group_final_confirm_completes_without_testing_reports -- --nocapture
```

预期：测试 FAIL，错误链包含 `VerificationGateResultMissing`。如果失败原因是 immutable revision identity mismatch、runtime binding mismatch、handoff missing 或 worktree dirty，先修正 fixture，不得进入实现。

- [ ] **Step 4：新增 legacy group gate-level RED 测试，并保留 terminal binding 失败关闭回归**

新建 `src/product/coding_workspace_engine/tests/legacy_completion_gate.rs`。该文件位于 crate 内，可调用 `pub(crate) run_group_completion_gates`；使用无 `WorkItemPlanLineage` 的 legacy fixture，避免把 gate 测试误变成 schema v2：

```rust
use super::*;
use crate::product::lifecycle_store::CreateVerificationPlanInput;
use crate::product::models::{
    RepositoryProfileConfidence, VerificationCommand, VerificationCommandSafety,
    VerificationCommandSource, VerificationFallbackPolicy, VerificationScope,
};

fn create_required_legacy_verification_plan(
    lifecycle: &LifecycleStore,
    work_item_id: &str,
    plan_id: &str,
) {
    lifecycle
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some(plan_id.to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            repository_profile_ref: None,
            provider_run_ref: None,
            scope: VerificationScope::Unit,
            commands: vec![VerificationCommand {
                id: "unit_tests".to_string(),
                label: "Unit tests".to_string(),
                command: "cargo test --locked --lib unit".to_string(),
                cwd: ".".to_string(),
                purpose: "unit tests".to_string(),
                required: true,
                timeout_seconds: 120,
                source: VerificationCommandSource::Provider,
                safety: VerificationCommandSafety::Approved,
            }],
            manual_checks: Vec::new(),
            required_gates: vec!["unit_tests".to_string()],
            risk_notes: Vec::new(),
            confidence: RepositoryProfileConfidence::High,
            fallback_policy: VerificationFallbackPolicy::ManualGate,
        })
        .expect("create required verification plan");
}

#[tokio::test]
async fn legacy_group_completion_gates_ignore_non_passed_testing_reports() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("group attempt");
    let lifecycle = LifecycleStore::new(store.paths());

    for (index, (work_item_id, plan_id)) in [
        ("work_item_0001", "verification_plan_0001"),
        ("work_item_0002", "verification_plan_0002"),
    ]
    .into_iter()
    .enumerate()
    {
        create_required_legacy_verification_plan(&lifecycle, work_item_id, plan_id);
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.to_string()),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                repository_id: "repository_0001".to_string(),
                title: work_item_id.to_string(),
                verification_plan_ref: Some(plan_id.to_string()),
                ..Default::default()
            })
            .expect("legacy work item");
        let unit = store
            .create_coding_unit(CreateCodingExecutionUnitInput {
                attempt_id: attempt.id.clone(),
                project_id: attempt.project_id.clone(),
                issue_id: attempt.issue_id.clone(),
                plan_id: "work_item_plan_0001".to_string(),
                logical_work_item_id: work_item_id.to_string(),
                work_item_revision_id: format!("work_item_revision_{:04}", index + 1),
                dependency_logical_work_item_ids: Vec::new(),
                order_index: index as u32,
                status: CodingExecutionUnitStatus::Completed,
            })
            .expect("completed coding unit");
        store
            .save_coding_unit_handoff(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                &WorkItemHandoff {
                    id: format!("work_item_handoff_{:04}", index + 1),
                    project_id: attempt.project_id.clone(),
                    issue_id: attempt.issue_id.clone(),
                    work_item_id: work_item_id.to_string(),
                    attempt_id: attempt.id.clone(),
                    provider_run_ref: None,
                    summary: format!("handoff for {work_item_id}"),
                    files_changed: Vec::new(),
                    commit_sha: Some("deadbeef".to_string()),
                    diff_summary: String::new(),
                    tests_run: Vec::new(),
                    test_result_summary: String::new(),
                    review_summary: None,
                    api_or_contract_changes: Vec::new(),
                    open_risks: Vec::new(),
                    next_work_item_notes: Vec::new(),
                    created_at: "2026-07-27T00:00:00Z".to_string(),
                },
            )
            .expect("legacy handoff");
        store
            .update_coding_unit_completion_commit(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &unit.id,
                Some("deadbeef".to_string()),
            )
            .expect("completion commit");
    }

    let attempt = store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some("deadbeef".to_string()),
        )
        .expect("attempt head commit");
    assert!(
        WorkItemRevisionStore::new(store.paths())
            .get_plan_lineage(
                &attempt.project_id,
                &attempt.issue_id,
                "work_item_plan_0001",
            )
            .is_err()
    );

    let mut blocked_report = blocked_report_with(Vec::new(), Vec::new());
    blocked_report.id = "testing_report_blocked".to_string();
    blocked_report.attempt_id = attempt.id.clone();
    blocked_report.plan_id = Some("verification_plan_0001".to_string());
    store
        .save_testing_report(&attempt, &blocked_report)
        .expect("blocked testing report");

    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    engine
        .run_group_completion_gates(&attempt)
        .await
        .expect("legacy completion gate must ignore testing report status");

    assert_eq!(
        store
            .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("testing reports"),
        vec![blocked_report]
    );
}
```

在 `src/product/coding_workspace_engine/tests.rs` 模块列表中注册：

```rust
mod legacy_completion_gate;
```

把 `tests/it_product/product_coding_workspace_engine/part_13.rs` 中 Task 1 新增的 `group_final_confirm_completes_without_passed_testing_reports_for_required_plans` 改为 terminal invariant 回归 `group_final_confirm_without_authoritative_plan_binding_fails_closed`：保留 completed unit、handoff、FinalConfirm、head commit 与 lock setup；删除 required verification plan 和 Blocked report 数据；结尾断言：

```rust
let error = engine
    .handle_final_confirm(&attempt.project_id, &attempt.issue_id, &attempt.id)
    .await
    .expect_err("legacy group without authoritative binding must fail closed");
assert!(error.to_string().contains("coding_attempt_plan_binding"));

let persisted = store
    .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
    .expect("persisted attempt");
assert_eq!(persisted.status, CodingAttemptStatus::WaitingForHuman);
assert!(persisted.completed_at.is_none());
```

该测试不验证 TestingReport gate；它只证明本 change 没有放宽 group terminal binding 不变量。

- [ ] **Step 5：运行 legacy group RED 与 terminal invariant 测试**

在尚未应用 Task 2 生产改动的基线运行：

```bash
cargo test --locked --lib legacy_group_completion_gates_ignore_non_passed_testing_reports -- --nocapture
```

预期：FAIL，错误为 `VerificationGateResultMissing`。随后运行：

```bash
cargo test --locked --test it_product group_final_confirm_without_authoritative_plan_binding_fails_closed -- --nocapture
```

预期：PASS，错误内容由测试断言为 `coding_attempt_plan_binding`；该测试不是 RED 行为测试，而是生产不变量保护测试。

- [ ] **Step 6：扩展 single-attempt final-confirm 测试**

在 `tests/it_product/product_coding_workspace_engine/part_08.rs`：

1. 将 `handle_final_confirm_completes_waiting_attempt_and_timeline_node` 改名为：

```rust
async fn handle_final_confirm_completes_without_testing_report_for_required_plan()
```

2. 在 `create_work_item` 前创建 required verification plan：

```rust
create_required_verification_plan(
    &lifecycle,
    "work_item_0001",
    "verification_plan_0001",
);
```

3. 在 `CreateWorkItemInput` 中加入：

```rust
verification_plan_ref: Some("verification_plan_0001".to_string()),
```

4. 调用 `handle_final_confirm` 前加入：

```rust
assert!(
    store
        .list_testing_reports("project_0001", "issue_0001", &attempt.id)
        .expect("testing reports")
        .is_empty()
);
```

5. 保留原有 Completed、timeline node、work item status 和 websocket event 断言。

- [ ] **Step 7：运行 single-attempt RED 测试**

运行：

```bash
cargo test --locked --test it_product handle_final_confirm_completes_without_testing_report_for_required_plan -- --nocapture
```

预期：FAIL，错误为 `VerificationGateResultMissing`。

- [ ] **Step 8：清理已无调用的 testing report fixture**

运行：

```bash
rg -n 'passed_testing_report_for_plan' tests/it_product/product_coding_workspace_engine
```

若只剩 `part_14.rs` 的函数定义，则删除该函数。随后运行：

```bash
cargo check --locked
```

预期：编译通过；schema v2、legacy gate-level 与 single-attempt 三个行为测试仍因生产门禁返回 `VerificationGateResultMissing`，legacy terminal invariant 测试保持 PASS。

---

### Task 2：移除 TestingReport 完成校验并转 GREEN

**文件：**

- 修改：`src/product/coding_workspace_engine/gates.rs:213-387`
- 修改：`src/product/coding_workspace_engine/gates/schema_v2.rs:142-167`
- 测试：Task 1 修改/新增的 completion gate 与 terminal invariant 测试文件

**接口：**

- Consumes：`run_completion_gates(&CodingExecutionAttempt)`、`run_group_completion_gates(&CodingExecutionAttempt)`、`validate_changed_files_for_work_item`、`validate_changed_files_for_runtime`。
- Produces：三条 completion gate 不读取 TestingReport，但仍返回原 `CompletionGateReport` 并执行所有非 testing gate；schema v2 group 与 single-attempt 完整 final-confirm 转 GREEN，legacy gate-level 转 GREEN，legacy terminal binding 失败关闭测试保持 GREEN。

- [ ] **Step 1：修改 single-attempt completion gate**

在 `run_completion_gates` 中，保留 `LifecycleStore`、work item 解析、changed files 校验、visible handoff 和 shared worktree clean 校验；仅删除：

```rust
let reports =
    self.store
        .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
self.verify_required_gates_satisfied(attempt, &lifecycle, &work_item, &reports)?;
```

在 changed-file 校验后留下明确注释：

```rust
// Testing is not orchestrated by the production pipeline and is intentionally
// not a completion prerequisite. All non-testing gates below remain required.
```

- [ ] **Step 2：修改 schema v2 group 与 legacy group completion gate**

在 `run_group_completion_gates` 中删除共同的 `list_testing_reports` 读取，并分别删除：

```rust
self.verify_schema_v2_required_gates_satisfied(attempt, &facts.runtime, &reports)?;
```

和：

```rust
self.verify_required_gates_satisfied(attempt, &lifecycle, work_item, &reports)?;
```

必须保留下列结构不变：

```rust
if self.schema_v2_group_plan_lineage(attempt)?.is_some() {
    for facts in self.schema_v2_group_completion_gate_facts(attempt)? {
        self.validate_changed_files_for_runtime(
            &facts.runtime,
            &facts.handoff.artifacts,
            worktree_path.as_ref(),
        )?;
    }
} else {
    // legacy handoff/work item lookup and validate_changed_files_for_work_item remain
}
```

在 `worktree_path` 计算前加入与 single-attempt 相同的产品决策注释。

- [ ] **Step 3：删除两个专用私有校验函数**

从 `gates.rs` 删除完整的：

```rust
fn verify_required_gates_satisfied(
    &self,
    attempt: &CodingExecutionAttempt,
    lifecycle: &LifecycleStore,
    work_item: &LifecycleWorkItemRecord,
    reports: &[TestingReport],
) -> Result<(), CodingWorkspaceEngineError>
```

从 `gates/schema_v2.rs` 删除完整的：

```rust
pub(super) fn verify_schema_v2_required_gates_satisfied(
    &self,
    attempt: &CodingExecutionAttempt,
    runtime: &ResolvedWorkItemRuntime,
    reports: &[TestingReport],
) -> Result<(), CodingWorkspaceEngineError>
```

不得删除 `src/product/coding_workspace_engine/mod.rs` 中 Testing 相关 import；其他 testing 模块和测试仍使用这些类型。不得删除 `VerificationGateResultMissing` 错误变体。

- [ ] **Step 4：运行 completion gate GREEN 与 terminal invariant 测试**

运行：

```bash
cargo test --locked --lib schema_v2_group_final_confirm_completes_without_testing_reports -- --nocapture
cargo test --locked --lib legacy_group_completion_gates_ignore_non_passed_testing_reports -- --nocapture
cargo test --locked --test it_product handle_final_confirm_completes_without_testing_report_for_required_plan -- --nocapture
cargo test --locked --test it_product group_final_confirm_without_authoritative_plan_binding_fails_closed -- --nocapture
```

预期：四个命令全部 PASS；前三个证明 TestingReport gate 已移除，第四个证明 group terminal binding 不变量未被放宽。

- [ ] **Step 5：确认完成门禁已完全解除 TestingReport 读取**

运行：

```bash
rg -n 'list_testing_reports|verify_required_gates_satisfied|verify_schema_v2_required_gates_satisfied' \
  src/product/coding_workspace_engine/gates.rs \
  src/product/coding_workspace_engine/gates/schema_v2.rs
```

预期：无匹配。随后确认公共错误变体仍存在：

```bash
rg -n 'VerificationGateResultMissing' src/product/coding_workspace_engine
```

预期：错误类型定义仍有匹配，completion gate 实现没有匹配。

- [ ] **Step 6：提交完成语义修改**

```bash
git add \
  src/product/coding_workspace_engine/tests.rs \
  src/product/coding_workspace_engine/tests/schema_v2_runtime.rs \
  src/product/coding_workspace_engine/tests/legacy_completion_gate.rs \
  src/product/coding_workspace_engine/gates.rs \
  src/product/coding_workspace_engine/gates/schema_v2.rs \
  tests/it_product/product_coding_workspace_engine/part_08.rs \
  tests/it_product/product_coding_workspace_engine/part_13.rs \
  tests/it_product/product_coding_workspace_engine/part_14.rs
git commit -m "fix: complete coding attempts without testing reports"
```

---

### Task 3：验证非 testing 门禁与仓库质量基线

**文件：**

- 验证：`src/product/coding_workspace_engine/gates.rs`
- 验证：`src/product/coding_workspace_engine/gates/schema_v2.rs`
- 验证：`tests/it_product/product_coding_workspace_engine/part_13.rs`
- 修改：`openspec/changes/relax-completion-testing-report-gate/tasks.md`（仅在证据通过后勾选）

**接口：**

- Consumes：Task 2 的 completion gate 实现。
- Produces：非 testing 门禁无回归、无新增 lint/format/test 失败的验证证据。

- [ ] **Step 1：运行非 testing 门禁定向回归**

```bash
cargo test --locked --test it_product group_final_confirm_rejects_unit_handoff_outside_exclusive_scope -- --nocapture
cargo test --locked --test it_product group_final_confirm_rejects_when_any_unit_not_completed -- --nocapture
cargo test --locked --test it_product final_confirm_owner_conflict_does_not_complete_attempt -- --nocapture
cargo test --locked --test it_product group_final_confirm_without_authoritative_plan_binding_fails_closed -- --nocapture
cargo test --locked --lib schema_v2_group_final_confirm_completes_without_testing_reports -- --nocapture
cargo test --locked --lib later_group_code_review_rejects_missing_head_commit -- --nocapture
```

预期：全部 PASS；分别证明 changed-file scope、unit 状态、lock owner、legacy group terminal binding、schema v2 runtime binding 与 completion commit 校验仍生效。

- [ ] **Step 2：运行全量 lib 与标准静态检查**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked --lib
```

预期：全部 PASS。若 `cargo fmt --check` 失败，只运行 `cargo fmt` 修复本次文件，再重新运行四条命令。

- [ ] **Step 3：运行相关集成测试并与基线比对**

```bash
cargo test --locked --test it_product
cargo test --locked --test it_core
```

预期：

- `it_product` 不得出现本 change 新增失败；当前已知基线为 221/224，既有失败：
  - `group_final_review_prompt_includes_all_unit_handoffs`
  - `execute_group_final_review_prompt_includes_request_commit_diff_and_function_context`
  - `group_final_confirm_completes_attempt_after_all_units_completed`
- `it_core` 不得出现本 change 新增失败；当前已知基线为 143/144，既有失败：
  - `large_file_guard::product_source_and_test_files_stay_under_line_limit`

如果任一新测试或本次修改相关测试失败，停止并进入 `systematic-debugging`；不得把它归为基线。

- [ ] **Step 4：运行仓库标准全量测试**

```bash
cargo test --locked
```

预期：可能因 Step 3 所列既有 4 个失败返回非零；必须保存测试计数和失败名称，确认没有新增失败。不得声称“全量测试通过”。

- [ ] **Step 5：严格校验 OpenSpec 并检查变更边界**

```bash
openspec validate relax-completion-testing-report-gate --strict
git diff --check
git status --short
git diff HEAD~1 -- \
  src/product/coding_workspace_engine/gates.rs \
  src/product/coding_workspace_engine/gates/schema_v2.rs \
  src/product/coding_workspace_engine/tests.rs \
  src/product/coding_workspace_engine/tests/schema_v2_runtime.rs \
  tests/it_product/product_coding_workspace_engine/part_08.rs \
  tests/it_product/product_coding_workspace_engine/part_13.rs \
  tests/it_product/product_coding_workspace_engine/part_14.rs
```

预期：OpenSpec 有效、无 whitespace error；diff 只包含测试 fixture、新语义测试、testing completion gate 移除与必要注释。

- [ ] **Step 6：勾选已有证据对应的 OpenSpec tasks**

根据实际完成情况更新：

```text
openspec/changes/relax-completion-testing-report-gate/tasks.md
```

只有完成且有新鲜证据的条目可改为 `[x]`。运行时重启与人工验收条目保持 `[ ]`，直到用户确认并实际执行。

- [ ] **Step 7：提交验证状态文档**

```bash
git add openspec/changes/relax-completion-testing-report-gate/tasks.md
git commit -m "docs: record completion gate implementation status"
```

若 tasks 没有可勾选变化，则跳过该提交，不创建空提交。

---

### Task 4：代码审查与用户确认后的运行时验收

**文件：**

- 审查：本 change 全部代码与 OpenSpec diff
- 运行时观察：`.aria/projects/project_0001/issues/issue_0001/coding-attempts/`
- 日志：`/tmp/aria-backend-watch.log`

**接口：**

- Consumes：Task 3 的测试与静态检查证据。
- Produces：审查结论，以及用户确认后由最新二进制驱动的手工完成流程证据。

- [ ] **Step 1：调用 `requesting-code-review` 审查本 change**

审查范围从 OpenSpec 基线提交 `1730019` 到当前 HEAD；重点检查：

- 三条 completion path 都不再读取 TestingReport；
- non-testing gates 没有被移动、短路或删除；
- schema v2 fixture 在 immutable write 前注入 checks；
- `VerificationGateResultMissing`、TestingReport 数据模型与 tester 基础设施仍保留；
- 没有新增 API、配置或持久化字段。

发现问题时先调用 `receiving-code-review` 核实，再按 TDD 修复并重跑 Task 3。

- [ ] **Step 2：完成前调用 `verification-before-completion`**

重新读取 Task 3 的新鲜命令输出；若证据过期或修复后未重跑，重新执行相应命令。汇报必须明确区分：

- 本 change 定向测试/静态检查是否通过；
- `it_product`、`it_core`、`cargo test --locked` 的既有失败；
- 尚未执行的后端重启和人工验收。

- [ ] **Step 3：请求用户确认后端重启**

当前前端 Vite 与后端 `cargo watch` 正在运行。未经用户明确确认，不停止、不 kill、不重启任何进程。确认后：

1. 停止当前后端 watch 及其服务子进程；
2. 使用既有命令重新启动：

```bash
cargo watch -w src -w Cargo.toml -w Cargo.lock \
  -x "run --locked -- web --workspace . --host 127.0.0.1 --port 4317"
```

3. 检查：

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
```

预期：返回 `{"status":"ok"}`。已知 btrfs/inotify 环境下 cargo-watch 可能不响应后续源码变化；本次以手动重启后的进程为准。

- [ ] **Step 4：由用户执行手工业务验收**

不主动创建业务数据或调用 Provider。用户操作现有或新 attempt，验收标准：

1. Coder completed；
2. Code Reviewer approved；
3. review request 创建/推送；
4. Internal Reviewer approved；
5. 没有 TestingReport 也能进入最终完成，UI 显示执行成功；
6. attempt `status=completed` 且 `completed_at` 非空；
7. 不产生伪造的 TestingReport 或 testing-success 状态。

若历史停滞 attempt 无法通过现有入口重放，不直接改写 JSON 状态；保留证据并使用新 attempt 验证。

- [ ] **Step 5：完成运行时验收后更新 OpenSpec tasks**

将 `3.3` 运行时交付条目勾选，重新运行：

```bash
openspec validate relax-completion-testing-report-gate --strict
git add openspec/changes/relax-completion-testing-report-gate/tasks.md
git commit -m "docs: record completion gate runtime verification"
```

在归档前按仓库流程执行 OpenSpec sync/archive 与分支收尾；未经用户指示不推送远端。
