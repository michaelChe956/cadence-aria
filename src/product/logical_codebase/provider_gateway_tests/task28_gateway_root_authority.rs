//! Task 2.8（映射 tasks.md 2.2；REQ-ENV-01/10/11）：gateway 复验拆除
//! 「canonical cwd == canonical target」错误等式 + 双工厂 root assertion。
//!
//! 三个验收维度：
//! 1. 双工厂 root projection（gateway 工厂 authority_root=manifest
//!    provider_context_root、登记工厂 record.aggregate_root、aggregate 生产
//!    driver 的 root recipe receipt canonical root、envelope 冻结 cwd）四路
//!    canonical 相等才放行——不一致 zero spawn（含 gateway 组装即失败）。
//!    fixture 带空格目录与 symlink，断言 canonical 相等而非字面相等。
//! 2. cwd authority：越出 authority 的 cwd 在 validate 即拒绝；symlink 逃逸
//!    在 spawn 前 canonical 复验拒绝；Git identity 漂移继续 zero spawn；
//!    cwd≠target 的合法分离形态（root cwd + member target）放行。
//! 3. 写证据不随 cwd 扩大：coding 的唯一 writable root 仍是 target
//!    worktree，root cwd 不得成为写证据（evidence gate 只作证据，不猜 OS
//!    sandbox）。
include!("task28_root_authority.inc.rs");
