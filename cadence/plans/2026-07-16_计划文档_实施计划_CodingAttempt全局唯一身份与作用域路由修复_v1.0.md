# Coding Attempt 全局唯一身份与作用域路由修复实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让新 Coding Attempt 使用全局唯一 UUID，并让 Coding Workspace、REST API 与 WebSocket 使用 Project、Issue、Attempt 作用域地址，同时无损兼容现有同名历史 Attempt。

**Architecture:** Product Store 使用 UUID 创建新 Attempt，并保留单 ID 全局查询作为旧地址兼容入口；规范业务链路使用 `get_attempt(project_id, issue_id, attempt_id)` 精确读取。后端新增作用域 REST/WS 路由，前端以 `CodingAttemptAddress` 在 Workbench、Router、API Client、Workspace Page 与 WebSocket Hook 之间传递完整身份，旧页面地址只负责唯一匹配跳转或显示歧义冲突。

**Tech Stack:** Rust 2024、Axum 0.8、serde、uuid v4、React 19、TypeScript、TanStack Router、Zustand、Vitest、Cargo integration tests。

## Global Constraints

- 新 Attempt ID 格式固定为 `coding_attempt_<32位UUID十六进制>`。
- 不新增第三方依赖；复用现有 `uuid = { features = ["v4"] }`。
- 不迁移、不重命名、不删除真实 `.aria` 历史 Attempt、Role Run、Timeline、Chat Entry、Artifact、JSONL 或 Issue 级序列文件。
- 新业务链路不得调用单 ID 全局扫描；单 ID 查询只允许旧 REST、旧 WebSocket 与旧页面地址使用。
- 精确未找到使用 `coding_attempt_not_found`；旧单 ID 多匹配使用 `coding_attempt_ambiguous`；路径和记录不一致使用 `coding_attempt_scope_mismatch`。
- Story Spec 与 Design Spec 继续使用 Workspace Session 路由；只修改 Work Item / Work Item Group 的 Coding Workspace 地址。
- 严格执行 TDD：每个任务先写失败测试、确认 RED、最小实现、确认 GREEN、再提交。
- Rust 使用宿主机 Cargo；禁止 Docker；禁止给任何 Cargo 命令添加 `-j 1`。
- 定向运行 `src/lib.rs` 单元测试时使用 `cargo test --locked --lib <过滤名>`；集成测试使用明确的 `--test it_product` 或 `--test it_web` 目标。
- 前端只使用 `pnpm`。

---

## File Structure

### 后端身份与存储

- Modify: `src/product/json_store.rs` — 增加结构化 Ambiguous 与 IdentityMismatch 错误。
- Modify: `src/product/coding_attempt_store/mod.rs` — 单 ID 兼容查询返回结构化歧义错误。
- Modify: `src/product/coding_attempt_store/attempt.rs` — UUID 分配、精确身份校验、删除旧序列分配逻辑。
- Modify: `src/product/coding_attempt_store/group.rs` — Work Item Group 使用同一 UUID 分配函数。
- Modify: `tests/it_product/product_coding_attempt_store/part_01.rs` — UUID、跨 Issue、历史同名与身份不一致回归。
- Modify: `tests/it_product/product_coding_attempt_store/part_02.rs` — 增加可指定 Issue 的输入夹具。

### REST API

- Create: `src/web/handlers/coding/scope.rs` — 统一解析可选 Project/Issue 路径参数并精确或兼容加载 Attempt。
- Modify: `src/web/handlers/coding.rs` — 所有 Attempt 操作接收统一路径结构。
- Modify: `src/web/handlers/support.rs` — Product Store 错误映射。
- Modify: `src/web/error.rs` — 409 状态映射。
- Modify: `src/web/app.rs` — 注册作用域 REST 路由，保留旧路由。
- Modify: `src/web/types.rs` — `CodingAttemptDto` 增加 `project_id`、`issue_id`。
- Modify: `src/web/handlers/dto.rs` — 填充 DTO 归属字段。
- Modify: `tests/it_web/web_coding_attempt_api/part_01.rs` 至 `part_04.rs` — 改用动态 UUID 和作用域接口。
- Modify: `tests/it_web/web_coding_attempt_api/part_05.rs` — 增加 UUID 与作用域 URI 测试辅助函数。
- Create: `tests/it_web/web_coding_attempt_api/part_06.rs` — 作用域读取、旧地址歧义和 Scope mismatch 回归。
- Modify: `tests/it_web/web_coding_attempt_api.rs` — 引入 `part_06.rs`。

### WebSocket

- Modify: `src/web/coding_ws_handler/socket.rs` — 增加作用域入口、真实错误映射和统一连接实现。
- Modify: `src/web/coding_ws_handler/protocol.rs` — Session State 增加 Project/Issue。
- Modify: `src/web/coding_ws_handler/state.rs` — 填充 Project/Issue。
- Modify: `src/web/app.rs` — 注册作用域 WebSocket 路由。
- Create: `tests/it_web/web_coding_ws_handler/part_11.rs` — 作用域连接、旧地址歧义和真正未找到回归。
- Modify: `tests/it_web/web_coding_ws_handler.rs` — 引入 `part_11.rs`。

### 前端规范地址

- Modify: `web/src/api/types/coding.ts` — `CodingAttemptAddress`、DTO 归属字段和 Session State 归属字段。
- Modify: `web/src/api/client.ts` — 规范作用域 API 与旧 Snapshot 兼容 API。
- Modify: `web/src/state/coding-workspace-store.ts` — 从 Session State 保存 Project/Issue。
- Modify: `web/src/hooks/useCodingWorkspaceWs.ts` — 使用完整地址建立 WebSocket。
- Modify: `web/src/pages/CodingWorkspacePage.tsx` — 接收完整地址并传递给删除、Diff、执行计划组件。
- Modify: `web/src/pages/CodingWorkspaceArtifacts.tsx` — 使用作用域 Diff API。
- Modify: `web/src/pages/CodingWorkspaceReports.tsx` — 使用作用域 Execution Plan API。
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx` — 创建或复用 Attempt 后传递完整地址。
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx` — 默认跳转使用规范页面地址。
- Modify: `web/src/app-shell.tsx` — Callback 类型改为 `CodingAttemptAddress`。
- Modify: `web/src/router.tsx` — 注册并渲染规范 Coding Workspace 路由。
- Modify: 对应 API、Hook、Page、Lifecycle 与 Router 测试和测试工具。

