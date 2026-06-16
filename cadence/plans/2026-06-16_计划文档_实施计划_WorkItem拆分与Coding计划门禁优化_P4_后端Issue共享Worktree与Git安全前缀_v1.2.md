# WorkItem 拆分 P4 后端 Issue 共享 Worktree 与 Git 安全前缀 Implementation Plan

> **文档版本：** v1.2
>
> **v1.1 修订摘要：** 依据设计评审对照真实源码修订：(1) 真实 `create_branch()`（`git_workspace_service.rs:74-84`）根本不调用任何 `ensure_safe_*` 校验（仅 `delete_local_branch` 校验），故 reject 测试无法通过——任务 2 显式新增步骤：把分支名安全校验注入 `create_branch` 与 `create_worktree` 的 branch 参数路径；(2) 测试断言的 `issue_worktree_active` 在 `ProductStoreError`（`json_store.rs:8-17`，仅 Io/Json/NotFound/PathEscape）无对应变体，明确统一改用 `Io(format!("issue_worktree_active..."))`；(3) 最终验证过滤名 `issue_shared_worktree` 匹配不到 `rejects_lock_when_another_work_item_is_active` 等用例，改用更宽的过滤名以覆盖本计划全部新增用例。
>
> **v1.2 修订摘要（架构评审修复）：** 为 `create_branch`/`create_worktree` 补充幂等语义——branch 已存在或 worktree 已注册同一 branch 时返回 `Ok(())`，不同 branch 时报错；新增 `git_workspace_service_reuses_existing_issue_branch_and_worktree` 测试覆盖第二次调用复用；明确 P5 `execute_worktree_prepare` 依赖此幂等性直接复用 Issue 共享 worktree。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 增加 Issue 级共享 worktree 记录与 Git 安全前缀参数化，让 `aria/issues/*` branch 和 `.worktrees/aria-issues/*` worktree 可创建、使用和清理，同时兼容存量 `aria/work-items/*`。

**Architecture:** 本计划只建立共享 worktree 数据与 Git 安全能力，不让 Coding attempt 复用共享 worktree。Git 安全校验从硬编码单前缀改成 allow-list；生命周期 store 负责持久化 `IssueSharedWorktree` 和应用层 active lock 字段。

**Tech Stack:** Rust 1.95.0、Git CLI、LifecycleStore、Cargo integration tests、TDD。

---

## 前置交付摘要

执行本计划前确认：

- P1 已收敛 `LifecycleWorkItemRecord`。
- P3 已完成多 Work Item 生成，并且 `src/product/lifecycle_store.rs` 当前无未合并改动。
- 当前计划与 P3/P5 都会修改 `lifecycle_store.rs`，必须按 P3 → P4 → P5 串行执行。

## 计划大小边界

本计划不做以下内容：

- 不修改 `create_coding_attempt` branch 生成逻辑。
- 不修改 `worktree_path_for_attempt()`。
- 不改 Coding Workspace engine 的 worktree prepare。
- 不实现 active Work Item 启动门禁。
- 不修改前端。

如果需要让 attempt 复用 Issue worktree，停止并留给 P5。

## 文件结构

- Modify: `src/product/models.rs`
  - 新增 `IssueSharedWorktree` 与 `IssueSharedWorktreeStatus`。
- Modify: `src/product/lifecycle_store.rs`
  - 新增 shared worktree create/get/update/lock/release 方法。
- Modify: `src/product/git_workspace_service.rs`
  - 参数化安全 worktree 路径前缀。
  - 参数化 branch 前缀。
  - 允许 `aria/work-items/*` 与 `aria/issues/*`。
- Modify: `tests/it_product/product_git_workspace_service.rs`
  - 覆盖新旧 branch/worktree 前缀。
- Modify: `tests/it_product/product_lifecycle_store.rs`
  - 覆盖 shared worktree 持久化和 lock/release。

## 任务 1：Add IssueSharedWorktree Model And Store APIs

**文件：**

- Modify: `src/product/models.rs`
- Modify: `src/product/lifecycle_store.rs`
- Modify: `tests/it_product/product_lifecycle_store.rs`

- [ ] **步骤 1：编写失败态 store tests**

Append to `tests/it_product/product_lifecycle_store.rs`:

