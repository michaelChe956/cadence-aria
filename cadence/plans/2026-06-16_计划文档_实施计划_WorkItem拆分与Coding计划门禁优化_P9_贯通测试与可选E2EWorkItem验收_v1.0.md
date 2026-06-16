# WorkItem 拆分 P9 贯通测试与可选 E2E WorkItem 验收 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 验证后端 Work Item、前端 Work Item、可选 Integration/E2E Work Item 的端到端关系：后端 handoff 被前端消费，Integration/E2E 等待前后端完成，用户跳过时记录风险但不阻塞。

**Architecture:** 本计划以测试为主，只在测试暴露真实缺陷时做最小生产修复。后端使用 `it_web` 贯通测试覆盖 API/状态机；前端使用 Vitest 覆盖 lifecycle 和 Coding Prepare；浏览器 E2E 放在现有 `web/e2e` 目录，复用仓库 Playwright 配置。

**Tech Stack:** Rust 1.95.0、Axum integration tests、Vitest、Playwright、pnpm、Cargo。

---

## 前置交付摘要

执行本计划前确认：

- P3 已能生成 Backend/Frontend/Integration/E2E Work Items，并保证每项有 session/artifact 关联。
- P5 已让 Coding attempt 受依赖和 handoff 门禁约束，并复用 Issue shared worktree。
- P6 已生成 execution plan 和 handoff，并在 completion 前强制 handoff 存在。
- P7/P8 已在前端展示 DAG、handoff 和 execution plan。

## 计划大小边界

本计划默认只写测试：

- 不主动改生产后端代码。
- 不主动改生产前端代码。
- 如果测试暴露生产缺陷，先把失败、根因、建议修复范围写入当前执行汇报；若修复超过 1-2 个文件，新增修复计划，不把 P9 扩成开发计划。
- 不创建真实远端 PR 或外部仓库数据。

## 文件结构

- Create: `tests/it_web/web_work_item_split_flow.rs`
  - 后端贯通测试。
- Modify: `tests/it_web.rs`
  - 引入新测试模块。
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`
  - 前端 lifecycle 贯通状态测试。
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`
  - execution plan/handoff 展示联动测试。
- Create: `web/e2e/work-item-split-flow.spec.ts`
  - 浏览器 E2E。仓库现有 Playwright 测试目录是 `web/e2e`。
- Modify: `web/e2e/helpers/coding.ts` or `web/e2e/helpers/workspace.ts`
  - 仅在需要复用 setup helper 时修改。

## 任务 1：Backend Flow Test For Split Generation And Dependency Gates

**文件：**

- Create: `tests/it_web/web_work_item_split_flow.rs`
- Modify: `tests/it_web.rs`

- [ ] **步骤 1：编写失败态 backend flow test**

创建 `tests/it_web/web_work_item_split_flow.rs`:

```rust
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::{Value, json};
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn work_item_split_flow_blocks_frontend_until_backend_handoff_exists() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_story_and_design(app.clone(), repo.path()).await;

    let (status, generated) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(generated["work_items"].as_array().unwrap().len(), 3);

    let (status, blocked) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(blocked["code"], "work_item_dependency_not_completed");

    mark_work_item_completed_with_handoff(
        root.path(),
        "project_0001",
        "issue_0001",
        "work_item_0001",
        "handoffs/work_item_0001.json",
    );

    let (status, attempt) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0002/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(attempt["branch_name"], "aria/issues/issue_0001");
}
```

在 `tests/it_web.rs`, add:

```rust
#[path = "it_web/web_work_item_split_flow.rs"]
mod web_work_item_split_flow;
```

- [ ] **步骤 2：运行 backend flow test 并确认失败 or pass**

运行:

```bash
cargo test --locked --test it_web work_item_split_flow_blocks_frontend_until_backend_handoff_exists
```

预期：

- P3/P5/P6 正确时，补齐测试 helper 后该测试通过。
- 如果失败，先确认失败是否来自 P3/P5/P6 的真实缺陷；只允许在已声明写入范围内做 1-2 个文件的最小修复，否则停止并新增修复计划。

- [ ] **步骤 3：添加 helper functions**

Implement test-only helpers in the test file:

- `git_repo()`
- `request_json()`
- `bootstrap_confirmed_story_and_design()`
- `mark_work_item_completed_with_handoff()`

使用 existing helper patterns from `web_lifecycle_api.rs` and `web_coding_attempt_api.rs` instead of inventing a new HTTP harness.

- [ ] **步骤 4：运行 backend flow test 并确认通过**

Run command from Step 2.

预期：通过。

## 任务 2：Backend Flow Test For Optional Integration/E2E Choices

**文件：**

- Modify: `tests/it_web/web_work_item_split_flow.rs`

- [ ] **步骤 1：Write optional test choices tests**

追加:

```rust
#[tokio::test]
async fn work_item_split_records_risk_when_integration_and_e2e_are_skipped() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_story_and_design(app.clone(), repo.path()).await;

    let (status, generated) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": false,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(generated["work_items"].as_array().unwrap().len(), 2);
    assert!(
        generated["validator_findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["code"] == "integration_or_e2e_skipped_risk")
    );
}

#[tokio::test]
async fn work_item_split_e2e_item_waits_for_backend_and_frontend() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_story_and_design(app.clone(), repo.path()).await;

    let (status, generated) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items:generate",
        json!({
            "title": "登录会话拆分实现",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": false,
            "include_e2e_tests": true,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let e2e = generated["work_items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "e2e")
        .expect("e2e item");
    assert_eq!(e2e["depends_on"], json!(["work_item_0001", "work_item_0002"]));
}
```

- [ ] **步骤 2：Run optional choice tests**

运行:

```bash
cargo test --locked --test it_web work_item_split_records_risk_when_integration_and_e2e_are_skipped
cargo test --locked --test it_web work_item_split_e2e_item_waits_for_backend_and_frontend
```

预期: pass after any minimal fix to warning findings.

## 任务 3：Frontend Integration Tests For Lifecycle And Coding Prepare

**文件：**

- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.test.tsx`

- [ ] **步骤 1：添加 lifecycle test for DAG display and skipped risk**

追加:

```tsx
it("shows generated split work items and skipped integration risk", async () => {
  vi.stubGlobal("fetch", lifecycleFetch({ splitWorkItems: true, skippedIntegrationRisk: true }));

  render(<IssueLifecycleWorkbench />);

  expect(await screen.findByText("后端 API")).toBeInTheDocument();
  expect(screen.getByText("前端 UI")).toBeInTheDocument();
  expect(screen.getByText(/等待依赖完成/)).toBeInTheDocument();
  expect(screen.getByText(/跳过贯通测试/)).toBeInTheDocument();
});
```

- [ ] **步骤 2：添加 Coding Prepare test for dependency handoff display**

追加:

```tsx
it("shows dependency handoff summary in execution plan", () => {
  useCodingWorkspaceStore.setState({
    ...readyCodingState(),
    stage: "prepare_context",
    workItemExecutionPlan: executionPlan({
      dependency_handoffs: [
        {
          work_item_id: "work_item_0001",
          summary_ref: "handoffs/work_item_0001.json",
          summary: "后端 API 已完成",
          commit_sha: "abc123",
        },
      ],
    }),
  });

  render(<CodingWorkspacePage attemptId="coding_attempt_0002" onBack={vi.fn()} />);

  expect(screen.getByText("后端 API 已完成")).toBeInTheDocument();
  expect(screen.getByText("abc123")).toBeInTheDocument();
});
```

- [ ] **步骤 3：Run frontend integration tests**

运行:

```bash
pnpm -C web test -- --run IssueLifecycleWorkbench
pnpm -C web test -- --run CodingWorkspacePage
```

预期：通过。

## 任务 4：Browser E2E Smoke Test

**文件：**

- Create: `web/e2e/work-item-split-flow.spec.ts`
- Modify: `web/e2e/helpers/workspace.ts` if needed

- [ ] **步骤 1：Write E2E smoke test**

创建 `web/e2e/work-item-split-flow.spec.ts`:

```ts
import { expect, test } from "@playwright/test";

test("work item split flow shows DAG and coding execution plan", async ({ page }) => {
  await page.goto("/workbench");

  await expect(page.getByText("Work Item")).toBeVisible();
  await expect(page.getByText(/后端|Backend|前端|Frontend/)).toBeVisible();

  const splitCard = page.getByText(/后端 API|Backend API/).first();
  if (await splitCard.isVisible()) {
    await splitCard.click();
    await expect(page.getByText(/写入范围|Allowed scope/)).toBeVisible();
  }
});
```

使用 existing fake/test-control setup patterns if the workbench requires seeded data; do not hit external services.

- [ ] **步骤 2：Run E2E test**

运行:

```bash
pnpm -C web test:e2e -- work-item-split-flow.spec.ts
```

预期：在现有 Playwright 配置下通过。若因缺少 fake seed 数据失败，通过现有 `web/e2e/helpers/*` 或测试控制 API 增加确定性 setup；不得依赖人工预置数据。

## 最终验证

运行:

```bash
cargo test --locked --test it_web work_item_split_flow
pnpm -C web test -- --run IssueLifecycleWorkbench
pnpm -C web test -- --run CodingWorkspacePage
pnpm -C web test:e2e -- work-item-split-flow.spec.ts
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

预期:

- Backend flow tests pass.
- Frontend Vitest tests pass.
- Playwright smoke test passes.
- Rust formatting, clippy and check pass.

## 提交

```bash
git add tests/it_web.rs tests/it_web/web_work_item_split_flow.rs web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx web/src/pages/CodingWorkspacePage.test.tsx web/e2e/work-item-split-flow.spec.ts web/e2e/helpers/coding.ts web/e2e/helpers/workspace.ts
git commit -m "test: verify split work item flow"
```