### 旧页面兼容

- Create: `web/src/pages/LegacyCodingWorkspaceRedirect.tsx` — 唯一匹配跳转、歧义提示和返回 Workbench。
- Create: `web/src/pages/LegacyCodingWorkspaceRedirect.test.tsx` — 成功、歧义和未找到测试。
- Modify: `web/src/router.tsx` — 旧地址挂载兼容组件。
- Modify: `web/src/router.test.tsx` — 新旧路由与 Provider Guard 回归。

---

### Task 1: Product Store 全局 UUID 与结构化身份错误

**Files:**
- Modify: `src/product/json_store.rs:7-17`
- Modify: `src/product/coding_attempt_store/mod.rs:37-68`
- Modify: `src/product/coding_attempt_store/attempt.rs:18-160, 360-390, 620-645`
- Modify: `src/product/coding_attempt_store/group.rs:73-75`
- Modify: `tests/it_product/product_coding_attempt_store/part_01.rs:27-145`
- Modify: `tests/it_product/product_coding_attempt_store/part_02.rs:112-145`

**Interfaces:**
- Consumes: `Uuid::new_v4()`、`CodingAttemptStore::attempt_path`、现有 `validate_relative_id`。
- Produces: `CodingAttemptStore::allocate_coding_attempt_id(&self) -> String`、结构化 `ProductStoreError::Ambiguous`、`ProductStoreError::IdentityMismatch`、经过身份校验的 `get_attempt`。

- [ ] **Step 1: 增加可指定 Issue 的测试输入与 UUID 断言辅助函数**

在 `tests/it_product/product_coding_attempt_store/part_02.rs` 将现有 `create_input` 改为：

```rust
fn create_input(work_item_id: &str) -> CreateCodingAttemptInput {
    create_input_for("project_0001", "issue_0001", work_item_id)
}

fn create_input_for(
    project_id: &str,
    issue_id: &str,
    work_item_id: &str,
) -> CreateCodingAttemptInput {
    CreateCodingAttemptInput {
        project_id: project_id.to_string(),
        issue_id: issue_id.to_string(),
        work_item_id: work_item_id.to_string(),
        base_branch: "main".to_string(),
        branch_name: format!("aria/issues/{issue_id}"),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Fake),
            review_rounds: 1,
        },
        max_auto_rework: 2,
    }
}

fn assert_global_coding_attempt_id(id: &str) {
    let uuid = id
        .strip_prefix("coding_attempt_")
        .expect("coding attempt prefix");
    assert_eq!(uuid.len(), 32);
    uuid::Uuid::parse_str(uuid).expect("valid UUID coding attempt id");
}
```

- [ ] **Step 2: 写 UUID、跨 Issue 唯一、历史同名和身份不一致失败测试**

在 `part_01.rs` 更新前三个创建测试，不再断言顺序号，并新增：

```rust
#[test]
fn coding_attempt_ids_are_global_across_issues() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));

    let first = store
        .create_attempt(create_input_for("project_0001", "issue_0001", "work_item_0001"))
        .expect("first attempt");
    let second = store
        .create_attempt(create_input_for("project_0001", "issue_0002", "work_item_0001"))
        .expect("second attempt");

    assert_global_coding_attempt_id(&first.id);
    assert_global_coding_attempt_id(&second.id);
    assert_ne!(first.id, second.id);
}

#[test]
fn scoped_lookup_reads_duplicate_legacy_ids_and_global_lookup_is_ambiguous() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let template = store
        .create_attempt(create_input("template_work_item"))
        .expect("template attempt");

    let mut first = template.clone();
    first.id = "coding_attempt_0001".to_string();
    first.issue_id = "issue_0001".to_string();
    first.work_item_id = "work_item_issue_1".to_string();
    store.save_coding_attempt(&first).expect("save first legacy attempt");

    let mut second = first.clone();
    second.issue_id = "issue_0002".to_string();
    second.work_item_id = "work_item_issue_2".to_string();
    store.save_coding_attempt(&second).expect("save second legacy attempt");

    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("first scoped attempt")
            .work_item_id,
        "work_item_issue_1"
    );
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0002", "coding_attempt_0001")
            .expect("second scoped attempt")
            .work_item_id,
        "work_item_issue_2"
    );
    assert!(matches!(
        store.get_attempt_by_id("coding_attempt_0001"),
        Err(ProductStoreError::Ambiguous {
            kind: "coding_attempt",
            ..
        })
    ));
}

#[test]
fn scoped_lookup_rejects_record_identity_mismatch() {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("attempt");
    let mismatch_path = root.path().join(
        ".aria/projects/project_0001/issues/issue_0002/coding-attempts/coding_attempt_legacy.json",
    );
    std::fs::create_dir_all(mismatch_path.parent().expect("parent")).expect("create parent");
    std::fs::write(
        &mismatch_path,
        serde_json::to_string_pretty(&attempt).expect("serialize attempt"),
    )
    .expect("write mismatch");

    assert!(matches!(
        store.get_attempt("project_0001", "issue_0002", "coding_attempt_legacy"),
        Err(ProductStoreError::IdentityMismatch {
            kind: "coding_attempt",
            ..
        })
    ));
}
```

同时在 `part_01.rs` 导入 `ProductStoreError`，并把删除后重建测试改为断言两个 UUID 合法且不相等，不再删除 `.meta/coding-attempt-sequence.json`。

- [ ] **Step 3: 运行 Product Store 测试确认 RED**

Run:

```bash
cargo test --locked --test it_product product_coding_attempt_store::coding_attempt_ids_are_global_across_issues
cargo test --locked --test it_product product_coding_attempt_store::scoped_lookup_reads_duplicate_legacy_ids_and_global_lookup_is_ambiguous
cargo test --locked --test it_product product_coding_attempt_store::scoped_lookup_rejects_record_identity_mismatch
```

Expected: FAIL；当前 ID 仍为 Issue 内顺序号，歧义仍是 `Io(String)`，固定路径记录不做身份校验。

- [ ] **Step 4: 增加结构化 Product Store 错误**

在 `src/product/json_store.rs` 的 `ProductStoreError` 增加：

```rust
#[error("product_store_ambiguous: {kind} {id}")]
Ambiguous { kind: &'static str, id: String },
#[error("product_store_identity_mismatch: {kind} {id}")]
IdentityMismatch { kind: &'static str, id: String },
```

在 `find_attempt_by_id` 的第二次匹配处返回：

