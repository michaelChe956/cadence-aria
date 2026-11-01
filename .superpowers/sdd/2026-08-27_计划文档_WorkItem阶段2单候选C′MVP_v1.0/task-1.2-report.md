# Task 1.2 完整报告：稳定 markdown/EARS grammar

## 状态

已完成 Task 1.2，采用单一原子提交（`feat(workitem): define deterministic plan markdown grammar`）。本任务严格限定为 grammar 常量、AST/diagnostic 类型、元数据和对应测试；未实现 parser/linter、未断言真实诊断行号/字段/修复示例。

## 完成内容

- 新增 `grammar.rs`，固定文档标题 `# Work Item Plan`、Work Item 标题前缀 `## Work Item WI-`、`WI-<digits>` 标识元数据、`- key: value` 与 `- TASK-001 | ...` 行形状。
- 固定 13 个结构化 section：`Identity`、`Goal`、`Non Goals`、`Dependencies`、`Inputs`、`Outputs`、`Tasks`、`Write Policy`、`Acceptance Criteria`、`Verification`、`Handoff Schema`、`Blockers`、`Traceability`；仅 `Notes`、`Rationale` 允许自由文本。
- 固定结构化 key 白名单，覆盖 field-source-matrix 中全部 markdown 来源字段；明确未知结构化 key 的 `fail_closed` 元数据。未将 `target_repository_id`、trusted command 或 compiler/runtime 字段登记为 markdown key。
- 登记既有契约允许值：compatibility 为 `require_all`/`require_any`，evidence 为 `source_diff`、`non_zero_test_execution`、`manual_check`、`handoff_field`，blocker route 为现有八项集合。
- 固定 EARS 模板 `WHEN <condition> THE SYSTEM SHALL <observable outcome>` 及关键字/分隔符元数据。
- 新增 `types.rs`：`WorkItemPlanAst`、`WorkItemPlanItemAst`、`CompilerDiagnostic`，均派生 `Debug, Clone, PartialEq, Eq`。AST 只保存 item ID、按 section 的原始行、Notes、Rationale；diagnostic 只定义公开字段形状。
- 模块入口公开 `grammar`、`types` 并 re-export 其公开项，供后续 parser/lower 使用。

## TDD 证据

### RED

先在 `tests.rs` 加入 `grammar_contract`，并接入 `grammar`/`types` 模块但尚未创建两个实现文件。首次执行：

```text
cargo test --locked --lib work_item_plan_compiler::tests::grammar_contract -- --list
```

观察到预期的模块缺失编译错误：`file not found for module grammar` 与 `file not found for module types`。同一次执行还暴露了并行 Task 3.2 尚未完成的无关编译错误（InitialPlanCompile*、事务字段等），未将其误判为本任务失败原因；已通知 controller。

### GREEN

并行 Task 3.2 恢复可编译后，先按要求列出匹配数：

```text
cargo test --locked --lib work_item_plan_compiler::tests::grammar_contract -- --list
```

输出：

```text
product::work_item_plan_compiler::tests::grammar_contract: test
1 test, 0 benchmarks
```

**已验证匹配 1 项**，N 非零。

随后执行同过滤名：

```text
cargo test --locked --lib work_item_plan_compiler::tests::grammar_contract
```

输出：`test ... grammar_contract ... ok`，`1 passed; 0 failed`。

测试断言内容只覆盖 grammar 常量、section/key 元数据、允许值集合、fail-closed/free-text 策略、EARS 元数据、compiler version、AST 字段和 derive trait；没有断言 parser/linter 真实诊断。

## 命令与结果

| 命令 | 结果 | 摘要 |
| --- | --- | --- |
| `codegraph init` | 通过 | 当前 worktree 已初始化。 |
| `cargo test --locked --lib work_item_plan_compiler::tests::grammar_contract -- --list`（RED） | 预期失败 | 实现文件尚不存在，准确报告 grammar/types 模块缺失；并行 3.2 同时处于未完成编译状态。 |
| `cargo check --locked`（等待期间） | 通过 | 并行 3.2 修复后当前树可编译；存在非本任务 dead_code 警告。 |
| `cargo test --locked --lib work_item_plan_compiler::tests::grammar_contract -- --list`（GREEN） | 通过 | **已验证匹配 1 项**。 |
| `cargo test --locked --lib work_item_plan_compiler::tests::grammar_contract` | 通过 | 1 passed，0 failed。 |
| `cargo fmt --check` | 通过 | 并行 worker 接线完成后全仓格式检查通过。 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 通过 | 并行 worker 接线完成后通过，无 warnings。 |
| `git diff --check`（本任务文件） | 通过 | 无空白错误。 |

全程未给任何 cargo 命令添加 `-j`；定向单测使用 `--locked --lib` 并先 `-- --list` 统计匹配数。

## 改动文件

