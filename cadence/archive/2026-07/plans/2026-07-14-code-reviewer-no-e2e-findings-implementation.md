# Code Reviewer 禁止 E2E Findings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 仅通过 Prompt 调优，使 CodeReviewer 与 WorkItemGroup GroupFinalReview 不再提出 E2E、Playwright 或浏览器自动化测试 findings，同时保留其他测试与验证建议能力。

**Architecture:** 在 `prompts.rs` 定义一个技术栈中立的共享 Reviewer 测试边界协议。单 Work Item CodeReviewer 和整组 GroupFinalReview 分别把该协议注入自己的 Prompt；不修改 Coder Prompt、Review parser、报告持久化、rework 或 gate 流程。

**Tech Stack:** Rust、Tokio 测试、Cargo；平台整体非 E2E 回归使用现有 Vitest 与 TypeScript 构建。

## Global Constraints

- CodeReviewer 与 GroupFinalReview 都不得把 E2E、Playwright、浏览器自动化测试或其浏览器环境安装转化为 finding、否决理由或 Coder 返修要求。
- 单元测试、非浏览器自动化集成测试、编译、构建、类型检查、静态分析、格式检查和 lint 仍允许提出。
- Reviewer 的测试建议不受 Verification Plan 已列命令的严格限制。
- 不修改 Coder Prompt 或 Coder 执行优先级。
- 不增加关键字过滤、运行时特殊拦截、Review schema、parser、rework 或 gate 逻辑。
- 共享 Prompt 协议不得假设被开发项目使用 Rust、React、Cargo、Vite、Vitest 或其他固定技术栈；Playwright 仅作为明确禁止的测试框架被点名。
- 不运行 Playwright 或任何 E2E 测试。
- 所有 Cargo 命令禁止携带 `-j 1`。

---

### Task 1: 为 CodeReviewer 增加技术栈中立的非 E2E 测试边界

**Files:**
- Modify: `src/product/coding_workspace_engine/prompts.rs:4-57,255-310`
- Test: `src/product/coding_workspace_engine/tests/parser_prompt.rs:61-215`

**Interfaces:**
- Consumes: 现有 `build_code_review_prompt()`、`code_review_material_protocol()`、`no_default_stack_assumption_contract()` 与测试辅助函数 `assert_no_fixed_stack_terms()`。
- Produces: `pub(crate) fn reviewer_test_scope_contract() -> &'static str`，供所有 Reviewer Prompt 复用。

- [ ] **Step 1: 先写共享协议与 CodeReviewer 接线的失败测试**

在 `parser_prompt.rs` 的 `code_review_material_protocol_requires_material_derived_checklist` 之前增加：

```rust
#[test]
fn reviewer_test_scope_contract_forbids_e2e_findings_without_restricting_other_tests() {
    let contract = reviewer_test_scope_contract();

    assert_no_fixed_stack_terms(contract);
    assert!(contract.contains("单元测试"));
    assert!(contract.contains("非浏览器自动化的集成测试"));
    assert!(contract.contains("编译、构建、类型检查、静态分析、格式检查或 lint"));
    assert!(contract.contains("不受 Verification Plan 已列命令的严格限制"));
    assert!(contract.contains("E2E"));
    assert!(contract.contains("Playwright"));
    assert!(contract.contains("浏览器自动化测试"));
    assert!(contract.contains("不得因为上述测试缺失、失败或缺少证据"));
    assert!(contract.contains("request_changes 或 blocked"));
    assert!(contract.contains("不得将其转换成 finding、否决理由或 Coder 返修要求"));
}
```

在现有 `group_attempt_prompts_use_current_work_item_id` 测试中，生成 `coding_prompt` 和 `review_prompt` 后增加：

```rust
assert!(review_prompt.contains("Reviewer 非 E2E 测试边界"));
assert!(review_prompt.contains("Playwright"));
assert!(review_prompt.contains("单元测试"));
assert!(!coding_prompt.contains("Reviewer 非 E2E 测试边界"));
```

- [ ] **Step 2: 运行测试并确认先失败**

Run:

```bash
cargo test --locked --lib reviewer_test_scope_contract_forbids_e2e_findings_without_restricting_other_tests
```

Expected: FAIL，编译错误包含 `cannot find function reviewer_test_scope_contract`。

- [ ] **Step 3: 实现共享 Reviewer 测试边界协议**

在 `prompts.rs` 的 `no_default_stack_assumption_contract()` 后增加：

```rust
pub(crate) fn reviewer_test_scope_contract() -> &'static str {
    "\nReviewer 非 E2E 测试边界:\n\
     - 你可以根据需求、当前 diff、仓库事实、测试证据和代码风险提出单元测试、非浏览器自动化的集成测试、编译、构建、类型检查、静态分析、格式检查或 lint 等验证要求。\n\
     - 这些测试建议不受 Verification Plan 已列命令的严格限制，但测试框架、命令和技术栈判断必须来自任务材料、仓库事实或项目规则，不得凭平台默认假设生成。\n\
     - 不得创建以新增、执行、补充、修复、配置或安装 E2E、端到端测试、Playwright、浏览器自动化测试或运行这些测试所需浏览器环境为目的的 finding。\n\
     - 不得因为上述测试缺失、失败或缺少证据而给出 request_changes 或 blocked。\n\
     - 即使 Work Item、Design Spec、Verification Plan、handoff 或 EvaluationContextPack 提到上述测试，也不得将其转换成 finding、否决理由或 Coder 返修要求。\n"
}
```

