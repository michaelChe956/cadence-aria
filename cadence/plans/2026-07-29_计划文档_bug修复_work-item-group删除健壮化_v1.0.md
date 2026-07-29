# Work Item Group 删除健壮化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 work item group 的删除接口在「完整」与「半残」两种状态下都能一次性删干净、无残留、不误伤其他数据；有 coding workspace 时拒绝删除并提示先删 coding workspace；删除失败时返回可定位的错误细节。

**Architecture:** 删除路径从「前置完整性校验 → 删除」改为「coding workspace 存在性门禁 → 尽力清理（每步 NotFound=OK）」。补齐漏删的 revisions / revision-publications / shared-worktree / attempt lock 四类产物。错误转换兜底带 `kind/id` 进 details。

**Tech Stack:** Rust（edition 2024）、Axum web handler、文件系统 JSON 存储。

**Change:** `harden-work-item-group-deletion`（OpenSpec 契约已 strict-valid 并获用户确认）。本 plan 展开该契约的 tasks.md 工作包 1.1–3.5，对应 spec.md 四组 requirement。

## Global Constraints

- **TDD**：每个实现步骤先写失败测试、确认失败原因正确、再实现。
- 构建命令：`cargo test --locked`（🔴 禁止 `-j 1`）、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo fmt --check`。
- 已知基线失败：`it_core` 的 `large_file_guard`（8 个既有超限文件，非本 plan 引入）。
- 文档命名遵循 `cadence/` 规范。
- 不改 `purge_plan_artifacts`、不改 `get_attempt_for_work_item_group` 判定逻辑、不改 `DELETE /api/coding-attempts/{id}` 容错（design 已知缺口一）。

## File Structure

| 文件 | 职责 |
|---|---|
| `src/product/work_item_revision_store/purge.rs`（新建） | `purge_plan_revisions`：删 revisions + publications 整目录 |
| `src/product/work_item_revision_store/mod.rs`（改） | 声明 `mod purge` |
| `src/product/lifecycle_store/worktree.rs`（改） | `delete_issue_shared_worktree`：删 shared-worktree json + lock |
| `src/web/handlers/support.rs`（改） | `product_store_api_error` 兜底带 details；新增 `coding_workspace_exists` 构造 |
| `src/web/handlers/lifecycle/deletion.rs`（改） | 重写 schema v2 删除路径 + legacy 路径加门禁 |
| `tests/it_web/web_coding_attempt_api/part_14.rs`（新建） | 端到端测试：门禁拒绝、完整无残留、半残无残留、不误伤、错误透明 |
| `tests/it_web/web_coding_attempt_api.rs`（改） | `include!("web_coding_attempt_api/part_14.rs")` |

---

### Task 1: store 层两个删除方法

**Files:**
- Create: `src/product/work_item_revision_store/purge.rs`
- Modify: `src/product/work_item_revision_store/mod.rs`（加 `mod purge;`，或就近放 `pub use`）
- Modify: `src/product/lifecycle_store/worktree.rs`
- Test: `src/product/work_item_revision_store/tests/`（现有测试模块）、`src/product/lifecycle_store/` 现有测试模块

**Interfaces:**
- Produces: `WorkItemRevisionStore::purge_plan_revisions(&self, project_id, issue_id, plan_id) -> Result<(), ProductStoreError>`；`LifecycleStore::delete_issue_shared_worktree(&self, project_id, issue_id) -> Result<(), ProductStoreError>`

- [ ] **Step 1.1: 写失败测试 `purge_plan_revisions`**

在 revision store 现有测试模块加：
```rust
#[test]
fn purge_plan_revisions_removes_revision_and_publication_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    paths.ensure().unwrap();
    let store = WorkItemRevisionStore::new(paths.clone());
    // 播种：在 plan_root 下建任意文件 + publications 下建任意文件
    let plan_root = tmp.path().join(".aria/projects/p/issues/i/work-item-revisions/plan_1");
    std::fs::create_dir_all(plan_root.join("plan-revisions")).unwrap();
    std::fs::write(plan_root.join("lineage.json"), "{}").unwrap();
    let pubs = tmp.path().join(".aria/projects/p/issues/i/work-item-revision-publications/plan_1");
    std::fs::create_dir_all(&pubs).unwrap();
    std::fs::write(pubs.join("compile_1.json"), "{}").unwrap();

    store.purge_plan_revisions("p", "i", "plan_1").unwrap();

    assert!(!plan_root.exists());
    assert!(!pubs.exists());
}