```rust
return Err(ProductStoreError::Ambiguous {
    kind: "coding_attempt",
    id: attempt_id.to_string(),
});
```

- [ ] **Step 5: 使用 UUID 分配新 ID 并校验固定路径身份**

在 `attempt.rs` 导入 `uuid::Uuid`，把分配方法替换为：

```rust
pub(crate) fn allocate_coding_attempt_id(&self) -> String {
    format!("coding_attempt_{}", Uuid::new_v4().simple())
}
```

单 Work Item 与 Group 创建均使用：

```rust
let id = self.allocate_coding_attempt_id();
```

删除 `record_coding_attempt_sequence_at_least`、`coding_attempt_sequence_path`、`max_existing_coding_attempt_sequence`、`coding_attempt_sequence_from_id`，并删除 `delete_attempt` 中的序列记录分支。不要删除磁盘上的历史 `.meta` 文件。

把 `get_attempt` 的结尾替换为：

```rust
let attempt: CodingExecutionAttempt = read_json(&path)?;
if attempt.project_id != project_id
    || attempt.issue_id != issue_id
    || attempt.id != attempt_id
{
    return Err(ProductStoreError::IdentityMismatch {
        kind: "coding_attempt",
        id: attempt_id.to_string(),
    });
}
Ok(attempt)
```

- [ ] **Step 6: 运行 Product Store 定向回归确认 GREEN**

Run:

```bash
cargo test --locked --test it_product product_coding_attempt_store
```

Expected: PASS；所有创建 ID 为 UUID，历史固定顺序 ID 夹具继续可读。

- [ ] **Step 7: 提交 Task 1**

```bash
git add src/product/json_store.rs src/product/coding_attempt_store/mod.rs src/product/coding_attempt_store/attempt.rs src/product/coding_attempt_store/group.rs tests/it_product/product_coding_attempt_store/part_01.rs tests/it_product/product_coding_attempt_store/part_02.rs
git commit -m "fix: make coding attempt identities global"
```

---

### Task 2: 作用域 REST API、DTO 与错误语义

**Files:**
- Create: `src/web/handlers/coding/scope.rs`
- Modify: `src/web/handlers/coding.rs`
- Modify: `src/web/handlers/support.rs`
- Modify: `src/web/error.rs`
- Modify: `src/web/app.rs`
- Modify: `src/web/types.rs`
- Modify: `src/web/handlers/dto.rs`
- Modify: `tests/it_web/web_coding_attempt_api.rs`
- Modify: `tests/it_web/web_coding_attempt_api/part_01.rs`
- Modify: `tests/it_web/web_coding_attempt_api/part_02.rs`
- Modify: `tests/it_web/web_coding_attempt_api/part_03.rs`
- Modify: `tests/it_web/web_coding_attempt_api/part_04.rs`
- Modify: `tests/it_web/web_coding_attempt_api/part_05.rs`
- Create: `tests/it_web/web_coding_attempt_api/part_06.rs`

**Interfaces:**
- Consumes: Task 1 的 `get_attempt`、`get_attempt_by_id`、Ambiguous 与 IdentityMismatch。
- Produces: `CodingAttemptRoutePath`、`CodingAttemptArtifactRoutePath`、`resolve_coding_attempt`、作用域 REST 路由、包含 Project/Issue 的 `CodingAttemptDto`。

- [ ] **Step 1: 添加 API 测试辅助函数并把现有创建断言改为动态 UUID**

在 `part_05.rs` 增加：

```rust
pub(crate) fn assert_global_attempt_id(value: &Value) -> String {
    let id = value["attempt_id"].as_str().expect("attempt id");
    let uuid = id.strip_prefix("coding_attempt_").expect("attempt prefix");
    assert_eq!(uuid.len(), 32);
    uuid::Uuid::parse_str(uuid).expect("valid attempt UUID");
    id.to_string()
}

pub(crate) fn scoped_attempt_uri(attempt_id: &str, suffix: &str) -> String {
    format!(
        "/api/projects/project_0001/issues/issue_0001/coding-attempts/{attempt_id}{suffix}"
    )
}
```

更新 `part_01.rs` 至 `part_04.rs`：创建响应先用 `assert_global_attempt_id` 取得 ID，后续 Snapshot、Diff、Abort、Delete、Artifact 和 Execution Plan 调用使用 `scoped_attempt_uri`。只在明确测试旧接口兼容时保留 `/api/coding-attempts/{attempt_id}`。

- [ ] **Step 2: 写作用域读取、旧接口歧义和 Scope mismatch API 失败测试**

创建 `part_06.rs`：

```rust
#[tokio::test]
async fn scoped_coding_attempt_api_loads_exact_attempt_and_legacy_route_reports_ambiguity() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item(app.clone(), repo.path()).await;

    let (_, created) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    let attempt_id = assert_global_attempt_id(&created);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut duplicate = store
        .get_attempt("project_0001", "issue_0001", &attempt_id)
        .expect("original attempt");
    duplicate.issue_id = "issue_0002".to_string();
    store.save_coding_attempt(&duplicate).expect("duplicate legacy scope");

    let (scoped_status, scoped) = request_json(
        app.clone(),
        Method::GET,
        &scoped_attempt_uri(&attempt_id, ""),
        json!({}),
    )
    .await;
    assert_eq!(scoped_status, StatusCode::OK);
    assert_eq!(scoped["attempt"]["project_id"], "project_0001");
    assert_eq!(scoped["attempt"]["issue_id"], "issue_0001");

    let (legacy_status, legacy) = request_json(
        app,
        Method::GET,
        &format!("/api/coding-attempts/{attempt_id}"),
        json!({}),
    )
    .await;
    assert_eq!(legacy_status, StatusCode::CONFLICT);
    assert_eq!(legacy["code"], "coding_attempt_ambiguous");
}

#[tokio::test]
async fn scoped_coding_attempt_api_reports_scope_mismatch() {
    let root = tempdir().expect("root");
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("attempt");
    let source = root.path().join(format!(
        ".aria/projects/project_0001/issues/issue_0001/coding-attempts/{}.json",
        attempt.id
    ));
    let target = root.path().join(format!(
        ".aria/projects/project_0001/issues/issue_0002/coding-attempts/{}.json",
        attempt.id
    ));
    std::fs::create_dir_all(target.parent().expect("parent")).expect("create parent");
    std::fs::copy(source, target).expect("copy mismatched record");

    let (status, body) = request_json(
        app,
        Method::GET,
        &format!(
            "/api/projects/project_0001/issues/issue_0002/coding-attempts/{}",
            attempt.id
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["code"], "coding_attempt_scope_mismatch");
}
```