```rust
#[test]
fn persists_issue_shared_worktree_and_active_lock() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let shared = store
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: PathBuf::from("/tmp/repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");

    assert_eq!(shared.status, IssueSharedWorktreeStatus::Ready);
    assert_eq!(shared.current_active_work_item_id, None);

    let locked = store
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
        .expect("lock");
    assert_eq!(
        locked.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );

    let reloaded = store
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("reload")
        .expect("shared worktree exists");
    assert_eq!(
        reloaded.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );

    let released = store
        .release_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
        .expect("release");
    assert_eq!(released.current_active_work_item_id, None);
}

#[test]
fn rejects_lock_when_another_work_item_is_active() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    store
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: PathBuf::from("/tmp/repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");
    store
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0001")
        .expect("first lock");

    let error = store
        .try_acquire_issue_worktree_lock("project_0001", "issue_0001", "work_item_0002")
        .expect_err("second lock should fail");

    assert!(format!("{error}").contains("issue_worktree_active"));
}
```

- [ ] **步骤 2：运行 store tests 并确认失败**

运行:

```bash
cargo test --locked --test it_product persists_issue_shared_worktree_and_active_lock
cargo test --locked --test it_product rejects_lock_when_another_work_item_is_active
```

预期：编译失败，因为 model and store APIs do not exist.

- [ ] **步骤 3：添加 models and store methods**

在 `src/product/models.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSharedWorktreeStatus {
    NotCreated,
    Ready,
    Running,
    Blocked,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueSharedWorktree {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub base_branch: String,
    pub status: IssueSharedWorktreeStatus,
    pub current_active_work_item_id: Option<String>,
    pub last_completed_work_item_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
```

在 `src/product/lifecycle_store.rs`, add `UpsertIssueSharedWorktreeInput` and methods:

```rust
pub fn upsert_issue_shared_worktree(
    &self,
    input: UpsertIssueSharedWorktreeInput,
) -> Result<IssueSharedWorktree, ProductStoreError>

pub fn get_issue_shared_worktree(
    &self,
    project_id: &str,
    issue_id: &str,
) -> Result<Option<IssueSharedWorktree>, ProductStoreError>

pub fn try_acquire_issue_worktree_lock(
    &self,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
) -> Result<IssueSharedWorktree, ProductStoreError>

pub fn release_issue_worktree_lock(
    &self,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
) -> Result<IssueSharedWorktree, ProductStoreError>
```

Persist record at:

```text
projects/{project_id}/issues/{issue_id}/issue-shared-worktree.json
```

> **🔴 错误变体说明：** 步骤 1 的 `rejects_lock_when_another_work_item_is_active` 断言错误信息含 `issue_worktree_active`，但 `ProductStoreError`（`src/product/json_store.rs:8-17`）当前只有 `Io / Json / NotFound / PathEscape` 四个变体，无对应专用变体。本计划统一采用**复用 `Io` 变体**的写法：`try_acquire_issue_worktree_lock` 在已被其它 Work Item 占用时返回 `ProductStoreError::Io(format!("issue_worktree_active: issue {issue_id} locked by {active_id}"))`，使 `format!("{error}")` 包含 `issue_worktree_active`。（如后续需要可读性更强的领域错误，可在专门的重构计划里新增 `Conflict` 变体，本计划不引入。）

- [ ] **步骤 4：运行 store tests 并确认通过**

Run the two commands from Step 2 again.

预期：两条测试都通过。

## 任务 2：Parameterize Git Worktree And Branch Safety

**文件：**

- Modify: `src/product/git_workspace_service.rs`
- Modify: `tests/it_product/product_git_workspace_service.rs`

- [ ] **步骤 1：编写失败态 Git safety tests**

追加:

