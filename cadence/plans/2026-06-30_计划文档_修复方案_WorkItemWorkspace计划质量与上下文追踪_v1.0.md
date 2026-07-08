# WorkItem Workspace 计划质量与上下文追踪修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 Work Item Plan 生成链路中 OpenSpec/Superpowers 约束弱、Draft 不像可执行计划、Final Compile 丢失 Draft 上下文、Child Work Item Workspace 无法指导后续 coding agent 的问题。

**Architecture:** 在 Work Item Plan 的生成 prompt、compile 投影、child workspace context 三层补齐同一条证据链。计划阶段上下文使用独立字段或独立 context summary，不复用 coding 完成后的 `handoff_summary_ref`，避免把“计划交接摘要”和“完成交接摘要”混义。OpenSpec/Superpowers 先以 runtime contract 和 traceability notes 强化，不在本次直接初始化真实 OpenSpec changes 目录。

**Tech Stack:** Rust 2024、serde JSON store、现有 WorkspaceEngine / WorkItemSplitEngine / WorkspaceContext 模块、宿主机 Cargo。遵守 `cadence/project-rules/build-test-commands.md`，禁止 `-j 1`。

---

## 背景与已确认问题

案例：`Work Item Plan #workspace_session_0003`

已确认现象：

- `workspace_session_0003` 的 `openspec_enabled=true`、`superpowers_enabled=true`，但 outline / draft provider prompt 没有 `[openspec_contract]`、`[superpowers_contract]`、`writing-plans` 运行契约。
- Draft record 中存在较强的 `implementation_context`、`handoff_summary`、`verification_plan`，但 Final Compile 后的 Work Item 只保留 title、write scopes、depends_on、verification_plan_ref。
- Child `workspace_session_0004..0008` 初始 context 没有 `implementation_context`、`handoff_summary`、`openspec_contract`、`superpowers_contract`，因此不能充分指导后续 Work Item artifact 或 coding agent。
- 当前 `openspec list --json` 报 `No OpenSpec changes directory found`，说明 enabled flag 目前主要是文本约束，不是真实 OpenSpec change/task artifact 写回链路。

非目标：

- 本次不重建完整 OpenSpec CLI 初始化和 changes 目录管理。
- 本次不改变 coding 完成后 `handoff_summary_ref` 的语义。
- 本次不做前端大改版；只在必要 DTO/context 展示点补字段。

## 文件结构

### 需要修改

- `src/product/work_item_split_engine/prompts.rs`
  给 Work Item Plan outline、outline revision、split legacy、draft prompt 注入统一 runtime contract；强化 draft 输出为 coding-agent 可执行计划。

- `src/product/work_item_split_engine/tests/part_02.rs` 或新增 `src/product/work_item_split_engine/tests/part_03.rs`
  增加 prompt contract 单测，断言 OpenSpec/Superpowers/writing-plans marker 存在。

- `src/product/models/lifecycle.rs`
  给 `LifecycleWorkItemRecord` 增加计划来源字段，建议字段：
  - `source_work_item_plan_id: Option<String>`
  - `source_outline_id: Option<String>`
  - `source_draft_id: Option<String>`
  - `planned_implementation_context: Option<String>`
  - `planned_handoff_summary: Option<String>`

- `src/product/lifecycle_store/inputs.rs`
  给 `CreateWorkItemInput` 增加同名可选字段，默认 `None`。

- `src/product/lifecycle_store/work_item.rs`
  `create_work_item` 写入新增计划来源字段。

- `src/product/workspace_engine/draft_batch/compile_support.rs`
  从 accepted draft 投影到 `LifecycleWorkItemRecord` 时写入 source plan/outline/draft 和 planned context。

- `src/product/workspace_engine/compile.rs`
  调用 `create_work_item` 时传递新增字段。

- `src/product/workspace_engine/tests/part_03/part_04.rs`
  增加 Final Compile 投影回归测试，断言最终 Work Item 带 source outline/draft 和 planned context。

