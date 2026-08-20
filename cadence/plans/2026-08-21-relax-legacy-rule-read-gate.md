# relax-legacy-rule-read-gate-for-generation 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Legacy（单仓）上下文中生成类 prompt 的规则引用从"强制完整读取 AGENTS.md/CLAUDE.md + 失败关闭"降级为按需查阅提示，降低弱模型结构化生成失败率。

**Architecture:** 在 `routing_reference.rs` 新增 `generation_cadence_routing_rules_reference(context)`——Legacy 分支返回降级文案、Logical 分支逐字复用现有政策文案；生成类 prompt 构建点从 `direct_...` 切换到新函数；coding 路径（`provider_context_builder.rs`、`coding_workspace_engine/prompts.rs`）与交互会话入口（`web/workspace_context/prompts.rs`）不动。

**Tech Stack:** Rust（aria daemon，`src/` crate）。

**Spec:** `openspec/changes/relax-legacy-rule-read-gate-for-generation/`（proposal.md / design.md / specs/project-rule-aware-prompts/spec.md / tasks.md）。

## Global Constraints

- REQ-PROMPT-03：Legacy 生成类 prompt 规则引用仅声明位置 + 按需查阅；读取不得作为输出前置；规则文件/工具不可用不得阻塞生成。
- Logical（多仓）文案与 `direct_cadence_routing_rules_reference` 一字不改。
- coding 阶段与交互会话入口的注入一字不改。
- 降级文案必须保留 `[cadence_project_rules]` 段标记（`has_direct_cadence_routing_rules_system_context` 去重谓词依赖它）。
- Rust 构建测试遵循 `cadence/project-rules/build-test-commands.md`：禁止 `-j 1`，使用标准 `cargo test` / 定向过滤。

---

### Task 1: `generation_cadence_routing_rules_reference` 函数与单测

**Files:**
- Modify: `src/product/cadence_skills/routing_reference.rs`
- Test: 同文件 `mod tests`

**Interfaces:**
- Produces: `pub(crate) fn generation_cadence_routing_rules_reference(context: &RoutingReferenceContext) -> String`（Task 2/3 消费）
- Consumes: 现有 `logical_cadence_routing_rules_reference(policy)`（私有，同文件内可见）

- [ ] **Step 1: 写失败测试**（追加到 `mod tests`）

```rust
    #[test]
    fn generation_legacy_reference_is_on_demand_and_non_blocking() {
        let prompt = generation_cadence_routing_rules_reference(&RoutingReferenceContext::Legacy);
        assert!(prompt.contains("[cadence_project_rules]"));
        assert!(prompt.contains("AGENTS.md"));
        assert!(prompt.contains("CLAUDE.md"));
        assert!(prompt.contains("按需"));
        // 不得保留强制完整读取与失败关闭语义
        assert!(!prompt.contains("完整读取"));
        assert!(!prompt.contains("只报告阻塞"));
        assert!(!prompt.contains("不得继续输出"));
    }

    #[test]
    fn generation_logical_reference_matches_direct_exactly() {
        let ctx = logical();
        assert_eq!(
            generation_cadence_routing_rules_reference(&ctx),
            direct_cadence_routing_rules_reference(&ctx)
        );
    }

    #[test]
    fn direct_legacy_reference_unchanged() {
        // 守卫：coding 路径与交互入口继续使用旧文案
        let prompt = direct_cadence_routing_rules_reference(&RoutingReferenceContext::Legacy);
        assert!(prompt.contains("完整读取"));
        assert!(prompt.contains("只报告阻塞"));
    }
```

- [ ] **Step 2: 运行确认编译失败**

Run: `cargo test generation_cadence_routing_rules_reference --lib 2>&1 | tail -5`
Expected: FAIL（函数未定义，编译错误）

- [ ] **Step 3: 最小实现**（加在 `direct_cadence_routing_rules_reference` 之后）

```rust
/// 生成类 prompt（outline/draft/plan/revision/生成侧 review）使用的规则引用。
///
/// Legacy 分支降级为按需查阅：不以规则读取为输出前置，不因文件缺失阻塞生成。
/// Logical 分支与 direct 版逐字一致（政策权威门禁不降级）。
/// 文案保留 `[cadence_project_rules]` 段标记，
/// `has_direct_cadence_routing_rules_system_context` 对新旧变体同判去重。
pub(crate) fn generation_cadence_routing_rules_reference(context: &RoutingReferenceContext) -> String {
    match context {
        RoutingReferenceContext::Legacy => concat!(
            "[cadence_project_rules]\n",
            "当前目标仓库根目录的 AGENTS.md 与 CLAUDE.md 是本任务的流程规则依据；生成候选产物时按需查阅其中适用章节即可，不必完整读取。\n",
            "规则文件缺失或读取失败时，在产物中注明\"项目规则未加载\"并继续生成，不得以此阻塞输出。\n",
        )
        .to_string(),
        RoutingReferenceContext::Logical(policy) => logical_cadence_routing_rules_reference(policy),
    }
}
```

同时把三个测试用到的 `generation_cadence_routing_rules_reference` 加入 `mod tests` 顶部的 `use super::{...}` 导入列表。

- [ ] **Step 4: 运行测试通过**

Run: `cargo test routing_reference --lib 2>&1 | tail -5`
Expected: PASS（含既有全部 routing_reference 测试）

- [ ] **Step 5: 提交**

```bash
git add src/product/cadence_skills/routing_reference.rs
git commit -m "feat(routing-reference): 新增 generation 版规则引用——Legacy 生成类按需查阅、Logical 不变"
```

---

