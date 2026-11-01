//! 真机 provider e2e 与 unix 进程组用例的**环境门**（仅测试构建）。
//!
//! 这些用例真的起 provider CLI（pi / claude / codex）等它响应，或依赖 unix
//! 进程组/shell exec 语义。CI runner 上这两类前提都**不具备**：
//!
//! - provider CLI 进程能起，但没有交互式 TTY / 网络凭证 → **永不响应**
//!   （2026-10-31 实测 `pi` 侧 `ProviderTimeout … HardTimeoutKilled`，
//!   硬超时 60s）；
//! - `sh -c '…'` 在 CI 上 fork 出子进程（组内仍有活进程），而本机多为直接
//!   exec（组内为空）→ 「清掉残留后应静止」这类断言随 shell 行为而变。
//!
//! 这两类都**不是产品缺陷**，是环境不具备。因此：
//!
//! - **只在 `CI=true`（GitHub Actions 等）时跳过**，本地照常执行（保留真机
//!   覆盖，不让 CI 的存在把本地覆盖也砍掉）；
//! - 跳过时**显式打印一行**，CI 日志里一眼可见「跳过了什么」——不是静默
//!   通过、不是假装绿灯。

/// CI 环境（`CI=true` / `CI=1`）不具备交互式 provider CLI 与本地 shell
/// exec 语义的前提。
pub(crate) fn ci_lacks_real_provider_env() -> bool {
    matches!(std::env::var("CI").as_deref(), Ok("true") | Ok("1"))
}

/// 用例开头的环境门：返回 `true` 表示「本次跳过」（原因已打印）。
///
/// 调用方写法：
///
/// ```ignore
/// if crate::cross_cutting::real_provider_e2e::skip_when_ci_lacks_provider(
///     "anchor_t35_provider_prepare_and_audit_share_full_fingerprint",
/// ) {
///     return;
/// }
/// ```
pub(crate) fn skip_when_ci_lacks_provider(test_name: &str) -> bool {
    if !ci_lacks_real_provider_env() {
        return false;
    }
    eprintln!(
        "[skip] {test_name}: CI 环境无交互式 provider CLI / 本地 shell exec 语义\
         （进程可起但永不响应，实测 60s 硬超时；进程组残留行为亦不同）。\
         该用例属真机 e2e，仅在本地执行。"
    );
    true
}