在 `web_coding_attempt_api.rs` 追加 `include!("web_coding_attempt_api/part_06.rs");`。

- [ ] **Step 3: 运行新 API 测试确认 RED**

Run:

```bash
cargo test --locked --test it_web scoped_coding_attempt_api
```

Expected: FAIL；作用域路由未注册，DTO 不含 Project/Issue，Ambiguous 与 IdentityMismatch 尚未映射为 409。

- [ ] **Step 4: 新增统一路由路径解析模块**

创建 `src/web/handlers/coding/scope.rs`：

```rust
use serde::Deserialize;

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::web::error::{ApiError, ApiResult};

use super::super::support::product_store_api_error;

#[derive(Debug, Deserialize)]
pub(crate) struct CodingAttemptRoutePath {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub issue_id: Option<String>,
    pub attempt_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodingAttemptArtifactRoutePath {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub issue_id: Option<String>,
    pub attempt_id: String,
    pub artifact_id: String,
}

pub(crate) fn resolve_coding_attempt(
    store: &CodingAttemptStore,
    project_id: Option<&str>,
    issue_id: Option<&str>,
    attempt_id: &str,
) -> ApiResult<CodingExecutionAttempt> {
    match (project_id, issue_id) {
        (Some(project_id), Some(issue_id)) => store
            .get_attempt(project_id, issue_id, attempt_id)
            .map_err(product_store_api_error),
        (None, None) => store
            .get_attempt_by_id(attempt_id)
            .map_err(product_store_api_error),
        _ => Err(ApiError::validation(
            "invalid_coding_attempt_scope",
            "project_id and issue_id must be provided together",
        )),
    }
}
```

在 `coding.rs` 增加 `mod scope;`，所有非 Artifact handler 使用 `Path(path): Path<CodingAttemptRoutePath>`，并通过：

```rust
let attempt = resolve_coding_attempt(
    &coding_store,
    path.project_id.as_deref(),
    path.issue_id.as_deref(),
    &path.attempt_id,
)?;
```

Artifact handler 使用 `CodingAttemptArtifactRoutePath`，先保存 `artifact_id`，再调用同一 resolver。

- [ ] **Step 5: 注册作用域 REST 路由并保留旧路由**

在 `src/web/app.rs` 现有创建接口之后增加：

```rust
.route(
    "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}",
    get(handlers::get_coding_attempt).delete(handlers::delete_coding_attempt),
)
.route(
    "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/diff",
    get(handlers::coding_attempt_diff),
)
.route(
    "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/abort",
    post(handlers::abort_coding_attempt),
)
.route(
    "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/execution-plan/confirm",
    post(handlers::confirm_work_item_execution_plan),
)
.route(
    "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/execution-plan/change-request",
    post(handlers::request_work_item_execution_plan_change),
)
.route(
    "/api/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}/artifacts/{artifact_id}",
    get(handlers::coding_attempt_artifact_content),
)
```

不要删除现有 `/api/coding-attempts/{attempt_id}` 路由。

- [ ] **Step 6: 映射 DTO 与结构化错误**

在 Rust `CodingAttemptDto` 和 `coding_attempt_dto` 增加：

```rust
pub project_id: String,
pub issue_id: String,
```

```rust
project_id: attempt.project_id.clone(),
issue_id: attempt.issue_id.clone(),
```

在 `product_store_api_error` 增加：

```rust
ProductStoreError::Ambiguous {
    kind: "coding_attempt",
    id,
} => ApiError::runtime(
    "coding_attempt_ambiguous",
    "coding attempt matches multiple issues",
    json!({"attempt_id": id}),
),
ProductStoreError::IdentityMismatch {
    kind: "coding_attempt",
    id,
} => ApiError::runtime(
    "coding_attempt_scope_mismatch",
    "coding attempt does not belong to the requested project and issue",
    json!({"attempt_id": id}),
),
```

在 `ApiError::into_response` 的 CONFLICT 分支加入：

```rust
"coding_attempt_ambiguous" | "coding_attempt_scope_mismatch"
```

- [ ] **Step 7: 运行 REST API 定向回归确认 GREEN**

Run:

```bash
cargo test --locked --test it_web web_coding_attempt_api
```

Expected: PASS；新接口使用精确作用域，旧接口仅在唯一匹配时成功。

- [ ] **Step 8: 提交 Task 2**

```bash
git add src/web/handlers/coding/scope.rs src/web/handlers/coding.rs src/web/handlers/support.rs src/web/error.rs src/web/app.rs src/web/types.rs src/web/handlers/dto.rs tests/it_web/web_coding_attempt_api.rs tests/it_web/web_coding_attempt_api/part_01.rs tests/it_web/web_coding_attempt_api/part_02.rs tests/it_web/web_coding_attempt_api/part_03.rs tests/it_web/web_coding_attempt_api/part_04.rs tests/it_web/web_coding_attempt_api/part_05.rs tests/it_web/web_coding_attempt_api/part_06.rs
git commit -m "feat: add scoped coding attempt api"
```

---

### Task 3: 作用域 WebSocket 与真实协议错误

**Files:**
- Modify: `src/web/coding_ws_handler/socket.rs:38-70`
- Modify: `src/web/coding_ws_handler/protocol.rs:20-60`
- Modify: `src/web/coding_ws_handler/state.rs:100-135`
- Modify: `src/web/app.rs:202-212`
- Create: `tests/it_web/web_coding_ws_handler/part_11.rs`
- Modify: `tests/it_web/web_coding_ws_handler.rs`

**Interfaces:**
- Consumes: Task 1 的精确/兼容读取和结构化错误。
- Produces: `scoped_coding_ws`、统一 `handle_coding_socket` lookup 参数、包含 Project/Issue 的 `CodingSessionState`。

- [ ] **Step 1: 写作用域连接、旧地址歧义与真正未找到失败测试**

创建 `part_11.rs`，使用现有 `app_with_group_attempt` 夹具验证作用域连接，并使用纯 Store 夹具验证旧地址歧义：

