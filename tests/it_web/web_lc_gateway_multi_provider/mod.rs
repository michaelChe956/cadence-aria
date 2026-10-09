//! Task 10a:四家 LC 五阶段真实 fresh/resume 矩阵(test-only 基建);
//! Task 11a:越界写、D4 与失败零 spawn 真实验收(失败矩阵结构面 +
//! 四家 `#[ignore]` 现场测试)。
//!
//! 契约来源:实施计划 Task 10/11(Files/Interfaces/Step1-5/每格证据形态)
//! 与 spec delta REQ-LCG-07。harness 复用 `web_lc_operations_api` 的 HTTP
//! 形态与真实 git fixture 建法;不复用 Noop/Fake 驱动当真实证据。
pub(crate) mod boundary_matrix;
pub(crate) mod failure_matrix;
pub(crate) mod direct_comparison;
pub(crate) mod policy_dependency;
pub(crate) mod harness;
pub(crate) mod live_matrix;
pub(crate) mod snapshot;
