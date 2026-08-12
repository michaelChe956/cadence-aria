# Task 2 完成报告：input 加 `target_snapshot` 字段 + 3 创建点接线

## 状态

完成，commit：`a6a6ae0d feat(coding-attempt): 创建时持久化目标快照`。

## 实现摘要

- 在 `CreateCodingAttemptInput` 与 `CreateGroupCodingAttemptInput` 增加 `target_snapshot: Option<AttemptTargetSnapshot>`。
- 三个持久化创建点均转发 input 中的快照：单 work-item attempt、直接 group attempt、group 初始化 journal attempt。
- 单 work-item / group Web handler 在 `RepositoryRouting::Logical` 分支调用 Task 1 的 `build_attempt_target_snapshot`，并传入创建 input；`Legacy` 分支明确传入 `None`。
- group 初始化重放匹配条件纳入 target snapshot，避免使用不同冻结快照重放同一 journal。
- 所有既有 input 构造点显式补 `target_snapshot: None`，维持旧单仓/测试夹具语义。
- 新增 Web 集成测试覆盖：Logical 单 attempt、Logical group attempt 快照落盘（目标 logical id 与非空 revision），以及 Legacy 单 attempt 仍为 `None`。

## 测试与验证

| 命令 | 结果 | 摘要 |
|---|---|---|
| `cargo test --locked --lib target_snapshot` | 通过 | 3/3：Task 1 factory 成功、inactive、policy missing。 |
| `cargo test --locked --test it_web target_snapshot -- --nocapture` | 通过 | 3/3：Logical 单/group attempt 快照落盘；Legacy 保持 None。 |
| `cargo test --locked --lib coding_attempt_store` | 通过 | 73/73。 |
| `cargo test --locked --test it_product product_coding_attempt_store` | 通过 | 28/28。 |
| `cargo test --locked --test it_web` | 通过 | 334 passed、12 ignored、0 failed。 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 | 无 warning。 |
| `cargo fmt --check` | 通过 | 无格式差异。 |
| `git diff --check` | 通过 | 无空白错误。 |

全程未使用 `-j 1`。

## 修改范围

### 生产代码

- `src/product/coding_attempt_store/inputs.rs`
- `src/product/coding_attempt_store/attempt.rs`
- `src/product/coding_attempt_store/group.rs`
- `src/product/coding_attempt_store/group_initialization.rs`
- `src/web/handlers/coding.rs`
- `src/web/handlers/coding/group.rs`

### 测试与兼容构造点

- `tests/it_web/web_coding_attempt_api/part_17.rs`：新增 3 个端到端回归。
- 其余 54 个 Rust 测试/fixture 构造点：显式补 `target_snapshot: None` 以适配新增必填 input 字段，未改变现有行为。

## Commit

`a6a6ae0d feat(coding-attempt): 创建时持久化目标快照`

## Fix round 1（2026-08-11）：修复 Logical group 初始化重放快照漂移

### 审查发现与根因

- **Important**：`journal_matches_request` 对 `attempt.target_snapshot` 保持全等校验是必要的完整性保护；但 group 创建 handler 在每一次请求均调用 snapshot factory。未完成 journal 重放时，factory 新生成的 `captured_at` 与 journal 中冻结值不同，导致 `prepare_group_initialization` 返回 `initialization journal identity differs`。

### 修复

- `create_group_coding_attempt` 在存在**未完成** group 初始化 journal 时，复用 `journal.attempt.target_snapshot` 构造重放 input；只有无 journal 或 journal 已 `Completed` 的新建/幂等读取路径才调用 factory。
- 未删除或放宽 `journal_matches_request` 的 `target_snapshot` 全等条件，持久化 attempt 与 journal 的快照一致性仍受保护。
- 新增 `PreparedBeforeAttemptPersisted` 测试检查点，以覆盖 attempt 尚未物化、但 journal 已落盘的真实重放边界。
- 新增 Logical group 回归：在 `PreparedBeforeAttemptPersisted`（Prepared）与 `PersistedBeforeBind`（AttemptPersisted）中断后重试均成功；断言 attempt 与完成 journal 的完整快照严格等于首次 journal 值，因而 `captured_at` 保持不变。

### 验证

| 命令 | 结果 | 摘要 |
|---|---|---|
| `cargo test --locked --test it_web logical_group_initialization_replay_reuses_journal_target_snapshot -- --nocapture` | 通过 | 1/1；两个 journal 阶段均验证重放成功及完整快照不变。 |
| `cargo test --locked --lib group_initialization` | 通过 | 命令过滤无匹配单测（0 executed），lib 编译及测试二进制通过。 |
| `cargo test --locked --test it_web` | 通过 | 335 passed、12 ignored、0 failed。 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 通过 | 无 warning。 |
| `cargo fmt --check` | 通过 | 无格式差异。 |
| `git diff --check` | 通过 | 无空白错误。 |

全程未使用 `-j 1`。Legacy 路径未修改，仍传递 `None`。


- 快照 factory 的错误目前映射为既有 `product_store_error`，未引入或改变稳定错误码；更细的 snapshot admission 稳定码属于后续 Task 6/16 范围。
- `.pi/subagents/` 及 `cadence/notes/` 下已有未跟踪文件不属于本任务，未暂存或提交。