```rust
#[tokio::test]
async fn scoped_coding_ws_returns_session_state_for_exact_issue() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let app = app_with_group_attempt(root.path());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });

    let url = format!(
        "ws://{addr}/ws/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001"
    );
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingSessionState {
            project_id,
            issue_id,
            attempt_id,
            ..
        } => {
            assert_eq!(project_id, "project_0001");
            assert_eq!(issue_id, "issue_0001");
            assert_eq!(attempt_id, "coding_attempt_0001");
        }
        other => panic!("expected session state, got {other:?}"),
    }
    server.abort();
}

#[tokio::test]
async fn legacy_coding_ws_reports_ambiguous_instead_of_not_found() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let template = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("template");
    for issue_id in ["issue_0001", "issue_0002"] {
        let mut legacy = template.clone();
        legacy.id = "coding_attempt_0001".to_string();
        legacy.issue_id = issue_id.to_string();
        store.save_coding_attempt(&legacy).expect("legacy attempt");
    }
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    let (mut ws, _) = connect_async(format!(
        "ws://{addr}/ws/coding-attempts/coding_attempt_0001"
    ))
    .await
    .expect("connect ws");

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingProtocolError { code, .. } => {
            assert_eq!(code, "coding_attempt_ambiguous");
        }
        other => panic!("expected protocol error, got {other:?}"),
    }
    server.abort();
}
```

再增加 scoped missing 测试，连接不存在 ID 后断言 `coding_attempt_not_found`。在入口文件追加 `include!("web_coding_ws_handler/part_11.rs");`。

- [ ] **Step 2: 运行新 WebSocket 测试确认 RED**

Run:

```bash
cargo test --locked --test it_web scoped_coding_ws_returns_session_state_for_exact_issue
cargo test --locked --test it_web legacy_coding_ws_reports_ambiguous_instead_of_not_found
```

Expected: FAIL；作用域 WS 路由不存在，Session State 不含 Project/Issue，旧错误仍被包装成 not_found。

- [ ] **Step 3: 增加作用域 WebSocket 入口并统一解析**

在 `socket.rs` 增加：

```rust
pub async fn scoped_coding_ws(
    ws: WebSocketUpgrade,
    AxumPath((project_id, issue_id, attempt_id)): AxumPath<(String, String, String)>,
    State(state): State<WebAppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| {
        handle_coding_socket(socket, Some((project_id, issue_id)), attempt_id, state)
    })
    .into_response()
}
```

旧 `coding_ws` 调用：

```rust
handle_coding_socket(socket, None, attempt_id, state)
```

统一 handler 签名和读取：

```rust
async fn handle_coding_socket(
    socket: WebSocket,
    scope: Option<(String, String)>,
    attempt_id: String,
    state: WebAppState,
) {
    let (mut socket_tx, mut socket_rx) = socket.split();
    let app_paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let coding_store = CodingAttemptStore::new(app_paths);
    let attempt_result = match scope.as_ref() {
        Some((project_id, issue_id)) => {
            coding_store.get_attempt(project_id, issue_id, &attempt_id)
        }
        None => coding_store.get_attempt_by_id(&attempt_id),
    };
    let attempt = match attempt_result {
        Ok(attempt) => attempt,
        Err(error) => {
            let (code, message) = coding_attempt_lookup_protocol_error(&error);
            let _ = send_coding_json(
                &mut socket_tx,
                &CodingWsOutMessage::CodingProtocolError { code, message },
            )
            .await;
            return;
        }
    };
```

错误辅助函数：

```rust
fn coding_attempt_lookup_protocol_error(error: &ProductStoreError) -> (String, String) {
    match error {
        ProductStoreError::NotFound {
            kind: "coding_attempt",
            ..
        } => (
            "coding_attempt_not_found".to_string(),
            "coding attempt not found".to_string(),
        ),
        ProductStoreError::Ambiguous {
            kind: "coding_attempt",
            ..
        } => (
            "coding_attempt_ambiguous".to_string(),
            "coding attempt matches multiple issues; reopen it from Workbench".to_string(),
        ),
        ProductStoreError::IdentityMismatch {
            kind: "coding_attempt",
            ..
        } => (
            "coding_attempt_scope_mismatch".to_string(),
            "coding attempt does not belong to the requested project and issue".to_string(),
        ),
        other => (
            "product_store_error".to_string(),
            format!("coding attempt lookup failed: {other}"),
        ),
    }
}
```

- [ ] **Step 4: Session State 携带 Project 与 Issue**

在 `CodingWsOutMessage::CodingSessionState` 增加：

```rust
project_id: String,
issue_id: String,
```

在 `build_coding_session_state` 填充：

```rust
project_id: attempt.project_id.clone(),
issue_id: attempt.issue_id.clone(),
```

- [ ] **Step 5: 注册作用域 WebSocket 路由**

在 `src/web/app.rs` 增加：

```rust
.route(
    "/ws/projects/{project_id}/issues/{issue_id}/coding-attempts/{attempt_id}",
    get(coding_ws_handler::scoped_coding_ws),
)
```

保留旧 `/ws/coding-attempts/{attempt_id}`。

- [ ] **Step 6: 运行 WebSocket 定向回归确认 GREEN**

Run:

```bash
cargo test --locked --test it_web web_coding_ws_handler
```

Expected: PASS；旧唯一 ID 测试继续通过，新作用域与歧义测试通过。

- [ ] **Step 7: 提交 Task 3**

```bash
git add src/web/coding_ws_handler/socket.rs src/web/coding_ws_handler/protocol.rs src/web/coding_ws_handler/state.rs src/web/app.rs tests/it_web/web_coding_ws_handler.rs tests/it_web/web_coding_ws_handler/part_11.rs
git commit -m "feat: scope coding workspace websocket"
```

---

### Task 4: 前端完整 CodingAttemptAddress 贯通