```rust
#[tokio::test]
async fn git_workspace_service_allows_issue_shared_branch_and_worktree_prefix() {
    let root = tempdir().expect("tempdir");
    let repo = root.path().join("repo");
    init_repo(&repo);
    let service = GitWorkspaceService::new();

    service
        .create_branch(&repo, "aria/issues/issue_0001", "HEAD")
        .await
        .expect("create issue branch");
    let worktree = repo
        .join(".worktrees")
        .join("aria-issues")
        .join("issue_0001");
    service
        .create_worktree(&repo, "aria/issues/issue_0001", &worktree)
        .await
        .expect("create issue worktree");

    assert!(worktree.join(".git").exists());
}

#[tokio::test]
async fn git_workspace_service_still_rejects_unsafe_issue_branch_names() {
    let root = tempdir().expect("tempdir");
    let repo = root.path().join("repo");
    init_repo(&repo);
    let service = GitWorkspaceService::new();

    let error = service
        .create_branch(&repo, "aria/issues/../main", "HEAD")
        .await
        .expect_err("unsafe branch rejected");

    assert!(format!("{error}").contains("outside allowed aria branch prefixes"));
}

#[tokio::test]
async fn git_workspace_service_reuses_existing_issue_branch_and_worktree() {
    let root = tempdir().expect("tempdir");
    let repo = root.path().join("repo");
    init_repo(&repo);
    let service = GitWorkspaceService::new();

    service
        .create_branch(&repo, "aria/issues/issue_0001", "HEAD")
        .await
        .expect("create issue branch first time");
    let worktree = repo
        .join(".worktrees")
        .join("aria-issues")
        .join("issue_0001");
    service
        .create_worktree(&repo, "aria/issues/issue_0001", &worktree)
        .await
        .expect("create issue worktree first time");

    // 第二次调用必须幂等复用，不得报错。
    service
        .create_branch(&repo, "aria/issues/issue_0001", "HEAD")
        .await
        .expect("reuse existing issue branch");
    service
        .create_worktree(&repo, "aria/issues/issue_0001", &worktree)
        .await
        .expect("reuse existing issue worktree");

    assert!(worktree.join(".git").exists());
}
```

- [ ] **步骤 2：运行 Git safety tests 并确认失败**

运行:

```bash
cargo test --locked --test it_product git_workspace_service_allows_issue_shared_branch_and_worktree_prefix
cargo test --locked --test it_product git_workspace_service_still_rejects_unsafe_issue_branch_names
cargo test --locked --test it_product git_workspace_service_reuses_existing_issue_branch_and_worktree
```

预期：第一条测试失败，因为 only `aria/work-items/` and `.worktrees/aria-work-items` are allowed；幂等复用测试同样失败，因为当前实现会直接再次创建。

- [ ] **步骤 3：实现 prefix allow-list**

在 `src/product/git_workspace_service.rs`, replace hard-coded helpers with allow-list helpers:

```rust
const SAFE_WORKTREE_PREFIXES: &[&str] = &["aria-work-items", "aria-issues"];
const SAFE_BRANCH_PREFIXES: &[&str] = &["aria/work-items/", "aria/issues/"];
```

`ensure_safe_worktree_path()` must allow normalized paths under either:

```text
{repo}/.worktrees/aria-work-items
{repo}/.worktrees/aria-issues
```

`ensure_safe_attempt_branch_name()` can be renamed to `ensure_safe_aria_branch_name()` and must reject:

- branch names containing `..`
- branch names starting with `/`
- branch names not starting with one of the allowed prefixes

Keep old callers working by updating internal references only.

> **🔴 阻塞：必须把分支名安全校验注入 `create_branch`（以及 `create_worktree` 的 branch 参数路径）。** 经核对真实源码，`create_branch()`（`src/product/git_workspace_service.rs:74-84`）当前**不调用任何 `ensure_safe_*` 校验**——只有 `delete_local_branch()`（:134）调用了 `ensure_safe_attempt_branch_name()`，`create_worktree()`（:93）只调用 `ensure_safe_worktree_path()` 而不校验 branch 名。因此步骤 1 的 `git_workspace_service_still_rejects_unsafe_issue_branch_names`（断言 `create_branch(&repo, "aria/issues/../main", ...)` 报错含 `outside allowed aria branch prefixes`）在当前实现下**永远无法通过**。本步骤必须显式完成以下注入，这超出"仅重命名 helper + 改 allow-list"的范围：

```rust
pub async fn create_branch(
    &self,
    repo_path: &Path,
    branch_name: &str,
    base_branch: &str,
) -> Result<(), GitWorkspaceError> {
    ensure_git_repo(repo_path).await?;
    ensure_safe_aria_branch_name(branch_name)?; // 新增注入
    self.run_git(repo_path, &["branch", branch_name, base_branch])
        .await
        .map(|_| ())
}

pub async fn create_worktree(
    &self,
    repo_path: &Path,
    branch_name: &str,
    worktree_path: &Path,
) -> Result<(), GitWorkspaceError> {
    ensure_git_repo(repo_path).await?;
    ensure_safe_aria_branch_name(branch_name)?; // 新增注入：worktree 的 branch 参数路径
    ensure_safe_worktree_path(repo_path, worktree_path)?;
    // ...既有逻辑...
}
```

