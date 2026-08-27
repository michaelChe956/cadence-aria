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

## 回归修复

### 根因

`dbe6afe8` 将 `prepare_initial_plan_publication`（其中包含 canonical dependency/handoff 校验）放入 `prepare_initial_plan_compile`。这使 publication 的 `unconsumed_required_handoff` 在 legacy strict validator 的 transaction 状态机之前返回，尚未写入 `Preparing`/`Validating`/`Committing` transaction；调用方因而把它误判为没有 compile transaction 的失败，batch accept-all 从既有的 final compile → human confirm 路径回退到了 batch confirm。

这不是 handoff 消费关系、accepted draft 集合、`depends_on` 派生或 validator 规则本身丢失：最小复现中三份 canonical draft 仍是 `wi_backend_session → wi_frontend_expiry → wi_integration_session`，并且 integration draft 的 output handoff 本来就没有下游 consumer。对 8b4ca62f 逐项打印证明，其旧 canonical validator 同样产生该 finding，但发生在 Committing transaction 已写入之后，后续被恢复/完成逻辑按既有语义处理。

### 证据

- 在隔离 worktree 中：`8b4ca62f` 对 `web_work_item_plan_batch::batch_confirm_accept_all_marks_all_valid_drafts_accepted` 通过，切到 `dbe6afe8` 后以同一命令稳定失败。
- 失败信息精确为 `unconsumed_required_handoff (contract.wi_integration_session.output)`；旧版打印的 contracts 与 validator report 证明该 finding 已存在，故排除「抽取丢失 consumer input_contract」假设。
- 比对 `8b4ca62f..dbe6afe8`：变化点是 `prepare_initial_plan_compile` 提前调用 `prepare_initial_plan_publication`，而非 `project_work_item_plan_drafts_for_compile` 的 `depends_on`、accepted drafts 或 validator 输入内容变更。

### 修复

`PreparedInitialPlanCompile` 现在携带确定性的 `InitialPlanPublicationInput`，而不是已经执行验证的 journal。`execute_initial_plan_compile` 先保持原有 transaction journal 顺序写入 Preparing → Validating → Committing；只有在 Committing 写入后才调用同一纯 `prepare_initial_plan_publication` 并 publish。

此修复没有放宽 canonical validator，也没有修改 accepted drafts、handoff、`depends_on` 或 dependency graph；只恢复了 legacy 错误处理所依赖的调用时序。纯 prepare 仍不访问 store/时钟，publication input 和 IDs 仍完全由外层注入。

### 验证

| 命令 | 结果 |
|---|---|
| `cargo test --locked --test it_web web_work_item_plan_batch::batch_confirm_accept_all_marks_all_valid_drafts_accepted -- --exact` | 通过，1/1 |
| `cargo test --locked --lib work_item_plan_initial_compile -- --list` | 通过，已验证 13 项 |
| `cargo test --locked --lib work_item_plan_initial_compile` | 通过，13/13（3.1/3.2 基线） |
| `cargo test --locked --lib compile_recovery_continue` | 通过，4/4（3.3 recovery parity） |
| `cargo test --locked --lib initial_plan_publication` | 通过，5/5（3.3 publication recovery parity） |
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 通过 |
| `cargo test --locked`（隔离 worktree，当前 HEAD + 本 fix 精确 patch） | 通过：2791 单测（2 ignored）、148 it_core、210 it_product、400 it_web（12 ignored）、44 logical-codebase、doc tests 全绿 |

共享 worktree 中直接运行的全量 `cargo test --locked` 受并行 4.3 未提交变更影响，出现其 `review/routing.rs` 超过 1200 行和 10 个无关 WebSocket interrupt/choice 失败；本 fix 的隔离 worktree 已以不含这些未提交文件的同一 HEAD + 精确 patch 通过全量。
