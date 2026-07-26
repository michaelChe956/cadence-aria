# Work Item Draft 串行上下文预算修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让含已接受直接依赖的串行 Work Item Draft 在保持依赖合同可消费的前提下始终低于 11,000-byte Provider 上下文限制，并恢复后续 outline 的生成。

**Architecture:** `build_work_item_draft_invocation` 继续根据 `depends_on` 区分直接依赖和其他已接受 Draft。直接依赖不再序列化整条持久化 `WorkItemDraftRecord` 或完整 Canonical Contract，而是投影稳定身份、上游 `output_contracts` 和 `handoff_contract`；这正是下游建立 `input_contracts`、消费 capability 与交接字段所需的事实，移除任务、验收、验证、写入策略以及持久化元数据。既有 11,000-byte fail-closed guard 保持不变，防止将超长 Prompt 发送至 Provider。

**Tech Stack:** Rust 2024、Serde JSON、既有 Work Item Split Engine 单元/集成测试、Cargo。

## Global Constraints

- 关联 Change：`improve-work-item-draft-generation-reliability`；落实其“Prompt 长度预算测试”和语义闭合 Draft 需求，不放宽 Parser、`WorkItemDraftLocalValidator`、接受门禁或 11,000-byte 上限。
- 仅修改 `src/product/work_item_split_engine/prompts.rs` 与其确定性测试；不得新增评估模块、CLI、CI、Hook、Provider 调用、持久化语料或产品 API。
- 直接依赖 Prompt 投影必须保留 `outline_id`、`draft_id`、`logical_work_item_id`、`output_contracts` 与 `handoff_contract`；不得保留任务、验收、验证、写入策略、`project_id`、`issue_id`、时间戳、状态或其他持久化元数据。
- `其他已 accepted draft 摘要` 维持现有紧凑摘要；非依赖 Draft 不得升级为完整内容。
- Rust 命令必须使用 `--locked` 且不得使用 `-j 1`。
- 此任务改变 Work Item Draft Prompt。确定性验证后，交付前必须提醒操作者按 `cadence/project-rules/work-item-draft-prompt-validation.md` 明确授权 Case A、Case B 各 10 个有效首次输出的真实 Claude Code 验证；未授权不得调用 Provider。

---

## 文件结构与职责

| 文件 | 动作 | 职责 |
|---|---|---|
| `src/product/work_item_split_engine/prompts.rs` | 修改 | 将直接依赖的持久化 Draft 记录投影为最小且可消费的输出/交接合同上下文。 |
| `src/product/work_item_split_engine/tests/part_01.rs` | 修改 | 覆盖“接受 backend Draft 后生成 frontend Draft”仍可构建、保留合同且不泄漏持久化元数据。 |
| `openspec/changes/improve-work-item-draft-generation-reliability/tasks.md` | 修改 | 验证完成后记录本次确定性 Prompt 长度回归。 |

### Task 1: 以串行直接依赖固定 Prompt 预算回归

**Files:**

- Modify: `src/product/work_item_split_engine/tests/part_01.rs:603-636`
- Test: `src/product/work_item_split_engine/tests/part_01.rs`

**Interfaces:**

- Consumes: `build_work_item_draft_invocation(&WorkItemPlanOutline, &str, WorkItemGenerationMode, &[WorkItemDraftRecord], Option<&str>) -> ApiResult<WorkItemDraftInvocation>`。
- Produces: 一个证明 frontend Draft 可消费 accepted backend Canonical Contract 且不会触发 `work_item_draft_prompt_too_large` 的回归测试。

- [ ] **Step 1: 将旧的“应拒绝”断言替换为目标行为测试**

把 `single_item_prompt_rejects_oversized_accepted_previous_context_before_provider_invocation` 重命名为 `single_item_prompt_projects_direct_dependency_within_provider_budget`。保留现有的两 outline fixture、`SessionStatusDto` output contract、`Some("补充错误态")` feedback 与 `accepted_backend`。将 `expect_err` 改为：

```rust
let invocation = build_work_item_draft_invocation(
    &outline,
    "outline_frontend",
    WorkItemGenerationMode::Serial,
    &[accepted_backend],
    Some("补充错误态"),
)
.expect("direct dependency context must stay within the provider budget");

assert!(
    invocation.prompt.len() < WORK_ITEM_DRAFT_PROMPT_MAX_BYTES,
    "direct dependency prompt is {} bytes",
    invocation.prompt.len()
);
assert!(invocation.prompt.contains("SessionStatusDto"));
assert!(invocation.prompt.contains("handoff_contract"));
assert!(!invocation.prompt.contains("\"project_id\""));
assert!(!invocation.prompt.contains("\"accepted_at\""));
```

在测试模块顶部从 `prompts` 导入 `WORK_ITEM_DRAFT_PROMPT_MAX_BYTES`，不使用硬编码 `11_000`。

- [ ] **Step 2: 运行 RED 验证**

Run:

```text
cargo test --locked --lib single_item_prompt_projects_direct_dependency_within_provider_budget
```

Expected: FAIL，错误为现有 `work_item_draft_prompt_too_large`；该失败证明当前完整 `WorkItemDraftRecord` 投影超过 budget，而非测试夹具错误。

### Task 2: 投影直接依赖的可消费交接合同并验证 GREEN

**Files:**

- Modify: `src/product/work_item_split_engine/prompts.rs:540-568`
- Modify: `src/product/work_item_split_engine/tests/part_01.rs:603-636`
- Test: `src/product/work_item_split_engine/tests/part_01.rs`

**Interfaces:**

