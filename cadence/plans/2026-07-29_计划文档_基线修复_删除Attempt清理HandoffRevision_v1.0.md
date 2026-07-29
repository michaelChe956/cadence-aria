# 删除 attempt 时清理 handoff revision 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 删除 coding attempt 时同步清理该 attempt 各 unit 已认领的 handoff revision，消除 issue 级 lineage 中的 attempt 交接孤儿，使删除后重建 attempt 可正常完成同一 work item。

**Architecture:** 在 `WorkItemRevisionStore` 新增定向删除能力（含归属校验，不暴露为通用 API），在 `delete_coding_attempt` HTTP 流程中、`coding_store.delete_attempt` 之前遍历 attempt 各 unit 的 `latest_handoff_revision_id` 调用清理。不改 handoff 发布路径、ID 派生规则、group completion 判定、不可变写入语义。

**Tech Stack:** Rust（edition 2024，stable 工具链，`cargo test --locked`，🔴 禁止 `-j 1`）、OpenSpec。

**关联契约：** `openspec/changes/cleanup-attempt-handoff-revisions/`（proposal/design/specs/coding-attempt-deletion-cleanup/spec.md/tasks.md）

## Global Constraints

- 所有仓库操作只在 worktree `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0722-add-workspace` 中进行。
- 宿主机 Rust/Cargo；定向单测用 `cargo test --locked --lib <过滤名>`；🔴 禁止 `-j 1`。
- 不改 handoff revision 发布路径（`put_handoff_revision` + `write_immutable`）、内容结构、ID 派生规则。
- 不改 group completion 的 handoff 发布与 preflight 判定逻辑。
- 不引入跨 attempt 引用扫描。
- 不把 handoff 删除暴露为通用存储操作或对外 HTTP 接口（只供 attempt 删除流程调用）。
- 不自动清理历史遗留孤儿。
- 不影响 plan revision、work item revision、projection bundle、verification plan revision、dependency graph revision、validation report。
- 不重启服务、不调用 Provider、不创建业务数据（除非用户在 Task 3.3 明确授权）。
- `src/product/work_item_revision_store/tests.rs` 当前 712 行；新测试优先放既有拆分文件或新建小文件，避免单文件膨胀。
- `git commit` 消息使用中文项目惯用前缀（`test:` / `fix:` / `docs:`）。
- 实施采用 Subagent-Driven 模式：每个任务完成后执行规格+质量双阶段审查。

---

## File Structure

**修改文件：**

- `src/product/work_item_revision_store/handoff.rs`（89 行）：新增 `delete_handoff_revision` 方法，含 `ensure_plan_scope` 与归属校验。
- `src/web/handlers/coding.rs:744-753`（`delete_coding_attempt` 末尾，`cleanup_coding_attempt_workspace` 与 `coding_store.delete_attempt` 之间）：插入清理调用。

**测试文件：**

- `src/product/work_item_revision_store/tests/handoff_deletion.rs`（新建）：存储层删除能力与归属校验测试。
- `tests/it_web/web_coding_attempt_api/part_01.rs`（已有文件追加）：HTTP 删除流程的端到端清理测试（单 unit、多 unit、删除后重建）。

**不修改文件：**

- `src/product/work_item_revision_store/paths.rs:190`（`handoff_revision_path`）：复用既有路径函数。
- `src/product/coding_attempt_store/utils.rs:314`（`remove_file_if_exists`）：复用既有删除工具（若可见性不足，在 `work_item_revision_store` 内写等价的小工具，不跨模块提权）。
- `src/product/coding_workspace_engine/group_completion.rs`：preflight 判定逻辑不动。

---

## Task 1: 失败测试 — 存储层删除能力与归属校验

**Files:**
- Create: `src/product/work_item_revision_store/tests/handoff_deletion.rs`
- Modify: `src/product/work_item_revision_store/tests.rs`（注册新模块）

