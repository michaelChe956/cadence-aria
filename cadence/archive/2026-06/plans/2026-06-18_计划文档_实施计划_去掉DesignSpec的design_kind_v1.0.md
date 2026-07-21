# 去掉 Design Spec 的 design_kind Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking in detailed plans.

**Goal:** 彻底删除 `DesignKind` 枚举与所有 `design_kind` 字段(后端 struct / API DTO / 前端类型 / 持久化记录),Design Spec 的 Workspace 标题恢复为 `{title}`,WorkItem Splitter prompt 去掉 `kind=` 标注,不影响 WorkItem 的前后端区分能力。

**Architecture:** `DesignKind` 是只挂在 `DesignSpecRecord` 上的死字段(前端始终写死 frontend)。它与 `WorkItemKind`(独立枚举,WorkItem 自己的前后端区分)互不依赖。删除路径:后端先删枚举与字段 → 同步删所有构造点(全为测试夹具)与 helper → 前端删类型与传值 → 测试夹具清理 → 全量验证。`force_frontend_backend_split` 选项与 `work_item_split_validator` 校验完全不动。

**Tech Stack:** Rust 1.95.0、Cargo、Axum、tokio(后端);React、TypeScript、Vitest、pnpm(前端)。本计划不做 Playwright 浏览器 E2E。

**版本:** v1.0
**创建日期:** 2026-06-18
**目标分支:** `feat-b-0616`
**工作区:** `.worktrees/feat-b-0616`
**设计文档:** `cadence/designs/2026-06-18_技术方案_去掉DesignSpec的design_kind_v1.0.md`

---

## 全局约束(Global Constraints)

- **运行命令固定**:Rust 1.95.0;cargo 命令带 `--locked`;🔴 **禁止 `-j 1`**(并行度由 `.cargo/config.toml` 的 `jobs = 8` 托管);前端用 `pnpm`。
- **强制检查链**:`cargo fmt --check` + `cargo clippy --all-targets --all-features --locked -- -D warnings` + `cargo check --locked` + 定向测试 + `pnpm -C web test` + `pnpm -C web build`,全绿。
- **TDD**:本计划是删除重构,先改测试夹具(使其因字段缺失而编译失败)→ 再改生产代码使编译通过 → 跑测试。每个 Task 结尾提交。
- **写入范围严格**:只改本计划声明的文件。不动 `WorkItemKind`、`force_frontend_backend_split`、`work_item_split_validator.rs`、`WORK_ITEM_SPLIT_OUTPUT_SCHEMA`。
- **行号是参考**:基于 `feat-b-0616` HEAD(含设计文档提交 `11c1324`);实现时以 `grep -n` 实际为准。
- **存量数据**:不写迁移。删除字段后,本地旧 stores 的 JSON 反序列化会失败,需清空本地 stores(见 Task 8)。

---

## File Structure

