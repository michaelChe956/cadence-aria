# Work Item Plan 单会话最少拆分实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在每个 Work Item 能由单个 coding session 可靠完成的前提下，以 40k 软警戒线和 50k 硬上限生成尽可能少的 Outline，并让 Reviewer 拒绝不必要拆分。

**Architecture:** 保持现有 Outline → Review → Human Confirm → Serial/Batch Draft 状态机和数据模型不变，仅调整两条 Work Item Plan 生成路径的 Prompt、结构化输出 Schema、确定性 Validator 与 Outline Reviewer 规则。过度拆分使用现有 `ReviewFinding` 表达：`severity=must_fix`，`message` 以 `[outline_unnecessary_split]` 开头，避免新增 finding code 字段或迁移历史数据。

**Tech Stack:** Rust、serde_json、现有 Workspace Engine、Cargo 单元测试。

## Global Constraints

- 预算 `1..=40000` 为正常范围，`40001..=50000` 为 Reviewer 判断范围，`>50000` 必须拒绝。
- 缺失或为 `0` 的 `estimated_context_tokens` 继续拒绝。
- Outline Author 必须以“最大内聚、最少拆分、先合并后证明必须拆”为目标。
- 用户显式拆分选项、外部中断点、独立回滚/验收边界和现有上下文代理指标优先于最少拆分。
- 不新增或修改 Outline、Draft、Work Item、ReviewFinding 数据字段。
- 不迁移、不重算、不重新审核、不自动合并任何历史数据。
- 逐个 Draft 与批量 Draft 流程不变。
- Rust 验证直接使用宿主机 Cargo；禁止 `-j 1`；定向单测必须使用 `cargo test --locked --lib <过滤名>`。

---

### Task 1: 对齐结构化 Outline Author、Revision、Draft Prompt 与 JSON Schema

**Files:**
- Modify: `src/product/work_item_split_engine/tests/part_01.rs`
- Modify: `src/product/work_item_split_engine/prompts.rs`
- Modify: `src/product/work_item_split_engine/schema.rs`

**Interfaces:**
- Consumes: `build_outline_prompt`、`build_outline_revision_prompt`、`build_work_item_draft_invocation`、`WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA`。
- Produces: 所有结构化 Outline/Draft Prompt 统一使用 40k 软线、50k 硬线与最少拆分规则；Schema 接受 `1..=50000`。

- [ ] **Step 1: 写 Outline Author 与 Revision Prompt 的失败断言**

将两个 runtime contract 测试中对 `20k` 的断言替换为以下规则断言：

```rust
for required in [
    "40k",
    "50k",
    "最大内聚",
    "最少拆分",
    "优先合并",
] {
    assert!(
        prompt.contains(required),
        "outline prompt must include `{required}`: {prompt}"
    );
}
assert!(!prompt.contains("1..19999"));
```

同时在 `single_item_prompt_requires_executable_plan_runtime_contracts` 增加：

```rust
assert!(invocation.prompt.contains("50k"));
assert!(!invocation.prompt.contains("小于 20k"));
```

- [ ] **Step 2: 将 Schema 边界期望改为 50000**

```rust
assert_eq!(
    outline_item["properties"]["estimated_context_tokens"]["maximum"],
    serde_json::json!(50000)
);
```

- [ ] **Step 3: 运行定向测试并确认 RED**

Run:

```bash
cargo test --locked --lib work_item_plan_outline_prompt_includes_runtime_contracts
cargo test --locked --lib work_item_plan_outline_revision_prompt_includes_runtime_contracts
cargo test --locked --lib outline_output_schema_makes_outline_and_context_blockers_mutually_exclusive
cargo test --locked --lib single_item_prompt_requires_executable_plan_runtime_contracts
```

Expected: FAIL；现有 Prompt 仍包含 20k/19999，Schema 最大值仍为 19999，且缺少最大内聚/最少拆分/优先合并文案。

- [ ] **Step 4: 最小修改结构化 Prompt**

在 `work_item_plan_runtime_contract`、首次 Outline Prompt、Outline Revision Prompt 中统一加入以下语义，不新增新函数：

