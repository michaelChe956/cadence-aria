# Task 2 完成报告：input 加 `target_snapshot` 字段 + 3 创建点接线

## 状态

完成，待本报告更新 commit SHA 后交付。

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

待本报告写入后创建。

## Concerns / 残余风险

- 快照 factory 的错误目前映射为既有 `product_store_error`，未引入或改变稳定错误码；更细的 snapshot admission 稳定码属于后续 Task 6/16 范围。
- `.pi/subagents/` 及 `cadence/notes/` 下已有未跟踪文件不属于本任务，未暂存或提交。