**Interfaces:**
- Consumes: `WorkItemRevisionStore::new(paths)`；`put_handoff_revision(plan, value)`（`handoff.rs:15`）；`get_handoff_revision(plan, wi, id)`（`handoff.rs:30`）；`get_plan_lineage(project, issue, plan)`（`plan.rs:51`）；`HandoffRevision` 结构（`product/models`）。
- Produces: 覆盖 spec requirement 3（归属校验）与 requirement 4 的部分（删除能力存在、不影响编译产物）。

- [ ] **Step 1: 注册新测试模块**

Modify `src/product/work_item_revision_store/tests.rs`，在现有 `mod` 声明区追加：

```rust
#[path = "tests/handoff_deletion.rs"]
mod handoff_deletion;
```

- [ ] **Step 2: 编写删除成功测试（先 publish 再 delete）**

Create `src/product/work_item_revision_store/tests/handoff_deletion.rs`。先建夹具：构造一个 `WorkItemPlanLineage` + 一个 `wi_a` 的 `HandoffRevision`，`put_handoff_revision` 写入。然后调用待新增的 `delete_handoff_revision`，断言 `get_handoff_revision` 返回 `NotFound`：

```rust
use super::*;
use crate::product::models::{HandoffRevision, WorkItemPlanLineage};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

// 复用本目录既有测试的 lineage/work_item 夹具构造方式（参考 initial_publication.rs / publication.rs）。
// 若无现成夹具，构造最小 WorkItemPlanLineage + logical work item + work item revision 并 put。

#[test]
fn delete_handoff_revision_removes_published_revision() {
    let (root, store, lineage) = minimal_lineage_fixture();
    let handoff = HandoffRevision {
        id: "handoff_revision_coding_unit_run_0001".to_string(),
        logical_work_item_id: "wi_a".to_string(),
        work_item_revision_id: lineage.work_item_revision_for("wi_a").clone(),
        coding_unit_run_id: "coding_unit_run_0001".to_string(),
        provided_contracts: Default::default(),
        provided_capabilities: Default::default(),
        contract_hash: "abc".to_string(),
        commit_sha: "deadbeef".to_string(),
        tests: Vec::new(),
        artifacts: Vec::new(),
        created_at: "2026-07-28T00:00:00Z".to_string(),
    };
    store.put_handoff_revision(&lineage, &handoff).unwrap();

    store
        .delete_handoff_revision(&lineage, "wi_a", "handoff_revision_coding_unit_run_0001")
        .unwrap();

    let err = store
        .get_handoff_revision(&lineage, "wi_a", "handoff_revision_coding_unit_run_0001")
        .unwrap_err();
    assert!(matches!(err, ProductStoreError::NotFound { kind: "handoff_revision", .. }));
    drop(root);
}
```

**关键**：夹具构造要确保 lineage 内 `wi_a` 的 logical work item 与 work item revision 真实存在（`put_handoff_revision` 会校验）。参考 `tests/initial_publication.rs` / `tests/publication.rs` 的构造范式。

- [ ] **Step 3: 编写归属校验失败测试**

```rust
#[test]
fn delete_handoff_revision_rejects_mismatched_logical_work_item_id() {
    let (root, store, lineage) = minimal_lineage_fixture();
    // handoff 属于 wi_a
    let handoff = handoff_for("wi_a", "handoff_revision_run_0001");
    store.put_handoff_revision(&lineage, &handoff).unwrap();

    // 用 wi_b 的名义删除 wi_a 的 handoff：必须失败
    let err = store
        .delete_handoff_revision(&lineage, "wi_b", "handoff_revision_run_0001")
        .unwrap_err();

    // 归属不符时：要么因 path 取向 wi_b 找不到档案（NotFound），要么读到档案后 logical_work_item_id 不匹配（identity_mismatch）。
    // 两种都属正确拒绝。断言档案仍在：
    assert!(store.get_handoff_revision(&lineage, "wi_a", "handoff_revision_run_0001").is_ok());
    drop(root);
}
```

