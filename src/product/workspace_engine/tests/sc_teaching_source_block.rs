//! RED tests — 无损 previous-source 围栏 renderer（Plan §2.4 / Task P21 / S18）。
//! 由 renderer 自身的 `#[path = "tests/sc_teaching_source_block.rs"]` test module 挂载，
//! 故用 `super::` 消费 renderer 公开 API。
//!
//! 消费侧约定（实现必须满足；测试只钉字节边界与围栏结构，不钉任何包装措辞）：
//! 1. 围栏行是纯反引号行，长度 `max(3, source 内最长连续反引号 run + 1)`；
//! 2. 渲染结果中恰有两行这样的围栏行：前者开围栏、后者闭围栏；
//! 3. 两围栏之间的原始字节逐字等于 source；实现可补恰好一个行尾 LF，该补行不计入正文；
//! 4. source 在渲染结果中逐字出现且只出现一次（不回声、不复制、不截断、不 trim、不转义）；
//! 5. metadata 的 sha256/bytes 与 source 不符时返回稳定错误码 `sc_failure_evidence_invalid`。

use sha2::Digest as _;

use super::{ScTeachingSourceMetadata, render_sc_previous_source_block};
use crate::web::workspace_ws_types::ScTeachingSourceOrigin;

const INVALID_EVIDENCE_CODE: &str = "sc_failure_evidence_invalid";
const SOURCE_REF: &str = "evidence://sc/split-output/round-7";

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

fn longest_backtick_run(source: &str) -> usize {
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
    longest
}

fn expected_fence_len(source: &str) -> usize {
    3usize.max(longest_backtick_run(source) + 1)
}

/// 消费侧独立提取：按围栏长度在渲染文本里定位开/闭围栏行，返回两围栏之间的原始字节。
fn extract_fenced_body(rendered: &str, fence_len: usize) -> Vec<u8> {
    let fence = "`".repeat(fence_len);
    let bytes = rendered.as_bytes();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut offset = 0usize;
    for line in rendered.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let content = content.strip_suffix('\r').unwrap_or(content);
        if content == fence {
            spans.push((offset, offset + line.len()));
        }
        offset += line.len();
    }
    assert_eq!(
        spans.len(),
        2,
        "渲染结果应恰含开/闭两行长度 {fence_len} 的围栏行，实际：{rendered:?}"
    );
    let (open_start, open_end) = spans[0];
    let (close_start, _close_end) = spans[1];
    assert!(open_start < close_start, "开围栏必须早于闭围栏");
    bytes[open_end..close_start].to_vec()
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

/// 逐字 round-trip 断言：围栏正文字节等于 source 字节，source 只出现一次。
fn assert_round_trip(source: &str, origin: ScTeachingSourceOrigin) -> String {
    let hash = sha256_hex(source.as_bytes());
    let metadata = ScTeachingSourceMetadata {
        source_ref: SOURCE_REF,
        sha256: &hash,
        bytes: source.len() as u64,
        origin,
    };
    let rendered = render_sc_previous_source_block(source, &metadata)
        .expect("合法 source + 一致 metadata 必须渲染成功");

    let fence_len = expected_fence_len(source);
    let fence = "`".repeat(fence_len);
    assert_eq!(
        rendered.lines().filter(|line| *line == fence).count(),
        2,
        "围栏长度应为 {fence_len} 且恰有两行，实际渲染：{rendered:?}"
    );

    let body = extract_fenced_body(&rendered, fence_len);
    let mut body_with_supplemented_newline = source.as_bytes().to_vec();
    body_with_supplemented_newline.push(b'\n');
    assert!(
        body == source.as_bytes() || body == body_with_supplemented_newline,
        "围栏正文必须逐字等于 source 字节（只允许一个补行 LF），source={source:?} body={body:?}"
    );

    if !source.is_empty() {
        assert_eq!(
            count_occurrences(rendered.as_bytes(), source.as_bytes()),
            1,
            "source 必须逐字出现且只出现一次，实际渲染：{rendered:?}"
        );
    }
    rendered
}

