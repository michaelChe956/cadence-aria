# Task 3.2 实施报告：唯一 InitialPlanCompileInput 与 prepare/execute 核心

## 范围与裁决

本次按 supervisor 裁决仅完成 **legacy 侧抽取**：

- 新增契约精确的 13 字段 `InitialPlanCompileInput`。
- 新增纯 `prepare_initial_plan_compile`、writer-only `execute_initial_plan_compile` 与 `CompileStores`。
- 将 legacy store/lifecycle 读取、`next_compile_id()` 和 `Utc::now()` 保留在 adapter 外层。
- 将 initial publication 拆为输入式 `prepare_initial_plan_publication` 与执行 publish。
- 为 `WorkItemPlanCompileTransaction` 增加七个可选 durable 字段及 legacy 默认语义。

按裁决，未实施 Task 2.5 尚未落地的 reservation/provenance CAS 和 SingleCandidate crash-boundary 时序；这些留给 Task 3.4/5.4。

## TDD 证据

### RED

先在 `src/product/workspace_engine/tests/part_03/part_15.rs` 增加无 store 的纯 prepare 确定性测试、legacy 投影对照与 durable context fail-closed 测试。执行：

```text
cargo test --locked --lib work_item_plan_initial_compile -- --list
cargo test --locked --lib work_item_plan_initial_compile
```

实现前编译按预期失败，错误包含：

```text
cannot find struct `InitialPlanCompileDurableContext`
cannot find type `InitialPlanCompileInput`
cannot find function `prepare_initial_plan_compile`
no method named `effective_flow_kind`
```

这证明新测试先于实现建立了失败基线。

### GREEN

完成最小实现后，定向过滤器列出 **11 项**（满足 brief 的 `>=11`），并且：

```text
cargo test --locked --lib work_item_plan_initial_compile
11 passed; 0 failed
```

新增/更新覆盖包括：

- 同一 `InitialPlanCompileInput` 两次纯 prepare 完全相等。
- legacy projection 与纯 prepare 的 projection/validator input 相等。
- pure test 不传 store handle。
- legacy JSON 缺少七字段时全部反序列化为 `None` 且 `effective_flow_kind()==Legacy`。
- SingleCandidate durable context 缺少任一 ref/hash 时 fail-closed。
- SingleCandidate transaction roundtrip 保留七字段。
- 3.1 transaction journal parity、恢复与 transient `updated_at` characterization 未改断言且保持通过。

## 实现摘要

1. `WorkItemPlanCompileTransaction` 的七个 durable 字段均为 `Option`，逐字段使用 `#[serde(default, skip_serializing_if = "Option::is_none")]`；缺失 `flow_kind` 按 Legacy 解释。
2. `prepare_initial_plan_compile` 只消费 adapter 注入值，构建 projection、validator input、initial transaction 和确定性 publication journal；不读 store、不调用时钟。
3. `execute_initial_plan_compile` 只持有 `CompileStores` writer，按 preparing → validating → committing 写 transaction 后执行 publication。
4. `compile_initial_plan_revision` 仅保留 legacy wrapper 的 lifecycle/latest outline/matching transaction 读取；其 publication 创建改为调用同一输入式 preparation。
5. publication ID allocation 与 journal construction 现可使用显式已分配 IDs 的纯 helper，便于未来 IR adapter 汇入同一路径。

## 改动文件

- `src/product/models/outline.rs`
- `src/product/work_item_revision_store/initial_publication.rs`
- `src/product/work_item_revision_store/mod.rs`
- `src/product/workspace_engine/compile.rs`
- `src/product/workspace_engine/draft_batch/compile_support.rs`
- `src/product/workspace_engine/mod.rs`
- `src/product/workspace_engine/plan_projection.rs`
- `src/product/workspace_engine/tests/part_03/part_09.rs`
- `src/product/workspace_engine/tests/part_03/part_15.rs`
- `src/product/coding_evaluation_context/tests/group_context.rs`
- `tests/it_product/product_work_item_plan_store/part_01.rs`
- `tests/it_product/product_work_item_plan_store/part_02.rs`

后四类 fixture/测试改动仅补齐 durable schema 新字段和 JSON 兼容覆盖。

## 验证记录

| 命令 | 结果 |
|---|---|
| `cargo test --locked --lib work_item_plan_initial_compile -- --list` | 通过，已验证匹配 11 项 |
| `cargo test --locked --lib work_item_plan_initial_compile` | 通过，11/11 |
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 通过 |
| `cargo test --locked --lib workspace_engine` | 通过，975 passed、1 ignored |
| `cargo test --locked --test it_product` | 通过，210/210 |
| `git diff --check` | 通过 |

## 自查

- `InitialPlanCompileInput` 严格为需求给出的 13 字段，未增删字段。
- 纯 prepare/publication 函数不接受 store，亦未直接调用 `Utc::now()`。
- `CompileStores` 只承载执行阶段 writer；legacy adapter 独占读取与 ID/时钟注入。
- 首个 transaction 写入来自 pure prepare 的完整 durable context；legacy 上下文为七字段全 `None`。
- 3.1 parity 断言未修改，定向基线继续全绿。
- 无 `-j` 参数；所有定向单测均使用 `cargo test --locked --lib`。

## 剩余风险

- 依 supervisor 裁决，SingleCandidate 的 reservation-first CAS、provenance snapshot 和四个 reservation crash boundary 尚未实现或测试；需由后续 Task 3.4/5.4 在 Task 2.5 API 已落地后完成。
- 本次只覆盖 legacy adapter；IR adapter 未实现，符合本任务边界。