**实现约束提醒（写进 Step 3 注释）**：归属校验有两种落地形态——(a) 先 `get_handoff_revision` 读出档案校验 `logical_work_item_id` 再删；(b) path 由传入的 `logical_work_item_id` 决定，归属不符会先撞 NotFound。两种都能通过本测试（关键断言是「档案未被删」）。Task 2 实现时选形态 (a)（显式校验更安全，防 path 拼写差异）。

- [ ] **Step 4: 编写编译产物不受影响测试**

```rust
#[test]
fn delete_handoff_revision_does_not_touch_plan_compilation_artifacts() {
    let (root, store, lineage) = minimal_lineage_fixture_with_compiled_plan();
    // lineage 下已有 plan revision / work item revision / projection bundle 等
    let handoff = handoff_for("wi_a", "handoff_revision_run_0001");
    store.put_handoff_revision(&lineage, &handoff).unwrap();

    store.delete_handoff_revision(&lineage, "wi_a", "handoff_revision_run_0001").unwrap();

    // 断言编译产物仍在
    assert!(store.get_plan_revision(&lineage, &lineage.active_revision_id).is_ok());
    // 其余编译产物按夹具实际可读 API 断言存在
    drop(root);
}
```

- [ ] **Step 5: 运行测试，确认 RED**

Run: `cargo test --locked --lib handoff_deletion -- --nocapture`
Expected: 编译失败——`delete_handoff_revision` 方法不存在（`no method named delete_handoff_revision`）。这是 RED 的合法形态（方法尚未实现）。

- [ ] **Step 6: `cargo check --locked` + `cargo fmt --check`**

Run: `cargo check --locked && cargo fmt --check`
Expected: Step 5 的编译失败是预期的（方法未实现）；Step 6 仅核对格式。

- [ ] **Step 7: Commit**

```bash
git add src/product/work_item_revision_store/tests/handoff_deletion.rs \
        src/product/work_item_revision_store/tests.rs
git commit -m "test: cover handoff revision deletion"
```

---

## Task 2: 生产实现 — 存储层删除能力

**Files:**
- Modify: `src/product/work_item_revision_store/handoff.rs`

**Interfaces:**
- Consumes: `ensure_plan_scope(plan)`（`mod.rs:59`）；`handoff_revision_path(...)`（`paths.rs:190`）；`get_handoff_revision(plan, wi, id)`（`handoff.rs:30`）。
- Produces: `pub fn delete_handoff_revision(&self, plan, logical_work_item_id, handoff_revision_id) -> Result<(), ProductStoreError>`。

- [ ] **Step 1: 在 handoff.rs 新增 delete_handoff_revision**

在 `WorkItemRevisionStore` impl 块内（建议置于 `put_handoff_revision` 之后、`get_handoff_revision` 之前）追加：

```rust
/// 删除单个 handoff revision。仅用于 coding attempt 删除流程清理该 attempt
/// 已认领的交接产物；不作为通用存储操作暴露。删除前校验档案归属的
/// logical_work_item_id 与传入参数一致，归属不符时不删除。
pub fn delete_handoff_revision(
    &self,
    plan: &WorkItemPlanLineage,
    logical_work_item_id: &str,
    handoff_revision_id: &str,
) -> Result<(), ProductStoreError> {
    self.ensure_plan_scope(plan)?;
    validate_relative_id(logical_work_item_id)?;
    validate_relative_id(handoff_revision_id)?;
    self.get_logical_work_item(plan, logical_work_item_id)?;
    // 显式校验归属：读到档案后确认 logical_work_item_id 一致再删，防止 path
    // 拼写差异或指针错配导致误删其他 work item 的交接产物。
    let existing = self.get_handoff_revision(plan, logical_work_item_id, handoff_revision_id)?;
    if existing.logical_work_item_id != logical_work_item_id {
        return Err(identity_mismatch("handoff_revision", handoff_revision_id));
    }
    let path = self.handoff_revision_path(
        &plan.project_id,
        &plan.issue_id,
        &plan.id,
        logical_work_item_id,
        handoff_revision_id,
    );
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}
```