**Files:**
- Modify: `web/src/api/types/coding.ts`
- Modify: `web/src/api/client.ts`
- Modify: `web/src/state/coding-workspace-store.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.ts`
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Modify: `web/src/pages/CodingWorkspaceArtifacts.tsx`
- Modify: `web/src/pages/CodingWorkspaceReports.tsx`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx`
- Modify: `web/src/app-shell.tsx`
- Modify: `web/src/router.tsx`
- Modify: `web/src/api/coding-attempts.test.ts`
- Modify: `web/src/api/client.test.ts`
- Modify: `web/src/api/types.test.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.test-utils.tsx`
- Modify: `web/src/hooks/useCodingWorkspaceWs.test.tsx`
- Modify: `web/src/hooks/useCodingWorkspaceWs.actions.test.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.test-utils.ts`
- Modify: `web/src/pages/CodingWorkspacePage*.test.tsx`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test-data.ts`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.generation.test.tsx`
- Modify: `web/src/router.test.tsx`

**Interfaces:**
- Consumes: Task 2 的作用域 REST 路由和 Task 3 的作用域 WS/Session State。
- Produces: `CodingAttemptAddress`、只接受完整地址的规范 API、Hook、Workspace Page 和 Workbench 跳转。

- [ ] **Step 1: 在 API、Hook、Page、Lifecycle 与 Router 测试中写完整地址失败断言**

统一测试常量：

```typescript
export const CODING_ATTEMPT_ADDRESS = {
  projectId: "project_0001",
  issueId: "issue_0001",
  attemptId: "coding_attempt_0001",
} as const;
```

关键失败断言包括：

```typescript
expect(harness.ws.url).toBe(
  "ws://localhost:3000/ws/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001",
);
```

```typescript
expect(onOpenCodingWorkspace).toHaveBeenCalledWith({
  projectId: "project_0001",
  issueId: "issue_0001",
  attemptId: "coding_attempt_0001",
});
```

```typescript
expect(router.routesByPath[
  "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId"
]).toBeDefined();
```

同时保留以下 Story/Design 共用 Workspace Session 路由断言，证明本次没有改变它们的身份链路：

```typescript
expect(router.routesByPath["/workbench/workspace/$sessionId"]).toBeDefined();
```

API Client 断言所有 Snapshot、Diff、Abort、Delete、Artifact、Confirm、Change Request 使用 `/api/projects/.../issues/.../coding-attempts/...`。

- [ ] **Step 2: 运行前端定向测试确认 RED**

Run:

```bash
cd web && pnpm test -- src/api/coding-attempts.test.ts src/hooks/useCodingWorkspaceWs.test.tsx src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx src/router.test.tsx
```

Expected: FAIL；当前 API、WS、Callback 与 Router 都只传 Attempt ID。

- [ ] **Step 3: 定义 CodingAttemptAddress 与归属字段**

在 `web/src/api/types/coding.ts` 增加：

```typescript
export type CodingAttemptAddress = {
  projectId: string;
  issueId: string;
  attemptId: string;
};
```

在 `CodingAttempt` 增加：

```typescript
project_id: string;
issue_id: string;
```

在 `coding_session_state` 增加：

```typescript
project_id: string;
issue_id: string;
```

更新 `codingAttemptRecord`、`codingGroupAttemptRecord`、API response fixtures 和 `types.test.ts` 中的所有 `CodingAttempt` 对象，统一填入 `project_0001`、`issue_0001`。

- [ ] **Step 4: API Client 使用规范作用域地址**

在 `client.ts` 增加：

```typescript
function codingAttemptApiPath(address: CodingAttemptAddress): string {
  return `/api/projects/${encodeURIComponent(address.projectId)}/issues/${encodeURIComponent(address.issueId)}/coding-attempts/${encodeURIComponent(address.attemptId)}`;
}
```

将规范函数签名改为：

```typescript
getCodingAttemptSnapshot(address: CodingAttemptAddress)
getCodingAttemptDiff(address: CodingAttemptAddress)
deleteCodingAttempt(address: CodingAttemptAddress)
abortCodingAttempt(address: CodingAttemptAddress)
getCodingAttemptArtifact(address: CodingAttemptAddress, artifactId: string)
confirmWorkItemExecutionPlan(address: CodingAttemptAddress)
requestWorkItemExecutionPlanChange(address: CodingAttemptAddress, payload: { note: string })
```

另保留唯一的旧式函数：

```typescript
export function getLegacyCodingAttemptSnapshot(
  attemptId: string,
): Promise<CodingAttemptSnapshotResponse> {
  return requestJson<CodingAttemptSnapshotResponse>(
    `/api/coding-attempts/${encodeURIComponent(attemptId)}`,
  );
}
```

- [ ] **Step 5: WebSocket Store 与 Hook 使用完整地址**

`useCodingWorkspaceWs` 签名改为：

```typescript
export function useCodingWorkspaceWs(address: CodingAttemptAddress | null)
```

Hello 使用 `address.attemptId`，连接地址使用：

```typescript
const ws = new WebSocket(
  `${protocol}//${window.location.host}/ws/projects/${encodeURIComponent(address.projectId)}/issues/${encodeURIComponent(address.issueId)}/coding-attempts/${encodeURIComponent(address.attemptId)}`,
);
```

Session State reducer 增加：

```typescript
projectId: snapshot.project_id,
issueId: snapshot.issue_id,
```

测试工具 `renderCodingHook` 默认接收 `CODING_ATTEMPT_ADDRESS`，Session State fixture 增加 `project_id`、`issue_id`。

- [ ] **Step 6: Coding Workspace 页面与子组件传递完整地址**

页面签名改为：

```typescript
export function CodingWorkspacePage({
  address,
  onBack,
}: {
  address: CodingAttemptAddress;
  onBack: () => void;
})
```

调用：

```typescript
const api = useCodingWorkspaceWs(address);
await deleteCodingAttempt({
  ...address,
  attemptId: store.attemptId ?? address.attemptId,
});
```

`CodingArtifactTabs` 接收 `address` 并调用 `getCodingAttemptDiff(address)`；`PrepareExecutionPlanPanel` 接收 `address` 并调用 Confirm/Change Request。所有 Page 测试使用 `address={CODING_ATTEMPT_ADDRESS}`，删除与执行计划断言改为完整对象。

- [ ] **Step 7: Workbench 与 Router 使用规范地址**

`IssueLifecycleWorkbench` 与 `AppShell` callback 类型改为：

```typescript
onOpenCodingWorkspace?: (address: CodingAttemptAddress) => void;
```

四个创建/复用出口分别调用：

```typescript
// 已有单 Work Item Attempt
onOpenCodingWorkspace({
  projectId: selectedProjectId,
  issueId: card.issueId,
  attemptId: card.raw.latest_attempt.attempt_id,
});

// 新建单 Work Item Attempt
onOpenCodingWorkspace({
  projectId: selectedProjectId,
  issueId: card.issueId,
  attemptId: attempt.attempt_id,
});

// 已有 Work Item Group Attempt
onOpenCodingWorkspace({
  projectId: selectedProjectId,
  issueId: card.issueId,
  attemptId: latestGroupAttempt.attempt_id,
});