| 文件 | 操作 | 职责 / 本计划改动 |
|---|---|---|
| `src/product/models.rs` | M | 删除 `DesignKind` 枚举(:202-205);删除 `DesignSpecRecord.design_kind` 字段(:362) |
| `src/product/lifecycle_store.rs` | M | 删除 `CreateDesignSpecInput.design_kind` 字段(:42);`create_design_spec` 不再写入(:320);删 import `DesignKind`(:12) |
| `src/web/handlers.rs` | M | 删 `parse_design_kind`(:2864)、`design_kind_text`(:2697);handler 不再解析/传(:477-484);DTO 不再序列化(:2214);删 import `DesignKind`(:38) |
| `src/web/types.rs` | M | 删 `DesignSpecDto.design_kind`(:417)、`GenerateDesignSpecsRequest.design_kind`(:549) |
| `src/web/workspace_context.rs` | M | Design 标题拼接改回 `design.title.clone()`(:237-241);删 `design_kind_label`(:574-578);删 4 处夹具 `design_kind` 赋值(:796/863/980/1087);删 import `DesignKind`(:591) |
| `src/product/work_item_split_engine.rs` | M | 删 `design_kind_text`(:406-411);prompt 行去掉 `kind=` 片段(:459-465) |
| `src/product/workspace_engine.rs` | M | 删 3 处夹具 `design_kind` 赋值(:9328/9610/9918) |
| `src/product/coding_evaluation_context.rs` | M | 删夹具 `design_kind` 赋值(:613);删 import `DesignKind`(:560) |
| `src/web/workspace_ws_handler.rs` | M | 删夹具 `design_kind` 赋值(:1442) |
| `tests/it_product/product_lifecycle_store.rs` | M | 删夹具 `design_kind`(:65);删 import `DesignKind`(:12) |
| `tests/it_product/product_work_item_split_engine.rs` | M | 删夹具 `design_kind`(:257);删 import `DesignKind`(:11) |
| `tests/it_web/web_lifecycle_api.rs` | M | 删 4 处请求 body 的 `design_kind`(:301/407/823)+ 1 处响应断言(:319) |
| `tests/it_web/web_coding_attempt_api.rs` | M | 删请求 body 的 `design_kind`(:1120) |
| `tests/it_web/web_work_item_generation.rs` | M | 删请求 body 的 `design_kind`(:334) |
| `tests/it_web/web_workspace_recovery_consistency.rs` | M | 删请求 body 的 `design_kind`(:158) |
| `web/src/api/types.ts` | M | 删 `DesignSpec.design_kind`(:100)、`GenerateDesignSpecsRequest.design_kind`(:828) |
| `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx` | M | 删 2 处 `design_kind: "frontend"` 传值(:283/487) |
| `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx` | M | 删 mock 类型里的 `design_kind`(:1261)、回显 `design_kind`(:1278)、2 处夹具 `design_kind`(:703/1700) |
| `web/src/api/client.test.ts` | M | 删 `generateDesignSpecs` 调用的 `design_kind`(:148) |
| `web/src/state/lifecycle-workbench-store.test.ts` | M | 删 2 处夹具 `design_kind`(:98/146) |
| `web/src/components/lifecycle/LifecycleCard.test.tsx` | M | 删夹具 `design_kind`(:196) |

**不改(明确边界):**
- ❌ `WorkItemKind` 枚举(`models.rs:280`)与 `LifecycleWorkItemRecord.kind`(`models.rs:386`)
- ❌ `force_frontend_backend_split` 选项(`models.rs:455`)与 `work_item_split_validator.rs:380` 校验
- ❌ `WORK_ITEM_SPLIT_OUTPUT_SCHEMA` 的 `work_items[].kind`(`work_item_split_engine.rs:64`)
- ❌ `work_item_kind_text`(`work_item_split_engine.rs:568`)
- ❌ `build_split_prompt` / `build_revision_prompt` 中 `force_frontend_backend_split` 的 prompt 片段

---

## Task 1:后端删除 DesignKind 枚举与 DesignSpecRecord 字段

**目标**:从 `models.rs` 删除 `DesignKind` 枚举与 `DesignSpecRecord.design_kind` 字段。改完后 `cargo check` 会大量报错(所有引用点未清理),这是预期 —— 后续 Task 逐一清理。

**Files:**
- Modify: `src/product/models.rs:202-205`(删枚举)、`:362`(删字段)

- [ ] **Step 1.1:删除 `DesignKind` 枚举定义**

`src/product/models.rs:200-205`,删除整个枚举:

```rust
// 删除以下 6 行(含 derive 与 serde 属性):
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignKind {
    Frontend,
    Backend,
}
```

保留其上方的 `}`(:198,前一个 enum 的结尾)与其下方的 `ProviderName` 枚举(:207 起)。

- [ ] **Step 1.2:删除 `DesignSpecRecord.design_kind` 字段**

`src/product/models.rs:357-368`,`DesignSpecRecord` 结构体内删除一行:

```rust
// 删除这一行:
    pub design_kind: DesignKind,
```

保留 `story_spec_ids`(:361)与 `title`(:363)等其他字段。

- [ ] **Step 1.3:验证编译报错符合预期**

Run: `cargo check --locked 2>&1 | grep -c "DesignKind"`
Expected: 输出 ≥ 10(所有未清理引用点报错)。不提交,继续 Task 2。

---

## Task 2:后端删除 lifecycle_store 的 design_kind

**目标**:`lifecycle_store.rs` 删除 `CreateDesignSpecInput.design_kind` 字段、`create_design_spec` 中的写入、`DesignKind` import。

**Files:**
- Modify: `src/product/lifecycle_store.rs:12`(import)、`:42`(字段)、`:320`(写入)

