//! 首轮失败诊断「可执行化」的回归用例（2026-11-15）。
//!
//! 目标：作者拿到诊断就能**一处自纠**——诊断必须同时给出「实际写了什么」
//! 与「允许什么 / 冲突什么」。本文件只钉**事实在场**（实际值、允许集），
//! 不钉整句文案（措辞会变，事实不会）。
//!
//! 覆盖：
//! 1. 未知结构化 key（`parse.rs`）——必须带该 section 的合法 key 集；
//! 2. 非 key/value 行（`parse.rs`）——必须带违规原文行；
//! 3. 游离正文（`parse.rs`，落在任何 `###` 之前）——必须带合法 section 列表；
//! 4. `provided_contract_refs` 的二元组方言（`lower.rs`）——`(provider, contract)`
//!    必须归一为 `contract`，否则同一 WI 两行即造出「`(WI-001` 重复」假阳性。

use super::grammar;
use super::parse::parse_work_item_plan;
use super::types::CompilerDiagnostic;

/// 单 WI 计划骨架（各 section 齐全）；`extra` 插入到 `section` 段内（紧随其
/// 首个内容行），`before_any_section` 则插在第一个 `###` 之前（游离正文）。
fn plan_with(section: &str, extra: &str, before_any_section: bool) -> String {
    let mut lines: Vec<String> = vec![
        "# Work Item Plan".to_string(),
        String::new(),
        "## Work Item WI-001: Fixture".to_string(),
        String::new(),
    ];
    if before_any_section {
        lines.push(extra.to_string());
        lines.push(String::new());
    }
    lines.extend(
        [
            "### Identity",
            "- schema_version: 1",
            "- logical_work_item_id: WI-001",
            "- title: Fixture",
            "- kind: backend",
            "",
            "### Goal",
            "- summary: 目标",
            "",
            "### Non Goals",
            "- non_goals: 无",
            "",
        ]
        .map(str::to_string),
    );
    if section == "Identity" {
        // 插到 Identity 段内（避免重复 section）。
        let index = lines
            .iter()
            .position(|line| line == "- kind: backend")
            .expect("Identity 段存在");
        lines.insert(index + 1, extra.to_string());
    }
    lines.push(String::new());
    lines.join("\n")
}

fn diagnostics_of(source: &str) -> Vec<CompilerDiagnostic> {
    parse_work_item_plan(source).expect_err("该 source 必须被拒绝")
}

/// 未知 key 的诊断必须带「实际 key」与「该 section 的合法 key 集」——作者据此
/// 一处自纠，不必反查 schema。
#[test]
fn unknown_key_diagnostic_carries_actual_key_and_section_vocabulary() {
    let source = plan_with("Identity", "- schema_verzion: 1", false);
    let diagnostics = diagnostics_of(&source);
    let unknown = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "unknown_structured_key")
        .expect("必须报未知 key");
    assert!(
        unknown.message.contains("schema_verzion"),
        "诊断必须回显实际写下的 key: {}",
        unknown.message
    );
    for allowed in grammar::section_allowed_keys("Identity") {
        assert!(
            unknown.message.contains(allowed),
            "诊断必须列出 Identity 的合法 key {allowed}: {}",
            unknown.message
        );
    }
}

/// 非 key/value 行必须带**违规原文行**——否则作者无法定位是哪一行坏了。
#[test]
fn non_key_value_line_diagnostic_carries_offending_raw_line() {
    let source = plan_with("Identity", "schema_version: 1", false);
    let diagnostics = diagnostics_of(&source);
    let bad = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "unknown_structured_key")
        .expect("必须报非 key/value 行");
    assert!(
        bad.message.contains("schema_version: 1"),
        "诊断必须回显违规原文行: {}",
        bad.message
    );
}

/// 游离正文（落在任何 `###` 之前）必须带**合法 section 列表**。
#[test]
fn stray_content_diagnostic_carries_legal_section_list() {
    let source = plan_with("", "游离的一行正文", true);
    let diagnostics = diagnostics_of(&source);
    let stray = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "unknown_structured_key")
        .expect("必须报游离正文");
    assert!(
        stray.message.contains("游离的一行正文"),
        "诊断必须回显游离原文行: {}",
        stray.message
    );
    for section in grammar::STRUCTURED_SECTIONS {
        assert!(
            stray.message.contains(section),
            "诊断必须列出合法 section {section}: {}",
            stray.message
        );
    }
}

/// `provided_contract_refs` 的二元组方言必须归一：`(provider_wi, contract_id)`
/// → `contract_id`。不归一时 `,` 切分会把 `(WI-001` 与 `c-x)` 当成两个 ref。
#[test]
fn provided_contract_refs_tuple_dialect_is_normalized_to_contract_id() {
    let normalized = super::lower::split_provided_contract_refs(vec!["(WI-001, CT-004)"]);
    assert_eq!(
        normalized,
        vec!["CT-004".to_string()],
        "二元组方言必须归一为 contract_id"
    );
}

/// 归一不得改动普通取值（多值、方括号空值、单值）。
#[test]
fn provided_contract_refs_normalization_keeps_plain_values() {
    let normalized =
        super::lower::split_provided_contract_refs(vec!["CT-001, CT-002", "[]", "CT-003"]);
    assert_eq!(
        normalized,
        vec![
            "CT-001".to_string(),
            "CT-002".to_string(),
            "CT-003".to_string(),
        ],
        "普通取值不得被归一改写"
    );
}
