//! 无损 previous-source 围栏 renderer（Plan §2.4 / Task P21 / S18）。
//!
//! 本文件当前只挂载 RED 测试：真实 renderer（`ScTeachingSourceMetadata`、
//! `ScTeachingPromptError`、`render_sc_previous_source_block`）尚未落地，
//! 因此注册该模块后 `cargo test` 会以「`super::` 导入未解析」编译失败——这是预期的 RED。
//!
//! 冻结契约（实现落地后必须逐条满足，测试已按此钉住字节边界）：
//! - `ScTeachingSourceMetadata<'a> { source_ref: &'a str, sha256: &'a str, bytes: u64, origin: ScTeachingSourceOrigin }`
//! - `render_sc_previous_source_block(previous_source: &str, metadata: &ScTeachingSourceMetadata<'_>) -> Result<String, ScTeachingPromptError>`
//! - `ScTeachingPromptError { code: String, detail: String }`；来源完整性不符时 `code == "sc_failure_evidence_invalid"`
//! - 围栏长度 `max(3, source 内最长连续反引号 run + 1)`；围栏正文逐字等于 source，最多补一个行尾 LF 且不计入正文。
//! - `ScTeachingSourceOrigin` 复用既有 DTO `crate::web::workspace_ws_types::ScTeachingSourceOrigin`，不另定义。

#[cfg(test)]
#[path = "tests/sc_teaching_source_block.rs"]
mod tests;