- [ ] **Step 2.1:删除 import 中的 `DesignKind`**

`src/product/lifecycle_store.rs:11-13`,把:
```rust
use crate::product::models::{
    DesignKind, DesignSpecRecord, IssueSharedWorktree, IssueSharedWorktreeStatus,
```
改为:
```rust
use crate::product::models::{
    DesignSpecRecord, IssueSharedWorktree, IssueSharedWorktreeStatus,
```

- [ ] **Step 2.2:删除 `CreateDesignSpecInput.design_kind` 字段**

`src/product/lifecycle_store.rs:37-44`,删除一行:
```rust
// 删除:
    pub design_kind: DesignKind,
```
保留 `story_spec_ids`(:41)与 `title`(:43)。

- [ ] **Step 2.3:删除 `create_design_spec` 中的字段写入**

`src/product/lifecycle_store.rs:315-322`,把:
```rust
        let design = DesignSpecRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            story_spec_ids: input.story_spec_ids,
            design_kind: input.design_kind,
            title: input.title,
            current_version: None,
```
改为(删 `design_kind` 那一行):
```rust
        let design = DesignSpecRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            story_spec_ids: input.story_spec_ids,
            title: input.title,
            current_version: None,
```

- [ ] **Step 2.4:提交**

```bash
git add src/product/models.rs src/product/lifecycle_store.rs
git commit -m "refactor(design_kind): 删除 DesignKind 枚举与 DesignSpecRecord/CreateDesignSpecInput 字段

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 3:后端删除 handlers.rs 的 design_kind

**目标**:`handlers.rs` 删 `parse_design_kind`、`design_kind_text`、handler 中的解析/传值、DTO 序列化、import。

**Files:**
- Modify: `src/web/handlers.rs:38`(import)、`:477-484`(handler)、`:2214`(DTO)、`:2697-2702`(`design_kind_text`)、`:2864-2872`(`parse_design_kind`)

- [ ] **Step 3.1:删除 import 中的 `DesignKind`**

`src/web/handlers.rs:38-40`,把:
```rust
use crate::product::models::{
    DesignKind, DesignSpecRecord, GateStatus, IssuePhase as ProductIssuePhase,
```
改为:
```rust
use crate::product::models::{
    DesignSpecRecord, GateStatus, IssuePhase as ProductIssuePhase,
```

- [ ] **Step 3.2:删除 handler 中的解析与传值**

`src/web/handlers.rs:475-486`,把:
```rust
    validate_confirmed_story_specs(&lifecycle, &project_id, &issue_id, &request.story_spec_ids)?;
    let design_kind = parse_design_kind(&request.design_kind)?;
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            story_spec_ids: request.story_spec_ids,
            design_kind,
            title: request.title,
        })
        .map_err(product_store_api_error)?;
```
改为(删 `let design_kind = ...` 行与 `design_kind,` 行):
```rust
    validate_confirmed_story_specs(&lifecycle, &project_id, &issue_id, &request.story_spec_ids)?;
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            story_spec_ids: request.story_spec_ids,
            title: request.title,
        })
        .map_err(product_store_api_error)?;