```text
- 拆分目标是在每个 Work Item 能由单个 Claude Code 或 Codex coding 会话可靠完成的前提下，使 outline 数量最少。
- 必须按最大内聚任务生成，优先合并目标一致、写入范围相同或重叠、可在同一 session 完成编码与验证的工作；先合并，再证明为什么必须拆。
- estimated_context_tokens 不超过 40k 属正常范围；40001..=50000 可输出并交由 Reviewer 判断；超过 50k 必须继续拆分。
- API、数据层、UI、测试或 TDD 子步骤本身不是独立拆分理由；用户显式拆分选项和必要中断边界除外。
```

把首次与 Revision Prompt 的字段规则从 `estimated_context_tokens(1..19999)` 改为：

```text
estimated_context_tokens(1..=50000)
```

把 Draft Prompt 的 20k 规则改为：

```text
- 当前 outline 的 estimated_context_tokens 必须在 1..=50000 且 session_fit 必须为 fits_single_agent_session；implementation_context 只能展开当前已确认 Outline，不得新增兄弟任务或 Issue 级计划。
```

- [ ] **Step 5: 将 JSON Schema 最大值改为 50000**

```json
"estimated_context_tokens": {
  "type": "integer",
  "minimum": 1,
  "maximum": 50000
}
```

- [ ] **Step 6: 运行定向测试并确认 GREEN**

Run: 重复 Step 3 的四条命令。

Expected: PASS。

- [ ] **Step 7: 提交 Task 1**

```bash
git add src/product/work_item_split_engine/tests/part_01.rs src/product/work_item_split_engine/prompts.rs src/product/work_item_split_engine/schema.rs
git commit -m "feat: raise work item outline session budget"
```

---

### Task 2: 调整 Outline Validator 的硬上限与边界测试

**Files:**
- Modify: `src/product/work_item_split_validator/tests.rs`
- Modify: `src/product/work_item_split_validator/outline.rs`

**Interfaces:**
- Consumes: `WorkItemPlanOutlineValidator::validate`、`WorkItemOutlineSessionFit`。
- Produces: `1..=50000` 合法；缺失、零值和 `>50000` 保持确定性错误；`TooLargeMustSplit` 仍拒绝。

- [ ] **Step 1: 拆分现有单 session 预算测试并写边界失败测试**

保留缺失与 `TooLargeMustSplit` 测试，把超限值改为 `50_001`：

```rust
#[test]
fn outline_validator_requires_single_session_budget() {
    let mut outline = valid_outline();
    outline.work_item_outlines[0].estimated_context_tokens = None;
    outline.work_item_outlines[0].session_fit = None;
    outline.work_item_outlines[1].estimated_context_tokens = Some(50_001);
    outline.work_item_outlines[1].session_fit =
        Some(WorkItemOutlineSessionFit::TooLargeMustSplit);

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "outline_budget_required");
    assert_has_code(&report, "outline_session_fit_required");
    assert_has_code(&report, "outline_exceeds_single_session_budget");
    assert_has_code(&report, "outline_too_large_must_split");
}
```

新增合法边界测试：

```rust
#[test]
fn outline_validator_accepts_soft_and_hard_session_budget_boundaries() {
    for value in [40_000, 40_001, 50_000] {
        let mut outline = valid_outline();
        outline.work_item_outlines[0].estimated_context_tokens = Some(value);

        let report = WorkItemPlanOutlineValidator::validate(&outline);

        assert!(
            !has_code(&report, "outline_exceeds_single_session_budget"),
            "budget {value} should be accepted, got {:?}",
            report.findings
        );
    }
}
```

- [ ] **Step 2: 运行 Validator 测试并确认 RED**

Run:

```bash
cargo test --locked --lib outline_validator_requires_single_session_budget
cargo test --locked --lib outline_validator_accepts_soft_and_hard_session_budget_boundaries
```

Expected: 至少第二条 FAIL；当前 Validator 会拒绝 40k、40001 和 50k。

- [ ] **Step 3: 最小修改 Validator**

将常量与匹配边界改为：

```rust
const SINGLE_AGENT_SESSION_CONTEXT_TOKEN_HARD_LIMIT: u32 = 50_000;

match item.estimated_context_tokens {
    Some(value)
        if value > 0 && value <= SINGLE_AGENT_SESSION_CONTEXT_TOKEN_HARD_LIMIT => {}
    Some(0) | None => findings.push(error(
        "outline_budget_required",
        format!(
            "outline {} must include estimated_context_tokens between 1 and 50000",
            item.outline_id
        ),
        vec![item.outline_id.clone()],
    )),
    Some(value) => findings.push(error(
        "outline_exceeds_single_session_budget",
        format!(
            "outline {} estimated_context_tokens {} exceeds the single-agent session budget of <=50000",
            item.outline_id, value
        ),
        vec![item.outline_id.clone()],
    )),
}
```