- Consumes: `direct_dependencies: &[&WorkItemDraftRecord]`，其中 `candidate.canonical_contract_candidate.output_contracts` 与 `handoff_contract` 是下游输入合同的唯一权威内容。
- Produces: `direct_dependency_json`，每项只含 `outline_id`、`draft_id`、`logical_work_item_id`、`output_contracts` 和 `handoff_contract`；仍以 `serde_json::to_string_pretty` 注入 `[直接依赖的可消费交接合同]`。

- [ ] **Step 1: 用可消费的输出/交接合同投影替换整条记录序列化**

在 `build_work_item_draft_prompt` 中将：

```rust
let direct_dependency_json =
    serde_json::to_string_pretty(direct_dependencies).unwrap_or_else(|_| "[]".to_string());
```

替换为：

```rust
let direct_dependency_json = serde_json::to_string_pretty(
    &direct_dependencies
        .iter()
        .map(|draft| {
            serde_json::json!({
                "outline_id": &draft.outline_id,
                "draft_id": &draft.draft_id,
                "logical_work_item_id": &draft.candidate.logical_work_item_id,
                "output_contracts": &draft.candidate.canonical_contract_candidate.output_contracts,
                "handoff_contract": &draft.candidate.canonical_contract_candidate.handoff_contract,
            })
        })
        .collect::<Vec<_>>(),
)
.unwrap_or_else(|_| "[]".to_string());
```

将 Prompt 段落标题从 `[直接依赖 draft 完整内容]` 改为 `[直接依赖的可消费交接合同]`，并说明这些输出 contract 与交接字段是建立当前 `input_contracts` 的唯一依赖事实。借用字段（`&draft.outline_id` 等）而非移动字段，以保持 `&WorkItemDraftRecord` 的所有权；实现可等价地使用显式局部 projection，但不得引入新的持久化类型或依赖。

- [ ] **Step 2: 运行 GREEN 与模块回归**

Run:

```text
cargo test --locked --lib single_item_prompt_projects_direct_dependency_within_provider_budget
cargo test --locked --lib work_item_split_engine
cargo test --locked --test it_web item_review_pass_starts_next_outline -- --nocapture
```

Expected: 三条命令通过；集成测试能为 `outline_frontend_expiry` 创建 Draft run，而不会出现 `work item draft prompt exceeds the 11000-byte provider-context limit`。

- [ ] **Step 3: 完成静态质量检查**

Run:

```text
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
git diff --check
```

Expected: 全部通过，且没有格式、lint 或空白错误。

### Task 3: 记录验证并保持真实 Prompt 试运行显式授权

**Files:**

- Modify: `openspec/changes/improve-work-item-draft-generation-reliability/tasks.md`
- Modify: `openspec/changes/add-repository-initialization-git-finalize/tasks.md`
- Test: 项目规定的 Git 收尾、文件大小守卫、Rust/前端门禁

**Interfaces:**

- Consumes: Task 1-2 的确定性通过证据与 Git 初始化 Change 的既有实现。
- Produces: 两个 Change 的真实完成状态；Provider 试运行仍须由人显式授权后才执行。

- [ ] **Step 1: 请求真实 Claude Code 试运行授权（不自动调用）**

向操作者发送以下提醒并等待明确授权：

> 本次改动涉及 Work Item Draft Prompt 或其结构化契约。建议按 `cadence/designs/2026-07-24_技术方案_WorkItemDraftPrompt测试基线_v1.0.md` 执行 Case A 与 Case B 各 10 个有效首次输出的 Claude Code 验证；是否授权执行？

未获授权时，停止在确定性验证状态；不得调用 Provider，也不得勾选 `improve-work-item-draft-generation-reliability` 的 3.3 或 4.1。

- [ ] **Step 2: 运行 Git 初始化 Change 的最终质量门禁**

Run:

```text
cargo test --locked --test it_core large_file_guard::product_source_and_test_files_stay_under_line_limit
cargo test --locked --lib --quiet
cargo test --locked --test it_web web_repository_initialization
pnpm exec vitest run src/components/lifecycle/CreateRepositoryDialog.test.tsx src/api/client.test.ts src/api/types.test.ts
pnpm tsc -b
pnpm test
openspec validate add-repository-initialization-git-finalize --strict
git diff --check
```

Expected: Git 收尾功能、800 行守卫、进程重试和前端进度面板均通过。需要回环端口的 `it_web` 测试以受限提权运行；不因 Provider 未授权而伪称 Prompt 试运行完成。

- [ ] **Step 3: 仅按获得的证据更新 OpenSpec tasks**

只有 Task 2 的质量门禁全部通过后，勾选 `add-repository-initialization-git-finalize/tasks.md` 的 1.1-3.3。`improve-work-item-draft-generation-reliability/tasks.md` 仅在操作者授权且 Case A、Case B 各 10/10 首次输出通过后勾选 3.3 和 4.1；否则保留未完成并在交付中说明。

## Plan Self-Review

- 覆盖：Task 1 固定了已接受 backend 后前端 Draft 无法启动的真实失败；Task 2 只保留下游实际消费的 output/handoff 合同，移除与依赖消费无关的 Canonical Contract 内容和持久化元数据；Task 3 区分确定性基线与需要人工授权的真实 Provider 验证。
- 类型一致性：实现消费 `WorkItemDraftRecord` 已有的 `candidate.canonical_contract_candidate`，没有新增 API 或存储类型；测试仍走公开 `build_work_item_draft_invocation`。
- 边界：保持 11,000-byte fail-closed guard、Parser、Validator 和接受门禁不变；没有把 Prompt 试运行固化进 CI 或产品代码。
