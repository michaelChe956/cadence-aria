//! Task 10a:四家 LC 五阶段真实 fresh/resume 矩阵(test-only 基建)。
//!
//! 契约来源:实施计划 Task 10(Files/Interfaces/Step1-5/每格证据形态)
//! 与 spec delta REQ-LCG-07。harness 复用 `web_lc_operations_api` 的 HTTP
//! 形态与真实 git fixture 建法;不复用 Noop/Fake 驱动当真实证据。
pub(crate) mod harness;
pub(crate) mod snapshot;
pub(crate) mod live_matrix;