#[test]
fn round_trips_chinese_blank_padded_crlf_and_backtick_sources_verbatim() {
    let cases = [
        "中文源码：你好，世界。\n第二行仍是中文。\n",
        "\n\n  首行前有空行与行首空白  \n末行之后还有空行   \n\n",
        "第一行用CRLF\r\n第二行也用CRLF\r\n",
        "最后一行没有换行符",
        "围栏嵌套：\n```\n```rust\nfn main() {}\n```\n",
        "八连反引号：````````",
    ];
    for case in cases {
        assert_round_trip(case, ScTeachingSourceOrigin::CompileSplitOutput);
    }
}

#[test]
fn fence_length_is_max_of_three_and_longest_backtick_run_plus_one() {
    for (source, expected_len) in [
        ("没有反引号\n", 3usize),
        ("两个反引号``\n", 3),
        ("三个反引号```\n", 4),
        ("四个反引号````\n", 5),
        ("八个反引号````````\n", 9),
    ] {
        assert_eq!(expected_fence_len(source), expected_len);
        assert_round_trip(source, ScTeachingSourceOrigin::StructureRevision);
    }
}

#[test]
fn empty_source_renders_an_empty_body() {
    let hash = sha256_hex(b"");
    let metadata = ScTeachingSourceMetadata {
        source_ref: SOURCE_REF,
        sha256: &hash,
        bytes: 0,
        origin: ScTeachingSourceOrigin::IrPreflightSplitOutput,
    };
    let rendered = render_sc_previous_source_block("", &metadata).expect("空 source 仍可渲染");
    let body = extract_fenced_body(&rendered, 3);
    assert!(
        body.is_empty() || body.as_slice() == b"\n",
        "空 source 的围栏正文应为空（允许一个补行 LF），实际：{body:?}"
    );
}

#[test]
fn tampered_hash_or_bytes_is_rejected_with_stable_evidence_code() {
    let source = "中文源码需要完整保留，不得截断。\n";
    let good_hash = sha256_hex(source.as_bytes());
    let good_bytes = source.len() as u64;
    let tampered_hash = sha256_hex(b"tampered-source");

    let bad_hash = ScTeachingSourceMetadata {
        source_ref: SOURCE_REF,
        sha256: &tampered_hash,
        bytes: good_bytes,
        origin: ScTeachingSourceOrigin::CompileSplitOutput,
    };
    let err = render_sc_previous_source_block(source, &bad_hash).expect_err("hash 篡改必须报错");
    assert_eq!(err.code, INVALID_EVIDENCE_CODE);

    let bad_bytes = ScTeachingSourceMetadata {
        source_ref: SOURCE_REF,
        sha256: &good_hash,
        bytes: good_bytes + 1,
        origin: ScTeachingSourceOrigin::CompileSplitOutput,
    };
    let err = render_sc_previous_source_block(source, &bad_bytes).expect_err("bytes 篡改必须报错");
    assert_eq!(err.code, INVALID_EVIDENCE_CODE);
}

#[test]
fn renders_for_all_three_source_origins_with_frozen_wire_names() {
    let source = "结构修订来源的中文源码。\n";
    let cases = [
        (
            ScTeachingSourceOrigin::CompileSplitOutput,
            "compile_split_output",
        ),
        (
            ScTeachingSourceOrigin::IrPreflightSplitOutput,
            "ir_preflight_split_output",
        ),
        (
            ScTeachingSourceOrigin::StructureRevision,
            "structure_revision",
        ),
    ];
    for (origin, wire) in cases {
        assert_eq!(
            serde_json::to_string(&origin).unwrap(),
            format!("\"{wire}\"")
        );
        assert_round_trip(source, origin);
    }
}

#[test]
fn oversized_source_is_rendered_without_truncation() {
    let source = "中".repeat(100_000);
    assert_eq!(source.len(), 300_000);
    assert_round_trip(&source, ScTeachingSourceOrigin::IrPreflightSplitOutput);
}