- [ ] **Step 4: 运行 Validator 测试并确认 GREEN**

Run: 重复 Step 2 两条命令，然后运行：

```bash
cargo test --locked --lib work_item_split_validator
```

Expected: PASS。

- [ ] **Step 5: 提交 Task 2**

```bash
git add src/product/work_item_split_validator/tests.rs src/product/work_item_split_validator/outline.rs
git commit -m "fix: align outline budget validator"
```

---

### Task 3: 为 Outline Reviewer 增加软区间与过度拆分检查

**Files:**
- Modify: `src/product/workspace_engine/tests/part_08.rs`
- Modify: `src/product/workspace_engine/prompts/review.rs`

**Interfaces:**
- Consumes: `WorkspaceEngine::build_work_item_plan_outline_review_input` 和现有 `ReviewFinding` schema。
- Produces: Reviewer Prompt 同时检查 40k–50k 单 session 可完成性与不必要拆分；无需修改 ReviewFinding 结构或路由代码。

- [ ] **Step 1: 写 Reviewer Prompt 的失败断言**

在 `build_work_item_plan_outline_review_input_includes_boundary_rules` 中替换 20k 断言并增加：

```rust
for required in [
    "40k",
    "50k",
    "最大内聚",
    "最少拆分",
    "不必要拆分",
    "[outline_unnecessary_split]",
] {
    assert!(
        input.prompt.contains(required),
        "outline reviewer prompt must include `{required}`: {}",
        input.prompt
    );
}
assert!(input.prompt.contains("severity=must_fix"));
assert!(input.prompt.contains("target_outline_id"));
assert!(!input.prompt.contains("小于 20k"));
```

- [ ] **Step 2: 运行 Reviewer Prompt 测试并确认 RED**

Run:

```bash
cargo test --locked --lib build_work_item_plan_outline_review_input_includes_boundary_rules
```

Expected: FAIL；现有 Prompt 仍要求小于 20k，且没有不必要拆分规则与稳定标记。

- [ ] **Step 3: 最小修改 Reviewer Prompt**

将 Outline 审核边界说明改为以下语义：

```text
每个 outline 必须能由单个 Claude Code 或 Codex coding 会话可靠完成。estimated_context_tokens 必须存在：不超过 40k 属正常范围，40001..=50000 必须结合目标内聚性、写入范围、编码、测试、返修与验证判断是否能在单 session 闭环，超过 50k 必须返回 revise 并要求拆分。

同时检查过度拆分：在不违反用户显式拆分选项、50k 上限、必要中断点、独立回滚/验收边界和上下文代理指标时，目标一致且可以在同一 session 闭环的 outline 必须合并。发现不必要拆分时返回 revise，并给出 severity=must_fix 的 finding；message 必须以 [outline_unnecessary_split] 开头，target_outline_id 引用其中一个现有 outline，evidence 列出全部可合并 outline ID，required_action 明确要求合并。
```

保持现有 structured output JSON schema 字段不变，避免新增 `code` 字段；只在 reviewer output contract 的说明中加入上述稳定标记约定。

- [ ] **Step 4: 运行 Reviewer Prompt 测试并确认 GREEN**

Run:

```bash
cargo test --locked --lib build_work_item_plan_outline_review_input_includes_boundary_rules
cargo test --locked --lib workspace_engine
```

Expected: PASS。

- [ ] **Step 5: 提交 Task 3**

```bash
git add src/product/workspace_engine/tests/part_08.rs src/product/workspace_engine/prompts/review.rs
git commit -m "feat: reject unnecessary outline splitting"
```

---

### Task 4: 对齐 Markdown Work Item 与 Work Item Plan Prompt

**Files:**
- Modify: `src/web/workspace_context/tests.rs`
- Modify: `src/web/workspace_context/prompts.rs`

**Interfaces:**
- Consumes: `output_schema_for(WorkspaceType::WorkItem)`、`output_schema_for(WorkspaceType::WorkItemPlan)`。
- Produces: Markdown 路径与结构化路径共享 40k/50k 和最少拆分语义；Story/Design Prompt 保持不变。

