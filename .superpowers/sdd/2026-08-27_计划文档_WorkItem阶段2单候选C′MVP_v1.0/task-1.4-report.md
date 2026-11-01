# Task 1.4 完整报告：grammar source linter 失败关闭

## 完成摘要

实现 `work_item_plan_compiler` 的最小 source linter/parser，保持范围仅在 grammar 输入层：

- 新增公开接口 `lint_work_item_plan_source(source: &str) -> Vec<CompilerDiagnostic>` 与 `parse_work_item_plan(source: &str) -> Result<WorkItemPlanAst, Vec<CompilerDiagnostic>>`。
- 对结构化区域实施失败关闭：未知 section/key、缺必需 section/field、非法/重复 WI/TASK/AC/CHECK ID、非法/不存在/自引用/循环依赖，以及非法 EARS statement 都产生诊断。
- 诊断严格引用 `grammar::DIAGNOSTIC_CODES` 的四个既有常量，无诊断码字面量；输出按 `(line, field, code)` 稳定排序，且所有诊断均有 1-based 行号、非空 field/message 与一个 `repair_example`。
- 逐字段实测 Task 1.3 的四个 diagnostic fixture；`code`、`line`、`field`、`repair_example` 均与 `expected.json` 一致，每 fixture 恰一条诊断。
- `Notes` / `Rationale` 以自由文本保存，Unicode、冒号和 Markdown 表格不作为结构化 token 解析。
- 未新增 heading 与 identity 的一致性校验，避免让 `invalid-id.md` 产生超出 fixture 契约的第二条诊断。
- 未修改 `workspace_engine::validate_workspace_artifact_constraints`，保留其既有行为。

## TDD 证据

### Red

先在 `src/product/work_item_plan_compiler/tests.rs` 写入 source linter tests，并导入尚未存在的两个公开接口。首次执行：

```text
cargo test --locked --lib work_item_plan_compiler::tests::source_linter -- --list
```

如预期失败，编译错误为：

```text
unresolved imports `super::lint_work_item_plan_source`, `super::parse_work_item_plan`
```

失败直接证明生产接口尚未实现。

### Green

新增 `parse.rs` 并在模块根 re-export 接口后，首先列举并记录匹配数：

```text
cargo test --locked --lib work_item_plan_compiler::tests::source_linter -- --list
# 已验证匹配 5 项
```

随后执行同一过滤测试：

```text
cargo test --locked --lib work_item_plan_compiler::tests::source_linter
# 5 passed; 0 failed
```

新增 tests 覆盖：

1. 四个 diagnostic fixture 与 `expected.json` 的 `code/line/field/repair_example` 逐字段一致。
2. 未知结构化 heading/key，缺 section/field。
3. 重复 WI/TASK/AC/CHECK ID；非法、缺失、自引用与循环 dependency。
4. 非法 EARS；Notes/Rationale 的任意 Unicode、冒号、表格自由文本允许；有效 rep4 可构造三 item AST。
5. 多错误输出的稳定排序与每条诊断的完整字段形状。

## 诊断范围说明

本 Task 的初始 vocabulary 只有 `missing_section`、`unknown_structured_key`、`invalid_work_item_id` 与 `invalid_ears`。为不越过 Task 1.4 既有 grammar 常量，重复 identifier 与依赖关系错误归入已有 `invalid_work_item_id`，unknown heading 归入已有 `unknown_structured_key`；后续 lowering-specific codes 仍留给 Task 2.1 追加。

## 改动文件

- `src/product/work_item_plan_compiler/parse.rs`：新增 fail-closed linter、最小 AST parser、诊断构造与稳定排序。
- `src/product/work_item_plan_compiler/mod.rs`：声明 parser 模块并公开两个 Task 1.4 接口。
- `src/product/work_item_plan_compiler/tests.rs`：新增 5 个 source linter regression tests。
- `.superpowers/sdd/2026-08-27_计划文档_WorkItem阶段2单候选C′MVP_v1.0/task-1.4-report.md`：本报告。

## 验证命令与结果

```text
cargo fmt --check
# passed

cargo clippy --all-targets --all-features --locked -- -D warnings
# passed

cargo test --locked --lib work_item_plan_compiler::tests::source_linter -- --list
# 已验证匹配 5 项

cargo test --locked --lib work_item_plan_compiler::tests::source_linter
# 5 passed; 0 failed

cargo test --locked --lib work_item_plan_compiler
# 12 passed; 0 failed

cargo test --locked --lib workspace_engine::artifact_constraints -- --list
# 0 tests, 0 benchmarks；按失败关闭规则未作为通过证据

cargo test --locked --lib artifact_constraint -- --list
# 已验证匹配 5 项（实际承载 artifact_constraints 私有模块回归的测试函数前缀）

cargo test --locked --lib artifact_constraint
# 5 passed; 0 failed
```

brief 指定的 `workspace_engine::artifact_constraints` 是私有实现模块路径，而测试实际注册在扁平的 `workspace_engine::tests` 中，故该精确过滤名返回 0 项。未将其误报为通过；已改用 nonzero 的 `artifact_constraint` 测试前缀，验证既有 artifact constraints 无回归。

## 剩余风险

- 未运行全量 `cargo test --locked`；本任务硬性门禁所要求的 fmt、全量 clippy、定向 compiler 与 artifact-constraint tests 均已通过。全量测试应由集成/控制器在并行改动稳定后再执行。
- parser 当前仅实现 Task 1.4 的 grammar preflight 与最小 AST；typed IR lowering、contract-level值域校验、source hash/version 与 publish freshness 仍明确留给后续任务。

## 并行工作区说明

开始时工作区已含 `workspace_engine` 的并行未暂存文件；本任务未修改或暂存它们。此后状态显示只有本任务的 compiler 三文件及本报告发生变化。提交前使用精确路径 `git add`，提交后检查 cached diff 为空和 `git status --short`，确保不携带并行 worker 改动。