#[test]
fn purge_plan_revisions_succeeds_when_dirs_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    paths.ensure().unwrap();
    let store = WorkItemRevisionStore::new(paths);
    // 不播种任何产物
    store.purge_plan_revisions("p", "i", "plan_1").unwrap();
    // 到这里即为成功：不存在的目录不报错
}
```
（项目根/issue 路径布局以 `ProductAppPaths` 实际为准；实现时参照 revision store 现有测试的夹具写法校正 `p/issues/i` 路径段。）

- [ ] **Step 1.2: 跑测试确认失败**

`cargo test --locked --lib purge_plan_revisions` —— 预期编译失败（方法不存在）。

- [ ] **Step 1.3: 实现 `purge_plan_revisions`**

新建 `src/product/work_item_revision_store/purge.rs`：
```rust
use super::WorkItemRevisionStore;
use crate::product::json_store::ProductStoreError;

impl WorkItemRevisionStore {
    /// 删除一个 plan 的全部 revision 产物与 publication 记录。
    /// revisions 的所有子目录都在 plan_root 下，一次清空；publications 是并列目录。
    /// 不存在视为成功（清理路径不应要求被清理对象存在）。
    pub fn purge_plan_revisions(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<(), ProductStoreError> {
        let plan_root = self.plan_root(project_id, issue_id, plan_id);
        remove_dir_all_if_exists(&plan_root)?;
        let publications = self
            .paths
            .issue_root(project_id, issue_id)
            .join("work-item-revision-publications")
            .join(plan_id);
        remove_dir_all_if_exists(&publications)?;
        Ok(())
    }
}

fn remove_dir_all_if_exists(path: &std::path::Path) -> Result<(), ProductStoreError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}
```
在 `mod.rs` 加 `mod purge;`。`plan_root`（`paths.rs:8`）与 `self.paths.issue_root` 已是 `pub(super)`/可见，确认 purge.rs 能访问（同模块树，应可）。

- [ ] **Step 1.4: 跑测试确认通过**

`cargo test --locked --lib purge_plan_revisions` —— 预期两个测试通过。

- [ ] **Step 1.5: 写失败测试 `delete_issue_shared_worktree`**

在 lifecycle_store worktree 测试模块加：播种 `issue-shared-worktree.json` + `.issue-shared-worktree.json.lock`，调用后断言两者不存在；另加一个不存在的用例返回成功。（参照 worktree.rs 现有 `upsert_issue_shared_worktree` 测试夹具。）

- [ ] **Step 1.6: 跑测试确认失败**

- [ ] **Step 1.7: 实现 `delete_issue_shared_worktree`**

在 `src/product/lifecycle_store/worktree.rs` 加（复用同文件已有路径构造与 `remove_file_if_exists`，`lifecycle_store/utils.rs:159`）：
```rust
pub fn delete_issue_shared_worktree(
    &self,
    project_id: &str,
    issue_id: &str,
) -> Result<(), ProductStoreError> {
    let root = self.paths.issue_lifecycle_root(project_id, issue_id);
    remove_file_if_exists(&root.join("issue-shared-worktree.json"))?;
    remove_file_if_exists(&root.join(".issue-shared-worktree.json.lock"))?;
    Ok(())
}
```
（`issue_lifecycle_root` 方法名以 `lifecycle_store` 现有为准备；实现时参照 `workspace.rs:638` 的 `issue_lifecycle_root` 用法校正。）

- [ ] **Step 1.8: 跑测试确认通过**

- [ ] **Step 1.9: Commit**

```sh
git add src/product/work_item_revision_store/purge.rs src/product/work_item_revision_store/mod.rs src/product/lifecycle_store/worktree.rs
git commit -m "feat: 新增 plan revisions 与 shared-worktree 的容错删除方法"
```

---

### Task 2: 错误透明 + 门禁错误码

**Files:**
- Modify: `src/web/handlers/support.rs`
- Test: `src/web/handlers/support.rs` 内联或 `tests/it_web`

**Interfaces:**
- Produces: `product_store_api_error` 兜底带 details；调用方可构造 `coding_workspace_exists` 错误（通过 `ApiError::runtime` 直接构造，无需新函数，但可在 support.rs 暴露一个 helper）

- [ ] **Step 2.1: 写失败测试——兜底带 details**

在 support.rs 测试模块或 it_web 加：构造一个未被精确映射的 `ProductStoreError::IdentityMismatch { kind: "runtime_binding_missing", id: "plan_1" }`，经 `product_store_api_error` 后断言返回的 `ApiError` details 含 `kind == "runtime_binding_missing"` 与 `id == "plan_1"`，code 仍为 `product_store_error`。

- [ ] **Step 2.2: 跑测试确认失败**（当前兜底返回空 `{}`）

- [ ] **Step 2.3: 改 `product_store_api_error` 兜底分支**

把 `src/web/handlers/support.rs` 末尾的：
```rust
_ => ApiError::runtime("product_store_error", "product store operation failed", json!({})),
```
改为：
```rust
other => {
    let details = match &other {
        ProductStoreError::NotFound { kind, id }
        | ProductStoreError::Ambiguous { kind, id }
        | ProductStoreError::Conflict { kind, id }
        | ProductStoreError::IdentityMismatch { kind, id } => {
            json!({ "kind": kind, "id": id })
        }
        ProductStoreError::Io(message)
        | ProductStoreError::Json(message)
        | ProductStoreError::PathEscape(message) => json!({ "message": message }),
    };
    ApiError::runtime("product_store_error", "product store operation failed", details)
}
```
（`match &other` 下 `kind` 为 `&&'static str`；`json!` 经 serde 对 `&&str` 可序列化。实现时若 json! 报类型错，改 `*kind`。）