// 新建 Work Item Group Attempt
onOpenCodingWorkspace({
  projectId: selectedProjectId,
  issueId: card.issueId,
  attemptId: attempt.attempt_id,
});
```

`defaultOpenCodingWorkspace` 改为：

```typescript
export function defaultOpenCodingWorkspace({
  projectId,
  issueId,
  attemptId,
}: CodingAttemptAddress) {
  window.location.assign(
    `/workbench/projects/${encodeURIComponent(projectId)}/issues/${encodeURIComponent(issueId)}/coding/${encodeURIComponent(attemptId)}`,
  );
}
```

新 Router 页面：

```typescript
function CodingWorkspaceRouteComponent() {
  const { projectId, issueId, attemptId } = useParams({
    from: "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId",
  });
  const navigate = useNavigate();
  return (
    <CodingWorkspacePage
      address={{ projectId, issueId, attemptId }}
      onBack={() => void navigate({ to: "/workbench" })}
    />
  );
}
```

Workbench navigate 使用同一路由和三个参数。Task 4 暂时移除旧 `/workbench/coding/$attemptId` 页面注册，Task 5 以兼容组件恢复。

- [ ] **Step 8: 运行前端定向测试和类型检查确认 GREEN**

Run:

```bash
cd web && pnpm test -- src/api/coding-attempts.test.ts src/api/client.test.ts src/api/types.test.ts src/hooks/useCodingWorkspaceWs.test.tsx src/hooks/useCodingWorkspaceWs.actions.test.tsx src/pages/CodingWorkspacePage.test.tsx src/pages/CodingWorkspacePage.reports.test.tsx src/pages/CodingWorkspacePage.execution-plan.test.tsx src/pages/CodingWorkspacePage.gates.test.tsx src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx src/components/lifecycle/IssueLifecycleWorkbench.generation.test.tsx src/router.test.tsx
cd web && pnpm tsc -b
```

Expected: PASS。

- [ ] **Step 9: 提交 Task 4**

```bash
git add web/src/api/types/coding.ts web/src/api/client.ts web/src/state/coding-workspace-store.ts web/src/hooks/useCodingWorkspaceWs.ts web/src/pages/CodingWorkspacePage.tsx web/src/pages/CodingWorkspaceArtifacts.tsx web/src/pages/CodingWorkspaceReports.tsx web/src/components/lifecycle/IssueLifecycleWorkbench.tsx web/src/components/lifecycle/IssueLifecycleWorkbenchParts.tsx web/src/app-shell.tsx web/src/router.tsx web/src/api/coding-attempts.test.ts web/src/api/client.test.ts web/src/api/types.test.ts web/src/hooks/useCodingWorkspaceWs.test-utils.tsx web/src/hooks/useCodingWorkspaceWs.test.tsx web/src/hooks/useCodingWorkspaceWs.actions.test.tsx web/src/pages/CodingWorkspacePage.test-utils.ts web/src/pages/CodingWorkspacePage.test.tsx web/src/pages/CodingWorkspacePage.reports.test.tsx web/src/pages/CodingWorkspacePage.execution-plan.test.tsx web/src/pages/CodingWorkspacePage.gates.test.tsx web/src/components/lifecycle/IssueLifecycleWorkbench.test-data.ts web/src/components/lifecycle/IssueLifecycleWorkbench.drawer.test.tsx web/src/components/lifecycle/IssueLifecycleWorkbench.generation.test.tsx web/src/router.test.tsx
git commit -m "feat: scope coding workspace frontend identity"
```

---

### Task 5: 旧 Coding Workspace 页面地址兼容

**Files:**
- Create: `web/src/pages/LegacyCodingWorkspaceRedirect.tsx`
- Create: `web/src/pages/LegacyCodingWorkspaceRedirect.test.tsx`
- Modify: `web/src/router.tsx`
- Modify: `web/src/router.test.tsx`

**Interfaces:**
- Consumes: Task 4 的 `getLegacyCodingAttemptSnapshot` 与 `CodingAttemptAddress`。
- Produces: 唯一匹配跳转、歧义/未找到提示、返回 Workbench 操作。

- [ ] **Step 1: 写旧地址成功与冲突失败测试**

创建 `LegacyCodingWorkspaceRedirect.test.tsx`，先定义组件实际会读取的最小 Snapshot 夹具：

```typescript
function snapshotResponse(): CodingAttemptSnapshotResponse {
  return {
    attempt: {
      project_id: "project_0001",
      issue_id: "issue_0001",
      attempt_id: "coding_attempt_0001",
    },
  } as CodingAttemptSnapshotResponse;
}
```

然后增加：

```typescript
it("resolves a unique legacy attempt to the scoped address", async () => {
  vi.mocked(getLegacyCodingAttemptSnapshot).mockResolvedValue(
    snapshotResponse(),
  );
  const onResolved = vi.fn();

  render(
    <LegacyCodingWorkspaceRedirect
      attemptId="coding_attempt_0001"
      onResolved={onResolved}
      onBack={vi.fn()}
    />,
  );

  await waitFor(() =>
    expect(onResolved).toHaveBeenCalledWith({
      projectId: "project_0001",
      issueId: "issue_0001",
      attemptId: "coding_attempt_0001",
    }),
  );
});

it("shows an actionable message for an ambiguous legacy attempt", async () => {
  vi.mocked(getLegacyCodingAttemptSnapshot).mockRejectedValue(
    new ApiRequestError({
      code: "coding_attempt_ambiguous",
      message: "coding attempt matches multiple issues",
      details: { attempt_id: "coding_attempt_0001" },
    }),
  );
  const onBack = vi.fn();

  render(
    <LegacyCodingWorkspaceRedirect
      attemptId="coding_attempt_0001"
      onResolved={vi.fn()}
      onBack={onBack}
    />,
  );

  expect(await screen.findByRole("alert")).toHaveTextContent(
    "该历史 Coding Attempt ID 对应多个 Issue",
  );
  await userEvent.click(screen.getByRole("button", { name: "返回 Workbench" }));
  expect(onBack).toHaveBeenCalledTimes(1);
});
```

再增加 not_found 测试，提示“Coding Attempt 不存在或已删除”。

- [ ] **Step 2: 运行兼容页面测试确认 RED**

Run:

```bash
cd web && pnpm test -- src/pages/LegacyCodingWorkspaceRedirect.test.tsx
```

Expected: FAIL；组件不存在。

- [ ] **Step 3: 实现兼容组件**

创建 `LegacyCodingWorkspaceRedirect.tsx`：

```typescript
import { useEffect, useState } from "react";
import { getLegacyCodingAttemptSnapshot } from "../api/client";
import type { CodingAttemptAddress } from "../api/types";

