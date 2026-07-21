# CodingWorkspace P2 CodeReview 自动返修配置 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 CodeReview 自动返修次数从配置页进入 attempt，并在额度耗尽后转入人工返修 gate。

**Architecture:** 本计划贯穿 attempt 创建输入、provider config snapshot/API、前端配置页和 reviewer-driven rework gate。它不处理单 WorkItem InternalPrReview 删除和 WorkItemGroup shared worktree；这些放到 P3。

**Tech Stack:** Rust backend, Axum handlers, product store models, React/TypeScript frontend, Zustand store, cargo/pnpm focused tests.

---

## Scope

实现来源：

- `cadence/designs/2026-07-07_技术方案_CodingWorkspace流程精简补充Delta_v1.0.md` 第 4、5、6、12 节
- `cadence/designs/2026-07-07_技术方案_CodingWorkspace材料驱动Prompt协议_v1.0.md` 的 coder delta 人工意见优先级

不做：

- 不改 GroupFinalReview runner 边界。
- 不改 WorkItemGroup worktree prepare。
- 不做 prompt 固定模板去技术栈；这属于 P1。

## Files

- Modify: `src/product/coding_attempt_store/inputs.rs`
- Modify: `src/product/coding_attempt_store/attempt.rs`
- Modify: `src/product/coding_models/provider_config.rs`
- Modify: `src/web/handlers/coding.rs`
- Modify: `src/product/coding_workspace_engine/rework.rs`
- Modify: `src/product/coding_workspace_engine/provider_stream.rs` if delta prompt wiring needs extra context
- Modify: `web/src/api/types/coding.ts`
- Modify: `web/src/state/coding-workspace-store.ts`
- Modify: `web/src/components/coding-workspace/CodingProviderConfigPanel.tsx`
- Test: backend coding attempt store / rework tests
- Test: frontend type/store/component tests if existing test harness covers them

## Task 1: 后端配置字段 TDD

- [ ] 在 coding attempt store 或 handler 相关测试中新增用例：创建 attempt 时 `max_auto_rework` 可以使用非默认值，例如 4。
- [ ] 单 WorkItem attempt 和 WorkItemGroup attempt 都覆盖。
- [ ] 若现有 handler 测试较重，先在 store 层覆盖 `CreateCodingAttemptInput` / `CreateGroupCodingAttemptInput` 的字段传递，再在 web handler 层补一个轻量测试。
- [ ] Run:

```bash
cargo test --locked --lib coding_attempt
```

Expected before implementation: 测试失败，当前 handler 仍硬编码 `max_auto_rework: 2`。

## Task 2: 定义自动返修次数配置模型

- [ ] 在 provider config snapshot 或 attempt creation request 中增加 `max_auto_rework` / `code_review_max_auto_rework` 字段。
- [ ] 默认值为 2。
- [ ] 合法范围为 0 到 5。
- [ ] 后端应 clamp 或 validation reject。推荐 validation reject，错误码使用明确语义，例如 `invalid_max_auto_rework`。
- [ ] 单 WorkItem 和 WorkItemGroup 创建路径都使用该值。
- [ ] 替换以下硬编码点：
  - `src/web/handlers/coding.rs` group 创建中的 `max_auto_rework: 2`
  - `src/web/handlers/coding.rs` single 创建中的 `max_auto_rework: 2`

## Task 3: API / 前端类型同步

- [ ] 更新 `web/src/api/types/coding.ts`：
  - attempt/session state 能读取 `max_auto_rework`。
  - provider config snapshot 或 start request 能携带自动返修次数。
- [ ] 更新 `web/src/state/coding-workspace-store.ts`：
  - 默认值为 2。
  - 设置动作限制 0 到 5。
  - 创建单 WorkItem / WorkItemGroup attempt 时传入配置值。
- [ ] 确保已有 snapshot 恢复时没有该字段时使用默认值 2。
- [ ] 不新增 tester/analyst 配置入口。

## Task 4: CodeReview 配置页 UI

- [ ] 修改 `CodingProviderConfigPanel.tsx`。
- [ ] 在 Code Reviewer 区域增加“自动返修次数”数值控件。
- [ ] 控件范围：0 到 5。
- [ ] 默认展示当前 store 值。
- [ ] 单 WorkItem 不展示 Internal Reviewer 配置。
- [ ] Tester / Analyst 行保持不可见；如果当前数组仍保留这些 role，只要不显示且不影响新响应即可。本计划不要求一次性删除全部历史类型。
- [ ] UI 文案必须简短，不在页面写解释性长文。

## Task 5: 自动返修额度耗尽后的 gate 行为

- [ ] 在 `execute_reviewer_driven_rework` 或相邻 rework 逻辑中确认当前计数判断。
- [ ] 行为调整为：
  - `rework_count < max_auto_rework`：自动返修。
  - `rework_count >= max_auto_rework`：创建人工返修 gate。
- [ ] 自动额度耗尽后不重置。同一 attempt 后续 `request_changes` 继续进入人工 gate。
- [ ] 人工 gate action 至少支持：
  - `provide_context`
  - `continue_rework`
  - `abort`
- [ ] gate payload 展示：
  - review summary
  - findings
  - evidence refs if present
  - raw provider output ref if present
- [ ] 用户继续返修后仍复用 coder provider，不调用 reviewer/analyst provider。

## Task 6: 人工返修意见进入 coder delta prompt

- [ ] 确认 gate 期间的 ContextNote / gate `extra_context` 进入 `ReworkContextNoteInput` 或等价结构。
- [ ] 在构建 coder delta prompt 时追加“人工返修意见”章节。
- [ ] 明确写入优先级：
  - 人工返修意见
  - 最新 CodeReviewer findings
  - 原 Work Item / VerificationPlan
  - 既有上下文
- [ ] 新增测试：当有人工意见和 reviewer findings 时，delta prompt 同时包含二者，并明确人工意见优先。

## Task 7: Verification

- [ ] Backend focused tests:

```bash
cargo test --locked --lib coding_attempt
cargo test --locked --lib gate_rework
cargo test --locked --lib parser_prompt
```

- [ ] Frontend focused tests if existing scripts support it:

```bash
pnpm -C web test
pnpm -C web tsc -b
```

- [ ] Standard backend checks:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

- [ ] Diff check:

```bash
git diff --stat
```

## Completion Criteria

- CodeReview 配置页可以设置自动返修次数，默认 2，范围 0 到 5。
- 单 WorkItem 和 WorkItemGroup attempt 的 `max_auto_rework` 来自配置，不再硬编码。
- `request_changes` 在额度内自动返修。
- 额度耗尽后后续全部进入人工返修 gate。
- 人工 gate 可以补充意见并继续 coder 返修。
- coder delta prompt 包含人工返修意见且声明最高优先级。
