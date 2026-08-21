
## 并发缺陷修复：终端输出共享 budget 竞争（terminal.rs）

**问题**：`read_terminal_stream` 中 stdout/stderr 两个 reader task 用 `Ordering::Relaxed` 的 `load`/`store` 共享输出 byte budget（`AtomicUsize`）。跨流并发时"读取 used + 写回 used+retained"非原子，可能丢失 budget 更新，导致总输出超过 `MAX_TERMINAL_OUTPUT_BYTES`（1048576）上限或截断判定不准。

**修复**：把"读取 + 扣减"改为 CAS 循环（`compare_exchange`，success `AcqRel` / failure `Acquire`）：先 load 当前 used，计算本次 take，再原子地把 `used -> used + take` 提交；若其他流在此期间更新了 budget 则用返回的最新值重试。`truncated` 标记同步升级为 `Release` store。语义保持不变：恰好 1048576 字节完整返回不截断；超出则只保留前 1048576 字节且 `truncated` 只置 true（`store(true)` 幂等，多流并发标记亦无碍）。

**新增测试**：`concurrent_streams_share_budget_without_overdraw` —— stdout/stderr 各并发输出 1048576 字节，断言总保留字节恰好等于上限且 truncated 置位，锁定无竞态。

**验证**：
- `cargo test --locked --lib client_services::terminal` — 12 passed
- `cargo test --locked --lib output_is_capped` — 1 passed
- `cargo fmt --check` / `cargo check --locked` — 通过