**关键约束：**
- 复用 `ensure_plan_scope` + `validate_relative_id` + `get_logical_work_item`，与 `put_handoff_revision` 一致的前置校验。
- 归属校验用 `get_handoff_revision`（它内部已校验 `id` 与 `logical_work_item_id` 一致，不一致返回 `identity_mismatch`）。所以 Step 1 的 `if existing.logical_work_item_id != logical_work_item_id` 实际是双保险——`get_handoff_revision` 已会先拒。保留这行让意图显式。
- 删除用内联 `fs::remove_file` + NotFound 容忍（与 `coding_attempt_store/utils.rs:314` `remove_file_if_exists` 同形态）。**不跨模块提权 `remove_file_if_exists`**（它是 `coding_attempt_store` 的 `pub(crate)`，跨 store 复用会破坏模块边界；work_item_revision_store 内联等价逻辑更清晰）。
- 不暴露为通用 API：方法签名 `pub fn` 是 store 内部公开（crate 内），但**不在任何 HTTP handler 或非 attempt-删除路径调用**。Task 3 的调用点是唯一消费者。

- [ ] **Step 2: 运行 Task 1 测试，确认 GREEN**

Run: `cargo test --locked --lib handoff_deletion -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 3: 确认既有 handoff 路径未破坏**

Run: `cargo test --locked --lib work_item_revision_store -- --nocapture`
Expected: 全部 PASS。重点：`initial_publication` / `publication` / `projection_artifacts` / `repair_status` / `concurrency` 未受影响。

- [ ] **Step 4: `cargo clippy --all-targets --all-features --locked -- -D warnings` + `cargo check --locked` + `cargo fmt --check`**

Expected: 全部通过。

- [ ] **Step 5: Commit**

```bash
git add src/product/work_item_revision_store/handoff.rs
git commit -m "fix: add handoff revision deletion for attempt cleanup"
```

---

## Task 3: 失败测试 — HTTP 删除流程的端到端清理

**Files:**
- Modify: `tests/it_web/web_coding_attempt_api/part_01.rs`（追加测试）

**Interfaces:**
- Consumes: `delete_coding_attempt` HTTP handler（DELETE attempt route）；`WorkItemRevisionStore::list_handoff_revisions(plan, wi)`（`handoff.rs:56`）或 `get_handoff_revision`；既有 group attempt 创建 fixture（参考 `creates_group_coding_attempt_from_schema_v2_revisions_without_legacy_work_items` line 285、`fixtures_authoritative_group.rs`）。
- Produces: 覆盖 spec requirement 1（单/多/无 unit 三个 scenario）、requirement 2（删除后重建）。

**夹具可行性说明（重要）：**
- HTTP 端到端测试需要：创建 schema_v2 group attempt → 推进到某 unit 完成（产出 handoff）→ DELETE attempt → 断言 lineage 中 handoff 已删。
- 推进到「unit 完成 + handoff 发布」可能需要真实 provider 或 test_controls 注入。优先用 `test_controls` 的 fake provider 推进 group 流程（参考 `tests/it_web/web_coding_ws_handler/fixtures_authoritative_group.rs` 与既有 group 完成测试）。
- 若端到端推进成本过高（需要完整 coding→review→completion 链路），降级为：直接用 store API 构造一个「已发布 handoff 的 attempt」状态，再调 HTTP DELETE，断言 lineage 清理。降级测试仍覆盖清理逻辑，只是不走完整 runner。在测试注释中说明降级原因。

- [ ] **Step 1: 编写单 unit 已发布 handoff 的删除测试**

在 `part_01.rs` 追加（参考 `delete_coding_attempt_releases_active_lock_when_clean` line 756 的 fixture 范式）：

```rust
#[tokio::test]
async fn delete_coding_attempt_removes_handoff_revision_for_completed_unit() {
    // 1. 创建 schema_v2 group attempt，推进首个 unit 至完成（产出 handoff_revision_coding_unit_run_0001）
    //    用 test_controls fake provider 或 store 直构，参考 fixtures_authoritative_group.rs
    let (app, attempt_id, lineage, store) = /* group_attempt_with_completed_first_unit().await */;
    let handoff_path = /* lineage 下 wi_format_duration_lib/handoff-revisions/handoff_revision_coding_unit_run_0001.json */;
    assert!(handoff_path.exists(), "precondition: handoff published");

    // 2. DELETE attempt
    let (status, _) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    ).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 3. 断言 handoff 已从 lineage 删除
    assert!(!handoff_path.exists(), "handoff revision must be removed from lineage");
}
```

- [ ] **Step 2: 编写删除后重建可完成测试（spec requirement 2）**

```rust
#[tokio::test]
async fn delete_coding_attempt_then_rebuild_completes_same_work_item() {
    // 1. 创建 group attempt，推进首个 unit 完成（产出 handoff）
    let (app, first_attempt_id, ..) = /* ... */;
    // 2. DELETE first attempt
    request_json(app.clone(), Method::DELETE, &scoped_attempt_uri(&first_attempt_id, ""), json!({})).await;
    // 3. 重新创建 attempt，推进同一 work item 完成
    //    断言：不返回 group_completion_handoff_revision_conflict，handoff 成功发布
    //    用 test_controls fake provider 推进，或断言 unit 能进入 completed 状态
}
```

- [ ] **Step 3: 编写多 unit 均已发布 handoff 的删除测试（spec requirement 1 第二个 scenario）**

```rust
#[tokio::test]
async fn delete_coding_attempt_removes_all_handoff_revisions_for_completed_units() {
    // 1. 创建 schema_v2 group attempt，推进全部 unit 至完成（各产出 handoff_revision_coding_unit_run_000N）
    //    需多 unit 的 group plan（如 wi_format_duration_lib + wi_demo_page 两个都完成）
    let (app, attempt_id, lineage, store) = /* group_attempt_with_all_units_completed().await */;
    let handoff_paths = /* 两个 unit 各自的 handoff 文件路径 */;
    assert!(handoff_paths.iter().all(|p| p.exists()), "precondition: all handoffs published");

    // 2. DELETE attempt
    let (status, _) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    ).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 3. 断言全部 handoff 已从 lineage 删除
    for p in &handoff_paths {
        assert!(!p.exists(), "handoff revision must be removed: {}", p.display());
    }
}
```

**夹具成本提醒**：多 unit 全完成需推进整个 group 流程至 review_request，成本较高。若推进链路复杂，可降级为 store 直构两个 unit 的 handoff + DELETE，断言清理。降级须在注释说明。

- [ ] **Step 4: 编写无 unit 认领 handoff 的删除测试（spec requirement 1 第三个 scenario）**

```rust
#[tokio::test]
async fn delete_coding_attempt_without_handoff_does_not_touch_lineage() {
    // 1. 创建 group attempt，但不推进任何 unit 完成（无 handoff）
    //    或推进到 coding 阶段未完成
    let (app, attempt_id) = /* group_attempt_without_completed_units().await */;
    // 预置：lineage 中无该 attempt 的 handoff（若有其他历史 handoff，记录其存在状态）

    // 2. DELETE attempt
    let (status, _) = request_json(
        app.clone(),
        Method::DELETE,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    ).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 3. 断言 lineage 中无新增删除（编译产物、其他 handoff 均在）
    //    特别地：若 lineage 下本就无 handoff，删除后仍无 handoff；
    //    若有其他 attempt 的 handoff（不应发生，因排他约束），未被误删。
    //    核心断言：删除正常完成（status 204），且未抛错。
}
```

**关键**：无 unit 认领 handoff 时，清理逻辑应为空操作（遍历 units，所有 `latest_handoff_revision_id` 均为 None，循环体不执行）。本测试锁定「空操作不报错、不误删」契约。

- [ ] **Step 5: 运行测试，确认 RED**

Run: `cargo test --locked --test it_web delete_coding_attempt_removes_handoff delete_coding_attempt_removes_all delete_coding_attempt_without_handoff delete_coding_attempt_then_rebuild -- --nocapture`
Expected: `removes_handoff` / `removes_all` FAIL（handoff 在 DELETE 后仍存在）；`without_handoff` / `then_rebuild` 可能因夹具推进复杂度而在 RED 阶段编译失败或失败，记录状态。

- [ ] **Step 6: `cargo check --locked`（确认编译）+ `cargo fmt --check`**

- [ ] **Step 7: Commit**

```bash
git add tests/it_web/web_coding_attempt_api/part_01.rs
git commit -m "test: cover attempt deletion handoff cleanup"
```

---

## Task 4: 生产实现 — 删除流程接入清理

**Files:**
- Modify: `src/web/handlers/coding.rs:744-753`（`delete_coding_attempt` 末尾）

**Interfaces:**
- Consumes: `coding_store.list_coding_units(project, issue, attempt)`（返回 `Vec<CodingExecutionUnit>`）；`WorkItemRevisionStore::new(paths)`；`get_plan_lineage`；新增的 `delete_handoff_revision`；`CodingAttemptPlanBinding`（取 plan_id）。
- Produces: attempt 删除流程中、`delete_attempt` 之前的清理调用。

- [ ] **Step 1: 在 delete_coding_attempt 插入清理调用**

`coding.rs:744-753` 当前末尾：

```rust
    cleanup_coding_attempt_workspace(&repository, &attempt).await?;
    coding_store
        .delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    Ok(StatusCode::NO_CONTENT.into_response())