export function LegacyCodingWorkspaceRedirect({
  attemptId,
  onResolved,
  onBack,
}: {
  attemptId: string;
  onResolved: (address: CodingAttemptAddress) => void;
  onBack: () => void;
}) {
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getLegacyCodingAttemptSnapshot(attemptId)
      .then((snapshot) => {
        if (cancelled) return;
        onResolved({
          projectId: snapshot.attempt.project_id,
          issueId: snapshot.attempt.issue_id,
          attemptId: snapshot.attempt.attempt_id,
        });
      })
      .catch((reason: { code?: string }) => {
        if (cancelled) return;
        setError(
          reason.code === "coding_attempt_ambiguous"
            ? "该历史 Coding Attempt ID 对应多个 Issue，请从目标 Issue 的 Workbench 重新进入。"
            : "Coding Attempt 不存在或已删除。",
        );
      });
    return () => {
      cancelled = true;
    };
  }, [attemptId, onResolved]);

  if (!error) {
    return <div role="status">正在定位 Coding Attempt…</div>;
  }
  return (
    <div role="alert">
      <p>{error}</p>
      <button type="button" onClick={onBack}>返回 Workbench</button>
    </div>
  );
}
```

- [ ] **Step 4: 在 Router 恢复旧地址并跳转到规范地址**

旧 Route Component：

```typescript
function LegacyCodingWorkspaceRouteComponent() {
  const { attemptId } = useParams({ from: "/workbench/coding/$attemptId" });
  const navigate = useNavigate();
  return (
    <LegacyCodingWorkspaceRedirect
      attemptId={attemptId}
      onResolved={({ projectId, issueId, attemptId: resolvedAttemptId }) =>
        void navigate({
          to: "/workbench/projects/$projectId/issues/$issueId/coding/$attemptId",
          params: {
            projectId,
            issueId,
            attemptId: resolvedAttemptId,
          },
          replace: true,
        })
      }
      onBack={() => void navigate({ to: "/workbench" })}
    />
  );
}
```

重新注册 `/workbench/coding/$attemptId`，并在 `routeTree` 同时包含新旧 Coding Workspace route。

- [ ] **Step 5: 运行兼容页面、Router 和类型检查确认 GREEN**

Run:

```bash
cd web && pnpm test -- src/pages/LegacyCodingWorkspaceRedirect.test.tsx src/router.test.tsx
cd web && pnpm tsc -b
```

Expected: PASS。

- [ ] **Step 6: 提交 Task 5**

```bash
git add web/src/pages/LegacyCodingWorkspaceRedirect.tsx web/src/pages/LegacyCodingWorkspaceRedirect.test.tsx web/src/router.tsx web/src/router.test.tsx
git commit -m "fix: preserve legacy coding workspace links"
```

---

### Task 6: 全量回归、历史数据边界与人工观察准备

**Files:**
- Verify only: Tasks 1-5 modified files
- Verify only: `.aria/`

**Interfaces:**
- Consumes: 完整实现。
- Produces: 标准门禁结果、历史数据未修改证据、可供用户手动验收的开发服务。

- [ ] **Step 1: 检查旧单 ID 业务调用是否只剩兼容入口**

Run:

```bash
rg -n '/api/coding-attempts|/ws/coding-attempts|/workbench/coding/\$attemptId' src web/src
```

Expected: 只剩后端旧路由注册、`getLegacyCodingAttemptSnapshot`、旧 WebSocket 兼容入口、旧页面 Redirect 和对应测试；规范页面与业务操作不使用旧地址。

- [ ] **Step 2: 检查历史 `.aria` 未进入 Git 差异**

Run:

```bash
git status --short --branch
git diff --name-status origin/feat-b-0715...HEAD
git diff -- .aria
```

Expected: `.aria` 无输出、无 staged/untracked 历史数据文件；差异仅包含设计、计划、业务实现和测试。

- [ ] **Step 3: 运行 Rust 标准门禁**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

Expected: 全部 exit 0；不得使用 `-j 1`。

- [ ] **Step 4: 运行前端标准门禁**

Run:

```bash
cd web && pnpm tsc -b
cd web && pnpm test
cd web && pnpm build
```

Expected: 全部 exit 0。

- [ ] **Step 5: 检查最终差异与提交边界**

Run:

```bash
git diff --check
git status --short --branch
git log --oneline -8 --decorate
```

Expected: worktree 干净；Tasks 1-5 每个任务一个原子提交，当前分支仍只领先远端、不自动 push。

- [ ] **Step 6: 按开发服务规范启动并仅检查就绪状态**

先确认工具和依赖：

```bash
cargo watch --version
pnpm --version
test -d web/node_modules
```

若 `web/node_modules` 不存在，停止并请求用户确认后再执行 `pnpm install`。存在时：

```bash
cd web && pnpm build
cargo watch -w src -w Cargo.toml -w Cargo.lock -x "run --locked -- web --workspace . --host 127.0.0.1 --port 4317"
cd web && pnpm dev --port 5173
```

两个开发服务都必须作为无超时后台任务启动；先等待后端 `/api/health` 就绪，再启动前端，避免 Vite 代理持续报 `ECONNREFUSED`。

只执行以下就绪检查：

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS -I http://127.0.0.1:5173/
curl --noproxy '*' -sS http://127.0.0.1:5173/api/health
```

Expected: 后端返回 `{"status":"ok"}`，前端 `/` 可访问，前端代理健康检查成功。随后等待用户从 Workbench 分别打开 `issue_0001` 与 `issue_0002` 的 Coding Workspace，不主动调用业务 API 或执行浏览器自动化。

---

## Completion Criteria

- Task 1-5 的 RED/GREEN 证据均已记录。
- 当前两个历史 `coding_attempt_0001` 能通过不同 Project/Issue 作用域地址精确访问。
- 新建 Attempt ID 使用 UUID，单 Work Item 与 Work Item Group 一致。
- REST、WebSocket、Frontend 全部使用同一 `CodingAttemptAddress` 语义。
- 旧唯一 ID 可以兼容，旧歧义 ID 明确返回冲突。
- Story Spec、Design Spec Workspace Session 路由回归通过。
- `.aria` 未被迁移或修改。
- Rust 与前端标准门禁全部通过。