- [ ] **Step 1: 写 Markdown Prompt 的失败断言**

更新 `work_item_output_schema_describes_single_task_not_issue_level_split`：

```rust
assert!(schema.contains("40k"));
assert!(schema.contains("50k"));
assert!(schema.contains("单个可执行任务"));
assert!(schema.contains("禁止跨任务"));
assert!(!schema.contains("20k"));
assert!(!schema.contains("任务拆分"));
```

更新 `work_item_plan_output_schema_requires_single_session_task_sizing`：

```rust
for required in ["40k", "50k", "最大内聚", "最少拆分", "优先合并"] {
    assert!(schema.contains(required), "missing `{required}`: {schema}");
}
assert!(schema.contains("单个 Claude Code 或 Codex 会话"));
assert!(schema.contains("继续拆分"));
assert!(!schema.contains("20k"));
```

- [ ] **Step 2: 运行 Markdown Prompt 测试并确认 RED**

Run:

```bash
cargo test --locked --lib work_item_output_schema_describes_single_task_not_issue_level_split
cargo test --locked --lib work_item_plan_output_schema_requires_single_session_task_sizing
```

Expected: FAIL；当前 Markdown Prompt 仍使用 20k，且缺少最少拆分语义。

- [ ] **Step 3: 最小修改 Markdown Prompt**

Work Item 文案调整为：

```text
内容规模不超过 40k 属正常范围，40001..=50000 必须仍能由单个会话完成编码、返修与验证，超过 50k 不得作为单个 Work Item；禁止跨任务内容、兄弟任务、Issue 级完整计划和其它任务的交叉内容。
```

Work Item Plan 文案调整为：

```text
拆分目标是在单个 Claude Code 或 Codex 会话可完成的前提下最少拆分。每个任务必须最大内聚，优先合并目标一致、范围重叠且可在同一会话闭环的工作；不超过 40k 属正常范围，40001..=50000 需经 Reviewer 判断，超过 50k 必须继续拆分。
```

- [ ] **Step 4: 运行 Markdown Prompt 与共享上下文测试并确认 GREEN**

Run:

```bash
cargo test --locked --lib work_item_output_schema_describes_single_task_not_issue_level_split
cargo test --locked --lib work_item_plan_output_schema_requires_single_session_task_sizing
cargo test --locked --lib workspace_context
```

Expected: PASS；Story/Design 既有回归测试同时通过。

- [ ] **Step 5: 提交 Task 4**

```bash
git add src/web/workspace_context/tests.rs src/web/workspace_context/prompts.rs
git commit -m "fix: align markdown work item sizing guidance"
```

---

### Task 5: 全量一致性检查与项目门禁验证

**Files:**
- Verify only: all files modified by Tasks 1–4

**Interfaces:**
- Consumes: 四个任务的最终实现。
- Produces: Prompt、Schema、Validator、Reviewer 和 Markdown 路径无 20k/19999 遗留，项目标准验证通过。

- [ ] **Step 1: 检查执行路径是否仍存在旧阈值**

Run:

```bash
rg -n '20k|19999|SINGLE_AGENT_SESSION_CONTEXT_TOKEN_LIMIT' \
  src/product/work_item_split_engine \
  src/product/work_item_split_validator \
  src/product/workspace_engine/prompts/review.rs \
  src/product/workspace_engine/tests/part_08.rs \
  src/web/workspace_context
```

Expected: 无输出。`src/product/work_item_split_validator/plan.rs` 中与 handoff 字符数有关的 `20_000` 不在本次 token 阈值范围，不得修改。

- [ ] **Step 2: 运行定向模块测试**

```bash
cargo test --locked --lib work_item_split_engine
cargo test --locked --lib work_item_split_validator
cargo test --locked --lib workspace_context
cargo test --locked --lib workspace_engine
```

Expected: 全部 PASS。

- [ ] **Step 3: 运行 Rust 标准门禁**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

Expected: 全部 exit 0；不得使用 `-j 1`。

- [ ] **Step 4: 检查 Git 差异与历史数据边界**

```bash
git diff --check
git status --short --branch
git diff --name-status origin/feat-b-0715...HEAD
```

Expected: 仅包含设计、计划、`.gitignore` 与本计划列出的源代码/测试文件；不得出现 `.aria/` 或历史数据文件修改。