- `src/web/workspace_context/entity.rs`
  `work_item_context_summary` 注入 `[work_item_plan_source]` 信息，包括 source ids、implementation context、planned handoff、verification commands。

- `src/web/workspace_context/prompts.rs`
  WorkItem / WorkItemPlan 的 context message 输出显式 `[openspec_contract]` / `[superpowers_contract]`，而不是只放一句弱文本。

- `src/web/workspace_context/tests.rs`
  增加 Work Item workspace context 回归测试，断言 child workspace system message 包含 source draft context 与 runtime contracts。

- `src/web/handlers/dto.rs` 与 `src/web/types.rs`
  若前端 Work Item detail 需要展示新增字段，同步 DTO；否则本任务可只服务后端 prompt/context，不强制前端展示。

### 不建议修改

- 不把 draft `handoff_summary` 写进 `handoff_summary_ref`，因为该字段当前由 coding workspace 完成后写入，表示真实执行后的交接摘要。
- 不直接修改 `.claude/rules/`。
- 不在本次创建 `openspec/changes`，除非后续单独立 OpenSpec change 管理需求。

---

## Task 1: Work Item Plan Prompt Runtime Contract

**Files:**
- Modify: `src/product/work_item_split_engine/prompts.rs`
- Test: `src/product/work_item_split_engine/tests/part_02.rs` 或新增 `src/product/work_item_split_engine/tests/part_03.rs`

- [ ] **Step 1: 写失败测试：outline prompt 必须包含 runtime contract**

在 work item split engine tests 中新增测试，覆盖 `build_outline_prompt` 或公开测试 helper 能拿到的 prompt：

```rust
#[test]
fn work_item_plan_outline_prompt_includes_runtime_contracts() {
    let prompt = build_sample_outline_prompt();

    assert!(prompt.contains("[openspec_contract]"));
    assert!(prompt.contains("[superpowers_contract]"));
    assert!(prompt.contains("writing-plans"));
    assert!(prompt.contains("任务拆分"));
    assert!(prompt.contains("追踪关系"));
}
```

若当前没有 `build_sample_outline_prompt()`，在测试文件内用现有 fixture 构造 `GenerateWorkItemsRequest`、`IssueRecord`、`RepositoryRecord`，调用 `build_outline_prompt(...)`。

- [ ] **Step 2: 写失败测试：draft prompt 必须包含 coding-agent 可执行计划约束**

```rust
#[test]
fn work_item_draft_prompt_requires_executable_plan_context() {
    let prompt = build_sample_work_item_draft_prompt("outline_backend");

    assert!(prompt.contains("[openspec_contract]"));
    assert!(prompt.contains("[superpowers_contract]"));
    assert!(prompt.contains("writing-plans"));
    assert!(prompt.contains("TDD"));
    assert!(prompt.contains("implementation_context"));
    assert!(prompt.contains("handoff_summary"));
    assert!(prompt.contains("后续 coding agent"));
}
```

- [ ] **Step 3: 运行定向测试，确认失败**

Run:

```bash
cargo test --locked --lib work_item_split_engine
```

Expected: 新增测试失败，提示 prompt 不包含 contract marker。

- [ ] **Step 4: 增加统一 contract helper**

在 `prompts.rs` 中新增内部函数：

```rust
fn work_item_plan_runtime_contract(role: &str) -> String {
    format!(
        "[openspec_contract]\n\
         Role: {role}\n\
         - 必须基于已确认 Story Spec 与 Design Spec 的 requirement/design trace 进行拆分。\n\
         - 每个 outline/draft 必须能追溯到 source_story_spec_ids 与 source_design_spec_ids。\n\
         - 发现 Story/Design/Work Item 之间冲突、缺失验收依据或无法确定写入边界时，必须输出 blocker 或 reviewer 可处理的风险，而不是猜测。\n\
         - 不得声称已写回 OpenSpec；当前仅生成可供 daemon 后续写回 OpenSpec tasks constraints 的结构化候选。\n\n\
         [superpowers_contract]\n\
         - 必须遵守 using-superpowers 的先读规则与 writing-plans 的计划结构要求。\n\
         - 生成的是计划和任务拆分，不执行代码修改。\n\
         - 每个 draft 必须给出后续 coding agent 可执行的目标、范围、非目标、TDD 顺序、验证命令、依赖输入、交接输出和风险。\n\
         - 结论必须能追溯到已提供的 Story/Design/Outline/Draft 证据。\n\n"
    )
}
```

