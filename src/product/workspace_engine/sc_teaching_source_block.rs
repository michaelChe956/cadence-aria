//! 无损 previous-source 围栏 renderer（Plan §2.4 / Task P21 / S18）。
//!
//! 上一轮失败的源码是**待修数据**而不是新指令：本模块只负责把它逐字放进一个
//! 长度安全的围栏块，并把来源元数据一并呈现；不 trim、不转义、不截断，也不在
//! 这里施加最终提示词预算（预算裁决属于 P22 的 facade，超限时仍需保留全文供审计）。
//!
//! 来源完整性由调用方给出的元数据与正文自身复算结果比对：sha256 或字节数不一致
//! 即返回稳定错误码 `sc_failure_evidence_invalid`。本模块不做任何 I/O。

use sha2::Digest as _;

use crate::web::workspace_ws_types::ScTeachingSourceOrigin;

/// 教学提示引用上一轮失败源码时的元数据（Plan §2.4）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScTeachingSourceMetadata<'a> {
    /// 证据库中的来源 locator（只读引用，本模块不做 I/O）。
    pub source_ref: &'a str,
    /// 来源正文 UTF-8 字节的 sha256 小写 hex。
    pub sha256: &'a str,
    /// 来源正文的 UTF-8 字节数。
    pub bytes: u64,
    /// 来源类别（复用 P04 已冻结的 DTO）。
    pub origin: ScTeachingSourceOrigin,
}

/// 教学提示构造失败（Plan §2.4）：只含稳定 code 与可读 detail。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScTeachingPromptError {
    /// 稳定错误码；来源完整性不符为 `sc_failure_evidence_invalid`。
    pub code: String,
    /// 面向人的失败原因。
    pub detail: String,
}

/// 来源完整性不符时的稳定错误码。
const SC_FAILURE_EVIDENCE_INVALID: &str = "sc_failure_evidence_invalid";

/// 正文 UTF-8 字节的 sha256 小写 hex（与证据库同一口径）。
fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

/// 围栏长度：`max(3, 正文内最长连续反引号 run + 1)`，保证正文自带的反引号行无法闭合围栏。
fn fence_length(source: &str) -> usize {
    let mut longest = 0usize;
    let mut current = 0usize;
    for ch in source.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    3usize.max(longest + 1)
}

/// origin 的 wire 名，与既有 DTO 的 `snake_case` 序列化名一致。
fn origin_wire_name(origin: &ScTeachingSourceOrigin) -> &'static str {
    match origin {
        ScTeachingSourceOrigin::CompileSplitOutput => "compile_split_output",
        ScTeachingSourceOrigin::IrPreflightSplitOutput => "ir_preflight_split_output",
        ScTeachingSourceOrigin::StructureRevision => "structure_revision",
    }
}

/// 把上一轮失败源码渲染成无损围栏块。
///
/// 成功时返回的字符串里，围栏之间的正文按 UTF-8 字节逐字等于 `previous_source`
/// （围栏行与为闭合围栏补的那一个行尾 LF 都不属于正文）；元数据的 sha256 或字节数
/// 与正文复算结果不一致时返回 `sc_failure_evidence_invalid`，不返回任何半成品文本。
pub fn render_sc_previous_source_block(
    previous_source: &str,
    metadata: &ScTeachingSourceMetadata<'_>,
) -> Result<String, ScTeachingPromptError> {
    let source_bytes = previous_source.as_bytes();
    let actual_bytes = source_bytes.len() as u64;
    if actual_bytes != metadata.bytes {
        return Err(ScTeachingPromptError {
            code: SC_FAILURE_EVIDENCE_INVALID.to_string(),
            detail: format!(
                "source bytes mismatch: metadata={} actual={} source_ref={}",
                metadata.bytes, actual_bytes, metadata.source_ref
            ),
        });
    }
    let actual_sha256 = sha256_hex(source_bytes);
    if actual_sha256 != metadata.sha256 {
        return Err(ScTeachingPromptError {
            code: SC_FAILURE_EVIDENCE_INVALID.to_string(),
            detail: format!(
                "source sha256 mismatch: metadata={} actual={} source_ref={}",
                metadata.sha256, actual_sha256, metadata.source_ref
            ),
        });
    }
    let fence = "`".repeat(fence_length(previous_source));
    Ok(format!(
        "上一轮失败源码（origin={}, source_ref={}, sha256={}, bytes={}）：\n{}\n{}\n{}",
        origin_wire_name(&metadata.origin),
        metadata.source_ref,
        metadata.sha256,
        metadata.bytes,
        fence,
        previous_source,
        fence
    ))
}

/// 本模块自带的测试挂载（Plan P21）：测试文件与 renderer 同目录树下，由 renderer
/// 以 `#[path]` 挂载，避免在 `engine/tests.rs` 里再登记一次。
#[cfg(test)]
#[path = "tests/sc_teaching_source_block.rs"]
mod tests;
