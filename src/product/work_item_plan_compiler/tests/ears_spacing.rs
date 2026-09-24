//! EARS 关键字空白确定性归一化契约(TDD)。
//!
//! 现场依据(F-48):pi 交付的 36 条 statement 中恰 2 条触发条件以全角右角括号
//! `」`(U+300D) 收尾后直接接 `THE SYSTEM SHALL`(缺 1 个半角空格),被 grammar
//! 字面量契约(`EARS_SHALL_PREFIX` 含前导半角空格)fail-closed 判 `invalid_ears`
//! 并直达 SingleCandidate 终态。输入为 durable 原文逐字节提取
//! (`f48_plan_ears_raw.md`,17005 字符 / 485 行,sha256 def17cd2…)。
//!
//! 契约:归一化只做三类空白操作——①`WHEN` 后补半角空格、②`THE SYSTEM SHALL`
//! 前补半角空格、③关键字邻位 U+3000/NBSP 归一为半角空格;且只在「原文不满足
//! EARS 契约、补齐后满足」时改写。正文与 CJK 标点逐字保留,非空白类语法错误
//! 照旧 fail-closed。
//!
//! fixture 提取纪律(承 F-46 教训):python `json.load` 读 `streaming_content`
//! 后逐字节写入,不得经 `jq -r`(会追加尾换行)。

use crate::product::work_item_plan_compiler::{
    WorkItemPlanSourceContext, compile_work_item_plan, normalize_delivery_before_compile,
    normalize_ears_statement_spacing,
};

use super::REP4_FIXTURE;

/// F-48 durable 原文(`timeline_node_002.streaming_content`)逐字节提取件。
const F48_PLAN_EARS_RAW: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/product/workspace_engine/tests/fixtures/f48_plan_ears_raw.md"
));

/// rep4 fixture 首条 statement 原文(仅 Tasks 段出现一次,可安全定点替换)。
const REP4_FIRST_STATEMENT: &str = "- statement: WHEN the levels API receives GET /api/levels \
     THE SYSTEM SHALL return the configured levels JSON.";

fn compile_item_count(source: &str) -> Result<usize, Vec<String>> {
    compile_work_item_plan(
        source,
        &WorkItemPlanSourceContext {
            target_repository_id: "repository_0001".to_string(),
        },
    )
    .map(|ir| ir.items.len())
    .map_err(|diagnostics| {
        diagnostics
            .iter()
            .map(|diagnostic| {
                format!(
                    "{}:{}:{}",
                    diagnostic.code, diagnostic.line, diagnostic.message
                )
            })
            .collect()
    })
}

/// 把 rep4 首条 statement 替换为给定值(保持计划其余部分合法)。
fn rep4_with_statement(statement_value: &str) -> String {
    let replaced = REP4_FIXTURE.replacen(
        REP4_FIRST_STATEMENT,
        &format!("- statement: {statement_value}"),
        1,
    );
    assert_ne!(
        replaced, REP4_FIXTURE,
        "rep4 fixture 首条 statement 原文必须存在"
    );
    replaced
}