- [ ] **Step 5: 注入 outline / outline revision / split / revision / draft prompt**

在以下 prompt 开头或 strict contract 前追加：

```rust
let runtime_contract = work_item_plan_runtime_contract("WorkItemPlan Outline Planner");
```

并在 `format!` 模板中放入：

```text
{runtime_contract}
```

draft prompt 使用：

```rust
let runtime_contract = work_item_plan_runtime_contract("Work Item Draft author");
```

- [ ] **Step 6: 强化 draft hard rules**

在 `build_work_item_draft_prompt` 的 `[hard_rules]` 中补充：

```text
- implementation_context 必须写给后续 coding agent，包含具体模块/文件边界、已有代码入口、TDD 起点、不要触碰的范围、验收命令顺序。
- handoff_summary 必须写给依赖它的后续 work item，列出本项完成后必须交付的类型、API、状态、测试 seam、错误码或 UI 契约。
- verification_plan.commands 必须优先包含定向快反馈命令，再包含必要的 fmt/clippy/check/test；Rust 命令必须遵守 cadence/project-rules/build-test-commands.md，禁止 -j 1。
- 若 Story/Design/Outline 证据不足以指导 coding agent，必须在 implementation_context 中显式写出阻塞点或待确认项，不得编造文件路径。
```

- [ ] **Step 7: 运行测试确认通过**

Run:

```bash
cargo test --locked --lib work_item_split_engine
```

Expected: 新增 prompt contract 测试通过。

---

## Task 2: Final Compile 保留 Draft 计划来源与上下文

**Files:**
- Modify: `src/product/models/lifecycle.rs`
- Modify: `src/product/lifecycle_store/inputs.rs`
- Modify: `src/product/lifecycle_store/work_item.rs`
- Modify: `src/product/workspace_engine/draft_batch/compile_support.rs`
- Modify: `src/product/workspace_engine/compile.rs`
- Test: `src/product/workspace_engine/tests/part_03/part_04.rs`

- [ ] **Step 1: 写失败测试：投影结果必须带 source draft context**

在 `part_03/part_04.rs` 新增：

```rust
#[test]
fn final_compile_projects_source_draft_context_into_work_items() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_draft_candidate("sess_wip_compile_source_context");
    engine.session.artifact = Some(ArtifactPayload::WorkItemPlanOutlineCandidate {
        outline_candidate: Box::new(WorkItemPlanOutlineCandidateDto {
            outline: test_work_item_plan_outline(vec![]),
            design_context_gaps: vec![],
            validator_findings: vec![],
            context_blockers: vec![],
            current_generation_round_id: Some("round_0001".to_string()),
            selected_generation_mode: Some(WorkItemGenerationModeDto::Serial),
        }),
    });

    let previous_plan = lifecycle
        .get_issue_work_item_plan("project_0001", "issue_0001", &plan_id)
        .expect("load previous plan");
    let draft_a = test_work_item_draft_record(
        &plan_id,
        "outline_a",
        "draft_a",
        WorkItemDraftStatus::Accepted,
        WorkItemGenerationMode::Serial,
        None,
    );

    let (_compiled_plan, work_items, _verification_plans) = engine
        .project_work_item_plan_drafts_for_compile(
            &previous_plan,
            &[draft_a],
            WorkItemPlanCompileProjectionContext {
                outline_order: &["outline_a".to_string()],
                outline_to_work_item_id: &BTreeMap::from([(
                    "outline_a".to_string(),
                    "work_item_a".to_string(),
                )]),
                outline_to_verification_plan_id: &BTreeMap::from([(
                    "outline_a".to_string(),
                    "verification_plan_a".to_string(),
                )]),
                repository_id: "repo_0001",
                now: "2026-06-30T00:00:00Z",
            },
        )
        .expect("project compile records");

    let work_item = work_items.first().expect("work item");
    assert_eq!(work_item.source_work_item_plan_id.as_deref(), Some(plan_id.as_str()));
    assert_eq!(work_item.source_outline_id.as_deref(), Some("outline_a"));
    assert_eq!(work_item.source_draft_id.as_deref(), Some("draft_a"));
    assert!(work_item
        .planned_implementation_context
        .as_deref()
        .expect("planned implementation context")
        .contains("实现 src/outline_a.rs"));
    assert_eq!(
        work_item.planned_handoff_summary.as_deref(),
        Some("outline_a handoff")
    );
    assert_eq!(work_item.handoff_summary_ref, None);
}
```

