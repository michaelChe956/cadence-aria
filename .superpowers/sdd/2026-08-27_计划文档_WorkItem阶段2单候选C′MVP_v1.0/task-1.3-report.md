# Task 1.3 完整报告：rep4 source 与 compiler diagnostic fixtures

## 完成摘要

已完成 Task 1.3 的静态 fixture/schema 范围，并保持 parser/linter 尚未实现的边界：

- 建立 `work-item-plan-rep4.md`，含恰好 backend、frontend、integration 三项的完整 markdown 源文档。
- 建立四个仅覆盖 grammar 失败的 diagnostic source fixture：缺 `Verification` section、未知结构化字段、非法 item ID、非法 EARS statement。
- 建立 `expected.json` 的五字段诊断 schema fixture，并新增静态测试验证每项字段集合、类型、非空性、fixture 一一对应与 diagnostic code 词汇表归属。
- 在 grammar 中增加初始 `DIAGNOSTIC_CODES` 单一来源词汇表；当前仅涵盖 Task 1.4 的四种 grammar 失败，Task 2.1 可追加 lowering code。

未调用 parser/linter，未将 `expected.json` 的实际 `line`、`field` 或 `repair_example` 同真实诊断输出比较；该断言明确留给 Task 1.4。

## TDD 证据

### Red

先在 `src/product/work_item_plan_compiler/tests.rs` 添加 fixture static/schema tests（`include_str!` 指向 Task 1.3 规定的六个新 fixture 文件），随后运行：

```text
cargo test --locked --lib work_item_plan_compiler::tests::fixtures -- --list
```

首次运行如预期失败，编译器报六个 fixture 路径不存在（`work-item-plan-rep4.md`、四个 diagnostic markdown 与 `expected.json`）。同一轮同时暴露了测试 helper 的 Rust lifetime 缺失，已在写入 fixture 前以最小签名修正；这不是产品行为实现。

### Green

补齐 markdown/JSON fixtures、初始 diagnostic code 词汇表和静态断言后，执行：

```text
cargo test --locked --lib work_item_plan_compiler::tests::fixtures -- --list
cargo test --locked --lib work_item_plan_compiler::tests::fixtures
```

`-- --list` 发现 3 项测试（N=3，非零）；定向测试结果为 `3 passed; 0 failed`。

新增的静态测试为：

1. `fixtures_rep4_has_complete_static_source_structure`：三项数量/标题、每项完整 section、integration `Non Goals` 精确边界、HTML 与 API 证据分离、全部 markdown 字段键覆盖、禁止 trusted commands/target repository/省略号。
2. `fixtures_diagnostic_sources_have_one_static_target_error`：每个错误 fixture 仅保持一个对应 grammar 失败，其他 section、keys、heading 或 EARS 都保持静态有效。
3. `fixtures_expected_json_has_the_diagnostic_schema`：每项严格为 `{fixture, code, line, field, repair_example}`，并验证 JSON 类型、非空性、fixture 一一对应和 `code ∈ grammar::DIAGNOSTIC_CODES`。

## 静态自查

- `work-item-plan-rep4.md` 只有 `WI-001` backend、`WI-002` frontend、`WI-003` integration 三项。
- integration 的 `Non Goals` 只写“产品实现不在范围、显式允许 `tests/integration/**`”及“上游测试不在范围”；没有禁止 integration 测试的矛盾。
- frontend 的 HTML 验收只写 `#level-selector` 与 `level-select.js`；`/api/levels` 仅在独立的脚本/集成证据中断言。
- 三项覆盖 matrix 的所有 markdown 语义 key；`trusted_commands` 和 `target_repository_id` 未写入 markdown。
- 四个 diagnostic fixtures 不含 reviewer finding；仅为缺 section、未知 field、非法 ID、非法 EARS。
- 受 Workspace 三模块联动规则影响的运行时/展示/状态链路未改动；本次仅添加 compiler 静态 fixture 与单元测试，因此 Story Spec、Design Spec、Work Item workspace 运行链路均不受影响。

## 改动文件

- `src/product/work_item_plan_compiler/grammar.rs`
- `src/product/work_item_plan_compiler/tests.rs`
- `openspec/changes/rearch-workitem-plan-pipeline/fixtures/work-item-plan-rep4.md`
- `openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/missing-verification.md`
- `openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/unknown-field.md`
- `openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/invalid-id.md`
- `openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/invalid-ears.md`
- `openspec/changes/rearch-workitem-plan-pipeline/fixtures/compiler-diagnostics/expected.json`
- `.superpowers/sdd/2026-08-27_计划文档_WorkItem阶段2单候选C′MVP_v1.0/task-1.3-report.md`

## 门禁结果

```text
cargo fmt --check
# passed

cargo clippy --all-targets --all-features --locked -- -D warnings
# passed

cargo test --locked --lib work_item_plan_compiler::tests::fixtures -- --list
# 3 tests, 0 benchmarks

cargo test --locked --lib work_item_plan_compiler::tests::fixtures
# 3 passed; 0 failed
```

## 剩余风险

- `expected.json` 的行号、字段与修复示例仅是 fixture schema 数据；尚未存在 parser/linter，故没有并且不应有真实 compiler-output 比对。Task 1.4 必须复用 `grammar::DIAGNOSTIC_CODES` 并接管实际诊断断言。
- fixture 所用命令字符串是作者 markdown 语义，不是 trusted command catalog；不会进入 session-confirmed context。

## 并行工作区门禁说明

本任务自己的新鲜证据已在工作区被并行改动污染前取得：定向 fixture tests 为 3/3 通过，`cargo fmt --check` 与 `cargo clippy --all-targets --all-features --locked -- -D warnings` 均通过。最终复跑全量 clippy 时，编译被未暂存且不属于本任务的 `src/product/workspace_engine/tests/part_03/part_15.rs:73` 阻断，错误为“functions used as tests can not have any arguments”。该文件不在本提交的 staged paths 内；依照并行隔离约束未修改。经 controller 明确裁决，保留自身已绿证据并提交，待 3.3 提交后在合并树复跑全量门禁。