/// 去掉全部 Unicode 空白后的字面量(用于「正文与标点零触碰」断言)。
fn without_whitespace(source: &str) -> String {
    source.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
fn f48_field_delivery_is_rescued_by_ears_spacing_normalization() {
    // 红:原文必须复现 durable 定案的恰好 2 条 invalid_ears(L158/L193)。
    let raw_diagnostics =
        compile_item_count(F48_PLAN_EARS_RAW).expect_err("F-48 原文必须复现 invalid_ears 编译失败");
    assert_eq!(
        raw_diagnostics.len(),
        2,
        "F-48 原文只应有两处 EARS 空白缺口: {raw_diagnostics:?}"
    );
    assert!(
        raw_diagnostics[0].starts_with("invalid_ears:158:"),
        "首条诊断必须落在 L158: {raw_diagnostics:?}"
    );
    assert!(
        raw_diagnostics[1].starts_with("invalid_ears:193:"),
        "第二条诊断必须落在 L193: {raw_diagnostics:?}"
    );

    // 绿:归一化只补 2 处关键字空白,编译通过且 item 数不变。
    let normalized = normalize_ears_statement_spacing(F48_PLAN_EARS_RAW);
    assert_eq!(normalized.normalized_ears_lines, 2);
    assert_eq!(normalized.normalized_heading_lines, 0);
    assert_eq!(
        compile_item_count(&normalized.source).expect("F-48 归一化后必须编译通过"),
        4,
        "救回不得改变 item 结构"
    );

    // 只有 L158/L193 两行变化,其余 483 行逐字节相同。
    let before: Vec<&str> = F48_PLAN_EARS_RAW.split('\n').collect();
    let after: Vec<&str> = normalized.source.split('\n').collect();
    assert_eq!(before.len(), after.len(), "归一化不得增删行");
    let changed: Vec<usize> = before
        .iter()
        .zip(&after)
        .enumerate()
        .filter(|(_, (left, right))| left != right)
        .map(|(index, _)| index + 1)
        .collect();
    assert_eq!(changed, vec![158, 193], "只有两处缺口行允许被改写");
    assert_eq!(
        after[157],
        "- statement: WHEN 玩家点击「重新开始」 THE SYSTEM SHALL 清空全部棋子并恢复黑方先行与 playing 状态。"
    );
    assert_eq!(
        after[192],
        "- statement: WHEN 点击「重新开始」 THE SYSTEM SHALL 移除全部棋子并恢复黑方先行。"
    );
}

#[test]
fn minimal_ears_spacing_gap_samples_are_rescued() {
    let cases: [(&str, &str); 6] = [
        // ② 条件以 CJK 标点收尾后直接接关键字(F-48 形态)。
        (
            "WHEN条件」THE SYSTEM SHALL 响应",
            "WHEN 条件」 THE SYSTEM SHALL 响应",
        ),
        // ① WHEN 后缺半角空格。
        (
            "WHEN条件 THE SYSTEM SHALL 响应",
            "WHEN 条件 THE SYSTEM SHALL 响应",
        ),
        // ③ WHEN 邻位 U+3000。
        (
            "WHEN\u{3000}条件 THE SYSTEM SHALL 响应",
            "WHEN 条件 THE SYSTEM SHALL 响应",
        ),
        // ③ 关键字前邻位 U+3000。
        (
            "WHEN 条件\u{3000}THE SYSTEM SHALL 响应",
            "WHEN 条件 THE SYSTEM SHALL 响应",
        ),
        // ③ 关键字前邻位 NBSP(U+00A0)。
        (
            "WHEN 条件\u{00a0}THE SYSTEM SHALL 响应",
            "WHEN 条件 THE SYSTEM SHALL 响应",
        ),
        // ③ 关键字后邻位 U+3000。
        (
            "WHEN 条件 THE SYSTEM SHALL\u{3000}响应",
            "WHEN 条件 THE SYSTEM SHALL 响应",
        ),
    ];
    for (gap, expected) in cases {
        let source = rep4_with_statement(gap);
        let normalized = normalize_ears_statement_spacing(&source);
        assert_eq!(
            normalized.normalized_ears_lines, 1,
            "缺口样本必须恰好救回一条: {gap:?}"
        );
        assert!(
            normalized
                .source
                .contains(&format!("- statement: {expected}")),
            "缺口样本 {gap:?} 的归一化结果不符: {:?}",
            normalized
                .source
                .lines()
                .find(|line| line.starts_with("- statement: WHEN"))
        );
        assert_eq!(
            compile_item_count(&normalized.source).unwrap_or_else(|diagnostics| panic!(
                "{gap:?} 归一化后必须编译通过: {diagnostics:?}"
            )),
            3
        );
    }
}

#[test]
fn ears_spacing_normalization_touches_only_keyword_adjacent_whitespace() {
    // 原文 + 三类缺口样本:归一化前后「去掉全部空白」必须逐字相同(CJK 标点与
    // 正文零触碰),行数不变,且被改写行只能是 `- statement:` 字段行。
    let samples = [
        F48_PLAN_EARS_RAW.to_string(),
        rep4_with_statement("WHEN条件」THE SYSTEM SHALL 响应"),
        rep4_with_statement("WHEN\u{3000}条件 THE SYSTEM SHALL\u{00a0}响应"),
    ];
    for source in samples {
        let normalized = normalize_ears_statement_spacing(&source);
        assert_eq!(
            source.split('\n').count(),
            normalized.source.split('\n').count(),
            "归一化不得增删行"
        );
        assert_eq!(
            without_whitespace(&source),
            without_whitespace(&normalized.source),
            "归一化只允许触碰空白(含 U+3000/NBSP→半角),正文与 CJK 标点必须逐字保留"
        );
        assert_eq!(
            source.matches('」').count(),
            normalized.source.matches('」').count(),
            "CJK 标点不得被改写"
        );
        for (before, after) in source.split('\n').zip(normalized.source.split('\n')) {
            if before != after {
                assert!(
                    before.starts_with("- statement: "),
                    "只有 statement 字段行允许被改写: {before:?}"
                );
            }
        }
    }
}

#[test]
fn ears_spacing_normalization_is_noop_without_gap_and_idempotent() {
    let already_legal = normalize_ears_statement_spacing(REP4_FIXTURE);
    assert_eq!(
        already_legal.normalized_ears_lines, 0,
        "合法交付不得触发救回审计"
    );
    assert_eq!(already_legal.source, REP4_FIXTURE);

    let rescued = normalize_ears_statement_spacing(F48_PLAN_EARS_RAW);
    let rescued_again = normalize_ears_statement_spacing(&rescued.source);
    assert_eq!(rescued_again.normalized_ears_lines, 0);
    assert_eq!(rescued_again.source, rescued.source, "归一化必须幂等");
}

#[test]
fn delivery_normalization_composes_headings_and_ears_spacing() {
    // 中文结构标题 + EARS 缺口同在一份交付里:一次净化必须同时解决两类并各自报数。
    let mixed = rep4_with_statement("WHEN条件」THE SYSTEM SHALL 响应")
        .replacen("### Identity", "### 身份", 1)
        .replacen("### Tasks", "### 任务", 1)
        .replacen(
            "### Acceptance Criteria",
            "### 验收标准 (Acceptance Criteria)",
            1,
        );

    let normalized = normalize_delivery_before_compile(&mixed);

    assert_eq!(normalized.normalized_heading_lines, 3);
    assert_eq!(normalized.normalized_ears_lines, 1);
    assert_eq!(
        compile_item_count(&normalized.source).expect("组合归一化后必须编译通过"),
        3
    );
}

#[test]
fn non_whitespace_ears_violations_stay_fail_closed() {
    // 非空白类语法错误:缺 WHEN 前缀 / 关键字缺失或顺序错误 / 关键字后缺空格
    // (不在三类允许操作内)/ 结果缺失。归一化必须零改动,compiler 照旧拒绝。
    let cases = [
        "条件」THE SYSTEM SHALL 响应",
        "WHEN 条件 THEN 响应",
        "WHEN 条件 THE SYSTEM SHALL响应",
        "WHEN 条件",
    ];
    for gap in cases {
        let source = rep4_with_statement(gap);
        let normalized = normalize_ears_statement_spacing(&source);
        assert_eq!(normalized.normalized_ears_lines, 0, "{gap:?} 不得被救回");
        assert_eq!(normalized.source, source, "{gap:?} 不得被改写");
        let diagnostics =
            compile_item_count(&normalized.source).expect_err("非空白缺口必须照旧 fail-closed");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.starts_with("invalid_ears:")),
            "{gap:?} 必须仍以 invalid_ears 拒绝: {diagnostics:?}"
        );
    }
}

#[test]
fn ears_spacing_normalization_leaves_headings_and_free_text_untouched() {
    // 表外未知标题不猜不改(仍是 fail-closed 的编译器职责);自由文本 section 中
    // 形似 statement 的行也不得被当 EARS 字段改写。
    let source = "# 工作项计划\n## 工作项 WI-001: x\n### 溯源清单\n- statement: WHEN条件」THE SYSTEM SHALL 响应\n### Rationale\n- statement: WHEN条件」THE SYSTEM SHALL 响应\n";

    let normalized = normalize_ears_statement_spacing(source);

    assert_eq!(normalized.normalized_ears_lines, 0);
    assert_eq!(normalized.source, source);
}