### Task 2: 生成类引擎注入点切换（split_engine + workspace_engine）

**Files:**
- Modify: `src/product/work_item_split_engine/prompts.rs:73,93`（outline 与 draft runtime contract）
- Modify: `src/product/workspace_engine/prompts.rs:69`（`initial_author_runtime_contract`）、`:175`（`reviewer_output_contract`）
- Modify: `src/product/workspace_engine/prompts/revision.rs:82,116`（revision delta/full prompt）
- Test: 上述文件内嵌测试 + `src/product/work_item_split_engine/tests/routing_reference_contract.rs` + `src/product/coding_workspace_engine/tests/parser_prompt/*`

**Interfaces:**
- Consumes: `generation_cadence_routing_rules_reference`（Task 1）
- Produces: 无（末端消费）

- [ ] **Step 1: 写失败测试**——在 `work_item_split_engine/tests/routing_reference_contract.rs` 追加断言（若无对应 harness 则在该文件既有测试函数中扩展）：

```rust
// outline/draft runtime contract 使用按需文案，不含失败关闭语义
let outline_prompt = /* 既有测试中构建 outline runtime contract 的调用 */;
assert!(outline_prompt.contains("按需查阅"));
assert!(!outline_prompt.contains("完整读取"));
assert!(!outline_prompt.contains("只报告阻塞"));
```

同型断言追加到：`workspace_engine/prompts.rs` 的 `initial_author_runtime_contract`（Legacy context）与 `reviewer_output_contract` 测试、`prompts/revision.rs` 的 delta/full prompt 测试（这三个文件已有内嵌 `mod tests` 用 `RoutingReferenceContext::Legacy` 断言旧文案，直接把它们从断言 `完整读取` 改为断言 `按需查阅` / `规则未加载`）。

- [ ] **Step 2: 运行确认失败**

Run: `cargo test routing_reference_contract --lib 2>&1 | tail -5; cargo test workspace_engine::prompts --lib 2>&1 | tail -5`
Expected: FAIL（新断言不匹配旧文案）

- [ ] **Step 3: 切换调用点**——5 处把 `direct_cadence_routing_rules_reference(context)` 改为 `generation_cadence_routing_rules_reference(context)`：
  1. `src/product/work_item_split_engine/prompts.rs:73`
  2. `src/product/work_item_split_engine/prompts.rs:93`
  3. `src/product/workspace_engine/prompts.rs:69`
  4. `src/product/workspace_engine/prompts.rs:175`
  5. `src/product/workspace_engine/prompts/revision.rs:82` 与 `:116`（两处）

同步修改各文件 `use crate::product::cadence_skills::routing_reference::{...}` 导入（保留 `RoutingReferenceContext`；`revision.rs` 若不再使用 direct 版则移除其导入）。

**禁止改动**：`src/cross_cutting/provider_context_builder.rs`、`src/product/coding_workspace_engine/prompts.rs:436`、`src/web/workspace_context/prompts.rs:121` 三处保持 `direct_...`。

- [ ] **Step 4: 定向测试全绿**

Run: `cargo test routing_reference --lib 2>&1 | tail -3 && cargo test parser_prompt --lib 2>&1 | tail -3 && cargo test workspace_engine --lib 2>&1 | tail -3`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/product/work_item_split_engine/prompts.rs src/product/workspace_engine/prompts.rs src/product/workspace_engine/prompts/revision.rs src/product/work_item_split_engine/tests/routing_reference_contract.rs
git commit -m "feat(prompts): 生成类注入点切换 generation 规则引用——outline/draft/author/reviewer/revision"
```

---

### Task 3: 守卫测试与全量回归

**Files:**
- Test: `src/product/coding_workspace_engine/tests/parser_prompt/routing_contract.rs`（如该文件断言生成 prompt 含旧文案需同步改）；新增守卫断言可放 `src/cross_cutting/provider_context_builder.rs` 内嵌测试

**Interfaces:**
- Consumes: Task 1/2 成果

- [ ] **Step 1: 写守卫测试**——在 `provider_context_builder.rs` 内嵌 `mod tests` 追加：

```rust
#[test]
fn coding_path_still_uses_direct_reference() {
    // coding 路径的 routing_reference 槽位必须仍是 direct 版（完整读取 + 失败关闭）
    let value = direct_cadence_routing_rules_reference(&RoutingReferenceContext::Legacy);
    assert!(value.contains("完整读取"));
    assert!(value.contains("只报告阻塞"));
}
```

- [ ] **Step 2: 运行**

Run: `cargo test provider_context_builder --lib 2>&1 | tail -3`
Expected: PASS

- [ ] **Step 3: 全量回归与格式检查**

Run: `cargo test --lib 2>&1 | tail -5 && cargo fmt --check && cargo clippy --lib 2>&1 | tail -5`
Expected: 全绿、无新告警

- [ ] **Step 4: 提交**

```bash
git add src/cross_cutting/provider_context_builder.rs
git commit -m "test(provider-context): coding 路由引用守卫——direct 版文案不变"
```

---

### Task 4: 冒烟验证（工作包 4.1）

- [ ] **Step 1:** 在测试项目（naruto 或等价单仓项目）用弱模型 provider（deepseek-v4-flash / glm-5.3）触发一次 Work Item Group outline 生成，观察：
  - prompt 渲染日志中 routing_reference 段为按需文案；
  - 规则文件缺失/读取失败场景不再出现 context blocker，产物进入正常校验流程。
- [ ] **Step 2:** 把观察结果记入本 Plan 文件末尾的"验证记录"小节，供 reviewer 审查。

## 验证记录

（实施时填写）