```

改为（在 `delete_attempt` 之前插入清理）：

```rust
    cleanup_coding_attempt_workspace(&repository, &attempt).await?;
    cleanup_attempt_handoff_revisions(&app_paths, &coding_store, &attempt)
        .map_err(product_store_api_error)?;
    coding_store
        .delete_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    Ok(StatusCode::NO_CONTENT.into_response())
```

并在 `coding.rs` 内（或相邻 helper 区）新增 helper：

```rust
/// 删除 attempt 时清理该 attempt 各 unit 已认领的 handoff revision。
/// 清理在 attempt 记录删除之前执行（依赖 unit 指针可读）。归属校验由
/// delete_handoff_revision 负责；找不到档案视为已清理（幂等）。
fn cleanup_attempt_handoff_revisions(
    app_paths: &ProductAppPaths,
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Result<(), ProductStoreError> {
    let binding = coding_store.get_plan_binding(attempt)?;
    let lineage = WorkItemRevisionStore::new(app_paths.clone())
        .get_plan_lineage(&attempt.project_id, &attempt.issue_id, &binding.plan_id)?;
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    let units = coding_store.list_coding_units(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    for unit in units {
        if let Some(handoff_id) = &unit.latest_handoff_revision_id {
            // 归属校验 + 删除；NotFound 视为已清理（幂等），其他错误上抛。
            let _ = revision_store.delete_handoff_revision(
                &lineage,
                &unit.logical_work_item_id,
                handoff_id,
            )?;
        }
    }
    Ok(())
}
```

**关键约束：**
- 清理必须在 `coding_store.delete_attempt` **之前**（unit 指针数据届时还可读）。
- `get_plan_binding` 失败（binding 不存在）按既有删除流程错误处理上抛——不静默跳过。
- `get_plan_lineage` 失败同上。
- 单个 unit 的 handoff 删除失败上抛，中断删除流程（不静默留不一致状态，符合 spec Risks 的「清理失败遵循既有删除流程错误处理」）。
- `delete_handoff_revision` 内部对 NotFound 返回 `Ok(())`（档案不存在视为已清理），所以 HTTP 层无需特殊处理 NotFound；`?` 直接传播其他 Err。
- `let _ =` 丢弃 `Ok(())` 值，不是忽略错误。
- `ProductAppPaths`、`WorkItemRevisionStore`、`CodingAttemptStore`、`CodingExecutionAttempt` 的 import 按 `coding.rs` 既有 use 块补齐（先 grep 确认哪些已 import）。

- [ ] **Step 2: 确认 import 完整**

grep `coding.rs` 顶部 use 块，确认 `WorkItemRevisionStore`、`ProductAppPaths`（或等价路径解析函数）已 import。缺则补。`product_app_paths(&state)` 已在 handler 顶部调用为 `app_paths`，直接传入 helper。

- [ ] **Step 3: 运行 Task 3 测试，确认 GREEN**

Run: `cargo test --locked --test it_web delete_coding_attempt_removes_handoff delete_coding_attempt_removes_all delete_coding_attempt_without_handoff delete_coding_attempt_then_rebuild -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 4: 运行既有删除回归**

Run: `cargo test --locked --test it_web delete_coding_attempt -- --nocapture`
Expected: 全部 PASS。重点：`delete_coding_attempt_releases_active_lock_when_clean`（line 756）、`delete_coding_attempt_with_dirty_shared_worktree_still_removes_workspace`（line 796）、`delete_failed_coding_attempt_with_dirty_shared_worktree_still_removes_workspace`（line 853）—— 既有删除行为不得改变（无 unit 认领 handoff 时清理为空操作）。

- [ ] **Step 5: 运行 group completion 回归**

Run: `cargo test --locked --lib group_completion -- --nocapture`
Expected: 全部 PASS。group completion 的 handoff 发布与 preflight 判定未受影响。

- [ ] **Step 6: `cargo clippy --all-targets --all-features --locked -- -D warnings` + `cargo check --locked` + `cargo fmt --check`**

Expected: 全部通过。

- [ ] **Step 7: Commit**

```bash
git add src/web/handlers/coding.rs
git commit -m "fix: cleanup handoff revisions when deleting coding attempt"
```

---

## Task 5: 全量验证与交付

**Files:**
- 无（仅验证与 OpenSpec 勾选）

- [ ] **Step 1: lib 全量测试**

Run: `cargo test --locked --lib`
Expected: 全部 PASS（区分既有失败基线）。

- [ ] **Step 2: it_web 定向回归**

Run: `cargo test --locked --test it_web web_coding_attempt_api -- --nocapture`
Expected: 全部 PASS。

- [ ] **Step 3: it_product 定向回归**

Run: `cargo test --locked --test it_product product_coding_workspace_engine -- --nocapture`
Expected: 仅有已知 3 项基线失败（`group_final_review_prompt_includes_all_unit_handoffs` / `execute_group_final_review_prompt_includes_request_commit_diff_and_function_context` / `group_final_confirm_completes_attempt_after_all_units_completed`），无新增失败。

- [ ] **Step 4: OpenSpec strict 校验**

Run: `openspec validate cleanup-attempt-handoff-revisions --strict`
Expected: Change valid.

- [ ] **Step 5: 勾选 OpenSpec tasks 1.1-1.6、2.1-2.3、3.1、3.2（3.3 保持未勾）**

- [ ] **Step 6: `git diff --check` + 状态提交**

```bash
git diff --check
git add openspec/changes/cleanup-attempt-handoff-revisions/tasks.md
git commit -m "docs: record attempt handoff cleanup status"
```

- [ ] **Step 7: 请求用户授权重启后端（对应 OpenSpec 3.3）**

向用户报告：实现与验证完成，请求重启后端以进行人工业务验收。用户确认前不重启、不调用 Provider、不创建业务数据。

---

## Verification Anchors

- **核心回归（必须 GREEN）：** `handoff_deletion` 全部、`work_item_revision_store` 全部、`delete_coding_attempt_removes_handoff*`、`delete_coding_attempt_then_rebuild*`、既有 `delete_coding_attempt*` 三项、`group_completion` 全部。
- **既有基线失败（不影响本 change）：** `it_product` 3 项。
- **安全断言：** handoff 发布路径（`put_handoff_revision` + `write_immutable`）不变；group completion preflight 判定不变；ID 派生规则不变；编译产物（plan/work-item/projection/verification/dependency）不被删除；handoff 删除不暴露为通用 API 或 HTTP 接口。