- **幂等复用要求（v1.2）：** P5 要求同一 Issue 下多次 Coding attempt 复用同一个 `aria/issues/{issue_id}` branch 与 `.worktrees/aria-issues/{issue_id}` worktree，因此 `create_branch` 与 `create_worktree` 必须具备幂等语义：
  - `create_branch` 在执行 `git branch` 前先检查 branch 是否已存在（例如 `git show-ref --verify --quiet refs/heads/{branch}`）。若已存在，返回 `Ok(())`；仅当创建失败且 branch 不存在时才返回错误。
  - `create_worktree` 在执行 `git worktree add` 前先运行 `git worktree list --porcelain`；若目标 path 已注册且绑定同一 branch，返回 `Ok(())`；若 path 已注册但绑定不同 branch，返回错误。
  - P5 的 `execute_worktree_prepare` 依赖此幂等性：它只需按 Issue branch 计算 worktree path，然后直接调用 `create_branch`/`create_worktree`，无需额外实现复用逻辑。

并确认 `ensure_safe_aria_branch_name()` 在 prefix 不匹配时返回的错误信息包含 `outside allowed aria branch prefixes`，使 reject 测试断言成立。

- [ ] **步骤 4：Run Git safety tests and old Git tests**

运行:

```bash
cargo test --locked --test it_product git_workspace_service
```

预期：旧的 `aria/work-items/*` test still passes and new `aria/issues/*` tests pass.

## 任务 3：Store Last Completed Work Item

**文件：**

- Modify: `src/product/lifecycle_store.rs`
- Modify: `tests/it_product/product_lifecycle_store.rs`

- [ ] **步骤 1：编写失败态 completion marker test**

追加:

```rust
#[test]
fn marks_issue_shared_worktree_last_completed_work_item() {
    let root = tempdir().expect("tempdir");
    let store = LifecycleStore::new(ProductAppPaths::new(root.path().join(".aria")));
    store
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: PathBuf::from("/tmp/repo/.worktrees/aria-issues/issue_0001"),
            base_branch: "main".to_string(),
        })
        .expect("shared worktree");

    let updated = store
        .mark_issue_worktree_completed_item("project_0001", "issue_0001", "work_item_0001")
        .expect("mark completed");

    assert_eq!(
        updated.last_completed_work_item_id.as_deref(),
        Some("work_item_0001")
    );
}
```

- [ ] **步骤 2：运行 completion marker test 并确认失败**

运行:

```bash
cargo test --locked --test it_product marks_issue_shared_worktree_last_completed_work_item
```

预期: method does not exist.

- [ ] **步骤 3：添加 completion marker method**

Add:

```rust
pub fn mark_issue_worktree_completed_item(
    &self,
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
) -> Result<IssueSharedWorktree, ProductStoreError>
```

The method updates `last_completed_work_item_id`, clears `current_active_work_item_id` if it matches the completed item, sets status to `Ready`, and updates `updated_at`.

- [ ] **步骤 4：运行 completion marker test 并确认通过**

重新运行步骤 2 的命令。

预期：通过。

## 最终验证

运行:

```bash
# 注意：过滤名 issue_shared_worktree 匹配不到 rejects_lock_when_another_work_item_is_active
# （该用例名不含此子串）。改用更宽的 issue 过滤名以覆盖本计划全部新增 store 用例：
#   persists_issue_shared_worktree_and_active_lock
#   rejects_lock_when_another_work_item_is_active
#   marks_issue_shared_worktree_last_completed_work_item
cargo test --locked --test it_product issue
cargo test --locked --test it_product git_workspace_service
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

> **过滤名说明：** 若担心 `issue` 过滤名命中过宽（其它含 `issue` 的用例），可改为显式追加用例名分别运行：`cargo test --locked --test it_product rejects_lock_when_another_work_item_is_active` 等三条，确保本计划新增的三个 store 用例与两个 Git safety 用例全部覆盖。

预期:

- Shared worktree store tests pass.
- Git workspace service tests pass.
- Formatting, clippy and check pass.

## 提交

```bash
git add src/product/models.rs src/product/lifecycle_store.rs src/product/git_workspace_service.rs tests/it_product/product_lifecycle_store.rs tests/it_product/product_git_workspace_service.rs
git commit -m "feat: add issue shared worktree records"
```