- [ ] **Step 2: 运行定向测试，确认失败**

Run:

```bash
cargo test --locked --lib final_compile_projects_source_draft_context_into_work_items
```

Expected: 编译失败或断言失败，因为 `LifecycleWorkItemRecord` 尚无新增字段。

- [ ] **Step 3: 扩展 `LifecycleWorkItemRecord`**

在 `src/product/models/lifecycle.rs` 的 `LifecycleWorkItemRecord` 中，在 `work_item_set_id` 附近新增：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub source_work_item_plan_id: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub source_outline_id: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub source_draft_id: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub planned_implementation_context: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub planned_handoff_summary: Option<String>,
```

- [ ] **Step 4: 扩展 `CreateWorkItemInput`**

在 `src/product/lifecycle_store/inputs.rs` 增加同名字段：

```rust
pub source_work_item_plan_id: Option<String>,
pub source_outline_id: Option<String>,
pub source_draft_id: Option<String>,
pub planned_implementation_context: Option<String>,
pub planned_handoff_summary: Option<String>,
```

在 `Default` 中设为 `None`。

- [ ] **Step 5: `create_work_item` 写入新增字段**

在 `src/product/lifecycle_store/work_item.rs` 构造 `LifecycleWorkItemRecord` 时传递：

```rust
source_work_item_plan_id: input.source_work_item_plan_id,
source_outline_id: input.source_outline_id,
source_draft_id: input.source_draft_id,
planned_implementation_context: input.planned_implementation_context,
planned_handoff_summary: input.planned_handoff_summary,
```

- [ ] **Step 6: compile projection 写入 draft 来源字段**

在 `src/product/workspace_engine/draft_batch/compile_support.rs` 的 `LifecycleWorkItemRecord` 构造中增加：

```rust
source_work_item_plan_id: Some(previous_plan.id.clone()),
source_outline_id: Some(record.outline_id.clone()),
source_draft_id: Some(record.draft_id.clone()),
planned_implementation_context: Some(candidate.implementation_context.clone()),
planned_handoff_summary: Some(candidate.handoff_summary.clone()),
```

- [ ] **Step 7: run compile 落盘时传递字段**

在 `src/product/workspace_engine/compile.rs` 调用 `CreateWorkItemInput` 时增加：

```rust
source_work_item_plan_id: work_item.source_work_item_plan_id.clone(),
source_outline_id: work_item.source_outline_id.clone(),
source_draft_id: work_item.source_draft_id.clone(),
planned_implementation_context: work_item.planned_implementation_context.clone(),
planned_handoff_summary: work_item.planned_handoff_summary.clone(),
```

- [ ] **Step 8: 运行测试确认通过**

Run:

```bash
cargo test --locked --lib final_compile_projects_source_draft_context_into_work_items
```

Expected: PASS。

---

## Task 3: Child Work Item Workspace Context 注入计划来源

**Files:**
- Modify: `src/web/workspace_context/entity.rs`
- Modify: `src/web/workspace_context/prompts.rs`
- Test: `src/web/workspace_context/tests.rs`

- [ ] **Step 1: 写失败测试：Work Item context 包含 source draft context**

在 `src/web/workspace_context/tests.rs` 新增测试，沿用现有 `work_item_workspace_context_includes_linked_design_markdown` 的 fixture 风格，创建带新增字段的 `LifecycleWorkItemRecord`，然后调用 `ensure_workspace_context_message`：

```rust
#[test]
fn work_item_workspace_context_includes_source_draft_plan_context() {
    let (_tmp, paths, lifecycle, session) =
        make_work_item_workspace_context_fixture_with_source_draft();

    let refreshed = ensure_workspace_context_message(&paths, &lifecycle, session)
        .expect("ensure workspace context");
    let content = &refreshed.messages[0].content;

    assert!(content.contains("[work_item_plan_source]"));
    assert!(content.contains("source_work_item_plan_id: issue_work_item_plan_0001"));
    assert!(content.contains("source_outline_id: outline_backend"));
    assert!(content.contains("source_draft_id: draft_backend"));
    assert!(content.contains("planned_implementation_context"));
    assert!(content.contains("实现 provider dependency core"));
    assert!(content.contains("planned_handoff_summary"));
    assert!(content.contains("交付 ProviderDependencyService"));
    assert!(content.contains("[openspec_contract]"));
    assert!(content.contains("[superpowers_contract]"));
}
```

- [ ] **Step 2: 运行定向测试，确认失败**

Run:

```bash
cargo test --locked --lib work_item_workspace_context_includes_source_draft_plan_context
```

Expected: 失败，因为 context 目前不包含新增 source draft context 和强 contract。

- [ ] **Step 3: 扩展 `work_item_context_summary`**

在 `src/web/workspace_context/entity.rs` 中，`verification_plan_summary` 后构造计划来源块：

```rust
let source_context = if work_item.source_work_item_plan_id.is_some()
    || work_item.source_outline_id.is_some()
    || work_item.source_draft_id.is_some()
    || work_item.planned_implementation_context.is_some()
    || work_item.planned_handoff_summary.is_some()
{
    format!(
        "\n[work_item_plan_source]\nsource_work_item_plan_id: {}\nsource_outline_id: {}\nsource_draft_id: {}\nplanned_implementation_context:\n{}\nplanned_handoff_summary:\n{}",
        work_item.source_work_item_plan_id.as_deref().unwrap_or("(none)"),
        work_item.source_outline_id.as_deref().unwrap_or("(none)"),
        work_item.source_draft_id.as_deref().unwrap_or("(none)"),
        work_item.planned_implementation_context.as_deref().unwrap_or("(none)"),
        work_item.planned_handoff_summary.as_deref().unwrap_or("(none)")
    )
} else {
    String::new()
};
```

在最终 `format!` 末尾追加 `{source_context}`。

- [ ] **Step 4: 扩展 workspace prompt runtime contract**

在 `src/web/workspace_context/prompts.rs` 新增 helper：

```rust
pub(super) fn runtime_contract_for(session: &WorkspaceSessionRecord) -> String {
    let openspec = if session.openspec_enabled {
        "[openspec_contract]\n- 必须保持 Story/Design/Work Item 追踪关系。\n- 不得忽略 source ids、verification commands 或 planned context。\n- 不要直接修改 OpenSpec；由 daemon 负责后续写回与 projection。"
    } else {
        "[openspec_contract]\n- OpenSpec 未启用，但仍需保持产物可追踪。"
    };
    let superpowers = if session.superpowers_enabled {
        "[superpowers_contract]\n- 必须遵守 using-superpowers。\n- Work Item / Work Item Plan 必须按 writing-plans 风格组织目标、范围、任务、验证、风险与追踪关系。\n- 生成计划，不执行代码。"
    } else {
        "[superpowers_contract]\n- Superpowers 未启用，但仍需明确假设、风险、验证与下一步。"
    };
    format!("{openspec}\n\n{superpowers}")
}
```

在 `build_workspace_context_message` 中插入：

```text
[runtime_contract]
{runtime_contract}
```

- [ ] **Step 5: 运行 context 测试确认通过**

Run:

```bash
cargo test --locked --lib work_item_workspace_context_includes_source_draft_plan_context
```

Expected: PASS。

---

## Task 4: Coding Evaluation Context 兼容新增字段

**Files:**
- Modify: `src/product/coding_evaluation_context/builder.rs`
- Modify: `src/product/coding_evaluation_context/mod.rs`
- Test: `src/product/coding_evaluation_context/tests.rs`

- [ ] **Step 1: 写测试：group context 优先读取 Work Item 显式 source 字段**

在 `src/product/coding_evaluation_context/tests.rs` 新增测试：

```rust
#[test]
fn group_context_prefers_work_item_source_fields_when_available() {
    let (_tmp, paths, attempt) = group_attempt_with_two_work_items(false);
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .update_work_item_source_context_for_test(
            PROJECT_ID,
            ISSUE_ID,
            "work_item_0001",
            "issue_work_item_plan_0001",
            "outline_backend",
            "draft_backend",
        )
        .expect("update source context");

    let pack = build_evaluation_context_pack(paths, &attempt, EvaluationContextRole::Coder)
        .expect("context pack");
    let group_context = pack.group_context.expect("group context");

    assert_eq!(group_context.source_outline_id.as_deref(), Some("outline_backend"));
    assert_eq!(group_context.source_draft_id.as_deref(), Some("draft_backend"));
}
```

实现时不要真的新增生产 API 只为测试；可以在 fixture 创建 work item 时直接填字段，或在测试 helper 中写 JSON fixture。

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib group_context_prefers_work_item_source_fields_when_available
```