- [ ] **Step 2.4: 跑测试确认通过**

- [ ] **Step 2.5: 新增 `coding_workspace_exists` helper**

在 support.rs 加（供 deletion.rs 调用）：
```rust
pub(crate) fn coding_workspace_exists_error(plan_id: &str, attempt_id: &str) -> ApiError {
    ApiError::runtime(
        "coding_workspace_exists",
        "存在 coding workspace，请先删除 coding workspace 再删除 work item group",
        json!({ "plan_id": plan_id, "attempt_id": attempt_id }),
    )
}
```

- [ ] **Step 2.6: Commit**

```sh
git add src/web/handlers/support.rs
git commit -m "feat: 删除失败错误带 kind/id 详情，新增 coding_workspace_exists 错误码"
```

---

### Task 3: 重写 schema v2 删除路径（核心）

**Files:**
- Modify: `src/web/handlers/lifecycle/deletion.rs`（`delete_schema_v2_work_item_plan_with_cleanup`，`deletion.rs:135`）
- Create: `tests/it_web/web_coding_attempt_api/part_14.rs`
- Modify: `tests/it_web/web_coding_attempt_api.rs`（加 include）

**Interfaces:**
- Consumes: Task 1 的 `purge_plan_revisions` / `delete_issue_shared_worktree`；Task 2 的 `coding_workspace_exists_error`；现有 `purge_work_item_plan_store_artifacts`、`delete_workspace_sessions_for_entity`、`get_attempt_for_work_item_group`
- Requirement 映射：门禁拒绝、删除无残留（完整+半残）、不得误伤、错误透明

- [ ] **Step 3.1: 写失败测试——门禁拒绝（对应 tasks 1.1）**

新建 `tests/it_web/web_coding_attempt_api/part_14.rs`：
```rust
#[tokio::test]
async fn delete_work_item_plan_rejected_when_coding_workspace_exists() {
    let (app, ..) = bootstrap_confirmed_work_item_plan_group_with_attempt(...).await;
    // bootstrap 到存在 attempt 的状态
    let (status, body) = request_json(app, Method::DELETE,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/issue_work_item_plan_0001", json!({}));
    assert_eq!(status, StatusCode::CONFLICT 或对应错误状态);
    assert_eq!(body["code"], "coding_workspace_exists");
    assert_eq!(body["details"]["plan_id"], "issue_work_item_plan_0001");
    // plan 与 attempt 仍在
    assert!(plan_still_exists(...));
    assert!(attempt_still_exists(...));
}
```
（bootstrap helper 以 `part_13.rs` 的 `delete_work_item_plan_cascades_*` 为参照；需要存在 attempt 的夹具，参照 it_web 现有「group coding attempt」bootstrap。）