- `src/product/work_item_plan_compiler/grammar.rs`：稳定 grammar、section/key 白名单、EARS、允许值与策略元数据。
- `src/product/work_item_plan_compiler/types.rs`：AST item/document 容器和公开 compiler diagnostic 类型。
- `src/product/work_item_plan_compiler/mod.rs`：模块声明及公开 re-export。
- `src/product/work_item_plan_compiler/tests.rs`：`grammar_contract` 元数据、类型字段与 derive 契约测试。
- `.superpowers/sdd/2026-08-27_计划文档_WorkItem阶段2单候选C′MVP_v1.0/task-1.2-report.md`：本报告。

## 自查

- [x] grammar 常量的精确值逐字对照 task brief。
- [x] 13 个结构化 section 与 2 个自由文本 section 顺序和值均固定。
- [x] 结构化 key 白名单覆盖矩阵中的 36 个 markdown 字段，未引入 context/runtime 第二来源。
- [x] compatibility、evidence、route 允许值来自现有 `work_item_contract/model.rs` 契约集合。
- [x] `WorkItemPlanAst` 与 `CompilerDiagnostic` 字段、derive 与 brief 一致；item AST 仅作为后续解析容器，不提前引入 parser 行号断言。
- [x] 未实现 parser/linter，不构造 fixture 诊断，不断言真实 `missing_section`/`invalid_ears` 行号、字段或修复示例。
- [x] 未触碰并行 Task 3.2 文件；暂存时只 add 本任务 5 个精确路径。

## 残余风险

- 并行 Task 3.2 在等待期间曾暂时导致编译、fmt、clippy 失败，最终接线后已恢复；其文件仍属于并行提交，不纳入本任务 commit。
- AST 的 section 原始行暂未携带 span，符合本任务“容器/元数据 only”边界；Task 2.1 再增加内部 span 和真实诊断。
- 结构化 key 白名单刻意只登记 markdown 来源；target repository/trusted command 等上下文来源由后续 lowering context 提供。

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "仅新增 compiler grammar/types 元数据、模块接线、对应契约测试和本报告；未实现 parser/linter 或扩大到 lowering/publish。"
    },
    {
      "id": "criterion-2",
      "status": "satisfied",
      "evidence": "RED 观察到 grammar/types 模块缺失；GREEN 先列出并验证匹配 1 项，再定向测试 1 passed/0 failed，并通过 fmt 与 clippy 全量门禁。"
    }
  ],
  "changedFiles": [
    "src/product/work_item_plan_compiler/grammar.rs",
    "src/product/work_item_plan_compiler/types.rs",
    "src/product/work_item_plan_compiler/mod.rs",
    "src/product/work_item_plan_compiler/tests.rs",
    ".superpowers/sdd/2026-08-27_计划文档_WorkItem阶段2单候选C′MVP_v1.0/task-1.2-report.md"
  ],
  "testsAddedOrUpdated": [
    "src/product/work_item_plan_compiler/tests.rs::grammar_contract"
  ],
  "commandsRun": [
    {
      "command": "codegraph init",
      "result": "passed",
      "summary": "当前 worktree 已初始化 CodeGraph。"
    },
    {
      "command": "cargo test --locked --lib work_item_plan_compiler::tests::grammar_contract -- --list",
      "result": "failed",
      "summary": "RED：grammar/types 实现文件不存在；另有并行 Task 3.2 暂时编译错误。"
    },
    {
      "command": "cargo check --locked",
      "result": "passed",
      "summary": "并行 Task 3.2 接线完成后树可编译。"
    },
    {
      "command": "cargo test --locked --lib work_item_plan_compiler::tests::grammar_contract -- --list",
      "result": "passed",
      "summary": "GREEN：已验证匹配 1 项。"
    },
    {
      "command": "cargo test --locked --lib work_item_plan_compiler::tests::grammar_contract",
      "result": "passed",
      "summary": "1 passed，0 failed。"
    },
    {
      "command": "cargo fmt --check",
      "result": "passed",
      "summary": "全仓格式检查通过。"
    },
    {
      "command": "cargo clippy --all-targets --all-features --locked -- -D warnings",
      "result": "passed",
      "summary": "全量 clippy 通过。"
    },
    {
      "command": "git diff --check",
      "result": "passed",
      "summary": "无空白错误。"
    }
  ],
  "validationOutput": [
    "已验证匹配 1 项。",
    "test result: ok. 1 passed; 0 failed。",
    "cargo fmt --check passed。",
    "cargo clippy --all-targets --all-features --locked -- -D warnings passed。"
  ],
  "residualRisks": [
    "AST 尚未携带 parser span；真实诊断由 Task 2.1 实现。",
    "并行 Task 3.2 文件不属于本任务改动，但共享 worktree 曾有短暂编译阻断，最终已恢复。"
  ],
  "noStagedFiles": true,
  "diffSummary": "固定 markdown/EARS grammar 元数据与允许值，新增 AST/diagnostic 类型并用 grammar_contract 锁定字段、derive 和版本契约。",
  "reviewFindings": [
    "无 blocker；本任务未实现 parser/linter，符合上一轮审查裁决的范围边界。"
  ],
  "manualNotes": "只暂存本任务四个源码文件和本报告；未触碰并行 Task 3.2 文件。"
}
```