Expected: 失败，因为当前只从 compile transaction 反推。

- [ ] **Step 3: 扩展 `CodingGroupContextPack`**

可选新增字段：

```rust
pub planned_implementation_context: Option<String>,
pub planned_handoff_summary: Option<String>,
```

- [ ] **Step 4: `build_group_context` 优先使用 Work Item 字段**

在 `build_group_context` 中找到当前 work item，如果存在 `source_outline_id` 与 `source_draft_id`，直接使用；否则保留现有 `resolve_group_draft_context` fallback。

建议逻辑：

```rust
let current_work_item = work_items
    .iter()
    .find(|record| record.id == current_work_item_id);
let explicit_source = current_work_item.and_then(|item| {
    item.source_outline_id
        .clone()
        .zip(item.source_draft_id.clone())
});
let (source_outline_id, source_draft_id) = if let Some((outline_id, draft_id)) = explicit_source {
    warnings.push("group_draft_context_loaded_from_work_item".to_string());
    (Some(outline_id), Some(draft_id))
} else {
    resolve_group_draft_context(lifecycle_paths, &plan, current_work_item_id, warnings)?
};
```

- [ ] **Step 5: 运行测试**

Run:

```bash
cargo test --locked --lib group_context_prefers_work_item_source_fields_when_available
cargo test --locked --lib group_context_includes_source_draft_mapping_when_compile_context_exists
```