- [ ] **Step 3.2: 跑测试确认失败**

- [ ] **Step 3.3: 写失败测试——完整删除无残留（tasks 1.3）+ 半残删除无残留（tasks 1.4）+ 不误伤（tasks 1.6/1.7）**

同一文件加：
- `delete_work_item_plan_removes_all_artifacts_no_residual`：bootstrap 完整 group（无 attempt，播种 revisions/publications/shared-worktree/plan-store/sessions），DELETE 后断言这些路径全部不存在，且 issue.json/story-spec/design-spec/versions/repository-initializations 仍在。
- `delete_work_item_plan_succeeds_on_half_deleted_state`：bootstrap 完整 group 后手动删掉部分 WorkItem session json、删掉 worktree 目录、删掉 attempt json 但留 `.lock`，DELETE 成功且无残留。
- `delete_work_item_plan_preserves_issue_specs_and_other_plans`（如 fixture 可支撑多 plan）：删除一个 plan，其他 plan 产物不动。

- [ ] **Step 3.4: 跑测试确认失败**

- [ ] **Step 3.5: 重写 `delete_schema_v2_work_item_plan_with_cleanup`**

新结构（替换 `deletion.rs:135` 函数体）：
```rust
async fn delete_schema_v2_work_item_plan_with_cleanup(
    app_paths: &ProductAppPaths,
    store: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    _lineage: &WorkItemPlanLineage,
) -> ApiResult<()> {
    use crate::product::work_item_revision_store::WorkItemRevisionStore;

    // 门禁：存在 coding attempt 则拒绝
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    if let Some(attempt) = coding_store
        .get_attempt_for_work_item_group(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?
    {
        return Err(coding_workspace_exists_error(plan_id, &attempt.id));
    }

    // 尽力删 WorkItem session（扫描 plan_id，不依赖 bindings 完整）
    for session in store
        .list_workspace_sessions(project_id, issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .filter(|s| s.workspace_type == WorkspaceType::WorkItem
            && s.work_item_runtime_binding.as_ref().is_some_and(|b| b.plan_id == plan_id))
    {
        let _ = store.delete_workspace_sessions_for_entity(
            project_id, issue_id, &session.entity_id, WorkspaceType::WorkItem,
        ); // 容错：单项失败不阻断其余
    }

    // plan 元数据（plan json + WorkItemPlan session）
    store
        .delete_schema_v2_issue_work_item_plan_metadata(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    // revisions + publications
    WorkItemRevisionStore::new(app_paths.clone())
        .purge_plan_revisions(project_id, issue_id, plan_id)
        .map_err(product_store_api_error)?;
    // plan store drafts/compiles/outlines
    purge_work_item_plan_store_artifacts(app_paths, project_id, issue_id, plan_id)?;
    // shared-worktree
    store
        .delete_issue_shared_worktree(project_id, issue_id)
        .map_err(product_store_api_error)?;
    // attempt 残留 lock
    purge_attempt_lock_residue(app_paths, project_id, issue_id, plan_id)?;

    Ok(())
)
```
新增 helper `purge_attempt_lock_residue`：删 `coding-attempts/.coding_attempt_*.lock`、`.group-initialization-arbitration.lock`、`work-item-attempt-locks/`（用 `remove_file_if_exists` / `remove_dir_all_if_exists`，容错）。路径用 `coding_attempts_root`；实现时列出该目录下 `.lock` 文件逐个删 + 删 `work-item-attempt-locks` 子目录、`group-initializations/<plan>.json` 残留。

注意：`_lineage` 参数现在未用（旧实现用它取 active_revision_id），保留签名以减少调用处改动，或同步简化调用处。`delete_work_item_plan` 调用处（`deletion.rs:37-41`）相应调整。

- [ ] **Step 3.6: 跑测试确认通过**

`cargo test --locked --test it_web -- delete_work_item_plan`

- [ ] **Step 3.7: Commit**