- [ ] **Step 4: 将共享协议注入 CodeReviewer Prompt**

把 `build_code_review_prompt()` 中 `Base` 后的协议占位符从两个扩展为三个：

```rust
             {}\
             {}\
             {}\
             \n代码规范:\n\
```

对应参数保持以下顺序：

```rust
            code_review_material_protocol(),
            reviewer_test_scope_contract(),
            no_default_stack_assumption_contract(),
```

- [ ] **Step 5: 运行定向单元测试并确认通过**

Run:

```bash
cargo test --locked --lib reviewer_test_scope_contract_forbids_e2e_findings_without_restricting_other_tests
cargo test --locked --lib group_attempt_prompts_use_current_work_item_id
cargo test --locked --lib code_review_material_protocol_requires_material_derived_checklist
```

Expected: 三条命令均 PASS；CodeReviewer Prompt 包含共享协议，Coder Prompt 不包含该协议。

- [ ] **Step 6: 提交 CodeReviewer Prompt 修改**

```bash
git add src/product/coding_workspace_engine/prompts.rs \
  src/product/coding_workspace_engine/tests/parser_prompt.rs
git commit -m "fix: 禁止 code reviewer 提出 E2E findings"
```

---

### Task 2: 将同一规则接入 WorkItemGroup GroupFinalReview

**Files:**
- Modify: `src/product/coding_workspace_engine/internal_pr_review.rs:36-113`
- Test: `tests/it_product/product_coding_workspace_engine/part_13.rs:489-505`

**Interfaces:**
- Consumes: Task 1 交付的 `reviewer_test_scope_contract() -> &'static str`。
- Produces: GroupFinalReview Prompt 与 CodeReviewer 使用完全相同的测试边界语义。

- [ ] **Step 1: 先写 GroupFinalReview 接线失败测试**

在 `group_final_review_prompt_includes_all_unit_handoffs` 中增加：

```rust
assert!(prompt.contains("Reviewer 非 E2E 测试边界"));
assert!(prompt.contains("Playwright"));
assert!(prompt.contains("单元测试"));
assert!(prompt.contains("不受 Verification Plan 已列命令的严格限制"));
assert!(prompt.contains("不得因为上述测试缺失、失败或缺少证据"));
```

- [ ] **Step 2: 运行 GroupFinalReview 测试并确认先失败**

Run:

```bash
cargo test --locked --test it_product group_final_review_prompt_includes_all_unit_handoffs
```

Expected: FAIL，断言显示 GroupFinalReview Prompt 尚不包含 `Reviewer 非 E2E 测试边界`。

- [ ] **Step 3: 将共享协议注入 GroupFinalReview Prompt**

把 `build_group_internal_pr_review_prompt()` 中完整 diff 后的协议占位符从三个扩展为四个：

```rust
             {}\
             {}\
             {}\
             {}\
             \n输出要求:\n\
```

对应参数保持以下顺序：

```rust
            group_final_review_material_protocol(),
            reviewer_test_scope_contract(),
            no_default_stack_assumption_contract(),
            retry_diagnostic_section
```

- [ ] **Step 4: 运行 GroupFinalReview 定向测试并确认通过**

Run:

```bash
cargo test --locked --test it_product group_final_review_prompt_includes_all_unit_handoffs
```

Expected: PASS；GroupFinalReview Prompt 包含与 CodeReviewer 相同的共享测试边界协议。

- [ ] **Step 5: 运行 Prompt 相关回归测试**

Run:

```bash
cargo test --locked --lib coding_workspace_engine::tests::parser_prompt
cargo test --locked --test it_product group_final_review_prompt_includes_all_unit_handoffs
```

Expected: 全部 PASS，无现有 Prompt、结构化输出或 group handoff 测试回归。

- [ ] **Step 6: 提交 GroupFinalReview Prompt 修改**

```bash
git add src/product/coding_workspace_engine/internal_pr_review.rs \
  tests/it_product/product_coding_workspace_engine/part_13.rs
git commit -m "fix: 禁止 group final review 提出 E2E findings"
```

---

## Final Verification

- [ ] **Step 1: 运行 Rust 标准验证**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

Expected: 所有命令退出码为 0；不得添加 `-j 1`。

- [ ] **Step 2: 运行当前平台前端非 E2E 回归**

```bash
cd web
pnpm test
pnpm tsc -b
```

Expected: Vitest 全部通过，TypeScript 构建退出码为 0；不运行 `pnpm test:e2e` 或 Playwright。

- [ ] **Step 3: 核对最终范围**

```bash
git status --short
git diff HEAD~2 --stat
git diff HEAD~2 -- src/product/coding_workspace_engine/prompts.rs \
  src/product/coding_workspace_engine/internal_pr_review.rs \
  src/product/coding_workspace_engine/tests/parser_prompt.rs \
  tests/it_product/product_coding_workspace_engine/part_13.rs
```

Expected: 实现改动仅限两个 Prompt 构造文件和两个测试文件；没有 Coder Prompt、Review parser、rework、gate、前端或 E2E 文件变更。