Expected: 两个测试都 PASS，显式字段和旧 transaction fallback 都可用。

---

## Task 5: Web DTO 同步与前端最小展示

**Files:**
- Modify: `src/web/types.rs`
- Modify: `src/web/handlers/dto.rs`
- Optional Modify: `web/src/api/types/*.ts`
- Optional Modify: Work Item detail component

- [ ] **Step 1: 后端 DTO 增加字段**

在 Work Item DTO 中增加：

```rust
pub source_work_item_plan_id: Option<String>,
pub source_outline_id: Option<String>,
pub source_draft_id: Option<String>,
pub planned_implementation_context: Option<String>,
pub planned_handoff_summary: Option<String>,
```

在 `dto.rs` 映射 `LifecycleWorkItemRecord` 时传递同名字段。

- [ ] **Step 2: 若前端类型有 Work Item DTO，同步 TS 类型**

增加：

```ts
source_work_item_plan_id?: string | null;
source_outline_id?: string | null;
source_draft_id?: string | null;
planned_implementation_context?: string | null;
planned_handoff_summary?: string | null;
```

- [ ] **Step 3: 不强制新增 UI 展示**

当前用户问题主要影响 agent context，而不是人工 UI。若已有 Work Item detail JSON viewer，会自然显示字段；否则先不加可视化组件，避免扩大范围。