```sh
git add src/web/handlers/lifecycle/deletion.rs tests/it_web/web_coding_attempt_api/part_14.rs tests/it_web/web_coding_attempt_api.rs
git commit -m "feat: work item group 删除改为门禁+尽力清理，补齐漏删产物"
```

---

### Task 4: legacy 路径门禁 + 调整现有测试

**Files:**
- Modify: `src/web/handlers/lifecycle/deletion.rs`（`delete_work_item_plan` else 分支 `:48`、`delete_work_item_with_cleanup`）
- Modify: `tests/it_web/web_coding_attempt_api/part_13.rs`（现有 `delete_work_item_plan_cascades_*`）

- [ ] **Step 4.1: legacy 路径加门禁**

在 `delete_work_item_plan` 的 else 分支（legacy，遍历 `plan.work_item_ids` 删 work item 之前）与 `delete_work_item_with_cleanup` 入口，加同一 `get_attempt_for_work_item_group` / `get_attempt_for_work_item` 门禁检查，命中则返回 `coding_workspace_exists_error`。legacy 无 schema_v2 lineage，但仍需检查 attempt 存在性。

- [ ] **Step 4.2: 调整现有 `delete_work_item_plan_cascades_children_sessions_and_attempts`**

该测试（`part_13.rs`）当前走「有 attempt 自动清理」的旧语义。新语义下有 attempt 应被门禁拒绝。调整：要么夹具不创建 attempt（测「无 attempt 时级联删 session/attempt 残留」），要么新增一个断言「有 attempt 时被拒绝」。避免测试编码旧行为导致假绿。同时确认该测试仍覆盖 plan store 产物清理（之前已补的 outline_context_index 断言）。

- [ ] **Step 4.3: 跑相关测试确认通过**

`cargo test --locked --test it_web -- delete_work_item_plan`

- [ ] **Step 4.4: Commit**

```sh
git add src/web/handlers/lifecycle/deletion.rs tests/it_web/web_coding_attempt_api/part_13.rs
git commit -m "feat: legacy 删除路径同步 coding workspace 门禁，修正既有测试语义"
```

---

### Task 5: 全量验证 + 线上数据

- [ ] **Step 5.1: 格式与 lint**

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
```

- [ ] **Step 5.2: 全量测试**

```sh
cargo test --locked --lib
cargo test --locked --test it_web
cargo test --locked --test it_product
```
预期：`large_file_guard` 仍为既有 8 文件超限（本 plan 不引入新超限；part_14.rs 控制 < 800 行）。

- [ ] **Step 5.3: 线上数据验证（用户操作）**

用户重启后端，对当前卡住的半残 group 调用删除接口。预期：attempt json 已不存在 → 门禁放行 → 尽力清理成功。验证：
```sh
find .aria/projects/project_0001/issues/issue_0001 -type f | sort
```
应只剩 `issue.json` + `story-specs/` + `design-specs/` + `versions/` + `repository-initializations/`，无 work-item-revisions / work_item_plan_* / issue-shared-worktree / coding-attempts 残留。

- [ ] **Step 5.4: 勾选 OpenSpec tasks 并 sync**

确认实现与验证完成后，勾选 `openspec/changes/harden-work-item-group-deletion/tasks.md` 各项，按 OpenSpec 流程 sync/archive。

## Self-Review

- **Spec 覆盖**：门禁拒绝（Task 3.1）→ Requirement「存在 coding workspace 时拒绝」；完整+半残无残留（Task 3.3）→ Requirement「删除无残留」；不误伤（Task 3.3）→ Requirement「不得误伤」；错误透明（Task 2.1）→ Requirement「错误透明」。全覆盖。
- **无 placeholder**：Step 3.5 的 `purge_attempt_lock_residue` 标注了要列目录删 `.lock`，实现时按 `coding_attempts_root` 实际子项处理；bootstrap helper 参照现有 part_13/it_web 夹具。
- **类型一致**：`purge_plan_revisions` / `delete_issue_shared_worktree` / `coding_workspace_exists_error` 在产出 Task 与消费 Task 名称一致。
- **large_file_guard**：新测试进 part_14.rs（独立文件），part_13.rs 仅微调现有测试，二者均远离 800 行限制。
