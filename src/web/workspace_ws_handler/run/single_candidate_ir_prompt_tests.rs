// 从 single_candidate.rs 移出（large_file_guard 登记上限锁定，纯移动）：
// r41/r45 可教解析失败与 IR 教学重驱提示词的定向用例。
//
// P26 切面：旧三条用例是措辞 pin（IR builder 名 + 中文说明句），随 P22 facade
// 落库一并退役（旧 builder 已从 preflight 移除，生产只经 facade 装配）。
// 此处改为经 facade 的行为断言——source 逐字在场、摘要/字节计数自洽、
// 已登记编号成员完整度；不钉提示词句子，也不重复 facade 自带的三个预算
// 边界用例（上限内/超限 1 字节/IR 尾段顶出）。
use super::*;

use super::sc_teaching_prompt::prepare_ir_teaching_prompt;
use super::sc_teaching_source::ScResolvedTeachingSource;
use super::single_candidate_compile_preflight::PreflightVerdict;
use crate::web::workspace_ws_types::ScTeachingSourceOrigin;
use sha2::Digest as _;

/// 教学 source fixture 的 locator（形态取自真实 resolver 产出）。
const SOURCE_REF: &str = "sc-teaching-source/v1/session-1/chain-1/0";
/// 教学 source fixture 正文（含中文 ⇒ 字符数 != 字节数）。
const SOURCE_TEXT: &str = "# Work Item Plan\n## 工作项 WI-001: 载荷\n### 溯源清单\n";

/// 全文摘要（与证据库同一口径：sha256 小写 hex）。
fn sha256_hex(text: &str) -> String {
    hex::encode(sha2::Sha256::digest(text.as_bytes()))
}

/// 显式 source fixture：摘要与字节由正文实算（不自造摘要）。
fn resolved_source() -> ScResolvedTeachingSource {
    ScResolvedTeachingSource {
        text: SOURCE_TEXT.to_string(),
        failure_source_ref: SOURCE_REF.to_string(),
        sha256: sha256_hex(SOURCE_TEXT),
        bytes: SOURCE_TEXT.len() as u64,
        origin: ScTeachingSourceOrigin::IrPreflightSplitOutput,
    }
}

/// IR 预检事实 fixture（typed 载荷缺席 ⇒ 仅呈现面；不扩资格面）。
fn preflight() -> PreflightVerdict {
    PreflightVerdict {
        ir_faults: Vec::new(),
        operational_failures: Vec::new(),
        failure_reasons: vec![
            "unknown_requirement_ref: task references unknown design requirement NFR-1".to_string(),
        ],
    }
}

/// r41 B2:unknown_structured_key 属可教面(一次自愈机会,F-48 先例)。
#[test]
fn unknown_structured_key_is_teachable() {
    assert!(is_teachable_parse_failure("unknown_structured_key"));
    assert!(is_teachable_parse_failure("missing_section"));
    assert!(!is_teachable_parse_failure("internal_error"));
}

/// IR 教学 prompt：source 逐字在场，且来源事实/摘要/字节与 source 自洽。
#[test]
fn ir_prompt_embeds_source_verbatim_with_matching_source_facts() {
    let source = resolved_source();
    let prepared =
        prepare_ir_teaching_prompt(&preflight(), &[], &source).expect("IR 教学 prompt 必须可装配");
    assert!(
        prepared.text.contains(SOURCE_TEXT),
        "source 必须逐字完整在场"
    );
    assert_eq!(prepared.source_ref, source.failure_source_ref);
    assert_eq!(prepared.source_sha256, source.sha256);
    assert_eq!(prepared.source_bytes, source.bytes);
    assert_eq!(prepared.prompt_digest, sha256_hex(&prepared.text));
    assert_eq!(prepared.prompt_bytes, prepared.text.len() as u64);
}

/// IR 教学 prompt：已登记编号逐个在场；空清单不注入编号（只减字节）。
#[test]
fn ir_prompt_carries_registered_requirement_ids_and_empty_list_adds_none() {
    let source = resolved_source();
    let ids = vec![
        "REQ-ENV-01".to_string(),
        "NFR-PERF-02".to_string(),
        "AC-001".to_string(),
    ];
    let with_ids = prepare_ir_teaching_prompt(&preflight(), &ids, &source)
        .expect("带清单的 IR 教学 prompt 必须可装配");
    for id in &ids {
        assert!(
            with_ids.text.contains(id.as_str()),
            "已登记编号必须完整在场: {id}"
        );
    }
    let without_ids = prepare_ir_teaching_prompt(&preflight(), &[], &source)
        .expect("空清单的 IR 教学 prompt 必须可装配");
    for id in &ids {
        assert!(
            !without_ids.text.contains(id.as_str()),
            "空清单不得注入已登记编号: {id}"
        );
    }
    assert!(
        with_ids.prompt_bytes > without_ids.prompt_bytes,
        "清单只增字节（空清单 {} < 带清单 {}）",
        without_ids.prompt_bytes,
        with_ids.prompt_bytes
    );
}