- [ ] **Step 4: 运行后端类型检查**

Run:

```bash
cargo check --locked
```

Expected: PASS。

若修改了前端 TS：

```bash
cd web && pnpm tsc -b
```

Expected: PASS。

---

## Task 6: 回归覆盖 Story / Design / Work Item 三模块联动规则

**Files:**
- Test: `src/web/workspace_context/tests.rs`
- Test: `src/product/workspace_engine/tests/part_03/part_04.rs`

- [ ] **Step 1: 明确影响范围**

本修复触及 `workspace_context` 的共享 system message 生成，按 `workspace-artifact-bug-triage.md` 必须确认 Story、Design、Work Item 三类产物：

- Story / Design：应新增 runtime contract，但不能破坏现有 AskUserQuestion/requestUserInput 约束。
- Work Item：新增 source draft context。
- Work Item Plan：新增 runtime contract，但 plan brief 仍正常。

- [ ] **Step 2: 跑现有 context 测试**

Run:

```bash
cargo test --locked --lib workspace_context
```

Expected: PASS。

- [ ] **Step 3: 跑 Work Item Plan compile 相关测试**

Run:

```bash
cargo test --locked --lib workspace_engine::tests::part_03
```

如果过滤路径不可用，使用更宽过滤：

```bash
cargo test --locked --lib final_compile
```

Expected: PASS。

- [ ] **Step 4: 最终后端验证**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

Expected: 全部 PASS。

---

## 验收标准

- Work Item Plan outline、outline revision、draft prompt 均包含 `[openspec_contract]`、`[superpowers_contract]` 和 `writing-plans` 约束。
- Draft prompt 明确要求 `implementation_context` 写给后续 coding agent，并包含 TDD、文件边界、验证顺序、依赖输入、交接输出。
- Final Compile 后的 Work Item JSON 保留 source plan/outline/draft id 与 planned context。
- Child Work Item workspace system message 包含 `[work_item_plan_source]`、planned implementation context、planned handoff summary、verification commands、OpenSpec/Superpowers runtime contract。
- `handoff_summary_ref` 仍只用于 coding 完成后的真实交接摘要，不被计划阶段 draft handoff 占用。
- Story / Design workspace context 现有结构化交互规则不回退。

## 风险与控制

- **序列化兼容风险**：新增字段必须带 `#[serde(default, skip_serializing_if = "Option::is_none")]`，旧数据可读取。
- **字段语义混淆风险**：计划阶段用 `planned_handoff_summary`，执行完成后仍用 `handoff_summary_ref`。
- **prompt 膨胀风险**：contract 文本保持短而硬，draft 原文只注入当前 Work Item source，不把全部 draft 批量塞进 child context。
- **OpenSpec 误导风险**：contract 必须写清“不要直接修改 OpenSpec，由 daemon 后续写回”，避免 provider 自行创建 OpenSpec artifacts。

## 建议提交拆分

1. `test: cover work item plan prompt runtime contracts`
2. `feat: add planned source context to compiled work items`
3. `feat: include source draft context in work item workspace brief`
4. `test: cover workspace context contracts across artifact types`

## 当前服务状态

调查时服务仍可用：

- Backend: `http://127.0.0.1:4317/api/health`
- Frontend: `http://127.0.0.1:5173/`
