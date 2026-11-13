//! Strict SC failure evidence store（Plan §2.3 前 6 个冻结接口）。
//!
//! P05t 拆分：store 模块本体留在本文件，消费者行为测试拆到同目录
//! `tests/sc_failure_evidence.rs`（主）与 `tests/sc_failure_evidence_fixture.rs`（共享 fixture）。
//!
//! RED 阶段：冻结类型与 `LifecycleStore` 方法尚未实现，
//! `cargo test --locked --lib lifecycle_store::sc_failure_evidence`
//! 预期在编译期 RED（E0432 未解析导入 / E0599 方法不存在）。

#[cfg(test)]
#[path = "tests/sc_failure_evidence.rs"]
mod tests;