```

- [ ] **Step 3.3:删除 DTO 序列化中的 design_kind**

`src/web/handlers.rs:2210-2219`,把:
```rust
    Ok(DesignSpecDto {
        design_spec_id: record.id.clone(),
        issue_id: record.issue_id.clone(),
        story_spec_ids: record.story_spec_ids.clone(),
        design_kind: design_kind_text(&record.design_kind).to_string(),
        title: record.title.clone(),
        current_version: record.current_version,
```
改为(删 `design_kind:` 那一行):
```rust
    Ok(DesignSpecDto {
        design_spec_id: record.id.clone(),
        issue_id: record.issue_id.clone(),
        story_spec_ids: record.story_spec_ids.clone(),
        title: record.title.clone(),
        current_version: record.current_version,
```

- [ ] **Step 3.4:删除 `design_kind_text` 函数**

`src/web/handlers.rs:2697-2702`,删除整个函数:
```rust
// 删除:
fn design_kind_text(kind: &DesignKind) -> &'static str {
    match kind {
        DesignKind::Frontend => "frontend",
        DesignKind::Backend => "backend",
    }
}
```

- [ ] **Step 3.5:删除 `parse_design_kind` 函数**

`src/web/handlers.rs:2864-2872`,删除整个函数(含其上的空行):
```rust
// 删除:
fn parse_design_kind(value: &str) -> ApiResult<DesignKind> {
    match value {
        "frontend" => Ok(DesignKind::Frontend),
        "backend" => Ok(DesignKind::Backend),
        _ => Err(ApiError::bad_request(
            "invalid_design_kind",
            "design_kind must be frontend or backend",
        )),
    }
}
```
> 注:确切错误构造方式以 `grep -n "fn parse_design_kind" -A 10 src/web/handlers.rs` 实际为准,若 `ApiError::bad_request` 签名不同则照原样删除整个函数体。

- [ ] **Step 3.6:提交**

```bash
git add src/web/handlers.rs
git commit -m "refactor(design_kind): 删除 handlers 的 parse/design_kind_text 与 DTO 字段

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 4:后端删除 types.rs 的 design_kind DTO 字段

**Files:**
- Modify: `src/web/types.rs:417`、`:549`

- [ ] **Step 4.1:删除 `DesignSpecDto.design_kind`**

`src/web/types.rs:413-421`,删除一行:
```rust
// 删除:
    pub design_kind: String,
```
保留 `story_spec_ids`(:416)与 `title`(:418)。

- [ ] **Step 4.2:删除 `GenerateDesignSpecsRequest.design_kind`**

`src/web/types.rs:546-550`,删除一行:
```rust
// 删除:
    pub design_kind: String,
```
保留 `story_spec_ids`(:548)与 `author_provider`(:550)。

- [ ] **Step 4.3:提交**

```bash
git add src/web/types.rs
git commit -m "refactor(design_kind): 删除 DesignSpecDto/GenerateDesignSpecsRequest 的 design_kind 字段

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 5:后端删除 workspace_context.rs 的标题拼接与 helper

**目标**:Design 标题恢复为 `design.title.clone()`;删 `design_kind_label`;删 4 处测试夹具赋值;删 import。

**Files:**
- Modify: `src/web/workspace_context.rs:237-241`(标题)、`:574-578`(helper)、`:796/863/980/1087`(夹具)、`:591`(import)

- [ ] **Step 5.1:修改 Design 标题拼接**

`src/web/workspace_context.rs:233-245`,把:
```rust
        WorkspaceType::Design => {
            let design = find_design_spec(lifecycle, session, &session.entity_id)?;
            let stories = linked_story_context(lifecycle, session, &design.story_spec_ids)?;
            Ok(WorkspaceEntityContext {
                title: format!(
                    "{} ({})",
                    design.title,
                    design_kind_label(&design.design_kind)
                ),
                repository_id: issue_repo_id(issue)?,
                linked_context: stories,
            })
        }
```
改为:
```rust
        WorkspaceType::Design => {
            let design = find_design_spec(lifecycle, session, &session.entity_id)?;
            let stories = linked_story_context(lifecycle, session, &design.story_spec_ids)?;
            Ok(WorkspaceEntityContext {
                title: design.title,
                repository_id: issue_repo_id(issue)?,
                linked_context: stories,
            })
        }
```

- [ ] **Step 5.2:删除 `design_kind_label` 函数**

`src/web/workspace_context.rs:574-578`,删除整个函数:
```rust
// 删除:
fn design_kind_label(kind: &crate::product::models::DesignKind) -> &'static str {
    match kind {
        crate::product::models::DesignKind::Frontend => "frontend",
        crate::product::models::DesignKind::Backend => "backend",
    }
}
```

- [ ] **Step 5.3:删除 import 中的 `DesignKind`**

`src/web/workspace_context.rs:590-592`(test 模块内 import),把:
```rust
    use crate::product::models::{
        DesignKind, IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, LifecycleConfirmationStatus,
        ProviderName, WorkspaceMessageRecord, WorkspaceType,
    };
```
改为:
```rust
    use crate::product::models::{
        IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, LifecycleConfirmationStatus,
        ProviderName, WorkspaceMessageRecord, WorkspaceType,
    };
```

- [ ] **Step 5.4:删除 4 处测试夹具的 design_kind 赋值**

`src/web/workspace_context.rs` 中 `:796`、`:863`、`:980`、`:1087` 四处 `CreateDesignSpecInput` 构造,每处删除一行:
```rust
// 在每处 CreateDesignSpecInput { ... } 内删除:
                design_kind: DesignKind::Backend,
```
> 用 `grep -n "design_kind: DesignKind::Backend" src/web/workspace_context.rs` 定位全部 4 处,逐一删除。

- [ ] **Step 5.5:提交**

```bash
git add src/web/workspace_context.rs
git commit -m "refactor(design_kind): Design 标题恢复为 title,删除 design_kind_label 与夹具赋值

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 6:后端删除 work_item_split_engine 的 prompt 标注

**目标**:prompt 行去掉 `kind=` 片段;删 `design_kind_text`。

**Files:**
- Modify: `src/product/work_item_split_engine.rs:406-411`(helper)、`:459-465`(prompt)

- [ ] **Step 6.1:修改 prompt 拼接**

`src/product/work_item_split_engine.rs:459-465`,把:
```rust
            Ok(format!(
                "Design Spec: {} ({}) kind={}\n{}",
                spec.title,
                spec.id,
                design_kind_text(&spec.design_kind),
                markdown
            ))
```
改为:
```rust
            Ok(format!(
                "Design Spec: {} ({})\n{}",
                spec.title,
                spec.id,
                markdown
            ))
```

- [ ] **Step 6.2:删除 `design_kind_text` 函数**

`src/product/work_item_split_engine.rs:406-411`,删除整个函数:
```rust
// 删除:
fn design_kind_text(kind: &crate::product::models::DesignKind) -> &'static str {
    match kind {
        crate::product::models::DesignKind::Frontend => "frontend",
        crate::product::models::DesignKind::Backend => "backend",
    }
}
```
> 注意:不要误删 `work_item_kind_text`(:568),那个保留。

- [ ] **Step 6.3:提交**

```bash
git add src/product/work_item_split_engine.rs
git commit -m "refactor(design_kind): WorkItem Splitter prompt 去掉 kind= 标注,删除 design_kind_text

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 7:后端清理剩余夹具赋值(workspace_engine / coding_evaluation_context / workspace_ws_handler)

**目标**:清理 3 个文件里 5 处 `CreateDesignSpecInput` 夹具的 `design_kind` 赋值与 1 处 import。

**Files:**
- Modify: `src/product/workspace_engine.rs:9328/9610/9918`
- Modify: `src/product/coding_evaluation_context.rs:560(import)、:613(夹具)`
- Modify: `src/web/workspace_ws_handler.rs:1442`

- [ ] **Step 7.1:清理 workspace_engine.rs 3 处夹具**

Run: `grep -n "design_kind: crate::product::models::DesignKind::Backend" src/product/workspace_engine.rs`
Expected: 输出 3 行(:9328、:9610、:9918)。每处删除该行:
```rust
// 删除:
                design_kind: crate::product::models::DesignKind::Backend,
```

- [ ] **Step 7.2:清理 coding_evaluation_context.rs**

`src/product/coding_evaluation_context.rs:560`,把:
```rust
    use crate::product::models::{DesignKind, ProviderName, WorkspaceType};
```
改为:
```rust
    use crate::product::models::{ProviderName, WorkspaceType};
```

`:613`,删除夹具中的一行:
```rust
// 删除:
                design_kind: DesignKind::Backend,
```

- [ ] **Step 7.3:清理 workspace_ws_handler.rs**

`src/web/workspace_ws_handler.rs:1442`,删除夹具中的一行:
```rust
// 删除:
                design_kind: crate::product::models::DesignKind::Backend,
```

- [ ] **Step 7.4:验证后端编译通过**

Run: `cargo check --locked 2>&1 | tail -5`
Expected: 无 `DesignKind` 相关错误。若仍有报错,用 `cargo check --locked 2>&1 | grep -i design_kind` 定位遗漏点并清理。

- [ ] **Step 7.5:提交**

```bash
git add src/product/workspace_engine.rs src/product/coding_evaluation_context.rs src/web/workspace_ws_handler.rs
git commit -m "refactor(design_kind): 清理 workspace_engine/coding_evaluation_context/ws_handler 夹具赋值

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 8:后端测试夹具与断言清理

**目标**:清理后端集成测试/单测里的 `design_kind` 夹具字段、请求 body 字段、响应断言、import。

**Files:**
- Modify: `tests/it_product/product_lifecycle_store.rs:12(import)、:65(夹具)`
- Modify: `tests/it_product/product_work_item_split_engine.rs:11(import)、:257(夹具)`
- Modify: `tests/it_web/web_lifecycle_api.rs:301/407/823(body)、:319(断言)`
- Modify: `tests/it_web/web_coding_attempt_api.rs:1120(body)`
- Modify: `tests/it_web/web_work_item_generation.rs:334(body)`
- Modify: `tests/it_web/web_workspace_recovery_consistency.rs:158(body)`

- [ ] **Step 8.1:清理 product_lifecycle_store.rs**

`tests/it_product/product_lifecycle_store.rs:12-13`,把:
```rust
use cadence_aria::product::models::{
    AgentRole, DesignKind, IssueSharedWorktreeStatus, IssueWorkItemDependencyEdge,
```
改为:
```rust
use cadence_aria::product::models::{
    AgentRole, IssueSharedWorktreeStatus, IssueWorkItemDependencyEdge,
```

`:65`,删除夹具中的一行:
```rust
// 删除:
            design_kind: DesignKind::Frontend,
```

- [ ] **Step 8.2:清理 product_work_item_split_engine.rs**

`tests/it_product/product_work_item_split_engine.rs:11`,把:
```rust
use cadence_aria::product::models::{
    DesignKind, IssueRecord, ProviderName, RepositoryRecord, WorkItemKind, WorkItemPlanStatus,
};
```
改为:
```rust
use cadence_aria::product::models::{
    IssueRecord, ProviderName, RepositoryRecord, WorkItemKind, WorkItemPlanStatus,
};
```

`:257`,删除夹具中的一行:
```rust
// 删除:
            design_kind: DesignKind::Backend,
```

- [ ] **Step 8.3:清理 web_lifecycle_api.rs 的请求 body 与断言**

三处请求 body 与一处断言,确切改动如下:

**`:298-307`(`:301` 处,body 中间项,其后有逗号)** —— 删除 `:301` 整行:
```rust
// 改前:
        json!({
            "title":"会话过期后端设计",
            "story_spec_ids":["story_spec_0001"],
            "design_kind":"backend",
            "author_provider":"codex",
            ...
        }),
// 改后(删 design_kind 行,其余不动):
        json!({
            "title":"会话过期后端设计",
            "story_spec_ids":["story_spec_0001"],
            "author_provider":"codex",
            ...
        }),
```

**`:404-408`(`:407` 处,body 末项,无尾逗号)** —— 删除 `:407` 整行,并把 `:406` 行末逗号去掉:
```rust
// 改前:
        json!({
            "title":"会话过期前端设计",
            "story_spec_ids":["story_spec_0001"],
            "design_kind":"frontend"
        }),
// 改后:
        json!({
            "title":"会话过期前端设计",
            "story_spec_ids":["story_spec_0001"]
        }),
```

**`:820-824`(`:823` 处,body 末项,无尾逗号)** —— 同 :407 处理:
```rust
// 改前:
        json!({
            "title":"爬楼梯问题 Design Spec",
            "story_spec_ids":["story_spec_0001"],
            "design_kind":"backend"
        }),
// 改后:
        json!({
            "title":"爬楼梯问题 Design Spec",
            "story_spec_ids":["story_spec_0001"]
        }),
```

**`:319` 断言** —— 删除整行:
```rust
// 删除:
    assert_eq!(design_response["design_specs"][0]["design_kind"], "backend");
```

- [ ] **Step 8.4:清理 web_coding_attempt_api.rs**

`tests/it_web/web_coding_attempt_api.rs:1117-1123`(`:1120` 处,body 中间项,其后 `:1121` 有逗号)—— 删除 `:1120` 整行:
```rust
// 改前:
        json!({
            "title":"爬楼梯 Design",
            "story_spec_ids":["story_spec_0001"],
            "design_kind":"backend",
            "author_provider":"fake",
            "reviewer_provider":"fake"
        }),
// 改后:
        json!({
            "title":"爬楼梯 Design",
            "story_spec_ids":["story_spec_0001"],
            "author_provider":"fake",
            "reviewer_provider":"fake"
        }),
```

- [ ] **Step 8.5:清理 web_work_item_generation.rs**

`tests/it_web/web_work_item_generation.rs:331-339`(`:334` 处,body 中间项,其后 `:335` 有逗号)—— 删除 `:334` 整行:
```rust
// 改前:
        json!({
            "title":"会话过期后端设计",
            "story_spec_ids":["story_spec_0001"],
            "design_kind":"backend",
            "author_provider":"codex",
            "reviewer_provider":"claude_code",
            "review_rounds":2,
            ...
        }),
// 改后:
        json!({
            "title":"会话过期后端设计",
            "story_spec_ids":["story_spec_0001"],
            "author_provider":"codex",
            "reviewer_provider":"claude_code",
            "review_rounds":2,
            ...
        }),
```

- [ ] **Step 8.6:清理 web_workspace_recovery_consistency.rs**

`tests/it_web/web_workspace_recovery_consistency.rs:155-163`(`:158` 处,body 中间项,其后 `:159` 有逗号)—— 删除 `:158` 整行:
```rust
// 改前:
        json!({
            "title": "第二个 Design",
            "story_spec_ids": ["story_spec_0001"],
            "design_kind": "backend",
            "author_provider": "fake",
            "reviewer_provider": null,
            ...
        }),
// 改后:
        json!({
            "title": "第二个 Design",
            "story_spec_ids": ["story_spec_0001"],
            "author_provider": "fake",
            "reviewer_provider": null,
            ...
        }),
```

- [ ] **Step 8.7:运行后端全量检查**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```
Expected: 全绿。若 clippy 报未使用 import,回到对应 Step 清理。若测试失败因 JSON 语法,检查 Step 8.3-8.6 的逗号。

- [ ] **Step 8.8:提交**

```bash
git add tests/
git commit -m "test(design_kind): 清理后端测试夹具、请求 body 与响应断言

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 9:前端删除 types.ts 的 design_kind 字段

**Files:**
- Modify: `web/src/api/types.ts:100`、`:828`

- [ ] **Step 9.1:删除 `DesignSpec.design_kind`**

`web/src/api/types.ts:96-104`,删除一行:
```ts
// 删除:
  design_kind: "frontend" | "backend";
```
保留 `story_spec_ids: string[];` 与 `title: string;`。

- [ ] **Step 9.2:删除 `GenerateDesignSpecsRequest.design_kind`**

`web/src/api/types.ts:825-829`,删除一行:
```ts
// 删除:
  design_kind: "frontend" | "backend";
```

- [ ] **Step 9.3:提交**

```bash
git add web/src/api/types.ts
git commit -m "refactor(design_kind): 删除前端 DesignSpec/GenerateDesignSpecsRequest 的 design_kind 类型

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 10:前端删除 IssueLifecycleWorkbench 的 design_kind 传值

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx:283`、`:487`

- [ ] **Step 10.1:删除两处传值**

`web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`,`:283` 与 `:487` 各删除一行:
```ts
// 删除(两处):
          design_kind: "frontend",
```
> 用 `grep -n 'design_kind: "frontend"' web/src/components/lifecycle/IssueLifecycleWorkbench.tsx` 定位。删后 `story_spec_ids` 与 `title` 等字段逗号需合法(`design_kind` 原为中间项,删后无逗号问题;若为末项则检查上一行逗号)。

- [ ] **Step 10.2:提交**

```bash
git add web/src/components/lifecycle/IssueLifecycleWorkbench.tsx
git commit -m "refactor(design_kind): IssueLifecycleWorkbench 不再传 design_kind

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 11:前端测试夹具清理

**Files:**
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx:1261/1278/703/1700`
- Modify: `web/src/api/client.test.ts:148`
- Modify: `web/src/state/lifecycle-workbench-store.test.ts:98/146`
- Modify: `web/src/components/lifecycle/LifecycleCard.test.tsx:196`

- [ ] **Step 11.1:清理 IssueLifecycleWorkbench.test.tsx**

`:1258-1267` 的 mock payload 类型,删除一行:
```ts
// 删除:
        design_kind: "frontend" | "backend";
```

`:1274-1284` 的 mock design 对象,删除一行:
```ts
// 删除:
        design_kind: payload.design_kind,
```

`:703` 与 `:1700` 两处夹具,各删除一行:
```ts
// 删除:
          design_kind: "frontend",
```
> 用 `grep -n 'design_kind' web/src/components/lifecycle/IssueLifecycleWorkbench.test.tsx` 定位全部 4 处。

- [ ] **Step 11.2:清理 client.test.ts**

`web/src/api/client.test.ts:148`,删除 `generateDesignSpecs` 调用参数中的一行:
```ts
// 删除:
      design_kind: "frontend",
```

- [ ] **Step 11.3:清理 lifecycle-workbench-store.test.ts**

`:98` 与 `:146` 两处夹具,各删除一行:
```ts
// 删除:
      design_kind: "frontend",
```
与:
```ts
// 删除:
      design_kind: "backend",
```

- [ ] **Step 11.4:清理 LifecycleCard.test.tsx**

`web/src/components/lifecycle/LifecycleCard.test.tsx:196`,删除夹具中的一行:
```ts
// 删除:
        design_kind: "frontend",
```

- [ ] **Step 11.5:运行前端全量检查**

```bash
pnpm -C web test
pnpm -C web build
```
Expected: 全绿。若有 TS 类型错误(`Property 'design_kind' does not exist`),回到对应 Step 确认夹具已删干净。

- [ ] **Step 11.6:提交**

```bash
git add web/src/
git commit -m "test(design_kind): 清理前端测试夹具的 design_kind 字段

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Task 12:全量验证与本地 stores 清空

**目标**:跑完整验证链;清空本地 stores 规避旧 JSON 反序列化失败;确认无 design_kind 残留。

- [ ] **Step 12.1:确认无 design_kind 残留**

```bash
grep -rn "design_kind\|DesignKind" src/ web/src/ tests/ 2>/dev/null | grep -v "WorkItemKind\|work_item_kind"
```
Expected: 空输出(无残留)。若仍有输出,定位并清理。

- [ ] **Step 12.2:清空本地 stores**

本地开发环境的 DesignSpecRecord JSON 文件因含 `design_kind` 字段,反序列化会失败。后端启动参数 `--workspace .` 指向 worktree 根,stores 位于 `.worktrees/feat-b-0616/.aria/`。清空该目录:
```bash
rm -rf .aria/
```
> ⚠️ 这一步删除本地开发数据(projects/issues/design specs 等),执行前先与用户确认。用户若要保留旧 project 数据用于手动验证,可改为逐个删除含 `design_kind` 的 DesignSpecRecord JSON(路径形如 `.aria/product/{project_id}/issues/{issue_id}/design_specs/`)。

- [ ] **Step 12.3:全量验证链**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
pnpm -C web test
pnpm -C web build
```
Expected: 全绿。

- [ ] **Step 12.4:定向回归(WorkItemPlan 全流程)**

```bash
cargo test --locked --test it_web work_item_plan
cargo test --locked --test it_product work_item_split
pnpm -C web test -- --run IssueLifecycleWorkbench
```
Expected: 全绿。确认 `force_frontend_backend_split` 校验与 WorkItemPlan 全流程未回归。

- [ ] **Step 12.5:最终提交(若有)**

若 Step 12.1-12.4 有任何清理改动:
```bash
git add -A
git commit -m "chore(design_kind): 收尾清理与验证

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## 验收标准

- [ ] `grep -rn "design_kind\|DesignKind" src/ web/src/ tests/` 无残留(除 `WorkItemKind`/`work_item_kind`)。
- [ ] workbench 中 Design Spec 的 Workspace 标题不再带 `(frontend)`,显示为 `{design.title}`(需启动服务手动确认,见下方「手动确认」)。
- [ ] WorkItemPlan 全流程(prepare → author → revert → review → revision → confirm)在 Fake provider 下贯通测试全绿。
- [ ] `force_frontend_backend_split: true` 校验行为不变。
- [ ] 第 12.3 节验证链全绿。

## 手动确认(可选,服务已启动时)

服务已在 `http://127.0.0.1:5173`(前端)与 `http://127.0.0.1:4317`(后端)运行。实现完成后:
1. 在 workbench 从 Story Spec 生成 Design Spec,打开 Design Workspace。
2. 确认标题栏显示 `{design.title}`,不再有 `(frontend)`。
3. 从 Design Spec 生成 WorkItemPlan,确认 provider 仍能拆出 frontend/backend WorkItem(`force_frontend_backend_split: true`)。

> 注:若后端代码改动,`cargo watch` 会自动重编译重启后端;前端 `vite` 会热更新。若本地 stores 有旧 Design Spec 数据,先清空。
